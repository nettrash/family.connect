//! A message's context menu, item by item (the approved design of 2026-10-05: "Long-press menu on a
//! voice or video message") — a port of `MessageMenu.cs`. A RECORDING has nothing to copy, edit or
//! select — its menu is the reactions first, then Reply, the thread, Show text, Playback speed for
//! a voice message, Save… and Open Full Screen for a video message (the order every client draws) —
//! and every other message's menu is exactly what it always was. The Safety group (Report…, Block)
//! follows all of these, and is the view's.

use fc_core::protocol::{ApiError, MessageDto};

use crate::transcripts::{TranscriptLook, TranscriptPhase, transcript_rules};

/// What a message IS when it is a recording and nothing else: a voice message, or a video message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordingKind {
    Voice,
    Round,
}

/// One item of a message's context menu, before the Safety group (Report…, Block), which follows
/// them all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MessageAction {
    Reply,
    React,
    ShowText,
    PlaybackSpeed,
    Save,
    OpenFullScreen,
    Copy,
    ViewThread,
    Edit,
}

/// What the menu's text item is for a recording's text as it stands ([`text_item_for`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextItem {
    /// No item: the text is being asked for, or could not be had and asking again would not help.
    None,
    /// "Show text": asks for it — as the words under the recording do.
    Show,
    /// "Hide text": folds it away.
    Hide,
}

/// Whether this message is a recording and nothing else: a video message
/// ([`MessageDto::round_video`], the shared test), or exactly one sound with NO BODY. A recording
/// sent with words is a message with words — Copy and Edit stay.
pub fn recording(message: &MessageDto) -> Option<RecordingKind> {
    if message.round_video().is_some() {
        return Some(RecordingKind::Round);
    }
    let voice = message.body.is_empty()
        && message.poll.is_none()
        && message.call.is_none()
        && matches!(message.media(), [only] if only.kind == "audio");
    voice.then_some(RecordingKind::Voice)
}

/// The text item for a recording whose text stands at `look` — offered EXACTLY where the words
/// under the recording offer the same thing: "Hide text" while it is open, "Show text" while it is
/// folded away or after a failure that trying again could help (the row's "Try Again"), and nothing
/// while it is being asked for ("Getting the text…") or after a failure the row deliberately offers
/// no way past.
pub fn text_item_for(look: &TranscriptLook) -> TextItem {
    match look.phase {
        TranscriptPhase::Open => TextItem::Hide,
        TranscriptPhase::Asking => TextItem::None,
        TranscriptPhase::Failed => {
            let retry = match &look.error {
                Some(error) => transcript_rules::may_retry(error),
                // No error recorded is the row's "no answer": a transport failure.
                None => transcript_rules::may_retry(&ApiError::transport("no answer")),
            };
            if retry {
                TextItem::Show
            } else {
                TextItem::None
            }
        }
        TranscriptPhase::Closed => TextItem::Show,
    }
}

/// The items, in order. `may_edit`: whether the reader may edit it
/// (`ConversationModel::may_edit`). `in_thread`: the menu is the thread panel's, where Edit and View
/// thread are not offered. `offers_text`: "Show text" can be asked for here — the recording's place
/// for its text is drawn.
pub fn actions(
    message: &MessageDto,
    may_edit: bool,
    in_thread: bool,
    offers_text: bool,
) -> Vec<MessageAction> {
    let chained = !in_thread && (message.thread_root_id.is_some() || message.reply_count.is_some());
    if let Some(kind) = recording(message) {
        let mut items = vec![MessageAction::React, MessageAction::Reply];
        // The thread straight after Reply, where iOS, the Mac, Android and the web put it on a
        // recording's menu.
        if chained {
            items.push(MessageAction::ViewThread);
        }
        if offers_text {
            items.push(MessageAction::ShowText);
        }
        if kind == RecordingKind::Voice {
            items.push(MessageAction::PlaybackSpeed);
        }
        items.push(MessageAction::Save);
        if kind == RecordingKind::Round {
            items.push(MessageAction::OpenFullScreen);
        }
        return items;
    }
    let mut ordinary = vec![MessageAction::Reply, MessageAction::React];
    if !message.body.is_empty() && message.call.is_none() {
        ordinary.push(MessageAction::Copy);
    }
    if chained {
        ordinary.push(MessageAction::ViewThread);
    }
    if !in_thread && may_edit {
        ordinary.push(MessageAction::Edit);
    }
    ordinary
}
