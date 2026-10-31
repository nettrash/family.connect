using FamilyConnect.Core.Store;

namespace FamilyConnect.Core.Protocol;

/// <summary>
/// What a live frame DOES (docs/protocol.md, "Server → client" and "Semantics"): the one place
/// the socket's frames meet the cache, so that a window only ever draws what is stored.
/// </summary>
/// <remarks>
/// <para>
/// The rules that are not obvious from the frame names, each of which cost somebody something in
/// one of the other ports:
/// </para>
/// <para>
/// <b>A `read` FRAME IS TWO DIFFERENT FACTS.</b> One naming YOURSELF is your own marker, arriving
/// because you read on another device — applied <c>max(stored, received)</c> and followed by a
/// recount. One naming somebody else is roster data, and is drawn as "seen" only in a DIRECT chat:
/// in the family chat — or a group — a per-member seen state over N members is a row of faces nobody asked for,
/// and a per-member ABSENCE is exactly what a blocker's suppressed inward frames would put on
/// screen. Applying the second as if it were the first clears your own unread count from somebody
/// else's reading — so who "you" are is asked of the store, which already has to know (a message
/// of the reader's own raises no unread count either).
/// </para>
/// <para>
/// <b>AN EDIT IS NOT A MESSAGE.</b> <c>message_edited</c> is a separate frame precisely so that it
/// bumps no unread count and raises no notification, and it carries the WHOLE message: the
/// assistant's picture answer arrives as an attachment added by exactly this frame, and a
/// body-only merge draws nothing. It also moves the chat's EDIT cursor, which no other route may.
/// </para>
/// <para>
/// <b>A FRAME MAY MOVE A CURSOR; EVIDENCE MAY NOT.</b> A live <c>reaction</c> or <c>poll</c> frame
/// is a complete statement about that chat's sequence, so the cursor follows it even when the
/// message it names is one this device does not hold — the state is dropped, the cursor is not.
/// </para>
/// <para>
/// <b>A FAMILY FRAME IS ABOUT THE FAMILY IT NAMES</b> (docs/protocol.md, "Several families"): <c>member_joined</c>,
/// <c>member_left</c>, <c>member_deleted</c> and <c>family_owner</c> are applied to the family their <c>family_id</c> names,
/// and <c>board_note</c> and <c>pack_item</c> to the wall and pack of the family their note and item say — never to "my
/// family". One naming the READER is a different fact: in a family (<see cref="JoinedFamily"/>), or out of one
/// (<see cref="LeftFamily"/>), which is the session's to act on.
/// </para>
/// </remarks>
public sealed class FrameRouter(ChatStore chats, BoardStore board, PackStore? pack = null)
{
    /// <summary>
    /// The READER is in a family — an approval, an accepted invitation, a family started on another device. Raised for
    /// every <c>member_joined</c> naming them; whether the family is one this device already holds is the session's to judge,
    /// and a new one means re-reading <c>GET /me</c> and that family.
    /// </summary>
    public event Action<long>? JoinedFamily;

    /// <summary>
    /// The READER is out of a family: left on another device, or removed. That family's chats, wall, pack, roster and cursors
    /// are to go — and only that family's (<c>Database.DropFamily</c>).
    /// </summary>
    public event Action<long>? LeftFamily;

    /// <summary>An invitation to the reader, or by the reader's family, appeared or changed.</summary>
    public event Action<InvitationDto>? InvitationChanged;

    /// <summary>An invitation lapsed.</summary>
    public event Action<long>? InvitationRemoved;

    /// <summary>
    /// A paid family's plan or deadline changed (docs/protocol.md, "Paid families"): the session's to apply to the family the
    /// frame names, which is not necessarily the one shown. Nothing about it is in the cache.
    /// </summary>
    public event Action<ServerFrame.FamilyPlan>? FamilyPlanChanged;

    /// <summary>A message that is NEW — the only frame that may raise a notification.</summary>
    /// <remarks>
    /// It fires for this device's own sends too: the sender's OTHER connections receive
    /// <c>message</c> like everybody else, and a REST send fans out to all of them including the
    /// sender's own. Whose message it is is the subscriber's to judge — it holds the signed-in id
    /// and the block list, and both matter for what it does next.
    /// </remarks>
    public event Action<MessageDto>? Arrived;

    /// <summary>A message whose words changed. Never a notification, never an unread count.</summary>
    public event Action<MessageDto>? Edited;

    /// <summary>Something about this chat changed and a drawn list or thread is now stale.</summary>
    public event Action<long>? ChatChanged;

    /// <summary>Somebody is typing in that chat, as of now. Never persisted.</summary>
    public event Action<long, long>? Typing;

    /// <summary>A peer's read marker moved in a DIRECT chat — the only place it is drawn.</summary>
    public event Action<long, long, long>? PeerRead;

    /// <summary>The wall changed: a note created, edited, moved, or taken down.</summary>
    public event Action<NoteDto>? BoardChanged;

    /// <summary>
    /// The family's sticker pack changed: an item added, or one removed. Never a notification and
    /// never an unread count — and not filtered by blocks: an item is a picture the family keeps,
    /// not something a person said (docs/protocol.md, "What does not touch the pack").
    /// </summary>
    public event Action<PackItemDto>? PackChanged;

    /// <summary>
    /// A GROUP this reader is no longer in has been dropped from the cache — left, removed from, or deleted
    /// (docs/protocol.md, "Groups"). <see cref="ChatChanged"/> fires for it too; this is for whoever has it OPEN.
    /// </summary>
    public event Action<long>? ChatRemoved;

    /// <summary>The roster changed — a join, a leave, a deleted account, a new owner.</summary>
    public event Action? RosterChanged;

    /// <summary>This reader's own block list changed, and with it what they can see.</summary>
    public event Action<long, bool>? BlockChanged;

    /// <summary>The assistant, mid-reply: text to append to that message as it is drawn.</summary>
    public event Action<long, long, string>? AiDelta;

    /// <summary>
    /// The assistant stopped early, and the half-written answer is all there is — with the reason,
    /// when the frame gave one this client knows, and when a used-up allowance renews.
    /// </summary>
    public event Action<long, long, AiErrorReason?, DateTimeOffset?>? AiStopped;

    /// <summary>
    /// A call frame, passed on whole. Signalling is a conversation with state of its own and no
    /// business in a cache — the window's call layer owns it.
    /// </summary>
    public event Action<ServerFrame>? Call;

    /// <summary>A refusal that answers nothing this device is waiting for.</summary>
    public event Action<ApiError>? Refused;

    /// <summary>One frame, applied. Unknown frames never reach here: the parser drops them.</summary>
    public void Hear(ServerFrame frame)
    {
        switch (frame)
        {
            case ServerFrame.Message message:
                // A LIVE frame, which is the only route that may raise an unread count.
                chats.Apply(message.Value, SeqRoute.LiveFrame);
                Arrived?.Invoke(message.Value);
                ChatChanged?.Invoke(message.Value.ChatId);
                break;

            case ServerFrame.Ack ack:
                // The send pipeline owns the row; the cache still wants the message, and
                // applying it twice is applying it once. It counts against nobody: an ack answers
                // this reader's own send.
                chats.Apply(ack.Value, SeqRoute.LiveFrame);
                ChatChanged?.Invoke(ack.Value.ChatId);
                break;

            case ServerFrame.MessageEdited edited:
                // Out of sequence: the edited message may be one this device never held.
                chats.Apply(edited.Value, SeqRoute.Evidence, inSequence: false);
                if (edited.Value.EditSeq is { } editSeq)
                {
                    chats.Advance(edited.Value.ChatId, editSeq: editSeq);
                }
                Edited?.Invoke(edited.Value);
                ChatChanged?.Invoke(edited.Value.ChatId);
                break;

            case ServerFrame.Read read when read.UserId == chats.Reader:
                // YOUR OWN marker, from your other device. Monotonic, and the count follows it.
                chats.MarkRead(read.ChatId, read.LastReadMessageId);
                ChatChanged?.Invoke(read.ChatId);
                break;

            case ServerFrame.Read read:
                // SOMEBODY ELSE'S, and it is drawn in a direct chat only.
                if (chats.Chat(read.ChatId)?.Chat.Kind == "direct")
                {
                    PeerRead?.Invoke(read.ChatId, read.UserId, read.LastReadMessageId);
                }
                break;

            case ServerFrame.Typing typing:
                Typing?.Invoke(typing.ChatId, typing.UserId);
                break;

            case ServerFrame.Reactions reactions:
                // The cursor follows the FRAME, not the message: a state for a message this
                // device does not hold is dropped, and the sequence still happened. That rule
                // lives in the store, which is why the route is passed rather than acted on.
                chats.ApplyReactions(
                    reactions.ChatId, reactions.MessageId, reactions.ReactionSeq,
                    reactions.Value, SeqRoute.LiveFrame);
                ChatChanged?.Invoke(reactions.ChatId);
                break;

            case ServerFrame.Poll poll:
                chats.ApplyPoll(poll.ChatId, poll.MessageId, poll.Value, SeqRoute.LiveFrame);
                ChatChanged?.Invoke(poll.ChatId);
                break;

            case ServerFrame.BoardNote note:
                // Guarded by `board_seq` in the store, exactly as the catch-up is: an
                // out-of-order frame cannot undo a newer move. A frame moves the board's cursor,
                // as a catch-up page does — and as the answer to our own write does not. On the wall of the family the note
                // says — each family's cursor is its own.
                WallOf(note.Note.FamilyId).Apply(note.Note, SeqRoute.LiveFrame);
                BoardChanged?.Invoke(note.Note);
                break;

            case ServerFrame.PackItem item when pack is not null:
                // Under the `pack_seq` guard, exactly as a note is applied under its own; and the
                // store decides whether the cursor follows, because a frame may move it only once
                // this connection has caught up.
                PackOf(pack, item.Item.FamilyId).Apply(item.Item, SeqRoute.LiveFrame);
                PackChanged?.Invoke(item.Item);
                break;

            case ServerFrame.MemberJoined joined:
                // The owner's copy carries whether the newcomer is theirs to reset; the store keeps what it holds when the
                // frame says nothing (everybody else's copy), so a resync's roster read first is never undone by it.
                chats.Joined(joined.User, Named(joined.FamilyId), joined.PasswordReset);
                RosterChanged?.Invoke();
                if (joined.User.Id == chats.Reader && joined.FamilyId is { } joinedFamily)
                {
                    JoinedFamily?.Invoke(joinedFamily);
                }
                break;

            case ServerFrame.MemberLeft left:
                chats.Left(left.UserId, Named(left.FamilyId));
                RosterChanged?.Invoke();
                if (left.UserId == chats.Reader && left.FamilyId is { } leftFamily)
                {
                    LeftFamily?.Invoke(leftFamily);
                }
                break;

            case ServerFrame.MemberDeleted deleted:
                // An account deleted leaves every family; one frame per family, and the tombstone written twice is harmless.
                chats.Deleted(deleted.Member);
                RosterChanged?.Invoke();
                break;

            case ServerFrame.FamilyOwner owner:
                chats.SetOwner(owner.UserId, Named(owner.FamilyId));
                RosterChanged?.Invoke();
                break;

            case ServerFrame.Invitation invitation:
                InvitationChanged?.Invoke(invitation.Value);
                break;

            case ServerFrame.InvitationRemoved lapsed:
                InvitationRemoved?.Invoke(lapsed.InvitationId);
                break;

            case ServerFrame.FamilyPlan plan:
                FamilyPlanChanged?.Invoke(plan);
                break;

            case ServerFrame.MemberBlocked blocked:
                // Full current state, so an unblock is this same frame with `false`.
                chats.SetBlocked(blocked.UserId, blocked.Blocked);
                BlockChanged?.Invoke(blocked.UserId, blocked.Blocked);
                break;

            case ServerFrame.ChatUpdated updated:
                // A group as it now stands, written whole: a member just added gets the chat this way, before any list
                // read (docs/protocol.md, "Groups").
                chats.Upsert(updated.Chat);
                ChatChanged?.Invoke(updated.Chat.Id);
                break;

            case ServerFrame.ChatRemoved removed:
                // Dropped with its messages and its queued sends — and only if it IS a group: the family chat, a direct
                // chat and an `ai` chat never leave this way, so a frame naming one changes nothing. `GET /chats` stays
                // the reconciler for whatever frame was missed.
                if (chats.Remove(removed.ChatId))
                {
                    ChatRemoved?.Invoke(removed.ChatId);
                    ChatChanged?.Invoke(removed.ChatId);
                }
                break;

            case ServerFrame.AiDelta delta:
                // Deliberately NOT stored: the answer lands as `message_edited` when it is
                // finished, and a half-written body written into the cache would be what a
                // relaunch mid-answer drew for ever.
                AiDelta?.Invoke(delta.ChatId, delta.MessageId, delta.Text);
                break;

            case ServerFrame.AiError stopped:
                AiStopped?.Invoke(stopped.ChatId, stopped.MessageId, stopped.Reason, stopped.RenewsAt);
                break;

            case ServerFrame.CallOffer:
            case ServerFrame.CallRinging:
            case ServerFrame.CallAnswer:
            case ServerFrame.CallIce:
            case ServerFrame.CallEnd:
                Call?.Invoke(frame);
                break;

            case ServerFrame.Error error when error.CallId is not null:
                Call?.Invoke(frame);
                break;

            case ServerFrame.Error error when error.ClientMsgId is null:
                // One that answers a send is the pipeline's, and it is waiting for it.
                Refused?.Invoke(new ApiError(error.Code, error.Detail));
                break;

            case ServerFrame.Pong:
            default:
                break;
        }
    }

    // A cache whose stores were never told of a family — one from before several families, or a test — keeps its one
    // roster, wall and pack unscoped, and a family named on a frame is not a reason to start keeping them per family.
    private long? Named(long? familyId) => board.Family is null ? null : familyId;

    private BoardStore WallOf(long? familyId) => Named(familyId) is { } family ? board.In(family) : board;

    private PackStore PackOf(PackStore held, long? familyId) => Named(familyId) is { } family ? held.In(family) : held;
}
