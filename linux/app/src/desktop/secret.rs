//! The token in the Secret Service (shell-design §20, D17) — `SecretTokenStore` of §2.4, the
//! [`TokenVault`] S0's `VaultedTokens` reads once at start-up and writes through.
//!
//! - libsecret 0.9 on the GLib main context, never a sync call: schema
//!   `me.nettrash.FamilyConnect.Session`, attributes `server` = the server's ORIGIN (`origin_of`:
//!   scheme, host and port — never a path) and `account` = `session`, label "Family Connect
//!   (<host>)", the DEFAULT collection (the login keyring, unlocked by `libpam-gnome-keyring` at
//!   login). The round trip was measured in the container (§1.5).
//! - Through a `libsecret::Service` the vault opens itself (`secret_service_open`, with a session),
//!   kept while it answers and opened afresh after a failure (a daemon started later). Its
//!   `service_bus_name` argument is IGNORED by libsecret 0.21 (measured: the proxy names
//!   `org.freedesktop.secrets` whatever is passed), so the self-test proves `Unavailable` in a child
//!   process on a bus with no Secret Service at all. The service's `lookup` is not used: its binding
//!   hands a missing item to `from_glib_full` as a NULL `Value`; `search` answers an empty list.
//! - A found item still LOCKED after `search` asked to unlock it is the person refusing GNOME's
//!   unlock prompt: `Unavailable`, not "nothing stored".
//! - **No Secret Service** (no keyring daemon, a locked collection the person refuses to unlock, a
//!   desktop without one): [`VaultRead::Unavailable`], and [`TokenVault::persistent`] is false from
//!   then on, so Settings' Server group says the sign-in is not remembered. The token then lives
//!   in memory for this run — never a plaintext file.
//! - Logs say what happened and an error's kind; never the token, never the origin.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use fc_core::rt::LocalBoxFuture;
use glib::prelude::*;
use libsecret::prelude::*;

use crate::seams::tokens::{TokenVault, VaultRead};
use crate::shell::BRAND;

/// The schema's name (§1.5, §20).
pub const SCHEMA_NAME: &str = "me.nettrash.FamilyConnect.Session";
/// The attribute holding the server's origin.
pub const SERVER_ATTRIBUTE: &str = "server";
/// The attribute naming what is stored.
pub const ACCOUNT_ATTRIBUTE: &str = "account";
/// Its one value.
pub const ACCOUNT: &str = "session";

/// The item's label in Seahorse: the brand and the host (no words to translate).
pub fn label_for(origin: &str) -> String {
    let host = origin
        .split_once("://")
        .map_or(origin, |(_, rest)| rest)
        .trim_end_matches('/');
    format!("{BRAND} ({host})")
}

/// The attributes an origin's item carries.
pub fn attributes(origin: &str) -> HashMap<&str, &str> {
    HashMap::from([(SERVER_ATTRIBUTE, origin), (ACCOUNT_ATTRIBUTE, ACCOUNT)])
}

fn schema() -> libsecret::Schema {
    libsecret::Schema::new(
        SCHEMA_NAME,
        libsecret::SchemaFlags::NONE,
        HashMap::from([
            (SERVER_ATTRIBUTE, libsecret::SchemaAttributeType::String),
            (ACCOUNT_ATTRIBUTE, libsecret::SchemaAttributeType::String),
        ]),
    )
}

/// The libsecret vault (see the module's comment).
pub struct SecretVault {
    inner: Rc<Inner>,
}

struct Inner {
    service: RefCell<Option<libsecret::Service>>,
    /// False once the service did not answer: nothing saved outlives the run.
    persistent: Cell<bool>,
}

impl Default for SecretVault {
    fn default() -> Self {
        SecretVault::new()
    }
}

impl SecretVault {
    /// The desktop's Secret Service.
    pub fn new() -> SecretVault {
        SecretVault {
            inner: Rc::new(Inner {
                service: RefCell::new(None),
                persistent: Cell::new(true),
            }),
        }
    }
}

impl Inner {
    /// The service, opened once (with a session for the secrets), or the error.
    async fn service(&self) -> Result<libsecret::Service, glib::Error> {
        if let Some(service) = self.service.borrow().as_ref() {
            return Ok(service.clone());
        }
        let service = libsecret::Service::open_future(
            libsecret::Service::static_type(),
            None,
            libsecret::ServiceFlags::OPEN_SESSION,
        )
        .await?;
        *self.service.borrow_mut() = Some(service.clone());
        Ok(service)
    }

    fn failed(&self, what: &str, error: &glib::Error) {
        if self.persistent.replace(false) {
            crate::fc_warn!(
                "token: no keyring answered the {what} ({} {}); the sign-in lives in memory",
                error.domain().as_str(),
                error_code(error)
            );
        }
        // A service that failed once is opened afresh next time (a daemon started later).
        self.service.borrow_mut().take();
    }

    async fn load(self: Rc<Self>, origin: String) -> VaultRead {
        let service = match self.service().await {
            Ok(service) => service,
            Err(error) => {
                self.failed("read", &error);
                return VaultRead::Unavailable;
            }
        };
        let found = service
            .search_future(
                Some(&schema()),
                attributes(&origin),
                libsecret::SearchFlags::UNLOCK | libsecret::SearchFlags::LOAD_SECRETS,
            )
            .await;
        let items = match found {
            Ok(items) => items,
            Err(error) => {
                self.failed("read", &error);
                return VaultRead::Unavailable;
            }
        };
        self.persistent.set(true);
        let Some(item) = items.first() else {
            crate::fc_info!("token: nothing in the keyring for this server");
            return VaultRead::Empty;
        };
        if item.is_locked() {
            // The person refused to unlock the keyring: not "nothing stored".
            crate::fc_info!("token: the keyring stayed locked");
            self.persistent.set(false);
            return VaultRead::Unavailable;
        }
        match item.secret().and_then(|value| value.text()) {
            Some(token) if !token.is_empty() => {
                crate::fc_info!("token: read from the keyring");
                VaultRead::Found(token.to_string())
            }
            _ => VaultRead::Empty,
        }
    }

    async fn save(self: Rc<Self>, origin: String, token: Option<String>) -> bool {
        let service = match self.service().await {
            Ok(service) => service,
            Err(error) => {
                self.failed("write", &error);
                return false;
            }
        };
        let done = match token {
            Some(token) => {
                let value = libsecret::Value::new(&token, "text/plain");
                service
                    .store_future(
                        Some(&schema()),
                        attributes(&origin),
                        Some(libsecret::COLLECTION_DEFAULT.as_str()),
                        &label_for(&origin),
                        &value,
                    )
                    .await
            }
            None => {
                service
                    .clear_future(Some(&schema()), attributes(&origin))
                    .await
            }
        };
        match done {
            Ok(()) => {
                self.persistent.set(true);
                crate::fc_info!("token: the keyring kept the change");
                true
            }
            Err(error) => {
                self.failed("write", &error);
                false
            }
        }
    }
}

/// The numeric code of a libsecret or D-Bus error.
fn error_code(error: &glib::Error) -> i32 {
    use glib::translate::IntoGlib;
    if let Some(kind) = error.kind::<gio::DBusError>() {
        return kind.into_glib();
    }
    if let Some(kind) = error.kind::<libsecret::Error>() {
        return kind.into_glib();
    }
    if let Some(kind) = error.kind::<gio::IOErrorEnum>() {
        return kind.into_glib();
    }
    -1
}

impl TokenVault for SecretVault {
    fn load(&self, origin: &str) -> LocalBoxFuture<'static, VaultRead> {
        Box::pin(Rc::clone(&self.inner).load(origin.to_string()))
    }

    fn save(&self, origin: &str, token: Option<String>) -> LocalBoxFuture<'static, bool> {
        Box::pin(Rc::clone(&self.inner).save(origin.to_string(), token))
    }

    fn persistent(&self) -> bool {
        self.inner.persistent.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_item_is_keyed_by_origin_and_labelled_by_host() {
        let keys = attributes("https://chat.example.com");
        assert_eq!(keys.get("server"), Some(&"https://chat.example.com"));
        assert_eq!(keys.get("account"), Some(&"session"));
        assert_eq!(keys.len(), 2);
        assert_eq!(
            label_for("https://chat.example.com"),
            "Family Connect (chat.example.com)"
        );
        assert_eq!(
            label_for("http://192.168.1.10:8080"),
            "Family Connect (192.168.1.10:8080)"
        );
    }

    #[test]
    fn a_vault_is_persistent_until_a_service_fails_it() {
        let vault = SecretVault::new();
        assert!(vault.persistent());
        vault.inner.failed(
            "read",
            &glib::Error::new(gio::DBusError::ServiceUnknown, "x"),
        );
        assert!(!vault.persistent());
    }
}
