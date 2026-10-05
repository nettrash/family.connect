//! Everything a person does in a chat, handled in one place.
//!
//! Views say WHAT happened — an `Action` — and never talk to the server or
//! the store themselves. That keeps every rule about a request (what is
//! optimistic, what waits for the answer, what a failure undoes, which
//! answers may and may not move a cursor) here, next to the others.

use fc_text::assistant_consent;
use fc_text::i18n::{t, t1};
use fc_text::transcript;
use std::rc::Rc;

use wasm_bindgen_futures::spawn_local;
use yew::Callback;

use crate::api::{self, ApiError, FamilyPatch, NewNote, NotePatch};
use crate::live::{Live, Opening, Panel, Viewing};
use crate::media::{MediaLoader, Variant};
use crate::model::{AiFailure, Attachment, Reaction};
use crate::outbox::Wake;
use crate::recorder::Handover;
use crate::socket::ClientFrame;
use crate::staged::Prepared;
use crate::store::{self, Draft, OpenPolls, Thread, ThreadView};
use crate::sync::{self, expiry, wake, Channels, Shared, PAGE};

/// What a person did.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    SelectChat(i64),
    LoadMore {
        chat_id: i64,
    },
    Send {
        chat_id: i64,
        draft: Draft,
    },
    /// Something prepared, into the chat's staging strip — refused past the
    /// ten a message may carry.
    Stage {
        chat_id: i64,
        item: Prepared,
    },
    Unstage {
        chat_id: i64,
        index: usize,
    },
    /// A voice recording stopped by something other than the person — the
    /// tab hidden, a call, its chat left, its microphone gone — kept as a
    /// "Voice message not sent" with the reply it was recorded under, never
    /// sent and never lost (the plan for #79, S2.8, S4). Shorter than a
    /// second, it is deleted: there is nothing worth keeping. `session` is
    /// the sign-in it was recorded in, which is the only one it may land in.
    Park {
        session: u64,
        chat_id: i64,
        recording: Handover,
        reply_to_message_id: Option<i64>,
    },
    /// A recording the person stopped that cannot be staged: finished only
    /// after its chat had been left — in review when they left — or with no
    /// room left on the strip. Kept the same way (S2.8) rather than thrown
    /// away, for a recording cannot be made again.
    ParkStopped {
        session: u64,
        chat_id: i64,
        note: Prepared,
        duration_ms: i64,
        reply_to_message_id: Option<i64>,
    },
    /// A voice message sent from the recorder — the Send slot pressed while
    /// it recorded with the box empty (the plan for #79, S1.3 row 2, S2.5):
    /// it goes alone, at once, with the reply the box was answering. Its
    /// row joins the outbox before its first byte, like every media send
    /// (docs/protocol.md, "Sending on an unreliable network"). Finished
    /// after the press, it lands only in the sign-in it was recorded in —
    /// in another chat if the chat was left meanwhile, since the person had
    /// already decided, but never in somebody else's tab after a sign-out.
    SendRecorded {
        session: u64,
        chat_id: i64,
        note: Prepared,
        reply_to_message_id: Option<i64>,
    },
    /// A not-sent voice message's own Send: it goes with ITS reply and
    /// caption, and nothing else (S2.8). `mentions` are its caption's.
    SendNotSent {
        chat_id: i64,
        id: u64,
        mentions: Vec<crate::model::Mention>,
    },
    /// A not-sent voice message's ✕ — which asked first, at ten seconds or
    /// more.
    DeleteNotSent {
        chat_id: i64,
        id: u64,
    },
    SaveEdit {
        chat_id: i64,
        message_id: i64,
        body: String,
        /// Called when the server has the edit — and only then, so a
        /// refused or lost edit leaves the words in the box.
        done: Callback<()>,
    },
    React {
        chat_id: i64,
        message_id: i64,
        emoji: String,
    },
    Unreact {
        chat_id: i64,
        message_id: i64,
    },
    Vote {
        chat_id: i64,
        message_id: i64,
        option_id: i64,
    },
    Unvote {
        chat_id: i64,
        message_id: i64,
    },
    ClosePoll {
        chat_id: i64,
        message_id: i64,
    },
    OpenThread {
        chat_id: i64,
        message_id: i64,
    },
    CloseThread,
    ShowOpenPolls {
        chat_id: i64,
    },
    HideOpenPolls,
    Report {
        user_id: i64,
        message_id: Option<i64>,
        reason: String,
    },
    /// What the ASSISTANT got wrong: a separate action from [`Action::Report`]
    /// because it is a separate endpoint with a separate reader — the people
    /// who run the server, never the family owner (docs/protocol.md,
    /// "Reporting the assistant").
    ReportAssistant {
        message_id: i64,
        reason: String,
        note: Option<String>,
    },
    /// This member's own answer to the assistant question — and nobody
    /// else's; there is no shape of this action that names a user
    /// (docs/protocol.md, "Consenting to the assistant").
    SetAssistantConsent {
        granted: bool,
    },
    /// This member's own answer to the LOOKUP question — whether the
    /// assistant may send a query it writes from their words to the
    /// providers `assistant.lookups` names (docs/protocol.md, "Consenting
    /// to the assistant", amended 2026-10-03). Never assumed: only a press
    /// of "Agree With Lookups" or the Settings screen's own "I Agree" sends
    /// `true`.
    SetAssistantLookupConsent {
        granted: bool,
    },
    /// "Agree With Lookups": the assistant consent, THEN the lookup consent
    /// — in that order and never at once, because the server refuses the
    /// second without the first (`assistant_consent_required`).
    AgreeWithLookups,
    Block {
        user_id: i64,
        blocked: bool,
    },
    Reveal {
        message_id: i64,
    },
    /// One level of a quote of a blocked member: 0 the quote, 1 the parent
    /// under it.
    RevealQuote {
        message_id: i64,
        level: u8,
    },
    /// The open chat's view says whether its reader is at the newest
    /// message — which is what decides whether the chat is being read.
    AtNewest(bool),
    /// A message's photos and videos, full size, starting at one.
    OpenViewer {
        items: Vec<Attachment>,
        index: usize,
    },
    StepViewer(usize),
    CloseViewer,
    /// Something went wrong that a person should read.
    Fail(String),
    OpenDirect {
        user_id: i64,
    },
    Retry(String),
    Discard(String),
    Typing {
        chat_id: i64,
    },
    /// The words in a chat's box as its composer goes — which it does only
    /// when the chat is left — and the reply the box was answering then. A
    /// voice note still in review there is not sent from now on, and takes
    /// the words along as its caption (the plan for #79, S2.8).
    SaveDraft {
        chat_id: i64,
        text: String,
        reply_to_message_id: Option<i64>,
    },
    DismissNotice,
    /// The family board, in the main pane instead of a chat.
    OpenBoard,
    /// The board is — or may just have come — in front of somebody: its
    /// badge's marks rise to what is on it.
    BoardShown,
    /// Pin a new note. `done` hears None once the server has it and what
    /// went wrong otherwise — and until then the editor keeps the words.
    CreateNote {
        note: NewNote,
        done: Callback<Option<String>>,
    },
    /// Change a note: its author's words, colour, size, face or event, or
    /// anybody's move. Heard the same way as a create.
    UpdateNote {
        note_id: i64,
        patch: NotePatch,
        done: Callback<Option<String>>,
    },
    /// Where a note was dropped. `done` hears when the move is over, either
    /// way, so the sticker can let go of where it was drawn in hand.
    MoveNote {
        note_id: i64,
        x: f64,
        y: f64,
        done: Callback<()>,
    },
    /// Take a note down — the author's.
    DeleteNote {
        note_id: i64,
        done: Callback<Option<String>>,
    },
    /// Going, maybe, can't — or None to take an answer back. Anybody's.
    /// `done` hears when the answer is in or refused, so a button lit at
    /// once can go back to the server's truth either way.
    AnswerEvent {
        note_id: i64,
        answer: Option<String>,
        done: Callback<()>,
    },
    /// Tick or untick one line of a task list — a STATE, not a toggle, so
    /// two phones tapping the same line cannot undo each other. Anybody's:
    /// ticking is not authorship (docs/protocol.md, "Board"). `done` hears
    /// when it is in or refused, like an answer's.
    TickTask {
        note_id: i64,
        item_id: i64,
        done_now: bool,
        done: Callback<()>,
    },
    /// Ask the assistant for a picture to sit behind an event — the
    /// AUTHOR's, drawn from the note's own title (docs/protocol.md,
    /// "Board"). `done` hears when it is in or refused, so a button that
    /// said "drawing…" can stop saying it either way — and whether the
    /// answer was the consent question, which the sheet asks.
    DrawBackdrop {
        note_id: i64,
        done: Callback<Backdrop>,
    },
    /// "Show text" under a voice note, an audio file or a video: the
    /// recording's text, for this member alone (docs/protocol.md,
    /// "Transcripts on request") — from the server's stored bytes where
    /// they will do, and otherwise from sound this device takes out of the
    /// file (fc_text::transcript::source). `ask_consent` is called instead
    /// when the answer is the consent screen — this member has not agreed,
    /// here or (the server says) since — and the member presses again once
    /// it is answered, as they do for a backdrop.
    ShowTranscript {
        chat_id: i64,
        message_id: i64,
        attachment: Attachment,
        ask_consent: Callback<()>,
    },
    /// "Hide text": folded away, and kept.
    HideTranscript {
        attachment_id: i64,
    },
    /// Peek at a note a block hides.
    RevealNote {
        note_id: i64,
    },
    /// A photo prepared for the wall, to pin at this fraction of it.
    PinPhoto {
        photo: Prepared,
        at: (f64, f64),
    },
    /// One sticker of the family's pack, sent as its own message — at
    /// once, with no caption and nothing to confirm (docs/protocol.md, "In
    /// the panel, one tap sends"). It may answer something, which is the
    /// only thing that rides with it.
    SendSticker {
        chat_id: i64,
        item_id: i64,
        reply_to_message_id: Option<i64>,
    },
    /// A picked picture into the family's pack — anybody's to add. `done`
    /// hears None once it is in and what went wrong otherwise.
    AddSticker {
        file: web_sys::File,
        label: String,
        done: Callback<Option<String>>,
    },
    /// A sticker somebody SENT, kept in the family's pack: its own bytes
    /// uploaded again and claimed, with the few words whoever keeps it may
    /// give it — adding offers a label by either door. Heard the same way.
    KeepSticker {
        attachment: Attachment,
        label: String,
        done: Callback<Option<String>>,
    },
    /// Take a sticker out of the pack — whoever added it, or the family's
    /// owner. Heard the same way.
    RemoveSticker {
        item_id: i64,
        done: Callback<Option<String>>,
    },
    /// A sticker in a chat, shown larger.
    OpenSticker(Attachment),
    CloseSticker,
    /// Open a panel from the bar — or close it.
    OpenPanel(Panel),
    ClosePanel,
    /// Read `/me` again, and everything after it when the family changed.
    RefreshAccount,
    /// The waiting screen's tick: `/me` alone, and the whole resync only
    /// once it says the request was answered (ios PendingApprovalView).
    PollAccount,
    /// Start a family, as its owner.
    CreateFamily {
        name: String,
        done: Done,
    },
    /// Join with an invite code: `done` hears `joined` or `pending`.
    JoinFamily {
        invite_code: String,
        done: Callback<Result<String, ApiError>>,
    },
    /// What a leave dialog says, from a FRESH roster (docs/protocol.md,
    /// `POST /families/leave`): who inherits, or that nobody is left.
    ReadLeaveContext {
        done: Callback<Result<LeaveContext, ApiError>>,
    },
    /// Leave the family: `done` hears who it passed to, if anybody.
    LeaveFamily {
        done: Callback<Result<Option<String>, ApiError>>,
    },
    ChangePassword {
        current: String,
        new: String,
        done: Done,
    },
    /// The account, for good — and a sign-out once the server has it.
    DeleteAccount {
        password: String,
        done: Done,
    },
    /// A birthday — the reader's own when `user_id` is None, a member's
    /// (the owner's tool) otherwise; None clears it.
    SetBirthday {
        user_id: Option<i64>,
        birthday: Option<crate::model::Birthday>,
        done: Done,
    },
    /// A new profile picture (prepared by prep::avatar), or None to go back
    /// to initials.
    SetAvatar {
        jpeg: Option<web_sys::Blob>,
        done: Done,
    },
    /// The owner's tools.
    ChangeFamily {
        patch: FamilyPatch,
        done: Done,
    },
    RotateInviteCode {
        done: Done,
    },
    DecideJoinRequest {
        id: i64,
        approve: bool,
        done: Done,
    },
    ResolveReport {
        id: i64,
        done: Done,
    },
    RemoveMember {
        user_id: i64,
        done: Done,
    },
    ResetMemberPassword {
        user_id: i64,
        new_password: String,
        done: Done,
    },
    /// The family's numbers, read afresh on every opening and kept by the
    /// view that asked (ios StatisticsView never caches them either).
    LoadStats {
        done: Callback<Result<crate::model::Stats, ApiError>>,
    },
    /// Ring somebody: the direct chat, and whether it is a video call —
    /// decided here and fixed for the call's life.
    PlaceCall {
        chat_id: i64,
        video: bool,
    },
    /// Take the call this tab is ringing with.
    AnswerCall,
    /// Refuse it — which ends it on every device of theirs.
    DeclineCall,
    /// Hang up, or cancel one still ringing: the stage decides the reason.
    EndCall,
    ToggleMute,
    ToggleCamera,
}

impl Action {
    /// What the consent screen's answer sends: the assistant alone, or the
    /// assistant and then lookups ("Agree With Lookups").
    pub fn agreement(with_lookups: bool) -> Action {
        if with_lookups {
            Action::AgreeWithLookups
        } else {
            Action::SetAssistantConsent { granted: true }
        }
    }
}

/// How an account or family change came back to the dialog that asked:
/// None when it went in, the refusal otherwise — worded by the dialog, which
/// knows what it was asking. A 401 never gets here: that is the sign-out.
pub type Done = Callback<Option<ApiError>>;

/// What leaving would mean, read from the roster as it stands now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaveContext {
    /// A member: the family goes on without them.
    Member,
    /// The owner, and this member inherits.
    Successor(String),
    /// The owner, and nobody is left: leaving deletes the family.
    LastMember,
}

/// The owner's leave dialog from a fresh roster. `next_owner_user_id` is the
/// server's prediction; ABSENT on that read — and only on that read — means
/// nobody is left. A successor the same roster does not name is no answer
/// at all, and never "last member": that dialog deletes the family.
pub fn leave_context(roster: &crate::model::Roster, owner: bool) -> Option<LeaveContext> {
    if !owner {
        return Some(LeaveContext::Member);
    }
    match roster.next_owner_user_id {
        None => Some(LeaveContext::LastMember),
        Some(id) => roster
            .members
            .iter()
            .find(|member| member.id == id)
            .map(|member| LeaveContext::Successor(member.display_name.clone())),
    }
}

/// What to tell somebody whose change to the board did not go in. The
/// protocol's `message` is English for developers; these are the words the
/// apps would use, and the fallback is that message when nothing better
/// fits.
pub fn board_failure(error: &ApiError) -> String {
    match error.code() {
        // Said to the person, never swallowed (docs/protocol.md, "Board").
        Some("board_full") => {
            t("The board is full. Take a note down to make room for this one.").to_string()
        }
        Some("not_note_author") => t("Only the person who wrote a note can change it.").to_string(),
        Some("note_not_found") => t("That note has been taken down.").to_string(),
        Some("attachment_expired") => t("The photo took too long to pin. Try again.").to_string(),
        Some("attachment_too_large") => t("That photo is too large to pin.").to_string(),
        Some("invalid_attachment") => t("The board pins photos only.").to_string(),
        Some("not_in_family") => t("You're not in a family, so there is no board.").to_string(),
        _ => error.detail(),
    }
}

/// How asking for an event's backdrop ended, for the button that asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backdrop {
    /// Drawn, or refused with a sentence on the bar: either way the button
    /// is a button again.
    Settled,
    /// The author has not agreed that their words may go to the model, so
    /// nothing was sent: the consent screen is the answer, and pressing
    /// the button again once it is answered asks again — as Send does for
    /// words in the assistant's chat.
    AskConsent,
}

/// Is this refusal the server asking for the author's consent first
/// (`assistant_consent_required`, docs/protocol.md, "Consenting to the
/// assistant")? Nothing was sent; it is a question, not a failure.
pub fn asks_for_consent(error: &ApiError) -> bool {
    error.code() == Some("assistant_consent_required")
}

/// What to tell the author whose event got no backdrop. `picture_refused`
/// is the provider's own filter declining the title, and it says what a
/// refused answer says — once, and not behind "Couldn't draw that.", which
/// would read as a fault worth a retry (docs/protocol.md, "Board"). Every
/// other failure is led by what did not happen, as the board's always are.
pub fn backdrop_failure(error: &ApiError) -> String {
    match error.code() {
        Some("picture_refused") => AiFailure::Refused.sentence().to_string(),
        _ => format!("{} {}", t("Couldn't draw that."), board_failure(error)),
    }
}

/// What to tell somebody whose change to the sticker pack did not go in —
/// said to the person, like `board_full`: the limits are the family's, and
/// a sticker that silently did not appear is one its adder goes looking
/// for (docs/protocol.md, "Sticker pack").
pub fn pack_failure(error: &ApiError) -> String {
    match error.code() {
        Some("pack_full") => pack_full().to_string(),
        Some("pack_item_too_large") | Some("attachment_too_large") | Some(api::TOO_LARGE) => {
            too_large_for_a_sticker().to_string()
        }
        Some("not_pack_item_author") => {
            t("Only the person who added a sticker, or the family owner, can remove it.")
                .to_string()
        }
        Some("pack_item_not_found") => t("That sticker has already been removed.").to_string(),
        Some("invalid_attachment") => t("A sticker is a WebP or PNG picture.").to_string(),
        Some("attachment_expired") => t("The sticker took too long to add. Try again.").to_string(),
        Some("validation") => t("A description is at most 64 characters.").to_string(),
        _ => error.detail(),
    }
}

/// Functions, not consts: a translated string is not a constant.
pub fn pack_full() -> &'static str {
    t("The family's sticker pack is full. Remove one to make room.")
}

pub fn too_large_for_a_sticker() -> &'static str {
    t("That picture is too large for a sticker.")
}

/// Why a picked picture cannot be a sticker, said beside the picker rather
/// than by a rejected request (docs/protocol.md, "Limits").
pub fn sticker_refusal(refusal: fc_text::pack::Refusal) -> &'static str {
    match refusal {
        fc_text::pack::Refusal::TooLarge => too_large_for_a_sticker(),
        fc_text::pack::Refusal::Full => pack_full(),
        fc_text::pack::Refusal::Unreadable => t("Couldn't read that file."),
        fc_text::pack::Refusal::Animated => t("An animated sticker must be a WebP file."),
    }
}

/// A colour for a new sticker, picked at random as the apps pick one, so a
/// run of new notes is not one yellow pile.
pub fn random_color() -> String {
    let palette = fc_text::board::COLORS;
    let index = (js_sys::Math::random() * palette.len() as f64) as usize;
    palette[index.min(palette.len() - 1)].to_string()
}

/// One sticker goes into the pack at a time, as one photo is pinned at a
/// time.
pub fn still_adding() -> &'static str {
    t("A sticker is still being added. Add the next one when it is in the pack.")
}

/// What the handler needs of the app.
#[derive(Clone)]
pub struct Actions {
    pub live: Live,
    pub channels: Shared<Option<Channels>>,
    pub sign_out: Callback<bool>,
    /// chat → when this client last SENT a typing frame there.
    pub last_typing: Shared<std::collections::HashMap<i64, f64>>,
    /// The media cache, for a picture this tab has just pinned: drawn from
    /// the bytes it sent rather than fetched back.
    pub media: MediaLoader,
    /// The call this tab could be on (calls.rs).
    pub calls: crate::calls::Calls,
}

impl Actions {
    pub fn callback(self) -> Callback<Action> {
        let actions = Rc::new(self);
        Callback::from(move |action: Action| actions.handle(action))
    }

    /// The sound of `attachment` as this device can send it for its text
    /// (fc_text::transcript_sound): told "too long" from its metadata
    /// before a byte is fetched where that is certain; otherwise the file
    /// is fetched through the media cache — the one a player already
    /// filled, or would — and its sound taken out. A fetch that fails is
    /// worth another try; a sound that cannot be made is not.
    async fn supplied_sound(
        &self,
        attachment: &Attachment,
        max_bytes: i64,
    ) -> Result<web_sys::Blob, transcript::Failure> {
        let max_bytes = u64::try_from(max_bytes).unwrap_or(0);
        if fc_text::transcript_sound::known_too_long(attachment.duration_ms, max_bytes) {
            return Err(transcript::Failure::TooLong);
        }
        let file = self
            .media
            .bytes(attachment.id, Variant::Original)
            .await
            .ok_or(transcript::Failure::TryAgain)?;
        crate::encode::sound_for_text(&file, attachment.mime.as_deref(), max_bytes).await
    }

    /// The request with this device's sound — `Err` when none could be
    /// made and nothing was sent, otherwise the server's answer.
    async fn ask_with_sound(
        &self,
        token: &str,
        chat_id: i64,
        message_id: i64,
        attachment: &Attachment,
        max_bytes: i64,
    ) -> Result<Result<transcript::Transcript, ApiError>, transcript::Failure> {
        let sound = self.supplied_sound(attachment, max_bytes).await?;
        Ok(api::transcript_of_sound(token, chat_id, message_id, attachment.id, &sound).await)
    }

    fn session_and_token(&self) -> Option<(u64, String)> {
        self.live
            .read(|state| state.token.clone().map(|token| (state.session, token)))
    }

    fn fail(&self, session: u64, error: &ApiError) {
        if *error == ApiError::Unauthorized {
            expiry(&self.live, session, &self.sign_out)();
            return;
        }
        let detail = error.detail();
        self.live
            .update(session, |state| state.failure = Some(detail));
    }

    /// A failure said in this client's words rather than the protocol's
    /// English, which is for developers (ios MacFamilyView) — unless it was
    /// the server asking for a slower pace, which says so.
    fn fail_saying(&self, session: u64, error: &ApiError, text: &str) {
        if *error == ApiError::Unauthorized {
            expiry(&self.live, session, &self.sign_out)();
            return;
        }
        let text = match error {
            ApiError::Throttled { .. } => error.detail(),
            _ => text.to_string(),
        };
        self.live
            .update(session, |state| state.failure = Some(text));
    }

    /// The lookup answer did not save. `assistant_consent_required` is the
    /// server saying this member has no assistant consent to stand it on —
    /// withdrawn on another device since the last `/me` — so this tab stops
    /// believing it has one too, and the screen that asks for both is what
    /// the member meets next.
    fn lookup_consent_failed(&self, session: u64, error: &ApiError) {
        if error.code() == Some("assistant_consent_required") {
            self.live
                .update(session, |state| state.store.set_assistant_consent(None));
        }
        self.fail_saying(session, error, t("Couldn't save your answer. Try again."));
    }

    fn notice(&self, session: u64, text: &str) {
        let text = text.to_string();
        self.live.update(session, |state| state.notice = Some(text));
    }

    pub fn handle(&self, action: Action) {
        let Some((session, token)) = self.session_and_token() else {
            return;
        };
        let live = self.live.clone();
        let this = self.clone();
        match action {
            Action::SelectChat(chat_id) => {
                if live.read(|state| state.open_chat == Some(chat_id) && state.panel.is_none()) {
                    return;
                }
                live.now(|state| {
                    state.board_open = false;
                    state.panel = None;
                });
                // Nothing is read until the view has decided where the chat
                // opens and says its reader is at the newest: a read
                // reported first would leave no unread messages to draw the
                // divider over (ios MacConversationView reads only once the
                // view has settled). The other chat's panels go with it.
                let held = live.now(|state| {
                    state.open_chat = Some(chat_id);
                    state.at_newest = false;
                    state.opening = None;
                    state.store.open_polls = None;
                    state.store.thread_view = None;
                    state.store.resume_point(chat_id)
                });
                spawn_local(async move {
                    match held {
                        // Messages held already: what came after them, not
                        // the newest page — which would leave a hole between
                        // the two that no page ever fills (docs/protocol.md,
                        // "Semantics": after_id from the highest id held).
                        Some((newest, counted)) => {
                            sync::catch_up_messages(
                                &live, session, &token, chat_id, newest, counted,
                            )
                            .await;
                        }
                        None => match api::messages(&token, chat_id, None, PAGE).await {
                            Ok(page) => {
                                let full = page.len() as u32 == PAGE;
                                live.update(session, |state| {
                                    state.store.apply_history(chat_id, page, full);
                                    state.failure = None;
                                });
                            }
                            Err(error) => this.fail(session, &error),
                        },
                    }
                    // Loaded, or as loaded as it will get: the unread state
                    // as it stands NOW — which a page that landed may have
                    // added to, and no read has taken from.
                    live.update(session, |state| {
                        if state.open_chat != Some(chat_id) {
                            return;
                        }
                        let unread_count = state.store.opening_unread(chat_id);
                        state.opening = state.store.item(chat_id).map(|item| Opening {
                            chat_id,
                            unread_count,
                            last_read_message_id: item.last_read_message_id,
                        });
                    });
                });
            }
            Action::LoadMore { chat_id } => {
                let Some(before) = live.read(|state| {
                    state
                        .store
                        .threads
                        .get(&chat_id)
                        .and_then(|thread| thread.oldest)
                }) else {
                    return;
                };
                spawn_local(async move {
                    match api::messages(&token, chat_id, Some(before), PAGE).await {
                        Ok(page) => {
                            let full = page.len() as u32 == PAGE;
                            live.update(session, |state| {
                                state.store.apply_history(chat_id, page, full)
                            });
                        }
                        Err(error) => this.fail(session, &error),
                    }
                });
            }
            Action::Send { chat_id, draft } => {
                // The bubble draws now and the row joins the outbox, whose
                // sender takes it from there (outbox.rs). Nothing here waits.
                live.now(|state| {
                    // What was staged went with it — when it is what went.
                    // A location goes alone and leaves the strip as it was.
                    let from_strip = !draft.attachments.is_empty()
                        && state.store.staged.get(&chat_id) == Some(&draft.attachments);
                    state
                        .store
                        .queue_send(chat_id, uuid::Uuid::new_v4().to_string(), draft);
                    state.store.drafts.remove(&chat_id);
                    if from_strip {
                        state.store.staged.remove(&chat_id);
                    }
                });
                wake(&self.channels, Wake::Queued);
            }
            Action::Stage { chat_id, item } => {
                live.now(|state| {
                    let staged = state.store.staged.entry(chat_id).or_default();
                    if fc_text::media::can_stage(staged.len()) {
                        staged.push(item);
                    }
                });
            }
            Action::Unstage { chat_id, index } => {
                live.now(|state| {
                    if let Some(staged) = state.store.staged.get_mut(&chat_id) {
                        if index < staged.len() {
                            staged.remove(index);
                        }
                        if staged.is_empty() {
                            state.store.staged.remove(&chat_id);
                        }
                    }
                });
            }
            Action::Park {
                session: recorded_in,
                chat_id,
                recording,
                reply_to_message_id,
            } => {
                let Some(active) = recording.take() else {
                    return;
                };
                // Recorded in a sign-in that has ended: it goes with it, and
                // the microphone with it, here.
                if recorded_in != session {
                    return;
                }
                spawn_local(async move {
                    // The microphone is let go of here, whatever comes of it.
                    let Some(recorded) = active.stop().await else {
                        return;
                    };
                    let duration_ms = recorded.duration_ms;
                    match crate::prep::recording(recorded.blob, recorded.mime, duration_ms).await {
                        Ok(note) => this.handle(Action::ParkStopped {
                            session,
                            chat_id,
                            note,
                            duration_ms,
                            reply_to_message_id,
                        }),
                        // Too big for this server to take at all: it could
                        // never be sent, and saying so is all there is.
                        Err(error) => {
                            live.update(session, |state| {
                                state.failure = Some(error.message().to_string());
                            });
                        }
                    }
                });
            }
            Action::ParkStopped {
                session: recorded_in,
                chat_id,
                note,
                duration_ms,
                reply_to_message_id,
            } => {
                // Under a second there is nothing worth keeping (S4).
                if duration_ms < fc_text::record::SHORTEST_RECORDING_MS as i64 {
                    return;
                }
                live.update(recorded_in, |state| {
                    state.store.park(
                        chat_id,
                        note,
                        duration_ms,
                        reply_to_message_id,
                        String::new(),
                    );
                });
            }
            Action::SendRecorded {
                session: recorded_in,
                chat_id,
                note,
                reply_to_message_id,
            } => {
                let queued = live
                    .update(recorded_in, |state| {
                        state.store.queue_send(
                            chat_id,
                            uuid::Uuid::new_v4().to_string(),
                            Draft {
                                reply_to_message_id,
                                attachments: vec![note],
                                ..Draft::default()
                            },
                        );
                    })
                    .is_some();
                if queued {
                    wake(&self.channels, Wake::Queued);
                }
            }
            Action::SendNotSent {
                chat_id,
                id,
                mentions,
            } => {
                let queued = live.now(|state| {
                    let Some(row) = state.store.take_not_sent(chat_id, id) else {
                        return false;
                    };
                    // Alone, with its own reply and caption — whatever the
                    // box holds now, and whatever is staged, stays there.
                    state.store.queue_send(
                        chat_id,
                        uuid::Uuid::new_v4().to_string(),
                        Draft {
                            body: row.caption,
                            reply_to_message_id: row.reply_to_message_id,
                            mentions,
                            attachments: vec![row.note],
                            ..Draft::default()
                        },
                    );
                    true
                });
                if queued {
                    wake(&self.channels, Wake::Queued);
                }
            }
            Action::DeleteNotSent { chat_id, id } => {
                live.now(|state| {
                    state.store.take_not_sent(chat_id, id);
                });
            }
            Action::SaveEdit {
                chat_id,
                message_id,
                body,
                done,
            } => {
                // NOT optimistic: the composer leaves edit mode only when
                // the edit is in, the way the apps do it, so a refused or
                // lost edit leaves the words where the person can fix them.
                spawn_local(async move {
                    match api::edit_message(&token, chat_id, message_id, &body).await {
                        // The answer to this device's own edit: applied under
                        // the guard, and moving NO cursor.
                        Ok(message) => {
                            if live
                                .update(session, |state| state.store.apply_edit(message))
                                .is_some()
                            {
                                done.emit(());
                            }
                        }
                        Err(error) => this.fail(session, &error),
                    }
                });
            }
            Action::React {
                chat_id,
                message_id,
                emoji,
            } => {
                let before =
                    self.set_my_reaction(session, chat_id, message_id, Some(emoji.clone()));
                spawn_local(async move {
                    match api::react(&token, chat_id, message_id, &emoji).await {
                        Ok(answer) => live.update(session, |state| {
                            state.store.apply_reactions(
                                chat_id,
                                answer.message_id,
                                answer.reaction_seq,
                                answer.reactions,
                            )
                        }),
                        Err(error) => {
                            this.roll_back_reactions(session, chat_id, message_id, before);
                            this.fail(session, &error);
                            None
                        }
                    };
                });
            }
            Action::Unreact {
                chat_id,
                message_id,
            } => {
                let before = self.set_my_reaction(session, chat_id, message_id, None);
                spawn_local(async move {
                    match api::unreact(&token, chat_id, message_id).await {
                        Ok(answer) => live.update(session, |state| {
                            state.store.apply_reactions(
                                chat_id,
                                answer.message_id,
                                answer.reaction_seq,
                                answer.reactions,
                            )
                        }),
                        Err(error) => {
                            this.roll_back_reactions(session, chat_id, message_id, before);
                            this.fail(session, &error);
                            None
                        }
                    };
                });
            }
            Action::Vote {
                chat_id,
                message_id,
                option_id,
            } => {
                spawn_local(async move {
                    let answer = api::vote(&token, chat_id, message_id, option_id).await;
                    this.apply_poll_answer(session, chat_id, answer, true);
                });
            }
            Action::Unvote {
                chat_id,
                message_id,
            } => {
                spawn_local(async move {
                    let answer = api::unvote(&token, chat_id, message_id).await;
                    this.apply_poll_answer(session, chat_id, answer, true);
                });
            }
            Action::ClosePoll {
                chat_id,
                message_id,
            } => {
                spawn_local(async move {
                    let answer = api::close_poll(&token, chat_id, message_id).await;
                    this.apply_poll_answer(session, chat_id, answer, true);
                });
            }
            Action::OpenThread {
                chat_id,
                message_id,
            } => {
                // Opened at once on whatever this client holds of it, then
                // read from the server: the root first, then every reply.
                let root_id = live.read(|state| {
                    state
                        .store
                        .find(chat_id, message_id)
                        .and_then(|message| message.thread_root_id)
                        .unwrap_or(message_id)
                });
                let generation = live.now(|state| {
                    let mut thread = Thread::default();
                    for id in [root_id, message_id] {
                        if let Some(held) = state.store.find(chat_id, id).cloned() {
                            thread.apply(held);
                        }
                    }
                    state.store.thread_openings += 1;
                    let generation = state.store.thread_openings;
                    state.store.thread_view = Some(ThreadView {
                        chat_id,
                        root_id,
                        generation,
                        thread,
                    });
                    generation
                });
                spawn_local(async move {
                    this.read_thread(session, &token, chat_id, message_id, generation)
                        .await
                });
            }
            Action::CloseThread => {
                live.now(|state| state.store.thread_view = None);
            }
            Action::ShowOpenPolls { chat_id } => {
                spawn_local(async move { this.read_open_polls(session, &token, chat_id).await });
            }
            Action::HideOpenPolls => {
                live.now(|state| state.store.open_polls = None);
            }
            Action::Report {
                user_id,
                message_id,
                reason,
            } => {
                spawn_local(async move {
                    match api::report(&token, user_id, &reason, message_id).await {
                        // Not "to the owner": a report ABOUT the owner is
                        // kept from them, and the server's answer is the same
                        // either way on purpose (docs/protocol.md, "Reporting
                        // a member") — so this says only what is true of both.
                        Ok(()) => this.notice(session, t("Report sent.")),
                        Err(error) => this.fail_saying(
                            session,
                            &error,
                            t("Couldn't send the report. Try again."),
                        ),
                    }
                });
            }
            Action::ReportAssistant {
                message_id,
                reason,
                note,
            } => {
                spawn_local(async move {
                    match api::report_assistant(&token, message_id, &reason, note.as_deref()).await
                    {
                        // The same sentence the member report answers with,
                        // and for a cousin of the same reason: what happens
                        // next is the operator's, and a client that promised
                        // more would be inventing it.
                        Ok(()) => this.notice(session, t("Report sent.")),
                        Err(error) => this.fail_saying(
                            session,
                            &error,
                            t("Couldn't send the report. Try again."),
                        ),
                    }
                });
            }
            Action::SetAssistantConsent { granted } => {
                let live = live.clone();
                spawn_local(async move {
                    match api::set_assistant_consent(&token, granted).await {
                        // The SERVER's stamp, not this client's clock:
                        // agreeing twice keeps the first one, and a client
                        // that invented a date would show one the server
                        // would not.
                        Ok(at) => live.now(move |state| state.store.set_assistant_consent(at)),
                        Err(error) => this.fail_saying(
                            session,
                            &error,
                            t("Couldn't save your answer. Try again."),
                        ),
                    }
                });
            }
            Action::SetAssistantLookupConsent { granted } => {
                spawn_local(async move {
                    match api::set_assistant_lookup_consent(&token, granted).await {
                        Ok(at) => {
                            live.update(session, move |state| {
                                state.store.set_assistant_lookup_consent(at)
                            });
                        }
                        Err(error) => this.lookup_consent_failed(session, &error),
                    }
                });
            }
            Action::AgreeWithLookups => {
                spawn_local(async move {
                    let agreed = match api::set_assistant_consent(&token, true).await {
                        Ok(at) => at,
                        Err(error) => {
                            this.fail_saying(
                                session,
                                &error,
                                t("Couldn't save your answer. Try again."),
                            );
                            return;
                        }
                    };
                    live.update(session, |state| state.store.set_assistant_consent(agreed));
                    match api::set_assistant_lookup_consent(&token, true).await {
                        Ok(at) => {
                            live.update(session, move |state| {
                                state.store.set_assistant_lookup_consent(at)
                            });
                        }
                        Err(error) => this.lookup_consent_failed(session, &error),
                    }
                });
            }
            Action::Block { user_id, blocked } => {
                // Applied when the server has it: the block is server-side,
                // and a client that hid a row the server did not hide would
                // be lying about what it had done.
                spawn_local(async move {
                    match api::set_blocked(&token, user_id, blocked).await {
                        Ok(()) => {
                            live.update(session, |state| state.store.set_blocked(user_id, blocked));
                            if !blocked {
                                let expired = expiry(&live, session, &this.sign_out);
                                sync::refresh_chats(&live, session, &token, &expired).await;
                            }
                        }
                        Err(error) => this.fail_saying(
                            session,
                            &error,
                            t("Couldn't change that right now. Try again."),
                        ),
                    }
                });
            }
            Action::Reveal { message_id } => {
                live.now(|state| {
                    state.store.revealed.insert(message_id);
                });
            }
            Action::RevealQuote { message_id, level } => {
                live.now(|state| {
                    state.store.revealed_quotes.insert((message_id, level));
                });
            }
            Action::AtNewest(at_newest) => {
                let changed = live.now(|state| {
                    let changed = state.at_newest != at_newest;
                    state.at_newest = at_newest;
                    changed
                });
                // Back at the newest is reading what is there.
                if changed && at_newest {
                    spawn_local(async move { sync::report_read(&live, session, &token).await });
                }
            }
            Action::OpenDirect { user_id } => {
                spawn_local(async move {
                    match api::direct_chat(&token, user_id).await {
                        Ok(chat) => {
                            let expired = expiry(&live, session, &this.sign_out);
                            sync::refresh_chats(&live, session, &token, &expired).await;
                            this.handle(Action::SelectChat(chat.id));
                        }
                        Err(error) => {
                            this.fail_saying(session, &error, t("That didn't work. Try again."))
                        }
                    }
                });
            }
            Action::Retry(client_msg_id) => {
                // A person asking again is as good a sign as any that it
                // might work now, so it cuts short whatever backoff the
                // sender is sitting in.
                live.now(|state| state.store.retry(&client_msg_id));
                wake(&self.channels, Wake::Network);
            }
            Action::Discard(client_msg_id) => {
                live.now(|state| state.store.discard(&client_msg_id));
            }
            Action::Typing { chat_id } => self.typing(chat_id),
            Action::SaveDraft {
                chat_id,
                text,
                reply_to_message_id,
            } => {
                live.now(|state| {
                    // A voice note left in review takes the words with it,
                    // and the box is left empty (S2.8): its caption never
                    // leaves without it, and never twice.
                    let taken = state.store.park_review(chat_id, &text, reply_to_message_id);
                    if taken || text.trim().is_empty() {
                        state.store.drafts.remove(&chat_id);
                    } else {
                        state.store.drafts.insert(chat_id, text);
                    }
                });
            }
            Action::OpenViewer { items, index } => {
                if !items.is_empty() {
                    live.now(|state| state.viewing = Some(Viewing { items, index }));
                }
            }
            Action::StepViewer(index) => {
                live.now(|state| {
                    if let Some(viewing) = state.viewing.as_mut() {
                        viewing.index = index.min(viewing.items.len().saturating_sub(1));
                    }
                });
            }
            Action::CloseViewer => {
                live.now(|state| state.viewing = None);
            }
            Action::Fail(text) => {
                live.now(|state| state.failure = Some(text));
            }
            Action::OpenPanel(panel) => {
                // A panel takes the pane the chat or the board had; nothing
                // is being read behind it (sync::reading).
                live.now(|state| {
                    state.panel = Some(panel);
                    state.board_open = false;
                    state.open_chat = None;
                    state.at_newest = false;
                    state.opening = None;
                    state.store.open_polls = None;
                    state.store.thread_view = None;
                });
                if panel == Panel::Family {
                    spawn_local(async move { this.read_family_admin(session, &token).await });
                }
            }
            Action::ClosePanel => {
                live.now(|state| state.panel = None);
            }
            Action::RefreshAccount => {
                spawn_local(async move { this.refresh_account(session, &token).await });
            }
            Action::PollAccount => {
                // One at a time: a slow server gets its answer waited for.
                if live.read(|state| state.polling) {
                    return;
                }
                let asked = live.now(|state| {
                    state.polling = true;
                    state.store.account_reads
                });
                spawn_local(async move {
                    let answer = api::me(&token).await;
                    let fresh = live
                        .update(session, |state| {
                            state.polling = false;
                            state.store.account_reads == asked
                        })
                        .unwrap_or(false);
                    match answer {
                        // Another `/me` was taken in while this one was out
                        // — the resync an approval sets off — and this answer
                        // is older than it: "still waiting" would undo it.
                        Ok(_) if !fresh => {}
                        // Approved: the whole resync, as a connection would
                        // run it — the chats, the board, the roster.
                        Ok(me) if me.family.is_some() => {
                            this.refresh_account(session, &token).await
                        }
                        // Still waiting, or declined: `/me` says which.
                        Ok(me) => sync::apply_account(&live, session, &me),
                        Err(ApiError::Unauthorized) => expiry(&live, session, &this.sign_out)(),
                        // A bad minute: the next tick asks again.
                        Err(_) => {}
                    }
                });
            }
            Action::CreateFamily { name, done } => {
                spawn_local(async move {
                    match api::create_family(&token, &name).await {
                        Ok(_) => {
                            this.refresh_account(session, &token).await;
                            done.emit(None);
                        }
                        Err(error) => this.said(session, error, &done),
                    }
                });
            }
            Action::JoinFamily { invite_code, done } => {
                spawn_local(async move {
                    match api::join_family(&token, &invite_code).await {
                        Ok(joined) => {
                            this.refresh_account(session, &token).await;
                            done.emit(Ok(joined.status));
                        }
                        Err(ApiError::Unauthorized) => expiry(&live, session, &this.sign_out)(),
                        Err(error) => {
                            // The one join that answers this: an account the
                            // server scrubbed while its join was on the way,
                            // and the session went with it (docs/protocol.md,
                            // "Accounts without a family"). `/me` says so —
                            // with the 401 that signs out.
                            if error.code() == Some("user_already_in_family") {
                                this.refresh_account(session, &token).await;
                            }
                            done.emit(Err(error));
                        }
                    }
                });
            }
            Action::ReadLeaveContext { done } => {
                spawn_local(async move {
                    let writes = live.read(|state| state.store.family_writes);
                    match api::family(&token).await {
                        Ok(roster) => {
                            live.update(session, |state| {
                                sync::take_roster(&mut state.store, roster.clone(), writes)
                            });
                            let owner = live.read(|state| state.store.is_owner());
                            // A successor the fresh roster does not name
                            // is no answer; the dialog says so rather than
                            // guess (see `leave_context`).
                            done.emit(leave_context(&roster, owner).ok_or_else(|| {
                                ApiError::Network("no successor on the roster".into())
                            }));
                        }
                        Err(ApiError::Unauthorized) => expiry(&live, session, &this.sign_out)(),
                        Err(error) => done.emit(Err(error)),
                    }
                });
            }
            Action::LeaveFamily { done } => {
                spawn_local(async move {
                    // Who inherits is named from the roster as it stood
                    // BEFORE leaving — afterwards there is none to read.
                    let names = live.read(|state| state.store.names.clone());
                    match api::leave_family(&token).await {
                        Ok(successor) => {
                            let heir = successor.and_then(|id| names.get(&id).cloned());
                            // Said on the family gate, which is where this
                            // account lands (ios "Ownership passed on").
                            if let Some(heir) = &heir {
                                this.notice(
                                    session,
                                    &format!(
                                        "{}: {}",
                                        t("Ownership passed on"),
                                        t1("%@ is now the owner of the family.", heir)
                                    ),
                                );
                            }
                            this.refresh_account(session, &token).await;
                            done.emit(Ok(heir));
                        }
                        Err(ApiError::Unauthorized) => expiry(&live, session, &this.sign_out)(),
                        Err(error) => done.emit(Err(error)),
                    }
                });
            }
            Action::ChangePassword { current, new, done } => {
                spawn_local(async move {
                    match api::change_password(&token, &current, &new).await {
                        Ok(()) => done.emit(None),
                        Err(error) => this.said(session, error, &done),
                    }
                });
            }
            Action::DeleteAccount { password, done } => {
                spawn_local(async move {
                    match api::delete_account(&token, &password).await {
                        Ok(()) => {
                            done.emit(None);
                            // Gone, and its sessions with it: nothing to
                            // revoke, only this tab to forget — if it is
                            // still the session that asked.
                            expiry(&live, session, &this.sign_out)();
                        }
                        Err(error) => this.said(session, error, &done),
                    }
                });
            }
            Action::SetBirthday {
                user_id,
                birthday,
                done,
            } => {
                spawn_local(async move {
                    let answer = match user_id {
                        None => api::set_my_birthday(&token, birthday).await.map(|user| {
                            live.update(session, |state| match user {
                                Some(user) => state.store.apply_my_user(user),
                                None => {
                                    let me = state.store.my_user_id;
                                    state.store.set_birthday(me, None);
                                }
                            });
                        }),
                        Some(user_id) => api::set_member_birthday(&token, user_id, birthday)
                            .await
                            .map(|member| {
                                live.update(session, |state| {
                                    state
                                        .store
                                        .set_birthday(user_id, member.and_then(|m| m.birthday))
                                });
                            }),
                    };
                    match answer {
                        Ok(()) => done.emit(None),
                        Err(error) => this.said(session, error, &done),
                    }
                });
            }
            Action::SetAvatar { jpeg, done } => {
                spawn_local(async move {
                    let answer = match jpeg {
                        Some(jpeg) => api::upload_avatar(&token, &jpeg).await.map(|user| {
                            // Drawn from the bytes just sent, never fetched
                            // back, under the version the server gave them.
                            if live.is_live(session) {
                                this.media.seed(
                                    user.id,
                                    crate::media::Variant::Avatar(user.avatar_version),
                                    jpeg.clone(),
                                );
                            }
                            live.update(session, |state| state.store.apply_my_user(user));
                        }),
                        // Only the version goes: the rest of the profile —
                        // the birthday — is as it was (ios drops it here).
                        None => api::delete_avatar(&token).await.map(|()| {
                            live.update(session, |state| {
                                let me = state.store.my_user_id;
                                state.store.set_avatar_version(me, 0);
                            });
                        }),
                    };
                    match answer {
                        Ok(()) => done.emit(None),
                        Err(error) => this.said(session, error, &done),
                    }
                });
            }
            Action::ChangeFamily { patch, done } => {
                spawn_local(async move {
                    match api::patch_family(&token, &patch).await {
                        Ok(family) => {
                            live.update(session, |state| state.store.apply_family(family));
                            done.emit(None);
                        }
                        Err(error) => this.said(session, error, &done),
                    }
                });
            }
            Action::RotateInviteCode { done } => {
                spawn_local(async move {
                    match api::rotate_invite_code(&token).await {
                        Ok(code) => {
                            live.update(session, |state| state.store.set_invite_code(code));
                            done.emit(None);
                        }
                        Err(error) => this.said(session, error, &done),
                    }
                });
            }
            Action::DecideJoinRequest { id, approve, done } => {
                spawn_local(async move {
                    match api::decide_join_request(&token, id, approve).await {
                        Ok(()) => {
                            live.update(session, |state| {
                                state.store.join_requests.retain(|request| request.id != id)
                            });
                            done.emit(None);
                            // A new member: the roster, and the chats it
                            // brings, as a resync reads them.
                            if approve {
                                this.refresh_account(session, &token).await;
                            }
                            this.read_family_admin(session, &token).await;
                        }
                        Err(error) => this.said(session, error, &done),
                    }
                });
            }
            Action::ResolveReport { id, done } => {
                spawn_local(async move {
                    match api::resolve_report(&token, id).await {
                        Ok(()) => {
                            live.update(session, |state| {
                                state.store.reports.retain(|report| report.id != id)
                            });
                            done.emit(None);
                        }
                        Err(error) => this.said(session, error, &done),
                    }
                });
            }
            Action::RemoveMember { user_id, done } => {
                spawn_local(async move {
                    match api::remove_member(&token, user_id).await {
                        Ok(()) => {
                            live.update(session, |state| state.store.member_left(user_id));
                            done.emit(None);
                            this.refresh_account(session, &token).await;
                        }
                        Err(error) => this.said(session, error, &done),
                    }
                });
            }
            Action::ResetMemberPassword {
                user_id,
                new_password,
                done,
            } => {
                spawn_local(async move {
                    match api::reset_member_password(&token, user_id, &new_password).await {
                        Ok(()) => done.emit(None),
                        Err(error) => this.said(session, error, &done),
                    }
                });
            }
            Action::PlaceCall { chat_id, video } => {
                // A call lives in a direct chat, and rings its other
                // member (docs/protocol.md, "Voice calls").
                let peer_user_id = live.read(|state| {
                    state
                        .store
                        .item(chat_id)
                        .filter(|item| item.chat.is_direct())
                        .and_then(|item| item.chat.peer_user_id)
                });
                if let Some(peer_user_id) = peer_user_id {
                    this.calls.place(token, chat_id, peer_user_id, video);
                }
            }
            Action::AnswerCall => this.calls.answer(),
            Action::DeclineCall => this.calls.decline(),
            Action::EndCall => this.calls.end(),
            Action::ToggleMute => this.calls.toggle_mute(),
            Action::ToggleCamera => this.calls.toggle_camera(),
            Action::LoadStats { done } => {
                spawn_local(async move {
                    match api::stats(&token).await {
                        Ok(stats) => done.emit(Ok(stats)),
                        Err(ApiError::Unauthorized) => expiry(&live, session, &this.sign_out)(),
                        Err(error) => done.emit(Err(error)),
                    }
                });
            }
            Action::DismissNotice => {
                live.now(|state| {
                    state.notice = None;
                    state.failure = None;
                });
            }
            Action::OpenBoard => {
                // The chat and its panels give way: nothing is being READ
                // while the board is in front (sync::reading), and the pane
                // a chat comes back to is a fresh one, caught up on opening.
                live.now(|state| {
                    state.board_open = true;
                    state.panel = None;
                    state.open_chat = None;
                    state.at_newest = false;
                    state.opening = None;
                    state.store.open_polls = None;
                    state.store.thread_view = None;
                });
                sync::mark_board_shown(&live, session);
                // Never read yet — the first read failed, or has not come
                // back: read it now rather than show "Loading" until the
                // socket next reconnects.
                let (loaded, link) = live.read(|state| {
                    (
                        state.store.board.loaded,
                        state.connected.then_some(state.link),
                    )
                });
                if !loaded {
                    spawn_local(async move {
                        sync::sync_board(&live, session, &token, 0, link).await;
                        sync::mark_board_shown(&live, session);
                    });
                }
            }
            Action::BoardShown => sync::mark_board_shown(&live, session),
            Action::CreateNote { note, done } => {
                spawn_local(async move {
                    match api::create_note(&token, &note).await {
                        // The answer to this device's own create: applied
                        // under the guard, and moving NO cursor.
                        Ok(note) => {
                            if live
                                .update(session, |state| state.store.board.apply(note))
                                .is_some()
                            {
                                done.emit(None);
                            }
                        }
                        Err(error) => this.board_refused(session, &error, None, &done),
                    }
                });
            }
            Action::UpdateNote {
                note_id,
                patch,
                done,
            } => {
                // An edit that changed nothing sends nothing.
                if patch.is_empty() {
                    done.emit(None);
                    return;
                }
                spawn_local(async move {
                    match api::patch_note(&token, note_id, &patch).await {
                        Ok(note) => {
                            if live
                                .update(session, |state| state.store.board.apply(note))
                                .is_some()
                            {
                                done.emit(None);
                            }
                        }
                        Err(error) => this.board_refused(session, &error, Some(note_id), &done),
                    }
                });
            }
            Action::MoveNote {
                note_id,
                x,
                y,
                done,
            } => {
                spawn_local(async move {
                    match api::patch_note(&token, note_id, &NotePatch::moved_to(x, y)).await {
                        Ok(note) => {
                            live.update(session, |state| state.store.board.apply(note));
                        }
                        Err(error) => {
                            this.board_refused(session, &error, Some(note_id), &Callback::noop());
                            if error.code() != Some("note_not_found") {
                                this.fail_with(session, &error, t("Couldn't move the note."));
                            }
                        }
                    }
                    done.emit(());
                });
            }
            Action::DeleteNote { note_id, done } => {
                spawn_local(async move {
                    match api::delete_note(&token, note_id).await {
                        Ok(()) => {
                            if live
                                .update(session, |state| state.store.board.forget(note_id))
                                .is_some()
                            {
                                done.emit(None);
                            }
                        }
                        Err(error) => this.board_refused(session, &error, Some(note_id), &done),
                    }
                });
            }
            Action::AnswerEvent {
                note_id,
                answer,
                done,
            } => {
                spawn_local(async move {
                    match api::answer_event(&token, note_id, answer.as_deref()).await {
                        Ok(note) => {
                            live.update(session, |state| state.store.board.apply(note));
                        }
                        Err(error) => {
                            this.board_refused(session, &error, Some(note_id), &Callback::noop());
                            this.fail_with(session, &error, t("Couldn't send your answer."));
                        }
                    }
                    done.emit(());
                });
            }
            Action::TickTask {
                note_id,
                item_id,
                done_now,
                done,
            } => {
                spawn_local(async move {
                    match api::tick_task(&token, note_id, item_id, done_now).await {
                        Ok(note) => {
                            live.update(session, |state| state.store.board.apply(note));
                        }
                        Err(error) => {
                            this.board_refused(session, &error, Some(note_id), &Callback::noop());
                            this.fail_with(session, &error, t("Couldn't tick that off."));
                        }
                    }
                    done.emit(());
                });
            }
            Action::DrawBackdrop { note_id, done } => {
                // NEVER ROUND THE CONSENT QUESTION: the title is the
                // author's words going to the model, so a backdrop is asked
                // about exactly as a `/draw` is. The sheet asks instead of
                // emitting this (views/board.rs); this is the same test made
                // again where the request is, so that no other door can be
                // opened onto the model.
                let ask = live.read(|state| {
                    assistant_consent::is_required_for_backdrop(
                        state
                            .store
                            .assistant
                            .as_ref()
                            .and_then(|assistant| assistant.processor.as_deref()),
                        state.store.assistant_consent_at().is_some(),
                    )
                });
                if ask {
                    done.emit(Backdrop::AskConsent);
                    return;
                }
                // No deadline of this client's own, on purpose: this is the
                // one request that waits on the model — up to three calls
                // in a row — and the protocol's floor for it is 90 s, which
                // is the proxy's own read timeout; the ordinary calls carry
                // none either (api.rs). A connection that closes first draws
                // nothing (docs/protocol.md, "Board").
                spawn_local(async move {
                    let outcome = match api::draw_backdrop(&token, note_id).await {
                        Ok(note) => {
                            live.update(session, |state| state.store.board.apply(note));
                            Backdrop::Settled
                        }
                        // The server's word over this tab's copy — consent
                        // withdrawn on another device since the last `/me`:
                        // nothing was sent, and the answer is the consent
                        // screen, not a failure (docs/protocol.md,
                        // "Consenting to the assistant").
                        Err(error) if asks_for_consent(&error) => {
                            live.update(session, |state| state.store.set_assistant_consent(None));
                            Backdrop::AskConsent
                        }
                        Err(error) => {
                            this.board_refused(session, &error, Some(note_id), &Callback::noop());
                            if error != ApiError::Unauthorized {
                                let text = backdrop_failure(&error);
                                live.update(session, |state| state.failure = Some(text));
                            }
                            Backdrop::Settled
                        }
                    };
                    done.emit(outcome);
                });
            }
            Action::ShowTranscript {
                chat_id,
                message_id,
                attachment,
                ask_consent,
            } => {
                let attachment_id = attachment.id;
                // NEVER ROUND THE CONSENT QUESTION: the asker is the one
                // sending a recording's sound to the provider, so the
                // asker's own consent is needed, and it is asked HERE, where
                // the request is, so that no door onto the provider can be
                // opened round it. Asked, nothing is held: the member presses
                // again once it is answered, as for a backdrop.
                let ask = live.read(|state| {
                    transcript::asks_for_consent(
                        state
                            .store
                            .assistant
                            .as_ref()
                            .and_then(|assistant| assistant.processor.as_deref()),
                        state.store.assistant_consent_at().is_some(),
                    )
                });
                if ask {
                    ask_consent.emit(());
                    return;
                }
                // Where the sound comes from: the server's stored copy, or
                // this device — decided once, here, by the same rule that
                // drew the button.
                let max_bytes = live.read(|state| {
                    transcript::Server {
                        transcribe: true,
                        max_bytes: state
                            .store
                            .assistant
                            .as_ref()
                            .and_then(|assistant| assistant.transcribe_max_bytes),
                        processor: None,
                    }
                    .max_bytes()
                });
                let Some(source) = transcript::source(
                    &transcript::Recording {
                        id: attachment.id,
                        kind: &attachment.kind,
                        mime: attachment.mime.as_deref(),
                        size: attachment.size,
                    },
                    max_bytes,
                ) else {
                    return;
                };
                // Held already — shown again, nobody asked — or already
                // on its way.
                if live.now(|state| state.store.transcripts.ask(attachment_id))
                    != transcript::Ask::Send
                {
                    return;
                }
                spawn_local(async move {
                    let recording = transcript::Recording {
                        id: attachment.id,
                        kind: &attachment.kind,
                        mime: attachment.mime.as_deref(),
                        size: attachment.size,
                    };
                    let answer = match source {
                        transcript::Source::Stored => {
                            match api::transcript(&token, chat_id, message_id, attachment_id).await
                            {
                                // The server's own reading of its copy
                                // disagrees with the metadata: this
                                // device's sound instead, as every client
                                // does.
                                Err(error)
                                    if transcript::falls_back_to_device(
                                        error.code(),
                                        &recording,
                                    ) =>
                                {
                                    this.ask_with_sound(
                                        &token,
                                        chat_id,
                                        message_id,
                                        &attachment,
                                        max_bytes,
                                    )
                                    .await
                                }
                                answer => Ok(answer),
                            }
                        }
                        transcript::Source::Device => {
                            this.ask_with_sound(&token, chat_id, message_id, &attachment, max_bytes)
                                .await
                        }
                    };
                    let answer = match answer {
                        Ok(answer) => answer,
                        // Nothing was sent: the device could not make the
                        // sound, or could not fetch the file to make it
                        // from.
                        Err(failure) => {
                            live.update(session, |state| {
                                state.store.transcripts.failed(attachment_id, failure)
                            });
                            return;
                        }
                    };
                    match answer {
                        // From stored bytes or supplied sound alike, kept
                        // on this device for the life of the tab and never
                        // in its storage — and an answer from supplied
                        // sound is kept NOWHERE else: the server hands it
                        // to nobody.
                        Ok(said) => {
                            live.update(session, |state| {
                                state.store.transcripts.answered(attachment_id, said)
                            });
                        }
                        Err(ApiError::Unauthorized) => {
                            live.update(session, |state| {
                                state.store.transcripts.forget(attachment_id)
                            });
                            expiry(&live, session, &this.sign_out)();
                        }
                        Err(error) => match transcript::after_refusal(error.code()) {
                            // The server's word over this tab's copy —
                            // consent withdrawn on another device: nothing
                            // was sent, and the answer is the consent
                            // screen, not a failure.
                            transcript::Next::AskConsent => {
                                let asked = live.update(session, |state| {
                                    state.store.transcripts.forget(attachment_id);
                                    state.store.set_assistant_consent(None);
                                });
                                if asked.is_some() {
                                    ask_consent.emit(());
                                }
                            }
                            transcript::Next::Fail(failure) => {
                                live.update(session, |state| {
                                    state.store.transcripts.failed(attachment_id, failure)
                                });
                            }
                        },
                    }
                });
            }
            Action::HideTranscript { attachment_id } => {
                live.now(|state| state.store.transcripts.hide(attachment_id));
            }
            Action::RevealNote { note_id } => {
                live.now(|state| {
                    state.store.board.revealed.insert(note_id);
                });
            }
            Action::PinPhoto { photo, at } => {
                if live.read(|state| state.store.board.pinning) {
                    live.now(|state| {
                        state.failure = Some(crate::views::board::still_pinning().to_string())
                    });
                    return;
                }
                live.now(|state| state.store.board.pinning = true);
                spawn_local(async move {
                    let pinned = this.pin_photo(session, &token, photo, at).await;
                    live.update(session, |state| {
                        state.store.board.pinning = false;
                        if let Err(reason) = pinned {
                            state.failure =
                                Some(format!("{} {reason}", t("Couldn't pin that photo.")));
                        }
                    });
                });
            }
            Action::SendSticker {
                chat_id,
                item_id,
                reply_to_message_id,
            } => {
                // NEVER ROUND THE CONSENT QUESTION: in the assistant's chat
                // a sticker is a picture shown to the model, and it is
                // queued only once this member has agreed — the panel asks
                // instead of emitting this (views/conversation.rs), and
                // this is the same test made again where the send is, so
                // that no other door can be opened onto the outbox.
                let gate = live.read(|state| {
                    crate::pack::send_gate(
                        state
                            .store
                            .item(chat_id)
                            .is_some_and(|item| item.chat.is_ai()),
                        state.store.assistant.as_ref(),
                        state.store.assistant_consent_at().is_some(),
                    )
                });
                if gate != crate::pack::Gate::Send {
                    return;
                }
                let Some(picture) = live.read(|state| {
                    state
                        .store
                        .pack
                        .items
                        .get(&item_id)
                        .and_then(|item| item.attachment.clone())
                }) else {
                    return;
                };
                // The pack's own bytes, as this tab holds them: at once
                // when the panel has drawn the sticker — which is what
                // makes this work with no network at all — and fetched
                // first otherwise. The ORIGINAL either way.
                let media = self.media.clone();
                self.media.load(
                    picture.id,
                    Variant::Sticker,
                    Callback::from(move |url: Option<String>| {
                        let bytes = url.and_then(|_| media.held(picture.id, Variant::Sticker));
                        match bytes {
                            Some(bytes) => this.queue_sticker(
                                session,
                                chat_id,
                                item_id,
                                &picture,
                                bytes,
                                reply_to_message_id,
                            ),
                            None => {
                                this.live.update(session, |state| {
                                    state.failure =
                                        Some(t("Couldn't send that sticker.").to_string())
                                });
                            }
                        }
                    }),
                );
            }
            Action::AddSticker { file, label, done } => {
                if live.read(|state| state.store.pack.adding) {
                    done.emit(Some(still_adding().to_string()));
                    return;
                }
                live.now(|state| state.store.pack.adding = true);
                spawn_local(async move {
                    let added = this
                        .add_sticker(session, &token, file.into(), Some(label))
                        .await;
                    if live
                        .update(session, |state| state.store.pack.adding = false)
                        .is_some()
                    {
                        done.emit(added.err());
                    }
                });
            }
            Action::KeepSticker {
                attachment,
                label,
                done,
            } => {
                if live.read(|state| state.store.pack.adding) {
                    done.emit(Some(still_adding().to_string()));
                    return;
                }
                live.now(|state| state.store.pack.adding = true);
                spawn_local(async move {
                    // The message's own bytes — this tab drew the sticker
                    // from them a moment ago — uploaded again, unprepared.
                    let kept = match this.sticker_bytes(attachment.id).await {
                        Some(bytes) => this.add_sticker(session, &token, bytes, Some(label)).await,
                        None => Err(t("Check your connection and try again.").to_string()),
                    };
                    if live
                        .update(session, |state| {
                            state.store.pack.adding = false;
                            match kept {
                                Ok(true) => {
                                    state.notice =
                                        Some(t("Added to the family's stickers.").to_string())
                                }
                                // `200`: it was there all along — the guess
                                // that it was not was wrong, and the server
                                // said so without making a second one.
                                Ok(false) => {
                                    state.notice =
                                        Some(t("Already in the family's stickers.").to_string())
                                }
                                Err(_) => {}
                            }
                        })
                        .is_some()
                    {
                        done.emit(kept.err());
                    }
                });
            }
            Action::RemoveSticker { item_id, done } => {
                spawn_local(async move {
                    let removed = api::remove_pack_item(&token, item_id).await;
                    // `pack_item_not_found` IS "ALREADY GONE": somebody else
                    // removed it first, or this device's own earlier
                    // request did and its answer was lost. What was asked
                    // for is so, and there is nothing to tell anybody — it
                    // leaves this panel exactly as an answered removal
                    // does, with no error.
                    let gone = matches!(&removed, Err(error) if error.code() == Some("pack_item_not_found"));
                    match removed {
                        Err(ApiError::Unauthorized) => {
                            expiry(&this.live, session, &this.sign_out)()
                        }
                        Err(error) if !gone => {
                            if live.is_live(session) {
                                done.emit(Some(pack_failure(&error)));
                            }
                        }
                        // Gone for good, whatever arrives late: this
                        // device's own removal is as final as a tombstone.
                        _ => {
                            if live
                                .update(session, |state| state.store.pack.forget(item_id))
                                .is_some()
                            {
                                done.emit(None);
                            }
                        }
                    }
                });
            }
            Action::OpenSticker(attachment) => {
                live.now(|state| state.sticker_open = Some(attachment));
            }
            Action::CloseSticker => {
                live.now(|state| state.sticker_open = None);
            }
        }
    }

    /// A sticker into the outbox: ONE attachment, no words, the flag — and
    /// the bytes exactly as the pack holds them. Nothing between here and
    /// the upload prepares a photograph, so nothing scales, re-encodes or
    /// strips them (docs/protocol.md, "A sticker is NOT prepared before
    /// upload"). From here it is a message like any other: the bubble draws
    /// now, the row is retried on a bad network, and a refusal fails it.
    fn queue_sticker(
        &self,
        session: u64,
        chat_id: i64,
        item_id: i64,
        picture: &Attachment,
        bytes: web_sys::Blob,
        reply_to_message_id: Option<i64>,
    ) {
        let copy = Prepared {
            kind: "photo".into(),
            mime: picture.mime.clone().unwrap_or_else(|| bytes.type_()),
            size: bytes.size() as i64,
            width: picture.width,
            height: picture.height,
            file: Some(bytes),
            // No preview: a preview is a JPEG.
            preview: None,
            source_attachment_id: Some(picture.id),
            ..Prepared::default()
        };
        let queued = self.live.update(session, |state| {
            state.store.queue_send(
                chat_id,
                uuid::Uuid::new_v4().to_string(),
                Draft {
                    reply_to_message_id,
                    attachments: vec![copy],
                    sticker: true,
                    ..Draft::default()
                },
            );
            // Which sticker was used is this device's to remember, and
            // nobody else's to know: kept here, never sent.
            let recents = crate::session::use_pack_item(state.store.my_user_id, item_id);
            state.store.pack.recents = recents;
        });
        if queued.is_some() {
            wake(&self.channels, Wake::Queued);
        }
    }

    /// A sticker's bytes, from this tab's cache or fetched — the original,
    /// never a preview.
    async fn sticker_bytes(&self, attachment_id: i64) -> Option<web_sys::Blob> {
        let (heard, hearing) = futures::channel::oneshot::channel::<bool>();
        let heard = std::cell::RefCell::new(Some(heard));
        self.media.load(
            attachment_id,
            Variant::Sticker,
            Callback::from(move |url: Option<String>| {
                if let Some(heard) = heard.borrow_mut().take() {
                    let _ = heard.send(url.is_some());
                }
            }),
        );
        if !hearing.await.unwrap_or(false) {
            return None;
        }
        self.media.held(attachment_id, Variant::Sticker)
    }

    /// Make a sticker of `picture`, upload it, claim it — in that order,
    /// the way a photo is pinned: the pack is a third way an upload is
    /// claimed (docs/protocol.md, "The pack"). Refused HERE, beside the
    /// picker, for what this tab can already see: a full pack, a picture
    /// too big, a label too long. Answers whether the pack gained an item —
    /// false when it already held these bytes, which is not an error.
    async fn add_sticker(
        &self,
        session: u64,
        token: &str,
        picture: web_sys::Blob,
        label: Option<String>,
    ) -> Result<bool, String> {
        let label = fc_text::pack::label(label.as_deref().unwrap_or_default())
            .map_err(|_| t("A description is at most 64 characters.").to_string())?;
        let (limits, room) = self
            .live
            .read(|state| (state.store.pack.limits, state.store.pack.has_room()));
        // No limits is a server from before packs: nothing offers this
        // there, and nothing is sent if something does.
        let limits = limits.ok_or_else(|| t("That didn't work. Try again.").to_string())?;
        if !room {
            return Err(pack_full().to_string());
        }
        let sticker = crate::prep::sticker(&picture, limits.bytes.max(0) as u64)
            .await
            .map_err(|refusal| sticker_refusal(refusal).to_string())?;
        let item = crate::staged::OutgoingItem::new(&sticker, -1);
        // `attachment_expired` ON THE CLAIM: the unclaimed sweep took the
        // upload before the claim reached it (docs/protocol.md, "The pack":
        // "the client uploads again"). The bytes are still in hand, so
        // they go up again and are claimed again — ONCE, by itself, with
        // nobody asked to do anything. Only a second refusal is said.
        let mut may_upload_again = true;
        let claimed = loop {
            let uploaded = api::upload_attachment(token, &item, sticker.file.as_ref())
                .await
                .map_err(|error| self.pack_refusal(session, &error))?;
            // No preview goes up, for a pack item as for a sticker message.
            match api::add_pack_item(token, uploaded.id, label.as_deref()).await {
                Ok(claimed) => break claimed,
                Err(error) if may_upload_again && error.code() == Some("attachment_expired") => {
                    may_upload_again = false;
                }
                Err(error) => return Err(self.pack_refusal(session, &error)),
            }
        };
        // Drawn from the bytes this tab just sent rather than fetched back
        // — under the id the pack HOLDS, which on a `200` is not the id
        // this upload was given.
        if let (Some(held), Some(file)) = (claimed.item.attachment.as_ref(), sticker.file) {
            if self.live.is_live(session) {
                self.media.seed(held.id, Variant::Sticker, file);
            }
        }
        let added = claimed.added;
        // Under the per-item guard, moving no cursor: the frame for this
        // add is what moves it, for the reason the board gives.
        self.live
            .update(session, |state| state.store.pack.apply(claimed.item));
        Ok(added)
    }

    /// The words for a refused step of adding — after signing out, if the
    /// refusal was the session ending.
    fn pack_refusal(&self, session: u64, error: &ApiError) -> String {
        if *error == ApiError::Unauthorized {
            expiry(&self.live, session, &self.sign_out)();
        }
        pack_failure(error)
    }

    /// A refused account or family change, handed to the dialog that asked
    /// — or, for a 401, the sign-out every other ended session gets.
    fn said(&self, session: u64, error: ApiError, done: &Done) {
        if error == ApiError::Unauthorized {
            expiry(&self.live, session, &self.sign_out)();
        } else if self.live.is_live(session) {
            done.emit(Some(error));
        }
    }

    /// `/me` again, and — when it says the account is in a family — the
    /// roster, the chats and the board with it: the whole resync, as a
    /// connection would run it.
    async fn refresh_account(&self, session: u64, token: &str) {
        let expired = expiry(&self.live, session, &self.sign_out);
        let link = self
            .live
            .read(|state| state.connected.then_some(state.link));
        sync::resync(&self.live, session, token, &expired, link).await;
    }

    async fn read_family_admin(&self, session: u64, token: &str) {
        sync::read_family_admin(&self.live, session, token).await;
    }

    /// A refusal to a change on the board: a 401 signs out, a note that is
    /// no longer there comes off this wall too, and `done` hears the words.
    fn board_refused(
        &self,
        session: u64,
        error: &ApiError,
        note_id: Option<i64>,
        done: &Callback<Option<String>>,
    ) {
        if *error == ApiError::Unauthorized {
            expiry(&self.live, session, &self.sign_out)();
            return;
        }
        if let (Some(note_id), Some("note_not_found")) = (note_id, error.code()) {
            self.live
                .update(session, |state| state.store.board.forget(note_id));
        }
        if self.live.is_live(session) {
            done.emit(Some(board_failure(error)));
        }
    }

    /// A failure for the bar, led by what did not happen.
    fn fail_with(&self, session: u64, error: &ApiError, lead: &str) {
        if *error == ApiError::Unauthorized {
            return;
        }
        let text = format!("{lead} {}", board_failure(error));
        self.live
            .update(session, |state| state.failure = Some(text));
    }

    /// Upload, preview, pin — in that order, because the note may not exist
    /// before the picture does: the server claims the upload inside the
    /// transaction that writes the note (docs/protocol.md, "Board").
    async fn pin_photo(
        &self,
        session: u64,
        token: &str,
        photo: Prepared,
        at: (f64, f64),
    ) -> Result<(), String> {
        let item = crate::staged::OutgoingItem::new(&photo, -1);
        let attachment = api::upload_attachment(token, &item, photo.file.as_ref())
            .await
            .map_err(|error| self.refusal(session, &error))?;
        if let Some(preview) = photo.preview.clone() {
            // The preview the sticker draws. Best effort, but not best effort
            // once: tried twice more beside the pin, as a message's is.
            if api::upload_preview(token, attachment.id, &preview)
                .await
                .is_err()
            {
                let token = token.to_string();
                let preview = preview.clone();
                let id = attachment.id;
                spawn_local(async move {
                    for wait in [2_000, 8_000] {
                        gloo_timers::future::TimeoutFuture::new(wait).await;
                        if api::upload_preview(&token, id, &preview).await.is_ok() {
                            break;
                        }
                    }
                });
            }
            // Only into the cache of the session that sent it: signed out
            // mid-upload, these are nobody's to draw.
            if self.live.is_live(session) {
                self.media.seed(attachment.id, Variant::Preview, preview);
            }
        }
        if let Some(file) = photo.file.clone().filter(|_| self.live.is_live(session)) {
            self.media.seed(attachment.id, Variant::Original, file);
        }
        let note = NewNote {
            text: String::new(),
            color: random_color(),
            size: fc_text::board::Size::Medium.name().to_string(),
            font: fc_text::board::Font::Plain.name().to_string(),
            x: at.0,
            y: at.1,
            kind: Some(fc_text::board::Kind::Photo.name().to_string()),
            attachment_id: Some(attachment.id),
            ..NewNote::default()
        };
        let note = api::create_note(token, &note)
            .await
            .map_err(|error| self.refusal(session, &error))?;
        self.live
            .update(session, |state| state.store.board.apply(note));
        Ok(())
    }

    /// The words for a refused step of pinning — after signing out, if the
    /// refusal was the session ending.
    fn refusal(&self, session: u64, error: &ApiError) -> String {
        if *error == ApiError::Unauthorized {
            expiry(&self.live, session, &self.sign_out)();
        }
        board_failure(error)
    }

    /// Put my reaction on (or take it off) at once, and answer with what
    /// was there before — to put back if the server says no.
    fn set_my_reaction(
        &self,
        session: u64,
        chat_id: i64,
        message_id: i64,
        emoji: Option<String>,
    ) -> Option<(Option<Vec<Reaction>>, Option<i64>)> {
        self.live
            .update(session, |state| {
                let me = state.store.my_user_id;
                let held =
                    state.store.threads.get_mut(&chat_id).and_then(|thread| {
                        thread.messages.iter_mut().find(|m| m.id == message_id)
                    })?;
                let before = (held.reactions.clone(), held.reaction_seq);
                let mut reactions = held.reactions.clone().unwrap_or_default();
                match emoji {
                    Some(emoji) => match reactions.iter_mut().find(|r| r.user_id == me) {
                        Some(mine) => mine.emoji = emoji,
                        None => reactions.push(Reaction { user_id: me, emoji }),
                    },
                    None => reactions.retain(|r| r.user_id != me),
                }
                held.reactions = Some(reactions);
                Some(before)
            })
            .flatten()
    }

    /// Undo an optimistic reaction — unless something newer has arrived
    /// meanwhile, which is then the truth and must stand.
    fn roll_back_reactions(
        &self,
        session: u64,
        chat_id: i64,
        message_id: i64,
        before: Option<(Option<Vec<Reaction>>, Option<i64>)>,
    ) {
        let Some((reactions, seq)) = before else {
            return;
        };
        self.live.update(session, |state| {
            if let Some(held) = state
                .store
                .threads
                .get_mut(&chat_id)
                .and_then(|thread| thread.messages.iter_mut().find(|m| m.id == message_id))
            {
                if held.reaction_seq == seq {
                    held.reactions = reactions;
                }
            }
        });
    }

    /// A vote's, a retraction's or a close's answer: the poll's whole state,
    /// applied under the guard and moving NO cursor. The open-polls list is
    /// read again afterwards, so a poll just answered or closed leaves it.
    fn apply_poll_answer(
        &self,
        session: u64,
        chat_id: i64,
        answer: Result<api::PollState, ApiError>,
        reread_open: bool,
    ) {
        match answer {
            Ok(state) => {
                self.live.update(session, |app| {
                    app.store.apply_poll(chat_id, state.message_id, state.poll)
                });
                let open = self
                    .live
                    .read(|app| app.store.open_polls.as_ref().map(|open| open.chat_id));
                if reread_open && open == Some(chat_id) {
                    if let Some((session, token)) = self.session_and_token() {
                        let this = self.clone();
                        spawn_local(
                            async move { this.read_open_polls(session, &token, chat_id).await },
                        );
                    }
                }
            }
            Err(error) => self.fail(session, &error),
        }
    }

    async fn read_open_polls(&self, session: u64, token: &str, chat_id: i64) {
        match api::open_polls(token, chat_id).await {
            Ok(messages) => {
                self.live.update(session, |state| {
                    state.store.open_polls = Some(OpenPolls { chat_id, messages });
                });
            }
            Err(error) => self.fail(session, &error),
        }
    }

    /// The chain, page by page — root first, then every reply, `after_id`
    /// looped until a short page. Folded into the SURFACE, and only while
    /// the surface is still this opening's (`generation`): a read in flight
    /// when the reader opened another thread belongs to nobody. The chat's
    /// own window takes these copies only for rows it already holds — their
    /// recomputed reply counts and edits — so no paging cursor moves.
    async fn read_thread(
        &self,
        session: u64,
        token: &str,
        chat_id: i64,
        message_id: i64,
        generation: u64,
    ) {
        let mut after: Option<i64> = None;
        loop {
            let page = match api::thread(token, chat_id, message_id, after, PAGE).await {
                Ok(page) => page,
                Err(error) => {
                    self.fail(session, &error);
                    return;
                }
            };
            let first_page = after.is_none();
            let short = (page.len() as u32) < PAGE;
            after = page.iter().map(|message| message.id).max().or(after);
            let still_open = self.live.update(session, |state| {
                state
                    .store
                    .apply_thread_page(chat_id, generation, first_page, page)
            });
            if still_open != Some(true) || short {
                return;
            }
        }
    }

    /// The throttle: at most one frame every TYPING_THROTTLE_MS per chat, the
    /// same 4 s the apps use — and only while the socket is up, because a
    /// typing frame is momentary and never saved for later.
    fn typing(&self, chat_id: i64) {
        if !self.live.read(|state| state.connected) {
            return;
        }
        let now = sync::now_ms();
        let mut last = self.last_typing.borrow_mut();
        if last
            .get(&chat_id)
            .is_some_and(|sent| now - sent < store::TYPING_THROTTLE_MS)
        {
            return;
        }
        last.insert(chat_id, now);
        if let Some(open) = self.channels.borrow().as_ref() {
            let _ = open.frames.unbounded_send(ClientFrame::Typing { chat_id });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    use wasm_bindgen_test::*;

    use crate::live::AppState;
    use crate::model::{Chat, ChatListItem, Message};
    use crate::store::Store;

    const ME: i64 = 7;
    const ANNA: i64 = 9;

    fn reaction(user_id: i64, emoji: &str) -> Reaction {
        Reaction {
            user_id,
            emoji: emoji.into(),
        }
    }

    fn actions() -> Actions {
        let mut message = Message {
            id: 100,
            chat_id: 42,
            sender_id: ANNA,
            body: "Dinner?".into(),
            created_at: "2026-09-10T10:00:00Z".into(),
            reactions: Some(vec![reaction(ANNA, "👍")]),
            reaction_seq: Some(5),
            ..Message::default()
        };
        message.client_msg_id = None;
        let mut store = Store {
            my_user_id: ME,
            chats: vec![ChatListItem {
                chat: Chat {
                    id: 42,
                    kind: "family".into(),
                    title: None,
                    peer_user_id: None,
                },
                last_message: None,
                unread_count: 0,
                last_read_message_id: 0,
                max_reaction_seq: None,
                max_edit_seq: None,
                max_poll_seq: None,
                mentioned: false,
            }],
            ..Store::default()
        };
        store.apply_history(42, vec![message], false);
        let live = Live::new(
            AppState {
                token: Some("t".into()),
                store,
                ..AppState::default()
            },
            Rc::new(|| {}),
        );
        Actions {
            live: live.clone(),
            channels: Rc::new(RefCell::new(None)),
            sign_out: Callback::noop(),
            last_typing: Rc::new(RefCell::new(HashMap::new())),
            media: MediaLoader::new(live.clone()),
            calls: crate::calls::Calls::new(live, crate::calls::Wire::new()),
        }
    }

    fn reactions_held(actions: &Actions) -> Vec<Reaction> {
        actions.live.read(|state| {
            state.store.threads[&42]
                .get(100)
                .map(|message| message.reactions().to_vec())
                .unwrap_or_default()
        })
    }

    /// Optimistic: mine is on at once, and a refusal puts back exactly
    /// what was there.
    #[wasm_bindgen_test]
    fn a_refused_reaction_is_rolled_back() {
        let actions = actions();
        let session = actions.live.session();
        let before = actions.set_my_reaction(session, 42, 100, Some("❤️".into()));
        assert_eq!(
            reactions_held(&actions),
            vec![reaction(ANNA, "👍"), reaction(ME, "❤️")]
        );
        actions.roll_back_reactions(session, 42, 100, before);
        assert_eq!(reactions_held(&actions), vec![reaction(ANNA, "👍")]);
    }

    /// …unless something NEWER arrived meanwhile: that is the truth now, and
    /// putting the old list back would undo somebody else's reaction.
    #[wasm_bindgen_test]
    fn a_rollback_never_undoes_a_newer_state() {
        let actions = actions();
        let session = actions.live.session();
        let before = actions.set_my_reaction(session, 42, 100, Some("❤️".into()));
        let newer = vec![reaction(ANNA, "👍"), reaction(11, "😂")];
        actions.live.update(session, |state| {
            state.store.apply_reactions(42, 100, 6, newer.clone())
        });
        actions.roll_back_reactions(session, 42, 100, before);
        assert_eq!(reactions_held(&actions), newer);
    }

    /// The words for a refused change to the board — `board_full` said out
    /// loud, never swallowed — and the server's message when nothing fits.
    #[wasm_bindgen_test]
    fn a_refused_board_change_is_said_in_words() {
        let refusal = |code: &str| ApiError::Server {
            code: code.into(),
            message: "for developers".into(),
        };
        assert!(board_failure(&refusal("board_full")).contains("The board is full"));
        assert!(board_failure(&refusal("not_note_author")).contains("Only the person who wrote"));
        assert_eq!(board_failure(&refusal("validation")), "for developers");
        assert!(fc_text::board::COLORS.contains(&random_color().as_str()));
    }

    /// A backdrop the provider's filter refused says what a refused answer
    /// says, and nothing of the server's English; any other failure is still
    /// led by what did not happen.
    #[wasm_bindgen_test]
    fn a_refused_backdrop_says_the_refusal_sentence() {
        let refusal = |code: &str| ApiError::Server {
            code: code.into(),
            message: "for developers".into(),
        };
        assert_eq!(
            backdrop_failure(&refusal("picture_refused")),
            "The assistant's provider refused that. Try putting it another way."
        );
        assert_eq!(
            backdrop_failure(&refusal("internal")),
            "Couldn't draw that. for developers"
        );
        assert_eq!(
            backdrop_failure(&refusal("note_not_found")),
            "Couldn't draw that. That note has been taken down."
        );
        assert_eq!(
            backdrop_failure(&ApiError::Answered { status: 502 }),
            "Couldn't draw that. The server answered 502."
        );
        assert!(asks_for_consent(&refusal("assistant_consent_required")));
        assert!(!asks_for_consent(&refusal("picture_refused")));
        assert!(!asks_for_consent(&refusal("not_note_author")));
        assert!(!asks_for_consent(&ApiError::Answered { status: 403 }));
    }

    /// A BACKDROP NEVER GOES ROUND THE CONSENT QUESTION (docs/protocol.md,
    /// "Consenting to the assistant", amended 2026-09-30): until the author
    /// has agreed nothing is asked of the server and the answer is the
    /// consent screen; agreed, it is asked; and a server that answers
    /// `assistant_consent_required` all the same — consent withdrawn on
    /// another device — is taken at its word: no failure on the bar, this
    /// tab's copy of the answer cleared, and the consent screen again.
    #[wasm_bindgen_test]
    async fn a_backdrop_waits_for_consent_and_takes_the_servers_word_for_it() {
        use crate::fake_server::{Answer, FakeServer};
        const ROUTE: &str = "/families/mine/board/notes/6/backdrop";
        let refuse_with = Rc::new(RefCell::new("assistant_consent_required"));
        let server = {
            let refuse_with = refuse_with.clone();
            FakeServer::answering(move |asked| {
                if asked.line() == format!("POST {ROUTE}") {
                    let code = *refuse_with.borrow();
                    let status = if code == "picture_refused" { 400 } else { 403 };
                    Answer::refusal(status, code)
                } else {
                    Answer::refusal(404, "not_found")
                }
            })
        };
        let actions = actions();
        actions.live.now(|state| {
            state.store.account = Some(crate::model::Me::default());
            state.store.assistant = Some(crate::model::Assistant {
                user_id: 2,
                display_name: "Assistant".into(),
                mention: Some("@ai".into()),
                draw: Some("/draw".into()),
                vision: false,
                images: true,
                processor: Some("Microsoft — Azure OpenAI".into()),
                transcribe: false,
                transcribe_max_bytes: None,
                lookups: Vec::new(),
                greeting_weather: false,
            });
        });
        let draw = |actions: &Actions| {
            let heard = Rc::new(RefCell::new(Vec::new()));
            let sink = heard.clone();
            actions.handle(Action::DrawBackdrop {
                note_id: 6,
                done: Callback::from(move |outcome: Backdrop| sink.borrow_mut().push(outcome)),
            });
            heard
        };
        let settle = |heard: Rc<RefCell<Vec<Backdrop>>>| async move {
            for _ in 0..40 {
                if !heard.borrow().is_empty() {
                    break;
                }
                gloo_timers::future::TimeoutFuture::new(25).await;
            }
            let heard = heard.borrow().clone();
            heard
        };
        let agreed = |actions: &Actions| {
            actions
                .live
                .read(|state| state.store.assistant_consent_at().is_some())
        };

        // Not agreed: the question, at once, and nothing sent.
        let heard = draw(&actions);
        assert_eq!(
            *heard.borrow(),
            vec![Backdrop::AskConsent],
            "answered at once"
        );
        gloo_timers::future::TimeoutFuture::new(50).await;
        assert!(
            server.lines(&[ROUTE]).is_empty(),
            "nothing reached the server"
        );

        // Agreed here, withdrawn elsewhere: the server's word wins.
        actions.live.now(|state| {
            state
                .store
                .set_assistant_consent(Some("2026-09-30T10:00:00Z".into()))
        });
        assert_eq!(settle(draw(&actions)).await, vec![Backdrop::AskConsent]);
        assert_eq!(server.lines(&[ROUTE]), vec![format!("POST {ROUTE}")]);
        assert!(!agreed(&actions), "this tab no longer believes it agreed");
        assert!(
            actions.live.read(|state| state.failure.is_none()),
            "a question, not a failure"
        );
        // …so pressing again asks again, without troubling the server.
        assert_eq!(*draw(&actions).borrow(), vec![Backdrop::AskConsent]);
        gloo_timers::future::TimeoutFuture::new(50).await;
        assert_eq!(server.lines(&[ROUTE]).len(), 1);

        // Any other refusal is the failure it always was.
        actions.live.now(|state| {
            state
                .store
                .set_assistant_consent(Some("2026-09-30T10:00:00Z".into()))
        });
        *refuse_with.borrow_mut() = "picture_refused";
        assert_eq!(settle(draw(&actions)).await, vec![Backdrop::Settled]);
        assert_eq!(server.lines(&[ROUTE]).len(), 2);
        assert!(agreed(&actions), "a refused title withdraws nothing");
        assert_eq!(
            actions.live.read(|state| state.failure.clone()).as_deref(),
            Some("The assistant's provider refused that. Try putting it another way.")
        );
    }

    /// THE TRANSCRIPT ACTION, end to end against a stand-in server: not
    /// agreed, the consent screen and nothing sent; agreed, one request and
    /// the text held for this attachment; hidden and shown again with no
    /// second request; `assistant_consent_required` taken at its word (the
    /// tab's consent cleared, nothing left behind, the screen again); and
    /// each failure kept as what it means — a transient one asked again.
    #[wasm_bindgen_test]
    async fn a_transcript_is_asked_once_and_kept() {
        use crate::fake_server::{Answer, FakeServer};
        use fc_text::transcript::{Failure, State, Transcript};
        const ROUTE: &str = "/chats/42/messages/100/attachments/34/transcript";
        let answer = Rc::new(RefCell::new(Answer::Json(
            200,
            serde_json::json!({"transcript": {"text": "Dinner at seven", "language": "en"}}),
        )));
        let server = {
            let answer = answer.clone();
            FakeServer::answering(move |asked| {
                if asked.path == ROUTE {
                    std::mem::replace(&mut *answer.borrow_mut(), Answer::Nothing)
                } else {
                    Answer::refusal(404, "not_found")
                }
            })
        };
        let actions = actions();
        actions.live.now(|state| {
            state.store.account = Some(crate::model::Me::default());
            state.store.assistant = Some(crate::model::Assistant {
                user_id: 2,
                display_name: "Assistant".into(),
                mention: Some("@ai".into()),
                draw: None,
                vision: false,
                images: false,
                processor: Some("Microsoft — Azure OpenAI".into()),
                transcribe: true,
                transcribe_max_bytes: Some(26_214_400),
                lookups: Vec::new(),
                greeting_weather: false,
            });
        });
        let asked_consent = Rc::new(RefCell::new(0));
        let show = |actions: &Actions| {
            let asked_consent = asked_consent.clone();
            actions.handle(Action::ShowTranscript {
                chat_id: 42,
                message_id: 100,
                attachment: Attachment {
                    id: 34,
                    kind: "audio".into(),
                    mime: Some("audio/mp4".into()),
                    size: Some(48_000),
                    duration_ms: Some(6_000),
                    ..Attachment::default()
                },
                ask_consent: Callback::from(move |()| *asked_consent.borrow_mut() += 1),
            });
        };
        let held = |actions: &Actions| {
            actions
                .live
                .read(|state| state.store.transcripts.get(34).cloned())
        };
        let settle = |actions: &Actions| {
            let actions = actions.clone();
            async move {
                for _ in 0..40 {
                    if held(&actions) != Some(State::Asking) {
                        break;
                    }
                    gloo_timers::future::TimeoutFuture::new(25).await;
                }
            }
        };
        let agree = |actions: &Actions| {
            actions.live.now(|state| {
                state
                    .store
                    .set_assistant_consent(Some("2026-10-02T10:00:00Z".into()))
            })
        };

        // Not agreed: the screen, and nothing sent.
        show(&actions);
        assert_eq!(*asked_consent.borrow(), 1);
        assert_eq!(held(&actions), None);
        gloo_timers::future::TimeoutFuture::new(50).await;
        assert!(server.lines(&[ROUTE]).is_empty());

        // Agreed: asked once, and held.
        agree(&actions);
        show(&actions);
        assert_eq!(held(&actions), Some(State::Asking));
        // A second press while it is out sends nothing more.
        show(&actions);
        settle(&actions).await;
        let said = Transcript {
            text: "Dinner at seven".into(),
            language: Some("en".into()),
        };
        assert_eq!(held(&actions), Some(State::Shown(said.clone())));
        assert_eq!(server.lines(&[ROUTE]), vec![format!("POST {ROUTE}")]);

        // Hidden, and shown again — by the device, not the server.
        actions.handle(Action::HideTranscript { attachment_id: 34 });
        assert_eq!(held(&actions), Some(State::Hidden(said.clone())));
        show(&actions);
        assert_eq!(held(&actions), Some(State::Shown(said)));
        gloo_timers::future::TimeoutFuture::new(50).await;
        assert_eq!(server.lines(&[ROUTE]).len(), 1, "asked once");

        // The server's word on consent: nothing kept, the screen again.
        actions
            .live
            .now(|state| state.store.transcripts = Default::default());
        *answer.borrow_mut() = Answer::refusal(403, "assistant_consent_required");
        show(&actions);
        settle(&actions).await;
        for _ in 0..40 {
            if *asked_consent.borrow() == 2 {
                break;
            }
            gloo_timers::future::TimeoutFuture::new(25).await;
        }
        assert_eq!(*asked_consent.borrow(), 2);
        assert_eq!(held(&actions), None);
        assert!(actions
            .live
            .read(|state| state.store.assistant_consent_at().is_none()));
        assert!(
            actions.live.read(|state| state.failure.is_none()),
            "a question, not a failure"
        );

        // Each failure, kept as what it means. (`not_transcribable` of a
        // stored copy is not an answer but a turn to the device's own
        // sound: a_stored_copy_the_server_refuses_goes_on_with_the_devices_sound.)
        for (status, code, failure) in [
            (400, "transcript_refused", Failure::Refused),
            (403, "transcript_not_allowed", Failure::NotAvailable),
            (403, "transcripts_unavailable", Failure::NotAvailable),
            (500, "internal", Failure::TryAgain),
        ] {
            agree(&actions);
            actions
                .live
                .now(|state| state.store.transcripts = Default::default());
            *answer.borrow_mut() = Answer::refusal(status, code);
            show(&actions);
            settle(&actions).await;
            assert_eq!(held(&actions), Some(State::Failed(failure)), "{code}");
        }
        // …and a transient one is asked again on the next press.
        let before = server.lines(&[ROUTE]).len();
        *answer.borrow_mut() = Answer::Json(200, serde_json::json!({"transcript": {"text": ""}}));
        show(&actions);
        settle(&actions).await;
        assert_eq!(server.lines(&[ROUTE]).len(), before + 1);
        assert!(held(&actions)
            .as_ref()
            .and_then(State::transcript)
            .is_some_and(Transcript::is_silence));
        assert!(actions.live.read(|state| state.failure.is_none()));
    }

    /// THE TRANSCRIPT OF A VIDEO, end to end against a stand-in server: the
    /// file fetched through the media cache, its AAC taken out of it on
    /// this device, and sent as the multipart form's one `audio` part —
    /// never the stored-bytes shape — and the answer kept here, so a second
    /// press asks nobody. A recording its metadata already says is too long
    /// is told so before a byte is fetched; a film with no sound, and a
    /// file that could not be fetched, send nothing at all.
    #[wasm_bindgen_test]
    async fn a_videos_text_is_asked_with_its_own_sound_and_kept_here() {
        use crate::encode::testing::{film, whole, QUICKTIME};
        use crate::fake_server::{Answer, FakeServer};
        use fc_text::transcript::{Failure, State, Transcript};
        let silent = whole(&film(160, 120, 30, 1, 200_000, None).await).await;
        let server = FakeServer::answering(move |asked| match asked.path.as_str() {
            "/attachments/36" => Answer::Bytes(200, QUICKTIME.to_vec(), "video/quicktime"),
            "/attachments/38" => Answer::Bytes(200, silent.clone(), "video/mp4"),
            "/chats/42/messages/100/attachments/36/transcript" => Answer::Json(
                200,
                serde_json::json!({"transcript": {"text": "Look at the snow", "language": "en"}}),
            ),
            _ => Answer::refusal(404, "not_found"),
        });
        let actions = actions();
        actions.live.now(|state| {
            state.store.account = Some(crate::model::Me::default());
            state.store.assistant = Some(crate::model::Assistant {
                user_id: 2,
                display_name: "Assistant".into(),
                mention: Some("@ai".into()),
                draw: None,
                vision: false,
                images: false,
                processor: Some("Microsoft — Azure OpenAI".into()),
                transcribe: true,
                transcribe_max_bytes: Some(26_214_400),
                lookups: Vec::new(),
                greeting_weather: false,
            });
            state
                .store
                .set_assistant_consent(Some("2026-10-02T10:00:00Z".into()));
        });
        let video = |id: i64, duration_ms: i64| Attachment {
            id,
            kind: "video".into(),
            mime: Some("video/quicktime".into()),
            size: Some(QUICKTIME.len() as i64),
            duration_ms: Some(duration_ms),
            width: Some(640),
            height: Some(360),
            ..Attachment::default()
        };
        let show = |actions: &Actions, attachment: Attachment| {
            actions.handle(Action::ShowTranscript {
                chat_id: 42,
                message_id: 100,
                attachment,
                ask_consent: Callback::noop(),
            });
        };
        let held = |actions: &Actions, id: i64| {
            actions
                .live
                .read(|state| state.store.transcripts.get(id).cloned())
        };
        let settle = |actions: &Actions, id: i64| {
            let actions = actions.clone();
            async move {
                for _ in 0..400 {
                    if held(&actions, id) != Some(State::Asking) {
                        break;
                    }
                    gloo_timers::future::TimeoutFuture::new(25).await;
                }
            }
        };
        const UNDER: &[&str] = &["/attachments/3", "/chats/42/messages/100/attachments/3"];

        show(&actions, video(36, 500));
        assert_eq!(held(&actions, 36), Some(State::Asking));
        settle(&actions, 36).await;
        assert_eq!(
            held(&actions, 36),
            Some(State::Shown(Transcript {
                text: "Look at the snow".into(),
                language: Some("en".into()),
            }))
        );
        assert_eq!(
            server.lines(UNDER),
            vec![
                "GET /attachments/36".to_string(),
                "POST /chats/42/messages/100/attachments/36/transcript".to_string(),
            ],
            "the file, then its sound — and nothing asked of the stored copy"
        );
        let posted = &server.asked(&["/chats/42/messages/100/attachments/36/transcript"])[0];
        assert!(
            posted.content_type.starts_with("multipart/form-data"),
            "{}",
            posted.content_type
        );
        let body = String::from_utf8_lossy(&posted.body);
        assert!(body.contains("name=\"audio\""), "the one part is `audio`");
        assert!(body.contains("Content-Type: audio/mp4"));
        assert!(body.contains("ftypM4A "), "an M4A, not the movie");
        assert!(!body.contains("avc1"), "nothing of the picture");

        // Held: shown again with nobody asked.
        actions.handle(Action::HideTranscript { attachment_id: 36 });
        show(&actions, video(36, 500));
        assert!(matches!(held(&actions, 36), Some(State::Shown(_))));
        gloo_timers::future::TimeoutFuture::new(50).await;
        assert_eq!(server.lines(UNDER).len(), 2, "asked once");

        // Two hours: too long even at 64 kbit/s, told at once — and the
        // file is never fetched.
        show(&actions, video(37, 2 * 60 * 60 * 1000));
        settle(&actions, 37).await;
        assert_eq!(held(&actions, 37), Some(State::Failed(Failure::TooLong)));
        // A film with no sound track: fetched, and nothing to send.
        show(&actions, video(38, 1_000));
        settle(&actions, 38).await;
        assert_eq!(held(&actions, 38), Some(State::Failed(Failure::Unreadable)));
        // A file that could not be fetched: worth another try.
        show(&actions, video(39, 1_000));
        settle(&actions, 39).await;
        assert_eq!(held(&actions, 39), Some(State::Failed(Failure::TryAgain)));
        assert_eq!(
            server.lines(UNDER),
            vec![
                "GET /attachments/36".to_string(),
                "POST /chats/42/messages/100/attachments/36/transcript".to_string(),
                "GET /attachments/38".to_string(),
                "GET /attachments/39".to_string(),
            ],
            "no sound was sent for any of them"
        );
        assert!(actions.live.read(|state| state.failure.is_none()));
    }

    /// A stored copy the server will not send after all — its own reading
    /// of the file disagreeing with the metadata — goes on with this
    /// device's sound, as the protocol allows ("a client that sent no body
    /// may send the sound track instead") and every other client does;
    /// any other refusal of the stored copy fetches nothing.
    #[wasm_bindgen_test]
    async fn a_stored_copy_the_server_refuses_goes_on_with_the_devices_sound() {
        use crate::encode::testing::QUICKTIME;
        use crate::fake_server::{Answer, FakeServer};
        use fc_text::transcript::{Failure, State, Transcript};
        let server = FakeServer::answering(move |asked| match asked.path.as_str() {
            "/attachments/40" => Answer::Bytes(200, QUICKTIME.to_vec(), "audio/mp4"),
            "/chats/42/messages/100/attachments/40/transcript" => {
                if asked.content_type.starts_with("multipart/form-data") {
                    Answer::Json(
                        200,
                        serde_json::json!({"transcript": {"text": "from here"}}),
                    )
                } else {
                    Answer::refusal(400, "not_transcribable")
                }
            }
            "/chats/42/messages/100/attachments/41/transcript" => {
                Answer::refusal(403, "transcript_not_allowed")
            }
            _ => Answer::refusal(404, "not_found"),
        });
        let actions = actions();
        actions.live.now(|state| {
            state.store.account = Some(crate::model::Me::default());
            state.store.assistant = Some(crate::model::Assistant {
                user_id: 2,
                display_name: "Assistant".into(),
                mention: Some("@ai".into()),
                draw: None,
                vision: false,
                images: false,
                processor: Some("Microsoft — Azure OpenAI".into()),
                transcribe: true,
                transcribe_max_bytes: Some(26_214_400),
                lookups: Vec::new(),
                greeting_weather: false,
            });
            state
                .store
                .set_assistant_consent(Some("2026-10-02T10:00:00Z".into()));
        });
        let voice = |id: i64| Attachment {
            id,
            kind: "audio".into(),
            mime: Some("audio/mp4".into()),
            size: Some(QUICKTIME.len() as i64),
            duration_ms: Some(500),
            ..Attachment::default()
        };
        let held = |actions: &Actions, id: i64| {
            actions
                .live
                .read(|state| state.store.transcripts.get(id).cloned())
        };
        for id in [40, 41] {
            actions.handle(Action::ShowTranscript {
                chat_id: 42,
                message_id: 100,
                attachment: voice(id),
                ask_consent: Callback::noop(),
            });
            for _ in 0..400 {
                if held(&actions, id) != Some(State::Asking) {
                    break;
                }
                gloo_timers::future::TimeoutFuture::new(25).await;
            }
        }
        assert_eq!(
            held(&actions, 40),
            Some(State::Shown(Transcript {
                text: "from here".into(),
                language: None,
            }))
        );
        assert_eq!(
            held(&actions, 41),
            Some(State::Failed(Failure::NotAvailable))
        );
        assert_eq!(
            server.lines(&["/attachments/4", "/chats/42/messages/100/attachments/4"]),
            vec![
                "POST /chats/42/messages/100/attachments/40/transcript".to_string(),
                "GET /attachments/40".to_string(),
                "POST /chats/42/messages/100/attachments/40/transcript".to_string(),
                "POST /chats/42/messages/100/attachments/41/transcript".to_string(),
            ],
            "the stored copy first; the file and its sound only after not_transcribable"
        );
    }

    /// A save that changed nothing is answered at once and sends nothing.
    #[wasm_bindgen_test]
    fn an_unchanged_note_is_not_sent() {
        let actions = actions();
        let heard = Rc::new(RefCell::new(Vec::new()));
        let sink = heard.clone();
        actions.handle(Action::UpdateNote {
            note_id: 3,
            patch: NotePatch::default(),
            done: Callback::from(move |answer: Option<String>| sink.borrow_mut().push(answer)),
        });
        assert_eq!(*heard.borrow(), vec![None]);
    }

    /// Opening the board puts the chat and its panels away — nothing is
    /// being READ while the board is in front — and choosing a chat puts the
    /// board away again.
    #[wasm_bindgen_test]
    fn the_board_and_a_chat_take_turns_in_the_main_pane() {
        let actions = actions();
        actions.live.now(|state| {
            state.open_chat = Some(42);
            state.at_newest = true;
        });
        actions.handle(Action::OpenBoard);
        actions.live.read(|state| {
            assert!(state.board_open);
            assert_eq!(state.open_chat, None);
            assert!(!state.at_newest);
            assert!(state.store.thread_view.is_none() && state.store.open_polls.is_none());
            assert_eq!(crate::sync::reading(state), None, "no chat is being read");
        });
        actions.handle(Action::SelectChat(42));
        actions.live.read(|state| {
            assert!(!state.board_open);
            assert_eq!(state.open_chat, Some(42));
        });
    }

    /// Peeking at a hidden note is per note, and forgotten with the note.
    #[wasm_bindgen_test]
    fn revealing_a_note_is_per_note() {
        let actions = actions();
        actions.handle(Action::RevealNote { note_id: 12 });
        actions
            .live
            .read(|state| assert!(state.store.board.revealed.contains(&12)));
        actions.live.now(|state| state.store.board.forget(12));
        actions
            .live
            .read(|state| assert!(!state.store.board.revealed.contains(&12)));
    }

    fn photo() -> Prepared {
        Prepared {
            kind: "photo".into(),
            mime: "image/jpeg".into(),
            size: 3,
            file: Some(web_sys::Blob::new().expect("a blob")),
            preview: Some(web_sys::Blob::new().expect("a blob")),
            ..Prepared::default()
        }
    }

    fn staged(actions: &Actions) -> usize {
        actions
            .live
            .read(|state| state.store.staged.get(&42).map(Vec::len).unwrap_or(0))
    }

    /// Ten a message, and the eleventh is not staged whichever door it
    /// came through.
    #[wasm_bindgen_test]
    fn no_more_than_ten_are_staged() {
        let actions = actions();
        for _ in 0..11 {
            actions.handle(Action::Stage {
                chat_id: 42,
                item: photo(),
            });
        }
        assert_eq!(staged(&actions), 10);
        actions.handle(Action::Unstage {
            chat_id: 42,
            index: 0,
        });
        assert_eq!(staged(&actions), 9);
    }

    /// Sending what was staged empties the strip; sending a place — which
    /// always goes alone — leaves the strip exactly as it was.
    #[wasm_bindgen_test]
    fn a_location_send_leaves_the_strip_alone() {
        let actions = actions();
        for _ in 0..2 {
            actions.handle(Action::Stage {
                chat_id: 42,
                item: photo(),
            });
        }
        actions.handle(Action::Send {
            chat_id: 42,
            draft: Draft {
                attachments: vec![Prepared::location(1.0, 2.0, None)],
                ..Draft::default()
            },
        });
        assert_eq!(staged(&actions), 2, "the photos wait for the next message");

        let strip = actions.live.read(|state| state.store.staged[&42].clone());
        actions.handle(Action::Send {
            chat_id: 42,
            draft: Draft {
                attachments: strip,
                ..Draft::default()
            },
        });
        assert_eq!(staged(&actions), 0, "what went is gone from the strip");
    }

    /// A voice note recorded here: audio with no name.
    fn voice(duration_ms: i64) -> Prepared {
        Prepared {
            kind: "audio".into(),
            mime: "audio/mp4".into(),
            size: 3,
            duration_ms: Some(duration_ms),
            file: Some(web_sys::Blob::new().expect("a blob")),
            ..Prepared::default()
        }
    }

    fn not_sent(actions: &Actions) -> Vec<crate::store::NotSent> {
        actions
            .live
            .read(|state| state.store.not_sent.get(&42).cloned().unwrap_or_default())
    }

    /// A VOICE NOTE LEFT IN REVIEW is not sent from then on (the plan for
    /// #79, S2.8): the chat left with it staged, it gets a row of its own,
    /// with the reply the box was answering and the box's words as its
    /// caption — and the box is left empty. Whatever else was staged stays
    /// staged, as it always has in this tab; and where nothing was in
    /// review, the words are kept as they always were.
    #[wasm_bindgen_test]
    fn a_voice_note_left_in_review_is_not_sent_and_takes_the_words_along() {
        let actions = actions();
        actions.handle(Action::Stage {
            chat_id: 42,
            item: photo(),
        });
        actions.handle(Action::Stage {
            chat_id: 42,
            item: voice(42_000),
        });
        actions.handle(Action::Stage {
            chat_id: 42,
            item: voice(7_000),
        });
        actions.handle(Action::SaveDraft {
            chat_id: 42,
            text: "  for you  ".into(),
            reply_to_message_id: Some(100),
        });
        let rows = not_sent(&actions);
        assert_eq!(rows.len(), 2, "each note its own row");
        assert_eq!(
            (rows[0].duration_ms, rows[0].caption.as_str()),
            (42_000, "for you")
        );
        assert_eq!(
            (rows[1].duration_ms, rows[1].caption.as_str()),
            (7_000, ""),
            "the words go once"
        );
        assert!(rows
            .iter()
            .all(|row| row.reply_to_message_id == Some(100) && row.note.is_voice_note()));
        assert_ne!(rows[0].id, rows[1].id);
        actions.live.read(|state| {
            let left: Vec<&str> = state.store.staged[&42]
                .iter()
                .map(|item| item.kind.as_str())
                .collect();
            assert_eq!(left, vec!["photo"], "the photo stays staged");
            assert!(
                !state.store.drafts.contains_key(&42),
                "the box is left empty"
            );
        });

        actions.handle(Action::SaveDraft {
            chat_id: 42,
            text: "half a thought".into(),
            reply_to_message_id: None,
        });
        actions.live.read(|state| {
            assert_eq!(
                state.store.drafts.get(&42).map(String::as_str),
                Some("half a thought")
            );
        });
        assert_eq!(not_sent(&actions).len(), 2);
    }

    /// A NOT-SENT VOICE MESSAGE goes by its own Send ALONE, with ITS reply
    /// and caption (S2.8): never with what is staged, never twice.
    #[wasm_bindgen_test]
    fn a_not_sent_voice_message_goes_alone_with_its_own_reply_and_caption() {
        let actions = actions();
        let id = actions.live.now(|state| {
            state
                .store
                .park(42, voice(12_000), 12_000, Some(100), "hi Anna".into())
        });
        actions.handle(Action::Stage {
            chat_id: 42,
            item: photo(),
        });
        let anna = crate::model::Mention {
            user_id: ANNA,
            name: "Anna".into(),
        };
        actions.handle(Action::SendNotSent {
            chat_id: 42,
            id,
            mentions: vec![anna.clone()],
        });
        actions.live.read(|state| {
            assert!(!state.store.not_sent.contains_key(&42), "its row is gone");
            assert_eq!(state.store.outbox.len(), 1);
            let row = &state.store.outbox[0];
            assert_eq!(row.body, "hi Anna");
            assert_eq!(row.reply_to_message_id, Some(100));
            assert_eq!(row.mentions, vec![anna.clone()]);
            assert_eq!(row.items.len(), 1, "alone");
            assert_eq!(
                (row.items[0].kind.as_str(), row.items[0].duration_ms),
                ("audio", Some(12_000))
            );
        });
        assert_eq!(staged(&actions), 1, "what is staged stays staged");
        actions.handle(Action::SendNotSent {
            chat_id: 42,
            id,
            mentions: Vec::new(),
        });
        actions
            .live
            .read(|state| assert_eq!(state.store.outbox.len(), 1, "and only once"));
    }

    #[wasm_bindgen_test]
    fn deleting_a_not_sent_voice_message_takes_only_it() {
        let actions = actions();
        let first = actions.live.now(|state| {
            state
                .store
                .park(42, voice(3_000), 3_000, None, String::new())
        });
        let second = actions.live.now(|state| {
            state
                .store
                .park(42, voice(4_000), 4_000, None, String::new())
        });
        actions.handle(Action::DeleteNotSent {
            chat_id: 42,
            id: first,
        });
        let rows = not_sent(&actions);
        assert_eq!(
            rows.iter().map(|row| row.id).collect::<Vec<_>>(),
            vec![second]
        );
        actions
            .live
            .read(|state| assert!(state.store.outbox.is_empty()));
    }

    /// A VOICE MESSAGE SENT FROM THE RECORDER (the plan for #79, S2.5) goes
    /// alone, with the reply the box was answering — whatever is staged stays
    /// staged — and only in the sign-in it was recorded in.
    #[wasm_bindgen_test]
    fn a_voice_message_sent_from_the_recorder_goes_alone_in_its_own_sign_in() {
        let actions = actions();
        actions.handle(Action::Stage {
            chat_id: 42,
            item: photo(),
        });
        let session = actions.live.session();
        actions.handle(Action::SendRecorded {
            session,
            chat_id: 42,
            note: voice(4_000),
            reply_to_message_id: Some(100),
        });
        actions.live.read(|state| {
            assert_eq!(state.store.outbox.len(), 1);
            let row = &state.store.outbox[0];
            assert_eq!(row.body, "");
            assert_eq!(row.reply_to_message_id, Some(100));
            assert_eq!(row.items.len(), 1, "alone");
            assert_eq!(
                (row.items[0].kind.as_str(), row.items[0].duration_ms),
                ("audio", Some(4_000))
            );
        });
        assert_eq!(staged(&actions), 1, "what is staged stays staged");

        actions.live.end_session();
        actions
            .live
            .now(|state| state.token = Some("theirs".into()));
        actions.handle(Action::SendRecorded {
            session,
            chat_id: 42,
            note: voice(4_000),
            reply_to_message_id: None,
        });
        actions
            .live
            .read(|state| assert!(state.store.outbox.is_empty(), "not in the next sign-in"));
    }

    /// A note finished after its pane went is kept from a second up, and
    /// only in the sign-in it was recorded in: one landing after a sign-out
    /// and somebody else's sign-in must not appear in their composer.
    #[wasm_bindgen_test]
    fn a_stopped_note_is_kept_from_a_second_and_only_in_its_own_sign_in() {
        let actions = actions();
        let first = actions.live.session();
        actions.live.end_session();
        actions
            .live
            .now(|state| state.token = Some("theirs".into()));
        let keep = |session: u64, duration_ms: i64| {
            actions.handle(Action::ParkStopped {
                session,
                chat_id: 42,
                note: voice(duration_ms),
                duration_ms,
                reply_to_message_id: Some(100),
            })
        };
        keep(first, 5_000);
        assert!(not_sent(&actions).is_empty(), "not in the next sign-in");
        let second = actions.live.session();
        keep(second, 900);
        assert!(
            not_sent(&actions).is_empty(),
            "under a second: nothing to keep"
        );
        keep(second, 5_000);
        let rows = not_sent(&actions);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].duration_ms, rows[0].reply_to_message_id),
            (5_000, Some(100))
        );
    }

    /// THE KEEPING, with a real microphone (the fake one webdriver.json
    /// starts the browser with): a recording handed on is finished — its
    /// microphone let go of — and kept as not sent with the reply it was
    /// recorded under; under a second it is deleted; recorded in another
    /// sign-in, it is dropped there and then.
    #[wasm_bindgen_test]
    async fn an_interrupted_recording_is_finished_and_kept_as_not_sent() {
        use crate::recorder::{in_progress, Handover, Listening, Recording};
        use gloo_timers::future::TimeoutFuture;
        let actions = actions();
        let session = actions.live.session();
        let park = |recording: Recording, session: u64, reply: Option<i64>| {
            actions.handle(Action::Park {
                session,
                chat_id: 42,
                recording: Handover::of(recording),
                reply_to_message_id: reply,
            })
        };
        let started = || async {
            Recording::start(Listening::in_the_click())
                .await
                .expect("recording starts")
        };

        let recording = started().await;
        TimeoutFuture::new(1_200).await;
        park(recording, session, Some(100));
        for _ in 0..250 {
            if !not_sent(&actions).is_empty() {
                break;
            }
            TimeoutFuture::new(20).await;
        }
        let rows = not_sent(&actions);
        assert_eq!(rows.len(), 1, "kept");
        assert_eq!(rows[0].note.kind, "audio");
        assert!(rows[0].duration_ms >= 1_000, "{} ms", rows[0].duration_ms);
        assert_eq!(rows[0].note.duration_ms, Some(rows[0].duration_ms));
        assert_eq!(rows[0].reply_to_message_id, Some(100));
        assert_eq!(rows[0].caption, "");
        assert!(!in_progress(), "its microphone let go of");

        let recording = started().await;
        TimeoutFuture::new(300).await;
        park(recording, session, None);
        TimeoutFuture::new(1_500).await;
        assert_eq!(not_sent(&actions).len(), 1, "under a second: deleted");
        assert!(!in_progress());

        let recording = started().await;
        TimeoutFuture::new(1_200).await;
        park(recording, session + 1, None);
        assert!(!in_progress(), "another sign-in's: let go of at once");
        TimeoutFuture::new(1_500).await;
        assert_eq!(not_sent(&actions).len(), 1);
    }

    fn roster(next_owner: Option<i64>) -> crate::model::Roster {
        crate::model::Roster {
            members: vec![
                crate::model::Member {
                    id: ME,
                    display_name: "Me".into(),
                    role: Some("owner".into()),
                    ..Default::default()
                },
                crate::model::Member {
                    id: ANNA,
                    display_name: "Anna".into(),
                    role: Some("member".into()),
                    ..Default::default()
                },
            ],
            next_owner_user_id: next_owner,
            ..Default::default()
        }
    }

    /// Absent on a FRESH read, and only there, is "nobody left" — the
    /// dialog that deletes the family. A successor the same read does not
    /// name is no answer at all.
    #[wasm_bindgen_test]
    fn the_leave_dialog_is_built_from_what_the_fresh_roster_says() {
        assert_eq!(
            leave_context(&roster(None), false),
            Some(LeaveContext::Member)
        );
        assert_eq!(
            leave_context(&roster(Some(ANNA)), true),
            Some(LeaveContext::Successor("Anna".into()))
        );
        assert_eq!(
            leave_context(&roster(None), true),
            Some(LeaveContext::LastMember)
        );
        assert_eq!(leave_context(&roster(Some(404)), true), None);
    }

    /// The waiting room asks one `/me` at a time: a tick while one is out
    /// waits for it, rather than piling another onto a slow server.
    #[wasm_bindgen_test]
    async fn the_waiting_room_asks_one_at_a_time() {
        let actions = actions();
        actions.live.now(|state| state.polling = true);
        actions.handle(Action::PollAccount);
        // A second poll would have gone out and come back (the test page has
        // no API behind it) and let go of the flag by now.
        gloo_timers::future::TimeoutFuture::new(300).await;
        assert!(
            actions.live.read(|state| state.polling),
            "no second one went"
        );
        actions.live.now(|state| state.polling = false);
        actions.handle(Action::PollAccount);
        assert!(actions.live.read(|state| state.polling), "one is out");
        gloo_timers::future::TimeoutFuture::new(300).await;
        assert!(
            !actions.live.read(|state| state.polling),
            "and let go once answered"
        );
    }

    fn webp(text: &str) -> web_sys::Blob {
        let parts = js_sys::Array::of1(&wasm_bindgen::JsValue::from_str(text));
        let options = web_sys::BlobPropertyBag::new();
        options.set_type("image/webp");
        web_sys::Blob::new_with_str_sequence_and_options(&parts, &options).expect("a blob")
    }

    fn pack_item(id: i64, attachment_id: i64) -> crate::model::PackItem {
        crate::model::PackItem {
            id,
            pack_seq: id,
            added_by: Some(ANNA),
            attachment: Some(Attachment {
                id: attachment_id,
                kind: "photo".into(),
                mime: Some("image/webp".into()),
                size: Some(11),
                width: Some(512),
                height: Some(512),
                // Inherited through dedup, as it can be: still no preview.
                has_preview: true,
                ..Attachment::default()
            }),
            ..Default::default()
        }
    }

    /// ONE CLICK SENDS: the pack's own bytes as this tab holds them, one
    /// attachment, no words, the flag — queued at once with no network at
    /// all, and nothing between the panel and the outbox prepares them as
    /// a photograph. The draft in the box and what is staged stay put.
    #[wasm_bindgen_test]
    async fn one_click_queues_the_packs_own_bytes_as_a_sticker() {
        let actions = actions();
        let bytes = webp("the sticker");
        actions.media.seed(71, Variant::Sticker, bytes.clone());
        actions.live.now(|state| {
            state.store.pack.limits = Some(crate::pack::Limits {
                items: 200,
                bytes: 524_288,
            });
            state.store.pack.apply(pack_item(5, 71));
            state.store.pack.apply(pack_item(6, 72));
            state.store.drafts.insert(42, "half a sentence".into());
            state.store.staged.insert(42, vec![photo()]);
        });

        actions.handle(Action::SendSticker {
            chat_id: 42,
            item_id: 5,
            reply_to_message_id: Some(100),
        });

        actions.live.read(|state| {
            let row = state.store.outbox.first().expect("queued at once");
            assert!(row.sticker);
            assert_eq!(row.body, "", "a sticker has no body");
            assert_eq!(
                row.reply_to_message_id,
                Some(100),
                "it may answer something"
            );
            assert_eq!(row.items.len(), 1, "exactly one attachment");
            let item = &row.items[0];
            assert_eq!(
                (item.kind.as_str(), item.mime.as_str()),
                ("photo", "image/webp")
            );
            assert_eq!(item.size, 11);
            assert!(
                !item.has_preview,
                "no preview is made, whatever the item says"
            );
            assert_eq!(item.source_attachment_id, Some(71));
            // THE BYPASS: the very blob the pack holds, not a re-encode.
            let staged = &state.store.bytes[&item.provisional_id];
            assert_eq!(staged.file.as_ref(), Some(&bytes));
            assert!(staged.preview.is_none());
            // The bubble is already a sticker, answering what was answered.
            let pending = state.store.threads[&42]
                .messages
                .last()
                .expect("its bubble");
            assert!(pending.sticker().is_some());
            assert!(pending.reply_to.is_some());
            // And nothing else of the composer went with it.
            assert_eq!(
                state.store.drafts.get(&42).map(String::as_str),
                Some("half a sentence")
            );
            assert_eq!(state.store.staged[&42].len(), 1);
            assert_eq!(
                state.store.pack.recents.first(),
                Some(&5),
                "used last, shown first"
            );
            assert_eq!(
                state
                    .store
                    .pack
                    .panel()
                    .iter()
                    .map(|item| item.id)
                    .collect::<Vec<_>>(),
                vec![5, 6]
            );
        });

        // An item that has since gone from the pack sends nothing.
        actions.handle(Action::SendSticker {
            chat_id: 42,
            item_id: 404,
            reply_to_message_id: None,
        });
        assert_eq!(actions.live.read(|state| state.store.outbox.len()), 1);
    }

    /// The refusals a person can run into, in words — `pack_full` and
    /// `pack_item_too_large` said out loud, like `board_full`.
    #[wasm_bindgen_test]
    fn a_refused_pack_change_is_said_in_words() {
        let refusal = |code: &str| ApiError::Server {
            code: code.into(),
            message: "for developers".into(),
        };
        assert!(pack_failure(&refusal("pack_full")).contains("sticker pack is full"));
        assert!(pack_failure(&refusal("pack_item_too_large")).contains("too large for a sticker"));
        assert!(pack_failure(&refusal("not_pack_item_author")).contains("family owner"));
        assert!(pack_failure(&refusal("pack_item_not_found")).contains("already been removed"));
        assert!(pack_failure(&refusal("attachment_expired")).contains("took too long to add"));
        assert!(pack_failure(&refusal("invalid_attachment")).contains("WebP or PNG"));
        assert_eq!(pack_failure(&refusal("not_in_family")), "for developers");
        assert_eq!(
            sticker_refusal(fc_text::pack::Refusal::TooLarge),
            too_large_for_a_sticker()
        );
        assert_eq!(
            sticker_refusal(fc_text::pack::Refusal::Animated),
            "An animated sticker must be a WebP file.",
            "its own words: its size is not what is wrong with it"
        );
    }

    /// Refused beside the picker, before anything is uploaded: a full
    /// pack, a label too long, and a server with no packs at all.
    #[wasm_bindgen_test]
    async fn adding_is_refused_at_the_picker_for_what_this_tab_can_see() {
        let actions = actions();
        let session = actions.live.session();
        let picture = webp("not really a picture");
        assert!(
            actions
                .add_sticker(session, "t", picture.clone(), None)
                .await
                .is_err(),
            "no limits: a server from before packs"
        );
        actions.live.now(|state| {
            state.store.pack.limits = Some(crate::pack::Limits {
                items: 1,
                bytes: 524_288,
            });
        });
        assert_eq!(
            actions
                .add_sticker(session, "t", picture.clone(), Some("x".repeat(65)))
                .await,
            Err("A description is at most 64 characters.".to_string())
        );
        actions.live.now(|state| {
            state.store.pack.apply(pack_item(5, 71));
        });
        assert_eq!(
            actions.add_sticker(session, "t", picture, None).await,
            Err(pack_full().to_string())
        );
    }

    /// A real PNG, as a canvas writes one — `add_sticker` decodes what it
    /// is handed before anything goes up.
    async fn png(width: u32, height: u32) -> web_sys::Blob {
        use wasm_bindgen::JsCast;
        let canvas: web_sys::HtmlCanvasElement = web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .create_element("canvas")
            .unwrap()
            .dyn_into()
            .unwrap();
        canvas.set_width(width);
        canvas.set_height(height);
        let context: web_sys::CanvasRenderingContext2d = canvas
            .get_context("2d")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        context.set_fill_style_str("rgba(200, 30, 30, 0.5)");
        context.fill_rect(0.0, 0.0, f64::from(width), f64::from(height));
        let (heard, hearing) = futures::channel::oneshot::channel::<web_sys::Blob>();
        let heard = RefCell::new(Some(heard));
        let done = wasm_bindgen::closure::Closure::once_into_js(move |blob: web_sys::Blob| {
            if let Some(heard) = heard.borrow_mut().take() {
                let _ = heard.send(blob);
            }
        });
        canvas
            .to_blob_with_type(done.unchecked_ref(), "image/png")
            .unwrap();
        hearing.await.expect("a PNG")
    }

    /// The requests an add is made of (`FakeServer::asked`).
    const PACK_ROUTES: &[&str] = &["/families/mine/pack", "/attachments"];

    fn with_a_pack(actions: &Actions, items: i64) {
        actions.live.now(|state| {
            state.store.pack.limits = Some(crate::pack::Limits {
                items,
                bytes: 524_288,
            });
        });
    }

    fn wire_item(id: i64, attachment_id: i64, label: Option<&str>) -> serde_json::Value {
        let mut item = serde_json::json!({
            "id": id, "added_by": ME, "pack_seq": 12, "created_at": "2026-09-30T10:00:00Z",
            "attachment": {"id": attachment_id, "kind": "photo", "mime": "image/png",
                           "size": 11, "width": 96, "height": 64}});
        if let Some(label) = label {
            item["label"] = label.into();
        }
        item
    }

    /// ADDING IS UPLOAD, THEN CLAIM — the bytes as they are, declared as
    /// what they are, with their pixel size and NO preview after them; then
    /// the claim, naming the upload and carrying the label trimmed. A `201`
    /// is a new item: it is in the pack at once, under the per-item guard
    /// with the cursor where it was, and drawn from the bytes this tab just
    /// sent rather than fetched back.
    #[wasm_bindgen_test]
    async fn adding_a_sticker_uploads_it_and_then_claims_it() {
        use crate::fake_server::{Answer, FakeServer};
        let server = FakeServer::answering(|asked| {
            if asked.method == "POST" && asked.path.starts_with("/attachments?") {
                Answer::Json(
                    201,
                    serde_json::json!({"attachment": {"id": 71, "kind": "photo",
                                                      "mime": "image/png"}}),
                )
            } else if asked.line() == "POST /families/mine/pack" {
                Answer::Json(
                    201,
                    serde_json::json!({"item": wire_item(5, 71, Some("party cat"))}),
                )
            } else {
                Answer::refusal(404, "not_found")
            }
        });
        let actions = actions();
        let session = actions.live.session();
        with_a_pack(&actions, 200);
        let picture = png(96, 64).await;
        let size = picture.size() as usize;

        let added = actions
            .add_sticker(session, "t", picture, Some("  party cat ".into()))
            .await;

        assert_eq!(added, Ok(true), "a 201 is a new item");
        assert_eq!(
            server.lines(PACK_ROUTES),
            vec![
                "POST /attachments?kind=photo&width=96&height=64",
                "POST /families/mine/pack"
            ],
            "upload, then claim — and no preview, no read-back"
        );
        let asked = server.asked(PACK_ROUTES);
        assert_eq!(asked[0].content_type, "image/png", "what the bytes are");
        assert_eq!(asked[0].body.len(), size, "as they are");
        assert_eq!(
            fc_text::pack::type_of(&asked[0].body),
            Some("image/png"),
            "never made a JPEG on the way"
        );
        assert_eq!(
            asked[1].json(),
            serde_json::json!({"attachment_id": 71, "label": "party cat"})
        );
        actions.live.read(|state| {
            let pack = &state.store.pack;
            assert_eq!(pack.items[&5].label.as_deref(), Some("party cat"));
            assert_eq!(pack.cursor, 0, "the frame moves the cursor, not the answer");
        });
        assert_eq!(
            actions
                .media
                .held(71, Variant::Sticker)
                .map(|held| held.size() as usize),
            Some(size),
            "seeded under the id the pack holds"
        );
    }

    /// A `200` IS NOT A NEW ITEM: the pack already held these bytes, and
    /// the item that comes back carries the attachment id the pack HAD, not
    /// the one this upload was given. So that is the id the bytes are
    /// cached under — and no label goes where none was given.
    #[wasm_bindgen_test]
    async fn a_sticker_the_pack_already_holds_is_answered_with_the_one_there() {
        use crate::fake_server::{Answer, FakeServer};
        let server = FakeServer::answering(|asked| {
            if asked.method == "POST" && asked.path.starts_with("/attachments?") {
                Answer::Json(
                    201,
                    serde_json::json!({"attachment": {"id": 71, "kind": "photo"}}),
                )
            } else {
                Answer::Json(200, serde_json::json!({"item": wire_item(3, 60, None)}))
            }
        });
        let actions = actions();
        let session = actions.live.session();
        with_a_pack(&actions, 200);

        let added = actions
            .add_sticker(session, "t", png(32, 32).await, Some("   ".into()))
            .await;

        assert_eq!(added, Ok(false), "there all along");
        assert_eq!(
            server.asked(PACK_ROUTES)[1].json(),
            serde_json::json!({"attachment_id": 71}),
            "an empty label is no label, and absent says so"
        );
        assert!(actions
            .live
            .read(|state| state.store.pack.items.contains_key(&3)));
        assert!(actions.media.held(60, Variant::Sticker).is_some());
        assert!(
            actions.media.held(71, Variant::Sticker).is_none(),
            "the upload's own id names a row the server dropped"
        );
    }

    /// A refused claim is said in words and adds nothing — `pack_full` for
    /// a pack another member filled while this picture was on its way up,
    /// and the upload's own refusal before any claim is made.
    #[wasm_bindgen_test]
    async fn a_refused_add_says_why_and_adds_nothing() {
        use crate::fake_server::{Answer, FakeServer};
        let server = FakeServer::answering(|asked| {
            if asked.method == "POST" && asked.path.starts_with("/attachments?") {
                Answer::Json(
                    201,
                    serde_json::json!({"attachment": {"id": 71, "kind": "photo"}}),
                )
            } else {
                Answer::refusal(409, "pack_full")
            }
        });
        let actions = actions();
        let session = actions.live.session();
        with_a_pack(&actions, 200);
        assert_eq!(
            actions
                .add_sticker(session, "t", png(32, 32).await, None)
                .await,
            Err(pack_full().to_string())
        );
        assert_eq!(server.lines(PACK_ROUTES).len(), 2);
        assert!(actions.live.read(|state| state.store.pack.items.is_empty()));
        drop(server);

        let server = FakeServer::answering(|_| Answer::refusal(413, "attachment_too_large"));
        assert_eq!(
            actions
                .add_sticker(session, "t", png(32, 32).await, None)
                .await,
            Err(too_large_for_a_sticker().to_string())
        );
        assert_eq!(server.lines(PACK_ROUTES).len(), 1, "nothing to claim");
    }

    /// `attachment_expired` ON THE CLAIM IS RETRIED ONCE, BY ITSELF: the
    /// same bytes go up again and are claimed again under the new id, and
    /// nobody is told anything — the sticker is simply in the pack. Only a
    /// SECOND expiry is said, in words, after exactly one more upload.
    #[wasm_bindgen_test]
    async fn an_expired_upload_goes_up_again_once_and_only_a_second_failure_is_said() {
        use crate::fake_server::{Answer, FakeServer};
        let uploads = Rc::new(RefCell::new(0i64));
        let server = {
            let uploads = uploads.clone();
            FakeServer::answering(move |asked| {
                if asked.method == "POST" && asked.path.starts_with("/attachments?") {
                    *uploads.borrow_mut() += 1;
                    let id = 70 + *uploads.borrow();
                    Answer::Json(
                        201,
                        serde_json::json!({"attachment": {"id": id, "kind": "photo",
                                                          "mime": "image/png"}}),
                    )
                } else if asked.json()["attachment_id"] == 71 {
                    Answer::refusal(410, "attachment_expired")
                } else {
                    Answer::Json(
                        201,
                        serde_json::json!({"item": wire_item(5, 72, Some("party cat"))}),
                    )
                }
            })
        };
        let actions = actions();
        let session = actions.live.session();
        with_a_pack(&actions, 200);
        let picture = png(32, 32).await;
        assert_eq!(
            actions
                .add_sticker(session, "t", picture, Some("party cat".into()))
                .await,
            Ok(true),
            "nothing is said of the first expiry"
        );
        let asked = server.asked(PACK_ROUTES);
        assert_eq!(
            asked
                .iter()
                .map(|asked| asked.method.as_str())
                .collect::<Vec<_>>(),
            vec!["POST"; 4]
        );
        assert!(asked[0].path.starts_with("/attachments?"));
        assert_eq!(asked[1].json()["attachment_id"], 71);
        assert_eq!(asked[2].path, asked[0].path, "the same upload, again");
        assert_eq!(asked[2].body, asked[0].body, "the same bytes");
        assert_eq!(
            asked[3].json(),
            serde_json::json!({"attachment_id": 72, "label": "party cat"}),
            "claimed under the NEW id, with the label it had"
        );
        assert!(actions
            .live
            .read(|state| state.store.pack.items.contains_key(&5)));
        drop(server);

        // Expired both times: one retry and no more, and then the words.
        let server = FakeServer::answering(|asked| {
            if asked.method == "POST" && asked.path.starts_with("/attachments?") {
                Answer::Json(
                    201,
                    serde_json::json!({"attachment": {"id": 81, "kind": "photo"}}),
                )
            } else {
                Answer::refusal(410, "attachment_expired")
            }
        });
        let actions = self::actions();
        let session = actions.live.session();
        with_a_pack(&actions, 200);
        assert_eq!(
            actions
                .add_sticker(session, "t", png(32, 32).await, None)
                .await,
            Err("The sticker took too long to add. Try again.".to_string())
        );
        assert_eq!(
            server.lines(PACK_ROUTES).len(),
            4,
            "upload, claim, upload, claim — and no third try: {:?}",
            server.lines(PACK_ROUTES)
        );
        assert!(actions.live.read(|state| state.store.pack.items.is_empty()));
        drop(server);

        // Any OTHER refusal of the claim is said at once, with no retry.
        let server = FakeServer::answering(|asked| {
            if asked.method == "POST" && asked.path.starts_with("/attachments?") {
                Answer::Json(
                    201,
                    serde_json::json!({"attachment": {"id": 81, "kind": "photo"}}),
                )
            } else {
                Answer::refusal(413, "pack_item_too_large")
            }
        });
        assert_eq!(
            actions
                .add_sticker(session, "t", png(32, 32).await, None)
                .await,
            Err(too_large_for_a_sticker().to_string())
        );
        assert_eq!(server.lines(PACK_ROUTES).len(), 2);
    }

    /// What `done` heard of a removal, once it has.
    async fn removal_heard(actions: &Actions, item_id: i64) -> Vec<Option<String>> {
        let heard = Rc::new(RefCell::new(Vec::new()));
        let done = {
            let heard = heard.clone();
            Callback::from(move |failure: Option<String>| heard.borrow_mut().push(failure))
        };
        actions.handle(Action::RemoveSticker { item_id, done });
        for _ in 0..40 {
            if !heard.borrow().is_empty() {
                break;
            }
            gloo_timers::future::TimeoutFuture::new(25).await;
        }
        let heard = heard.borrow().clone();
        heard
    }

    /// `pack_item_not_found` ON A REMOVAL MEANS IT IS ALREADY GONE: the
    /// item leaves this panel, stays gone, and NO error is shown — exactly
    /// what an answered removal does. Any other refusal is said, and takes
    /// nothing out.
    #[wasm_bindgen_test]
    async fn removing_a_sticker_that_is_already_gone_is_not_an_error() {
        use crate::fake_server::{Answer, FakeServer};
        let actions = actions();
        with_a_pack(&actions, 200);
        actions.live.now(|state| {
            for id in [5, 6, 7] {
                state.store.pack.apply(pack_item(id, 70 + id));
            }
        });
        let held = |actions: &Actions| -> Vec<i64> {
            actions.live.read(|state| {
                state
                    .store
                    .pack
                    .listed()
                    .iter()
                    .map(|item| item.id)
                    .collect()
            })
        };

        let server = FakeServer::answering(|asked| match asked.line().as_str() {
            "DELETE /families/mine/pack/5" => Answer::refusal(404, "pack_item_not_found"),
            "DELETE /families/mine/pack/6" => Answer::Json(204, serde_json::Value::Null),
            _ => Answer::refusal(403, "not_pack_item_author"),
        });
        assert_eq!(
            removal_heard(&actions, 5).await,
            vec![None],
            "already gone: no error"
        );
        assert_eq!(held(&actions), vec![6, 7]);
        assert!(
            actions.live.read(|state| state.failure.is_none()),
            "and none anywhere else on the page"
        );
        assert!(
            !actions
                .live
                .update(actions.live.session(), |state| state
                    .store
                    .pack
                    .apply(pack_item(5, 75)))
                .expect("the session is live"),
            "and it stays gone, whatever arrives late"
        );

        assert_eq!(removal_heard(&actions, 6).await, vec![None], "a 204");
        assert_eq!(held(&actions), vec![7]);

        assert_eq!(
            removal_heard(&actions, 7).await,
            vec![Some(
                "Only the person who added a sticker, or the family owner, can remove it."
                    .to_string()
            )]
        );
        assert_eq!(held(&actions), vec![7], "refused: still in the pack");
        assert_eq!(
            server.lines(&["/families/mine/pack"]),
            vec![
                "DELETE /families/mine/pack/5",
                "DELETE /families/mine/pack/6",
                "DELETE /families/mine/pack/7"
            ]
        );
    }

    /// A chat of `kind` in the list, so the handler can tell whose it is.
    fn with_a_chat(actions: &Actions, id: i64, kind: &str) {
        actions.live.now(|state| {
            let mut item = state.store.chats[0].clone();
            item.chat.id = id;
            item.chat.kind = kind.into();
            state.store.chats.push(item);
        });
    }

    /// IN THE ASSISTANT'S CHAT A STICKER NEVER GOES ROUND THE CONSENT
    /// QUESTION: until this member has agreed, the send queues NOTHING —
    /// the panel asks instead — and once they have, it is a sticker like
    /// any other. On a server that names nobody it is never sent at all. A
    /// chat between people is untouched by any of it.
    #[wasm_bindgen_test]
    async fn a_sticker_to_the_assistant_waits_for_consent() {
        let actions = actions();
        actions
            .media
            .seed(71, Variant::Sticker, webp("the sticker"));
        with_a_pack(&actions, 200);
        with_a_chat(&actions, 43, "ai");
        let named = |processor: Option<&str>| crate::model::Assistant {
            user_id: 2,
            display_name: "Assistant".into(),
            mention: None,
            draw: None,
            vision: true,
            transcribe: false,
            transcribe_max_bytes: None,
            lookups: Vec::new(),
            greeting_weather: false,
            images: false,
            processor: processor.map(str::to_string),
        };
        actions.live.now(|state| {
            state.store.account = Some(crate::model::Me::default());
            state.store.assistant = Some(named(Some("Microsoft — Azure OpenAI")));
            state.store.pack.apply(pack_item(5, 71));
        });
        let queued = |actions: &Actions| -> Vec<i64> {
            actions
                .live
                .read(|state| state.store.outbox.iter().map(|row| row.chat_id).collect())
        };
        let send = |chat_id: i64| {
            actions.handle(Action::SendSticker {
                chat_id,
                item_id: 5,
                reply_to_message_id: None,
            });
        };

        send(43);
        assert_eq!(queued(&actions), Vec::<i64>::new(), "not before the answer");
        assert!(
            actions
                .live
                .read(|state| state.store.pack.recents.is_empty()),
            "nothing was used"
        );
        send(42);
        assert_eq!(queued(&actions), vec![42], "the family chat asks nothing");

        actions.live.now(|state| {
            state
                .store
                .set_assistant_consent(Some("2026-09-30T10:00:00Z".into()))
        });
        send(43);
        assert_eq!(queued(&actions), vec![42, 43], "agreed: sent");
        let row = actions.live.read(|state| state.store.outbox[1].clone());
        assert!(row.sticker && row.body.is_empty() && row.items.len() == 1);

        // A server that will not say who answers gets nothing, agreed or not.
        actions
            .live
            .now(|state| state.store.assistant = Some(named(None)));
        send(43);
        assert_eq!(queued(&actions), vec![42, 43]);
    }

    /// A sticker opens larger, and closes; leaving the family closes it too.
    #[wasm_bindgen_test]
    fn a_sticker_opens_larger_and_closes() {
        let actions = actions();
        let picture = pack_item(5, 71).attachment.expect("its picture");
        actions.handle(Action::OpenSticker(picture.clone()));
        assert_eq!(
            actions.live.read(|state| state.sticker_open.clone()),
            Some(picture.clone())
        );
        actions.handle(Action::CloseSticker);
        assert!(actions.live.read(|state| state.sticker_open.is_none()));
        actions.handle(Action::OpenSticker(picture));
        actions.live.now(sync::put_away_panes);
        assert!(actions.live.read(|state| state.sticker_open.is_none()));
    }

    /// THE LOOKUP CONSENT, end to end against a stand-in server
    /// (docs/protocol.md, "Consenting to the assistant", amended
    /// 2026-10-03): "Agree With Lookups" asks for the assistant consent
    /// FIRST and the lookup consent only once that is held, each stamp the
    /// server's own; stopping lookups withdraws only them; a failed first
    /// consent sends no second; and `assistant_consent_required` on the
    /// second is taken at its word — this tab stops believing in the first.
    #[wasm_bindgen_test]
    async fn agreeing_with_lookups_asks_for_the_assistant_first() {
        use crate::fake_server::{Answer, FakeServer};
        const FIRST: &str = "/me/assistant-consent";
        const SECOND: &str = "/me/assistant-lookup-consent";
        let first_answers = Rc::new(RefCell::new(true));
        let second_refuses = Rc::new(RefCell::new(false));
        let server = {
            let first_answers = first_answers.clone();
            let second_refuses = second_refuses.clone();
            FakeServer::answering(move |asked| {
                let granted = asked.json()["granted"].as_bool() == Some(true);
                if asked.path == FIRST {
                    if !*first_answers.borrow() {
                        return Answer::refusal(500, "internal");
                    }
                    let at = granted.then_some("2026-10-03T09:00:00Z");
                    Answer::Json(200, serde_json::json!({"assistant_consent_at": at}))
                } else if asked.path == SECOND {
                    if *second_refuses.borrow() {
                        return Answer::refusal(403, "assistant_consent_required");
                    }
                    let at = granted.then_some("2026-10-03T09:30:00Z");
                    Answer::Json(200, serde_json::json!({"assistant_lookup_consent_at": at}))
                } else {
                    Answer::refusal(404, "not_found")
                }
            })
        };
        let actions = actions();
        actions.live.now(|state| {
            state.store.account = Some(crate::model::Me::default());
        });
        let held = |actions: &Actions| {
            actions.live.read(|state| {
                (
                    state.store.assistant_consent_at().map(str::to_owned),
                    state.store.assistant_lookup_consent_at().map(str::to_owned),
                )
            })
        };
        let settle = |count: usize| {
            let server = &server;
            async move {
                for _ in 0..40 {
                    if server.asked(&[FIRST, SECOND]).len() >= count {
                        break;
                    }
                    gloo_timers::future::TimeoutFuture::new(25).await;
                }
                gloo_timers::future::TimeoutFuture::new(50).await;
            }
        };

        actions.handle(Action::AgreeWithLookups);
        settle(2).await;
        assert_eq!(
            server.lines(&[FIRST, SECOND]),
            vec![format!("POST {FIRST}"), format!("POST {SECOND}")],
            "the assistant first, then the lookups"
        );
        assert_eq!(
            held(&actions),
            (
                Some("2026-10-03T09:00:00Z".into()),
                Some("2026-10-03T09:30:00Z".into())
            ),
            "the server's stamps, both"
        );

        // Stopping lookups: only them.
        actions.handle(Action::SetAssistantLookupConsent { granted: false });
        settle(3).await;
        assert_eq!(
            server.asked(&[SECOND]).last().unwrap().json(),
            serde_json::json!({"granted": false})
        );
        assert_eq!(held(&actions), (Some("2026-10-03T09:00:00Z".into()), None));

        // The first consent fails: no second request, nothing believed.
        actions
            .live
            .now(|state| state.store.set_assistant_consent(None));
        *first_answers.borrow_mut() = false;
        actions.handle(Action::AgreeWithLookups);
        settle(4).await;
        assert_eq!(server.asked(&[FIRST, SECOND]).len(), 4);
        assert_eq!(
            server.asked(&[SECOND]).len(),
            2,
            "no lookup consent without the first"
        );
        assert_eq!(held(&actions), (None, None));
        assert_eq!(
            actions.live.read(|state| state.failure.clone()).as_deref(),
            Some("Couldn't save your answer. Try again.")
        );

        // Withdrawn elsewhere: the server's word on the second wins.
        actions.live.now(|state| {
            state.failure = None;
            state
                .store
                .set_assistant_consent(Some("2026-10-03T09:00:00Z".into()));
        });
        *second_refuses.borrow_mut() = true;
        actions.handle(Action::SetAssistantLookupConsent { granted: true });
        settle(5).await;
        assert_eq!(
            held(&actions),
            (None, None),
            "this tab no longer believes it agreed"
        );
        assert!(actions.live.read(|state| state.failure.is_some()));
    }
}
