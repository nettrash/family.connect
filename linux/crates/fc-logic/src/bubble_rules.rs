//! What a bubble is, beyond its words (`BubbleRules.cs`) — the web client's `timeline` and bubble
//! rules (ios `MessagePresentation`): who counts as the assistant, an answer still being written,
//! the seen tick, and who may be reported or blocked from a message.

use fc_core::protocol::MessageDto;

/// The assistant: its reserved account in the family chat, and anybody but the reader in its own
/// chat.
pub fn is_assistant(
    message: &MessageDto,
    me: i64,
    assistant_chat: bool,
    assistant_user_id: Option<i64>,
) -> bool {
    Some(message.sender_id) == assistant_user_id || (assistant_chat && message.sender_id != me)
}

/// A member other than the reader — who the Safety items are about. Never the assistant.
pub fn is_other_member(
    message: &MessageDto,
    me: i64,
    assistant_chat: bool,
    assistant_user_id: Option<i64>,
) -> bool {
    message.sender_id != me && !is_assistant(message, me, assistant_chat, assistant_user_id)
}

/// An assistant answer not written yet: asked of the MESSAGE and nothing else — a numbered row
/// carrying nothing, not the reader's, from somebody who can be the assistant. A set of ids a live
/// delta touched would forget the state on relaunch, which is the blank bubble this rule exists to
/// remove. `body` is the body as drawn, streamed text included.
pub fn awaited(
    message: &MessageDto,
    body: &str,
    me: i64,
    assistant_chat: bool,
    assistant_user_id: Option<i64>,
) -> bool {
    body.is_empty()
        && message.media().is_empty()
        && message.poll.is_none()
        && message.call.is_none()
        && message.sender_id != me
        && message.id != 0
        && (assistant_chat || Some(message.sender_id) == assistant_user_id)
}

/// Whether the reader's own message carries a tick: a numbered one, anywhere but the family chat or
/// a group (`ChatKinds::is_shared`) — where a seen state over many members is a row of faces
/// nobody asked for (docs/protocol.md, "read" frames; "Groups": "Seen" is not drawn, as in the
/// family chat).
pub fn shows_tick(message: &MessageDto, me: i64, shared_chat: bool) -> bool {
    !shared_chat && message.sender_id == me && message.id != 0
}

/// The tick is the double one: the other person's marker has reached it.
pub fn seen(message: &MessageDto, me: i64, shared_chat: bool, peer_read_up_to: i64) -> bool {
    shows_tick(message, me, shared_chat) && message.id <= peer_read_up_to
}

/// A message may be reported once it is numbered, and only another member's — hidden or not.
pub fn may_report(
    message: &MessageDto,
    me: i64,
    assistant_chat: bool,
    assistant_user_id: Option<i64>,
) -> bool {
    message.id != 0 && is_other_member(message, me, assistant_chat, assistant_user_id)
}

/// An ASSISTANT reply may be reported, once it is numbered — a separate path from [`may_report`]
/// and deliberately so (docs/protocol.md, "Reporting the assistant"). The assistant belongs to no
/// family, so the member-report endpoint refuses it with `not_same_family`; what a member needs
/// here is to say that a MODEL got something wrong, which is the operator's business and not the
/// family owner's.
pub fn may_report_assistant(
    message: &MessageDto,
    me: i64,
    assistant_chat: bool,
    assistant_user_id: Option<i64>,
) -> bool {
    message.id != 0 && is_assistant(message, me, assistant_chat, assistant_user_id)
}

/// What a bubble's attachments look like, for a redraw key: which branch draws them (a sticker, a
/// circle, or the tiles and rows of everything else) and over which ids. The edit, reaction and
/// poll sequences say nothing about a set, so a key made of them alone kept a sticker an older
/// build cached as a grey photo tile on screen after the repair had put its flag back (schema step
/// 8) — until something else about the chat changed.
pub fn media_mark(message: &MessageDto) -> String {
    if let Some(sticker) = message.sticker_picture() {
        return format!("s{}", sticker.id);
    }
    if let Some(circle) = message.round_video() {
        return format!("r{}", circle.id);
    }
    let media = message.media();
    if media.is_empty() {
        return String::new();
    }
    let marks: Vec<String> = media
        .iter()
        .map(|attachment| {
            format!(
                "{}{}{}",
                attachment.id,
                attachment.kind,
                if attachment.has_preview { "+" } else { "-" }
            )
        })
        .collect();
    format!("m{}", marks.join("."))
}
