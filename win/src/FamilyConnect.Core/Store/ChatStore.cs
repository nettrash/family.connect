using FamilyConnect.Core.Protocol;
using Microsoft.Data.Sqlite;

namespace FamilyConnect.Core.Store;

/// <summary>
/// HOW a reaction or a poll state reached this client, which decides whether it may move the
/// chat's catch-up cursor (docs/protocol.md, "Best-effort delivery").
/// </summary>
/// <remarks>
/// <b>Only a live frame and a catch-up page may move either chat cursor.</b> A state that arrives
/// by any other route — embedded on a fetched <c>Message</c>, or in the answer to this client's
/// own reaction, vote or close — is EVIDENCE about one message and is applied under the
/// per-message guard alone. Advancing the cursor from evidence would step the chat past states for
/// messages this device has never seen, and `after_seq` can never look back.
/// </remarks>
public enum SeqRoute
{
    /// <summary>A frame off the socket.</summary>
    LiveFrame,

    /// <summary>A page of <c>…/reactions?after_seq=</c> or <c>…/polls?after_seq=</c>.</summary>
    CatchUpPage,

    /// <summary>Anything else: a fetched message, or the answer to our own write.</summary>
    Evidence,
}

/// <summary>
/// The chats and what was said in them, as this device holds them.
/// </summary>
/// <remarks>
/// <para>
/// Three protocol rules shape every write here. A MESSAGE IS IMMUTABLE except through two guarded
/// paths — an edit under <c>edit_seq</c> and reactions under <c>reaction_seq</c> — and both apply
/// the WHOLE message rather than the field they seem to be about, because the assistant's picture
/// answer arrives as an attachment added by an edit and a body-only merge draws nothing. The READ
/// MARKER is monotonic (<c>max(stored, received)</c>): a response still in flight while the reader
/// is reading must never walk it backwards. And the three <c>max_*_seq</c> cursors are HIGH-WATER
/// MARKS that never go down, so a chat whose polls retention has swept still reports one.
/// </para>
/// <para>
/// Apple counterpart: the SwiftData stores behind <c>ChatSyncCoordinator</c>. Android:
/// <c>ChatRepository</c> + the DAOs.
/// </para>
/// </remarks>
public sealed class ChatStore(Database database)
{
    // ---- the list --------------------------------------------------------

    /// <summary>
    /// What <c>GET /chats</c> answered IS the list — but a chat it does not carry is HIDDEN, not
    /// forgotten.
    /// </summary>
    /// <remarks>
    /// A direct chat with somebody the reader has blocked is not listed, for the blocker alone,
    /// and <b>comes back whole on unblock — nothing about it is deleted</b> (docs/protocol.md,
    /// "Blocking a member"). Deleting the row would take its messages with it, so an unblock would
    /// hand the reader an empty chat where a conversation used to be.
    /// </remarks>
    public void Replace(IReadOnlyList<ChatRowDto> rows)
    {
        using var transaction = database.Connection.BeginTransaction();
        var keep = rows.Select(row => row.Chat.Id).ToHashSet();
        foreach (var id in Ids(transaction).Where(id => !keep.Contains(id)))
        {
            using var hide = database.Connection.CreateCommand();
            hide.Transaction = transaction;
            hide.CommandText = "UPDATE chats SET listed = 0 WHERE chat_id = $id";
            hide.Parameters.AddWithValue("$id", id);
            hide.ExecuteNonQuery();
        }
        foreach (var row in rows)
        {
            Write(row, transaction);
            if (row.LastMessage is { } preview)
            {
                // A preview is a TRIMMED message: no reactions, no poll, no quote. It is stored so
                // the row can draw itself, and a fuller copy from a page overwrites it — which is
                // why the merge below is per-field and not a replace.
                Apply(preview, transaction, isPreview: true);
            }
        }
        transaction.Commit();
    }

    public IReadOnlyList<ChatRowDto> Chats()
    {
        var rows = new List<ChatRowDto>();
        using var command = database.Connection.CreateCommand();
        // Hidden chats are not on the list and count towards nothing: "there is nothing here for
        // a client to count" (docs/protocol.md, "Blocking a member").
        command.CommandText =
            "SELECT * FROM chats WHERE listed = 1 ORDER BY last_message_at DESC, chat_id";
        using var reader = command.ExecuteReader();
        while (reader.Read())
        {
            rows.Add(ReadChat(reader));
        }
        return [.. rows.Select(row => row with { LastMessage = Newest(row.Chat.Id) })];
    }

    /// <summary>
    /// One chat's row, listed or not: a blocked chat still has a read marker and cursors, and the
    /// thread it holds is still there to come back to.
    /// </summary>
    public ChatRowDto? Chat(long chatId)
    {
        using var command = database.Connection.CreateCommand();
        command.CommandText = "SELECT * FROM chats WHERE chat_id = $id";
        command.Parameters.AddWithValue("$id", chatId);
        using var reader = command.ExecuteReader();
        return reader.Read()
            ? ReadChat(reader) with { LastMessage = Newest(chatId) }
            : null;
    }

    /// <summary>Whether this chat is on the list — false while the reader has its peer blocked.</summary>
    public bool IsListed(long chatId)
    {
        using var command = database.Connection.CreateCommand();
        command.CommandText = "SELECT listed FROM chats WHERE chat_id = $id";
        command.Parameters.AddWithValue("$id", chatId);
        return command.ExecuteScalar() is { } value and not DBNull && Convert.ToInt64(value) == 1;
    }

    /// <summary>How many messages are unread across every chat — the badge.</summary>
    public int Unread() => Chats().Sum(row => row.UnreadCount);

    /// <summary>
    /// The caller's own read marker, applied MONOTONICALLY. The unread count is the other half of
    /// it and is recomputed here from what this device holds, which is the best it can do between
    /// reads: the server's own number arrives with the next list.
    /// </summary>
    public void MarkRead(long chatId, long lastReadMessageId)
    {
        using var command = database.Connection.CreateCommand();
        command.CommandText =
            """
            UPDATE chats
               SET last_read_message_id = MAX(last_read_message_id, $read),
                   unread_count = (
                       SELECT COUNT(*) FROM messages
                        WHERE messages.chat_id = chats.chat_id
                          AND messages.message_id > MAX(chats.last_read_message_id, $read)),
                   mentioned = CASE
                       WHEN $read >= (SELECT COALESCE(MAX(message_id), 0) FROM messages
                                       WHERE messages.chat_id = chats.chat_id
                                         AND mentions_json IS NOT NULL)
                       THEN 0 ELSE mentioned END
             WHERE chat_id = $chat
            """;
        command.Parameters.AddWithValue("$chat", chatId);
        command.Parameters.AddWithValue("$read", lastReadMessageId);
        command.ExecuteNonQuery();
    }

    /// <summary>
    /// The three catch-up cursors, as high-water marks. Absent on the wire means "nothing of that
    /// kind has ever happened here", which is 0 — and a 0 that arrives after a 124 is not a reset.
    /// </summary>
    public void Advance(long chatId, long? reactionSeq = null, long? editSeq = null, long? pollSeq = null)
    {
        using var command = database.Connection.CreateCommand();
        command.CommandText =
            """
            UPDATE chats
               SET max_reaction_seq = MAX(max_reaction_seq, $reaction),
                   max_edit_seq     = MAX(max_edit_seq, $edit),
                   max_poll_seq     = MAX(max_poll_seq, $poll)
             WHERE chat_id = $chat
            """;
        command.Parameters.AddWithValue("$chat", chatId);
        command.Parameters.AddWithValue("$reaction", reactionSeq ?? 0);
        command.Parameters.AddWithValue("$edit", editSeq ?? 0);
        command.Parameters.AddWithValue("$poll", pollSeq ?? 0);
        command.ExecuteNonQuery();
    }

    // ---- what was said ---------------------------------------------------

    /// <summary>
    /// One message, from a page, a frame or an ack.
    /// </summary>
    /// <returns>Whether anything changed — false for an answer this device had already applied.</returns>
    public bool Apply(MessageDto message)
    {
        using var transaction = database.Connection.BeginTransaction();
        var changed = Apply(message, transaction, isPreview: false);
        transaction.Commit();
        return changed;
    }

    /// <summary>A page of history, applied in order.</summary>
    public int Apply(IReadOnlyList<MessageDto> messages)
    {
        using var transaction = database.Connection.BeginTransaction();
        var changed = messages.Count(message => Apply(message, transaction, isPreview: false));
        transaction.Commit();
        return changed;
    }

    /// <summary>
    /// Reactions, from the frame or the catch-up: complete state, never a delta, and applied only
    /// when the seq is newer than the one held.
    /// </summary>
    public bool ApplyReactions(
        long messageId,
        long reactionSeq,
        ReactionDto[] reactions,
        SeqRoute route = SeqRoute.LiveFrame)
    {
        var held = Message(messageId);
        if (held is null || reactionSeq < (held.ReactionSeq ?? 0))
        {
            return false;
        }
        using var command = database.Connection.CreateCommand();
        command.CommandText =
            "UPDATE messages SET reaction_seq = $seq, reactions_json = $reactions WHERE message_id = $id";
        command.Parameters.AddWithValue("$seq", reactionSeq);
        command.Parameters.AddWithValue("$reactions", Wire.Encode(reactions));
        command.Parameters.AddWithValue("$id", messageId);
        command.ExecuteNonQuery();
        if (route != SeqRoute.Evidence)
        {
            Advance(held.ChatId, reactionSeq: reactionSeq);
        }
        return true;
    }

    /// <summary>A poll's state, under its own sequence.</summary>
    public bool ApplyPoll(long messageId, PollDto poll, SeqRoute route = SeqRoute.LiveFrame)
    {
        var held = Message(messageId);
        if (held is null || poll.PollSeq < (held.Poll?.PollSeq ?? 0))
        {
            return false;
        }
        using var command = database.Connection.CreateCommand();
        command.CommandText = "UPDATE messages SET poll_json = $poll WHERE message_id = $id";
        command.Parameters.AddWithValue("$poll", Wire.Encode(poll));
        command.Parameters.AddWithValue("$id", messageId);
        command.ExecuteNonQuery();
        if (route != SeqRoute.Evidence)
        {
            Advance(held.ChatId, pollSeq: poll.PollSeq);
        }
        return true;
    }

    public MessageDto? Message(long messageId)
    {
        using var command = database.Connection.CreateCommand();
        command.CommandText = "SELECT * FROM messages WHERE message_id = $id";
        command.Parameters.AddWithValue("$id", messageId);
        using var reader = command.ExecuteReader();
        return reader.Read() ? ReadMessage(reader) : null;
    }

    /// <summary>
    /// A page of one chat, NEWEST FIRST — the direction a chat view pages in, backwards from
    /// where the reader is.
    /// </summary>
    public IReadOnlyList<MessageDto> Messages(long chatId, long? beforeId = null, int limit = 50)
    {
        var messages = new List<MessageDto>();
        using var command = database.Connection.CreateCommand();
        command.CommandText = beforeId is null
            ? "SELECT * FROM messages WHERE chat_id = $chat ORDER BY message_id DESC LIMIT $limit"
            : "SELECT * FROM messages WHERE chat_id = $chat AND message_id < $before " +
              "ORDER BY message_id DESC LIMIT $limit";
        command.Parameters.AddWithValue("$chat", chatId);
        command.Parameters.AddWithValue("$limit", limit);
        if (beforeId is { } before)
        {
            command.Parameters.AddWithValue("$before", before);
        }
        using var reader = command.ExecuteReader();
        while (reader.Read())
        {
            messages.Add(ReadMessage(reader));
        }
        return messages;
    }

    /// <summary>
    /// One chain, oldest first: the root and everything that names it. A chain is read far more
    /// often than it is written, which is why the root is stored rather than walked.
    /// </summary>
    public IReadOnlyList<MessageDto> Thread(long rootId)
    {
        var messages = new List<MessageDto>();
        using var command = database.Connection.CreateCommand();
        command.CommandText =
            "SELECT * FROM messages WHERE message_id = $root OR thread_root_id = $root " +
            "ORDER BY message_id";
        command.Parameters.AddWithValue("$root", rootId);
        using var reader = command.ExecuteReader();
        while (reader.Read())
        {
            messages.Add(ReadMessage(reader));
        }
        return messages;
    }

    /// <summary>The newest message of a chat — the row's preview.</summary>
    public MessageDto? Newest(long chatId) => Messages(chatId, limit: 1).FirstOrDefault();

    // ---- who -------------------------------------------------------------

    /// <summary>
    /// The roster, replaced whole. A member who LEFT and an account that was DELETED both keep
    /// their row: their messages and notes keep their authors, and a name has to resolve for an
    /// old message.
    /// </summary>
    public void Replace(IReadOnlyList<MemberDto> members, IReadOnlyList<MemberDto>? former = null)
    {
        using var transaction = database.Connection.BeginTransaction();
        foreach (var member in members.Concat(former ?? []))
        {
            using var command = database.Connection.CreateCommand();
            command.Transaction = transaction;
            command.CommandText =
                """
                INSERT INTO members (user_id, username, display_name, avatar_version, owner,
                                     has_left, deleted, birthday_month, birthday_day)
                VALUES ($id, $username, $name, $avatar, $owner, $left, $deleted, $month, $day)
                ON CONFLICT(user_id) DO UPDATE SET
                    username = excluded.username, display_name = excluded.display_name,
                    avatar_version = excluded.avatar_version, owner = excluded.owner,
                    has_left = excluded.has_left, deleted = excluded.deleted,
                    birthday_month = excluded.birthday_month, birthday_day = excluded.birthday_day
                """;
            command.Parameters.AddWithValue("$id", member.Id);
            command.Parameters.AddWithValue("$username", member.Username);
            command.Parameters.AddWithValue("$name", member.DisplayName);
            command.Parameters.AddWithValue("$avatar", member.AvatarVersion);
            command.Parameters.AddWithValue("$owner", member.Owner ? 1 : 0);
            command.Parameters.AddWithValue("$left", member.HasLeft ? 1 : 0);
            command.Parameters.AddWithValue("$deleted", member.Deleted ? 1 : 0);
            command.Parameters.AddWithValue("$month", DBNull.Value);
            command.Parameters.AddWithValue("$day", DBNull.Value);
            command.ExecuteNonQuery();
        }
        transaction.Commit();
    }

    public IReadOnlyList<MemberDto> Members()
    {
        var members = new List<MemberDto>();
        using var command = database.Connection.CreateCommand();
        command.CommandText = "SELECT * FROM members ORDER BY display_name, user_id";
        using var reader = command.ExecuteReader();
        while (reader.Read())
        {
            members.Add(new MemberDto(
                Id: Convert.ToInt64(reader["user_id"]),
                Username: reader.GetString(reader.GetOrdinal("username")),
                DisplayName: reader.GetString(reader.GetOrdinal("display_name")),
                AvatarVersion: Convert.ToInt32(reader["avatar_version"]),
                Owner: Convert.ToInt64(reader["owner"]) == 1,
                HasLeft: Convert.ToInt64(reader["has_left"]) == 1,
                Deleted: Convert.ToInt64(reader["deleted"]) == 1));
        }
        return members;
    }

    /// <summary>
    /// The member behind an id, or null when this device has never heard of them — which happens,
    /// and is the caller's to name ("Deleted account" is a CLIENT's word, not the server's).
    /// </summary>
    public MemberDto? Member(long userId) => Members().FirstOrDefault(member => member.Id == userId);

    // ---- rows ------------------------------------------------------------

    private IReadOnlyList<long> Ids(SqliteTransaction transaction)
    {
        var ids = new List<long>();
        using var command = database.Connection.CreateCommand();
        command.Transaction = transaction;
        command.CommandText = "SELECT chat_id FROM chats";
        using var reader = command.ExecuteReader();
        while (reader.Read())
        {
            ids.Add(reader.GetInt64(0));
        }
        return ids;
    }

    private void Write(ChatRowDto row, SqliteTransaction transaction)
    {
        using var command = database.Connection.CreateCommand();
        command.Transaction = transaction;
        command.CommandText =
            """
            INSERT INTO chats (chat_id, kind, title, peer_user_id, unread_count,
                               last_read_message_id, mentioned,
                               max_reaction_seq, max_edit_seq, max_poll_seq, last_message_at)
            VALUES ($id, $kind, $title, $peer, $unread, $read, $mentioned,
                    $reaction, $edit, $poll, $at)
            ON CONFLICT(chat_id) DO UPDATE SET
                kind = excluded.kind, title = excluded.title, peer_user_id = excluded.peer_user_id,
                -- Back on the list: an unblock relists the chat it hid.
                listed = 1,
                unread_count = excluded.unread_count,
                -- MONOTONIC: a list read that crossed a `read` frame must not walk the marker
                -- back, for the same reason the server applies it that way.
                last_read_message_id = MAX(chats.last_read_message_id, excluded.last_read_message_id),
                mentioned = excluded.mentioned,
                -- HIGH-WATER MARKS: absent on the wire is 0 here, and a 0 never lowers one.
                max_reaction_seq = MAX(chats.max_reaction_seq, excluded.max_reaction_seq),
                max_edit_seq = MAX(chats.max_edit_seq, excluded.max_edit_seq),
                max_poll_seq = MAX(chats.max_poll_seq, excluded.max_poll_seq),
                last_message_at = MAX(chats.last_message_at, excluded.last_message_at)
            """;
        command.Parameters.AddWithValue("$id", row.Chat.Id);
        command.Parameters.AddWithValue("$kind", row.Chat.Kind);
        command.Parameters.AddWithValue("$title", row.Chat.Title);
        command.Parameters.AddWithValue("$peer", row.Chat.PeerUserId ?? (object)DBNull.Value);
        command.Parameters.AddWithValue("$unread", row.UnreadCount);
        command.Parameters.AddWithValue("$read", row.LastReadMessageId);
        // `mentioned` rides only when an unread message names the caller — absent, never false.
        command.Parameters.AddWithValue("$mentioned", row.Mentioned == true ? 1 : 0);
        command.Parameters.AddWithValue("$reaction", row.MaxReactionSeq ?? 0);
        command.Parameters.AddWithValue("$edit", row.MaxEditSeq ?? 0);
        command.Parameters.AddWithValue("$poll", row.MaxPollSeq ?? 0);
        command.Parameters.AddWithValue("$at",
            Times.Instant(row.LastMessage?.CreatedAt) ?? 0);
        command.ExecuteNonQuery();
    }

    /// <summary>
    /// One message, merged under the guards. A PREVIEW never overwrites a fuller copy's
    /// reactions, poll or quote — it does not carry them, and absent means "not included here",
    /// not "cleared".
    /// </summary>
    private bool Apply(MessageDto message, SqliteTransaction transaction, bool isPreview)
    {
        var held = Message(message.Id);
        if (held is not null)
        {
            // AN EDIT IS GUARDED: overwrite only when the incoming seq is at least the one held,
            // or a history page fetched before an edit and delivered after it quietly restores
            // the old text and two devices disagree about what was said.
            var incoming = message.EditSeq ?? 0;
            var stored = held.EditSeq ?? 0;
            if (incoming < stored)
            {
                return false;
            }
            if (isPreview && incoming == stored)
            {
                // Nothing new, and a preview carries less than what is held.
                return false;
            }
        }
        using var command = database.Connection.CreateCommand();
        command.Transaction = transaction;
        command.CommandText =
            """
            INSERT INTO messages (message_id, chat_id, sender_id, client_msg_id, body, created_at,
                                  edited_at, edit_seq, reply_to_id, thread_root_id, reply_count,
                                  reaction_seq, reactions_json, attachments_json, mentions_json,
                                  poll_json, call_json)
            VALUES ($id, $chat, $sender, $client, $body, $created,
                    $edited, $editSeq, $reply, $root, $replies,
                    $reactionSeq, $reactions, $attachments, $mentions, $poll, $call)
            ON CONFLICT(message_id) DO UPDATE SET
                body = excluded.body, edited_at = excluded.edited_at, edit_seq = excluded.edit_seq,
                reply_count = excluded.reply_count,
                -- These three are only overwritten by an answer that HAS them: a preview and a
                -- page carry different amounts of one message, and absent is not empty.
                reactions_json = COALESCE(excluded.reactions_json, messages.reactions_json),
                attachments_json = COALESCE(excluded.attachments_json, messages.attachments_json),
                poll_json = COALESCE(excluded.poll_json, messages.poll_json),
                reaction_seq = MAX(COALESCE(excluded.reaction_seq, 0),
                                   COALESCE(messages.reaction_seq, 0)),
                mentions_json = COALESCE(excluded.mentions_json, messages.mentions_json)
            """;
        command.Parameters.AddWithValue("$id", message.Id);
        command.Parameters.AddWithValue("$chat", message.ChatId);
        command.Parameters.AddWithValue("$sender", message.SenderId);
        command.Parameters.AddWithValue("$client", message.ClientMsgId ?? (object)DBNull.Value);
        command.Parameters.AddWithValue("$body", message.Body);
        command.Parameters.AddWithValue("$created", Times.Instant(message.CreatedAt) ?? 0);
        command.Parameters.AddWithValue("$edited",
            Times.Instant(message.EditedAt) ?? (object)DBNull.Value);
        command.Parameters.AddWithValue("$editSeq", message.EditSeq ?? (object)DBNull.Value);
        command.Parameters.AddWithValue("$reply",
            message.ReplyTo?.MessageId ?? (object)DBNull.Value);
        command.Parameters.AddWithValue("$root", message.ThreadRootId ?? (object)DBNull.Value);
        command.Parameters.AddWithValue("$replies", message.ReplyCount ?? (object)DBNull.Value);
        command.Parameters.AddWithValue("$reactionSeq",
            message.ReactionSeq ?? (object)DBNull.Value);
        command.Parameters.AddWithValue("$reactions",
            message.Reactions is null ? DBNull.Value : Wire.Encode(message.Reactions));
        command.Parameters.AddWithValue("$attachments",
            message.Attachments is null && message.Attachment is null
                ? DBNull.Value
                : Wire.Encode(message.Media));
        command.Parameters.AddWithValue("$mentions",
            message.Mentions is null ? DBNull.Value : Wire.Encode(message.Mentions));
        command.Parameters.AddWithValue("$poll",
            message.Poll is null ? DBNull.Value : Wire.Encode(message.Poll));
        command.Parameters.AddWithValue("$call",
            message.Call is null ? DBNull.Value : Wire.Encode(message.Call));
        command.ExecuteNonQuery();
        Touch(message, transaction);
        return true;
    }

    /// <summary>
    /// The chat's ordering and its unread count, after a message landed. An EDIT moves neither:
    /// it never re-notifies, never bumps a count and never moves the chat's ordering.
    /// </summary>
    private void Touch(MessageDto message, SqliteTransaction transaction)
    {
        using var command = database.Connection.CreateCommand();
        command.Transaction = transaction;
        command.CommandText =
            """
            UPDATE chats
               SET last_message_at = MAX(last_message_at, $at),
                   unread_count = (
                       SELECT COUNT(*) FROM messages
                        WHERE messages.chat_id = chats.chat_id
                          AND messages.message_id > chats.last_read_message_id),
                   mentioned = CASE WHEN $mentions AND $id > last_read_message_id
                                    THEN 1 ELSE mentioned END
             WHERE chat_id = $chat
            """;
        command.Parameters.AddWithValue("$chat", message.ChatId);
        command.Parameters.AddWithValue("$id", message.Id);
        command.Parameters.AddWithValue("$at",
            message.EditSeq is null ? Times.Instant(message.CreatedAt) ?? 0 : 0);
        command.Parameters.AddWithValue("$mentions", message.Mentions is { Length: > 0 } ? 1 : 0);
        command.ExecuteNonQuery();
    }

    private static ChatRowDto ReadChat(SqliteDataReader reader)
    {
        long? Optional(string column) =>
            Convert.ToInt64(reader[column]) is var value && value == 0 ? null : value;
        return new ChatRowDto(
            Chat: new ChatDto(
                Id: Convert.ToInt64(reader["chat_id"]),
                Kind: reader.GetString(reader.GetOrdinal("kind")),
                Title: reader.GetString(reader.GetOrdinal("title")),
                PeerUserId: reader["peer_user_id"] is DBNull
                    ? null
                    : Convert.ToInt64(reader["peer_user_id"])),
            LastMessage: null,
            UnreadCount: Convert.ToInt32(reader["unread_count"]),
            LastReadMessageId: Convert.ToInt64(reader["last_read_message_id"]),
            // Back to the wire's shape: absent rather than false, and absent rather than 0.
            MaxReactionSeq: Optional("max_reaction_seq"),
            MaxEditSeq: Optional("max_edit_seq"),
            MaxPollSeq: Optional("max_poll_seq"),
            Mentioned: Convert.ToInt64(reader["mentioned"]) == 1 ? true : null);
    }

    private static MessageDto ReadMessage(SqliteDataReader reader)
    {
        string? Text(string column) =>
            reader[column] is DBNull ? null : Convert.ToString(reader[column]);
        long? Number(string column) =>
            reader[column] is DBNull ? null : Convert.ToInt64(reader[column]);
        var attachments = Text("attachments_json") is { } media
            ? Wire.Decode<AttachmentDto[]>(media)
            : null;
        return new MessageDto(
            Id: Convert.ToInt64(reader["message_id"]),
            ChatId: Convert.ToInt64(reader["chat_id"]),
            SenderId: Convert.ToInt64(reader["sender_id"]),
            ClientMsgId: Text("client_msg_id"),
            Body: Text("body") ?? string.Empty,
            CreatedAt: Times.Rfc3339(Number("created_at")) ?? string.Empty,
            Reactions: Text("reactions_json") is { } reactions
                ? Wire.Decode<ReactionDto[]>(reactions)
                : null,
            ReactionSeq: Number("reaction_seq"),
            // The quote is a SNAPSHOT and not a reference; only its id survives here, and the
            // excerpt comes back with the message from the server.
            ReplyTo: Number("reply_to_id") is { } replyTo
                ? new ReplyToDto(replyTo, 0, string.Empty)
                : null,
            ThreadRootId: Number("thread_root_id"),
            ReplyCount: Number("reply_count") is { } replies ? (int)replies : null,
            Mentions: Text("mentions_json") is { } mentions
                ? Wire.Decode<MentionDto[]>(mentions)
                : null,
            EditedAt: Times.Rfc3339(Number("edited_at")),
            EditSeq: Number("edit_seq"),
            Attachments: attachments,
            Attachment: attachments is { Length: > 0 } ? attachments[0] : null,
            Poll: Text("poll_json") is { } poll ? Wire.Decode<PollDto>(poll) : null,
            Call: Text("call_json") is { } call ? Wire.Decode<CallRecordDto>(call) : null);
    }
}
