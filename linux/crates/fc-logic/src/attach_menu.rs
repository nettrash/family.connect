//! The paperclip's menu, the same on every client (docs/attachment-menu-2026-10-07.md, issue #78) —
//! `AttachMenu.cs`: three groups read top to bottom — what to pick (a photo or video, a file, the
//! clipboard), what to record, and the rest (a place, a poll) — with a separator between groups and
//! none around a group that is empty. Linux, like Windows, has no Camera line (S1.5 of
//! docs/audio-video-messages-2026-10-04.md). Only WHICH lines show is decided here; whether a line
//! is enabled (a call in progress, an unsent voice message, a busy composer, an edit) stays with the
//! view.
//!
//! Amended 2026-10-20 (#106, docs/composer-and-wide-layout-2026-10-20.md): the `@ai` button left the
//! composer's row, and "Ask the Assistant" opens the last group in the family chat — only on a
//! server with an assistant that names who answers ([`assistant_available`]), exactly where the
//! button was drawn.
//!
//! The menu itself is `fc_text::attach_menu`'s, called rather than copied — the same table the web
//! client draws — and so is [`AttachItem`], whose `label()` is the line's words in the reader's
//! language (`fc_text::i18n`).

use fc_core::protocol::AssistantDto;
use fc_text::attach_menu::{AttachOffers, attach_menu};

use crate::assistant_consent;

/// One line of the paperclip's menu ([`groups`]): `fc_text`'s own.
pub use fc_text::attach_menu::AttachItem;

/// What the "Photo or Video" picker offers: every picture and video type the client knows
/// (`fc_text::media::mime_for`, the C#'s `MediaPrep.MimeFor`) — the photos it re-draws, the GIF and
/// WebP that go with their own bytes, and the videos the server takes as videos. SVG is a drawing,
/// not a photo, and stays behind "File".
pub const PHOTO_OR_VIDEO_TYPES: &[&str] = &[
    ".jpg", ".jpeg", ".png", ".heic", ".heif", ".webp", ".gif", ".bmp", ".tif", ".tiff", ".avif",
    ".mp4", ".m4v", ".mov",
];

/// Whether this server's assistant can be asked at all: there is one, and it names who answers.
pub fn assistant_available(assistant: Option<&AssistantDto>) -> bool {
    assistant
        .is_some_and(|assistant| assistant_consent::is_available(assistant.processor.as_deref()))
}

/// The menu's lines, group by group, top to bottom. In the assistant's chat, "Show the Assistant a
/// Photo…" stands where "Photo or Video" does — and only where `offers_picture_attach` (the server
/// can see and the family allows it); never both lines. Nothing is recorded for the assistant, where
/// every message is a consented model call; a video message only where one can be recorded; "Ask
/// the Assistant" and a poll only in the family chat or a group (`chat_kinds::is_shared`;
/// docs/protocol.md, "Groups") — the first only where `assistant_available`: the server has an
/// assistant and names its processor.
pub fn groups(
    assistant_chat: bool,
    offers_picture_attach: bool,
    shared_chat: bool,
    round_available: bool,
    assistant_available: bool,
) -> Vec<Vec<AttachItem>> {
    attach_menu(AttachOffers {
        assistant_chat,
        assistant_pictures: assistant_chat && offers_picture_attach,
        record_voice: !assistant_chat,
        record_video: !assistant_chat && round_available,
        ask_assistant: shared_chat && assistant_available,
        poll: shared_chat,
    })
}
