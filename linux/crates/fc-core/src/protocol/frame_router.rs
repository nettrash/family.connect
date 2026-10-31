//! What a live frame DOES (docs/protocol.md, "Server → client" and "Semantics"): the one place the
//! socket's frames meet the cache, so that a window only ever draws what is stored — a port of
//! `win/src/FamilyConnect.Core/Protocol/FrameRouter.cs`.
//!
//! The rules that are not obvious from the frame names, each of which cost somebody something in
//! one of the other ports:
//!
//! **A `read` FRAME IS TWO DIFFERENT FACTS.** One naming YOURSELF is your own marker, arriving
//! because you read on another device — applied `max(stored, received)` and followed by a recount.
//! One naming somebody else is roster data, and is drawn as "seen" only in a DIRECT chat: in the
//! family chat — or a group — a per-member seen state over N members is a row of faces nobody asked
//! for, and a per-member ABSENCE is exactly what a blocker's suppressed inward frames would put on
//! screen. Applying the second as if it were the first clears your own unread count from somebody
//! else's reading — so who "you" are is asked of the store, which already has to know (a message of
//! the reader's own raises no unread count either).
//!
//! **AN EDIT IS NOT A MESSAGE.** `message_edited` is a separate frame precisely so that it bumps no
//! unread count and raises no notification, and it carries the WHOLE message: the assistant's
//! picture answer arrives as an attachment added by exactly this frame, and a body-only merge draws
//! nothing. It also moves the chat's EDIT cursor, which no other route may.
//!
//! **A FRAME MAY MOVE A CURSOR; EVIDENCE MAY NOT.** A live `reaction` or `poll` frame is a complete
//! statement about that chat's sequence, so the cursor follows it even when the message it names is
//! one this device does not hold — the state is dropped, the cursor is not.
//!
//! **A FAMILY FRAME IS ABOUT THE FAMILY IT NAMES** (docs/protocol.md, "Several families"):
//! `member_joined`, `member_left`, `member_deleted` and `family_owner` are applied to the family
//! their `family_id` names, and `board_note` and `pack_item` to the wall and pack of the family their
//! note and item say — never to "my family". One naming the READER is a different fact: in a family
//! ([`on_joined_family`](FrameRouter::on_joined_family)), or out of one
//! ([`on_left_family`](FrameRouter::on_left_family)), which is the session's to act on.
//!
//! **How the app wires it.** One router per signed-in server, beside the socket:
//! `socket.on_frame().subscribe(move |frame| router.hear(frame))` — every frame, in arrival order,
//! on the main context. [`FrameRouter::hear`] answers a [`StoreResult`]: a cache that refused a
//! write stops that frame's events (the C# threw out of `Hear` at the same point), and the
//! subscriber logs the failure (ids and counts only) and goes on to the next frame — `GET /chats`
//! and the catch-up feeds are the reconciler for whatever one frame failed to write.
//!
//! The C# `event`s are [`Event`]s here; an event with several arguments carries them as a tuple, in
//! the C#'s order, and every one is named on its accessor.

use std::rc::Rc;

use super::api_error::ApiError;
use super::dtos::{MessageDto, NoteDto, PackItemDto};
use super::frames::{AiErrorReason, ServerFrame};
use super::requests::InvitationDto;
use crate::rt::Event;
use crate::store::{BoardStore, ChatStore, PackStore, SeqRoute, StoreResult};

/// What a live frame does to the cache, and who is told (`FrameRouter`).
pub struct FrameRouter {
    chats: Rc<ChatStore>,
    board: Rc<BoardStore>,
    pack: Option<Rc<PackStore>>,
    joined_family: Event<i64>,
    left_family: Event<i64>,
    invitation_changed: Event<InvitationDto>,
    invitation_removed: Event<i64>,
    family_plan: Event<(i64, Option<String>, Option<String>)>,
    arrived: Event<MessageDto>,
    edited: Event<MessageDto>,
    chat_changed: Event<i64>,
    typing: Event<(i64, i64)>,
    peer_read: Event<(i64, i64, i64)>,
    board_changed: Event<NoteDto>,
    pack_changed: Event<PackItemDto>,
    chat_removed: Event<i64>,
    roster_changed: Event<()>,
    block_changed: Event<(i64, bool)>,
    ai_delta: Event<(i64, i64, String)>,
    ai_stopped: Event<(i64, i64, Option<AiErrorReason>)>,
    call: Event<ServerFrame>,
    refused: Event<ApiError>,
}

impl std::fmt::Debug for FrameRouter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameRouter")
            .field("pack", &self.pack.is_some())
            .finish_non_exhaustive()
    }
}

impl FrameRouter {
    /// `new FrameRouter(chats, board, pack)`: the stores every frame is applied to. A router with
    /// no pack — nothing here has one to keep — drops `pack_item` frames quietly.
    pub fn new(chats: Rc<ChatStore>, board: Rc<BoardStore>, pack: Option<Rc<PackStore>>) -> Self {
        FrameRouter {
            chats,
            board,
            pack,
            joined_family: Event::new(),
            left_family: Event::new(),
            invitation_changed: Event::new(),
            invitation_removed: Event::new(),
            family_plan: Event::new(),
            arrived: Event::new(),
            edited: Event::new(),
            chat_changed: Event::new(),
            typing: Event::new(),
            peer_read: Event::new(),
            board_changed: Event::new(),
            pack_changed: Event::new(),
            chat_removed: Event::new(),
            roster_changed: Event::new(),
            block_changed: Event::new(),
            ai_delta: Event::new(),
            ai_stopped: Event::new(),
            call: Event::new(),
            refused: Event::new(),
        }
    }

    /// The READER is in a family — an approval, an accepted invitation, a family started on
    /// another device: the family's id. Raised for every `member_joined` naming them; whether the
    /// family is one this device already holds is the session's to judge, and a new one means
    /// re-reading `GET /me` and that family.
    pub fn on_joined_family(&self) -> &Event<i64> {
        &self.joined_family
    }

    /// The READER is out of a family — left on another device, or removed: the family's id. That
    /// family's chats, wall, pack, roster and cursors are to go — and only that family's
    /// ([`Database::drop_family`](crate::store::Database::drop_family)).
    pub fn on_left_family(&self) -> &Event<i64> {
        &self.left_family
    }

    /// An invitation to the reader, or by the reader's family, appeared or changed.
    pub fn on_invitation_changed(&self) -> &Event<InvitationDto> {
        &self.invitation_changed
    }

    /// An invitation lapsed: its id.
    pub fn on_invitation_removed(&self) -> &Event<i64> {
        &self.invitation_removed
    }

    /// A paid family's plan and deadline as `family_plan` says them (docs/protocol.md, "Paid
    /// families"): the family's id, its `plan`, and its `delete_at` — `None` when the frame has
    /// none, which CLEARS the one held. Applied to the family it names, by the session.
    pub fn on_family_plan(&self) -> &Event<(i64, Option<String>, Option<String>)> {
        &self.family_plan
    }

    /// A message that is NEW — the only frame that may raise a notification.
    ///
    /// It fires for this device's own sends too: the sender's OTHER connections receive `message`
    /// like everybody else, and a REST send fans out to all of them including the sender's own.
    /// Whose message it is is the subscriber's to judge — it holds the signed-in id and the block
    /// list, and both matter for what it does next.
    pub fn on_arrived(&self) -> &Event<MessageDto> {
        &self.arrived
    }

    /// A message whose words changed. Never a notification, never an unread count.
    pub fn on_edited(&self) -> &Event<MessageDto> {
        &self.edited
    }

    /// Something about this chat (its id) changed and a drawn list or thread is now stale.
    pub fn on_chat_changed(&self) -> &Event<i64> {
        &self.chat_changed
    }

    /// Somebody is typing in that chat, as of now: `(chat_id, user_id)`. Never persisted.
    pub fn on_typing(&self) -> &Event<(i64, i64)> {
        &self.typing
    }

    /// A peer's read marker moved in a DIRECT chat — the only place it is drawn:
    /// `(chat_id, user_id, last_read_message_id)`.
    pub fn on_peer_read(&self) -> &Event<(i64, i64, i64)> {
        &self.peer_read
    }

    /// The wall changed: a note created, edited, moved, or taken down.
    pub fn on_board_changed(&self) -> &Event<NoteDto> {
        &self.board_changed
    }

    /// The family's sticker pack changed: an item added, or one removed. Never a notification and
    /// never an unread count — and not filtered by blocks: an item is a picture the family keeps,
    /// not something a person said (docs/protocol.md, "What does not touch the pack").
    pub fn on_pack_changed(&self) -> &Event<PackItemDto> {
        &self.pack_changed
    }

    /// A GROUP this reader is no longer in has been dropped from the cache — left, removed from,
    /// or deleted (docs/protocol.md, "Groups"): its id. [`on_chat_changed`](Self::on_chat_changed)
    /// fires for it too; this is for whoever has it OPEN.
    pub fn on_chat_removed(&self) -> &Event<i64> {
        &self.chat_removed
    }

    /// The roster changed — a join, a leave, a deleted account, a new owner.
    pub fn on_roster_changed(&self) -> &Event<()> {
        &self.roster_changed
    }

    /// This reader's own block list changed, and with it what they can see:
    /// `(user_id, blocked)`.
    pub fn on_block_changed(&self) -> &Event<(i64, bool)> {
        &self.block_changed
    }

    /// The assistant, mid-reply: text to append to that message as it is drawn —
    /// `(chat_id, message_id, text)`.
    pub fn on_ai_delta(&self) -> &Event<(i64, i64, String)> {
        &self.ai_delta
    }

    /// The assistant stopped early, and the half-written answer is all there is — with the reason,
    /// when the frame gave one this client knows: `(chat_id, message_id, reason)`.
    pub fn on_ai_stopped(&self) -> &Event<(i64, i64, Option<AiErrorReason>)> {
        &self.ai_stopped
    }

    /// A call frame, passed on whole. Signalling is a conversation with state of its own and no
    /// business in a cache — the window's call layer owns it.
    pub fn on_call(&self) -> &Event<ServerFrame> {
        &self.call
    }

    /// A refusal that answers nothing this device is waiting for.
    pub fn on_refused(&self) -> &Event<ApiError> {
        &self.refused
    }

    /// One frame, applied. Unknown frames never reach here: the parser drops them.
    ///
    /// A cache that refuses a write answers its error before the frame's events are raised, as the
    /// C# threw out of `Hear` at the same point.
    pub fn hear(&self, frame: &ServerFrame) -> StoreResult<()> {
        let chats = &self.chats;
        match frame {
            ServerFrame::Message(message) => {
                // A LIVE frame, which is the only route that may raise an unread count.
                chats.apply(message, SeqRoute::LiveFrame, true)?;
                self.arrived.emit(message);
                self.chat_changed.emit(&message.chat_id);
            }

            ServerFrame::Ack { value, .. } => {
                // The send pipeline owns the row; the cache still wants the message, and applying
                // it twice is applying it once. It counts against nobody: an ack answers this
                // reader's own send.
                chats.apply(value, SeqRoute::LiveFrame, true)?;
                self.chat_changed.emit(&value.chat_id);
            }

            ServerFrame::MessageEdited(edited) => {
                // Out of sequence: the edited message may be one this device never held.
                chats.apply(edited, SeqRoute::Evidence, false)?;
                if let Some(edit_seq) = edited.edit_seq {
                    chats.advance(edited.chat_id, None, Some(edit_seq), None)?;
                }
                self.edited.emit(edited);
                self.chat_changed.emit(&edited.chat_id);
            }

            ServerFrame::Read {
                chat_id,
                user_id,
                last_read_message_id,
            } if *user_id == chats.reader() => {
                // YOUR OWN marker, from your other device. Monotonic, and the count follows it.
                chats.mark_read(*chat_id, *last_read_message_id)?;
                self.chat_changed.emit(chat_id);
            }

            ServerFrame::Read {
                chat_id,
                user_id,
                last_read_message_id,
            } => {
                // SOMEBODY ELSE'S, and it is drawn in a direct chat only.
                if chats.kind(*chat_id)?.as_deref() == Some("direct") {
                    self.peer_read
                        .emit(&(*chat_id, *user_id, *last_read_message_id));
                }
            }

            ServerFrame::Typing { chat_id, user_id } => {
                self.typing.emit(&(*chat_id, *user_id));
            }

            ServerFrame::Reactions {
                chat_id,
                message_id,
                reaction_seq,
                value,
            } => {
                // The cursor follows the FRAME, not the message: a state for a message this device
                // does not hold is dropped, and the sequence still happened. That rule lives in the
                // store, which is why the route is passed rather than acted on.
                chats.apply_reactions(
                    *chat_id,
                    *message_id,
                    *reaction_seq,
                    value,
                    SeqRoute::LiveFrame,
                )?;
                self.chat_changed.emit(chat_id);
            }

            ServerFrame::Poll {
                chat_id,
                message_id,
                value,
            } => {
                chats.apply_poll(*chat_id, *message_id, value, SeqRoute::LiveFrame)?;
                self.chat_changed.emit(chat_id);
            }

            ServerFrame::BoardNote(note) => {
                // Guarded by `board_seq` in the store, exactly as the catch-up is: an out-of-order
                // frame cannot undo a newer move. A frame moves the board's cursor, as a catch-up
                // page does — and as the answer to our own write does not. On the wall of the
                // family the note says — each family's cursor is its own.
                match self.named(note.family_id) {
                    Some(family) => self
                        .board
                        .in_family(Some(family))
                        .apply(note, SeqRoute::LiveFrame)?,
                    None => self.board.apply(note, SeqRoute::LiveFrame)?,
                };
                self.board_changed.emit(note);
            }

            ServerFrame::PackItem(item) => {
                // Under the `pack_seq` guard, exactly as a note is applied under its own; and the
                // store decides whether the cursor follows, because a frame may move it only once
                // this connection has caught up. A router with no pack drops the frame.
                if let Some(pack) = &self.pack {
                    match self.named(item.family_id) {
                        Some(family) => pack
                            .in_family(Some(family))
                            .apply(item, SeqRoute::LiveFrame)?,
                        None => pack.apply(item, SeqRoute::LiveFrame)?,
                    };
                    self.pack_changed.emit(item);
                }
            }

            ServerFrame::MemberJoined {
                family_id,
                user,
                password_reset,
            } => {
                // The owner's copy carries whether the newcomer is theirs to reset; the store keeps
                // what it holds when the frame says nothing (everybody else's copy), so a resync's
                // roster read first is never undone by it.
                chats.joined(user, self.named(*family_id), *password_reset)?;
                self.roster_changed.emit(&());
                if user.id == chats.reader()
                    && let Some(joined) = family_id
                {
                    self.joined_family.emit(joined);
                }
            }

            ServerFrame::MemberLeft { family_id, user_id } => {
                chats.left(*user_id, self.named(*family_id))?;
                self.roster_changed.emit(&());
                if *user_id == chats.reader()
                    && let Some(left) = family_id
                {
                    self.left_family.emit(left);
                }
            }

            ServerFrame::MemberDeleted { member, .. } => {
                // An account deleted leaves every family; one frame per family, and the tombstone
                // written twice is harmless.
                chats.deleted(member)?;
                self.roster_changed.emit(&());
            }

            ServerFrame::FamilyOwner { family_id, user_id } => {
                chats.set_owner(*user_id, self.named(*family_id))?;
                self.roster_changed.emit(&());
            }

            ServerFrame::FamilyPlan {
                family_id,
                plan,
                delete_at,
            } => {
                // The session's, not the cache's: a family's plan and deadline live on the family it
                // holds (docs/protocol.md, "Paid families").
                self.family_plan
                    .emit(&(*family_id, plan.clone(), delete_at.clone()));
            }

            ServerFrame::Invitation(invitation) => {
                self.invitation_changed.emit(invitation);
            }

            ServerFrame::InvitationRemoved { invitation_id } => {
                self.invitation_removed.emit(invitation_id);
            }

            ServerFrame::MemberBlocked { user_id, blocked } => {
                // Full current state, so an unblock is this same frame with `false`.
                chats.set_blocked(*user_id, *blocked)?;
                self.block_changed.emit(&(*user_id, *blocked));
            }

            ServerFrame::ChatUpdated(chat) => {
                // A group as it now stands, written whole: a member just added gets the chat this
                // way, before any list read (docs/protocol.md, "Groups").
                chats.upsert(chat)?;
                self.chat_changed.emit(&chat.id);
            }

            ServerFrame::ChatRemoved { chat_id } => {
                // Dropped with its messages and its queued sends — and only if it IS a group: the
                // family chat, a direct chat and an `ai` chat never leave this way, so a frame
                // naming one changes nothing. `GET /chats` stays the reconciler for whatever frame
                // was missed.
                if chats.remove(*chat_id)? {
                    self.chat_removed.emit(chat_id);
                    self.chat_changed.emit(chat_id);
                }
            }

            ServerFrame::AiDelta {
                chat_id,
                message_id,
                text,
            } => {
                // Deliberately NOT stored: the answer lands as `message_edited` when it is
                // finished, and a half-written body written into the cache would be what a
                // relaunch mid-answer drew for ever.
                self.ai_delta.emit(&(*chat_id, *message_id, text.clone()));
            }

            ServerFrame::AiError {
                chat_id,
                message_id,
                reason,
            } => {
                self.ai_stopped.emit(&(*chat_id, *message_id, *reason));
            }

            ServerFrame::CallOffer { .. }
            | ServerFrame::CallRinging { .. }
            | ServerFrame::CallAnswer { .. }
            | ServerFrame::CallIce { .. }
            | ServerFrame::CallEnd { .. }
            | ServerFrame::Error {
                call_id: Some(_), ..
            } => {
                self.call.emit(frame);
            }

            ServerFrame::Error {
                code,
                detail,
                client_msg_id: None,
                ..
            } => {
                // One that answers a send is the pipeline's, and it is waiting for it.
                self.refused.emit(&ApiError::coded(code, detail));
            }

            ServerFrame::Error { .. } | ServerFrame::Pong => {}
        }
        Ok(())
    }

    /// A cache whose stores were never told of a family — one from before several families, or a
    /// test — keeps its one roster, wall and pack unscoped, and a family named on a frame is not a
    /// reason to start keeping them per family.
    fn named(&self, family_id: Option<i64>) -> Option<i64> {
        self.board.family().and(family_id)
    }
}
