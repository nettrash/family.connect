//! Family Connect for Linux — the wire, the local cache, sync, the outbox and the board's rules: a
//! port of `win/src/FamilyConnect.Core` (see `linux/docs/port-plan.md`). It runs on one thread —
//! the GLib main context in the app, a `LocalPool` in the tests — and reaches the platform only
//! through the seams in [`rt`].

pub mod board;
pub mod protocol;
pub mod rt;
pub mod store;
pub mod testing;
pub mod text;
