//! The token's plumbing (FROZEN, W.3; §20, D17).
//!
//! fc-core's `TokenStore` is synchronous and is read on every request; the Secret Service is
//! asynchronous. So the connection's token store is [`VaultedTokens`] — the token in memory, read
//! ONCE from a [`TokenVault`] at start-up (the gate stays Booting until it answers), every write
//! going to both, the vault's half spawned on the main context (Windows' `LockerTokenStore`: "read
//! once, write to both").
//!
//! P5 implements [`TokenVault`] over libsecret (schema `me.nettrash.FamilyConnect.Session`,
//! attributes `server` = the origin and `account` = "session", label "Family Connect (<host>)",
//! the default collection) and installs it in `Shell::vault`. S0's [`MemoryVault`] keeps tokens
//! for this run only — the "isn't remembered on this computer" state. Never a plaintext file.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use fc_core::protocol::{ServerUrl, TokenStore};
use fc_core::rt::LocalBoxFuture;
use fc_logic::calls::Spawn;

/// What a vault answered for an origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultRead {
    Found(String),
    /// Nothing stored for this origin.
    Empty,
    /// No vault answered (no Secret Service, a locked collection the person refused to unlock).
    /// The sign-in then lives in memory for this run.
    Unavailable,
}

/// Where tokens are kept between runs (P5: the Secret Service). `origin` is
/// [`origin_of`]'s — scheme, host and port, never a path.
pub trait TokenVault {
    /// The token stored for `origin`.
    fn load(&self, origin: &str) -> LocalBoxFuture<'static, VaultRead>;

    /// Store `token` for `origin` (`None`: clear it). Answers whether the vault kept the change.
    fn save(&self, origin: &str, token: Option<String>) -> LocalBoxFuture<'static, bool>;

    /// Whether what is saved outlives the process — Settings' Server group says "Your sign-in
    /// isn't remembered on this computer: no keyring is available." when it does not (or when the
    /// last load or save failed: [`VaultedTokens::remembered`]).
    fn persistent(&self) -> bool;
}

/// The vault key of a server: `https://chat.example.com`, `http://192.168.1.10:8080`.
pub fn origin_of(server: &ServerUrl) -> String {
    server.to_string().trim_end_matches('/').to_string()
}

/// S0's vault: this process's memory. Nothing outlives the run.
#[derive(Debug, Default)]
pub struct MemoryVault(Rc<RefCell<HashMap<String, String>>>);

impl TokenVault for MemoryVault {
    fn load(&self, origin: &str) -> LocalBoxFuture<'static, VaultRead> {
        let read = match self.0.borrow().get(origin) {
            Some(token) => VaultRead::Found(token.clone()),
            None => VaultRead::Empty,
        };
        Box::pin(std::future::ready(read))
    }

    fn save(&self, origin: &str, token: Option<String>) -> LocalBoxFuture<'static, bool> {
        match token {
            Some(token) => self.0.borrow_mut().insert(origin.to_string(), token),
            None => self.0.borrow_mut().remove(origin),
        };
        Box::pin(std::future::ready(true))
    }

    fn persistent(&self) -> bool {
        false
    }
}

/// fc-core's `TokenStore` for one server: memory first, the vault behind it.
pub struct VaultedTokens {
    origin: String,
    vault: Rc<dyn TokenVault>,
    spawn: Spawn,
    token: RefCell<Option<String>>,
    /// Whether the vault keeps it: `false` after an `Unavailable` load or a failed save, and for a
    /// vault that is not persistent.
    remembered: Rc<Cell<bool>>,
}

impl std::fmt::Debug for VaultedTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the token.
        f.debug_struct("VaultedTokens")
            .field("held", &self.token.borrow().is_some())
            .field("remembered", &self.remembered.get())
            .finish_non_exhaustive()
    }
}

impl VaultedTokens {
    pub fn new(server: &ServerUrl, vault: Rc<dyn TokenVault>, spawn: Spawn) -> VaultedTokens {
        let remembered = vault.persistent();
        VaultedTokens {
            origin: origin_of(server),
            vault,
            spawn,
            token: RefCell::new(None),
            remembered: Rc::new(Cell::new(remembered)),
        }
    }

    /// Reads the vault once (start-up, a new server): what it holds becomes the token in memory —
    /// unless a sign-in set one meanwhile, which wins.
    pub async fn load(&self) -> VaultRead {
        let read = self.vault.load(&self.origin).await;
        match &read {
            VaultRead::Found(token) => {
                let mut held = self.token.borrow_mut();
                if held.is_none() && !token.is_empty() {
                    *held = Some(token.clone());
                }
            }
            VaultRead::Empty => {}
            VaultRead::Unavailable => self.remembered.set(false),
        }
        read
    }

    /// Whether the sign-in is kept for the next run.
    pub fn remembered(&self) -> bool {
        self.remembered.get()
    }
}

impl TokenStore for VaultedTokens {
    fn token(&self) -> Option<String> {
        self.token.borrow().clone()
    }

    fn set_token(&self, token: Option<String>) {
        let token = token.filter(|t| !t.is_empty());
        *self.token.borrow_mut() = token.clone();
        let saving = self.vault.save(&self.origin, token);
        let (remembered, persistent) = (Rc::clone(&self.remembered), self.vault.persistent());
        (self.spawn)(Box::pin(async move {
            let kept = saving.await;
            if !kept {
                crate::fc_warn!("token: the keyring did not keep the change");
            }
            remembered.set(kept && persistent);
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::LocalPool;
    use futures::task::LocalSpawnExt;

    fn server() -> ServerUrl {
        ServerUrl::normalise(Some("chat.example.com")).unwrap()
    }

    #[test]
    fn the_origin_is_scheme_host_and_port_without_a_path() {
        assert_eq!(origin_of(&server()), "https://chat.example.com");
        let local = ServerUrl::normalise(Some("http://192.168.1.10:8080/some/path")).unwrap();
        assert_eq!(origin_of(&local), "http://192.168.1.10:8080");
    }

    #[test]
    fn a_token_is_read_once_held_in_memory_and_every_write_goes_to_both() {
        let mut pool = LocalPool::new();
        let spawner = pool.spawner();
        let spawn: Spawn = Rc::new(move |future| spawner.spawn_local(future).unwrap());
        let vault = Rc::new(MemoryVault::default());
        let tokens = VaultedTokens::new(&server(), vault.clone(), spawn.clone());
        assert_eq!(pool.run_until(tokens.load()), VaultRead::Empty);
        assert_eq!(tokens.token(), None);
        tokens.set_token(Some("t1".into()));
        assert_eq!(tokens.token().as_deref(), Some("t1"));
        pool.run_until_stalled();
        // A second store over the same vault (a relaunch within the run) reads it back.
        let again = VaultedTokens::new(&server(), vault.clone(), spawn.clone());
        assert_eq!(pool.run_until(again.load()), VaultRead::Found("t1".into()));
        assert_eq!(again.token().as_deref(), Some("t1"));
        again.set_token(Some(String::new()));
        assert_eq!(again.token(), None);
        pool.run_until_stalled();
        let third = VaultedTokens::new(&server(), vault, spawn);
        assert_eq!(pool.run_until(third.load()), VaultRead::Empty);
        // Memory is not a keyring: never "remembered".
        assert!(!third.remembered());
    }

    #[test]
    fn a_sign_in_during_the_read_wins_over_what_the_vault_held() {
        let mut pool = LocalPool::new();
        let spawner = pool.spawner();
        let spawn: Spawn = Rc::new(move |future| spawner.spawn_local(future).unwrap());
        let vault = Rc::new(MemoryVault::default());
        vault
            .0
            .borrow_mut()
            .insert("https://chat.example.com".into(), "old".into());
        let tokens = VaultedTokens::new(&server(), vault, spawn);
        tokens.set_token(Some("new".into()));
        pool.run_until(tokens.load());
        assert_eq!(tokens.token().as_deref(), Some("new"));
    }
}
