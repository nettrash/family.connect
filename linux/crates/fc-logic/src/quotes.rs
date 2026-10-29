//! The two places a reply names what it answers: the quote over a bubble, and the banner over the
//! composer while a reply is being written (`Quotes.cs`).
//!
//! BOTH LEVELS, BECAUSE THE SERVER SENDS BOTH. A quote of a reply carries that reply's own quote —
//! one level, structurally capped — so answering an answer shows both halves of the exchange rather
//! than a snippet with no idea what it was responding to (docs/protocol.md, "Replies"). The
//! excerpts are the server's, cut the way `fc_text::excerpt` cuts.
//!
//! A BLOCKED MEMBER'S WORDS ARE NOT QUOTED EITHER — AT EITHER LEVEL. A level whose sender is blocked
//! draws as "Replying to a hidden message" — the same rule as their own bubble, one step removed,
//! because a quote is their words in somebody else's bubble — and reveals under the same one-tap
//! rule, each level on its own (docs/protocol.md, "Blocking a member"). The quoting message above
//! it is untouched and stays readable.

use fc_core::protocol::MessageDto;
use fc_core::store::{ChatStore, StoreResult};
use fc_core::text::string_catalog::StringCatalog;

/// Which level of a quote something is about — the answered message, or its own quote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuoteLevel {
    /// The message this reply answers.
    Reply,
    /// What THAT message was itself answering: one level under, and no deeper.
    Parent,
}

/// One level of the quote over a reply: who wrote it, what they said, and which message it is —
/// the id being how a click on it finds the message (docs/protocol.md, "Replies").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteLine {
    pub message_id: i64,
    pub name: String,
    pub excerpt: String,
    pub hidden: bool,
}

impl QuoteLine {
    pub fn new(message_id: i64, name: &str, excerpt: &str, hidden: bool) -> Self {
        QuoteLine {
            message_id,
            name: name.to_string(),
            excerpt: excerpt.to_string(),
            hidden,
        }
    }
}

/// The quote over a reply, as drawn: the message it answers, and one level under that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quote {
    pub reply: QuoteLine,
    pub parent: Option<QuoteLine>,
}

/// What a click on a quote does — which is not one thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteClick {
    /// Show the masked level naming the answered message.
    RevealReply,
    /// Show the masked level under that one.
    RevealParent,
    /// Nothing left masked: go to the message this reply answers.
    GoToMessage,
}

/// The quote over a bubble, or `None` when it answers nothing.
///
/// `revealed`: which hidden levels this reader has asked to see (the C#'s optional argument; pass
/// `|_| false` for none). Held by the surface, not by the bubble, so a reveal outlives the next
/// redraw.
pub fn of(
    message: &MessageDto,
    chats: &ChatStore,
    say: &dyn StringCatalog,
    revealed: impl Fn(QuoteLevel) -> bool,
) -> StoreResult<Option<Quote>> {
    let Some(quoted) = &message.reply_to else {
        return Ok(None);
    };
    let reply = line(
        quoted.message_id,
        quoted.sender_id,
        &quoted.excerpt,
        QuoteLevel::Reply,
        &revealed,
        chats,
        say,
    )?;
    let parent = match &quoted.parent {
        Some(parent) => Some(line(
            parent.message_id,
            parent.sender_id,
            &parent.excerpt,
            QuoteLevel::Parent,
            &revealed,
            chats,
            say,
        )?),
        None => None,
    };
    Ok(Some(Quote { reply, parent }))
}

/// What a click on the quote asks for. A MASKED LEVEL IS SHOWN BEFORE ANYTHING MOVES — outermost
/// first, then the one under it — which is the one-tap reveal the blocking rule gives every surface
/// that draws a quote (docs/protocol.md, "Blocking a member"); the same staged click the apps use.
/// Only a quote with nothing left masked goes to the message, so a click never jumps to a bubble
/// whose words the reader has chosen not to see.
pub fn click_on(quote: &Quote) -> QuoteClick {
    if quote.reply.hidden {
        QuoteClick::RevealReply
    } else if quote.parent.as_ref().is_some_and(|parent| parent.hidden) {
        QuoteClick::RevealParent
    } else {
        QuoteClick::GoToMessage
    }
}

/// The banner over the composer while replying to `message`.
pub fn banner(
    message: &MessageDto,
    chats: &ChatStore,
    say: &dyn StringCatalog,
) -> StoreResult<String> {
    Ok(if hides(message.sender_id, chats)? {
        say.get("Replying to a hidden message").to_string()
    } else {
        let name = name_of(message.sender_id, chats, say)?;
        say.format("Replying to %@", &[&name])
    })
}

fn line(
    message_id: i64,
    sender_id: i64,
    excerpt: &str,
    level: QuoteLevel,
    revealed: &impl Fn(QuoteLevel) -> bool,
    chats: &ChatStore,
    say: &dyn StringCatalog,
) -> StoreResult<QuoteLine> {
    Ok(if hides(sender_id, chats)? && !revealed(level) {
        let masked = match level {
            QuoteLevel::Reply => say.get("Replying to a hidden message"),
            QuoteLevel::Parent => say.get("which replied to a hidden message"),
        };
        QuoteLine::new(message_id, "", masked, true)
    } else {
        QuoteLine {
            message_id,
            name: name_of(sender_id, chats, say)?,
            excerpt: shown(message_id, excerpt, chats, say)?,
            hidden: false,
        }
    })
}

/// The server's excerpt, except that a quoted VIDEO MESSAGE — which has no words, so an empty
/// excerpt — says "Video message" (S5.7) when this device holds it and can tell. One it does not
/// hold stays as the server cut it.
fn shown(
    message_id: i64,
    excerpt: &str,
    chats: &ChatStore,
    say: &dyn StringCatalog,
) -> StoreResult<String> {
    if !excerpt.is_empty() {
        return Ok(excerpt.to_string());
    }
    Ok(match chats.message(message_id)? {
        Some(held) if held.round_video().is_some() => say.get("Video message").to_string(),
        // A caption-less ANIMATION says "GIF" where a video would say nothing (docs/protocol.md,
        // "Animations").
        Some(held) if held.animation_video().is_some() => say.get("GIF").to_string(),
        _ => excerpt.to_string(),
    })
}

fn hides(sender_id: i64, chats: &ChatStore) -> StoreResult<bool> {
    Ok(sender_id != chats.reader() && chats.is_blocked(sender_id)?)
}

fn name_of(user_id: i64, chats: &ChatStore, say: &dyn StringCatalog) -> StoreResult<String> {
    if user_id == chats.reader() {
        return Ok(say.get("You").to_string());
    }
    Ok(match chats.member(user_id)? {
        Some(member) if member.deleted => say.get("Deleted account").to_string(),
        Some(member) if !member.display_name.is_empty() => member.display_name,
        _ => say.get("Someone").to_string(),
    })
}
