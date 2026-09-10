//! A message's body, drawn: the markdown subset, links, `@ai` and `/draw`,
//! and member mentions — composed in exactly the order the Apple app
//! composes them (fc_text::markdown's "Composition"), so a message reads the
//! same on every device.
//!
//! 1. markdown makes styled runs — only a table splits a body into blocks;
//! 2. per text block, over its RENDERED characters: the author's markdown
//!    links (only those that can be opened), then detected links, emails and
//!    phone numbers wherever no link already is;
//! 3. `@ai`, and a leading `/draw` in the first block only, drawn bold —
//!    replacing the run's own style, inside links and code too;
//! 4. member mentions last, skipped wherever a link covers them.
//!
//! Table cells get none of 2–4. A body is plain text on the wire; all of
//! this is drawing (docs/protocol.md, "A body is plain text on the wire").

use std::collections::HashSet;
use std::ops::Range;

use fc_text::markdown::{self, Block, ColumnAlignment, Font, Style, Table, Text};
use fc_text::{assistant, links, mentions};
use gloo_timers::callback::Timeout;
use yew::prelude::*;

use crate::model::Mention;

/// How long a link waits before it opens, so a double-click on it can be
/// the heart reaction instead — the Mac's 350 ms.
const LINK_DELAY_MS: u32 = 350;

#[derive(Properties, PartialEq)]
pub struct BodyProps {
    pub text: String,
    /// The members this message named, as sent.
    pub mentions: Vec<Mention>,
    pub my_user_id: i64,
    /// Members a mention may OPEN a chat with: the live roster. The reader's
    /// own name, somebody who has left and a deleted account are highlighted
    /// and not tappable (docs/protocol.md, "Mentioning a member").
    pub member_ids: HashSet<i64>,
    pub on_open_direct: Callback<i64>,
}

/// What covers one piece of a text block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mark {
    /// A member's `@Name`.
    Mention(i64),
    /// The assistant's `@ai`, or a leading `/draw`.
    Assistant,
}

/// One piece of a text block, with everything that applies to it — the
/// unit the renderer draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    pub text: String,
    pub style: Style,
    /// What a click opens: always http(s), mailto: or tel:.
    pub link: Option<String>,
    pub mark: Option<Mark>,
}

/// Lay one text block out as pieces, in the Apple app's composition order.
///
/// `draw_source` is the RAW body for the first block and None for the rest:
/// `/draw` is a request only at the very start of the whole body, and it is
/// marked only where markdown left the token recognisable — never a mark
/// on a request the server would ignore.
pub fn pieces(text: &Text, draw_source: Option<&str>, named: &[Mention]) -> Vec<Piece> {
    let plain = text.plain();
    // The runs, and where each sits in the rendered characters.
    let mut runs: Vec<(Range<usize>, &Style)> = Vec::with_capacity(text.spans.len());
    let mut at = 0;
    for span in &text.spans {
        runs.push((at..at + span.text.len(), &span.style));
        at += span.text.len();
    }

    // 2. Links: the author's first — those that can actually be opened —
    // then whatever the detector finds where none already is.
    let mut declared: Vec<links::LinkSpan> = Vec::new();
    for (range, style) in &runs {
        let Some(link) = &style.link else { continue };
        let target = links::normalize_destination(&link.destination);
        if !links::is_openable(&target) {
            continue;
        }
        match declared.last_mut() {
            // A label split into runs is still one link.
            Some(previous) if previous.range.end == range.start && previous.target == target => {
                previous.range.end = range.end;
                previous.text = plain[previous.range.clone()].to_string();
            }
            _ => declared.push(links::LinkSpan {
                range: range.clone(),
                text: plain[range.clone()].to_string(),
                target,
            }),
        }
    }
    let linked = links::merge(declared, links::detect(&plain));

    // 3. The assistant's tokens.
    let mut marks: Vec<(Range<usize>, Mark)> = assistant::ranges(&plain)
        .into_iter()
        .map(|range| (range, Mark::Assistant))
        .collect();
    if draw_source.is_some_and(|raw| assistant::draw_token_range(raw).is_some()) {
        if let Some(range) = assistant::draw_token_range(&plain) {
            marks.push((range, Mark::Assistant));
        }
    }

    // 4. Mentions, never under a link.
    let wire: Vec<mentions::Member> = named
        .iter()
        .map(|mention| mentions::Member {
            user_id: mention.user_id,
            name: &mention.name,
        })
        .collect();
    for token in mentions::tokens(&plain, &wire) {
        let under_link = linked
            .iter()
            .any(|link| link.range.start < token.range.end && token.range.start < link.range.end);
        let under_mark = marks
            .iter()
            .any(|(range, _)| range.start < token.range.end && token.range.start < range.end);
        if !under_link && !under_mark {
            marks.push((token.range, Mark::Mention(token.member.user_id)));
        }
    }

    // Cut the block at every boundary any of it has.
    let mut cuts: Vec<usize> = vec![0, plain.len()];
    cuts.extend(runs.iter().flat_map(|(range, _)| [range.start, range.end]));
    cuts.extend(
        linked
            .iter()
            .flat_map(|link| [link.range.start, link.range.end]),
    );
    cuts.extend(marks.iter().flat_map(|(range, _)| [range.start, range.end]));
    cuts.retain(|cut| plain.is_char_boundary(*cut));
    cuts.sort_unstable();
    cuts.dedup();

    let mut out = Vec::with_capacity(cuts.len());
    for window in cuts.windows(2) {
        let (start, end) = (window[0], window[1]);
        if start == end {
            continue;
        }
        let covers = |range: &Range<usize>| range.start <= start && end <= range.end;
        let mut style = runs
            .iter()
            .find(|(range, _)| covers(range))
            .map(|(_, style)| (*style).clone())
            .unwrap_or_default();
        let link = linked
            .iter()
            .find(|link| covers(&link.range))
            .map(|link| link.target.clone());
        let mark = marks
            .iter()
            .find(|(range, _)| covers(range))
            .map(|(_, mark)| mark.clone());
        if mark.is_some() {
            // The mark REPLACES the run's inline style with bold — `*@ai*`
            // loses its italic, `` `@ai` `` its code face — and keeps the font.
            style = Style {
                strong: true,
                font: style.font,
                ..Style::default()
            };
        }
        out.push(Piece {
            text: plain[start..end].to_string(),
            style,
            link,
            mark,
        });
    }
    out
}

/// A run's inline style, as elements.
fn styled(text: Html, style: &Style) -> Html {
    let mut html = text;
    if style.code {
        html = html! { <code>{ html }</code> };
    }
    if style.strikethrough {
        html = html! { <s>{ html }</s> };
    }
    if style.emphasis {
        html = html! { <em>{ html }</em> };
    }
    if style.strong {
        html = html! { <strong>{ html }</strong> };
    }
    match style.font {
        Font::Heading(level) => {
            let class = format!("h{}", level.clamp(1, 3));
            html! { <span class={class}>{ html }</span> }
        }
        Font::Monospaced => html! { <span class="mono">{ html }</span> },
        Font::Body => html,
    }
}

#[function_component(Body)]
pub fn body(props: &BodyProps) -> Html {
    // A link opens a beat late, and a double-click in that beat cancels it —
    // the second click of a double-click is a heart, not a second visit.
    let pending = use_mut_ref(|| Option::<Timeout>::None);
    let cancel = {
        let pending = pending.clone();
        Callback::from(move |_: MouseEvent| {
            pending.borrow_mut().take();
        })
    };
    let open = {
        let pending = pending.clone();
        move |target: String| {
            let pending = pending.clone();
            Callback::from(move |event: MouseEvent| {
                event.prevent_default();
                let target = target.clone();
                *pending.borrow_mut() = Some(Timeout::new(LINK_DELAY_MS, move || {
                    if let Some(window) = web_sys::window() {
                        let _ = window.open_with_url_and_target_and_features(
                            &target,
                            "_blank",
                            "noopener,noreferrer",
                        );
                    }
                }));
            })
        }
    };

    let draw_piece = |piece: &Piece| -> Html {
        let text = html! { { piece.text.clone() } };
        let inner = match &piece.mark {
            Some(Mark::Mention(user_id))
                if *user_id != props.my_user_id && props.member_ids.contains(user_id) =>
            {
                let open_direct = props.on_open_direct.clone();
                let user_id = *user_id;
                html! {
                    <button class="mention" title="Message them"
                        onclick={Callback::from(move |event: MouseEvent| {
                            event.stop_propagation();
                            open_direct.emit(user_id);
                        })}>
                        { styled(text, &piece.style) }
                    </button>
                }
            }
            Some(_) => html! { <span class="mention">{ styled(text, &piece.style) }</span> },
            None => styled(text, &piece.style),
        };
        match &piece.link {
            Some(target) => html! {
                <a href={target.clone()} target="_blank" rel="noopener noreferrer"
                   onclick={open(target.clone())}>{ inner }</a>
            },
            None => inner,
        }
    };

    let blocks = markdown::blocks(&props.text);
    html! {
        <div class="md" ondblclick={cancel}>
            { for blocks.iter().enumerate().map(|(index, block)| match block {
                Block::Text(text) => {
                    let draw_source = (index == 0).then_some(props.text.as_str());
                    html! {
                        <p>{ for pieces(text, draw_source, &props.mentions).iter().map(draw_piece) }</p>
                    }
                }
                Block::Table(table) => table_html(table),
            }) }
        </div>
    }
}

/// A cell: its runs' styles and nothing else — no links, no marks.
fn cell_html(text: &Text) -> Html {
    html! {
        for text.spans.iter().map(|span| styled(html! { { span.text.clone() } }, &Style {
            link: None,
            ..span.style.clone()
        }))
    }
}

fn align(alignment: ColumnAlignment) -> &'static str {
    match alignment {
        ColumnAlignment::Leading => "text-align:start",
        ColumnAlignment::Center => "text-align:center",
        ColumnAlignment::Trailing => "text-align:end",
    }
}

/// A table: the header bold over a rule, columns sharing the width, cells
/// wrapping (ios MessageBodyView).
fn table_html(table: &Table) -> Html {
    html! {
        <div class="table-wrap">
            <table>
                <thead>
                    <tr>
                        { for table.header.iter().enumerate().map(|(column, cell)| html! {
                            <th style={align(table.alignment(column))}>{ cell_html(cell) }</th>
                        }) }
                    </tr>
                </thead>
                <tbody>
                    { for table.rows.iter().map(|row| html! {
                        <tr>
                            { for row.iter().enumerate().map(|(column, cell)| html! {
                                <td style={align(table.alignment(column))}>{ cell_html(cell) }</td>
                            }) }
                        </tr>
                    }) }
                </tbody>
            </table>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    fn named(user_id: i64, name: &str) -> Mention {
        Mention {
            user_id,
            name: name.into(),
        }
    }

    fn laid_out(body: &str, mentions: &[Mention]) -> Vec<Piece> {
        let blocks = markdown::blocks(body);
        match &blocks[0] {
            Block::Text(text) => pieces(text, Some(body), mentions),
            Block::Table(_) => Vec::new(),
        }
    }

    fn find<'a>(pieces: &'a [Piece], text: &str) -> &'a Piece {
        pieces
            .iter()
            .find(|piece| piece.text == text)
            .unwrap_or_else(|| panic!("no piece {text:?} in {pieces:?}"))
    }

    #[wasm_bindgen_test]
    fn markdown_styles_reach_the_pieces() {
        let pieces = laid_out("**bold** and `code` and ~~gone~~", &[]);
        assert!(find(&pieces, "bold").style.strong);
        assert!(find(&pieces, "code").style.code);
        assert!(find(&pieces, "gone").style.strikethrough);
    }

    /// A detected link is a link; the author's markdown destination wins
    /// over anything detected in its label.
    #[wasm_bindgen_test]
    fn links_are_found_and_the_authors_destination_wins() {
        let pieces = laid_out("see https://example.com/x now", &[]);
        assert_eq!(
            find(&pieces, "https://example.com/x").link.as_deref(),
            Some("https://example.com/x")
        );
        let labelled = laid_out("[https://a.com](https://b.com)", &[]);
        assert_eq!(
            labelled[0].link.as_deref(),
            Some("https://b.com"),
            "{labelled:?}"
        );
        let schemeless = laid_out("[shop](example.com)", &[]);
        assert_eq!(schemeless[0].link.as_deref(), Some("https://example.com"));
    }

    /// In a browser, a link that is not http(s), mail or a phone number is
    /// a way to run script. It never becomes one.
    #[wasm_bindgen_test]
    fn only_web_mail_and_phone_links_are_links() {
        for body in [
            "[click](javascript:alert(1))",
            "javascript://%0Aalert(1)",
            "[x](data:text/html,hi)",
        ] {
            assert!(
                laid_out(body, &[]).iter().all(|piece| piece.link.is_none()),
                "{body}"
            );
        }
        let mail = laid_out("write to me@example.com", &[]);
        assert_eq!(
            find(&mail, "me@example.com").link.as_deref(),
            Some("mailto:me@example.com")
        );
    }

    /// The assistant's tokens are bold and REPLACE the run's style — and
    /// `/draw` is marked only as the body's first word.
    #[wasm_bindgen_test]
    fn the_assistants_tokens_are_marked_the_apps_way() {
        let pieces = laid_out("ask *@ai* please", &[]);
        let ai = find(&pieces, "@ai");
        assert_eq!(ai.mark, Some(Mark::Assistant));
        assert!(
            ai.style.strong && !ai.style.emphasis,
            "the italic is replaced"
        );

        // Swift's own vectors (MessageLinksTests.drawMarkComesFromTheRawBody).
        for (body, marked) in [
            ("**/draw** a cat", &[][..]),
            ("*/draw* a cat", &[][..]),
            ("`/draw` a cat", &[][..]),
            ("~~/draw~~ a cat", &[][..]),
            ("# /draw a cat", &[][..]),
            ("- /draw a cat", &[][..]),
            ("/draw a cat", &["/draw"][..]),
            ("/draw a **fluffy** cat", &["/draw"][..]),
            ("@ai /draw a cat", &["@ai", "/draw"][..]),
        ] {
            let got: Vec<String> = laid_out(body, &[])
                .into_iter()
                .filter(|piece| piece.mark == Some(Mark::Assistant))
                .map(|piece| piece.text)
                .collect();
            assert_eq!(got, marked, "{body}");
        }
    }

    /// Only the first block of a table-split body may carry `/draw`.
    #[wasm_bindgen_test]
    fn only_the_first_block_carries_the_picture_mark() {
        let body = "| a |\n| --- |\n| 1 |\n/draw a cat";
        for (index, block) in markdown::blocks(body).iter().enumerate() {
            if let Block::Text(text) = block {
                let draw_source = (index == 0).then_some(body);
                assert!(
                    pieces(text, draw_source, &[])
                        .iter()
                        .all(|piece| piece.mark.is_none()),
                    "block {index}"
                );
            }
        }
    }

    /// Mentions come last and never inside a link.
    #[wasm_bindgen_test]
    fn mentions_are_marked_unless_a_link_covers_them() {
        let pieces = laid_out("**@Anna** dinner?", &[named(9, "Anna")]);
        let anna = find(&pieces, "@Anna");
        assert_eq!(anna.mark, Some(Mark::Mention(9)));
        assert!(anna.style.strong);

        let linked = laid_out("[@Anna](https://x.com)", &[named(9, "Anna")]);
        assert!(
            linked.iter().all(|piece| piece.mark.is_none()),
            "{linked:?}"
        );
    }

    /// Every piece boundary is a character boundary, in any script.
    #[wasm_bindgen_test]
    fn pieces_never_cut_inside_a_character() {
        let body = "Привет @Ана\u{301}, **смотри** https://пример.рф/путь 😀 @ai 🇷🇸";
        let joined: String = laid_out(body, &[named(9, "Ана")])
            .into_iter()
            .map(|piece| piece.text)
            .collect();
        assert_eq!(joined, markdown::render(body).plain());
    }
}
