using FamilyConnect.Core.Protocol;
using Microsoft.Data.Sqlite;

namespace FamilyConnect.Core.Store;

/// <summary>
/// One message this device has been given and the server has not acknowledged.
/// </summary>
/// <remarks>
/// The <c>ClientMsgId</c> is the row's identity AND the wire's dedup key, which is what makes a
/// repeat safe. <c>NextAttemptAt</c> null means due now — a row somebody has just asked to retry.
/// <c>FailedCode</c> is set only on a TERMINAL refusal, and a row with one is the only kind the
/// user is told about (docs/protocol.md, "Sending on an unreliable network").
/// </remarks>
public sealed record OutboxRow(
    string ClientMsgId,
    long ChatId,
    string Body,
    long? ReplyToMessageId = null,
    long[]? AttachmentIds = null,
    string[]? PendingFiles = null,
    string[]? PollOptions = null,
    MentionDto[]? Mentions = null,
    DateTimeOffset QueuedAt = default,
    int Attempts = 0,
    DateTimeOffset? NextAttemptAt = null,
    string? FailedCode = null)
{
    /// <summary>Shown as failed, with a retry affordance — and nothing else is coming.</summary>
    public bool Failed => FailedCode is not null;

    /// <summary>
    /// Whether this row still owes an upload. Until every one has landed the row must NEVER be
    /// posted: a message claiming no attachments is a text message, and the server would accept it
    /// happily, leaving a delivered bubble with the pictures gone.
    /// </summary>
    public bool OwesUploads => PendingFiles is { Length: > 0 };
}

/// <summary>
/// What has not been said yet: the send queue, which is the one table holding something the
/// server has never seen.
/// </summary>
/// <remarks>
/// It survives the app closing, on purpose — that is the whole point of writing a send down before
/// the first byte moves, so an interrupted send is a bubble that can be finished rather than
/// nothing at all.
/// </remarks>
public sealed class OutboxStore(Database database)
{
    /// <summary>Write a send down — before the first byte of it moves.</summary>
    public void Queue(OutboxRow row)
    {
        using var command = database.Connection.CreateCommand();
        command.CommandText =
            """
            INSERT INTO outbox (client_msg_id, chat_id, body, reply_to_id, attachment_ids,
                                pending_files, poll_json, mentions_json, queued_at, attempts,
                                next_attempt_at, failed_code)
            VALUES ($id, $chat, $body, $reply, $attachments, $files, $poll, $mentions,
                    $queued, $attempts, $next, $failed)
            ON CONFLICT(client_msg_id) DO UPDATE SET
                body = excluded.body, attachment_ids = excluded.attachment_ids,
                pending_files = excluded.pending_files, attempts = excluded.attempts,
                next_attempt_at = excluded.next_attempt_at, failed_code = excluded.failed_code
            """;
        var queued = row.QueuedAt == default ? DateTimeOffset.UtcNow : row.QueuedAt;
        command.Parameters.AddWithValue("$id", row.ClientMsgId);
        command.Parameters.AddWithValue("$chat", row.ChatId);
        command.Parameters.AddWithValue("$body", row.Body);
        command.Parameters.AddWithValue("$reply", row.ReplyToMessageId ?? (object)DBNull.Value);
        command.Parameters.AddWithValue("$attachments",
            row.AttachmentIds is null ? DBNull.Value : Wire.Encode(row.AttachmentIds));
        command.Parameters.AddWithValue("$files",
            row.PendingFiles is null ? DBNull.Value : Wire.Encode(row.PendingFiles));
        command.Parameters.AddWithValue("$poll",
            row.PollOptions is null ? DBNull.Value : Wire.Encode(row.PollOptions));
        command.Parameters.AddWithValue("$mentions",
            row.Mentions is null ? DBNull.Value : Wire.Encode(row.Mentions));
        command.Parameters.AddWithValue("$queued", queued.ToUnixTimeMilliseconds());
        command.Parameters.AddWithValue("$attempts", row.Attempts);
        command.Parameters.AddWithValue("$next",
            row.NextAttemptAt?.ToUnixTimeMilliseconds() ?? (object)DBNull.Value);
        command.Parameters.AddWithValue("$failed", row.FailedCode ?? (object)DBNull.Value);
        command.ExecuteNonQuery();
    }

    /// <summary>Every queued row, oldest first — the order they were written in.</summary>
    public IReadOnlyList<OutboxRow> All() => Select("SELECT * FROM outbox ORDER BY queued_at, rowid");

    /// <summary>One chat's queue, for the pending bubbles under its last message.</summary>
    public IReadOnlyList<OutboxRow> ForChat(long chatId) =>
        Select("SELECT * FROM outbox WHERE chat_id = $chat ORDER BY queued_at, rowid",
            ("$chat", chatId));

    /// <summary>
    /// What to send now: the rows that are due and not failed, oldest first. A row that owes
    /// uploads is included — the caller finishes the uploads and then posts it, which is the one
    /// order that cannot deliver a message without its pictures.
    /// </summary>
    public IReadOnlyList<OutboxRow> Due(DateTimeOffset now) =>
        All().Where(row => !row.Failed && SendRules.IsDue(row.NextAttemptAt, now)).ToList();

    public OutboxRow? Find(string clientMsgId) =>
        Select("SELECT * FROM outbox WHERE client_msg_id = $id", ("$id", clientMsgId))
            .FirstOrDefault();

    /// <summary>
    /// The send landed: the row goes, and the message itself arrives through the ack or the feed.
    /// Idempotent — an ack that crossed a REST answer for the same id is ordinary.
    /// </summary>
    public bool Delivered(string clientMsgId)
    {
        using var command = database.Connection.CreateCommand();
        command.CommandText = "DELETE FROM outbox WHERE client_msg_id = $id";
        command.Parameters.AddWithValue("$id", clientMsgId);
        return command.ExecuteNonQuery() > 0;
    }

    /// <summary>
    /// One failed attempt, judged by <see cref="SendRules"/>: queued again with a delay, or shown
    /// failed. Answers what was decided, so the caller can tell the user only when it is final.
    /// </summary>
    public SendRules.Outcome Failed(
        string clientMsgId,
        ApiError error,
        DateTimeOffset now,
        ReconnectBackoff backoff,
        bool holdsBytes = true)
    {
        var row = Find(clientMsgId);
        if (row is null)
        {
            // Delivered by the other path while this attempt was in flight. Nothing to do — and
            // certainly not a failure to show.
            return SendRules.Outcome.Retry;
        }
        var attempts = row.Attempts + 1;
        var (what, next) = SendRules.Verdict(error, attempts, now, backoff, holdsBytes);
        using var command = database.Connection.CreateCommand();
        command.CommandText =
            "UPDATE outbox SET attempts = $attempts, next_attempt_at = $next, failed_code = $failed " +
            "WHERE client_msg_id = $id";
        command.Parameters.AddWithValue("$attempts", attempts);
        command.Parameters.AddWithValue("$next",
            next?.ToUnixTimeMilliseconds() ?? (object)DBNull.Value);
        command.Parameters.AddWithValue("$failed",
            what == SendRules.Outcome.Failed ? error.Code : (object)DBNull.Value);
        command.Parameters.AddWithValue("$id", clientMsgId);
        command.ExecuteNonQuery();
        return what;
    }

    /// <summary>
    /// The ids that DID land are kept and reused within the grace, so a retry pushes only the
    /// remainder — and the files still owed shrink as they go.
    /// </summary>
    public void Uploaded(string clientMsgId, long attachmentId, string file)
    {
        var row = Find(clientMsgId);
        if (row is null)
        {
            return;
        }
        var ids = (row.AttachmentIds ?? []).Append(attachmentId).Distinct().ToArray();
        var files = (row.PendingFiles ?? []).Where(pending => pending != file).ToArray();
        using var command = database.Connection.CreateCommand();
        command.CommandText =
            "UPDATE outbox SET attachment_ids = $ids, pending_files = $files WHERE client_msg_id = $id";
        command.Parameters.AddWithValue("$ids", Wire.Encode(ids));
        command.Parameters.AddWithValue("$files", Wire.Encode(files));
        command.Parameters.AddWithValue("$id", clientMsgId);
        command.ExecuteNonQuery();
    }

    /// <summary>
    /// Somebody pressed retry: the budget starts again and the row is due at once. That is what
    /// the affordance promises, and a row that waited out a backoff after being asked would be a
    /// button that did nothing.
    /// </summary>
    public void Retry(string clientMsgId)
    {
        using var command = database.Connection.CreateCommand();
        command.CommandText =
            "UPDATE outbox SET attempts = 0, next_attempt_at = NULL, failed_code = NULL " +
            "WHERE client_msg_id = $id";
        command.Parameters.AddWithValue("$id", clientMsgId);
        command.ExecuteNonQuery();
    }

    /// <summary>The user gave up on it. The bytes are the caller's to clean up.</summary>
    public bool Discard(string clientMsgId) => Delivered(clientMsgId);

    private IReadOnlyList<OutboxRow> Select(string sql, params (string Name, object Value)[] bind)
    {
        var rows = new List<OutboxRow>();
        using var command = database.Connection.CreateCommand();
        command.CommandText = sql;
        foreach (var (name, value) in bind)
        {
            command.Parameters.AddWithValue(name, value);
        }
        using var reader = command.ExecuteReader();
        while (reader.Read())
        {
            rows.Add(Read(reader));
        }
        return rows;
    }

    private static OutboxRow Read(SqliteDataReader reader)
    {
        string? Text(string column) =>
            reader[column] is DBNull ? null : Convert.ToString(reader[column]);
        long? Number(string column) =>
            reader[column] is DBNull ? null : Convert.ToInt64(reader[column]);
        return new OutboxRow(
            ClientMsgId: reader.GetString(reader.GetOrdinal("client_msg_id")),
            ChatId: Convert.ToInt64(reader["chat_id"]),
            Body: Convert.ToString(reader["body"]) ?? string.Empty,
            ReplyToMessageId: Number("reply_to_id"),
            AttachmentIds: Text("attachment_ids") is { } ids ? Wire.Decode<long[]>(ids) : null,
            PendingFiles: Text("pending_files") is { } files ? Wire.Decode<string[]>(files) : null,
            PollOptions: Text("poll_json") is { } poll ? Wire.Decode<string[]>(poll) : null,
            Mentions: Text("mentions_json") is { } mentions
                ? Wire.Decode<MentionDto[]>(mentions)
                : null,
            QueuedAt: DateTimeOffset.FromUnixTimeMilliseconds(Convert.ToInt64(reader["queued_at"])),
            Attempts: Convert.ToInt32(reader["attempts"]),
            NextAttemptAt: Number("next_attempt_at") is { } next
                ? DateTimeOffset.FromUnixTimeMilliseconds(next)
                : null,
            FailedCode: Text("failed_code"));
    }
}
