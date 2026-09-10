//! The quote above a reply (docs/protocol.md, "Replies"): `reply_to.excerpt`
//! is the quoted body cut to at most 120 Unicode scalar values, never
//! mid-scalar.
//!
//! The SERVER cuts it, recomputing on every read (server/src/models.rs
//! `ReplyTo::excerpt`, `body.chars().take(120)`). A client cuts its own in
//! two places and must cut identically, or the quote visibly changes length
//! when the server's copy lands: while its own reply is still pending, and
//! when it applies an EDIT to a message its local replies quote ("Editing":
//! clients re-cut "the same way the server does"). Apple does this in
//! `ReplyToSnapshot.excerpt(of:)` (Models/Snapshots.swift), Android in
//! `ReplyToDto.excerpt`; both count scalars.
//!
//! Scalars, not grapheme clusters: a cut may land inside a family emoji or
//! between a letter and its accent. The protocol allows exactly that —
//! "never cut mid-SCALAR" — because the alternative is a server that knows
//! Unicode segmentation, and the whole point is that every client can
//! reproduce the cut with nothing but a `chars()`.

/// Longest excerpt sent, in Unicode scalar values —
/// `ReplyTo::MAX_EXCERPT_CHARS` on the server.
pub const MAX_EXCERPT_CHARS: usize = 120;

/// `body` cut to [`MAX_EXCERPT_CHARS`] scalars, as the server cuts it. A
/// slice of the caller's body, always ending on a char boundary.
pub fn excerpt(body: &str) -> &str {
    match body.char_indices().nth(MAX_EXCERPT_CHARS) {
        Some((end, _)) => &body[..end],
        None => body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // MARK: - Swift's (MessageGroupingTests)

    /// A family emoji is seven scalars and one Character; the cut counts
    /// scalars, and 120 does not divide by seven, so it lands inside one.
    #[test]
    fn excerpt_cuts_by_scalars() {
        let family = "👨‍👩‍👧‍👦";
        let body = family.repeat(40);
        let cut = excerpt(&body);
        assert_eq!(cut.chars().count(), 120);
        assert_eq!(
            cut.chars().collect::<Vec<_>>(),
            body.chars().take(120).collect::<Vec<_>>()
        );
    }

    #[test]
    fn excerpt_short_body() {
        assert_eq!(excerpt("See you at six"), "See you at six");
        assert_eq!(excerpt(""), "");
    }

    #[test]
    fn excerpt_never_splits_a_scalar() {
        let body = "é中😀".repeat(100);
        let cut = excerpt(&body);
        assert_eq!(cut.chars().count(), 120);
        assert!(std::str::from_utf8(cut.as_bytes()).is_ok());
    }

    // MARK: - The server's (server/src/models.rs)

    /// The server's own cut, spelled the server's way.
    fn servers(body: &str) -> String {
        body.chars().take(MAX_EXCERPT_CHARS).collect()
    }

    #[test]
    fn a_long_body_is_cut_to_the_documented_length() {
        let body = "x".repeat(500);
        assert_eq!(excerpt(&body).chars().count(), MAX_EXCERPT_CHARS);
    }

    #[test]
    fn the_cut_never_lands_inside_a_character() {
        let body = "é中😀".repeat(100);
        assert_eq!(excerpt(&body), servers(&body));
    }

    /// The cut at exactly the limit and either side of it, in every width a
    /// scalar comes in.
    #[test]
    fn the_cut_is_the_servers_at_every_length() {
        for unit in ["a", "é", "中", "😀", "e\u{301}", "👨‍👩‍👧‍👦"] {
            for count in 0..=140 {
                let body = unit.repeat(count);
                assert_eq!(excerpt(&body), servers(&body), "{count} × {unit:?}");
            }
        }
    }
}
