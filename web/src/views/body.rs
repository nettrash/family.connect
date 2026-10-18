//! A message's body, drawn: the markdown subset, links, `@ai` and `/draw`,
//! and member mentions — composed in exactly the order the Apple app
//! composes them (fc_text::markdown's "Composition"), so a message reads the
//! same on every device.
//!
//! 1. markdown makes styled runs — only a table or a rich block (a formula,
//!    a diagram or a graph on its own, issue #97) splits a body into blocks;
//! 2. per text block, over its RENDERED characters: the author's markdown
//!    links (only those that can be opened), then detected links, emails and
//!    phone numbers wherever no link already is;
//! 3. `@ai`, and a leading `/draw` in the first block only, drawn bold —
//!    replacing the run's own style, inside links and code too;
//! 4. member mentions last, skipped wherever a link covers them.
//!
//! An inline formula is DRAWN, not set in type, so 2–4 lay nothing over one:
//! a link, an `@ai` or a mention that would touch a formula's run is not
//! made (docs/protocol.md, rich blocks rule 4: "a link, mention or `@ai`
//! mark that overlaps a formula is not one"). The author's own links never
//! touch one — the parser flattens a formula in a link's label to text — so
//! the filter is applied AFTER the merge, and step 2's declared links are
//! exactly what they were (win/tools/board-oracle's `declared_links` prints
//! them). Windows does the same (MessageBody.cs `Pieces`).
//!
//! Table cells get none of 2–4. A body is plain text on the wire; all of
//! this is drawing (docs/protocol.md, "A body is plain text on the wire").

use std::collections::HashSet;
use std::ops::Range;

use fc_text::i18n::t;
use fc_text::markdown::{self, Block, ColumnAlignment, Font, Style, Table, Text};
use fc_text::{assistant, links, mentions};
use gloo_timers::callback::Timeout;
use yew::prelude::*;

use crate::model::Mention;
use crate::views::rich::{MathInline, RichBlock};

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
    // Where the formulas are. Nothing below may lay anything over one.
    let formulas: Vec<Range<usize>> = runs
        .iter()
        .filter(|(_, style)| style.math.is_some())
        .map(|(range, _)| range.clone())
        .collect();
    let touches_formula = |range: &Range<usize>| {
        formulas
            .iter()
            .any(|formula| formula.start < range.end && range.start < formula.end)
    };
    let mut linked = links::merge(declared, links::detect(&plain));
    // A URL inside `$…$` is part of what is drawn, never a link.
    linked.retain(|link| !touches_formula(&link.range));

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
    marks.retain(|(range, _)| !touches_formula(range));

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
        if !under_link && !under_mark && !touches_formula(&token.range) {
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
        // An inline formula: nothing is laid over it (`pieces`), so it is
        // its run, whole, in that run's own style — `*see $a*b$*` draws it
        // inside the emphasis.
        if let Some(formula) = &piece.style.math {
            let math = html! { <MathInline source={formula.clone()} typed={piece.text.clone()} /> };
            return styled(math, &piece.style);
        }
        let text = html! { { piece.text.clone() } };
        let inner = match &piece.mark {
            Some(Mark::Mention(user_id))
                if *user_id != props.my_user_id && props.member_ids.contains(user_id) =>
            {
                let open_direct = props.on_open_direct.clone();
                let user_id = *user_id;
                html! {
                    <button class="mention" title={t("Message them")}
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
                // Issue #97: drawn by fc-rich.js (views/rich.rs); what was
                // typed, in the code face, until then.
                Block::Rich(rich) => html! { <RichBlock rich={rich.clone()} /> },
            }) }
        </div>
    }
}

/// A cell: its runs' styles and nothing else — no links, no marks.
///
/// A formula in a cell is drawn as one in a line is: it is markdown's, like
/// the cell's bold, not a decoration laid over it.
fn cell_html(text: &Text) -> Html {
    html! {
        for text.spans.iter().map(|span| {
            let inner = match &span.style.math {
                Some(formula) => html! { <MathInline source={formula.clone()} typed={span.text.clone()} /> },
                None => html! { { span.text.clone() } },
            };
            styled(inner, &Style {
                link: None,
                ..span.style.clone()
            })
        })
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
            Block::Table(_) | Block::Rich(_) => Vec::new(),
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

    /// An inline formula is drawn, so nothing is laid over it: a URL, an
    /// `@ai` or a mention inside `$…$` is part of the formula and not a
    /// link or a mark (docs/protocol.md, rich blocks rule 4) — while the
    /// same things beside a formula are what they always were.
    #[wasm_bindgen_test]
    fn nothing_is_laid_over_a_formula() {
        let anna = [named(9, "Anna")];
        for body in [
            "see $https://example.com/x$ now",
            "ask $@ai$ now",
            "hi $@Anna$ now",
            "$a@b.co$",
            "$x + https://a.com$",
        ] {
            let pieces = laid_out(body, &anna);
            let formula: Vec<&Piece> = pieces
                .iter()
                .filter(|piece| piece.style.math.is_some())
                .collect();
            assert_eq!(formula.len(), 1, "{body}: {pieces:?}");
            assert!(
                pieces
                    .iter()
                    .all(|piece| piece.link.is_none() && piece.mark.is_none()),
                "{body}: {pieces:?}"
            );
            // The run reaches the bubble whole, as typed.
            assert!(body.contains(&formula[0].text), "{body}");
        }
        let beside = laid_out("@ai see https://example.com $x$ @Anna", &anna);
        assert_eq!(find(&beside, "@ai").mark, Some(Mark::Assistant));
        assert_eq!(
            find(&beside, "https://example.com").link.as_deref(),
            Some("https://example.com")
        );
        assert_eq!(find(&beside, "@Anna").mark, Some(Mark::Mention(9)));
        assert_eq!(find(&beside, "$x$").style.math.as_deref(), Some("x"));
        // A link the detector would stretch INTO a formula is dropped
        // whole, not cut: half a URL is not a place to go.
        // (The detector reads `example.com/$x$` as one address.)
        let touching = laid_out("see example.com/$x$ now", &anna);
        assert!(
            touching.iter().all(|piece| piece.link.is_none()),
            "{touching:?}"
        );
    }

    /// `body` in a bubble at the end of a chat's scrolling list — with the
    /// shipped stylesheet, which is what hides a fallback once drawn.
    /// Returns (list, bubble).
    fn mount(body: &str) -> (web_sys::Element, web_sys::Element) {
        crate::layout_tests::install_stylesheet();
        let document = web_sys::window().unwrap().document().unwrap();
        let list = document.create_element("div").unwrap();
        list.set_class_name("messages");
        list.set_attribute(
            "style",
            "position:fixed;top:0;left:0;width:320px;height:200px;display:block;padding:0",
        )
        .unwrap();
        let earlier = document.create_element("div").unwrap();
        earlier.set_attribute("style", "height:600px").unwrap();
        list.append_child(&earlier).unwrap();
        let bubble = document.create_element("article").unwrap();
        bubble.set_class_name("bubble");
        list.append_child(&bubble).unwrap();
        document.body().unwrap().append_child(&list).unwrap();
        let props = BodyProps {
            text: body.into(),
            mentions: Vec::new(),
            my_user_id: 7,
            member_ids: HashSet::new(),
            on_open_direct: Callback::noop(),
        };
        yew::Renderer::<Body>::with_root_and_props(bubble.clone(), props).render();
        (list, bubble)
    }

    fn visible_fallbacks(root: &web_sys::Element) -> Vec<String> {
        use wasm_bindgen::JsCast;
        let found = root.query_selector_all(".rich-fallback").unwrap();
        (0..found.length())
            .filter_map(|index| found.item(index))
            .filter_map(|node| node.dyn_into::<web_sys::HtmlElement>().ok())
            .filter(|element| element.offset_width() > 0)
            .map(|element| element.text_content().unwrap_or_default())
            .collect()
    }

    /// THE REAL RENDERER, end to end: wasm-bindgen's test server serves the
    /// crate's own folder beside the test page, so `rich/fc-rich.js?v=…`
    /// and the engines it loads are the shipped files.
    ///
    /// Until a block is drawn it shows what was typed, in the code face — a
    /// display formula with its delimiters, an inline one its own text —
    /// and the moment it is drawn, the drawing replaces it in one step: a
    /// formula by KaTeX (with MathML, for a screen reader) in the words'
    /// colour on nothing, a diagram or a graph on the theme's card, scaled
    /// down to the bubble. A block the engine refuses keeps its source. And
    /// a reader at the newest message is still there when the bubble has
    /// grown by its drawings.
    #[wasm_bindgen_test]
    async fn rich_blocks_show_what_was_typed_until_the_renderer_draws_them() {
        use wasm_bindgen::JsCast;
        let wide = (0..14)
            .map(|node| format!("n{node}"))
            .collect::<Vec<_>>()
            .join(" -> ");
        let body = format!(
            "Look:\n$$\n\\frac{{a}}{{b}}\n$$\n```mermaid\ngraph TD\nA-->B\n```\n\
             ```dot\ndigraph {{ rankdir=LR; {wide} }}\n```\n```plot\nsin(x)\n```\n\
             ```neato\ngraph {{\n```\nArea $\\pi r^2$ here"
        );
        let (list, root) = mount(&body);
        // Drawn by Yew on its next turn — before fc-rich.js, injected by
        // that same render, can have loaded, let alone an engine after it:
        // every block as typed.
        gloo_timers::future::TimeoutFuture::new(0).await;
        // And the reader at the newest message.
        list.set_scroll_top(list.scroll_height());
        let typed = [
            "$$\n\\frac{a}{b}\n$$".to_string(),
            "graph TD\nA-->B".to_string(),
            format!("digraph {{ rankdir=LR; {wide} }}"),
            "sin(x)".to_string(),
            "graph {".to_string(),
            "$\\pi r^2$".to_string(),
        ];
        assert_eq!(visible_fallbacks(&root), typed);
        assert!(root.query_selector("[data-drawn]").unwrap().is_none());
        let asked = web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .query_selector(&format!(
                "script[src=\"{}\"]",
                crate::views::rich::script_url()
            ))
            .unwrap();
        assert!(
            asked.is_some(),
            "fc-rich.js is asked for under the hash of rich/"
        );

        // Drawn — as many as can be — within a generous bound: the whole
        // suite runs beside this test, video encoders and all, on the one
        // main thread the engines share.
        for _ in 0..600 {
            if root.query_selector_all("[data-drawn]").unwrap().length() == 5 {
                break;
            }
            gloo_timers::future::TimeoutFuture::new(50).await;
        }
        let state = || -> Vec<String> {
            let wraps = root
                .query_selector_all(".rich-block, .rich-inline")
                .unwrap();
            (0..wraps.length())
                .filter_map(|index| wraps.item(index))
                .filter_map(|node| node.dyn_into::<web_sys::Element>().ok())
                .map(|wrap| {
                    format!(
                        "{} {}",
                        wrap.class_name(),
                        if wrap.has_attribute("data-drawn") {
                            "drawn"
                        } else {
                            "typed"
                        }
                    )
                })
                .collect()
        };
        assert_eq!(
            root.query_selector_all("[data-drawn]").unwrap().length(),
            5,
            "{:?}",
            state()
        );
        // The broken graph keeps its source, and nothing is left behind on
        // a stage.
        assert_eq!(visible_fallbacks(&root), ["graph {"]);
        assert!(root.query_selector(".rich-stage *").unwrap().is_none());

        let drawn = |selector: &str| -> web_sys::HtmlElement {
            root.query_selector(&format!("{selector} > .rich-host > .fc-rich"))
                .unwrap()
                .unwrap_or_else(|| panic!("nothing drawn for {selector}"))
                .dyn_into()
                .unwrap()
        };
        // Formulas: KaTeX, with MathML, in the words' colour, on nothing.
        let display = drawn(".rich-math");
        assert!(display.query_selector(".katex-display").unwrap().is_some());
        assert!(display.query_selector("math").unwrap().is_some(), "MathML");
        let inline = drawn(".rich-inline");
        assert!(inline.query_selector(".katex").unwrap().is_some());
        let words = web_sys::window()
            .unwrap()
            .get_computed_style(&root)
            .unwrap()
            .unwrap()
            .get_property_value("color")
            .unwrap();
        assert_eq!(inline.style().get_property_value("color").unwrap(), words);
        assert_eq!(
            inline
                .style()
                .get_property_value("background-color")
                .unwrap(),
            "transparent"
        );
        // Diagrams and graphs: on the card, in an SVG that fits the bubble.
        let bubble = root.get_bounding_client_rect().width();
        for kind in [".rich-mermaid", ".rich-graphviz", ".rich-plot"] {
            let card = drawn(kind);
            let ground = card.style().get_property_value("background-color").unwrap();
            assert!(
                ground == "rgb(255, 255, 255)" || ground == "rgb(28, 28, 30)",
                "{kind}: {ground}"
            );
            let svg = card.query_selector("svg").unwrap().expect("an svg");
            let width = svg.get_bounding_client_rect().width();
            assert!(
                width > 0.0 && width <= bubble,
                "{kind}: {width} in {bubble}"
            );
        }
        // The fourteen-node chain is far wider than the bubble: scaled down,
        // its height with it.
        let chain = drawn(".rich-graphviz")
            .query_selector("svg")
            .unwrap()
            .unwrap();
        let natural: f64 = chain.get_attribute("width").unwrap().parse().unwrap();
        assert!(natural > bubble, "{natural}");
        // The drawn graph is a control named by its source.
        let host = root
            .query_selector(".rich-graphviz > .rich-host")
            .unwrap()
            .unwrap();
        assert_eq!(host.get_attribute("role").as_deref(), Some("button"));
        assert_eq!(
            host.get_attribute("aria-label"),
            Some(format!("digraph {{ rankdir=LR; {wide} }}"))
        );
        // The reader who was at the newest message still is.
        assert!(
            list.scroll_top() + list.client_height() >= list.scroll_height() - 2,
            "{} + {} < {}",
            list.scroll_top(),
            list.client_height(),
            list.scroll_height()
        );
        list.remove();
    }

    /// A daily greeting that used the weather ends with the server's credit
    /// (docs/protocol.md, "Today's weather, for places the owner chose"):
    /// plain markdown in the body, drawn as a link a click opens — to
    /// exactly the URL the server wrote — in every language it is written
    /// in, and nothing else about the greeting is a link.
    #[wasm_bindgen_test]
    async fn a_greetings_weather_credit_is_a_link_to_open_meteo() {
        use wasm_bindgen::JsCast;
        let document = web_sys::window().unwrap().document().unwrap();
        for (body, label) in [
            (
                "Good morning! Belgrade, Serbia: sunny, 24°/13°.\n\n[Weather data by Open-Meteo.com](https://open-meteo.com/)",
                "Weather data by Open-Meteo.com",
            ),
            (
                "Доброе утро! Москва, Россия: облачно, +12°.\n\n[Данные о погоде: Open-Meteo.com](https://open-meteo.com/)",
                "Данные о погоде: Open-Meteo.com",
            ),
        ] {
            let root = document.create_element("div").unwrap();
            document.body().unwrap().append_child(&root).unwrap();
            let props = BodyProps {
                text: body.into(),
                mentions: Vec::new(),
                my_user_id: 7,
                member_ids: HashSet::new(),
                on_open_direct: Callback::noop(),
            };
            yew::Renderer::<Body>::with_root_and_props(root.clone(), props).render();
            gloo_timers::future::TimeoutFuture::new(20).await;
            let links = root.query_selector_all("a").unwrap();
            assert_eq!(links.length(), 1, "{}", root.inner_html());
            let link: web_sys::Element = links.item(0).unwrap().dyn_into().unwrap();
            assert_eq!(link.get_attribute("href").as_deref(), Some("https://open-meteo.com/"));
            assert_eq!(link.text_content().as_deref(), Some(label));
            assert!(!root.text_content().unwrap().contains("]("), "no markdown shows");
            root.remove();
        }
    }
}
