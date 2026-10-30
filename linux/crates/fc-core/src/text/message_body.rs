//! A message body composed for drawing (`MessageBody.cs`), in exactly the order the Apple app
//! composes it (web `views/body.rs`, `fc_text::markdown`'s "Composition"): markdown runs; per text
//! block the author's openable links, then detected links wherever none already is; `@ai` and a
//! leading `/draw` drawn bold, REPLACING the run's style; member mentions last, skipped under any
//! link. Table cells get none of it. All drawing — a body is plain text on the wire.
//!
//! Every rule it composes is `fc_text`'s (`markdown`, `links`, `assistant`, `mentions`); the
//! composition is the web client's and lives in no crate, so it is here.
//!
//! An inline formula (`Style::math`, issue #97) is DRAWN, not set in type, so nothing is laid over
//! it: a link, an `@ai` or a mention that would touch one is not made, and the formula's run
//! reaches the bubble whole. (The author's own links never touch one — a formula in a link's label
//! is flattened to text by the parser.)
//!
//! Offsets are UTF-8 byte offsets into the text block's plain text, as `fc_text`'s are, and every
//! piece is cut on a character boundary.

use std::collections::BTreeSet;
use std::ops::Range;

use fc_text::links::{self, LinkSpan};
use fc_text::markdown::{self, Style, Text};
use fc_text::mentions::{self, Member};

/// What covers one piece of a text block: a member's `@Name`, or the assistant's `@ai` or leading
/// `/draw`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BodyMark {
    Assistant,
    Member(i64),
}

/// One piece of a text block with everything that applies to it — the unit a bubble draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyPiece {
    pub text: String,
    pub style: Style,
    /// What a click opens: always http(s), mailto or tel.
    pub link: Option<String>,
    pub mark: Option<BodyMark>,
}

/// The first https link a body DRAWS, which is what the preview card under it describes (ios
/// `MessageLinks.firstWebLinkAsDrawn`): over the flat render — so a link typed only in a table cell
/// still gets a card — the author's links merged over the detector's. Every markdown link counts in
/// that merge, openable or not, as on Apple: a detected https link inside the label of an `ftp:`
/// link is covered by it and previews nothing.
pub fn first_web_link(body: &str) -> Option<String> {
    let rendered = markdown::render(body);
    let formulas = formulas(&rendered);
    let linked: Vec<LinkSpan> = links::merge(
        declared_links(&rendered, false),
        links::detect(&rendered.plain()),
    )
    .into_iter()
    // A URL inside a formula is drawn as part of the formula, never as a link — so it is not what
    // the card describes.
    .filter(|link| !touches(&formulas, &link.range))
    .collect();
    links::first_web_link(&linked).map(|link| link.target.clone())
}

/// Where a text block's formulas are, as byte ranges of its plain text.
fn formulas(text: &Text) -> Vec<Range<usize>> {
    let mut formulas = Vec::new();
    let mut at = 0;
    for span in &text.spans {
        let end = at + span.text.len();
        if span.style.math.is_some() {
            formulas.push(at..end);
        }
        at = end;
    }
    formulas
}

/// Whether `range` overlaps any of `formulas`.
fn touches(formulas: &[Range<usize>], range: &Range<usize>) -> bool {
    formulas
        .iter()
        .any(|formula| formula.start < range.end && range.start < formula.end)
}

/// A text block's own markdown links that can be opened, as spans — a label split into runs still
/// one link.
pub fn declared(text: &Text) -> Vec<LinkSpan> {
    declared_links(text, true)
}

fn declared_links(text: &Text, openable_only: bool) -> Vec<LinkSpan> {
    let plain = text.plain();
    let mut declared: Vec<LinkSpan> = Vec::new();
    let mut at = 0;
    for span in &text.spans {
        let (start, end) = (at, at + span.text.len());
        at = end;
        let Some(link) = &span.style.link else {
            continue;
        };
        let target = links::normalize_destination(&link.destination);
        if openable_only && !links::is_openable(&target) {
            continue;
        }
        match declared.last_mut() {
            Some(previous) if previous.range.end == start && previous.target == target => {
                previous.range.end = end;
                previous.text = plain[previous.range.clone()].to_string();
            }
            _ => declared.push(LinkSpan {
                range: start..end,
                text: span.text.clone(),
                target,
            }),
        }
    }
    declared
}

/// One text block laid out as pieces. `draw_source` is the RAW body for the first block and `None`
/// for the rest: `/draw` is a request only at the start of the whole body, marked only where
/// markdown left it recognisable.
pub fn pieces(text: &Text, draw_source: Option<&str>, named: &[Member<'_>]) -> Vec<BodyPiece> {
    let plain = text.plain();
    let mut runs: Vec<(Range<usize>, &Style)> = Vec::with_capacity(text.spans.len());
    let mut at = 0;
    for span in &text.spans {
        let end = at + span.text.len();
        runs.push((at..end, &span.style));
        at = end;
    }

    let formulas = formulas(text);
    let linked: Vec<LinkSpan> = links::merge(declared(text), links::detect(&plain))
        .into_iter()
        .filter(|link| !touches(&formulas, &link.range))
        .collect();

    let mut marks: Vec<(Range<usize>, BodyMark)> = fc_text::assistant::ranges(&plain)
        .into_iter()
        .map(|range| (range, BodyMark::Assistant))
        .collect();
    if draw_source.is_some_and(|source| fc_text::assistant::draw_token_range(source).is_some())
        && let Some(draw) = fc_text::assistant::draw_token_range(&plain)
    {
        marks.push((draw, BodyMark::Assistant));
    }
    marks.retain(|(range, _)| !touches(&formulas, range));

    let overlaps = |a: &Range<usize>, b: &Range<usize>| a.start < b.end && b.start < a.end;
    for token in mentions::tokens(&plain, named) {
        let under_link = linked
            .iter()
            .any(|link| overlaps(&link.range, &token.range));
        let under_mark = marks.iter().any(|(range, _)| overlaps(range, &token.range));
        if !under_link && !under_mark && !touches(&formulas, &token.range) {
            marks.push((token.range, BodyMark::Member(token.member.user_id)));
        }
    }

    // Cut the block at every boundary any of it has.
    let mut cuts: BTreeSet<usize> = BTreeSet::from([0, plain.len()]);
    for range in runs
        .iter()
        .map(|(range, _)| range)
        .chain(linked.iter().map(|link| &link.range))
        .chain(marks.iter().map(|(range, _)| range))
    {
        cuts.insert(range.start);
        cuts.insert(range.end);
    }
    let boundaries: Vec<usize> = cuts
        .into_iter()
        .filter(|&cut| plain.is_char_boundary(cut))
        .collect();

    let plain_style = Style::default();
    boundaries
        .windows(2)
        .map(|pair| {
            let (start, end) = (pair[0], pair[1]);
            let covers = |range: &Range<usize>| range.start <= start && end <= range.end;
            let mut style = runs
                .iter()
                .find(|(range, _)| covers(range))
                .map_or(&plain_style, |(_, style)| *style)
                .clone();
            let link = linked
                .iter()
                .find(|link| covers(&link.range))
                .map(|link| link.target.clone());
            let mark = marks
                .iter()
                .find(|(range, _)| covers(range))
                .map(|(_, mark)| *mark);
            if mark.is_some() {
                // The mark REPLACES the run's inline style with bold — and keeps the face.
                style = Style {
                    strong: true,
                    font: style.font,
                    ..Style::default()
                };
            }
            BodyPiece {
                text: plain[start..end].to_string(),
                style,
                link,
                mark,
            }
        })
        .collect()
}
