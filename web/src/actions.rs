//! Everything a person does in a chat, handled in one place.
//!
//! Views say WHAT happened — an `Action` — and never talk to the server or
//! the store themselves. That keeps every rule about a request (what is
//! optimistic, what waits for the answer, what a failure undoes, which
//! answers may and may not move a cursor) here, next to the others.

use fc_text::i18n::{t, t1};
use std::rc::Rc;

use wasm_bindgen_futures::spawn_local;
use yew::Callback;

use crate::api::{self, ApiError, FamilyPatch, NewNote, NotePatch};
use crate::live::{Live, Opening, Panel, Viewing};
use crate::media::{MediaLoader, Variant};
use crate::model::{Attachment, Reaction};
use crate::outbox::Wake;
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
    SaveDraft {
        chat_id: i64,
        text: String,
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
    /// Peek at a note a block hides.
    RevealNote {
        note_id: i64,
    },
    /// A photo prepared for the wall, to pin at this fraction of it.
    PinPhoto {
        photo: Prepared,
        at: (f64, f64),
    },
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

/// A colour for a new sticker, picked at random as the apps pick one, so a
/// run of new notes is not one yellow pile.
pub fn random_color() -> String {
    let palette = fc_text::board::COLORS;
    let index = (js_sys::Math::random() * palette.len() as f64) as usize;
    palette[index.min(palette.len() - 1)].to_string()
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
            Action::SaveDraft { chat_id, text } => {
                live.now(|state| {
                    if text.trim().is_empty() {
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
        }
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
}
