//! `MarkdownOracleTests`, THE THIRD DIFFERENTIAL ORACLE: a message body as a bubble draws it.
//! Every vector in `fixtures/markdown-vectors.json` was printed from `fc_text::markdown`,
//! `fc_text::links` and `fc_text::assistant` over every string literal in those modules' tests and
//! a fixed-seed run of markup-heavy bodies; the C# port is held to it.
//!
//! On Linux those rules ARE `fc_text`, so a case here holds the `fc_text` this build links to the
//! file Windows is held to — a vector that no longer matches is a divergence between the two
//! clients. The author's declared links are `fc_core::text::message_body::declared`.

mod common;

use serde_json::{Value, json};

use fc_core::text::message_body::declared;
use fc_text::links::{self, LinkSpan};
use fc_text::markdown::{self, Block, ColumnAlignment, Font, Link, Text};

fn cases() -> &'static Vec<Value> {
    use std::sync::OnceLock;
    static CASES: OnceLock<Value> = OnceLock::new();
    CASES.get_or_init(|| common::fixture_json("markdown-vectors.json"))["cases"]
        .as_array()
        .expect("cases")
}

fn link(link: &Option<Link>) -> Value {
    match link {
        Some(link) => json!({"destination": link.destination, "title": link.title}),
        None => Value::Null,
    }
}

fn text(text: &Text) -> Value {
    Value::Array(
        text.spans
            .iter()
            .map(|span| {
                let style = &span.style;
                let font = match style.font {
                    Font::Body => "body".to_string(),
                    Font::Heading(level) => format!("h{level}"),
                    Font::Monospaced => "mono".to_string(),
                };
                json!({
                    "text": span.text,
                    "emphasis": style.emphasis,
                    "strong": style.strong,
                    "code": style.code,
                    "strikethrough": style.strikethrough,
                    "line_break": style.line_break,
                    "html": style.html,
                    "link": link(&style.link),
                    "image": link(&style.image),
                    "font": font,
                    "math": style.math,
                })
            })
            .collect(),
    )
}

fn blocks(body: &str) -> Value {
    Value::Array(
        markdown::blocks(body)
            .iter()
            .map(|block| match block {
                Block::Text(cell) => json!({"text": text(cell)}),
                Block::Table(table) => json!({"table": {
                    "alignments": table.alignments.iter().map(|alignment| match alignment {
                        ColumnAlignment::Leading => "leading",
                        ColumnAlignment::Center => "center",
                        ColumnAlignment::Trailing => "trailing",
                    }).collect::<Vec<_>>(),
                    "header": table.header.iter().map(text).collect::<Vec<_>>(),
                    "rows": table.rows.iter()
                        .map(|row| row.iter().map(text).collect::<Vec<_>>())
                        .collect::<Vec<_>>(),
                }}),
                // Issue #97: a formula, a diagram or a graph standing on its own.
                Block::Rich(rich) => json!({"rich": {
                    "kind": rich.kind.word(),
                    "engine": rich.engine,
                    "source": rich.source,
                    "shown": rich.shown,
                }}),
            })
            .collect(),
    )
}

fn spans(spans: &[LinkSpan]) -> Value {
    Value::Array(
        spans
            .iter()
            .map(|span| {
                json!({
                    "start": span.range.start,
                    "end": span.range.end,
                    "text": span.text,
                    "target": span.target,
                })
            })
            .collect(),
    )
}

/// Every case, the port against the original; the first differences named so a failure says what
/// broke.
fn check(aspect: &str, pick: impl Fn(&Value, &str) -> (Value, Value)) {
    assert!(cases().len() > 2000, "the corpus is missing");
    let mut failures = Vec::new();
    for item in cases() {
        let body = item["body"].as_str().expect("a body");
        let (expected, actual) = pick(item, body);
        if expected != actual {
            failures.push(format!(
                "{}\n  expected {expected}\n  actual   {actual}",
                Value::from(body)
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{aspect}: {} of {} differ\n{}",
        failures.len(),
        cases().len(),
        failures
            .iter()
            .take(12)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

fn count(predicate: impl Fn(&Value) -> bool) -> usize {
    cases().iter().filter(|item| predicate(item)).count()
}

fn any_in(item: &Value, name: &str, predicate: impl Fn(&Value) -> bool) -> bool {
    item[name].as_array().expect(name).iter().any(predicate)
}

/// The corpus reaches what it claims to: tables, rich blocks, formulas, links, the assistant's
/// tokens.
#[test]
fn the_corpus_covers_every_part() {
    assert!(
        count(|item| any_in(item, "blocks", |block| !block["table"].is_null())) >= 20,
        "tables"
    );
    assert!(
        count(|item| any_in(item, "blocks", |block| !block["rich"].is_null())) >= 40,
        "rich blocks"
    );
    assert!(
        count(|item| any_in(item, "render", |run| !run["math"].is_null())) >= 150,
        "formulas"
    );
    assert!(
        count(|item| !item["merged"].as_array().expect("merged").is_empty()) >= 500,
        "links"
    );
    assert!(
        count(|item| any_in(item, "render", |run| !run["link"].is_null())) >= 100,
        "markdown links"
    );
    assert!(
        count(|item| !item["assistant_ranges"]
            .as_array()
            .expect("ranges")
            .is_empty())
            >= 100,
        "@ai"
    );
    assert!(count(|item| !item["draw_range"].is_null()) >= 10, "/draw");
}

/// The blocks a bubble draws: runs and their styles, headings, bullets, fences, tables, rich
/// blocks and formulas.
#[test]
fn blocks_are_the_originals() {
    check("blocks", |item, body| {
        (item["blocks"].clone(), blocks(body))
    });
}

/// The flat render the preview card and the link pass index.
#[test]
fn the_flat_render_is_the_originals() {
    check("render", |item, body| {
        (item["render"].clone(), text(&markdown::render(body)))
    });
}

/// Links, addresses and phone numbers, over the raw body and over the rendered text.
#[test]
fn detection_is_the_originals() {
    check("detect (raw)", |item, body| {
        (item["detect_body"].clone(), spans(&links::detect(body)))
    });
    check("detect (rendered)", |item, body| {
        (
            item["detect"].clone(),
            spans(&links::detect(&markdown::render(body).plain())),
        )
    });
}

fn merged(body: &str) -> Vec<LinkSpan> {
    let rendered = markdown::render(body);
    links::merge(declared(&rendered), links::detect(&rendered.plain()))
}

/// The author's links merged over the detector's, and the one https link the preview card
/// describes.
#[test]
fn merged_links_and_the_preview_link_are_the_originals() {
    check("merged", |item, body| {
        (item["merged"].clone(), spans(&merged(body)))
    });
    check("first web link", |item, body| {
        (
            item["first_web"].clone(),
            links::first_web_link(&merged(body))
                .map_or(Value::Null, |first| Value::from(first.target.as_str())),
        )
    });
}

/// Every markdown destination, normalised, and whether it may be opened.
#[test]
fn destinations_normalise_as_the_original() {
    check("normalized", |item, body| {
        let normalized: Vec<Value> = markdown::render(body)
            .spans
            .iter()
            .filter_map(|span| span.style.link.as_ref())
            .map(|link| {
                let target = links::normalize_destination(&link.destination);
                json!({
                    "destination": link.destination,
                    "normalized": target,
                    "openable": links::is_openable(&target),
                })
            })
            .collect();
        (item["normalized"].clone(), Value::Array(normalized))
    });
}

/// `@ai` ranges and the `/draw` token over the rendered text, and whether the raw body asks.
#[test]
fn assistant_tokens_are_the_originals() {
    check("assistant ranges", |item, body| {
        let ranges: Vec<Value> = fc_text::assistant::ranges(&markdown::render(body).plain())
            .into_iter()
            .map(|range| json!([range.start, range.end]))
            .collect();
        (item["assistant_ranges"].clone(), Value::Array(ranges))
    });
    check("draw range", |item, body| {
        (
            item["draw_range"].clone(),
            fc_text::assistant::draw_token_range(&markdown::render(body).plain())
                .map_or(Value::Null, |range| json!([range.start, range.end])),
        )
    });
    check("draw asked", |item, body| {
        (
            item["draw_asked"].clone(),
            Value::from(fc_text::assistant::draw_token_range(body).is_some()),
        )
    });
}
