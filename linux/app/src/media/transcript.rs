//! A recording's text under its player (docs/protocol.md, "Transcripts on request"; the Mac's
//! `TranscriptSection`): "Show text" under a voice note and a circle where `transcript_rules::offers`
//! says so, "Getting the text…" while it is asked for, then the text — selectable, "No speech" for
//! silence — with "Hide text"; a failure says why under the player, with "Show text" again only
//! where trying again could help.
//!
//! **What is drawn is the `TranscriptModel`'s**: a row is rebuilt on every redraw, so the section
//! draws `look` and redraws on `on_changed`. The ask itself ([`ask`]) is the model's — the kept text
//! at once, else the consent question (`composer::consent`) where it is needed, then the request:
//! the server's own copy, or sound this device makes from the file ([`GstSoundTracks`]) for a circle
//! and for what the server cannot read. Nothing here logs a word of the text.

use std::rc::Rc;

use fc_core::protocol::{AttachmentDto, MessageDto};
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::backdrop_consent;
use fc_logic::message_menu::{self, TextItem};
use fc_logic::transcript_sound::{self, TranscriptForm};
use fc_logic::transcripts::{TranscriptPhase, transcript_rules};
use gtk::prelude::*;

use super::sound_tracks::GstSoundTracks;
use crate::composer::consent;
use crate::services::log;
use crate::shell::{Connection, Shell};

/// The recording of `message` whose text "Show text" asks for: its circle, or its one voice note.
pub fn recording(message: &MessageDto) -> Option<&AttachmentDto> {
    match message_menu::recording(message)? {
        message_menu::RecordingKind::Round => message.round_video(),
        message_menu::RecordingKind::Voice => message.media().first(),
    }
}

/// Whether "Show text" is offered for `attachment` of `message` now: text this device holds, or a
/// server that transcribes, the rule, and a form to ask in.
fn offered(connection: &Connection, message: &MessageDto, attachment: &AttachmentDto) -> bool {
    let state = connection.session.state();
    let held = connection.transcripts.holds(attachment.id).unwrap_or(false);
    let chat_kind = connection
        .chats
        .chat(message.chat_id)
        .ok()
        .flatten()
        .map(|row| row.chat.kind);
    let reader = state.me.as_ref().map_or(0, |me| me.id);
    transcript_rules::offers(
        state.assistant.as_ref(),
        state.family.as_ref(),
        chat_kind.as_deref(),
        message.sender_id,
        reader,
        attachment,
        held,
        GstSoundTracks::available(),
    )
}

/// The message menu's "Show text": offered exactly where the words under the recording offer it.
pub fn menu_offers(message: &MessageDto) -> bool {
    let Some(connection) = Shell::try_get().and_then(|shell| shell.connection()) else {
        return false;
    };
    let Some(attachment) = recording(message) else {
        return false;
    };
    offered(&connection, message, attachment)
        && message_menu::text_item_for(&connection.transcripts.look(attachment.id))
            == TextItem::Show
}

/// "Show text" for `attachment` of `message`; the consent question, when it is asked, over
/// `anchor`'s window. Spawned on the main context: the section redraws from the model.
pub fn ask(anchor: &impl IsA<gtk::Widget>, message: &MessageDto, attachment: &AttachmentDto) {
    let shell = Shell::get();
    let Some(connection) = shell.connection() else {
        return;
    };
    let state = connection.session.state();
    let extracts = GstSoundTracks::available();
    let form = state
        .assistant
        .as_ref()
        .map_or(TranscriptForm::None, |assistant| {
            transcript_rules::form(attachment, assistant, extracts)
        });
    // Also for the stored form: a server that cannot read its own copy is sent the sound instead.
    let sound = state
        .assistant
        .as_ref()
        .filter(|_| extracts)
        .map(|assistant| {
            transcript_sound::device_source(
                Rc::clone(&connection.attachments),
                attachment.clone(),
                transcript_rules::max_bytes(assistant),
                Rc::new(GstSoundTracks::new(shell.paths.tmp_dir())),
                shell.clock.clone(),
                Rc::new(|line: &str| crate::fc_info!("transcript: {line}")),
            )
        });
    let anchor = anchor.as_ref().downgrade();
    let (chat_id, message_id, attachment_id) = (message.chat_id, message.id, attachment.id);
    glib::spawn_future_local(async move {
        let session = Rc::clone(&connection.session);
        let asks_first = || {
            let state = session.state();
            backdrop_consent::asks_first(
                state
                    .assistant
                    .as_ref()
                    .and_then(|assistant| assistant.processor.as_deref()),
                state.assistant_consent_at.as_deref(),
            )
        };
        let asking = Rc::clone(&connection);
        let question = || {
            let (anchor, connection) = (anchor.upgrade(), Rc::clone(&asking));
            async move {
                match anchor {
                    Some(anchor) => {
                        consent::ask(&anchor, &connection).await == consent::Answer::Agreed
                    }
                    None => false,
                }
            }
        };
        let shown = connection
            .transcripts
            .show(
                chat_id,
                message_id,
                attachment_id,
                form,
                sound,
                asks_first,
                question,
            )
            .await;
        if let Err(error) = shown {
            crate::fc_warn!(
                "transcript: the cache refused ({})",
                log::store_error(&error)
            );
        }
    });
}

/// The section under a voice note's player (`own`: in the reader's own balloon) or under a circle.
/// Empty, and not drawn, where nothing is offered.
pub fn section(message: &MessageDto, attachment: &AttachmentDto, own: bool) -> gtk::Widget {
    numbered_section(message, attachment, own, None)
}

/// The section under a video's tile — and, under a pile of pictures, one for each of its videos,
/// headed "Video 2" so each says which it is the text of (`number`, only where there are several;
/// the Mac's `TranscriptDoor.pileVideos`).
pub fn numbered_section(
    message: &MessageDto,
    attachment: &AttachmentDto,
    own: bool,
    number: Option<usize>,
) -> gtk::Widget {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 2);
    root.add_css_class("fc-media-text");
    if own {
        root.add_css_class("fc-media-text-own");
    }
    let Some(connection) = Shell::try_get().and_then(|shell| shell.connection()) else {
        root.set_visible(false);
        return root.upcast();
    };
    let draw: Rc<dyn Fn()> = {
        let weak = root.downgrade();
        let (connection, message, attachment) =
            (Rc::clone(&connection), message.clone(), attachment.clone());
        Rc::new(move || {
            if let Some(root) = weak.upgrade() {
                fill(&root, &connection, &message, &attachment, number);
            }
        })
    };
    draw();
    let id = attachment.id;
    let changed = {
        let draw = Rc::clone(&draw);
        connection
            .transcripts
            .on_changed()
            .subscribe(move |changed| {
                if *changed == id {
                    draw();
                }
            })
    };
    let changed = std::cell::Cell::new(Some(changed));
    root.connect_destroy(move |_| {
        if let Some(changed) = changed.take() {
            connection.transcripts.on_changed().unsubscribe(changed);
        }
    });
    root.upcast()
}

/// Draws the section for the model's look now.
fn fill(
    root: &gtk::Box,
    connection: &Connection,
    message: &MessageDto,
    attachment: &AttachmentDto,
    number: Option<usize>,
) {
    while let Some(child) = root.first_child() {
        root.remove(&child);
    }
    let say = Shell::get().say.clone();
    let look = connection.transcripts.look(attachment.id);
    let show = || {
        let button = action(say.get("Show text"));
        let (message, attachment) = (message.clone(), attachment.clone());
        button.connect_clicked(move |button| ask(button, &message, &attachment));
        button
    };
    match look.phase {
        TranscriptPhase::Closed => {
            if offered(connection, message, attachment) {
                root.append(&show());
            }
        }
        TranscriptPhase::Asking => {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            line.add_css_class("fc-media-text-asking");
            let spinner = adw::Spinner::new();
            line.append(&spinner);
            line.append(&note(say.get("Getting the text…")));
            root.append(&line);
        }
        TranscriptPhase::Open => {
            let text = if look.no_speech() {
                let silence = note(say.get("No speech"));
                silence.add_css_class("fc-media-text-silence");
                silence
            } else {
                gtk::Label::builder()
                    .label(look.text.as_deref().unwrap_or_default())
                    .wrap(true)
                    .wrap_mode(gtk::pango::WrapMode::WordChar)
                    .xalign(0.0)
                    .max_width_chars(34)
                    .selectable(true)
                    .css_classes(["fc-media-text-words"])
                    .build()
            };
            root.append(&text);
            let hide = action(say.get("Hide text"));
            let (model, id) = (Rc::clone(&connection.transcripts), attachment.id);
            hide.connect_clicked(move |_| model.hide(id));
            root.append(&hide);
        }
        TranscriptPhase::Failed => {
            let error = look
                .error
                .unwrap_or_else(|| fc_core::protocol::ApiError::transport("no answer"));
            root.append(&note(&transcript_rules::failure_sentence(
                &error,
                say.as_ref(),
            )));
            if transcript_rules::may_retry(&error) && offered(connection, message, attachment) {
                root.append(&show());
            }
        }
    }
    // Which video of the pile, over whatever the section says.
    if let Some(number) = number
        && root.first_child().is_some()
    {
        let heading = note(&format!("{} {number}", say.get("Video")));
        root.prepend(&heading);
    }
    root.set_visible(root.first_child().is_some());
}

/// "Show text" / "Hide text": a small flat action in the link's colour (white in the own balloon).
fn action(label: &str) -> gtk::Button {
    gtk::Button::builder()
        .label(label)
        .halign(gtk::Align::Start)
        .css_classes(["flat", "caption", "fc-media-text-action"])
        .build()
}

/// A dimmed caption that wraps: "Getting the text…", "No speech", a failure.
fn note(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .wrap(true)
        .xalign(0.0)
        .max_width_chars(34)
        .css_classes(["caption", "fc-media-text-note"])
        .build()
}
