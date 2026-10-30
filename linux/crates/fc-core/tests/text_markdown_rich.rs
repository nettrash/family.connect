//! `MarkdownRichTests`: formulas, diagrams and graphs (issue #97). Most of the C# class holds the
//! design doc's tables, which `fc_text::markdown`'s own tests hold row for row — those are not
//! ported (see the report). What is here: the three tests whose cases `fc_text` does not pin as
//! given (against `fc_text::markdown`), and what a bubble's composition must leave alone around a
//! formula (against `fc_core::text::message_body`).

use fc_core::text::message_body::{BodyPiece, first_web_link, pieces};
use fc_text::markdown::{self, Block, Text};
use fc_text::mentions::Member;

/// Every formula a body's text and table cells carry, in order.
fn formulas(body: &str) -> Vec<String> {
    fn of(text: &Text, out: &mut Vec<String>) {
        out.extend(text.spans.iter().filter_map(|span| span.style.math.clone()));
    }
    let mut out = Vec::new();
    for block in markdown::blocks(body) {
        match &block {
            Block::Text(text) => of(text, &mut out),
            Block::Table(table) => {
                for cell in table.header.iter().chain(table.rows.iter().flatten()) {
                    of(cell, &mut out);
                }
            }
            Block::Rich(_) => {}
        }
    }
    out
}

/// A body's blocks as words: `text <plain>`, `table`, or `rich <kind> <engine> <source>`.
fn shape(body: &str) -> Vec<String> {
    markdown::blocks(body)
        .iter()
        .map(|block| match block {
            Block::Text(text) => format!("text {}", text.plain()),
            Block::Table(_) => "table".to_string(),
            Block::Rich(rich) => {
                format!("rich {} {} {}", rich.kind.word(), rich.engine, rich.source)
            }
        })
        .collect()
}

/// What a bubble shows until a block is drawn: a fence's source, as a code block shows it, but a
/// display formula's lines as typed — delimiters, spaces and all — which the source, trimmed and
/// without them, cannot give back.
#[test]
fn a_rich_block_shows_what_was_typed() {
    for (body, shown) in [
        ("```math\nx^2\n```", "x^2"),
        ("```mermaid\ngraph TD\nA-->B\n```", "graph TD\nA-->B"),
        ("$$x^2$$", "$$x^2$$"),
        ("$$\nx^2\n$$", "$$\nx^2\n$$"),
        ("\\[\na\n\\]", "\\[\na\n\\]"),
        ("  $$ y $$  ", "$$ y $$"),
        ("\t\\[ x \\]\t", "\\[ x \\]"),
        ("$$ a +\n\tb \n$$ ", "$$ a +\n\tb \n$$"),
        ("Look:\n$$ a +\n\tb \n$$\nDone", "$$ a +\n\tb \n$$"),
        ("€ before\n$$ é $$", "$$ é $$"),
    ] {
        let rich: Vec<String> = markdown::blocks(body)
            .into_iter()
            .filter_map(|block| match block {
                Block::Rich(rich) => Some(rich.shown),
                _ => None,
            })
            .collect();
        assert_eq!(rich, [shown], "{body:?}");
    }
}

#[test]
fn a_formula_is_a_run_of_its_own_and_nothing_inside_is_markdown() {
    assert_eq!(markdown::render("\\$x$").plain(), "$x$");
    let emphasis = match markdown::blocks("*see $a*b$*").as_slice() {
        [Block::Text(text)] => text.clone(),
        other => panic!("not one text block: {other:?}"),
    };
    assert!(emphasis.spans.iter().all(|span| span.style.emphasis));
    let math: Vec<&str> = emphasis
        .spans
        .iter()
        .filter(|span| span.style.math.is_some())
        .map(|span| span.text.as_str())
        .collect();
    assert_eq!(math, ["$a*b$"]);

    let runs: Vec<(String, Option<String>)> =
        markdown::render("Area $\\pi r^2$, or \\(a\\)\\(a\\).")
            .spans
            .into_iter()
            .map(|span| (span.text, span.style.math))
            .collect();
    let expected: Vec<(String, Option<String>)> = [
        ("Area ", None),
        ("$\\pi r^2$", Some("\\pi r^2")),
        (", or ", None),
        ("\\(a\\)", Some("a")),
        ("\\(a\\)", Some("a")),
        (".", None),
    ]
    .into_iter()
    .map(|(text, math)| (text.to_string(), math.map(str::to_string)))
    .collect();
    assert_eq!(runs, expected);
    for body in [
        "$**x**$",
        "$<b>x</b>$",
        "$&amp;$",
        "$https://example.com$",
        "$me@example.com$",
        "\\(a`b\\)",
    ] {
        let rendered = markdown::render(body);
        assert_eq!(rendered.spans.len(), 1, "{body:?}");
        assert_eq!(rendered.spans[0].text, body);
        assert_eq!(rendered.spans[0].style.link, None);
    }
    // Code spans come first, and a formula never reaches across one.
    assert!(formulas("$a `b$` c$").is_empty());
    assert_eq!(formulas("$a`b$"), ["a`b"]);
}

/// Non-ASCII around and inside: never a byte offset off a character boundary.
#[test]
fn formulas_over_non_ascii() {
    assert_eq!(formulas("Привет $α+β$ мир"), ["α+β"]);
    assert_eq!(formulas("\\(привет\\)"), ["привет"]);
    assert_eq!(formulas("😀 $$😀$$ 😀"), ["😀"]);
    assert_eq!(shape("$$ Привет $$"), ["rich math  Привет"]);
    assert_eq!(formulas("$x\u{301}$"), ["x\u{301}"]);
}

/// Nothing is laid over a formula: no detected link, no `@ai`, no mention — it reaches the bubble
/// whole.
#[test]
fn a_bubble_lays_nothing_over_a_formula() {
    fn laid(body: &str, named: &[Member<'_>]) -> Vec<BodyPiece> {
        match markdown::blocks(body).first() {
            Some(Block::Text(text)) => pieces(text, Some(body), named),
            other => panic!("not a text block: {other:?}"),
        }
    }

    let linked = laid("see $https://e.com/x$ and https://e.com/y", &[]);
    let formula: Vec<&BodyPiece> = linked
        .iter()
        .filter(|piece| piece.style.math.is_some())
        .collect();
    assert_eq!(formula.len(), 1);
    assert_eq!(formula[0].text, "$https://e.com/x$");
    assert_eq!(formula[0].link, None);
    let links: Vec<&str> = linked
        .iter()
        .filter_map(|piece| piece.link.as_deref())
        .collect();
    assert_eq!(links, ["https://e.com/y"]);

    let marked = laid(
        "$@ai$ @ai $@Anna$ @Anna",
        &[Member {
            user_id: 7,
            name: "Anna",
        }],
    );
    assert!(
        marked
            .iter()
            .filter(|piece| piece.style.math.is_some())
            .all(|piece| piece.mark.is_none())
    );
    assert_eq!(
        marked
            .iter()
            .filter(|piece| piece.style.math.is_some())
            .count(),
        2
    );
    assert_eq!(
        marked.iter().filter(|piece| piece.mark.is_some()).count(),
        2
    );

    // And a URL drawn only inside a formula is not what the preview card describes.
    assert_eq!(first_web_link("$https://e.com/x$"), None);
    assert_eq!(
        first_web_link("$https://e.com/x$ https://e.com/y").as_deref(),
        Some("https://e.com/y")
    );
}
