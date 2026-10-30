//! The local cache: one SQLite file, opened once and migrated forward by number — a port of
//! `Database.cs`.
//!
//! WHY A CACHE AT ALL. The wire is authoritative and this file is a copy — every read replaces
//! what it holds, and nothing here is a second source of truth. What it buys is a window that
//! opens with the family's chat already in it rather than a spinner, and an outbox that survives
//! the app being closed mid-send.
//!
//! MIGRATIONS ARE NUMBERED AND THERE IS NO DESTRUCTIVE FALLBACK. Android's board shipped a column
//! without a migration once and every upgraded install would have crashed on the next launch
//! (issue #70); the only thing that catches a forgotten column is comparing a migrated schema
//! against a fresh one, which `tests/store_database.rs` does. `user_version` is the schema's
//! number, which is what SQLite gives us for free and what Room uses too.
//!
//! The SQLite is the SYSTEM's (`rusqlite` without `bundled`): Ubuntu's `libsqlite3`.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::Path;
use std::rc::Rc;

use rusqlite::{Connection, OpenFlags, named_params};

use super::{board_store, family_keys, migrations, pack_store};
use crate::protocol::wire;

/// Why the cache could not do what was asked.
#[derive(Debug)]
pub enum StoreError {
    /// SQLite refused: the disk, the file, a constraint. The C# lets the same exception through.
    Sqlite(rusqlite::Error),
    /// A file written by a NEWER build (the C#'s `InvalidOperationException`). Refusing is the
    /// honest answer: the alternative is an older client writing rows a newer schema will misread,
    /// and nothing here may delete a family's outbox to make itself comfortable.
    NewerSchema { found: i32, known: usize },
    /// A file whose `user_version` is below 0, which no build writes. The C# indexes its step list
    /// with it and throws `IndexOutOfRangeException` (measured on .NET 10); this says what happened
    /// instead.
    UnknownSchema { found: i32 },
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Sqlite(error) => write!(f, "the cache: {error}"),
            StoreError::NewerSchema { found, known } => write!(
                f,
                "this cache was written by a newer version (schema {found}, this build knows {known})"
            ),
            StoreError::UnknownSchema { found } => {
                write!(
                    f,
                    "this cache carries schema {found}, which no build writes"
                )
            }
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StoreError::Sqlite(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        StoreError::Sqlite(error)
    }
}

/// What every store operation answers.
pub type StoreResult<T> = Result<T, StoreError>;

/// The cache. Opened as an `Rc`, because every store holds it — several stores sit on one file, as
/// the C#'s do — and an `Rc` cannot leave the thread that opened it.
pub struct Database {
    connection: Connection,
    /// How many times an operation has held the cache ([`Database::hold`]).
    holds: Cell<u64>,
    /// How deep the hold is now: 0 between operations.
    depth: Cell<u32>,
    roster_generation: Cell<i64>,
    chats_generation: Cell<i64>,
    chat_touches: Cell<i64>,
    touched_at: RefCell<HashMap<i64, i64>>,
}

impl fmt::Debug for Database {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Database")
            .field("holds", &self.holds.get())
            .field("depth", &self.depth.get())
            .finish_non_exhaustive()
    }
}

/// The cache, held for one operation. Dropping it lets the operation go.
#[must_use = "an operation holds the cache for as long as this lives"]
pub struct Held<'a> {
    database: &'a Database,
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        self.database.depth.set(self.database.depth.get() - 1);
    }
}

impl Database {
    /// The schema this build expects.
    pub fn schema_version() -> i32 {
        // Eleven steps; a count of steps that does not fit an i32 is not a schema.
        i32::try_from(migrations::ALL.len()).unwrap_or(i32::MAX)
    }

    /// Open (or create) the cache at `path` and migrate it forward. The file is created on first
    /// run; a missing directory is the caller's problem.
    pub fn open(path: impl AsRef<Path>) -> StoreResult<Rc<Database>> {
        // One connection, one cache: this client has a single writer and no reason to share (the
        // C#'s `ReadWriteCreate`, private cache and no pooling). No URI flag: a path is a path.
        Database::begin(Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_PRIVATE_CACHE,
        )?)
    }

    /// A cache that lives only as long as the value — for tests.
    pub fn open_in_memory() -> StoreResult<Rc<Database>> {
        Database::begin(Connection::open_in_memory()?)
    }

    fn begin(connection: Connection) -> StoreResult<Rc<Database>> {
        Database::configure(&connection)?;
        let database = Database {
            connection,
            holds: Cell::new(0),
            depth: Cell::new(0),
            roster_generation: Cell::new(0),
            chats_generation: Cell::new(0),
            chat_touches: Cell::new(0),
            touched_at: RefCell::new(HashMap::new()),
        };
        database.migrate()?;
        Ok(Rc::new(database))
    }

    fn configure(connection: &Connection) -> StoreResult<()> {
        // FOREIGN KEYS ARE OFF BY DEFAULT IN SQLITE, per connection, and this schema leans on
        // them: a chat's messages and a note's rows go with it, and a cascade that silently did
        // nothing would leave rows nobody can reach.
        connection.execute_batch("PRAGMA foreign_keys = ON")?;
        // A crash mid-write must not cost the whole cache. WAL also lets a read proceed while the
        // sync writes, which is most of what this file does.
        connection.execute_batch("PRAGMA journal_mode = WAL")?;
        connection.execute_batch("PRAGMA synchronous = NORMAL")?;
        connection.execute_batch("PRAGMA busy_timeout = 5000")?;
        // Every store statement is prepared once and kept: a conversation asks the same few
        // questions once per bubble.
        connection.set_prepared_statement_cache_capacity(128);
        Ok(())
    }

    /// The one connection. SQLite is fine with one writer, and this client has one.
    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// One text kept in `meta` under `key`, or `None` — wiped with everything else at a sign-out.
    pub fn meta_text(&self, key: &str) -> StoreResult<Option<String>> {
        board_store::meta(self, key)
    }

    /// Keeps `value` in `meta` under `key`, replacing what was there.
    pub fn set_meta_text(&self, key: &str, value: &str) -> StoreResult<()> {
        let _held = self.hold();
        self.connection
            .prepare_cached(
                "INSERT INTO meta (key, value) VALUES ($key, $value) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            )?
            .execute(named_params! { "$key": key, "$value": value })?;
        Ok(())
    }

    /// Whether the cache holds any chat — what an offline launch can open.
    pub fn holds_chats(&self) -> StoreResult<bool> {
        let _held = self.hold();
        Ok(self
            .connection
            .prepare_cached("SELECT EXISTS (SELECT 1 FROM chats)")?
            .query_row([], |row| row.get::<_, bool>(0))?)
    }

    /// Hold the cache for ONE WHOLE OPERATION: every store method takes this first.
    ///
    /// IN THE C# THIS IS A LOCK, because there the socket applies frames on its own thread, the
    /// resync applies pages on the thread pool and the window reads on the UI thread — all through
    /// one connection, where a read that lands inside another thread's transaction throws, and CI
    /// caught exactly that on both runners. PER OPERATION, NOT PER COMMAND, because a transaction
    /// spans several commands and a read slipped between two of them is the failure.
    ///
    /// HERE IT WAITS FOR NOTHING, because nothing can be in flight beside it. Everything runs on
    /// ONE thread (`linux/docs/port-plan.md`): the cache is `!Sync` and every store holds it
    /// through an `Rc`, so no second thread can reach it, and every store operation is a plain
    /// synchronous `fn` that never yields — so on that one thread no other operation can begin
    /// until it has returned, transaction and all. That is the C#'s "every named operation
    /// WAITS", given by the types instead of a monitor. The hold is kept as the same per-operation
    /// mark — re-entrant, so an operation that calls another (the chat list asking for each chat's
    /// newest message) simply holds it deeper — and counted ([`Database::holds`]), so the
    /// concurrency suite can still insist that every named operation takes it.
    pub fn hold(&self) -> Held<'_> {
        self.holds.set(self.holds.get() + 1);
        self.depth.set(self.depth.get() + 1);
        Held { database: self }
    }

    /// How many times an operation has held the cache since it opened — what the concurrency
    /// suite counts.
    pub fn holds(&self) -> u64 {
        self.holds.get()
    }

    /// Whether an operation holds the cache right now.
    pub fn is_held(&self) -> bool {
        self.depth.get() > 0
    }

    // ---- what a store keeps in memory, and how it knows it is stale -------------------------

    /// How many times the roster or the block list may have changed — the members and blocked
    /// tables. Read and bumped inside an operation only.
    ///
    /// WHAT LETS A STORE KEEP THEM IN MEMORY. A conversation asks "is this sender blocked?" and
    /// "who is this?" for every bubble it draws, and each of those was a query — hundreds of them
    /// per redraw. They are tiny tables that change a few times a day, so [`ChatStore`] keeps them
    /// in memory and asks this number whether what it keeps is still the table.
    ///
    /// ON THE DATABASE, NOT THE STORE, because every writer has to bump it and more than one store
    /// can sit on one file (the tests make several): a counter in each store would let one store's
    /// write leave another's copy stale. And [`Database::wipe_all`] bumps it too, which no store
    /// sees happen.
    ///
    /// [`ChatStore`]: super::ChatStore
    pub(crate) fn roster_generation(&self) -> i64 {
        self.roster_generation.get()
    }

    /// The members or the blocked table is about to change: every copy kept of them is stale.
    pub(crate) fn roster_changed(&self) {
        self.roster_generation.set(self.roster_generation.get() + 1);
    }

    /// The same for the chats table's rows — what `ChatStore::kind` keeps. Bumped by every write
    /// of a chat's list row and by [`Database::wipe_all`].
    pub(crate) fn chats_generation(&self) -> i64 {
        self.chats_generation.get()
    }

    /// A chat's list row is about to be written.
    pub(crate) fn chats_changed(&self) {
        self.chats_generation.set(self.chats_generation.get() + 1);
    }

    /// How many times a chat has been TOUCHED outside a list read — upserted by `chat_updated` or
    /// a group endpoint's answer, or removed — and when each was last (see `ChatStore::list_mark`).
    /// On the database, like the generations, so a second store on the same file sees them.
    pub(crate) fn chat_touches(&self) -> i64 {
        self.chat_touches.get()
    }

    /// This chat changed outside a list read, as of now.
    pub(crate) fn touched(&self, chat_id: i64) {
        let touch = self.chat_touches.get() + 1;
        self.chat_touches.set(touch);
        self.touched_at.borrow_mut().insert(chat_id, touch);
    }

    /// The chats touched after `mark`.
    pub(crate) fn touched_since(&self, mark: i64) -> HashSet<i64> {
        self.touched_at
            .borrow()
            .iter()
            .filter(|&(_, &touch)| touch > mark)
            .map(|(&chat, _)| chat)
            .collect()
    }

    // ---- the schema ---------------------------------------------------------------------------

    /// Run every migration this build knows and the file has not had, in order, each in its own
    /// transaction with the version bumped inside it — so an interrupted upgrade is either done or
    /// not done, never half.
    pub fn migrate(&self) -> StoreResult<()> {
        let _held = self.hold();
        let from = self.user_version()?;
        let known = migrations::ALL.len();
        let Ok(start) = usize::try_from(from) else {
            return Err(StoreError::UnknownSchema { found: from });
        };
        if start > known {
            // A file written by a NEWER build. Refusing is the honest answer: the alternative is
            // an older client writing rows a newer schema will misread, and nothing here may
            // delete a family's outbox to make itself comfortable.
            return Err(StoreError::NewerSchema { found: from, known });
        }
        for (version, step) in migrations::ALL.iter().enumerate().skip(start) {
            let transaction = self.connection.unchecked_transaction()?;
            for statement in step.iter() {
                transaction.execute_batch(statement)?;
            }
            // PRAGMA takes no parameter binding; the value is a number this code computed.
            transaction.execute_batch(&format!("PRAGMA user_version = {}", version + 1))?;
            transaction.commit()?;
        }
        Ok(())
    }

    /// The schema number the file carries.
    pub fn user_version(&self) -> StoreResult<i32> {
        let _held = self.hold();
        Ok(self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))?)
    }

    /// Every table in the file, in name order — what a schema test compares.
    pub fn tables(&self) -> StoreResult<Vec<String>> {
        let _held = self.hold();
        let mut statement = self.connection.prepare_cached(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )?;
        let names = statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(names)
    }

    /// One table's columns as `name TYPE [NOT NULL]`, in declaration order — enough to catch the
    /// failure that matters: a column a fresh schema has and a migrated one does not.
    pub fn columns(&self, table: &str) -> StoreResult<Vec<String>> {
        let _held = self.hold();
        // `PRAGMA table_info` takes no parameters either; the name is checked against the file's
        // own table list first, so nothing user-supplied reaches the statement.
        if !self.tables()?.iter().any(|name| name == table) {
            return Ok(Vec::new());
        }
        let mut statement = self
            .connection
            .prepare(&format!("PRAGMA table_info({table})"))?;
        let columns = statement
            .query_map([], |row| {
                let name: String = row.get(1)?;
                let kind: String = row.get(2)?;
                let not_null: i64 = row.get(3)?;
                Ok(if not_null == 1 {
                    format!("{name} {kind} NOT NULL")
                } else {
                    format!("{name} {kind}")
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(columns)
    }

    // ---- what a sign-out and leaving a family take -----------------------------------------------

    /// Every row of every table, gone — the schema stays. What a sign-out, a session that expired
    /// and being removed from a family all do.
    ///
    /// THE OUTBOX GOES WITH IT, and deliberately: a queued message belongs to the account that
    /// wrote it, and a send that landed in the next person's family would be the worst bug this
    /// app could have. The tables are read from the file rather than listed here, so a table added
    /// later is wiped without anybody remembering to come back — which is exactly the mistake a
    /// list would eventually make.
    pub fn wipe_all(&self) -> StoreResult<()> {
        let _held = self.hold();
        // Whatever a store keeps in memory of these tables goes with them.
        self.roster_changed();
        self.chats_changed();
        self.touched_at.borrow_mut().clear();
        let transaction = self.connection.unchecked_transaction()?;
        for table in self.tables()? {
            transaction.execute(&format!("DELETE FROM \"{table}\""), [])?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// ONE FAMILY's rows, gone — and nothing else (docs/protocol.md, "Several families"): the
    /// account left that family, was removed from it, or the family went. Its board, its pack, its
    /// roster and every cursor and mark kept for it; its chats with their messages and queued
    /// sends — except a direct chat still listed in another family, which only stops naming this
    /// one (a direct chat belongs to the pair, and stays while they share another family).
    ///
    /// [`Database::wipe_all`] is still what a sign-out, an expired session and leaving the LAST
    /// family do. This is the narrower thing leaving one of several needs: the people stay on
    /// `members`, because a name still has to resolve for an old message in a chat that stays.
    ///
    /// Answers the chats that went, so a window showing one can close it.
    pub fn drop_family(&self, family_id: i64) -> StoreResult<Vec<i64>> {
        let _held = self.hold();
        self.roster_changed();
        self.chats_changed();
        let mut gone = Vec::new();
        let transaction = self.connection.unchecked_transaction()?;
        let mut listed: Vec<(i64, Vec<i64>)> = Vec::new();
        {
            let mut read = transaction.prepare_cached(
                "SELECT chat_id, family_ids_json FROM chats WHERE family_ids_json IS NOT NULL",
            )?;
            let mut rows = read.query([])?;
            while let Some(row) = rows.next()? {
                let families: String = row.get(1)?;
                if let Some(families) = wire::decode::<Vec<i64>>(&families)
                    && families.contains(&family_id)
                {
                    listed.push((row.get(0)?, families));
                }
            }
        }
        for (chat, families) in listed {
            self.touched(chat);
            let rest: Vec<i64> = families
                .into_iter()
                .filter(|&family| family != family_id)
                .collect();
            if !rest.is_empty() {
                transaction.execute(
                    "UPDATE chats SET family_ids_json = $families WHERE chat_id = $id",
                    named_params! { "$families": wire::encode(&rest), "$id": chat },
                )?;
                continue;
            }
            gone.push(chat);
            for statement in [
                "DELETE FROM outbox WHERE chat_id = $id",
                "DELETE FROM messages WHERE chat_id = $id",
                "DELETE FROM chats WHERE chat_id = $id",
            ] {
                transaction.execute(statement, named_params! { "$id": chat })?;
            }
        }
        // Bound as TEXT, as the C# binds it: the meta pattern needs the digits, and the two
        // INTEGER columns compare the text by their affinity.
        let family = family_id.to_string();
        for statement in [
            "DELETE FROM pack_recents WHERE item_id IN (SELECT item_id FROM pack_items WHERE family_id = $family)",
            "DELETE FROM pack_items WHERE family_id = $family",
            "DELETE FROM notes WHERE family_id = $family",
            "DELETE FROM family_members WHERE family_id = $family",
            // Its cursors, its marks and its ceilings: every key spelled for it (family_keys).
            "DELETE FROM meta WHERE key LIKE '%@' || $family",
        ] {
            transaction.execute(statement, named_params! { "$family": family })?;
        }
        transaction.commit()?;
        Ok(gone)
    }

    /// Every family this cache holds anything per family for — a roster, a note, a pack item — so
    /// a family the account left while this app was not running is found and dropped once
    /// `GET /me` no longer lists it.
    pub fn held_families(&self) -> StoreResult<Vec<i64>> {
        let _held = self.hold();
        let mut statement = self.connection.prepare_cached(
            "SELECT family_id FROM family_members
UNION SELECT family_id FROM notes WHERE family_id IS NOT NULL
UNION SELECT family_id FROM pack_items WHERE family_id IS NOT NULL
ORDER BY 1",
        )?;
        let ids = statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<i64>, _>>()?;
        Ok(ids)
    }

    /// What a cache from before several families held, made the HOME family's — the one family
    /// such a build ever saw — once `GET /me` names it: the notes and pack items migration 11 left
    /// without a family, the roster as that family's, and the board's and the pack's cursors, marks
    /// and ceilings under that family's keys. Idempotent: a second call finds nothing unadopted.
    pub fn adopt_legacy(&self, home_family_id: i64) -> StoreResult<()> {
        let _held = self.hold();
        self.roster_changed();
        let transaction = self.connection.unchecked_transaction()?;
        for statement in [
            "UPDATE notes SET family_id = $family WHERE family_id IS NULL",
            "UPDATE pack_items SET family_id = $family WHERE family_id IS NULL",
            // Only while no roster is kept per family at all: afterwards `members` is everybody,
            // of every family.
            "INSERT INTO family_members (family_id, user_id, role, has_left)
SELECT $family, user_id, role, MAX(has_left, deleted) FROM members
 WHERE NOT EXISTS (SELECT 1 FROM family_members)",
        ] {
            transaction.execute(statement, named_params! { "$family": home_family_id })?;
        }
        for key in board_store::META_KEYS
            .iter()
            .chain(pack_store::META_KEYS.iter())
        {
            // The family's own key wins if it is already there; the bare one goes either way.
            transaction.execute(
                "INSERT OR IGNORE INTO meta (key, value) SELECT $scoped, value FROM meta WHERE key = $bare",
                named_params! {
                    "$scoped": family_keys::of(key, Some(home_family_id)),
                    "$bare": *key,
                },
            )?;
            transaction.execute(
                "DELETE FROM meta WHERE key = $bare",
                named_params! { "$bare": *key },
            )?;
        }
        transaction.commit()?;
        Ok(())
    }
}
