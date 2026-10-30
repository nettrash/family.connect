//! `MessageBodyTests`: a body composed for drawing — the web client's `views/body.rs` tests, case
//! for case.

use fc_core::text::message_body::{BodyMark, BodyPiece, declared, first_web_link, pieces};
use fc_text::links;
use fc_text::markdown::{self, Block, Font};
use fc_text::mentions::Member;

fn laid_out(body: &str, named: &[Member<'_>]) -> Vec<BodyPiece> {
    match markdown::blocks(body).first() {
        Some(Block::Text(text)) => pieces(text, Some(body), named),
        _ => Vec::new(),
    }
}

fn find<'a>(pieces: &'a [BodyPiece], text: &str) -> &'a BodyPiece {
    pieces
        .iter()
        .find(|piece| piece.text == text)
        .unwrap_or_else(|| {
            let all: Vec<&str> = pieces.iter().map(|piece| piece.text.as_str()).collect();
            panic!("no piece {text:?} in [{}]", all.join(" | "))
        })
}

fn anna(user_id: i64) -> Member<'static> {
    Member {
        user_id,
        name: "Anna",
    }
}

#[test]
fn markdown_styles_reach_the_pieces() {
    let laid = laid_out("**bold** and `code` and ~~gone~~", &[]);
    assert!(find(&laid, "bold").style.strong);
    assert!(find(&laid, "code").style.code);
    assert!(find(&laid, "gone").style.strikethrough);
}

/// A detected link is a link; the author's markdown destination wins over anything detected in
/// its label.
#[test]
fn links_are_found_and_the_authors_destination_wins() {
    assert_eq!(
        find(
            &laid_out("see https://example.com/x now", &[]),
            "https://example.com/x"
        )
        .link
        .as_deref(),
        Some("https://example.com/x")
    );
    assert_eq!(
        laid_out("[https://a.com](https://b.com)", &[])[0]
            .link
            .as_deref(),
        Some("https://b.com")
    );
    assert_eq!(
        laid_out("[shop](example.com)", &[])[0].link.as_deref(),
        Some("https://example.com")
    );
}

/// A link that is not http(s), mail or a phone number never becomes one.
#[test]
fn only_web_mail_and_phone_links_are_links() {
    for body in [
        "[click](javascript:alert(1))",
        "javascript://%0Aalert(1)",
        "[x](data:text/html,hi)",
    ] {
        assert!(
            laid_out(body, &[]).iter().all(|piece| piece.link.is_none()),
            "{body:?}"
        );
    }
    assert_eq!(
        find(&laid_out("write to me@example.com", &[]), "me@example.com")
            .link
            .as_deref(),
        Some("mailto:me@example.com")
    );
    assert_eq!(
        find(&laid_out("ring 555-123-4567", &[]), "555-123-4567")
            .link
            .as_deref(),
        Some("tel:5551234567")
    );
}

/// The assistant's tokens are bold and REPLACE the run's style — and `/draw` is marked only as the
/// body's first word.
#[test]
fn the_assistants_tokens_are_marked_the_apps_way() {
    let laid = laid_out("ask *@ai* please", &[]);
    let ai = find(&laid, "@ai");
    assert_eq!(ai.mark, Some(BodyMark::Assistant));
    assert!(ai.style.strong);
    assert!(!ai.style.emphasis);

    // Swift's own vectors (MessageLinksTests.drawMarkComesFromTheRawBody).
    let vectors: [(&str, &[&str]); 9] = [
        ("**/draw** a cat", &[]),
        ("*/draw* a cat", &[]),
        ("`/draw` a cat", &[]),
        ("~~/draw~~ a cat", &[]),
        ("# /draw a cat", &[]),
        ("- /draw a cat", &[]),
        ("/draw a cat", &["/draw"]),
        ("/draw a **fluffy** cat", &["/draw"]),
        ("@ai /draw a cat", &["@ai", "/draw"]),
    ];
    for (body, marked) in vectors {
        let found: Vec<String> = laid_out(body, &[])
            .into_iter()
            .filter(|piece| piece.mark == Some(BodyMark::Assistant))
            .map(|piece| piece.text)
            .collect();
        assert_eq!(found, marked, "{body:?}");
    }
}

/// The heading's face survives the mark: only the inline style is replaced.
#[test]
fn a_mark_keeps_the_face() {
    let laid = laid_out("# ask @ai", &[]);
    let ai = find(&laid, "@ai");
    assert_eq!(ai.style.font, Font::Heading(1));
    assert!(ai.style.strong);
}

/// Only the first block of a table-split body may carry `/draw`.
#[test]
fn only_the_first_block_carries_the_picture_mark() {
    let body = "| a |\n| --- |\n| 1 |\n/draw a cat";
    let blocks = markdown::blocks(body);
    assert!(blocks.iter().any(Block::is_table));
    for (index, block) in blocks.iter().enumerate() {
        if let Block::Text(text) = block {
            let source = (index == 0).then_some(body);
            assert!(
                pieces(text, source, &[])
                    .iter()
                    .all(|piece| piece.mark.is_none())
            );
        }
    }
}

/// Mentions come last and never inside a link.
#[test]
fn mentions_are_marked_unless_a_link_covers_them() {
    let laid = laid_out("**@Anna** dinner?", &[anna(9)]);
    let found = find(&laid, "@Anna");
    assert_eq!(found.mark, Some(BodyMark::Member(9)));
    assert!(found.style.strong);

    assert!(
        laid_out("[@Anna](https://x.com)", &[anna(9)])
            .iter()
            .all(|piece| piece.mark.is_none())
    );
}

/// The preview card's link: the first https one drawn — declared, detected, or typed in a table
/// cell — and none from another scheme.
#[test]
fn the_preview_describes_the_first_https_link_as_drawn() {
    assert_eq!(
        first_web_link(
            "mail me@example.com or http://a.example then https://b.example/x and https://c.example"
        )
        .as_deref(),
        Some("https://b.example/x")
    );
    assert_eq!(
        first_web_link("[the shop](shop.example) and https://other.example").as_deref(),
        Some("https://shop.example")
    );
    assert_eq!(
        first_web_link("| a |\n| --- |\n| https://cell.example |").as_deref(),
        Some("https://cell.example")
    );
    assert_eq!(
        first_web_link("ring 555-123-4567, mail me@example.com, see http://plain.example"),
        None
    );
    // On Apple every markdown link covers what is detected in its label — even one a click here
    // would not open.
    assert_eq!(
        first_web_link("[see https://example.com](ftp://files.example.com)"),
        None
    );
    let rendered = markdown::render("[see https://example.com](ftp://files.example.com)");
    let merged = links::merge(declared(&rendered), links::detect(&rendered.plain()));
    assert_eq!(
        links::first_web_link(&merged).map(|link| link.target.as_str()),
        Some("https://example.com")
    );
}

/// A label split into runs is still one link.
#[test]
fn a_label_split_into_runs_is_one_link() {
    let found = declared(&markdown::render(
        "*[a](https://e.com)*[b](https://e.com) [c](https://f.com)",
    ));
    let pairs: Vec<(&str, &str)> = found
        .iter()
        .map(|span| (span.text.as_str(), span.target.as_str()))
        .collect();
    assert_eq!(pairs, [("ab", "https://e.com"), ("c", "https://f.com")]);
    assert_eq!(found[0].range, 0..2);
}

/// Every piece boundary is a character boundary, in any script.
#[test]
fn pieces_never_cut_inside_a_character() {
    let body = "Привет @Ана́, **смотри** https://пример.рф/путь 😀 @ai 🇷🇸";
    let laid = laid_out(
        body,
        &[Member {
            user_id: 9,
            name: "Ана",
        }],
    );
    let joined: String = laid.iter().map(|piece| piece.text.as_str()).collect();
    assert_eq!(joined, markdown::render(body).plain());
    // Past an emoji, a flag and a combining mark, the offsets still land on the tokens themselves.
    assert_eq!(find(&laid, "@ai").mark, Some(BodyMark::Assistant));
    assert_eq!(find(&laid, "@Ана́").mark, Some(BodyMark::Member(9)));
    // The parser's own autolink made it a markdown link, so its destination — as typed — is the
    // one that stands.
    assert_eq!(
        find(&laid, "https://пример.рф/путь").link.as_deref(),
        Some("https://пример.рф/путь")
    );
}
