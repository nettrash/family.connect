//! A bubble's words as Pango markup (shell-design §5.2 "Body"): fc-logic lays a body out as runs
//! (`bubble_body::lay` — markdown, links, mentions, the assistant's tokens) and this module writes
//! each run as markup for one `gtk::Label` per text block, deciding nothing about the words. Pure:
//! tested without a display.
//!
//! - emphasis `<i>`, strong `<b>`, strikethrough `<s>`, code `<tt>`; a heading is bold and larger
//!   (the ladder `x-large` / `large` / bold), a fenced run monospaced;
//! - a link is `<a href>` only when `bubble_body::openable` takes it (http(s), mailto, tel with a
//!   host); anything else is drawn as its words;
//! - a mention or `@ai`/`/draw` (`marked`) is bold in the bubble's ink; a mention that opens a chat
//!   (`opens`) is a link to [`MEMBER_SCHEME`]`:<id>`, which the label's `activate-link` turns into
//!   the direct chat;
//! - raw HTML runs (`style.html`) are their own characters — every run is escaped.

use fc_logic::bubble_body::{self, BodyRun};
use fc_text::markdown::{Font, Span, Style};

/// The scheme of a mention's link: `fc-member:<user id>`.
pub const MEMBER_SCHEME: &str = "fc-member";

/// `text` escaped for Pango markup (`& < > " '`).
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// One run of styled words as markup, with an optional link around it.
fn styled(text: &str, style: &Style, link: Option<&str>, bold: bool) -> String {
    let mut inner = escape(text);
    if style.code || style.font == Font::Monospaced {
        inner = format!("<tt>{inner}</tt>");
    }
    if style.strikethrough {
        inner = format!("<s>{inner}</s>");
    }
    if style.emphasis {
        inner = format!("<i>{inner}</i>");
    }
    if style.strong || bold {
        inner = format!("<b>{inner}</b>");
    }
    if let Font::Heading(level) = style.font {
        let size = match level {
            1 => "x-large",
            2 => "large",
            _ => "medium",
        };
        inner = format!("<span size=\"{size}\" weight=\"bold\">{inner}</span>");
    }
    match link {
        Some(href) => format!("<a href=\"{}\">{inner}</a>", escape(href)),
        None => inner,
    }
}

/// A text block's runs as one markup string. A formula run (`math`) is written as what was typed,
/// monospaced — the bubble splits blocks that hold formulas before it gets here
/// ([`split_math`]), so this is only ever the fallback.
pub fn runs(runs: &[BodyRun]) -> String {
    let mut out = String::new();
    for run in runs {
        if run.math().is_some() {
            out.push_str(&format!("<tt>{}</tt>", escape(&run.text)));
            continue;
        }
        let link = match (run.opens, run.member_id) {
            (true, Some(id)) => Some(format!("{MEMBER_SCHEME}:{id}")),
            _ => bubble_body::openable(run.link.as_deref()),
        };
        out.push_str(&styled(&run.text, &run.style, link.as_deref(), run.marked));
    }
    out
}

/// A table cell's spans as markup (links drawn, never followed from a cell — the Mac's).
pub fn spans(spans: &[Span]) -> String {
    spans
        .iter()
        .map(|span| {
            let link = span
                .style
                .link
                .as_ref()
                .and_then(|link| bubble_body::openable(Some(&link.destination)));
            styled(&span.text, &span.style, link.as_deref(), false)
        })
        .collect()
}

/// The member a mention's link names, or `None` for any other link.
pub fn member_of(uri: &str) -> Option<i64> {
    uri.strip_prefix(MEMBER_SCHEME)?
        .strip_prefix(':')?
        .parse()
        .ok()
        .filter(|id: &i64| *id > 0)
}

/// A text block cut where its formulas are: words, formula, words … — so a formula can be drawn
/// as a picture between labels. A block with no formula is one piece.
pub enum Piece<'a> {
    Words(&'a [BodyRun]),
    /// `(source, typed)`: what is drawn, and what was typed (shown until drawn).
    Formula(&'a str, &'a str),
}

pub fn split_math(block: &[BodyRun]) -> Vec<Piece<'_>> {
    let mut pieces = Vec::new();
    let mut start = 0;
    for (index, run) in block.iter().enumerate() {
        if let Some(source) = run.math() {
            if start < index {
                pieces.push(Piece::Words(&block[start..index]));
            }
            pieces.push(Piece::Formula(source, &run.text));
            start = index + 1;
        }
    }
    if start < block.len() {
        pieces.push(Piece::Words(&block[start..]));
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;
    use fc_logic::bubble_body::{BodyBlock, lay};

    fn text_block(body: &str) -> Vec<BodyRun> {
        match lay(body, None, &[], 1, &|_| false).into_iter().next() {
            Some(BodyBlock::Text(runs)) => runs,
            other => panic!("not text: {other:?}"),
        }
    }

    #[test]
    fn markup_is_escaped_so_html_is_drawn_as_its_characters() {
        assert_eq!(escape("<b>&\"'"), "&lt;b&gt;&amp;&quot;&#39;");
        let markup = runs(&text_block("a <b>bold</b> & c"));
        assert!(markup.contains("&lt;b&gt;"), "{markup}");
        assert!(!markup.contains("<b>bold"), "{markup}");
    }

    #[test]
    fn emphasis_strong_code_and_strikethrough_become_pango_tags() {
        let markup = runs(&text_block("*it* **bo** `co` ~~st~~"));
        assert!(markup.contains("<i>it</i>"), "{markup}");
        assert!(markup.contains("<b>bo</b>"), "{markup}");
        assert!(markup.contains("<tt>co</tt>"), "{markup}");
        assert!(markup.contains("<s>st</s>"), "{markup}");
    }

    #[test]
    fn only_openable_links_are_links() {
        let markup = runs(&text_block("see https://example.com/a?b=1&c=2 now"));
        assert!(
            markup.contains("<a href=\"https://example.com/a?b=1&amp;c=2\">"),
            "{markup}"
        );
        let markup = runs(&text_block("[x](javascript:alert(1))"));
        assert!(!markup.contains("<a "), "{markup}");
    }

    #[test]
    fn a_mention_that_opens_links_to_the_member() {
        assert_eq!(member_of("fc-member:42"), Some(42));
        assert_eq!(member_of("fc-member:-1"), None);
        assert_eq!(member_of("https://x"), None);
        let run = BodyRun {
            text: "@Bob".into(),
            style: Style::default(),
            link: None,
            member_id: Some(7),
            opens: true,
            marked: true,
        };
        assert_eq!(runs(&[run]), "<a href=\"fc-member:7\"><b>@Bob</b></a>");
    }

    #[test]
    fn cyrillic_and_emoji_pass_through_untouched() {
        assert_eq!(runs(&text_block("Привет 😀")), "Привет 😀");
    }

    #[test]
    fn formulas_are_cut_out_of_their_block() {
        let block = text_block("area $x^2$ here");
        let pieces = split_math(&block);
        assert_eq!(pieces.len(), 3, "{block:?}");
        assert!(matches!(pieces[1], Piece::Formula("x^2", "$x^2$")));
        assert_eq!(split_math(&text_block("plain")).len(), 1);
    }
}
