//! A message body for a bubble (ios `MessageBodyView`, web `views/body.rs`) — `BubbleBody.cs`:
//! markdown, links, the assistant's tokens and member mentions, composed by
//! `fc_core::text::message_body`, and which mentions open a chat — the rule the plain runs had
//! ([`composer_mentions::opens_chat`]).
//!
//! Offsets never leave this module: a run is its own text, cut on character boundaries by the
//! composition.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Duration;

use fc_core::protocol::{MemberDto, MentionDto};
use fc_core::text::message_body::{self, BodyMark};
use fc_text::markdown::{self, Block, RichKind, Style, Table};
use fc_text::mentions::Member as Named;

use crate::composer_mentions;

/// One stretch of a text block with everything the window draws over it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyRun {
    pub text: String,
    pub style: Style,
    /// What a click opens — always http(s), mailto or tel.
    pub link: Option<String>,
    /// The member a mention names, when it is one.
    pub member_id: Option<i64>,
    /// A mention of somebody this reader can message: a door onto that chat.
    pub opens: bool,
    /// A mention or the assistant's `@ai` or `/draw`: drawn bold in the bubble's own ink.
    pub marked: bool,
}

impl BodyRun {
    /// An inline formula (issue #97): the source to draw (job kind `math-inline`), or `None` for
    /// words. [`text`](Self::text) is then what was typed, delimiters and all — what the bubble
    /// shows, monospaced, until the drawing lands or when it cannot be drawn. A formula run never
    /// carries a link or a mark.
    pub fn math(&self) -> Option<&str> {
        self.style.math.as_deref()
    }
}

/// A formula, a diagram or a graph standing on its own (issue #97; `fc_text::markdown::Rich`): what
/// to draw it with ([`RichKind::word`] is the job's `kind`; `engine` Graphviz's layout, empty
/// otherwise) and what to draw. Until it has been drawn — and for good when it cannot be — the
/// bubble shows [`shown`](Self::shown) in the monospaced face, as it shows a code block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyRichBlock {
    pub kind: RichKind,
    pub engine: String,
    pub source: String,
    /// The monospaced stand-in: the source, or a display formula's lines as typed with their
    /// delimiters. Never what is drawn.
    pub shown: String,
}

impl BodyRichBlock {
    /// `new BodyRichBlock(kind, engine, source)`: shown as its source.
    pub fn new(kind: RichKind, engine: &str, source: &str) -> Self {
        BodyRichBlock {
            kind,
            engine: engine.to_string(),
            source: source.to_string(),
            shown: source.to_string(),
        }
    }

    /// `… { Shown = shown }`.
    pub fn with_shown(mut self, shown: &str) -> Self {
        self.shown = shown.to_string();
        self
    }
}

/// A body as the window lays it out: text blocks, and tables and rich blocks between them (the
/// C#'s `BodyTextBlock`, `BodyTableBlock` and `BodyRichBlock`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyBlock {
    Text(Vec<BodyRun>),
    Table(Table),
    Rich(BodyRichBlock),
}

impl BodyBlock {
    /// The runs of a text block; `None` for a table or a rich block.
    pub fn runs(&self) -> Option<&[BodyRun]> {
        match self {
            BodyBlock::Text(runs) => Some(runs),
            _ => None,
        }
    }
}

/// How long a link waits before it opens, so a double-click on it can be the heart instead — the
/// Mac's 350 ms.
pub const LINK_DELAY: Duration = Duration::from_millis(350);

/// The body laid out for a bubble: `named` is the message's own `mentions`, `members` the roster a
/// name may open a chat with, `blocked` whom the reader blocked.
pub fn lay(
    body: &str,
    named: Option<&[MentionDto]>,
    members: &[MemberDto],
    me: i64,
    blocked: &dyn Fn(i64) -> bool,
) -> Vec<BodyBlock> {
    let mentions: Vec<Named<'_>> = named
        .unwrap_or_default()
        .iter()
        .map(|mention| Named {
            user_id: mention.user_id,
            name: &mention.name,
        })
        .collect();
    markdown::blocks(body)
        .into_iter()
        .enumerate()
        .map(|(index, block)| match block {
            Block::Table(table) => BodyBlock::Table(table),
            Block::Rich(rich) => BodyBlock::Rich(BodyRichBlock {
                kind: rich.kind,
                engine: rich.engine,
                source: rich.source,
                shown: rich.shown,
            }),
            // Only the first block may carry `/draw`: it is a request only at the start of the
            // whole body.
            Block::Text(text) => BodyBlock::Text(
                message_body::pieces(&text, (index == 0).then_some(body), &mentions)
                    .into_iter()
                    .map(|piece| {
                        let member_id = match piece.mark {
                            Some(BodyMark::Member(id)) => Some(id),
                            _ => None,
                        };
                        BodyRun {
                            text: piece.text,
                            style: piece.style,
                            link: piece.link,
                            member_id,
                            opens: member_id.is_some_and(|id| {
                                composer_mentions::opens_chat(id, members, me, blocked)
                            }),
                            marked: piece.mark.is_some(),
                        }
                    })
                    .collect(),
            ),
        })
        .collect()
}

/// How many bodies' preview links are remembered before the memory starts again (the C#'s 512).
const PREVIEW_LINKS_HELD: usize = 512;

thread_local! {
    /// Remembered per body, as the Mac does — every redraw asks again. One thread, the window's,
    /// so a thread-local and not a lock.
    static PREVIEW_LINKS: RefCell<HashMap<String, Option<String>>> = RefCell::new(HashMap::new());
}

/// The link a bubble's preview card describes: the first https link its body draws
/// (`message_body::first_web_link`), and none for a body of nothing but emoji, which draws no markup
/// to find one in. Remembered per body, as the Mac does — every redraw asks again.
pub fn preview_link(body: &str, emoji_only: bool) -> Option<String> {
    if emoji_only || body.is_empty() {
        return None;
    }
    PREVIEW_LINKS.with(|held| {
        if let Some(known) = held.borrow().get(body) {
            return known.clone();
        }
        let found = message_body::first_web_link(body);
        let mut held = held.borrow_mut();
        if held.len() >= PREVIEW_LINKS_HELD {
            held.clear();
        }
        held.insert(body.to_string(), found.clone());
        found
    })
}

/// A link target as the system opens it, or `None` when it is not an absolute http(s), mailto or
/// tel URI — `fc_text::links::is_openable` for the scheme, and, for http(s), an address with a host
/// to open (`has_host`). The C# answered a `Uri`; the shell hands this string to
/// `gtk::UriLauncher`.
pub fn openable(target: Option<&str>) -> Option<String> {
    target
        .filter(|target| fc_text::links::is_openable(target) && has_host(target))
        .map(str::to_string)
}

/// What the C#'s `Uri.TryCreate(target, UriKind.Absolute, …)` adds to the scheme test, as far as a
/// link can reach it: an http(s) address with no host to open is refused. Measured on .NET 10:
/// `https:`, `https://`, `https:/x`, `https:///x`, `http:example`, `https://:80/`, a host with a
/// space or a `%` in it (`http://a%41.com` too), an empty label (`https://a..b`), an unclosed
/// `[::1`, and a port above 65535 are refused; `https://example.com:` (an empty port), user info,
/// `[::1]`, `exa_mple.com`, `-a.com`, a space or a bad escape in the PATH or query, and an
/// upper-case scheme are taken. `mailto:` and `tel:` are taken as they are, even empty or with a
/// space (`mailto:`, `tel:555 1234`).
fn has_host(target: &str) -> bool {
    let Some((scheme, rest)) = target.split_once(':') else {
        return false;
    };
    let web = scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https");
    if !web {
        return true;
    }
    let Some(rest) = rest.strip_prefix("//") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_and_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let (host, port) = match host_and_port.strip_prefix('[') {
        Some(bracketed) => match bracketed.split_once(']') {
            Some((host, after)) if after.is_empty() || after.starts_with(':') => {
                (host, after.strip_prefix(':'))
            }
            _ => return false,
        },
        None => match host_and_port.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (host_and_port, None),
        },
    };
    !host.is_empty()
        && !host.contains("..")
        && !host.chars().any(|c| c.is_whitespace() || c == '%')
        && port.is_none_or(|port| port.is_empty() || port.parse::<u16>().is_ok())
}
