//! How a store that keeps one of something PER FAMILY spells its `meta` keys (docs/protocol.md,
//! "Several families") — a port of `FamilyKeys.cs`: the key a cache from before several families
//! used, with `@` and the family's id after it — `board.cursor.board_seq@3`. No family is the old
//! key itself, which is what the tests and a cache not yet adopted read
//! ([`Database::adopt_legacy`](super::Database::adopt_legacy)).

/// `key@family`, or `key` itself for no family. The id is written as Rust writes an `i64`, which is
/// .NET's invariant spelling of a `long`.
pub(crate) fn of(key: &str, family_id: Option<i64>) -> String {
    match family_id {
        Some(family) => format!("{key}@{family}"),
        None => key.to_string(),
    }
}
