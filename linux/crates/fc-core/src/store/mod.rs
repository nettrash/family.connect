//! The local cache: SQLite with numbered migrations, a port of `win/src/FamilyConnect.Core/Store`.
//!
//! One Rust file per C# file:
//!
//! | C# (`FamilyConnect.Core.Store`) | Rust |
//! | --- | --- |
//! | `Database` (and its `Held`) | [`database::Database`], [`database::Held`] |
//! | `Migrations.All` | [`migrations::ALL`] |
//! | `ChatStore`, `SeqRoute` | [`chat_store::ChatStore`], [`chat_store::SeqRoute`] |
//! | `BoardStore` | [`board_store::BoardStore`] |
//! | `OutboxStore`, `OutboxRow` | [`outbox_store::OutboxStore`], [`outbox_store::OutboxRow`] |
//! | `PackStore`, `PackLimits` | [`pack_store::PackStore`], [`pack_store::PackLimits`] |
//! | `PlayedStore`, `PlayedRoundStore`, `PlayedVoiceStore` | [`played_round_store`] |
//! | `TranscriptStore`, `KeptTranscript` | [`transcript_store::TranscriptStore`], [`transcript_store::KeptTranscript`] |
//! | `FamilyKeys` | `family_keys` (crate-private, as the C# is `internal`) |
//! | `Times` | [`times`] |
//!
//! Three things differ in shape, none in behaviour:
//!
//! - **Errors.** A C# store method throws whatever `Microsoft.Data.Sqlite` throws; here every
//!   operation that touches the file answers a [`StoreResult`], and a cache from a newer build is
//!   [`StoreError::NewerSchema`] rather than an `InvalidOperationException`.
//! - **Sharing.** A C# store holds a reference to its `Database`; here every store holds an
//!   `Rc<Database>` ([`Database::open`] answers one), so several stores sit on one file as the
//!   C#'s do — and none of them can leave the thread that opened it.
//! - **The lock.** See [`Database::hold`]: on one thread an operation cannot be overtaken, so the
//!   C#'s re-entrant monitor is kept as the same per-operation hold, which waits for nothing and
//!   is counted so a test can still insist every operation takes it.

pub mod board_store;
pub mod chat_store;
pub mod database;
pub(crate) mod family_keys;
pub mod migrations;
pub mod outbox_store;
pub mod pack_store;
pub mod played_round_store;
pub mod times;
pub mod transcript_store;

pub use board_store::BoardStore;
pub use chat_store::{ChatStore, SeqRoute};
pub use database::{Database, Held, StoreError, StoreResult};
pub use outbox_store::{OutboxRow, OutboxStore};
pub use pack_store::{PackLimits, PackStore};
pub use played_round_store::{PlayedRoundStore, PlayedStore, PlayedVoiceStore};
pub use transcript_store::{KeptTranscript, TranscriptStore};
