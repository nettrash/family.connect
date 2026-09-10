//! Member mentions (docs/protocol.md, "Mentioning a member"): the grammar
//! every composer and every bubble share, ported by value from
//! ios/FamilyConnect/Models/MemberMentions.swift (and the composer's
//! `@`-strip rules it carries for Views/MentionSuggestions.swift).
//!
//! The GRAMMAR is the assistant token's rule applied to a name: `@`
//! followed by exactly the name, at a boundary on both sides — an ASCII
//! letter, digit or `_` after it means it is a longer word, so `@Ann` is
//! not found inside `@Anna` and `mail@Anna` is an address. The server
//! checks the same thing (server/src/mentions.rs `names_member`) and
//! refuses a mention the body does not carry, so the two must agree or a
//! message is refused for a name the sender can see in front of them.
//!
//! The RESOLUTION is from the text, at send: every member whose `@Name` the
//! body carries, each member once, in order of first appearance. Names are
//! tried LONGEST FIRST — by UTF-8 byte length, which is what Swift's
//! `name.utf8.count` measures — ties to the LOWER `user_id`, and a token
//! once claimed is not offered again, so `@Anna Lee` names Anna Lee and not
//! also Anna, and one `@Anna` in a family with two of them names the same
//! Anna on every platform.
//!
//! ONE deliberate difference from the Swift, in the server's favour. Swift
//! keeps a match only when its `@` falls on a `Character` boundary, so a
//! `@` bound to a prepended mark before it — `\u{600}@Anna` — names nobody
//! on Apple while the server and Android accept the mention. This port
//! takes the match by the server's byte rule, at both ends, and widens the
//! range outward to the grapheme clusters it touches: exactly what Swift
//! already does for the END ("The END may not be…"), applied to the start.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use super::assistant::{is_boundary, widen_to_clusters};

/// A member a message may name, or does name: the id, and the display name
/// AS TYPED after the `@`, so a bubble can find the token without knowing
/// what the member is called today. `MentionDTO`, borrowed: the caller's
/// own roster or wire `mentions` list lends the name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Member<'a> {
    pub user_id: i64,
    pub name: &'a str,
}

/// One `@Name` a bubble draws, and the member it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token<'a> {
    /// A byte range into the text, on grapheme boundaries, `@` included.
    pub range: Range<usize>,
    pub member: Member<'a>,
}

/// The private scheme Apple's bubble puts on a highlighted name, so a tap
/// reaches the member through the bubble's own link handling rather than a
/// browser. Never on the wire.
pub const SCHEME: &str = "fcmember";

/// `fcmember://<id>` — `MemberMentions.url(for:)`.
pub fn url(user_id: i64) -> String {
    format!("{SCHEME}://{user_id}")
}

/// The member id a `fcmember://` link carries, or `None` for anything else —
/// `MemberMentions.userID(from:)`, which reads the URL's HOST: the scheme
/// is compared as written, and a path, query, fragment, user or port around
/// the host does not change the answer.
pub fn user_id_from(url: &str) -> Option<i64> {
    let rest = url.strip_prefix(SCHEME)?.strip_prefix("://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host_and_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = host_and_port.split(':').next().unwrap_or_default();
    // Swift's `Int64(_:)` and Rust's `parse` agree: an optional sign, then
    // ASCII digits and nothing else.
    host.parse().ok()
}

/// Every `@name` in `body`, the `@` included, as byte ranges on grapheme
/// boundaries, in order — the whole name, at a boundary on both sides.
///
/// The scan is the server's `names_member`, byte for byte, and it indexes
/// only the byte slice: a `&str` is sliced nowhere here. The ends it finds
/// are char boundaries — the `@` is ASCII and the name is whole UTF-8 — but
/// not always CLUSTER boundaries: a combining mark, a ZWJ or a variation
/// selector after the name rides on its last letter, and a prepended mark
/// before the `@` binds to it. The match stands either way, because the
/// server's does; the range grows to the whole cluster, which is the only
/// way it can be drawn.
///
/// A match whose grown range would overlap the one before it is left out,
/// so the ranges are disjoint. Swift never finds that second match at all —
/// it needs a name ending in a prepended mark — and the server has already
/// said yes on the first.
pub fn ranges(body: &str, name: &str) -> Vec<Range<usize>> {
    if name.is_empty() {
        return Vec::new();
    }
    let bytes = body.as_bytes();
    let token = name.as_bytes();
    let length = token.len() + 1;
    let mut found: Vec<Range<usize>> = Vec::new();
    let mut index = 0;
    while index + length <= bytes.len() {
        if bytes[index] == b'@'
            && &bytes[index + 1..index + length] == token
            && (index == 0 || is_boundary(bytes[index - 1]))
            && (index + length == bytes.len() || is_boundary(bytes[index + length]))
        {
            let range = widen_to_clusters(body, index..index + length);
            if found.last().is_none_or(|last| last.end <= range.start) {
                found.push(range);
            }
            index += length;
        } else {
            index += 1;
        }
    }
    found
}

/// Does `body` name this member — say `@` followed by exactly `name`? The
/// server's `names_member`, answer for answer.
pub fn names(body: &str, name: &str) -> bool {
    !ranges(body, name).is_empty()
}

/// The members `body` names, resolved against the roster — see the module
/// header. Empty when it names nobody.
///
/// The roster is every CURRENT member by the name the app calls them — the
/// reader and the blocked included (the protocol lets a sender name
/// themself, and a block is never refused), the assistant not, since `@ai`
/// is its own grammar. Each member comes back once, in order of first
/// appearance, whatever order the roster lists them in.
pub fn resolve<'a>(body: &str, roster: &[Member<'a>]) -> Vec<Member<'a>> {
    if !body.contains('@') {
        return Vec::new();
    }
    let mut seen: Vec<i64> = Vec::new();
    let mut claimed: Vec<Range<usize>> = Vec::new();
    let mut found: Vec<(usize, Member<'a>)> = Vec::new();
    for member in longest_first(roster) {
        if seen.contains(&member.user_id) {
            continue;
        }
        let Some(first) = ranges(body, member.name)
            .into_iter()
            .find(|range| !claimed.iter().any(|taken| overlaps(taken, range)))
        else {
            continue;
        };
        seen.push(member.user_id);
        found.push((first.start, member));
        claimed.push(first);
    }
    found.sort_by_key(|(offset, _)| *offset);
    found.into_iter().map(|(_, member)| member).collect()
}

/// Every `@Name` token the message draws, one owner per token: the
/// message's own `mentions` list, longest name first, a token claimed once
/// — the same rule [`resolve`] named them by, so the bubble marks `@Anna
/// Lee` as Anna Lee even when the message names Anna as well. In order of
/// position.
pub fn tokens<'a>(text: &str, mentions: &[Member<'a>]) -> Vec<Token<'a>> {
    let mut found: Vec<Token<'a>> = Vec::new();
    for mention in longest_first(mentions) {
        for range in ranges(text, mention.name) {
            if !found.iter().any(|token| overlaps(&token.range, &range)) {
                found.push(Token {
                    range,
                    member: mention,
                });
            }
        }
    }
    found.sort_by_key(|token| token.range.start);
    found
}

/// Longest name first — UTF-8 bytes, Swift's `name.utf8.count` — then the
/// LOWER id: roster order differs between the ports, the id does not.
/// A stable sort, as Swift's is, so exact duplicates keep their order.
fn longest_first<'a>(members: &[Member<'a>]) -> Vec<Member<'a>> {
    let mut sorted = members.to_vec();
    sorted.sort_by(|a, b| {
        b.name
            .len()
            .cmp(&a.name.len())
            .then(a.user_id.cmp(&b.user_id))
    });
    sorted
}

/// Swift's `Range.overlaps` for the non-empty half-open ranges used here.
fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

// MARK: - The composer's `@` strip (Views/MentionSuggestions.swift)

/// The prefix being typed after a trailing `@`, or `None` when the composer
/// is not mid-mention: no `@`, one that follows an ASCII letter, digit or
/// `_` (an address), or a line break after it. Empty when the `@` was just
/// typed — every candidate is offered then.
///
/// Mirrors Swift CHARACTER BY CHARACTER, and the two places that shows are
/// deliberate ports of what Swift answers:
///
/// - the `@` is the last CHARACTER that is `@` (`lastIndex(of: "@")`). An
///   `@` a combining mark rides on, or a prepended mark binds to, is part of
///   a larger Character and is not found;
/// - "a line break" is a CHARACTER equal to `"\n"` (`tail.contains("\n")`).
///   A CR LF pair is ONE Character, `"\r\n"`, which is not `"\n"` — so on
///   Apple, and here, a Windows line break does not end the token. (Android
///   looks for the `'\n'` code unit and does end it.)
pub fn query(draft: &str) -> Option<&str> {
    let at = last_at(draft)?;
    if at > 0 && !is_boundary(draft.as_bytes()[at - 1]) {
        return None;
    }
    // The Character at `at` is the single byte `@`, so `at + 1` is a
    // boundary of every kind.
    let tail = &draft[at + 1..];
    if tail.graphemes(true).any(|character| character == "\n") {
        return None;
    }
    Some(tail)
}

/// The roster narrowed to what `query` could be the start of — minus
/// `excluding` (the reader themself and the blocked; the assistant is not
/// in the roster) — in roster order.
///
/// Mirrors Swift's `name.lowercased().hasPrefix(query.lowercased())`:
///
/// - `lowercased()` is each scalar's full Unicode lowercase mapping with NO
///   context — `ΟΔΟΣ` lowercases to `οδοσ`, not `οδος`, and `İ` to `i̇`.
///   That is `char::to_lowercase`, one char at a time; `str::to_lowercase`
///   would apply the final-sigma rule Swift does not;
/// - `hasPrefix` compares CHARACTERS, so `zoe` is not a prefix of `zoë` —
///   written precomposed OR as `e` + U+0308 — while a code-unit comparison
///   (Android's `startsWith`) says it is of the second.
///
/// The one place this port is narrower than Swift: Swift's Characters are
/// equal under canonical equivalence, so a query typed in NFC matches a
/// name stored in NFD. Here clusters are compared as written. Saying that
/// exactly would take Unicode normalization tables for a filter over a
/// dozen names that nothing on the wire depends on.
pub fn candidates<'a>(roster: &[Member<'a>], query: &str, excluding: &[i64]) -> Vec<Member<'a>> {
    let needle = lowercased(query);
    roster
        .iter()
        .filter(|member| {
            !excluding.contains(&member.user_id)
                && (needle.is_empty() || has_prefix(&lowercased(member.name), &needle))
        })
        .copied()
        .collect()
}

/// The draft with the trailing `@prefix` replaced by `@Name `. With no `@`
/// in it at all, `@Name ` is appended — Swift's fallback, which the strip
/// never reaches because [`query`] answered `None` there.
pub fn accept(draft: &str, name: &str) -> String {
    match last_at(draft) {
        None => format!("{draft}@{name} "),
        Some(at) => format!("{}@{name} ", &draft[..at]),
    }
}

/// The byte offset of the last Character that is exactly `@` — Swift's
/// `lastIndex(of: "@")` on a `String`.
fn last_at(text: &str) -> Option<usize> {
    text.grapheme_indices(true)
        .rev()
        .find(|(_, character)| *character == "@")
        .map(|(offset, _)| offset)
}

/// Swift's `String.lowercased()`: every scalar's full lowercase mapping,
/// with no context.
fn lowercased(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).collect()
}

/// Swift's `String.hasPrefix`: Character by Character.
fn has_prefix(text: &str, prefix: &str) -> bool {
    let mut characters = text.graphemes(true);
    prefix
        .graphemes(true)
        .all(|expected| characters.next() == Some(expected))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(user_id: i64, name: &str) -> Member<'_> {
        Member { user_id, name }
    }

    fn drawn<'t>(text: &'t str, found: &[Range<usize>]) -> Vec<&'t str> {
        found.iter().map(|range| &text[range.clone()]).collect()
    }

    #[test]
    fn grammar() {
        assert!(names("@Anna are you in?", "Anna"));
        assert!(names("hey @Anna, dinner?", "Anna"));
        assert!(names("(@Anna)", "Anna"));
        assert!(names("@Uncle Bob is here", "Uncle Bob"));
        assert!(names("@Анна привет", "Анна"));
        assert!(names("@Анна", "Анна"));
        // Not the whole name, or not at a boundary.
        assert!(!names("@Annabel", "Anna"));
        assert!(!names("@Ann", "Anna"));
        assert!(!names("@anna", "Anna"), "case is the sender's");
        assert!(!names("mail@Anna", "Anna"));
        assert!(!names("Anna", "Anna"), "no @");
        assert!(!names("@Anna", ""));
        // The second @ is the one: the first is a longer word.
        assert!(names("@Annabel and @Anna", "Anna"));
        let body = "@Anna and @Anna";
        assert_eq!(drawn(body, &ranges(body, "Anna")), ["@Anna", "@Anna"]);
    }

    #[test]
    fn resolve() {
        let roster = [member(1, "Bob"), member(2, "Uncle Bob"), member(3, "Anna")];
        assert_eq!(
            super::resolve("@Anna and @Uncle Bob: 7?", &roster),
            [roster[2], roster[1]]
        );
        assert_eq!(
            super::resolve("@Uncle Bob and @Anna", &roster),
            [roster[1], roster[2]]
        );
        assert_eq!(super::resolve("@Bob @Bob", &roster), [roster[0]]);
        assert!(super::resolve("no one here", &roster).is_empty());
        assert!(super::resolve("@Bobby", &roster).is_empty());
    }

    /// The grammar is the SERVER's, byte for byte — including where a
    /// combining mark, a ZWJ sequence or a variation selector follows the
    /// token.
    #[test]
    fn grapheme_boundaries_do_not_change_the_grammar() {
        let body = "@Anna\u{0301} are you in?";
        assert!(names(body, "Anna"));
        let range = ranges(body, "Anna").into_iter().next().expect("a range");
        assert_eq!(
            &body[range], "@Anna\u{0301}",
            "the highlight takes the whole cluster: the mark belongs to the token's last letter"
        );
        assert!(names("@Anna\u{200D}\u{1F469} hi", "Anna"));
        assert!(names("@Anna\u{FE0F}", "Anna"));
        // And the ordinary vectors are untouched.
        assert!(names("@Anna, dinner?", "Anna"));
        assert!(!names("@Annabel", "Anna"));
        // The boundary class is ASCII alphanumerics and `_`, and NOTHING
        // else.
        assert!(names("@AnnaЖ", "Anna"));
        assert!(names("@Anna文", "Anna"));
        assert!(!names("@Annab", "Anna"));
        assert!(!names("@Anna9", "Anna"));
        assert!(!names("@Anna_", "Anna"));
    }

    /// Two members called Anna and one `@Anna`: the token names the one
    /// with the lower id, on every platform.
    #[test]
    fn duplicate_names_resolve_by_id() {
        let anna_high = member(12, "Anna");
        let anna_low = member(3, "Anna");
        assert_eq!(super::resolve("@Anna?", &[anna_high, anna_low]), [anna_low]);
        assert_eq!(super::resolve("@Anna?", &[anna_low, anna_high]), [anna_low]);
        let owners: Vec<i64> = tokens("@Anna?", &[anna_high, anna_low])
            .iter()
            .map(|token| token.member.user_id)
            .collect();
        assert_eq!(owners, [3], "and the bubble marks it for the same member");
    }

    /// A name that is the start of another's: only the claim keeps Anna
    /// out of `@Anna Lee`.
    #[test]
    fn prefix_names() {
        let anna = member(3, "Anna");
        let anna_lee = member(4, "Anna Lee");
        let roster = [anna, anna_lee];
        assert_eq!(super::resolve("@Anna Lee is here", &roster), [anna_lee]);
        assert_eq!(
            super::resolve("@Anna, is @Anna Lee coming?", &roster),
            [anna, anna_lee]
        );
        assert_eq!(
            super::resolve("@Anna Lee and @Anna", &roster),
            [anna_lee, anna]
        );
        // And the bubble marks the tokens the same way.
        let text = "@Anna, is @Anna Lee coming?";
        let marked: Vec<(&str, i64)> = tokens(text, &[anna, anna_lee])
            .into_iter()
            .map(|token| (&text[token.range], token.member.user_id))
            .collect();
        assert_eq!(marked, [("@Anna", 3), ("@Anna Lee", 4)]);
        // Swift then runs this through its renderer (MessageLinks), which is
        // not ported: for a plain-text body the link it puts on each token
        // is exactly its owner's `fcmember://` URL, which is this.
        let links: Vec<(&str, String)> = tokens(text, &[anna, anna_lee])
            .into_iter()
            .map(|token| (&text[token.range], url(token.member.user_id)))
            .collect();
        assert_eq!(
            links,
            [("@Anna", url(3)), ("@Anna Lee", url(4))],
            "got {links:?}"
        );
    }

    #[test]
    fn query_and_accept() {
        assert_eq!(query("hey @An"), Some("An"));
        assert_eq!(query("@"), Some(""));
        assert_eq!(query("hey @Uncle "), Some("Uncle "));
        assert_eq!(query("mail@x"), None, "an address, not a mention");
        assert_eq!(query("@Anna\nnext"), None, "a line break ends the token");
        assert_eq!(query("no at"), None);
        assert_eq!(accept("hey @An", "Anna"), "hey @Anna ");
        assert_eq!(accept("@", "Uncle Bob"), "@Uncle Bob ");
    }

    #[test]
    fn candidates() {
        let roster = [
            member(7, "Me"),
            member(8, "Anna"),
            member(9, "Andy"),
            member(10, "Bob"),
        ];
        assert_eq!(super::candidates(&roster, "an", &[7, 9]), [roster[1]]);
        assert_eq!(super::candidates(&roster, "", &[7]).len(), 3);
        assert!(super::candidates(&roster, "zz", &[]).is_empty());
    }

    #[test]
    fn scheme() {
        let link = url(42);
        assert_eq!(user_id_from(&link), Some(42));
        assert_eq!(user_id_from("https://example.com/42"), None);
        // What Swift's URL-host reading answers around the edges, probed on
        // the real `MemberMentions.userID(from:)`.
        assert_eq!(user_id_from("fcmember://42/x"), Some(42));
        assert_eq!(user_id_from("fcmember://42:80"), Some(42));
        assert_eq!(user_id_from("fcmember://+42"), Some(42));
        assert_eq!(user_id_from("fcmember://-7"), Some(-7));
        assert_eq!(user_id_from("fcmember:42"), None);
        assert_eq!(user_id_from("FCMEMBER://42"), None);
        assert_eq!(user_id_from("fcmember://"), None);
        assert_eq!(user_id_from("fcmember://4 2"), None);
    }

    // MARK: - The server's own vectors (server/src/mentions.rs)

    /// `a_member_is_named_only_by_the_whole_name_at_a_boundary`, which the
    /// client must agree with or the server refuses what the composer sent.
    #[test]
    fn the_servers_names_member_vectors() {
        for (body, name, expected) in [
            ("@Anna are you in?", "Anna", true),
            ("hey @Anna, dinner?", "Anna", true),
            ("(@Anna)", "Anna", true),
            ("@Uncle Bob is here", "Uncle Bob", true),
            ("@Анна привет", "Анна", true),
            ("@Анна", "Анна", true),
            ("@Annabel", "Anna", false),
            ("@Ann", "Anna", false),
            ("@anna", "Anna", false),
            ("mail@Anna", "Anna", false),
            ("Anna", "Anna", false),
            ("@Anna", "", false),
            ("@Annabel and @Anna", "Anna", true),
            ("@AnnaЖ", "Anna", true),
            ("@Anna文", "Anna", true),
            ("@Anna\u{0301} are you in?", "Anna", true),
            ("@Anna\u{FE0F}", "Anna", true),
            ("@Annab", "Anna", false),
            ("@Anna9", "Anna", false),
            ("@Anna_", "Anna", false),
        ] {
            assert_eq!(names(body, name), expected, "{body:?} names {name:?}");
        }
    }

    // MARK: - Names in every script, and the shapes that panic

    /// Cyrillic, CJK, emoji and combining-mark names, found where they
    /// are and sliced cleanly — including right at the end of the text.
    #[test]
    fn names_in_every_script_slice_cleanly() {
        let roster = [
            member(1, "Анна"),
            member(2, "王芳"),
            member(3, "Zoë"),
            member(4, "Zoe\u{308}y"),
            member(5, "Mama 🌸"),
            member(6, "👨‍👩‍👧"),
        ];
        let body = "Привет @Анна! 你好@王芳，@Zoë and @Zoe\u{308}y, @Mama 🌸 and @👨‍👩‍👧";
        let found = super::resolve(body, &roster);
        assert_eq!(
            found.iter().map(|m| m.user_id).collect::<Vec<_>>(),
            [1, 2, 3, 4, 5, 6]
        );
        let marked: Vec<&str> = tokens(body, &roster)
            .into_iter()
            .map(|t| &body[t.range])
            .collect();
        assert_eq!(
            marked,
            ["@Анна", "@王芳", "@Zoë", "@Zoe\u{308}y", "@Mama 🌸", "@👨‍👩‍👧"]
        );
        // At the very end of the text, with nothing after the name.
        for (body, name) in [
            ("hi @Анна", "Анна"),
            ("@王芳", "王芳"),
            ("x @Mama 🌸", "Mama 🌸"),
        ] {
            let found = ranges(body, name);
            assert_eq!(found.len(), 1, "{body:?}");
            assert_eq!(
                found[0],
                body.len() - name.len() - 1..body.len(),
                "{body:?}"
            );
        }
        // The match is exact bytes: precomposed `ë` is not `e`, so `Zoe` is
        // not in `@Zoë` — while `e` + U+0308 IS `e` and then a mark, which
        // is a boundary to the server, so there it is named and drawn with
        // the mark.
        assert!(!names("@Zoë", "Zoe"));
        let body = "@Zoe\u{308} hi";
        assert_eq!(drawn(body, &ranges(body, "Zoe")), ["@Zoe\u{308}"]);
        // Combining mark after the name: the highlight takes the cluster,
        // ending on a char boundary a slice will accept.
        let body = "@Анна\u{301}!";
        assert_eq!(drawn(body, &ranges(body, "Анна")), ["@Анна\u{301}"]);
    }

    /// Two members with the same name, and names that prefix each other in
    /// a non-Latin script: lower id, longest first, claimed once.
    #[test]
    fn same_names_and_prefix_names_in_cyrillic() {
        let roster = [
            member(20, "Анна"),
            member(10, "Анна"),
            member(30, "Анна Ли"),
        ];
        assert_eq!(
            super::resolve("@Анна Ли и @Анна", &roster),
            [roster[2], roster[1]]
        );
        assert_eq!(super::resolve("@Анна", &roster), [roster[1]]);
        let owners: Vec<i64> = tokens("@Анна Ли и @Анна", &roster)
            .iter()
            .map(|token| token.member.user_id)
            .collect();
        assert_eq!(owners, [30, 10]);
    }

    /// Longest first is measured in UTF-8 BYTES, as Swift measures it — so
    /// a four-letter Cyrillic name (eight bytes) outranks a seven-letter
    /// Latin one. It only decides anything when two tokens can overlap,
    /// which here takes a name with an `@` in it.
    #[test]
    fn longest_is_counted_in_utf8_bytes() {
        let order: Vec<i64> = longest_first(&[member(1, "Annabel"), member(2, "Анна")])
            .iter()
            .map(|m| m.user_id)
            .collect();
        assert_eq!(order, [2, 1]);
        // `Я @B` is 5 bytes and 4 UTF-16 units; `B CD` is 4 of each. In
        // `@Я @B CD` their tokens overlap and exactly one can be named.
        // By bytes `Я @B` goes first and takes it — whichever id is lower.
        let roster = [member(1, "B CD"), member(2, "Я @B")];
        assert_eq!(super::resolve("@Я @B CD", &roster), [roster[1]]);
    }

    /// The mention the server accepts and Swift does not: an `@` bound to a
    /// prepended mark before it. The match stands, and the range widens to
    /// the cluster the `@` sits in.
    #[test]
    fn a_prepended_mark_before_the_at_is_the_servers_mention() {
        let body = "\u{600}@Anna hi";
        assert!(names(body, "Anna"));
        assert_eq!(drawn(body, &ranges(body, "Anna")), ["\u{600}@Anna"]);
        let roster = [member(3, "Anna")];
        assert_eq!(super::resolve(body, &roster), roster);
    }

    /// Every cut of every test body, at every char boundary, answered
    /// without a panic — and every range returned slices.
    #[test]
    fn no_prefix_panics() {
        let names_ = ["Anna", "Анна", "王芳", "Zoë", "Mama 🌸", "\u{600}", "a@a"];
        let bodies = [
            "Привет @Анна! 你好@王芳，@Zoë and @Zoe\u{308}y, @Mama 🌸",
            "\u{600}@Anna\u{600}@a@a@a\u{301}",
            "@Anna\r\n@Анна\u{200D}\u{1F469}🇷🇸@",
        ];
        for body in bodies {
            for cut in body.char_indices().map(|(i, _)| i).chain([body.len()]) {
                let head = &body[..cut];
                for name in names_ {
                    for range in ranges(head, name) {
                        let _ = &head[range];
                    }
                }
                if let Some(tail) = query(head) {
                    let _ = accept(head, tail);
                }
            }
        }
    }

    // MARK: - The strip, Character by Character

    #[test]
    fn the_query_reads_characters_as_swift_does() {
        // A CR LF pair is one Character, and it is not "\n".
        assert_eq!(query("@Anna\r\nnext"), Some("Anna\r\nnext"));
        assert_eq!(query("@Anna\n"), None);
        // An `@` a mark rides on, or a prepended mark binds to, is not the
        // Character `@`.
        assert_eq!(query("hey @\u{301}x"), None);
        assert_eq!(query("x \u{600}@An"), None);
        assert_eq!(query("@An \u{600}@x"), Some("An \u{600}@x"));
        // After a non-ASCII letter the `@` is at a boundary.
        assert_eq!(query("Привет@Ан"), Some("Ан"));
        assert_eq!(accept("Привет @Ан", "Анна"), "Привет @Анна ");
        assert_eq!(accept("no at", "Anna"), "no at@Anna ");
    }

    #[test]
    fn candidates_compare_characters_and_lowercase_without_context() {
        let roster = [
            member(1, "Zoë"),
            member(2, "Zoe\u{308}"),
            member(3, "ΟΔΟΣ"),
            member(4, "Анна"),
            member(5, "İlkay"),
        ];
        // `zoe` is not a prefix of `zoë`, however the ë is written.
        assert!(super::candidates(&roster, "zoe", &[]).is_empty());
        assert_eq!(
            super::candidates(&roster, "zo", &[]),
            [roster[0], roster[1]]
        );
        // No final sigma: the name lowercases to `οδοσ`.
        assert_eq!(super::candidates(&roster, "οδοσ", &[]), [roster[2]]);
        assert!(super::candidates(&roster, "οδος", &[]).is_empty());
        assert_eq!(super::candidates(&roster, "АН", &[]), [roster[3]]);
        // `İ` lowercases to `i` + U+0307, so plain `il` is not its prefix.
        assert!(super::candidates(&roster, "il", &[]).is_empty());
        assert_eq!(super::candidates(&roster, "i\u{307}l", &[]), [roster[4]]);
    }
}
