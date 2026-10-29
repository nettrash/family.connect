//! The message row and the wide window (docs/composer-and-wide-layout-2026-10-20.md, issue #106) —
//! a port of `ComposerRow.cs`: one row of three controls — the paperclip, the field, the slot —
//! with the field's own glyphs inside its trailing edge; the conversation as a column no wider than
//! 1024, centred, its bubbles no wider than 560; and the least window that still leaves the field a
//! run of text to type in. What is decided here is WHICH glyphs and HOW WIDE; the view only draws
//! it.
//!
//! Units: Windows' effective pixels are GTK's logical pixels here (both are the platform's
//! scale-independent unit), so the names drop the C#'s `Epx` and the numbers stay the approved
//! design's — linux/docs/shell-design.md §5.1 reads `conversation_column::MAX_COLUMN`,
//! `PANE_INSET`, `BUBBLE_MAX` and `BUBBLE_SPACER` from here. Windows' Segoe Fluent code points
//! for the row's controls are not ported: [`FieldGlyph`] says WHICH control is drawn, and the shell
//! maps it (and the paperclip) to its own symbolic icons.

use fc_core::protocol::AssistantDto;

use crate::join_memory::AssistantButtons;
use crate::pack::pack_sending;

/// A control inside the field's trailing edge, in the order it is drawn, leading to trailing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldGlyph {
    /// The family's stickers (☺): wherever a sticker can be sent, empty or typing — it never moves
    /// under the pointer.
    Sticker,
    /// "Ask for a picture" (the paintbrush): the assistant's own chat, where it draws — still a
    /// button, beside the stickers.
    Picture,
    /// The video button (S1.4 of docs/audio-video-messages-2026-10-04.md): only while the field is
    /// empty and the slot a microphone.
    VideoDoor,
}

/// Every control of the row is a target this size; glyphs keep their own size inside it
/// (`fc_text::record::MIN_TARGET_WINDOWS_EPX`, Windows' 44 — and GTK's, shell-design §6.1).
pub const TARGET: u32 = fc_text::record::MIN_TARGET_WINDOWS_EPX;

/// What a field adds to its style's padding above and below, so one line stands as tall as the
/// targets beside it and centred on them — the field bottom-aligned with the controls, its last
/// line level with theirs as it grows. (WinUI's measure; the GTK field is the shell's to fit,
/// shell-design §6.1.)
pub const FIELD_LIFT: u32 = 5;

/// The glyphs inside the conversation's field, leading to trailing: the stickers where
/// [`pack_sending::offered`] has them, the paintbrush where [`AssistantButtons::offered`] does, and
/// the video button where `video_door` (`fc_text::record::video_door`) shows it — never the
/// paintbrush and the video button together, which are the assistant's chat and the others'.
pub fn in_field(
    chat_kind: Option<&str>,
    editing: bool,
    pack_offered: bool,
    assistant: Option<&AssistantDto>,
    video_door: bool,
) -> Vec<FieldGlyph> {
    let mut glyphs = Vec::new();
    if pack_sending::offered(chat_kind, editing, pack_offered) {
        glyphs.push(FieldGlyph::Sticker);
    }
    if AssistantButtons::offered(chat_kind, assistant, editing).ask_picture {
        glyphs.push(FieldGlyph::Picture);
    } else if video_door {
        glyphs.push(FieldGlyph::VideoDoor);
    }
    glyphs
}

/// The glyphs inside the thread's field: the stickers, wherever that composer can send.
pub fn in_thread_field(has_root: bool, pack_offered: bool) -> Vec<FieldGlyph> {
    if pack_sending::offered_in_thread(has_root, pack_offered) {
        vec![FieldGlyph::Sticker]
    } else {
        Vec::new()
    }
}

/// The trailing room a field gives its glyphs, over its own padding: one target per glyph, so text
/// never runs under a button.
pub fn trailing_room(glyphs: &[FieldGlyph]) -> u32 {
    u32::try_from(glyphs.len())
        .unwrap_or(u32::MAX)
        .saturating_mul(TARGET)
}

/// The thread's Send: a reply needs a root to answer and something to say (`IsNullOrWhiteSpace`,
/// whose whitespace is Rust's `char::is_whitespace`).
pub fn thread_send_enabled(has_root: bool, draft: Option<&str>) -> bool {
    has_root && draft.is_some_and(|draft| !draft.trim().is_empty())
}

/// Where the chat list, the conversation and the thread sit across the window
/// ([`conversation_column::arrange`]): the chat list's width, between
/// [`LIST_MIN`](conversation_column::LIST_MIN) and [`LIST`](conversation_column::LIST); whether the
/// thread sits beside the conversation (false is OVER it, when beside would starve the field); and
/// what the conversation's pane is left with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaneArrangement {
    pub list_width: f64,
    pub thread_beside: bool,
    pub pane_width: f64,
}

/// The wide rule (section 2.2) and the least window (section 3, fix 8): the conversation column —
/// messages AND composer — is `min(pane − 2×16, 1024)` and centred, a bubble at most
/// `min(column − 48, 560)`, and the window never so narrow that the field's run of text drops under
/// [`MIN_TEXT_RUN`](conversation_column::MIN_TEXT_RUN).
///
/// **THE BUBBLE'S RULE IS ITS MARGIN AND ITS CAP**: a bubble keeps
/// [`BUBBLE_SPACER`](conversation_column::BUBBLE_SPACER) clear on the side it does not lean to and
/// is capped at [`BUBBLE_MAX`](conversation_column::BUBBLE_MAX), which inside a column is exactly
/// [`bubble_max`](conversation_column::bubble_max) — so a resize moves no bubble by hand, and the
/// incremental list (`conversation_rows`) is never redrawn for one.
///
/// **THE FIELD'S FLOOR IS ARITHMETIC**, not a guess: the pane's insets, the card's border and
/// padding, the paperclip, the slot and the spacing between them, the field's own chrome, the two
/// glyphs a field carries at most, and the run of text. Below it the chat list narrows to
/// [`LIST_MIN`](conversation_column::LIST_MIN), the thread goes over the conversation instead of
/// beside it, and the window will not be made narrower than
/// [`MINIMUM_WINDOW_WIDTH`](conversation_column::MINIMUM_WINDOW_WIDTH).
///
/// The chrome numbers ([`CARD_CHROME`](conversation_column::CARD_CHROME),
/// [`FIELD_CHROME`](conversation_column::FIELD_CHROME), [`RAIL`](conversation_column::RAIL),
/// [`FRAME`](conversation_column::FRAME)) are WinUI's, as measured for Windows; the Linux window
/// has no rail and sets its own minimum (shell-design §3.1: 420 wide, one pane at a time below
/// 680), so the shell takes the column, the bubble and [`arrange`](conversation_column::arrange)
/// from here and its window minimum from its own design.
pub mod conversation_column {
    use super::{PaneArrangement, TARGET};

    /// The clear margin either side of the column.
    pub const PANE_INSET: f64 = 16.0;

    /// The widest a column grows.
    pub const MAX_COLUMN: f64 = 1024.0;

    /// The widest a bubble grows — a readable line on a wide window.
    pub const BUBBLE_MAX: f64 = 560.0;

    /// What a bubble leaves clear on the side it does not lean to.
    pub const BUBBLE_SPACER: f64 = 48.0;

    /// The chat list as wide as it is drawn, and the least it narrows to before anything else
    /// gives.
    pub const LIST: f64 = 300.0;
    pub const LIST_MIN: f64 = 260.0;

    /// The line between the list and the conversation.
    pub const DIVIDER: f64 = 1.0;

    /// The thread panel beside the conversation, its own left line included.
    pub const THREAD: f64 = 400.0;

    /// Windows' navigation rail on the window's left edge (LeftCompact).
    pub const RAIL: f64 = 48.0;

    /// Windows' frame and resize borders, which the outer size counts and the content never gets.
    pub const FRAME: f64 = 16.0;

    /// The least run of text the conversation's field keeps.
    pub const MIN_TEXT_RUN: f64 = 200.0;

    /// The composer card's border and padding, either side.
    pub const CARD_CHROME: f64 = 1.0 + 6.0;

    /// The spacing between the paperclip, the field and the slot.
    pub const ROW_SPACING: f64 = 6.0;

    /// The field's own border and its style's padding (WinUI's TextControlThemePadding: 10
    /// leading, 6 trailing).
    pub const FIELD_CHROME: f64 = 1.0 + 10.0 + 6.0 + 1.0;

    /// The most glyphs a field carries at once: the stickers and the paintbrush, or the stickers
    /// and the video button.
    pub const MOST_FIELD_GLYPHS: u32 = 2;

    const TARGET_WIDTH: f64 = TARGET as f64;

    /// The least pane that leaves the field [`MIN_TEXT_RUN`].
    pub const MIN_PANE: f64 = (2.0 * PANE_INSET)
        + (2.0 * CARD_CHROME)
        + TARGET_WIDTH
        + ROW_SPACING
        + ROW_SPACING
        + TARGET_WIDTH
        + FIELD_CHROME
        + (MOST_FIELD_GLYPHS as f64 * TARGET_WIDTH)
        + MIN_TEXT_RUN;

    /// The least window, outer size, with the list at its narrowest and the thread over the
    /// conversation — Windows' (see the module's note).
    pub const MINIMUM_WINDOW_WIDTH: f64 = RAIL + LIST_MIN + DIVIDER + MIN_PANE + FRAME;

    /// How far inside the column's trailing edge the jump-to-newest button stands.
    pub const JUMP_INSIDE: f64 = 12.0;

    /// The conversation column for a pane this wide: `min(pane − 2×16, 1024)`, never below
    /// nothing.
    pub fn width(pane: f64) -> f64 {
        (pane - (2.0 * PANE_INSET)).clamp(0.0, MAX_COLUMN)
    }

    /// What the jump-to-newest button leaves clear of the pane's trailing edge: the column's
    /// margin there and [`JUMP_INSIDE`] more, so it stands inside the column on any window — 28
    /// wherever the column fills the pane, as it always stood.
    pub fn jump_inset(pane: f64) -> f64 {
        ((pane - width(pane)).max(0.0) / 2.0) + JUMP_INSIDE
    }

    /// The widest bubble in a column this wide: `min(column − 48, 560)`, never below nothing.
    pub fn bubble_max(column: f64) -> f64 {
        (column - BUBBLE_SPACER).clamp(0.0, BUBBLE_MAX)
    }

    /// The list, the pane and the thread for a view this wide: the thread beside the conversation
    /// while that leaves the pane [`MIN_PANE`] with the list at its narrowest, and over it
    /// otherwise; the list as wide as it is drawn while the pane keeps its floor, narrowing to
    /// [`LIST_MIN`] before the pane goes under it.
    pub fn arrange(view: f64, thread_open: bool) -> PaneArrangement {
        let beside = thread_open && view - LIST_MIN - DIVIDER - THREAD >= MIN_PANE;
        let thread = if beside { THREAD } else { 0.0 };
        let list = (view - DIVIDER - thread - MIN_PANE).clamp(LIST_MIN, LIST);
        PaneArrangement {
            list_width: list,
            thread_beside: beside,
            pane_width: (view - list - DIVIDER - thread).max(0.0),
        }
    }
}
