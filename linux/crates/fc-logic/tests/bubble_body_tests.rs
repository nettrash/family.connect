//! `BubbleBodyTests`: a body as a bubble lays it out — blocks, styled runs, links, and which names
//! open a chat.

use std::time::Duration;

use fc_core::protocol::{MemberDto, MentionDto};
use fc_logic::bubble_body::{
    BodyBlock, BodyRichBlock, BodyRun, LINK_DELAY, lay, openable, preview_link,
};
use fc_text::markdown::{ColumnAlignment, RichKind};

fn roster() -> Vec<MemberDto> {
    vec![
        MemberDto::new(7, "anna", "Anna"),
        MemberDto::new(11, "bob", "Bob"),
        MemberDto {
            has_left: true,
            ..MemberDto::new(12, "carl", "Carl")
        },
    ]
}

fn laid(
    body: &str,
    named: Option<&[MentionDto]>,
    me: i64,
    blocked: &dyn Fn(i64) -> bool,
) -> Vec<BodyBlock> {
    lay(body, named, &roster(), me, blocked)
}

fn runs(block: &BodyBlock) -> &[BodyRun] {
    block.runs().expect("a text block")
}

fn single(blocks: &[BodyBlock]) -> &BodyBlock {
    assert_eq!(1, blocks.len());
    &blocks[0]
}

fn find<'a>(runs: &'a [BodyRun], text: &str) -> &'a BodyRun {
    let found: Vec<&BodyRun> = runs.iter().filter(|run| run.text == text).collect();
    assert_eq!(1, found.len(), "{text:?} in {runs:?}");
    found[0]
}

#[test]
fn markdown_and_links_reach_the_runs_with_or_without_names() {
    let blocks = laid("**hi** see https://example.com and *so*", None, 1, &|_| {
        false
    });
    let runs = runs(single(&blocks));
    assert!(find(runs, "hi").style.strong);
    assert_eq!(
        Some("https://example.com"),
        find(runs, "https://example.com").link.as_deref()
    );
    assert!(find(runs, "so").style.emphasis);
    assert!(runs.iter().all(|run| !run.marked));
}

/// A name is a door only onto somebody this reader can message: not themselves, not somebody gone,
/// not somebody blocked.
#[test]
fn a_name_opens_only_where_the_reader_can_message_them() {
    let named = [
        MentionDto::new(7, "Anna"),
        MentionDto::new(11, "Bob"),
        MentionDto::new(12, "Carl"),
    ];
    let blocks = laid("**@Anna** @Bob @Carl", Some(&named), 11, &|_| false);
    let runs = runs(single(&blocks));
    let seen = |text: &str| {
        let run = find(runs, text);
        (run.member_id, run.opens, run.marked)
    };
    assert_eq!((Some(7), true, true), seen("@Anna"));
    assert_eq!((Some(11), false, true), seen("@Bob"));
    assert_eq!((Some(12), false, true), seen("@Carl"));

    let blocked = laid("@Anna", Some(&named), 11, &|id| id == 7);
    assert!(!find(self::runs(single(&blocked)), "@Anna").opens);
}

/// A table is a block of its own, and only the first block may carry the picture token.
#[test]
fn a_table_splits_the_body_and_only_the_first_block_asks_for_a_picture() {
    let blocks = laid(
        "/draw a cat\n| a | b |\n| --- | :-: |\n| 1 | 2 |\n/draw a dog",
        None,
        1,
        &|_| false,
    );
    assert_eq!(3, blocks.len());
    assert!(find(runs(&blocks[0]), "/draw").marked);
    let BodyBlock::Table(table) = &blocks[1] else {
        panic!("a table: {:?}", blocks[1]);
    };
    assert_eq!(
        vec![ColumnAlignment::Leading, ColumnAlignment::Center],
        table.alignments
    );
    assert_eq!("2", table.rows[0][1].plain());
    assert!(runs(&blocks[2]).iter().all(|run| !run.marked));
}

/// A formula, diagram or graph is a block of its own (issue #97), and an inline formula a run of
/// its own with nothing over it.
#[test]
fn rich_blocks_and_formulas_reach_the_window() {
    let named = [MentionDto::new(7, "Anna")];
    let blocks = laid(
        "Area $\\pi r^2$ for @Anna\n```neato\ngraph {a--b}\n```\n$$\n\\sum_i x_i\n$$\nDone $https://e.com$",
        Some(&named),
        1,
        &|_| false,
    );
    assert_eq!(4, blocks.len());
    let area = runs(&blocks[0]);
    let formulas: Vec<&BodyRun> = area.iter().filter(|run| run.math().is_some()).collect();
    assert_eq!(1, formulas.len());
    assert_eq!(
        ("$\\pi r^2$", Some("\\pi r^2")),
        (formulas[0].text.as_str(), formulas[0].math())
    );
    assert!(find(area, "@Anna").marked);
    assert_eq!(
        BodyBlock::Rich(BodyRichBlock::new(
            RichKind::Graphviz,
            "neato",
            "graph {a--b}"
        )),
        blocks[1]
    );
    // Drawn from the source; shown, until then, as typed — delimiters and all.
    assert_eq!(
        BodyBlock::Rich(
            BodyRichBlock::new(RichKind::Math, "", "\\sum_i x_i").with_shown("$$\n\\sum_i x_i\n$$")
        ),
        blocks[2]
    );
    let BodyBlock::Rich(graph) = &blocks[1] else {
        panic!("a graph");
    };
    assert_eq!("graph {a--b}", graph.shown);
    let links: Vec<&BodyRun> = runs(&blocks[3])
        .iter()
        .filter(|run| run.math().is_some())
        .collect();
    assert_eq!(1, links.len());
    assert_eq!(None, links[0].link);
    assert!(!links[0].marked);
}

/// Only an absolute http(s), mailto or tel target is handed to the system.
#[test]
fn only_web_mail_and_phone_targets_open() {
    assert!(openable(Some("https://example.com/a?b#c")).is_some());
    assert!(openable(Some("mailto:me@example.com")).is_some());
    assert!(openable(Some("tel:5551234567")).is_some());
    assert_eq!(None, openable(Some("javascript:alert(1)")));
    assert_eq!(None, openable(Some("file:///etc/passwd")));
    assert_eq!(None, openable(None));
}

/// Not in the C#: what its `Uri.TryCreate(…, Absolute)` refused or took on top of the scheme test,
/// measured on .NET 10 — an http(s) address needs a host to open; `mailto:` and `tel:` are taken as
/// they are.
#[test]
fn an_address_with_no_host_to_open_is_not_handed_to_the_system() {
    for refused in [
        "https:",
        "https://",
        "http:example",
        "https:/example.com",
        "https:///example.com",
        "https://:80/",
        "https://exa mple.com",
        "https://ex ample.com/",
        "http://a%41.com",
        "https://ex%zzample.com",
        "https://a..b",
        "https://[::1",
        "https://[::1]:99999/",
        "https://example.com:65536",
    ] {
        assert_eq!(None, openable(Some(refused)), "{refused}");
    }
    for taken in [
        "https://example.com:",
        "https://user@example.com",
        "https://example.com:8080/x",
        "https://example.com:65535",
        "https://[::1]/",
        "http://exa_mple.com",
        "https://-a.com",
        "https://example.com/a b",
        "https://example.com/%zz",
        "https://a.b?x y",
        "HTTP://x",
        "HTTPS://EXAMPLE.COM",
        "https://例え.jp/パス",
        "mailto:",
        "tel:",
        "tel:555 1234",
    ] {
        assert_eq!(Some(taken.to_string()), openable(Some(taken)), "{taken}");
    }
}

/// The card's link is the first https one the body draws — and a body of nothing but emoji has
/// none.
#[test]
fn the_preview_link_is_the_first_https_link_drawn() {
    let body = "mail me@example.com, http://a.example, https://b.example/x";
    assert_eq!(
        Some("https://b.example/x".to_string()),
        preview_link(body, false)
    );
    // Asked again: the memory answers the same.
    assert_eq!(
        Some("https://b.example/x".to_string()),
        preview_link(body, false)
    );
    assert_eq!(None, preview_link("https://b.example/x", true));
    assert_eq!(None, preview_link("ring 555-123-4567", false));
    assert_eq!(None, preview_link("", false));
}

#[test]
fn a_link_waits_the_macs_beat_for_a_double_click() {
    assert_eq!(Duration::from_millis(350), LINK_DELAY);
}
