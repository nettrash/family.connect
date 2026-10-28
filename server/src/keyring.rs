//! The data keys at run time (#59): unwrapped once, kept in memory, found by
//! the family, conversation or account a value belongs to.
//!
//! `vault.rs` is the cryptography over bytes; this is where it meets the
//! database. Every key in `storage_keys` is unwrapped at start ([`Keyring::start`])
//! and every key made since is added as it is made, so OPENING a value needs no
//! query: the value names its key in its first bytes, and the key is already
//! here. SEALING needs the owner's key, which is one small query the first
//! time an owner is asked for and none after.
//!
//! The owner row names its key (`families.key_id`, `chats.key_id`,
//! `users.key_id`, migration 0059), and a family chat names its family's. A key
//! is made in the same transaction as its owner ([`Keyring::create`]), so an
//! owner never exists without one; a key never moves to another owner, and only
//! an account's is ever replaced (when it is scrubbed), so remembering which
//! key an owner has is safe for as long as the server runs.
//!
//! Nothing here logs or returns content. A [`KeyError`] names a key id, a
//! column or an owner — never a byte of what failed to open.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, RwLock};

use anyhow::{Context, bail};
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, PgExecutor, Row};
use tracing::info;

use crate::vault::{self, DataKey, Lookup, OpenError, ServerKey};

/// What a key belongs to: what the server deletes, and its key with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Owner {
    /// A family: its name and code, its chat, its board, its pack.
    Family(i64),
    /// A chat. A family chat answers with its family's key; a group, a direct
    /// chat or an assistant chat with its own.
    Chat(i64),
    /// An account: who somebody is, and how the server reaches them.
    Account(i64),
}

impl fmt::Display for Owner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Owner::Family(id) => write!(f, "family {id}"),
            Owner::Chat(id) => write!(f, "chat {id}"),
            Owner::Account(id) => write!(f, "account {id}"),
        }
    }
}

/// Why a value could not be sealed or opened. Says which key, column or owner
/// — never anything about the content.
#[derive(Debug)]
pub enum KeyError {
    /// A value that would not open under the key it names.
    Open {
        column: Option<String>,
        key_id: Option<i64>,
        error: OpenError,
    },
    /// A value naming a key this server does not hold: deleted with its
    /// owner, or never restored.
    UnknownKey {
        column: Option<String>,
        key_id: i64,
    },
    /// A value that opened but is not the UTF-8 text it should be.
    NotText {
        column: Option<String>,
        key_id: i64,
    },
    /// A value that opened but is not in the form its column holds (a JSON
    /// list that does not parse, say).
    Malformed {
        column: Option<String>,
        key_id: i64,
    },
    /// An owner with no key — or no owner at all.
    NoKey(Owner),
    /// A wrapped key that would not unwrap under this server key.
    Unwrap {
        key_id: i64,
        wrapped_by: String,
    },
    Db(sqlx::Error),
}

impl KeyError {
    /// The same error, saying which column it came from.
    pub fn in_column(self, name: &str) -> Self {
        let name = Some(name.to_string());
        match self {
            KeyError::Open { key_id, error, .. } => KeyError::Open {
                column: name,
                key_id,
                error,
            },
            KeyError::UnknownKey { key_id, .. } => KeyError::UnknownKey {
                column: name,
                key_id,
            },
            KeyError::NotText { key_id, .. } => KeyError::NotText {
                column: name,
                key_id,
            },
            KeyError::Malformed { key_id, .. } => KeyError::Malformed {
                column: name,
                key_id,
            },
            other => other,
        }
    }
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let column = |column: &Option<String>| match column {
            Some(name) => format!(" in column {name}"),
            None => String::new(),
        };
        match self {
            KeyError::Open {
                column: c,
                key_id: Some(key_id),
                error,
            } => write!(f, "a value{} under key {key_id}: {error}", column(c)),
            KeyError::Open {
                column: c,
                key_id: None,
                error,
            } => write!(f, "a value{}: {error}", column(c)),
            KeyError::UnknownKey { column: c, key_id } => write!(
                f,
                "a value{} names key {key_id}, which this server does not hold",
                column(c)
            ),
            KeyError::NotText { column: c, key_id } => {
                write!(f, "a value{} under key {key_id} is not text", column(c))
            }
            KeyError::Malformed { column: c, key_id } => write!(
                f,
                "a value{} under key {key_id} is not in its column's form",
                column(c)
            ),
            KeyError::NoKey(owner) => write!(f, "{owner} has no storage key"),
            KeyError::Unwrap { key_id, wrapped_by } => write!(
                f,
                "storage key {key_id} does not unwrap with this key file \
                 (it was wrapped by server key {wrapped_by})"
            ),
            KeyError::Db(err) => write!(f, "reading a storage key: {err}"),
        }
    }
}

impl std::error::Error for KeyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            KeyError::Db(err) => Some(err),
            _ => None,
        }
    }
}

impl From<sqlx::Error> for KeyError {
    fn from(err: sqlx::Error) -> Self {
        KeyError::Db(err)
    }
}

/// One owner's key, ready to seal with.
#[derive(Clone)]
pub struct SealKey {
    id: i64,
    key: Arc<DataKey>,
}

impl fmt::Debug for SealKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SealKey({})", self.id)
    }
}

impl SealKey {
    /// The key's id: what an owner row's `key_id` holds.
    pub fn id(&self) -> i64 {
        self.id
    }

    /// Seal bytes.
    pub fn seal(&self, plaintext: &[u8]) -> Vec<u8> {
        self.key.seal(self.id, plaintext)
    }

    /// Seal text.
    pub fn seal_str(&self, text: &str) -> Vec<u8> {
        self.seal(text.as_bytes())
    }

    /// Seal text that may be absent; absent stays NULL.
    pub fn seal_opt(&self, text: Option<&str>) -> Option<Vec<u8>> {
        text.map(|text| self.seal_str(text))
    }

    /// Seal a value as JSON — a list of greeting places, say.
    pub fn seal_json<T: serde::Serialize + ?Sized>(&self, value: &T) -> Vec<u8> {
        self.seal(&serde_json::to_vec(value).expect("a plain value always serializes"))
    }

    /// The dedup column's value for a file whose plaintext SHA-256 is `sha256`.
    pub fn content_hash(&self, sha256: &[u8]) -> String {
        self.key.content_hash(sha256)
    }
}

/// Every data key this server holds, unwrapped, and which owner has which.
pub struct Keyring {
    server: ServerKey,
    keys: RwLock<HashMap<i64, Arc<DataKey>>>,
    owners: RwLock<HashMap<Owner, i64>>,
}

impl fmt::Debug for Keyring {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Keyring({})", self.server.fingerprint())
    }
}

impl Keyring {
    /// An empty keyring for this server key. [`Keyring::start`] fills it.
    pub fn new(server: ServerKey) -> Self {
        Self {
            server,
            keys: RwLock::new(HashMap::new()),
            owners: RwLock::new(HashMap::new()),
        }
    }

    /// The server key's fingerprint, for logs and refusals.
    pub fn fingerprint(&self) -> &str {
        self.server.fingerprint()
    }

    /// The lookup column's value for `value` (see [`Lookup`]).
    pub fn lookup(&self, what: Lookup, value: &str) -> Vec<u8> {
        self.server.lookup(what, value)
    }

    fn cached(&self, key_id: i64) -> Option<Arc<DataKey>> {
        self.keys
            .read()
            .expect("the keyring lock is never poisoned")
            .get(&key_id)
            .cloned()
    }

    fn insert(&self, key_id: i64, key: DataKey) -> Arc<DataKey> {
        let key = Arc::new(key);
        self.keys
            .write()
            .expect("the keyring lock is never poisoned")
            .insert(key_id, Arc::clone(&key));
        key
    }

    fn unwrap_row(
        &self,
        key_id: i64,
        wrapped: &[u8],
        wrapped_by: &str,
    ) -> Result<DataKey, KeyError> {
        self.server
            .unwrap(key_id, wrapped)
            .map_err(|_| KeyError::Unwrap {
                key_id,
                wrapped_by: wrapped_by.to_string(),
            })
    }

    /// Unwrap every key in `storage_keys`. Refuses — before anything is sealed
    /// or served — when any of them was wrapped by a different server key: the
    /// wrong key file, or a key copy from before a rotation.
    pub async fn load(&self, db: &mut PgConnection) -> anyhow::Result<usize> {
        let mine = self.fingerprint().to_string();
        let foreign: Vec<(String, i64)> = sqlx::query_as(
            "SELECT wrapped_by, count(*) FROM storage_keys
              WHERE wrapped_by <> $1 GROUP BY wrapped_by ORDER BY count(*) DESC",
        )
        .bind(&mine)
        .fetch_all(&mut *db)
        .await
        .context("reading storage_keys")?;
        if !foreign.is_empty() {
            let named = foreign
                .iter()
                .map(|(fingerprint, count)| format!("{count} by server key {fingerprint}"))
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "the storage keys in this database were wrapped by another key file ({named}); \
                 the key file configured in [storage] key_file is server key {mine}. Start with \
                 the key file whose fingerprint is named — after a rotation, the key copies \
                 from before it need the old key (docs/operations.md, \"Rotating the key\")"
            );
        }
        let rows = sqlx::query("SELECT id, wrapped, wrapped_by FROM storage_keys")
            .fetch_all(&mut *db)
            .await
            .context("reading storage_keys")?;
        let mut keys = HashMap::with_capacity(rows.len());
        for row in &rows {
            let key_id: i64 = row.get("id");
            let wrapped: Vec<u8> = row.get("wrapped");
            let wrapped_by: String = row.get("wrapped_by");
            let key = self.unwrap_row(key_id, &wrapped, &wrapped_by)?;
            keys.insert(key_id, Arc::new(key));
        }
        let count = keys.len();
        *self
            .keys
            .write()
            .expect("the keyring lock is never poisoned") = keys;
        Ok(count)
    }

    /// The keyring's start, once the migrations have run: unwrap every key,
    /// refuse to serve when an owner has lost its key, and drop the keys whose
    /// owner is gone.
    ///
    /// A lost key is what restoring data with a key copy from the wrong day
    /// looks like (docs/operations.md, "Restoring"). The refusal counts what
    /// is affected and names the two ways out; it never deletes anything
    /// itself, because one of the ways out is a newer key copy.
    pub async fn start(&self, db: &mut PgConnection) -> anyhow::Result<()> {
        let loaded = self.load(&mut *db).await?;
        let (families, chats, accounts): (i64, i64, i64) = sqlx::query_as(
            "SELECT
                (SELECT count(*) FROM families o
                  WHERE NOT EXISTS (SELECT 1 FROM storage_keys k WHERE k.id = o.key_id)),
                (SELECT count(*) FROM chats o
                  WHERE NOT EXISTS (SELECT 1 FROM storage_keys k WHERE k.id = o.key_id)),
                (SELECT count(*) FROM users o
                  WHERE NOT EXISTS (SELECT 1 FROM storage_keys k WHERE k.id = o.key_id))",
        )
        .fetch_one(&mut *db)
        .await
        .context("checking that every owner has its storage key")?;
        if families + chats + accounts > 0 {
            bail!(
                "{families} families, {chats} conversations and {accounts} accounts have no \
                 storage key. After a restore this means the key copy is from a different day \
                 than the database. A key copy OLDER than the database misses the newest \
                 families: restore a newer one. A key copy NEWER than the database is missing \
                 what was deleted since: run `family-connect storage forget-missing` to delete \
                 it again (docs/operations.md, \"Restoring\")"
            );
        }
        let dropped = sqlx::query(
            "DELETE FROM storage_keys k
              WHERE NOT EXISTS (SELECT 1 FROM families o WHERE o.key_id = k.id)
                AND NOT EXISTS (SELECT 1 FROM chats o WHERE o.key_id = k.id)
                AND NOT EXISTS (SELECT 1 FROM users o WHERE o.key_id = k.id)",
        )
        .execute(&mut *db)
        .await
        .context("dropping storage keys whose owner is gone")?
        .rows_affected();
        info!(
            server_key = self.fingerprint(),
            keys = loaded as u64 - dropped,
            dropped,
            "storage keys loaded"
        );
        Ok(())
    }

    /// A new key, stored wrapped, for an owner this transaction is about to
    /// create. The owner row's `key_id` is set to [`SealKey::id`]; call
    /// [`Keyring::remember`] once the owner's id is known.
    pub async fn create(&self, db: &mut PgConnection) -> Result<SealKey, KeyError> {
        let key = DataKey::generate();
        let key_id = loop {
            let id = rand::random::<i64>() & i64::MAX;
            if id != 0 && self.cached(id).is_none() {
                break id;
            }
        };
        sqlx::query("INSERT INTO storage_keys (id, wrapped, wrapped_by) VALUES ($1, $2, $3)")
            .bind(key_id)
            .bind(self.server.wrap(key_id, &key))
            .bind(self.fingerprint())
            .execute(&mut *db)
            .await?;
        Ok(SealKey {
            id: key_id,
            key: self.insert(key_id, key),
        })
    }

    /// Delete a key [`Keyring::create`] made for an owner that turned out to
    /// exist already — an insert that met its conflict — in the same
    /// transaction, so no key is left that nothing names.
    pub async fn discard(&self, db: &mut PgConnection, key: SealKey) -> Result<(), KeyError> {
        sqlx::query("DELETE FROM storage_keys WHERE id = $1")
            .bind(key.id)
            .execute(&mut *db)
            .await?;
        self.keys
            .write()
            .expect("the keyring lock is never poisoned")
            .remove(&key.id);
        Ok(())
    }

    /// Wrap every key this keyring holds under `new` instead — the database
    /// half of `family-connect storage-key rotate`. Run after [`Keyring::load`],
    /// inside the caller's transaction, so the whole table moves at once.
    /// Answers how many keys were re-wrapped.
    pub async fn rewrap(&self, db: &mut PgConnection, new: &ServerKey) -> anyhow::Result<u64> {
        let keys: Vec<(i64, Arc<DataKey>)> = self
            .keys
            .read()
            .expect("the keyring lock is never poisoned")
            .iter()
            .map(|(id, key)| (*id, Arc::clone(key)))
            .collect();
        for (key_id, key) in &keys {
            sqlx::query("UPDATE storage_keys SET wrapped = $2, wrapped_by = $3 WHERE id = $1")
                .bind(key_id)
                .bind(new.wrap(*key_id, key))
                .bind(new.fingerprint())
                .execute(&mut *db)
                .await
                .context("re-wrapping a storage key")?;
        }
        Ok(keys.len() as u64)
    }

    /// Remember which key an owner has, so the next [`Keyring::key`] for it
    /// asks nothing.
    pub fn remember(&self, owner: Owner, key: &SealKey) {
        self.owners
            .write()
            .expect("the keyring lock is never poisoned")
            .insert(owner, key.id);
    }

    /// Forget which key an owner has — an account whose key was replaced, or
    /// an owner that is gone — and the key itself, when it was deleted.
    pub fn forget(&self, owner: Owner, deleted_key: Option<i64>) {
        self.owners
            .write()
            .expect("the keyring lock is never poisoned")
            .remove(&owner);
        if let Some(key_id) = deleted_key {
            self.keys
                .write()
                .expect("the keyring lock is never poisoned")
                .remove(&key_id);
        }
    }

    /// The key to seal an owner's values with.
    pub async fn key<'e>(
        &self,
        db: impl PgExecutor<'e>,
        owner: Owner,
    ) -> Result<SealKey, KeyError> {
        let known = self
            .owners
            .read()
            .expect("the keyring lock is never poisoned")
            .get(&owner)
            .copied();
        if let Some(key_id) = known
            && let Some(key) = self.cached(key_id)
        {
            return Ok(SealKey { id: key_id, key });
        }
        let sql = match owner {
            Owner::Family(_) => {
                "SELECT k.id, k.wrapped, k.wrapped_by FROM families o
                   JOIN storage_keys k ON k.id = o.key_id WHERE o.id = $1"
            }
            Owner::Chat(_) => {
                "SELECT k.id, k.wrapped, k.wrapped_by FROM chats o
                   JOIN storage_keys k ON k.id = o.key_id WHERE o.id = $1"
            }
            Owner::Account(_) => {
                "SELECT k.id, k.wrapped, k.wrapped_by FROM users o
                   JOIN storage_keys k ON k.id = o.key_id WHERE o.id = $1"
            }
        };
        let id = match owner {
            Owner::Family(id) | Owner::Chat(id) | Owner::Account(id) => id,
        };
        let row = sqlx::query(sql)
            .bind(id)
            .fetch_optional(db)
            .await?
            .ok_or(KeyError::NoKey(owner))?;
        let key_id: i64 = row.get("id");
        let key = match self.cached(key_id) {
            Some(key) => key,
            None => {
                let wrapped: Vec<u8> = row.get("wrapped");
                let wrapped_by: String = row.get("wrapped_by");
                let key = self.unwrap_row(key_id, &wrapped, &wrapped_by)?;
                self.insert(key_id, key)
            }
        };
        let key = SealKey { id: key_id, key };
        self.remember(owner, &key);
        Ok(key)
    }

    /// A key by its id, when the owner's `key_id` is already in hand.
    pub fn by_id(&self, key_id: i64) -> Result<SealKey, KeyError> {
        let key = self.cached(key_id).ok_or(KeyError::UnknownKey {
            column: None,
            key_id,
        })?;
        Ok(SealKey { id: key_id, key })
    }

    /// Open a sealed value.
    pub fn open(&self, sealed: &[u8]) -> Result<Vec<u8>, KeyError> {
        let key_id = vault::key_id_of(sealed).map_err(|error| KeyError::Open {
            column: None,
            key_id: None,
            error,
        })?;
        let key = self.cached(key_id).ok_or(KeyError::UnknownKey {
            column: None,
            key_id,
        })?;
        key.open(sealed).map_err(|error| KeyError::Open {
            column: None,
            key_id: Some(key_id),
            error,
        })
    }

    /// Open a sealed value that holds text.
    pub fn open_text(&self, sealed: &[u8]) -> Result<String, KeyError> {
        let plain = self.open(sealed)?;
        String::from_utf8(plain).map_err(|_| KeyError::NotText {
            column: None,
            key_id: vault::key_id_of(sealed).unwrap_or_default(),
        })
    }
}

/// Reading sealed columns straight off a row.
pub trait SealedRow {
    /// A sealed text column.
    fn text(&self, keys: &Keyring, column: &str) -> Result<String, KeyError>;
    /// A sealed text column that may be NULL.
    fn text_opt(&self, keys: &Keyring, column: &str) -> Result<Option<String>, KeyError>;
    /// A sealed column of bytes.
    fn opened(&self, keys: &Keyring, column: &str) -> Result<Vec<u8>, KeyError>;
    /// A sealed column of bytes that may be NULL.
    fn opened_opt(&self, keys: &Keyring, column: &str) -> Result<Option<Vec<u8>>, KeyError>;
    /// A sealed column holding JSON.
    fn json<T: serde::de::DeserializeOwned>(
        &self,
        keys: &Keyring,
        column: &str,
    ) -> Result<T, KeyError>;
    /// A sealed column that a narrow SELECT may not have asked for at all:
    /// absent from the row and NULL both read as `None`. Only absence is
    /// forgiven — a column that is there and will not open is an error.
    fn opened_if_selected(&self, keys: &Keyring, column: &str)
    -> Result<Option<Vec<u8>>, KeyError>;
}

impl SealedRow for PgRow {
    fn text(&self, keys: &Keyring, column: &str) -> Result<String, KeyError> {
        let sealed: Vec<u8> = self.try_get(column)?;
        keys.open_text(&sealed).map_err(|err| err.in_column(column))
    }

    fn text_opt(&self, keys: &Keyring, column: &str) -> Result<Option<String>, KeyError> {
        let sealed: Option<Vec<u8>> = self.try_get(column)?;
        sealed
            .map(|sealed| keys.open_text(&sealed).map_err(|err| err.in_column(column)))
            .transpose()
    }

    fn opened(&self, keys: &Keyring, column: &str) -> Result<Vec<u8>, KeyError> {
        let sealed: Vec<u8> = self.try_get(column)?;
        keys.open(&sealed).map_err(|err| err.in_column(column))
    }

    fn opened_opt(&self, keys: &Keyring, column: &str) -> Result<Option<Vec<u8>>, KeyError> {
        let sealed: Option<Vec<u8>> = self.try_get(column)?;
        sealed
            .map(|sealed| keys.open(&sealed).map_err(|err| err.in_column(column)))
            .transpose()
    }

    fn opened_if_selected(
        &self,
        keys: &Keyring,
        column: &str,
    ) -> Result<Option<Vec<u8>>, KeyError> {
        match self.try_get::<Option<Vec<u8>>, _>(column) {
            Ok(Some(sealed)) => keys
                .open(&sealed)
                .map(Some)
                .map_err(|err| err.in_column(column)),
            Ok(None) | Err(sqlx::Error::ColumnNotFound(_)) => Ok(None),
            Err(err) => Err(KeyError::Db(err)),
        }
    }

    fn json<T: serde::de::DeserializeOwned>(
        &self,
        keys: &Keyring,
        column: &str,
    ) -> Result<T, KeyError> {
        let sealed: Vec<u8> = self.try_get(column)?;
        let plain = keys.open(&sealed).map_err(|err| err.in_column(column))?;
        serde_json::from_slice(&plain).map_err(|_| KeyError::Malformed {
            column: Some(column.to_string()),
            key_id: vault::key_id_of(&sealed).unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyring_with(key_id: i64) -> (Keyring, SealKey) {
        let keyring = Keyring::new(ServerKey::generate());
        let key = SealKey {
            id: key_id,
            key: keyring.insert(key_id, DataKey::generate()),
        };
        (keyring, key)
    }

    #[test]
    fn a_value_opens_by_the_key_it_names() {
        let (keyring, key) = keyring_with(77);
        let sealed = key.seal_str("Grandma's soup");
        assert_eq!(vault::key_id_of(&sealed).unwrap(), 77);
        assert_eq!(keyring.open_text(&sealed).unwrap(), "Grandma's soup");
    }

    #[test]
    fn a_value_under_a_key_the_server_does_not_hold_is_an_error_naming_the_key() {
        let (keyring, _) = keyring_with(77);
        let (_, other) = keyring_with(78);
        let err = keyring
            .open(&other.seal_str("secret words"))
            .unwrap_err()
            .in_column("body");
        let text = err.to_string();
        assert!(matches!(err, KeyError::UnknownKey { key_id: 78, .. }));
        assert!(text.contains("78") && text.contains("body"), "{text}");
        assert!(!text.contains("secret"), "{text}");
    }

    #[test]
    fn a_forgotten_key_no_longer_opens_what_it_sealed() {
        let (keyring, key) = keyring_with(77);
        let sealed = key.seal_str("words");
        keyring.forget(Owner::Family(1), Some(77));
        assert!(matches!(
            keyring.open(&sealed),
            Err(KeyError::UnknownKey { key_id: 77, .. })
        ));
    }

    #[test]
    fn bytes_that_are_not_text_are_refused_as_text_but_open_as_bytes() {
        let (keyring, key) = keyring_with(77);
        let sealed = key.seal(&[0xff, 0xfe]);
        assert_eq!(keyring.open(&sealed).unwrap(), vec![0xff, 0xfe]);
        assert!(matches!(
            keyring.open_text(&sealed),
            Err(KeyError::NotText { key_id: 77, .. })
        ));
    }

    #[test]
    fn plaintext_left_by_the_upgrade_is_refused_not_shown() {
        let (keyring, _) = keyring_with(77);
        let mut legacy = vec![vault::NOT_YET_SEALED];
        legacy.extend_from_slice(b"hello");
        assert!(matches!(
            keyring.open(&legacy),
            Err(KeyError::Open {
                error: OpenError::NotSealed,
                ..
            })
        ));
    }

    #[test]
    fn the_debug_form_shows_neither_key() {
        let (keyring, key) = keyring_with(77);
        assert_eq!(format!("{key:?}"), "SealKey(77)");
        assert!(format!("{keyring:?}").starts_with("Keyring("));
    }
}
