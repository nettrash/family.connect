//! The box a message is written in — with the rules of the Apple app's
//! composer (fc_text::mentions, ::assistant, ::composer): the "@" that
//! offers the roster, who a text names when Send is pressed, the `@ai` and
//! `/draw` buttons, the 4,000-character limit, and the reply and edit
//! banners.
//!
//! In a conversation its trailing button is the Send SLOT of the plan for
//! #79 (docs/audio-video-messages-2026-10-04.md, S1.3): one fixed icon
//! button that is Send when there is something to send and a microphone
//! when there is not, whose every state is fc_text::record::composer_slot's.
//! A click, Enter or Space on it records hands-free; a touch acts on its
//! `pointerup` (S8.8); a right-click, Shift+F10 or the Menu key opens its
//! menu (S1.6, S8.7). A thread's box is not changed: today's Send.

use std::collections::HashSet;
use std::rc::Rc;

use fc_text::assistant_pictures::{self, Candidate, MentionNotice, Switches};
use fc_text::i18n::{t, t1};
use fc_text::record::{self, Slot};
use fc_text::{assistant, assistant_consent, composer, media, mentions};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use web_sys::{File, HtmlElement, HtmlTextAreaElement};
use yew::prelude::*;

use crate::model::{Assistant, Member, Mention};
use crate::recorder::now_ms;
use crate::store::Draft;
use crate::views::consent::AssistantConsentBar;
use crate::views::quiet::LiveRegion;

/// How long after a touch's `pointerup` — on which the slot has already
/// acted (S8.8) — the `click` a browser may send after it is swallowed.
const TOUCH_CLICK_MS: f64 = 800.0;

/// A press on the slot, from the moment it went down.
struct Press {
    /// What made it: "touch", "mouse", "pen" — or "key", for Enter and Space.
    kind: String,
    /// When it came up, by `recorder::now_ms`.
    up_ms: Option<f64>,
    /// It went down while the slot ignored activation, so it is ignored
    /// whole (S1.1): come up after the guard, it is still neither a click
    /// nor a tap.
    ignored: bool,
    /// What it became is decided — or it came up off the slot, where no
    /// click on the slot follows.
    spent: bool,
}

impl Press {
    fn down(kind: String, ignored: bool) -> Press {
        Press {
            kind,
            up_ms: None,
            ignored,
            spent: false,
        }
    }

    /// The activation it ends, decided once: whether it is ignored.
    fn spend(&mut self) -> bool {
        !std::mem::replace(&mut self.spent, true) && self.ignored
    }
}

/// What the Send slot needs to be the plan's (S1.3) — given only by the
/// conversation, for its own box, and never by a thread, whose composer the
/// plan leaves as it is. An edit in the conversation's box keeps the slot
/// Save (row 4), never a microphone.
#[derive(Clone, PartialEq)]
pub struct Records {
    /// The voice recording the slot is showing.
    pub recording: record::Recording,
    /// The recording row (S2.4), drawn in place of the field — and of the
    /// paperclip, the sticker and `@ai` buttons — while one runs.
    pub row: Html,
    /// A call in any phase but ended (S1.2 **call**).
    pub call: bool,
    /// The chat holds a voice message that was not sent (S2.8).
    pub not_sent: bool,
    /// This page can record at all — a secure context (S1.2).
    pub can_record: bool,
    /// The microphone is being asked for: a Send waits for the answer.
    pub starting: bool,
    /// The slot activated as a microphone, or as the recording's Send arrow
    /// or Stop square (rows 2, 3, 7–10) — in the click or the touch's
    /// `pointerup` itself, which is what lets the page's audio start.
    pub on_slot: Callback<()>,
    /// The microphone's menu chose "Record Voice Message".
    pub on_record: Callback<()>,
    /// Esc while a recording runs: Stop, never Delete (S2.4, S8.7).
    pub on_stop: Callback<()>,
    /// The slot's own Send or Save just emptied the box.
    pub on_emptied: Callback<()>,
    /// The person changed what the box holds — typed, deleted, took a
    /// suggestion, pasted, added `@ai` or `/draw` — which lifts the guard
    /// (S1.1). The conversation says the same of what it stages.
    pub on_changed: Callback<()>,
    /// Until when the slot ignores activation (S1.1), by `recorder::now_ms`.
    pub guard_until: Callback<(), u64>,
    /// Whether the words in the box are blank, each time that changes — for
    /// a recording started beside them (row 3).
    pub on_blank: Callback<bool>,
    /// Moves each time the five-minute limit stops a recording into review:
    /// the cursor goes back into the box only if the focus is still the
    /// composer's, or nobody's (S2.4, S2.7). An ending by the person moves
    /// the box's own `focus`; an interruption moves neither (S4).
    pub refocus: u32,
    /// The video recorder is open, and owns the row (S1.3 row 1).
    pub recorder_open: bool,
    /// Video messages — where this server has them and this device has a
    /// camera, in a family or a direct chat (S1.2, S1.4–S1.6). None: no
    /// video entry at all.
    pub video: Option<VideoEntry>,
}

/// The ways into the video recorder the composer draws: the video button in
/// the field, and "Record Video Message" in the microphone's menu.
#[derive(Clone, PartialEq)]
pub struct VideoEntry {
    /// This browser can record one (the probe, S8.7). Where it cannot, the
    /// button is not drawn and the menu item says why instead of opening.
    pub records: bool,
    /// Open the recorder.
    pub on_open: Callback<()>,
    /// Say why it does not open, on the conversation's notice line.
    pub on_explain: Callback<String>,
}

/// "This browser can't record video messages. Voice messages work." — what
/// a way into the recorder says in a browser whose probe fails (S1.5).
pub fn cannot_record_video() -> &'static str {
    t("This browser can't record video messages. Voice messages work.")
}

/// The video button's picture: a video camera in a circle (S1.4).
const DOOR_PATH: &str = "M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20zm0 1.8a8.2 8.2 0 1 1 0 16.4 8.2 8.2 0 0 1 0-16.4zM7.5 9h6a1 1 0 0 1 1 1v.9l2.5-1.6v5.4l-2.5-1.6v.9a1 1 0 0 1-1 1h-6a1 1 0 0 1-1-1v-4a1 1 0 0 1 1-1z";

/// The slot's pictures (S1.3's glyphs, drawn inline).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Glyph {
    Microphone,
    Send,
    Stop,
    Save,
}

impl Glyph {
    fn name(self) -> &'static str {
        match self {
            Glyph::Microphone => "microphone",
            Glyph::Send => "send",
            Glyph::Stop => "stop",
            Glyph::Save => "save",
        }
    }

    fn path(self) -> &'static str {
        match self {
            Glyph::Microphone => {
                "M12 14a3 3 0 0 0 3-3V5a3 3 0 0 0-6 0v6a3 3 0 0 0 3 3zm5-3a5 5 0 0 1-10 0H5a7 7 0 \
                 0 0 6 6.92V21h2v-3.08A7 7 0 0 0 19 11z"
            }
            Glyph::Send => "M12 4 5 11l1.41 1.41L11 7.83V20h2V7.83l4.59 4.58L19 11z",
            Glyph::Stop => "M7 7h10v10H7z",
            Glyph::Save => "M9 16.17 4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41z",
        }
    }

    /// Keyed by what it shows, so a change draws it anew — and the
    /// stylesheet's fade, the slot's cross-fade (S1.3), runs again.
    fn draw(self) -> Html {
        html! {
            <svg key={self.name()} class="slot-icon" viewBox="0 0 24 24"
                 aria-hidden="true" focusable="false">
                <path fill="currentColor" d={self.path()} />
            </svg>
        }
    }
}

/// What the slot says it is, in the reader's language: its accessibility
/// label and the word it keeps as visually hidden text (S6, S8.7) — the
/// English of each is fc_text::record's `Slot::label`.
fn slot_word(slot: Slot) -> &'static str {
    match slot {
        Slot::Recorder => "",
        Slot::HeldMicrophone | Slot::SendVoice => t("Send voice message"),
        Slot::StopRecording => t("Stop recording"),
        Slot::Save { .. } => t("Save"),
        Slot::Send | Slot::SendDisabled => t("Send"),
        Slot::Dimmed(_) | Slot::Microphone => t("Record voice message"),
    }
}

/// Whether the slot's activation guard covers a press now (S1.1): the
/// slot's own last activation was under 600 ms ago. A change the person made
/// since — words typed or deleted, a suggestion, a paste, something staged —
/// is never guarded, and has lifted it (`Records::on_changed`), so "ok" sent
/// at once still goes.
fn guarded(records: Option<&Records>) -> bool {
    records.is_some_and(|records| now_ms() < records.guard_until.emit(()) as f64)
}

/// The person changed what the box holds: said to the slot's rules, for
/// whom such a change is never guarded.
fn changed(records: &Option<Records>) {
    if let Some(records) = records {
        records.on_changed.emit(());
    }
}

/// The most members one message may name — the server refuses more with
/// `validation`, and neither app caps, so theirs would fail. Here the
/// first twenty named, in the order they appear, are the list.
pub const MAX_MENTIONS: usize = 20;

/// What a reply banner shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Replying {
    pub message_id: i64,
    pub name: String,
    /// Empty when the quoted row is hidden behind a block.
    pub excerpt: String,
}

/// What the picture disclosures read: what is staged, what the draft
/// replies to, and the locks (docs/protocol.md, "Pictures").
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Pictures {
    pub staged: Vec<Candidate>,
    /// The attachments of the message being replied to.
    pub quoted: Vec<Candidate>,
    pub server_can_see: bool,
    pub server_can_draw: bool,
    pub family_allows: bool,
    pub family_history: bool,
    pub family_history_photos: bool,
}

impl Pictures {
    /// What the strip above the box says, if anything: the assistant's own
    /// chat says it of every staged photo; the family chat of an `@ai`
    /// draft that carries one or replies to one.
    pub fn notice(&self, draft: &str, is_ai_chat: bool, is_family_chat: bool) -> Option<String> {
        if is_ai_chat {
            let can_see = assistant_pictures::offers_picture_attach(
                true,
                self.server_can_see,
                self.family_allows,
            );
            return assistant_pictures::private_notice(&self.staged, can_see);
        }
        if !is_family_chat {
            return None;
        }
        MentionNotice::of(
            draft,
            &self.staged,
            &self.quoted,
            Switches {
                server_can_see: self.server_can_see,
                family_allows: self.family_allows,
                family_history: self.family_history,
                family_history_photos: self.family_history_photos,
                server_can_draw: self.server_can_draw,
            },
        )
        .map(|notice| notice.sentence())
    }
}

/// The message being edited.
#[derive(Debug, Clone, PartialEq)]
pub struct Editing {
    pub message_id: i64,
    pub body: String,
}

#[derive(Properties, PartialEq)]
pub struct ComposerProps {
    pub chat_id: i64,
    pub is_family_chat: bool,
    pub is_ai_chat: bool,
    pub my_user_id: i64,
    pub members: Vec<Member>,
    pub blocked: HashSet<i64>,
    pub assistant: Option<Assistant>,
    /// Whether this member has agreed that their words may go to the model
    /// (docs/protocol.md, "Consenting to the assistant"). While they have
    /// not, a draft that would travel is not sent: the strip above the box
    /// says where it would go, and Send raises the screen that asks.
    #[prop_or_default]
    pub agreed_to_assistant: bool,
    /// Open that screen. Emitted instead of sending, with the draft left
    /// exactly as typed — pressing Send again once the answer is recorded
    /// sends it.
    #[prop_or_default]
    pub on_review_consent: Callback<()>,
    pub replying: Option<Replying>,
    pub editing: Option<Editing>,
    /// What was being typed here when the reader last left.
    pub initial: String,
    pub on_send: Callback<Draft>,
    pub on_save_edit: Callback<(i64, String)>,
    /// Esc, or a banner's ✕: the reply or the edit is given up.
    pub on_cancel: Callback<()>,
    pub on_typing: Callback<()>,
    /// The words in the box when the composer goes, kept for the reader's
    /// return.
    pub on_draft: Callback<String>,
    /// On the thread surface: no edits.
    #[prop_or_default]
    pub in_thread: bool,
    /// What goes before the box: the conversation's attach menu.
    #[prop_or_default]
    pub attach: Html,
    /// How many attachments are staged — with any, Send goes with no words
    /// at all (a photo needs no caption).
    #[prop_or_default]
    pub staged: usize,
    /// Something is being prepared or found: Send waits for it.
    #[prop_or_default]
    pub busy: bool,
    /// Words to add to the end of the draft, each time the number moves —
    /// a paste that did not land in the box, a dropped link.
    #[prop_or_default]
    pub append: (u32, String),
    /// Each time this moves, the draft is taken — handed to `on_take` and
    /// the box cleared — for a send that goes at once with something the
    /// conversation holds: a location, with the draft as its caption.
    #[prop_or_default]
    pub take: u32,
    #[prop_or_default]
    pub on_take: Callback<Draft>,
    /// Files pasted into the box — taken only where attachments are
    /// (`takes_files`): the thread's box takes none, and a paste there must
    /// stay the browser's rather than vanish.
    #[prop_or_default]
    pub on_files: Callback<Vec<File>>,
    #[prop_or_default]
    pub takes_files: bool,
    /// What the picture disclosures read.
    #[prop_or_default]
    pub pictures: Pictures,
    /// Moves the cursor into the box each time it changes — a row's
    /// "Reply" on a surface whose every send is already a reply.
    #[prop_or_default]
    pub focus: u32,
    /// The Send slot of the plan for #79 — see [`Records`]. None: today's
    /// Send, as a thread keeps it.
    #[prop_or_default]
    pub records: Option<Records>,
}

/// Who the text names, resolved against the whole live roster at send —
/// a name typed by hand mentions too, a name deleted after picking does
/// not — and only in the family chat, where a mention means anything.
pub fn resolve_mentions(body: &str, members: &[Member], is_family_chat: bool) -> Vec<Mention> {
    if !is_family_chat {
        return Vec::new();
    }
    let roster: Vec<mentions::Member> = members
        .iter()
        .filter(|member| !member.deleted)
        .map(|member| mentions::Member {
            user_id: member.id,
            name: &member.display_name,
        })
        .collect();
    mentions::resolve(body, &roster)
        .into_iter()
        .take(MAX_MENTIONS)
        .map(|member| Mention {
            user_id: member.user_id,
            name: member.name.to_string(),
        })
        .collect()
}

#[function_component(Composer)]
pub fn composer(props: &ComposerProps) -> Html {
    let text = use_state(|| props.initial.clone());
    let notice = use_state(|| Option::<String>::None);
    let active = use_state(|| 0usize);
    let area = use_node_ref();
    // The draft in progress when an edit began — set aside, and back when
    // the edit is done, the way the apps do it.
    let aside = use_mut_ref(|| Option::<String>::None);
    // What is in the box, for the unmount below to hand back.
    let latest = use_mut_ref(String::new);
    *latest.borrow_mut() = (*text).clone();
    let slot_ref = use_node_ref();
    let wrap_ref = use_node_ref();

    {
        let text = text.clone();
        let aside = aside.clone();
        let editing = props.editing.clone();
        use_effect_with(
            editing.as_ref().map(|edit| edit.message_id),
            move |_| match editing {
                Some(edit) => {
                    if aside.borrow().is_none() {
                        *aside.borrow_mut() = Some((*text).clone());
                    }
                    text.set(edit.body);
                }
                None => {
                    if let Some(kept) = aside.borrow_mut().take() {
                        text.set(kept);
                    }
                }
            },
        );
    }
    {
        let on_draft = props.on_draft.clone();
        let latest = latest.clone();
        let aside = aside.clone();
        use_effect_with((), move |_| {
            move || {
                // Leaving mid-edit keeps the draft that was set aside, not
                // the edited message's own words.
                let kept = aside
                    .borrow()
                    .clone()
                    .unwrap_or_else(|| latest.borrow().clone());
                on_draft.emit(kept);
            }
        });
    }
    // The reply banner puts the cursor where the answer goes — when a reply
    // is primed, never when one is taken away: a recording that carries the
    // reply off, interrupted by a call, must leave the focus where the call
    // put it (the plan for #79, S4).
    {
        let area = area.clone();
        use_effect_with(
            props.replying.as_ref().map(|reply| reply.message_id),
            move |replying| {
                if replying.is_some() {
                    focus_the_box(&area);
                }
            },
        );
    }
    // And so does a surface asking for it, each time it asks.
    {
        let area = area.clone();
        use_effect_with(props.focus, move |focus| {
            if *focus > 0 {
                focus_the_box(&area);
            }
        });
    }
    // The five-minute limit stopping a recording into review gives the
    // cursor back to the box only if the focus is still the composer's — on
    // its slot, in its row — or nobody's: anywhere else, the person or the
    // app put it there, and it stays (S2.4, S2.7).
    {
        let area = area.clone();
        let wrap_ref = wrap_ref.clone();
        let refocus = props.records.as_ref().map_or(0, |records| records.refocus);
        use_effect_with(refocus, move |refocus| {
            if *refocus > 0 && focus_is_within(&wrap_ref) {
                focus_the_box(&area);
            }
        });
    }

    let editing = props.editing.clone();
    let suggestions: Vec<Member> = if props.is_family_chat && editing.is_none() {
        match mentions::query(&text) {
            Some(query) => {
                let roster: Vec<mentions::Member> = props
                    .members
                    .iter()
                    .filter(|member| !member.deleted)
                    .map(|member| mentions::Member {
                        user_id: member.id,
                        name: &member.display_name,
                    })
                    .collect();
                let mut excluding: Vec<i64> = props.blocked.iter().copied().collect();
                excluding.push(props.my_user_id);
                let offered: HashSet<i64> = mentions::candidates(&roster, query, &excluding)
                    .into_iter()
                    .map(|member| member.user_id)
                    .collect();
                props
                    .members
                    .iter()
                    .filter(|member| offered.contains(&member.id))
                    .cloned()
                    .collect()
            }
            None => Vec::new(),
        }
    } else {
        Vec::new()
    };

    let accept = {
        let text = text.clone();
        let area = area.clone();
        let records = props.records.clone();
        Callback::from(move |name: String| {
            changed(&records);
            text.set(mentions::accept(&text, &name));
            if let Some(area) = area.cast::<HtmlTextAreaElement>() {
                let _ = area.focus();
            }
        })
    };

    // Words from outside the box, onto the end of the draft, against the
    // ceiling (fc_text::composer::appending — the Mac's appendToDraft).
    {
        let text = text.clone();
        let notice = notice.clone();
        let area = area.clone();
        let latest = latest.clone();
        let records = props.records.clone();
        use_effect_with(props.append.clone(), move |(count, addition)| {
            if *count == 0 || addition.is_empty() {
                return;
            }
            let draft = latest.borrow().clone();
            let outcome = composer::appending(addition, &draft);
            notice.set(outcome.notice().map(|notice| notice.said()));
            match outcome {
                composer::Paste::Appended(updated) | composer::Paste::Truncated(updated) => {
                    changed(&records);
                    text.set(updated)
                }
                composer::Paste::Full => {}
            }
            if let Some(area) = area.cast::<HtmlTextAreaElement>() {
                let _ = area.focus();
            }
        });
    }
    // One line tall, growing with what is typed to the five the stylesheet
    // allows — the Mac's `TextField(axis: .vertical).lineLimit(1...5)`.
    // Fitted after every render that changed the text rather than in the
    // input handler: the box is also filled from outside it (a mention
    // accepted, words appended, an edit begun, a draft restored with a
    // chat) and emptied by a send, and each of those has to fit too.
    {
        let area = area.clone();
        use_effect_with((*text).clone(), move |_| {
            if let Some(area) = area.cast::<HtmlTextAreaElement>() {
                fit(&area);
            }
        });
    }
    // The draft, taken for a send that goes at once.
    {
        let text = text.clone();
        let latest = latest.clone();
        let on_take = props.on_take.clone();
        let members = props.members.clone();
        let is_family = props.is_family_chat;
        use_effect_with(props.take, move |take| {
            if *take == 0 {
                return;
            }
            let body = composer::trimmed_for_send(&latest.borrow())
                .unwrap_or("")
                .to_string();
            let mentioned = resolve_mentions(&body, &members, is_family);
            text.set(String::new());
            on_take.emit(Draft {
                body,
                mentions: mentioned,
                ..Draft::default()
            });
        });
    }

    let staged = props.staged;
    let busy = props.busy;
    // Which of the assistant's surfaces this composer is on, in the words
    // the shared rule uses (fc_text::assistant_consent, and `model_surface`
    // on the server).
    let chat_kind = if props.is_ai_chat {
        "ai"
    } else if props.is_family_chat {
        "family"
    } else {
        "direct"
    };
    let processor = props
        .assistant
        .as_ref()
        .and_then(|assistant| assistant.processor.clone());
    let send = {
        let text = text.clone();
        let notice = notice.clone();
        let on_send = props.on_send.clone();
        let on_save_edit = props.on_save_edit.clone();
        let on_cancel = props.on_cancel.clone();
        let members = props.members.clone();
        let is_family = props.is_family_chat;
        let editing = editing.clone();
        let on_review_consent = props.on_review_consent.clone();
        let agreed = props.agreed_to_assistant;
        let has_assistant = props.assistant.is_some();
        let processor = processor.clone();
        let records = props.records.clone();
        let starting = props
            .records
            .as_ref()
            .is_some_and(|records| records.starting);
        Callback::from(move |_: ()| {
            if busy || starting {
                return;
            }
            // A second press of what was a moment ago the slot's Stop square
            // or Send arrow must not send what it staged (S1.1).
            if guarded(records.as_ref()) {
                return;
            }
            let body = match composer::trimmed_for_send(&text) {
                Some(body) => body.to_string(),
                // Nothing typed is still a send when something is staged.
                None if staged > 0 && editing.is_none() => String::new(),
                None => return,
            };
            notice.set(None);
            // What the box held is going: the slot that takes its place —
            // a microphone, as likely as not — waits out the guard.
            let emptied = || {
                if let Some(records) = &records {
                    records.on_emptied.emit(());
                }
            };
            if let Some(edit) = &editing {
                // Saving what was already there is done at once: nothing to
                // send, and nothing to stay in edit mode for.
                if body == edit.body {
                    on_cancel.emit(());
                } else {
                    on_save_edit.emit((edit.message_id, body));
                }
                emptied();
                return;
            }
            // Nothing reaches the model unasked. The server refuses this
            // with `assistant_consent_required` anyway; asking here is what
            // turns that refusal into a question with the message still in
            // the box (docs/protocol.md, "Consenting to the assistant").
            if editing.is_none()
                && assistant_consent::is_required(chat_kind, &body, processor.as_deref(), agreed)
            {
                on_review_consent.emit(());
                return;
            }
            // And a server that will not say WHO answers gets nothing at
            // all: there is no honest way to ask, so there is nothing to
            // send. The strip above the box says so.
            if editing.is_none()
                && assistant_consent::is_withheld_from_an_unnamed_assistant(
                    chat_kind,
                    &body,
                    has_assistant,
                    processor.as_deref(),
                )
            {
                return;
            }
            let mentioned = resolve_mentions(&body, &members, is_family);
            text.set(String::new());
            on_send.emit(Draft {
                body,
                mentions: mentioned,
                ..Draft::default()
            });
            emptied();
        })
    };

    let on_input = {
        let text = text.clone();
        let notice = notice.clone();
        let on_typing = props.on_typing.clone();
        let active = active.clone();
        let records = props.records.clone();
        Callback::from(move |event: InputEvent| {
            changed(&records);
            let area: HtmlTextAreaElement = event.target_unchecked_into();
            let value = area.value();
            // Over the limit — a paste, usually — is cut between Characters
            // and said so, rather than refused later by the server.
            let value = match composer::clamping(&value) {
                Some(clamped) => {
                    notice.set(Some(composer::Notice::Clamped.said()));
                    let clamped = clamped.to_string();
                    area.set_value(&clamped);
                    clamped
                }
                None => value,
            };
            // Only when there is something to be typing. Clearing the box is
            // not typing, and a frame for it would tell the family somebody
            // is writing when they have just given up.
            if !value.trim().is_empty() {
                on_typing.emit(());
            }
            active.set(0);
            text.set(value);
        })
    };

    // Enter sends, Shift+Enter makes a line, Esc gives up a reply or an
    // edit — and while the roster is offered, the arrows walk it and Enter
    // or Tab takes the highlighted name.
    let on_key = {
        let send = send.clone();
        let accept = accept.clone();
        let active = active.clone();
        let on_cancel = props.on_cancel.clone();
        let offered: Vec<String> = suggestions
            .iter()
            .map(|member| member.display_name.clone())
            .collect();
        Callback::from(move |event: KeyboardEvent| {
            let key = event.key();
            if !offered.is_empty() {
                match key.as_str() {
                    "ArrowDown" => {
                        event.prevent_default();
                        active.set((*active + 1) % offered.len());
                        return;
                    }
                    "ArrowUp" => {
                        event.prevent_default();
                        active.set((*active + offered.len() - 1) % offered.len());
                        return;
                    }
                    "Enter" | "Tab" if !event.shift_key() => {
                        event.prevent_default();
                        accept.emit(offered[(*active).min(offered.len() - 1)].clone());
                        return;
                    }
                    _ => {}
                }
            }
            if key == "Enter" && !event.shift_key() && !event.is_composing() {
                event.prevent_default();
                send.emit(());
            } else if key == "Escape" {
                on_cancel.emit(());
            }
        })
    };

    let ask_assistant = {
        let text = text.clone();
        let records = props.records.clone();
        Callback::from(move |_: MouseEvent| {
            changed(&records);
            text.set(assistant::with_assistant_mention(&text))
        })
    };
    let ask_picture = {
        let text = text.clone();
        let records = props.records.clone();
        Callback::from(move |_: MouseEvent| {
            changed(&records);
            text.set(assistant::with_draw_token(&text))
        })
    };
    let cancel = {
        let on_cancel = props.on_cancel.clone();
        Callback::from(move |_: MouseEvent| on_cancel.emit(()))
    };

    let has_assistant = props.assistant.is_some();
    let can_draw = props.is_ai_chat
        && props
            .assistant
            .as_ref()
            .is_some_and(|assistant| assistant.images);
    // Where a `/draw` is drawn at all — this chat's 🎨 button aside, the
    // family chat's `@ai /draw` is drawn by the same deployment.
    let server_draws = assistant_pictures::server_draws(
        props
            .assistant
            .as_ref()
            .is_some_and(|assistant| assistant.images),
        processor.as_deref(),
    );
    let offers_ai = props.is_family_chat && has_assistant && editing.is_none();
    let empty =
        composer::trimmed_for_send(&text).is_none() && (props.staged == 0 || editing.is_some());

    // --- The Send slot (the plan for #79, S1.3) ------------------------------
    let records = props.records.clone();
    let blank = composer::trimmed_for_send(&text).is_none();
    let slot = records.as_ref().map(|records| {
        record::composer_slot(&record::SlotInputs {
            recorder_open: records.recorder_open,
            recording: records.recording,
            editing: editing.is_some(),
            draft_blank: blank,
            staged: props.staged > 0,
            assistant_chat: props.is_ai_chat,
            can_record: records.can_record,
            call: records.call,
            busy: props.busy,
            not_sent: records.not_sent,
        })
    });
    // A recording runs: the row takes the field's place (S2.4).
    let recording_now = matches!(
        slot,
        Some(Slot::SendVoice | Slot::StopRecording | Slot::HeldMicrophone)
    );
    let is_microphone = slot.is_some_and(Slot::is_microphone);
    // THE VIDEO BUTTON (S1.4), inside the field while it is empty: the
    // shared rule decides whether it is drawn, dimmed or shown.
    let video = records.as_ref().and_then(|records| records.video.clone());
    let door = match (records.as_ref(), video.as_ref()) {
        (Some(records), Some(video)) => record::video_door(&record::DoorInputs {
            slot: record::SlotInputs {
                recorder_open: records.recorder_open,
                recording: records.recording,
                editing: editing.is_some(),
                draft_blank: blank,
                staged: props.staged > 0,
                assistant_chat: props.is_ai_chat,
                can_record: records.can_record,
                call: records.call,
                busy: props.busy,
                not_sent: records.not_sent,
            },
            family_or_direct_chat: !props.is_ai_chat,
            undo_window: false,
            server_offers_round: true,
            has_camera: true,
            encoder_probe_passes: video.records,
            records_round_video: true,
        }),
        _ => record::Door::Hidden,
    };
    // It ignores activation for 600 ms after it appears (S1.1): it comes up
    // beside the slot the moment a Send empties the field, and a second tap
    // that drifts must not turn the camera on.
    let door_since = use_mut_ref(|| Option::<f64>::None);
    let door_reason_id = use_memo((), |_| crate::views::dialog::fresh_id("door-reason"));
    {
        let mut since = door_since.borrow_mut();
        match door {
            record::Door::Hidden => *since = None,
            _ if since.is_none() => *since = Some(now_ms()),
            _ => {}
        }
    }
    let door_html = match (door.label(), video.clone()) {
        // The shared rule's words (`record::VIDEO_DOOR_LABEL`, `…_TOOLTIP`),
        // written out so that the catalogue's scan finds them.
        (Some(_), Some(video)) => {
            let reason = door.notice().map(t);
            let onclick = {
                let door_since = door_since.clone();
                let reason = reason.map(str::to_string);
                Callback::from(move |_: MouseEvent| {
                    let appeared = door_since.borrow().unwrap_or(0.0);
                    if now_ms() - appeared < record::ACTIVATION_GUARD_MS as f64 {
                        return;
                    }
                    match &reason {
                        Some(reason) => video.on_explain.emit(reason.clone()),
                        None => video.on_open.emit(()),
                    }
                })
            };
            let dimmed = door.notice().is_some();
            html! {
                <button
                    type="button"
                    class={classes!("video-door", dimmed.then_some("is-dimmed"))}
                    aria-label={t("Record video message")}
                    title={t("Record a video message")}
                    aria-disabled={dimmed.then_some("true")}
                    aria-describedby={dimmed.then(|| (*door_reason_id).clone())}
                    {onclick}
                >
                    <svg class="door-icon" viewBox="0 0 24 24" aria-hidden="true" focusable="false">
                        <path fill="currentColor" d={DOOR_PATH} />
                    </svg>
                    if let Some(reason) = door.notice() {
                        <span id={(*door_reason_id).clone()} hidden=true>{ t(reason) }</span>
                    }
                </button>
            }
        }
        _ => Html::default(),
    };
    let door_shown = door != record::Door::Hidden;
    {
        let on_blank = records.as_ref().map(|records| records.on_blank.clone());
        use_effect_with(blank, move |blank| {
            if let Some(on_blank) = on_blank {
                on_blank.emit(*blank);
            }
        });
    }
    // Focus goes to the slot as a recording starts, and stays there while it
    // runs (S2.4): Return is the slot, Esc is Stop. It comes back to the box
    // when the recording ends — the conversation moves `focus` for that.
    {
        let slot_ref = slot_ref.clone();
        use_effect_with(recording_now, move |recording| {
            if *recording {
                if let Some(button) = slot_ref.cast::<HtmlElement>() {
                    let _ = button.focus();
                }
            }
        });
    }
    // What activating the slot does, row by row.
    let act = {
        let send = send.clone();
        let on_slot = records.as_ref().map(|records| records.on_slot.clone());
        Callback::from(move |_: ()| match slot {
            Some(Slot::Send | Slot::Save { .. }) => send.emit(()),
            Some(Slot::SendDisabled | Slot::Recorder) | None => {}
            Some(_) => {
                if let Some(on_slot) = &on_slot {
                    on_slot.emit(());
                }
            }
        })
    };
    // The last press on the slot (see [`Press`]) — and the `click` to
    // swallow after a touch acted on its `pointerup` (S8.8).
    let pressed = use_mut_ref(|| Option::<Press>::None);
    let swallow_until = use_mut_ref(|| 0.0f64);
    let menu_open = use_state(|| false);
    let reason_id = use_memo((), |_| crate::views::dialog::fresh_id("slot-reason"));
    let video_reason_id = use_memo((), |_| crate::views::dialog::fresh_id("video-reason"));
    // Whether a press going down NOW goes down while the slot ignores
    // activation: inside its 600 ms guard (S1.1), or while the microphone is
    // being asked for, when the slot is a microphone that ignores clicks.
    let ignores_now = {
        let records = records.clone();
        Rc::new(move || {
            records.as_ref().is_some_and(|records| records.starting) || guarded(records.as_ref())
        })
    };
    let on_slot_down = {
        let pressed = pressed.clone();
        let swallow_until = swallow_until.clone();
        let ignores_now = ignores_now.clone();
        Callback::from(move |event: PointerEvent| {
            let kind = event.pointer_type();
            // A mouse or a pen going down starts a click of its own: no
            // touch's `click` is still to come.
            if kind != "touch" {
                *swallow_until.borrow_mut() = 0.0;
            }
            *pressed.borrow_mut() = Some(Press::down(kind, ignores_now()));
        })
    };
    // A TOUCH acts on its `pointerup` inside the button — which is user
    // activation where the `pointerdown` is not, and comes whether or not
    // the browser sends a `click` after a long press. A mouse, a pen and
    // the keyboard keep `click`.
    let on_slot_up = {
        let pressed = pressed.clone();
        let swallow_until = swallow_until.clone();
        let slot_ref = slot_ref.clone();
        let act = act.clone();
        Callback::from(move |event: PointerEvent| {
            let touch = event.pointer_type() == "touch";
            // A touch acts now or never: whatever follows is not this press.
            let ignored = pressed.borrow_mut().as_mut().is_some_and(|press| {
                press.up_ms = Some(now_ms());
                touch && press.spend()
            });
            if !touch {
                return;
            }
            let inside = slot_ref.cast::<web_sys::Element>().is_some_and(|button| {
                let rect = button.get_bounding_client_rect();
                let (x, y) = (f64::from(event.client_x()), f64::from(event.client_y()));
                x >= rect.left() && x <= rect.right() && y >= rect.top() && y <= rect.bottom()
            });
            if inside {
                *swallow_until.borrow_mut() = now_ms() + TOUCH_CLICK_MS;
                if !ignored {
                    act.emit(());
                }
            }
        })
    };
    let on_slot_click = {
        let swallow_until = swallow_until.clone();
        let pressed = pressed.clone();
        let act = act.clone();
        Callback::from(move |_: MouseEvent| {
            if now_ms() < *swallow_until.borrow() {
                *swallow_until.borrow_mut() = 0.0;
                return;
            }
            // The press this click ends: one that went down while the slot
            // ignored activation is ignored whole, however long after the
            // guard it comes up (S1.1).
            let ignored = pressed.borrow_mut().as_mut().is_some_and(Press::spend);
            if !ignored {
                act.emit(());
            }
        })
    };
    // A press that comes up anywhere but on the slot was no click on it: it
    // is spent, so that no later activation — one that comes with no press
    // of its own, an assistive technology's — is taken for it.
    {
        let pressed = pressed.clone();
        let slot_ref = slot_ref.clone();
        use_effect_with((), move |_| {
            let listener = Closure::<dyn Fn(web_sys::Event)>::new(move |event: web_sys::Event| {
                let on_slot = slot_ref.cast::<web_sys::Node>().is_some_and(|slot| {
                    event
                        .target()
                        .and_then(|target| target.dyn_into::<web_sys::Node>().ok())
                        .is_some_and(|target| slot.contains(Some(&target)))
                });
                if !on_slot {
                    if let Some(press) = pressed.borrow_mut().as_mut() {
                        press.spent = true;
                    }
                }
            });
            let document = web_sys::window().and_then(|window| window.document());
            if let Some(document) = &document {
                let _ = document.add_event_listener_with_callback_and_bool(
                    "pointerup",
                    listener.as_ref().unchecked_ref(),
                    true,
                );
            }
            move || {
                if let Some(document) = document {
                    let _ = document.remove_event_listener_with_callback_and_bool(
                        "pointerup",
                        listener.as_ref().unchecked_ref(),
                        true,
                    );
                }
            }
        });
    }
    // THE MICROPHONE'S MENU: a right-click, Shift+F10 or the Menu key — and
    // never a touch held down, which on a phone's browser would be the
    // browser's own menu or callout, on every layout (S1.6, S8.7, S8.8).
    let on_slot_menu = {
        let pressed = pressed.clone();
        let menu_open = menu_open.clone();
        Callback::from(move |event: MouseEvent| {
            let said_touch = js_sys::Reflect::get(&event, &"pointerType".into())
                .ok()
                .and_then(|kind| kind.as_string())
                .is_some_and(|kind| kind == "touch");
            let touching = pressed.borrow().as_ref().is_some_and(|press| {
                press.kind == "touch" && press.up_ms.is_none_or(|up| now_ms() - up < TOUCH_CLICK_MS)
            });
            if said_touch || touching {
                event.prevent_default();
                return;
            }
            if is_microphone {
                event.prevent_default();
                menu_open.set(true);
            }
        })
    };
    let on_slot_key = {
        let menu_open = menu_open.clone();
        let pressed = pressed.clone();
        let ignores_now = ignores_now.clone();
        Callback::from(move |event: KeyboardEvent| {
            let key = event.key();
            if key == "ContextMenu" || (key == "F10" && event.shift_key()) {
                event.prevent_default();
                if is_microphone {
                    menu_open.set(true);
                }
            } else if key == "Enter" || key == " " {
                if event.repeat() {
                    // A key held down is one press: its repeats click
                    // nothing — not the Send arrow of the recording the
                    // first one started.
                    if key == "Enter" {
                        event.prevent_default();
                    }
                } else {
                    // Enter and Space are presses too; Space clicks only as
                    // it comes up.
                    *pressed.borrow_mut() = Some(Press::down("key".into(), ignores_now()));
                }
            }
        })
    };
    // Opened, the menu takes the focus, so Esc and Enter reach it.
    {
        let open = *menu_open && is_microphone;
        use_effect_with(open, move |open| {
            if *open {
                if let Some(item) = web_sys::window()
                    .and_then(|window| window.document())
                    .and_then(|document| {
                        document
                            .query_selector(".slot-menu [role=menuitem]")
                            .ok()
                            .flatten()
                    })
                    .and_then(|element| element.dyn_into::<HtmlElement>().ok())
                {
                    let _ = item.focus();
                }
            }
        });
    }
    let close_menu = {
        let menu_open = menu_open.clone();
        let slot_ref = slot_ref.clone();
        Callback::from(move |back: bool| {
            menu_open.set(false);
            if back {
                if let Some(button) = slot_ref.cast::<HtmlElement>() {
                    let _ = button.focus();
                }
            }
        })
    };
    // Esc while a recording runs is STOP — never Delete (S2.4, S8.7).
    let on_row_key = {
        let on_stop = records.as_ref().map(|records| records.on_stop.clone());
        Callback::from(move |event: KeyboardEvent| {
            if recording_now && event.key() == "Escape" {
                event.prevent_default();
                if let Some(on_stop) = &on_stop {
                    on_stop.emit(());
                }
            }
        })
    };
    let slot_html = match slot {
        // A thread: today's Send, unchanged.
        None => html! {
            <button
                onclick={let send = send.clone(); Callback::from(move |_: MouseEvent| send.emit(()))}
                disabled={empty || props.busy}
            >
                { if editing.is_some() { t("Save") } else { t("Send") } }
            </button>
        },
        Some(slot) => {
            let starting = records.as_ref().is_some_and(|records| records.starting);
            let (glyph, title, disabled) = match slot {
                Slot::Recorder => (Glyph::Microphone, None, true),
                Slot::HeldMicrophone | Slot::SendVoice => {
                    (Glyph::Send, Some(t("Send voice message")), false)
                }
                Slot::StopRecording => (Glyph::Stop, Some(t("Stop recording")), false),
                Slot::Save { enabled } => (Glyph::Save, None, !enabled || props.busy),
                Slot::Send => (Glyph::Send, None, props.busy || starting),
                Slot::SendDisabled => (Glyph::Send, None, true),
                Slot::Dimmed(_) | Slot::Microphone => {
                    (Glyph::Microphone, Some(t("Record a voice message")), false)
                }
            };
            // DIMMED, NOT DISABLED (S1.3, S1.7): it looks unavailable, stays
            // focusable and clickable, carries its reason, and says it when
            // activated. The slot is never `disabled` while it records.
            let reason = slot.notice().map(t);
            let word = slot_word(slot);
            let menu_item_dimmed = reason.is_some();
            html! {
                <div class="slot-wrap">
                    <button
                        ref={slot_ref.clone()}
                        type="button"
                        class={classes!(
                            "slot",
                            format!("is-{}", glyph.name()),
                            reason.is_some().then_some("is-dimmed"),
                        )}
                        aria-label={word}
                        title={title}
                        aria-disabled={reason.is_some().then_some("true")}
                        aria-describedby={reason.is_some().then(|| (*reason_id).clone())}
                        {disabled}
                        onclick={on_slot_click}
                        onpointerdown={on_slot_down}
                        onpointerup={on_slot_up}
                        oncontextmenu={on_slot_menu}
                        onkeydown={on_slot_key}
                    >
                        { glyph.draw() }
                        <span class="visually-hidden">{ word }</span>
                    </button>
                    if let Some(reason) = reason {
                        <span id={(*reason_id).clone()} hidden=true>{ reason }</span>
                    }
                    if *menu_open && is_microphone {
                        <div class="menu-backdrop" aria-hidden="true"
                             onclick={close_menu.reform(|_: MouseEvent| false)}></div>
                        <div class="menu slot-menu" role="menu"
                             onkeydown={{
                                 let close_menu = close_menu.clone();
                                 Callback::from(move |event: KeyboardEvent| {
                                     if event.key() == "Escape" {
                                         event.prevent_default();
                                         close_menu.emit(true);
                                     }
                                 })
                             }}>
                            <button
                                role="menuitem"
                                class={classes!(menu_item_dimmed.then_some("is-dimmed"))}
                                aria-disabled={menu_item_dimmed.then_some("true")}
                                aria-describedby={menu_item_dimmed.then(|| (*reason_id).clone())}
                                onclick={{
                                    let close_menu = close_menu.clone();
                                    let on_record = records.as_ref().map(|records| records.on_record.clone());
                                    Callback::from(move |_: MouseEvent| {
                                        close_menu.emit(false);
                                        if let Some(on_record) = &on_record {
                                            on_record.emit(());
                                        }
                                    })
                                }}
                            >
                                { t("Record Voice Message") }
                            </button>
                            if let Some(video) = video.clone() {
                                { video_menu_item(slot, video, close_menu.clone(), &video_reason_id) }
                            }
                        </div>
                    }
                </div>
            }
        }
    };
    // Files pasted into the box are staged; words stay the box's own.
    let on_paste = {
        let on_files = props.on_files.clone();
        let takes_files = props.takes_files;
        Callback::from(move |event: Event| {
            if !takes_files {
                return;
            }
            let Some(data) = wasm_bindgen::JsCast::dyn_ref::<web_sys::ClipboardEvent>(&event)
                .and_then(web_sys::ClipboardEvent::clipboard_data)
            else {
                return;
            };
            let files: Vec<File> = data
                .files()
                .map(|list| {
                    (0..list.length())
                        .filter_map(|index| list.get(index))
                        .collect()
                })
                .unwrap_or_default();
            let names: Vec<String> = files.iter().map(File::name).collect();
            let text = data.get_data("text/plain").unwrap_or_default();
            if media::paste_decision(&names, &text) == media::PasteDecision::Attach {
                event.prevent_default();
                on_files.emit(files);
            }
        })
    };

    html! {
        <div class="composer-wrap" ref={wrap_ref}>
            if let Some(reply) = props.replying.clone() {
                <div class="composer-banner">
                    <span class="banner-text">
                        { t1("Replying to %@", &reply.name) }
                        if !reply.excerpt.is_empty() { { format!(": {}", reply.excerpt) } }
                    </span>
                    <button class="link" onclick={cancel.clone()} aria-label={t("Cancel reply")}>{ "✕" }</button>
                </div>
            }
            if editing.is_some() {
                <div class="composer-banner">
                    <span class="banner-text">{ t("Editing message") }</span>
                    <button class="link" onclick={cancel} aria-label={t("Cancel editing")}>{ "✕" }</button>
                </div>
            }
            if let Some(message) = (*notice).clone() {
                <LiveRegion class="composer-notice" role="status">{ message }</LiveRegion>
            }
            if let Some(sentence) = props.pictures.notice(&text, props.is_ai_chat, props.is_family_chat).filter(|_| editing.is_none()) {
                <p class="picture-notice" role="note"><span aria-hidden="true">{ "👁 " }</span>{ sentence }</p>
            }
            // While a picture is being asked for: the image model's filter
            // refuses real names and brands (docs/protocol.md, "Pictures").
            if assistant_pictures::shows_picture_hint(chat_kind, &text, editing.is_some(), server_draws) {
                <LiveRegion class="picture-notice picture-hint" role="status">{ assistant_pictures::picture_hint() }</LiveRegion>
            }
            if editing.is_none() && assistant_consent::is_required(chat_kind, &text, processor.as_deref(), props.agreed_to_assistant) {
                <AssistantConsentBar
                    processor={processor.clone().unwrap_or_default()}
                    on_review={props.on_review_consent.clone()}
                />
            } else if editing.is_none() && assistant_consent::is_withheld_from_an_unnamed_assistant(chat_kind, &text, props.assistant.is_some(), processor.as_deref()) {
                // Nothing to agree TO on such a server, so there is no
                // screen to raise — only this, and a send that does not
                // happen.
                <p class="consent-notice" role="note">
                    <span aria-hidden="true">{ "✋ " }</span>
                    { assistant_consent::unnamed_processor_notice() }
                </p>
            }
            if !suggestions.is_empty() {
                <div class="suggestions" role="listbox" aria-label={t("Members")}>
                    { for suggestions.iter().enumerate().map(|(index, member)| {
                        let accept = accept.clone();
                        let name = member.display_name.clone();
                        html! {
                            <button
                                role="option"
                                class={classes!((index == *active).then_some("is-active"))}
                                aria-selected={(index == *active).to_string()}
                                onmousedown={Callback::from(move |event: MouseEvent| {
                                    event.prevent_default();
                                    accept.emit(name.clone());
                                })}
                            >
                                { &member.display_name }
                                if !member.username.is_empty() {
                                    <span class="meta">{ format!("  @{}", member.username) }</span>
                                }
                            </button>
                        }
                    }) }
                </div>
            }
            <div class="composer" onkeydown={on_row_key}>
                if recording_now {
                    // The recording row takes the field's place, and the
                    // paperclip's, the sticker's and `@ai`'s, at the same
                    // height (S2.4).
                    { records.as_ref().map(|records| records.row.clone()).unwrap_or_default() }
                } else {
                    { props.attach.clone() }
                    if offers_ai {
                        <button class="tool" title={t("Ask the assistant")} aria-label={t("Ask the assistant")} onclick={ask_assistant}>{ "✨" }</button>
                    }
                    if can_draw && editing.is_none() {
                        <button class="tool" title={t("Ask for a picture")} aria-label={t("Ask for a picture")} onclick={ask_picture}>{ "🎨" }</button>
                    }
                }
                // Kept while a recording hides it, words and all: they are
                // the box's own, and come back with it.
                <textarea
                    ref={area}
                    class={classes!(door_shown.then_some("has-door"))}
                    aria-label={t("Message")}
                    rows="1"
                    hidden={recording_now}
                    value={(*text).clone()}
                    oninput={on_input}
                    onkeydown={on_key}
                    onpaste={on_paste}
                />
                // Inside the field, at its trailing edge (S1.4).
                { door_html }
                { slot_html }
            </div>
        </div>
    }
}

/// "Record Video Message" in the microphone's menu (S1.6): it opens the
/// recorder — in row 9 too, since the not-sent rule is about voice — and in
/// rows 7 and 8, or in a browser that cannot record one, it says why
/// instead.
fn video_menu_item(
    slot: Slot,
    video: VideoEntry,
    close_menu: Callback<bool>,
    reason_id: &str,
) -> Html {
    let reason = match slot {
        Slot::Dimmed(reason @ (record::Dimmed::Call | record::Dimmed::Busy)) => {
            Some(t(reason.notice()))
        }
        _ if !video.records => Some(cannot_record_video()),
        _ => None,
    };
    let onclick = {
        let reason = reason.map(str::to_string);
        Callback::from(move |_: MouseEvent| {
            close_menu.emit(false);
            match &reason {
                Some(reason) => video.on_explain.emit(reason.clone()),
                None => video.on_open.emit(()),
            }
        })
    };
    let dimmed = reason.is_some();
    html! {
        <>
            <button
                role="menuitem"
                class={classes!(dimmed.then_some("is-dimmed"))}
                aria-disabled={dimmed.then_some("true")}
                aria-describedby={dimmed.then(|| reason_id.to_string())}
                {onclick}
            >
                { t("Record Video Message") }
            </button>
            if let Some(reason) = reason {
                <span id={reason_id.to_string()} hidden=true>{ reason }</span>
            }
        </>
    }
}

/// The cursor into the box.
fn focus_the_box(area: &NodeRef) {
    if let Some(area) = area.cast::<HtmlTextAreaElement>() {
        let _ = area.focus();
    }
}

/// Whether the focus is inside `wrap` — or on nothing at all, the page
/// itself, where a control that had it went away under it.
fn focus_is_within(wrap: &NodeRef) -> bool {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return false;
    };
    let Some(active) = document.active_element() else {
        return true;
    };
    let on_the_page = document
        .body()
        .is_some_and(|body| body.unchecked_ref::<web_sys::Element>() == &active);
    on_the_page
        || wrap
            .cast::<web_sys::Node>()
            .is_some_and(|wrap| wrap.contains(Some(&active)))
}

/// A textarea as tall as what it holds, inside the ceiling the stylesheet
/// sets. `height: auto` comes first, or `scroll_height` only ever reports
/// what the box already grew to; the borders are added back because
/// everything here is `box-sizing: border-box` and `scroll_height` counts
/// the padding but not them.
fn fit(area: &HtmlTextAreaElement) {
    let style = area.style();
    let _ = style.set_property("height", "auto");
    let borders = (area.offset_height() - area.client_height()).max(0);
    let _ = style.set_property("height", &format!("{}px", area.scroll_height() + borders));
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    fn member(id: i64, name: &str) -> Member {
        Member {
            id,
            display_name: name.into(),
            username: name.to_lowercase(),
            role: Some("member".into()),
            deleted: false,
            ..Default::default()
        }
    }

    /// The Send slot the way a conversation gives it, counting the times
    /// it was activated as a microphone (or as the recording's Send arrow
    /// or Stop square).
    fn slot_records(activated: std::rc::Rc<std::cell::Cell<u32>>) -> Records {
        Records {
            recording: record::Recording::None,
            row: Html::default(),
            call: false,
            not_sent: false,
            can_record: true,
            starting: false,
            on_slot: Callback::from(move |_: ()| activated.set(activated.get() + 1)),
            on_record: Callback::noop(),
            on_stop: Callback::noop(),
            on_emptied: Callback::noop(),
            on_changed: Callback::noop(),
            guard_until: Callback::from(|_: ()| 0),
            on_blank: Callback::noop(),
            refocus: 0,
            recorder_open: false,
            video: None,
        }
    }

    /// At send, the text is resolved against the roster — longest name
    /// first, so "@Anna Lee" is Anna Lee and not also Anna — and only in
    /// the family chat.
    #[wasm_bindgen_test]
    fn mentions_are_resolved_from_the_text_at_send() {
        let roster = vec![member(9, "Anna"), member(12, "Anna Lee"), member(7, "Me")];
        let found = resolve_mentions("@Anna Lee and @Anna, dinner?", &roster, true);
        assert_eq!(
            found,
            vec![
                Mention {
                    user_id: 12,
                    name: "Anna Lee".into()
                },
                Mention {
                    user_id: 9,
                    name: "Anna".into()
                },
            ]
        );
        assert!(
            resolve_mentions("@Anna", &roster, false).is_empty(),
            "a direct chat names nobody"
        );
        assert!(resolve_mentions("no names here", &roster, true).is_empty());
    }

    /// The server refuses a twenty-first; so this never sends one.
    #[wasm_bindgen_test]
    fn no_more_than_twenty_are_named() {
        let roster: Vec<Member> = (1..=25).map(|n| member(n, &format!("M{n:02}"))).collect();
        let body: String = (1..=25).map(|n| format!("@M{n:02} ")).collect();
        assert_eq!(resolve_mentions(&body, &roster, true).len(), MAX_MENTIONS);
    }

    /// Saving an edit that changed nothing is done at once — nothing to
    /// send, and no edit mode to be stuck in.
    #[wasm_bindgen_test]
    async fn saving_an_unchanged_edit_leaves_edit_mode_and_sends_nothing() {
        use std::cell::{Cell, RefCell};
        use std::rc::Rc;
        use wasm_bindgen::JsCast;
        use web_sys::HtmlElement;

        let cancelled = Rc::new(Cell::new(0));
        let saved = Rc::new(RefCell::new(Vec::new()));
        let props = ComposerProps {
            chat_id: 42,
            is_family_chat: true,
            is_ai_chat: false,
            my_user_id: 7,
            members: Vec::new(),
            blocked: HashSet::new(),
            assistant: None,
            agreed_to_assistant: true,
            on_review_consent: Callback::noop(),
            replying: None,
            editing: Some(Editing {
                message_id: 5,
                body: "hello".into(),
            }),
            initial: String::new(),
            on_send: Callback::noop(),
            on_save_edit: {
                let saved = saved.clone();
                Callback::from(move |edit: (i64, String)| saved.borrow_mut().push(edit))
            },
            on_cancel: {
                let cancelled = cancelled.clone();
                Callback::from(move |_: ()| cancelled.set(cancelled.get() + 1))
            },
            on_typing: Callback::noop(),
            on_draft: Callback::noop(),
            in_thread: false,
            focus: 0,
            attach: Html::default(),
            staged: 0,
            busy: false,
            append: (0, String::new()),
            take: 0,
            on_take: Callback::noop(),
            on_files: Callback::noop(),
            takes_files: false,
            pictures: Pictures::default(),
            records: Some(slot_records(Rc::new(Cell::new(0)))),
        };
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("div").unwrap();
        document.body().unwrap().append_child(&root).unwrap();
        let handle = yew::Renderer::<Composer>::with_root_and_props(root.clone(), props).render();
        gloo_timers::future::TimeoutFuture::new(20).await;
        // The slot keeps its word as hidden text, so "Save" is still what
        // it reads (S8.7) — and an edit is never a microphone (S1.3 row 4).
        let slot = || {
            let buttons = root.query_selector_all(".composer button").unwrap();
            buttons
                .item(buttons.length() - 1)
                .unwrap()
                .dyn_into::<HtmlElement>()
                .unwrap()
        };
        let save = || {
            let last = slot();
            assert_eq!(last.text_content().unwrap_or_default(), "Save");
            assert_eq!(last.get_attribute("aria-label").as_deref(), Some("Save"));
            last.click();
        };

        save();
        gloo_timers::future::TimeoutFuture::new(20).await;
        assert_eq!(cancelled.get(), 1);
        assert!(saved.borrow().is_empty());

        let area: HtmlTextAreaElement = root
            .query_selector("textarea")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        let init = web_sys::EventInit::new();
        init.set_bubbles(true);
        area.set_value("");
        area.dispatch_event(&web_sys::Event::new_with_event_init_dict("input", &init).unwrap())
            .unwrap();
        gloo_timers::future::TimeoutFuture::new(20).await;
        assert_eq!(slot().get_attribute("aria-label").as_deref(), Some("Save"));
        assert!(
            slot().has_attribute("disabled"),
            "Save waits for words, as it always has"
        );
        area.set_value("hello!");
        area.dispatch_event(&web_sys::Event::new_with_event_init_dict("input", &init).unwrap())
            .unwrap();
        gloo_timers::future::TimeoutFuture::new(20).await;
        save();
        gloo_timers::future::TimeoutFuture::new(20).await;
        assert_eq!(*saved.borrow(), vec![(5, "hello!".to_string())]);
        assert_eq!(cancelled.get(), 1);

        handle.destroy();
        root.remove();
    }

    /// The disclosure a composer shows is the port's, decided from what is
    /// staged, what is quoted, the draft and the locks.
    #[wasm_bindgen_test]
    fn the_picture_disclosure_follows_the_draft_and_the_locks() {
        let photo = Candidate::new("photo", "image/jpeg", Some(40_000));
        let open = Pictures {
            staged: vec![photo.clone()],
            server_can_see: true,
            server_can_draw: true,
            family_allows: true,
            family_history: true,
            ..Pictures::default()
        };
        let family = |draft: &str, pictures: &Pictures| pictures.notice(draft, false, true);
        assert!(family("@ai what is this?", &open)
            .unwrap()
            .starts_with("This goes to the model"));
        assert_eq!(
            family("what is this?", &open),
            None,
            "no mention, no disclosure"
        );
        let shut = Pictures {
            family_allows: false,
            ..open.clone()
        };
        assert_eq!(family("@ai what is this?", &shut), None);
        assert_eq!(
            open.notice("@ai look", false, false),
            None,
            "a direct chat carries none"
        );
        // The assistant's own chat says it of every staged photo.
        assert!(shut
            .notice("", true, false)
            .unwrap()
            .starts_with("The assistant on this server can't look at pictures"));
        assert!(open
            .notice("", true, false)
            .unwrap()
            .ends_with("Nothing else from this chat does."));
    }

    /// **A message that would reach the model is not sent until this
    /// member has agreed** (docs/protocol.md, "Consenting to the
    /// assistant"). Send raises the screen that asks instead, and the
    /// words stay in the box so agreeing can finish what was started.
    ///
    /// The server refuses it anyway with `assistant_consent_required`;
    /// what this pins is that the person is ASKED rather than shown a
    /// failed bubble.
    #[wasm_bindgen_test]
    async fn a_send_that_would_reach_the_model_asks_first() {
        use std::cell::Cell;
        use std::cell::RefCell;
        use std::rc::Rc;
        use wasm_bindgen::JsCast;
        use web_sys::HtmlElement;

        // The assistant's own chat, where every word travels; and the
        // family chat, where only an `@ai` does — the second is the one
        // that must still send ordinary words.
        for (is_ai, is_family, body, asks) in [
            (true, false, "are you there?", true),
            (false, true, "@ai when is dinner?", true),
            (false, true, "dinner at 7?", false),
        ] {
            let sent = Rc::new(RefCell::new(Vec::new()));
            let asked = Rc::new(Cell::new(0));
            let props = ComposerProps {
                chat_id: 42,
                is_family_chat: is_family,
                is_ai_chat: is_ai,
                my_user_id: 7,
                members: Vec::new(),
                blocked: HashSet::new(),
                assistant: Some(Assistant {
                    user_id: 2,
                    display_name: "Assistant".into(),
                    mention: Some("@ai".into()),
                    draw: Some("/draw".into()),
                    vision: false,
                    images: false,
                    processor: Some("Microsoft — Azure OpenAI".into()),
                    transcribe: false,
                    transcribe_max_bytes: None,
                    lookups: Vec::new(),
                    greeting_weather: false,
                }),
                agreed_to_assistant: false,
                on_review_consent: {
                    let asked = asked.clone();
                    Callback::from(move |_: ()| asked.set(asked.get() + 1))
                },
                replying: None,
                editing: None,
                initial: body.to_string(),
                on_send: {
                    let sent = sent.clone();
                    Callback::from(move |draft: Draft| sent.borrow_mut().push(draft))
                },
                on_save_edit: Callback::noop(),
                on_cancel: Callback::noop(),
                on_typing: Callback::noop(),
                on_draft: Callback::noop(),
                in_thread: false,
                focus: 0,
                attach: Html::default(),
                staged: 0,
                busy: false,
                append: (0, String::new()),
                take: 0,
                on_take: Callback::noop(),
                on_files: Callback::noop(),
                takes_files: true,
                pictures: Pictures::default(),
                records: Some(slot_records(Rc::new(Cell::new(0)))),
            };
            let document = web_sys::window().unwrap().document().unwrap();
            let root = document.create_element("div").unwrap();
            document.body().unwrap().append_child(&root).unwrap();
            let handle =
                yew::Renderer::<Composer>::with_root_and_props(root.clone(), props).render();
            gloo_timers::future::TimeoutFuture::new(20).await;
            // The strip says where the words would go, and only where they
            // would go.
            assert_eq!(
                root.query_selector(".consent-notice").unwrap().is_some(),
                asks,
                "the strip, for {body:?}"
            );
            let buttons = root.query_selector_all(".composer button").unwrap();
            let send = buttons
                .item(buttons.length() - 1)
                .unwrap()
                .dyn_into::<HtmlElement>()
                .unwrap();
            // With words in the box the slot is Send (S1.3 row 5) — in the
            // assistant's chat too, where it is never a microphone.
            assert_eq!(send.get_attribute("aria-label").as_deref(), Some("Send"));
            send.click();
            gloo_timers::future::TimeoutFuture::new(20).await;
            if asks {
                assert_eq!(sent.borrow().len(), 0, "nothing was sent for {body:?}");
                assert_eq!(asked.get(), 1, "and the screen was raised for {body:?}");
                assert_eq!(
                    root.query_selector("textarea")
                        .unwrap()
                        .unwrap()
                        .dyn_into::<web_sys::HtmlTextAreaElement>()
                        .unwrap()
                        .value(),
                    body,
                    "the words stay in the box"
                );
            } else {
                assert_eq!(sent.borrow().len(), 1, "an ordinary message still goes");
                assert_eq!(asked.get(), 0);
            }
            handle.destroy();
            root.remove();
        }
    }

    /// With something staged, Send goes with no words at all — a photo
    /// needs no caption — and without, an empty box sends nothing: its slot
    /// is the microphone then (S1.3 row 10), which asks to RECORD.
    #[wasm_bindgen_test]
    async fn staged_attachments_send_with_no_caption() {
        use std::cell::{Cell, RefCell};
        use std::rc::Rc;
        use wasm_bindgen::JsCast;
        use web_sys::HtmlElement;

        for (staged, expect) in [(1usize, 1usize), (0, 0)] {
            let sent = Rc::new(RefCell::new(Vec::new()));
            let recorded = Rc::new(Cell::new(0));
            let props = ComposerProps {
                chat_id: 42,
                is_family_chat: true,
                is_ai_chat: false,
                my_user_id: 7,
                members: Vec::new(),
                blocked: HashSet::new(),
                assistant: None,
                agreed_to_assistant: true,
                on_review_consent: Callback::noop(),
                replying: None,
                editing: None,
                initial: String::new(),
                on_send: {
                    let sent = sent.clone();
                    Callback::from(move |draft: Draft| sent.borrow_mut().push(draft))
                },
                on_save_edit: Callback::noop(),
                on_cancel: Callback::noop(),
                on_typing: Callback::noop(),
                on_draft: Callback::noop(),
                in_thread: false,
                focus: 0,
                attach: Html::default(),
                staged,
                busy: false,
                append: (0, String::new()),
                take: 0,
                on_take: Callback::noop(),
                on_files: Callback::noop(),
                takes_files: true,
                pictures: Pictures::default(),
                records: Some(slot_records(recorded.clone())),
            };
            let document = web_sys::window().unwrap().document().unwrap();
            let root = document.create_element("div").unwrap();
            document.body().unwrap().append_child(&root).unwrap();
            let handle =
                yew::Renderer::<Composer>::with_root_and_props(root.clone(), props).render();
            gloo_timers::future::TimeoutFuture::new(20).await;
            let buttons = root.query_selector_all(".composer button").unwrap();
            let last = buttons
                .item(buttons.length() - 1)
                .unwrap()
                .dyn_into::<HtmlElement>()
                .unwrap();
            let label = if staged > 0 {
                "Send"
            } else {
                "Record voice message"
            };
            assert_eq!(last.get_attribute("aria-label").as_deref(), Some(label));
            last.click();
            gloo_timers::future::TimeoutFuture::new(20).await;
            assert_eq!(sent.borrow().len(), expect, "staged {staged}");
            if let Some(draft) = sent.borrow().first() {
                assert_eq!(draft.body, "");
            }
            assert_eq!(
                recorded.get(),
                u32::from(staged == 0),
                "the microphone asks to record, and only the microphone"
            );
            handle.destroy();
            root.remove();
        }
    }

    /// The hint about what the picture filter refuses: under a draft being
    /// written as a picture request, where the server draws — from the
    /// moment the 🎨 button has typed the token — and nowhere else.
    #[wasm_bindgen_test]
    async fn the_picture_hint_follows_a_picture_request() {
        use wasm_bindgen::JsCast;
        use web_sys::HtmlElement;

        const HINT: &str =
            "Describe people and things in general words — real names and brands are often refused.";
        // (ai chat, family chat, images, draft, shows)
        for (is_ai, is_family, images, draft, shows) in [
            (true, false, true, "/draw a cat", true),
            (true, false, true, "hello", false),
            (true, false, false, "/draw a cat", false),
            (false, true, true, "@ai /draw a cat", true),
            (false, true, true, "/draw a cat", false),
            (false, false, true, "/draw a cat", false),
        ] {
            let props = ComposerProps {
                chat_id: 42,
                is_family_chat: is_family,
                is_ai_chat: is_ai,
                my_user_id: 7,
                members: Vec::new(),
                blocked: HashSet::new(),
                assistant: Some(Assistant {
                    user_id: 2,
                    display_name: "Assistant".into(),
                    mention: Some("@ai".into()),
                    draw: Some("/draw".into()),
                    vision: false,
                    transcribe: false,
                    transcribe_max_bytes: None,
                    lookups: Vec::new(),
                    greeting_weather: false,
                    images,
                    processor: Some("Microsoft — Azure OpenAI".into()),
                }),
                agreed_to_assistant: true,
                on_review_consent: Callback::noop(),
                replying: None,
                editing: None,
                initial: draft.to_string(),
                on_send: Callback::noop(),
                on_save_edit: Callback::noop(),
                on_cancel: Callback::noop(),
                on_typing: Callback::noop(),
                on_draft: Callback::noop(),
                in_thread: false,
                focus: 0,
                attach: Html::default(),
                staged: 0,
                busy: false,
                append: (0, String::new()),
                take: 0,
                on_take: Callback::noop(),
                on_files: Callback::noop(),
                takes_files: true,
                pictures: Pictures::default(),
                records: None,
            };
            let document = web_sys::window().unwrap().document().unwrap();
            let root = document.create_element("div").unwrap();
            document.body().unwrap().append_child(&root).unwrap();
            let handle =
                yew::Renderer::<Composer>::with_root_and_props(root.clone(), props).render();
            gloo_timers::future::TimeoutFuture::new(20).await;
            let hint = root.query_selector(".picture-hint").unwrap();
            assert_eq!(
                hint.is_some(),
                shows,
                "{draft:?} ai={is_ai} images={images}"
            );
            if let Some(hint) = hint {
                assert_eq!(hint.text_content().unwrap_or_default(), HINT);
                // Said, not only shown: it appears as the member types.
                assert_eq!(hint.get_attribute("role").as_deref(), Some("status"));
            }

            // In the assistant's own chat on a server that draws, an empty
            // box gains the hint the moment 🎨 has typed `/draw `.
            if is_ai && images && !shows {
                let area: web_sys::HtmlTextAreaElement = root
                    .query_selector("textarea")
                    .unwrap()
                    .unwrap()
                    .dyn_into()
                    .unwrap();
                area.set_value("");
                let init = web_sys::EventInit::new();
                init.set_bubbles(true);
                area.dispatch_event(
                    &web_sys::Event::new_with_event_init_dict("input", &init).unwrap(),
                )
                .unwrap();
                gloo_timers::future::TimeoutFuture::new(20).await;
                assert!(root.query_selector(".picture-hint").unwrap().is_none());
                root.query_selector("button[aria-label='Ask for a picture']")
                    .unwrap()
                    .expect("the 🎨 button")
                    .dyn_into::<HtmlElement>()
                    .unwrap()
                    .click();
                gloo_timers::future::TimeoutFuture::new(20).await;
                assert_eq!(area.value(), "/draw ");
                assert!(
                    root.query_selector(".picture-hint").unwrap().is_some(),
                    "the hint is up before any description is typed"
                );
            }
            handle.destroy();
            root.remove();
        }
    }

    // --- The Send slot (the plan for #79, S1.3, S1.6, S1.7, S6, S8.7, S8.8) ---

    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use wasm_bindgen::JsCast;

    /// What a slot test hears from the box.
    #[derive(Default)]
    struct Heard {
        sent: RefCell<Vec<Draft>>,
        activated: Cell<u32>,
        recorded: Cell<u32>,
        stopped: Cell<u32>,
        emptied: Cell<u32>,
        changed: Cell<u32>,
    }

    fn slot_props(heard: &Rc<Heard>, records: Option<Records>) -> ComposerProps {
        ComposerProps {
            chat_id: 42,
            is_family_chat: true,
            is_ai_chat: false,
            my_user_id: 7,
            members: Vec::new(),
            blocked: HashSet::new(),
            assistant: None,
            agreed_to_assistant: true,
            on_review_consent: Callback::noop(),
            replying: None,
            editing: None,
            initial: String::new(),
            on_send: {
                let heard = heard.clone();
                Callback::from(move |draft: Draft| heard.sent.borrow_mut().push(draft))
            },
            on_save_edit: Callback::noop(),
            on_cancel: Callback::noop(),
            on_typing: Callback::noop(),
            on_draft: Callback::noop(),
            in_thread: false,
            focus: 0,
            attach: Html::default(),
            staged: 0,
            busy: false,
            append: (0, String::new()),
            take: 0,
            on_take: Callback::noop(),
            on_files: Callback::noop(),
            takes_files: true,
            pictures: Pictures::default(),
            records,
        }
    }

    /// The slot as a conversation gives it, every callback heard.
    fn heard_records(heard: &Rc<Heard>) -> Records {
        let on = |count: fn(&Heard) -> &Cell<u32>| {
            let heard = heard.clone();
            Callback::from(move |_: ()| {
                let cell = count(&heard);
                cell.set(cell.get() + 1);
            })
        };
        Records {
            recording: record::Recording::None,
            row: html! { <div class="test-row"><button>{ "Row" }</button></div> },
            call: false,
            not_sent: false,
            can_record: true,
            starting: false,
            on_slot: on(|heard| &heard.activated),
            on_record: on(|heard| &heard.recorded),
            on_stop: on(|heard| &heard.stopped),
            on_emptied: on(|heard| &heard.emptied),
            on_changed: on(|heard| &heard.changed),
            guard_until: Callback::from(|_: ()| 0),
            on_blank: Callback::noop(),
            refocus: 0,
            recorder_open: false,
            video: None,
        }
    }

    struct Mounted {
        root: web_sys::Element,
        handle: yew::AppHandle<Composer>,
    }

    impl Mounted {
        async fn with(props: ComposerProps) -> Mounted {
            let document = web_sys::window().unwrap().document().unwrap();
            let root = document.create_element("div").unwrap();
            document.body().unwrap().append_child(&root).unwrap();
            let handle =
                yew::Renderer::<Composer>::with_root_and_props(root.clone(), props).render();
            gloo_timers::future::TimeoutFuture::new(20).await;
            Mounted { root, handle }
        }

        fn slot(&self) -> HtmlElement {
            self.root
                .query_selector(".composer button.slot, .composer > button")
                .unwrap()
                .expect("the trailing button")
                .dyn_into()
                .unwrap()
        }

        /// Its reason, read the way a screen reader reads it.
        fn reason(&self) -> Option<String> {
            let id = self.slot().get_attribute("aria-describedby")?;
            self.root
                .query_selector(&format!("#{id}"))
                .ok()
                .flatten()?
                .text_content()
        }

        fn menu(&self) -> Option<web_sys::Element> {
            self.root.query_selector(".slot-menu").unwrap()
        }

        async fn type_in(&self, words: &str) {
            let area: HtmlTextAreaElement = self
                .root
                .query_selector("textarea")
                .unwrap()
                .unwrap()
                .dyn_into()
                .unwrap();
            area.set_value(words);
            let init = web_sys::EventInit::new();
            init.set_bubbles(true);
            area.dispatch_event(&web_sys::Event::new_with_event_init_dict("input", &init).unwrap())
                .unwrap();
            gloo_timers::future::TimeoutFuture::new(20).await;
        }

        fn gone(self) {
            self.handle.destroy();
            self.root.remove();
        }
    }

    /// What the slot is, as a person and a screen reader meet it.
    #[derive(Debug, PartialEq)]
    struct Seen {
        label: String,
        word: String,
        glyph: String,
        disabled: bool,
        dimmed: bool,
        reason: Option<String>,
        title: Option<String>,
    }

    fn seen(mounted: &Mounted) -> Seen {
        let slot = mounted.slot();
        let class = slot.get_attribute("class").unwrap_or_default();
        Seen {
            label: slot.get_attribute("aria-label").unwrap_or_default(),
            word: slot.text_content().unwrap_or_default(),
            glyph: class
                .split(' ')
                .find_map(|class| class.strip_prefix("is-"))
                .filter(|glyph| *glyph != "dimmed")
                .unwrap_or_default()
                .to_string(),
            disabled: slot.has_attribute("disabled"),
            dimmed: slot.get_attribute("aria-disabled").as_deref() == Some("true")
                && class.contains("is-dimmed"),
            reason: mounted.reason(),
            title: slot.get_attribute("title"),
        }
    }

    /// EVERY ROW OF S1.3, as the web draws it: one fixed icon button whose
    /// label is fc_text::record's, which keeps its word as visually hidden
    /// text (S8.7), and which is never `disabled` while it is the
    /// recording's control — dimmed, it carries its reason (S6) and stays
    /// clickable (S1.7).
    #[wasm_bindgen_test]
    async fn the_slot_is_the_plans_row_by_row() {
        let heard = Rc::new(Heard::default());
        let microphone = |reason: Option<&str>| Seen {
            label: "Record voice message".into(),
            word: "Record voice message".into(),
            glyph: "microphone".into(),
            disabled: false,
            dimmed: reason.is_some(),
            reason: reason.map(str::to_string),
            title: Some("Record a voice message".into()),
        };
        let send = |disabled: bool| Seen {
            label: "Send".into(),
            word: "Send".into(),
            glyph: "send".into(),
            disabled,
            dimmed: false,
            reason: None,
            title: None,
        };
        let records = || heard_records(&heard);
        type Shape = (
            &'static str,
            Option<Records>,
            Box<dyn Fn(&mut ComposerProps)>,
            Seen,
        );
        let cases: Vec<Shape> = vec![
            (
                "row 10: the microphone",
                Some(records()),
                Box::new(|_| {}),
                microphone(None),
            ),
            (
                "row 7: a call",
                Some(Records {
                    call: true,
                    ..records()
                }),
                Box::new(|_| {}),
                microphone(Some("You can record a message after the call.")),
            ),
            (
                "row 8: busy",
                Some(records()),
                Box::new(|props| props.busy = true),
                microphone(Some("Wait until the current attachment is done.")),
            ),
            (
                "row 9: not sent",
                Some(Records {
                    not_sent: true,
                    ..records()
                }),
                Box::new(|_| {}),
                microphone(Some(
                    "Send or delete the voice message that wasn't sent first.",
                )),
            ),
            (
                "a call before everything else",
                Some(Records {
                    call: true,
                    not_sent: true,
                    ..records()
                }),
                Box::new(|props| props.busy = true),
                microphone(Some("You can record a message after the call.")),
            ),
            (
                "row 6: the assistant's chat",
                Some(records()),
                Box::new(|props| {
                    props.is_ai_chat = true;
                    props.is_family_chat = false;
                }),
                send(true),
            ),
            (
                "row 6: a page that cannot record",
                Some(Records {
                    can_record: false,
                    ..records()
                }),
                Box::new(|_| {}),
                send(true),
            ),
            (
                "row 5: words",
                Some(records()),
                Box::new(|props| props.initial = "dinner?".into()),
                send(false),
            ),
            (
                "row 5: something staged",
                Some(Records {
                    call: true,
                    ..records()
                }),
                Box::new(|props| props.staged = 1),
                send(false),
            ),
            (
                "row 5 waits for an attachment being prepared, as Send always has",
                Some(records()),
                Box::new(|props| {
                    props.initial = "dinner?".into();
                    props.busy = true;
                }),
                send(true),
            ),
            (
                "row 2: recording an empty box",
                Some(Records {
                    recording: record::Recording::HandsFree,
                    ..records()
                }),
                Box::new(|props| props.busy = true),
                Seen {
                    label: "Send voice message".into(),
                    word: "Send voice message".into(),
                    glyph: "send".into(),
                    disabled: false,
                    dimmed: false,
                    reason: None,
                    title: Some("Send voice message".into()),
                },
            ),
            (
                "row 3: recording beside words",
                Some(Records {
                    recording: record::Recording::HandsFreeBesideDraft,
                    call: true,
                    ..records()
                }),
                Box::new(|props| props.initial = "dinner?".into()),
                Seen {
                    label: "Stop recording".into(),
                    word: "Stop recording".into(),
                    glyph: "stop".into(),
                    disabled: false,
                    dimmed: false,
                    reason: None,
                    title: Some("Stop recording".into()),
                },
            ),
            (
                "row 4: an edit, never a microphone",
                Some(records()),
                Box::new(|props| {
                    props.editing = Some(Editing {
                        message_id: 5,
                        body: "hello".into(),
                    })
                }),
                Seen {
                    label: "Save".into(),
                    word: "Save".into(),
                    glyph: "save".into(),
                    disabled: false,
                    dimmed: false,
                    reason: None,
                    title: None,
                },
            ),
        ];
        for (name, records, shape, expected) in cases {
            let mut props = slot_props(&heard, records);
            shape(&mut props);
            let mounted = Mounted::with(props).await;
            assert_eq!(seen(&mounted), expected, "{name}");
            mounted.gone();
        }

        // A thread's box is not changed (S1.3): today's Send, a word.
        let mounted = Mounted::with(slot_props(&heard, None)).await;
        let button = mounted.slot();
        assert!(!button.class_list().contains("slot"));
        assert_eq!(button.text_content().as_deref(), Some("Send"));
        assert!(button.has_attribute("disabled"));
        assert!(button.get_attribute("aria-label").is_none());
        mounted.gone();
    }

    /// The label each row says is fc_text::record's — the one every port is
    /// held to by the vectors — in the reader's language.
    #[wasm_bindgen_test]
    fn the_slots_words_are_the_shared_rules() {
        use record::Dimmed;
        for slot in [
            Slot::HeldMicrophone,
            Slot::SendVoice,
            Slot::StopRecording,
            Slot::Save { enabled: true },
            Slot::Send,
            Slot::SendDisabled,
            Slot::Dimmed(Dimmed::Call),
            Slot::Dimmed(Dimmed::Busy),
            Slot::Dimmed(Dimmed::NotSent),
            Slot::Microphone,
        ] {
            assert_eq!(Some(slot_word(slot)), slot.label(), "{slot:?}");
        }
    }

    /// ACTIVATING THE SLOT: a microphone — dimmed or not — and the
    /// recording's arrow and square are the conversation's to decide (the
    /// shared reducer explains a dimmed one); Send sends; a disabled Send
    /// does nothing.
    #[wasm_bindgen_test]
    async fn the_slot_hands_a_microphone_to_the_recorder_and_sends_words() {
        for (records, initial, activated, sent) in [
            (Records::default_for_tests(), "", 1, 0),
            (
                Records {
                    not_sent: true,
                    ..Records::default_for_tests()
                },
                "",
                1,
                0,
            ),
            (
                Records {
                    recording: record::Recording::HandsFree,
                    ..Records::default_for_tests()
                },
                "",
                1,
                0,
            ),
            (Records::default_for_tests(), "dinner?", 0, 1),
        ] {
            let heard = Rc::new(Heard::default());
            let records = Records {
                on_slot: {
                    let heard = heard.clone();
                    Callback::from(move |_: ()| heard.activated.set(heard.activated.get() + 1))
                },
                ..records
            };
            let mut props = slot_props(&heard, Some(records));
            props.initial = initial.into();
            let mounted = Mounted::with(props).await;
            mounted.slot().click();
            gloo_timers::future::TimeoutFuture::new(20).await;
            assert_eq!(heard.activated.get(), activated, "{initial:?}");
            assert_eq!(heard.sent.borrow().len(), sent, "{initial:?}");
            mounted.gone();
        }
    }

    impl Records {
        fn default_for_tests() -> Records {
            Records {
                recording: record::Recording::None,
                row: Html::default(),
                call: false,
                not_sent: false,
                can_record: true,
                starting: false,
                on_slot: Callback::noop(),
                on_record: Callback::noop(),
                on_stop: Callback::noop(),
                on_emptied: Callback::noop(),
                on_changed: Callback::noop(),
                guard_until: Callback::from(|_: ()| 0),
                on_blank: Callback::noop(),
                refocus: 0,
                recorder_open: false,
                video: None,
            }
        }
    }

    fn pointer(kind: &str, name: &str, at: &HtmlElement, inside: bool) -> web_sys::PointerEvent {
        let rect = at.get_bounding_client_rect();
        let init = web_sys::PointerEventInit::new();
        init.set_pointer_type(kind);
        init.set_bubbles(true);
        init.set_cancelable(true);
        let (x, y) = if inside {
            (
                rect.left() + rect.width() / 2.0,
                rect.top() + rect.height() / 2.0,
            )
        } else {
            (rect.right() + 60.0, rect.top() - 60.0)
        };
        init.set_client_x(x as i32);
        init.set_client_y(y as i32);
        if name == "pointerdown" && kind == "mouse" {
            init.set_button(2);
        }
        let event = web_sys::PointerEvent::new_with_event_init_dict(name, &init).unwrap();
        at.dispatch_event(&event).unwrap();
        event
    }

    fn context_menu(at: &HtmlElement) -> web_sys::MouseEvent {
        let init = web_sys::MouseEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_button(2);
        let event =
            web_sys::MouseEvent::new_with_mouse_event_init_dict("contextmenu", &init).unwrap();
        at.dispatch_event(&event).unwrap();
        event
    }

    fn key(at: &HtmlElement, key: &str, shift: bool) -> web_sys::KeyboardEvent {
        let init = web_sys::KeyboardEventInit::new();
        init.set_key(key);
        init.set_shift_key(shift);
        init.set_bubbles(true);
        init.set_cancelable(true);
        let event =
            web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
        at.dispatch_event(&event).unwrap();
        event
    }

    /// A TOUCH ACTS ON ITS `pointerup` inside the button — user activation,
    /// where its `pointerdown` is not — and the `click` a browser may send
    /// after it is swallowed, so one tap is one activation (S8.8). Lifted
    /// outside, it does nothing. A mouse keeps `click`.
    #[wasm_bindgen_test]
    async fn a_touch_acts_on_its_pointerup_and_the_click_after_it_is_swallowed() {
        let heard = Rc::new(Heard::default());
        let mounted = Mounted::with(slot_props(&heard, Some(heard_records(&heard)))).await;
        let slot = mounted.slot();

        pointer("touch", "pointerdown", &slot, true);
        assert_eq!(heard.activated.get(), 0, "never on the way down");
        pointer("touch", "pointerup", &slot, true);
        assert_eq!(heard.activated.get(), 1, "on the way up");
        slot.click();
        assert_eq!(heard.activated.get(), 1, "the click after it is swallowed");

        // A long press the browser sends no click after: the next real
        // click, later, is a click.
        pointer("touch", "pointerdown", &slot, true);
        pointer("touch", "pointerup", &slot, true);
        assert_eq!(heard.activated.get(), 2);
        let mut later = crate::recorder::testing::ClockAhead::by(TOUCH_CLICK_MS + 50.0);
        slot.click();
        assert_eq!(heard.activated.get(), 3);
        later.more(1_000.0);

        // Lifted outside the button: nothing.
        pointer("touch", "pointerdown", &slot, true);
        pointer("touch", "pointerup", &slot, false);
        assert_eq!(heard.activated.get(), 3, "lifted outside");

        // A mouse is a click, not its pointerup — even one that comes at
        // once after a touch: it went down itself.
        pointer("touch", "pointerdown", &slot, true);
        pointer("touch", "pointerup", &slot, true);
        assert_eq!(heard.activated.get(), 4);
        pointer("mouse", "pointerdown", &slot, true);
        pointer("mouse", "pointerup", &slot, true);
        assert_eq!(heard.activated.get(), 4);
        slot.click();
        assert_eq!(heard.activated.get(), 5);
        drop(later);
        mounted.gone();
    }

    /// THE MICROPHONE'S MENU (S1.6, S8.7): a right-click, the Menu key or
    /// Shift+F10 opens it — "Record Voice Message", which records — and a
    /// touch held down never does, on any layout: its `contextmenu` is
    /// prevented, and with it the browser's own menu and callout. Esc
    /// closes it, back to the slot. On Send there is none of it.
    #[wasm_bindgen_test]
    async fn the_microphones_menu_opens_for_a_right_click_or_a_key_and_never_a_touch() {
        let heard = Rc::new(Heard::default());
        let mounted = Mounted::with(slot_props(&heard, Some(heard_records(&heard)))).await;
        let slot = mounted.slot();

        pointer("touch", "pointerdown", &slot, true);
        let held = context_menu(&slot);
        gloo_timers::future::TimeoutFuture::new(20).await;
        assert!(
            held.default_prevented(),
            "the browser's own menu is held back"
        );
        assert!(mounted.menu().is_none(), "and a touch hold opens none");
        pointer("touch", "pointerup", &slot, false);
        // (Lifted outside: no tap either.)
        assert_eq!(heard.activated.get(), 0);

        // A right-click well after that touch.
        let later = crate::recorder::testing::ClockAhead::by(TOUCH_CLICK_MS + 50.0);
        pointer("mouse", "pointerdown", &slot, true);
        let right = context_menu(&slot);
        gloo_timers::future::TimeoutFuture::new(20).await;
        drop(later);
        assert!(right.default_prevented());
        let menu = mounted.menu().expect("a right-click opens the menu");
        assert_eq!(menu.get_attribute("role").as_deref(), Some("menu"));
        let item: HtmlElement = menu
            .query_selector("[role=menuitem]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        assert_eq!(item.text_content().as_deref(), Some("Record Voice Message"));
        let active = web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .active_element();
        assert_eq!(
            active.as_ref(),
            Some(item.unchecked_ref::<web_sys::Element>()),
            "it takes the focus"
        );
        item.click();
        gloo_timers::future::TimeoutFuture::new(20).await;
        assert_eq!(heard.recorded.get(), 1, "Record Voice Message records");
        assert!(mounted.menu().is_none(), "and the menu goes");

        // The keyboard: the Menu key, and Shift+F10. Esc closes, back to
        // the slot.
        for (pressed, shift) in [("ContextMenu", false), ("F10", true)] {
            let event = key(&slot, pressed, shift);
            gloo_timers::future::TimeoutFuture::new(20).await;
            assert!(event.default_prevented());
            let menu = mounted
                .menu()
                .unwrap_or_else(|| panic!("{pressed} opens the menu"));
            key(&menu.dyn_into::<HtmlElement>().unwrap(), "Escape", false);
            gloo_timers::future::TimeoutFuture::new(20).await;
            assert!(mounted.menu().is_none());
            let active = web_sys::window()
                .unwrap()
                .document()
                .unwrap()
                .active_element();
            assert_eq!(
                active.as_ref(),
                Some(slot.unchecked_ref::<web_sys::Element>()),
                "back to the slot"
            );
        }
        assert_eq!(heard.recorded.get(), 1);
        mounted.gone();

        // Dimmed, its item is dimmed too and carries the same reason — and
        // is still chosen: the recorder is what refuses, and says why.
        let heard = Rc::new(Heard::default());
        let mounted = Mounted::with(slot_props(
            &heard,
            Some(Records {
                call: true,
                ..heard_records(&heard)
            }),
        ))
        .await;
        context_menu(&mounted.slot());
        gloo_timers::future::TimeoutFuture::new(20).await;
        let item: HtmlElement = mounted
            .menu()
            .expect("a dimmed microphone has its menu")
            .query_selector("[role=menuitem]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        assert_eq!(item.get_attribute("aria-disabled").as_deref(), Some("true"));
        assert_eq!(
            item.get_attribute("aria-describedby"),
            mounted.slot().get_attribute("aria-describedby")
        );
        item.click();
        gloo_timers::future::TimeoutFuture::new(20).await;
        assert_eq!(heard.recorded.get(), 1);
        mounted.gone();

        // On Send — words in the box — the browser keeps its menu.
        let heard = Rc::new(Heard::default());
        let mut props = slot_props(&heard, Some(heard_records(&heard)));
        props.initial = "dinner?".into();
        let mounted = Mounted::with(props).await;
        let right = context_menu(&mounted.slot());
        gloo_timers::future::TimeoutFuture::new(20).await;
        assert!(!right.default_prevented());
        assert!(mounted.menu().is_none());
        mounted.gone();
    }

    /// THE ACTIVATION GUARD (S1.1): a Send pressed while the slot's own
    /// last activation is under 600 ms old — the Stop square a moment ago,
    /// say, which staged a note — sends nothing; words typed since are
    /// never guarded — the box tells the rules, which lift the guard — so
    /// "ok" sent at once still goes. A send that empties the box says so,
    /// for the microphone it turns into to be guarded.
    #[wasm_bindgen_test]
    async fn a_send_inside_the_guard_sends_nothing_unless_words_were_typed_since() {
        let heard = Rc::new(Heard::default());
        let guard = Rc::new(Cell::new(0u64));
        let records = Records {
            guard_until: {
                let guard = guard.clone();
                Callback::from(move |_: ()| guard.get())
            },
            // The rules, as fc_text::record has them: the person's change
            // lifts the guard (HoldEvent::OtherAction).
            on_changed: {
                let heard = heard.clone();
                let guard = guard.clone();
                Callback::from(move |_: ()| {
                    heard.changed.set(heard.changed.get() + 1);
                    guard.set(0);
                })
            },
            ..heard_records(&heard)
        };
        let mut props = slot_props(&heard, Some(records));
        props.staged = 1;
        let mounted = Mounted::with(props).await;
        guard.set(now_ms() as u64 + record::ACTIVATION_GUARD_MS);
        mounted.slot().click();
        gloo_timers::future::TimeoutFuture::new(20).await;
        assert!(heard.sent.borrow().is_empty(), "guarded");
        assert_eq!(heard.emptied.get(), 0);
        // Enter in the box is the same Send.
        let area: HtmlElement = mounted
            .root
            .query_selector("textarea")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        key(&area, "Enter", false);
        gloo_timers::future::TimeoutFuture::new(20).await;
        assert!(heard.sent.borrow().is_empty(), "Enter, guarded too");
        assert_eq!(heard.changed.get(), 0, "nothing typed yet");

        mounted.type_in("ok").await;
        assert!(heard.changed.get() > 0, "typing is said to the rules");
        mounted.slot().click();
        gloo_timers::future::TimeoutFuture::new(20).await;
        assert_eq!(heard.sent.borrow().len(), 1, "typed since: not guarded");
        assert_eq!(heard.sent.borrow()[0].body, "ok");
        assert_eq!(heard.emptied.get(), 1, "the box emptied, said");
        mounted.gone();

        // Once the guard has run out, a Send goes.
        let heard = Rc::new(Heard::default());
        let until = now_ms() as u64 + record::ACTIVATION_GUARD_MS;
        let records = Records {
            guard_until: Callback::from(move |_: ()| until),
            ..heard_records(&heard)
        };
        let mut props = slot_props(&heard, Some(records));
        props.staged = 1;
        let mounted = Mounted::with(props).await;
        {
            let _later =
                crate::recorder::testing::ClockAhead::by(record::ACTIVATION_GUARD_MS as f64 + 1.0);
            mounted.slot().click();
        }
        gloo_timers::future::TimeoutFuture::new(20).await;
        assert_eq!(heard.sent.borrow().len(), 1);
        mounted.gone();
    }

    #[derive(Properties, PartialEq)]
    struct QuietComposerProps {
        recording: bool,
    }

    #[function_component(Recording)]
    fn recording(props: &QuietComposerProps) -> Html {
        crate::views::quiet::use_quiet_while(props.recording);
        Html::default()
    }

    /// A thread's box, a picture being asked for in it, beside a recording
    /// somewhere in the app.
    #[function_component(QuietComposer)]
    fn quiet_composer(props: &QuietComposerProps) -> Html {
        use crate::views::quiet::QuietRoot;
        let heard = Rc::new(Heard::default());
        let mut asked = slot_props(&heard, None);
        asked.assistant = Some(Assistant {
            user_id: 2,
            display_name: "Assistant".into(),
            mention: Some("@ai".into()),
            draw: Some("/draw".into()),
            vision: false,
            transcribe: false,
            transcribe_max_bytes: None,
            lookups: Vec::new(),
            greeting_weather: false,
            images: true,
            processor: Some("Microsoft — Azure OpenAI".into()),
        });
        asked.initial = "@ai /draw a cat".into();
        html! {
            <QuietRoot>
                <Recording recording={props.recording} />
                <Composer ..asked />
            </QuietRoot>
        }
    }

    /// The box's own notices — the picture hint, the words cut at the limit —
    /// are live regions of the app's: quiet while a voice message is being
    /// recorded (the plan for #79, S6).
    #[wasm_bindgen_test]
    async fn the_boxs_notices_are_quiet_while_a_voice_message_is_recorded() {
        for recording in [false, true] {
            let document = web_sys::window().unwrap().document().unwrap();
            let root = document.create_element("div").unwrap();
            document.body().unwrap().append_child(&root).unwrap();
            let handle = yew::Renderer::<QuietComposer>::with_root_and_props(
                root.clone(),
                QuietComposerProps { recording },
            )
            .render();
            gloo_timers::future::TimeoutFuture::new(20).await;
            let expected = if recording {
                (None, Some("off".to_string()))
            } else {
                (Some("status".to_string()), None)
            };
            let hint = root
                .query_selector(".picture-hint")
                .unwrap()
                .expect("the picture hint");
            assert_eq!(
                (hint.get_attribute("role"), hint.get_attribute("aria-live")),
                expected,
                "the hint, recording={recording}"
            );
            let area: HtmlTextAreaElement = root
                .query_selector("textarea")
                .unwrap()
                .unwrap()
                .dyn_into()
                .unwrap();
            area.set_value(&"a".repeat(composer::BODY_LIMIT + 1));
            let init = web_sys::EventInit::new();
            init.set_bubbles(true);
            area.dispatch_event(&web_sys::Event::new_with_event_init_dict("input", &init).unwrap())
                .unwrap();
            gloo_timers::future::TimeoutFuture::new(20).await;
            let notice = root
                .query_selector(".composer-notice")
                .unwrap()
                .expect("the box says it cut the words");
            assert_eq!(
                (
                    notice.get_attribute("role"),
                    notice.get_attribute("aria-live")
                ),
                expected,
                "the notice, recording={recording}"
            );
            handle.destroy();
            root.remove();
        }
    }

    /// A KEY HELD DOWN ON THE SLOT is one press (S1.1): Enter's repeats click
    /// nothing — not the Send arrow of the recording its first press started
    /// — while the first Enter, and the menu's keys, act as they always do.
    #[wasm_bindgen_test]
    async fn a_key_held_on_the_slot_is_one_press() {
        let heard = Rc::new(Heard::default());
        let mounted = Mounted::with(slot_props(&heard, Some(heard_records(&heard)))).await;
        let slot = mounted.slot();
        let held = |repeat: bool, key: &str| {
            let init = web_sys::KeyboardEventInit::new();
            init.set_key(key);
            init.set_repeat(repeat);
            init.set_bubbles(true);
            init.set_cancelable(true);
            let event = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init)
                .unwrap();
            slot.dispatch_event(&event).unwrap();
            event.default_prevented()
        };
        assert!(!held(false, "Enter"), "the first Enter is the button's own");
        assert!(held(true, "Enter"), "its repeats click nothing");
        assert!(held(true, "Enter"));
        assert!(!held(false, " "), "Space, as ever");
        mounted.gone();
    }

    /// WHILE A RECORDING RUNS the row takes the field's place — and the
    /// paperclip's — at the same height (S2.4); the words wait hidden; focus
    /// goes to the slot; and Esc is Stop, never Delete (S8.7).
    #[wasm_bindgen_test]
    async fn while_recording_the_row_has_the_fields_place_and_esc_is_stop() {
        let heard = Rc::new(Heard::default());
        let mut props = slot_props(&heard, Some(heard_records(&heard)));
        props.attach = html! { <button class="tool paperclip">{ "📎" }</button> };
        props.initial = "dinner?".into();
        let mut mounted = Mounted::with(props).await;
        assert!(mounted.root.query_selector(".paperclip").unwrap().is_some());
        assert!(mounted.root.query_selector(".test-row").unwrap().is_none());

        let mut recording = slot_props(
            &heard,
            Some(Records {
                recording: record::Recording::HandsFreeBesideDraft,
                ..heard_records(&heard)
            }),
        );
        recording.attach = html! { <button class="tool paperclip">{ "📎" }</button> };
        recording.initial = "dinner?".into();
        mounted.handle.update(recording);
        gloo_timers::future::TimeoutFuture::new(30).await;
        assert!(
            mounted.root.query_selector(".test-row").unwrap().is_some(),
            "the row"
        );
        assert!(
            mounted.root.query_selector(".paperclip").unwrap().is_none(),
            "in the paperclip's place"
        );
        let area: HtmlTextAreaElement = mounted
            .root
            .query_selector("textarea")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        assert!(area.hidden(), "the field hidden behind it");
        assert_eq!(area.value(), "dinner?", "its words kept");
        let active = web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .active_element();
        assert_eq!(
            active.as_ref(),
            Some(mounted.slot().unchecked_ref::<web_sys::Element>()),
            "focus on the slot"
        );

        key(&mounted.slot(), "Escape", false);
        gloo_timers::future::TimeoutFuture::new(20).await;
        assert_eq!(heard.stopped.get(), 1, "Esc is Stop");
        assert_eq!(heard.activated.get(), 0, "and nothing else");
        mounted.gone();
    }
}
