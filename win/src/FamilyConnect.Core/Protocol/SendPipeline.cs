namespace FamilyConnect.Core.Protocol;

using FamilyConnect.Core.Store;

/// <summary>
/// Everything between "somebody pressed send" and "the server has it" (docs/protocol.md,
/// "Sending on an unreliable network").
/// </summary>
/// <remarks>
/// <para>
/// A SEND HAS THREE OUTCOMES: delivered, refused, and unknown. Unknown is by far the most common
/// on a bad network and is the one that must never be shown — so the row is written down BEFORE
/// anything moves, the frame is given <see cref="SendRules.AckDeadline"/> to be answered, an
/// unanswered frame falls back to <c>POST /chats/{id}/messages</c> with the SAME
/// <c>client_msg_id</c> (which is what makes the repeat safe), and only a terminal code marks the
/// row failed.
/// </para>
/// <para>
/// A RETURNING NETWORK IS A TRIGGER. Waiting for the next reconnect to come round on its own is
/// what makes a message sit unsent under a full signal bar, so the app flushes on connectivity,
/// on the socket connecting, on the window coming forward and on a timer — and the flush runs
/// FIRST in a resync rather than last.
/// </para>
/// </remarks>
public sealed class SendPipeline
{
    private readonly IFrameSender socket;
    private readonly OutboxStore outbox;
    private readonly ChatStore chats;
    private readonly Func<OutboxRow, CancellationToken, Task<ApiResult<MessageResponse>>> post;
    private readonly ReconnectBackoff backoff;
    private readonly Func<DateTimeOffset> now;
    private readonly Func<string> nextId;
    private readonly Func<TimeSpan, CancellationToken, Task> wait;

    /// <summary>Sends waiting for an <c>ack</c> or an <c>error</c>, by dedup key.</summary>
    private readonly Dictionary<string, TaskCompletionSource<Answered>> waiting = new();
    private readonly object gate = new();

    /// <summary>One send at a time per row: two passes would each upload the remainder.</summary>
    private readonly SemaphoreSlim flushing = new(1, 1);

    private readonly record struct Answered(MessageDto? Message, ApiError? Error);

    public SendPipeline(
        IFrameSender socket,
        OutboxStore outbox,
        ChatStore chats,
        Func<OutboxRow, CancellationToken, Task<ApiResult<MessageResponse>>> post,
        ReconnectBackoff? backoff = null,
        Func<DateTimeOffset>? now = null,
        Func<string>? nextId = null,
        Func<TimeSpan, CancellationToken, Task>? wait = null)
    {
        this.socket = socket;
        // The ack deadline is a real ten seconds in the app and a function in a test: a suite
        // that sleeps it is a suite nobody runs.
        this.wait = wait ?? Task.Delay;
        this.outbox = outbox;
        this.chats = chats;
        this.post = post;
        this.backoff = backoff ?? new ReconnectBackoff();
        this.now = now ?? (() => DateTimeOffset.UtcNow);
        // A REAL UUID: the server takes the dedup key as given, and a client that invents
        // something shorter finds out on a collision, in a family's chat.
        this.nextId = nextId ?? (() => Guid.NewGuid().ToString());
        socket.Frame += Heard;
    }

    /// <summary>The pipeline over a live client: the REST fallback is that client's own send.</summary>
    public SendPipeline(
        IFrameSender socket,
        OutboxStore outbox,
        ChatStore chats,
        ApiClient api,
        ReconnectBackoff? backoff = null,
        Func<DateTimeOffset>? now = null,
        Func<string>? nextId = null,
        Func<TimeSpan, CancellationToken, Task>? wait = null)
        : this(
            socket, outbox, chats,
            (row, ct) => api.SendMessage(
                row.ChatId, row.ClientMsgId, row.Body, row.ReplyToMessageId,
                row.AttachmentIds, row.PollOptions, row.Mentions, ct),
            backoff, now, nextId, wait)
    {
    }

    /// <summary>A row was refused for good, and the user has to be told.</summary>
    public event Action<OutboxRow, ApiError>? Refused;

    /// <summary>
    /// Write a send down and try it. Answers the row as queued — the bubble is drawn from this,
    /// before anything has moved.
    /// </summary>
    public OutboxRow Enqueue(
        long chatId,
        string body,
        long? replyToMessageId = null,
        IReadOnlyList<long>? attachmentIds = null,
        IReadOnlyList<string>? pendingFiles = null,
        IReadOnlyList<string>? pollOptions = null,
        IReadOnlyList<MentionDto>? mentions = null)
    {
        var row = new OutboxRow(
            ClientMsgId: nextId(),
            ChatId: chatId,
            Body: body,
            ReplyToMessageId: replyToMessageId,
            AttachmentIds: attachmentIds is { Count: > 0 } ? [.. attachmentIds] : null,
            PendingFiles: pendingFiles is { Count: > 0 } ? [.. pendingFiles] : null,
            PollOptions: pollOptions is { Count: > 0 } ? [.. pollOptions] : null,
            Mentions: mentions is { Count: > 0 } ? [.. mentions] : null,
            QueuedAt: now());
        outbox.Queue(row);
        return row;
    }

    /// <summary>
    /// Try everything that is due. Answers how many landed.
    /// </summary>
    /// <remarks>
    /// The trigger is recorded rather than acted on — every one of them means the same thing here
    /// ("try now") and naming them is how the app says why, in a log and in a test.
    /// </remarks>
    public async Task<int> FlushAsync(
        SendRules.FlushTrigger trigger = SendRules.FlushTrigger.Timer,
        CancellationToken ct = default)
    {
        // THE SAME SEND MUST NOT RUN TWICE AT ONCE: two passes over one row would each upload the
        // remainder, and the second would post a message the first had already posted.
        if (!await flushing.WaitAsync(0, ct).ConfigureAwait(false))
        {
            return 0;
        }
        try
        {
            var landed = 0;
            foreach (var row in outbox.Due(now()))
            {
                if (row.OwesUploads)
                {
                    // Until every upload has landed this row must never be posted: a message
                    // claiming no attachments is a text message, and the server would take it.
                    continue;
                }
                if (await SendAsync(row, ct).ConfigureAwait(false))
                {
                    landed++;
                }
            }
            return landed;
        }
        finally
        {
            flushing.Release();
        }
    }

    /// <summary>
    /// One row: the socket first, then REST for whatever the socket could not answer for.
    /// </summary>
    private async Task<bool> SendAsync(OutboxRow row, CancellationToken ct)
    {
        var answer = await OverSocket(row, ct).ConfigureAwait(false);
        // No answer at all is NOT a refusal — it says only that the frame was not acknowledged,
        // which is the case the REST path exists for.
        if (answer is { Message: { } acked })
        {
            return Delivered(acked);
        }
        if (answer is { Error: { } refused } && !refused.Transient)
        {
            return Failed(row, refused);
        }
        var rest = await post(row, ct).ConfigureAwait(false);
        if (rest.Ok && rest.Value is { } response)
        {
            return Delivered(response.Message);
        }
        return Failed(row, rest.Error ?? ApiError.Transport("no answer"));
    }

    /// <summary>
    /// The frame, and the ack deadline. A <c>send</c> with no answer inside the window is
    /// unanswered — and an <c>error</c> frame carrying a TRANSIENT code is treated exactly as no
    /// answer at all.
    /// </summary>
    private async Task<Answered?> OverSocket(OutboxRow row, CancellationToken ct)
    {
        if (!socket.IsConnected)
        {
            return null;
        }
        var pending = new TaskCompletionSource<Answered>(
            TaskCreationOptions.RunContinuationsAsynchronously);
        lock (gate)
        {
            waiting[row.ClientMsgId] = pending;
        }
        try
        {
            var frame = ClientFrames.Send(
                row.ChatId, row.ClientMsgId, row.Body, row.ReplyToMessageId,
                row.AttachmentIds, row.PollOptions, row.Mentions);
            if (!await socket.TrySend(frame, ct).ConfigureAwait(false))
            {
                // The write itself did not go — a socket whose connection is dead absorbs one, so
                // this is as much of an answer as the deadline expiring.
                return null;
            }
            // An answer already in hand beats the deadline, always: the server may answer before
            // the write even returns. `Task.WhenAny` over two ALREADY-COMPLETE tasks happens to
            // return the first of them, which would be this one — but "happens to" is not a rule
            // to rely on, and a mutation run cannot tell the difference, so it is written out.
            if (pending.Task.IsCompleted)
            {
                return await pending.Task.ConfigureAwait(false);
            }
            var settled = await Task.WhenAny(
                pending.Task, wait(SendRules.AckDeadline, ct)).ConfigureAwait(false);
            return settled == pending.Task ? await pending.Task.ConfigureAwait(false) : null;
        }
        catch (OperationCanceledException)
        {
            return null;
        }
        finally
        {
            lock (gate)
            {
                waiting.Remove(row.ClientMsgId);
            }
        }
    }

    /// <summary>Every frame the socket hands over; two of them answer a send.</summary>
    private void Heard(ServerFrame frame)
    {
        switch (frame)
        {
            case ServerFrame.Ack ack:
                Answer(ack.ClientMsgId, new Answered(ack.Value, null));
                break;
            case ServerFrame.Error error when error.ClientMsgId is { } id:
                Answer(id, new Answered(null, new ApiError(error.Code, error.Detail)));
                break;
            case ServerFrame.Message message when message.Value.ClientMsgId is { } id:
                // This device's own send, arriving as an ordinary message because the ack was
                // lost: still a delivery, and the row goes.
                Answer(id, new Answered(message.Value, null));
                break;
        }
    }

    private void Answer(string clientMsgId, Answered answered)
    {
        TaskCompletionSource<Answered>? pending;
        lock (gate)
        {
            waiting.TryGetValue(clientMsgId, out pending);
        }
        if (pending is not null)
        {
            pending.TrySetResult(answered);
            return;
        }
        // Nobody is waiting: an ack that arrived after the deadline, or after a relaunch. The row
        // is still here, and it is still delivered.
        if (answered.Message is { } message && outbox.Find(clientMsgId) is not null)
        {
            Delivered(message);
        }
    }

    private bool Delivered(MessageDto message)
    {
        chats.Apply(message);
        if (message.ClientMsgId is { } id)
        {
            outbox.Delivered(id);
        }
        return true;
    }

    private bool Failed(OutboxRow row, ApiError error)
    {
        var what = outbox.Failed(
            row.ClientMsgId, error, now(), backoff,
            // A client that has thrown its copy away has no way to recover an expired upload.
            holdsBytes: row.PendingFiles is not null || row.AttachmentIds is null);
        if (what == SendRules.Outcome.Failed)
        {
            // The one case the user is told about, and the only one with a retry affordance.
            Refused?.Invoke(row, error);
        }
        return false;
    }
}
