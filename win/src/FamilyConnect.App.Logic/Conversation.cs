using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic;

/// <summary>One message as the conversation draws it: the message, and what the reader may see.</summary>
public sealed record Bubble(
    MessageDto Message,
    /// <summary>
    /// Sent by somebody this reader has blocked, so it is drawn as the hidden row — a line saying
    /// something was said, and not what. Unlike the chat list's, THIS one can be revealed.
    /// </summary>
    bool Hidden,
    /// <summary>Whether the reader asked to see it after all.</summary>
    bool Revealed,
    /// <summary>Whether it is this reader's own.</summary>
    bool Mine)
{
    /// <summary>Whether the words are drawn: not hidden, or hidden and asked for.</summary>
    public bool Reads => !Hidden || Revealed;
}

/// <summary>
/// One open chat: the window of messages the reader is looking at, and the four things looking at
/// it does — page back, report the read, say you are typing, and send.
/// </summary>
/// <remarks>
/// <para>
/// <b>THE CACHE IS THE STATE.</b> This model holds no copy of the messages: <see cref="Bubbles"/>
/// is a query, so a live frame that lands in the store is drawn by the next read and there is no
/// second list to keep in step. What it DOES hold is what the cache has no business knowing — how
/// far back the reader has paged, which hidden bubbles they asked to see, and when this device
/// last said "typing".
/// </para>
/// <para>
/// <b>THE READ MARKER GOES FORWARD ONLY, AND ONLY WHEN THERE IS SOMETHING TO REPORT.</b> It is
/// monotonic per USER, shared across that person's devices, and the server keeps the maximum ever
/// reported — so a device that reports an older id achieves nothing but a request, and a device
/// that reports on every redraw costs the family's server a request per redraw.
/// </para>
/// <para>
/// <b>A REVEAL BELONGS TO THE CONVERSATION, NOT TO THE BUBBLE.</b> Held here, it outlives the
/// bubble being drawn again — which is the difference between tapping "show" and tapping it again
/// after every arriving message.
/// </para>
/// <para>
/// <b>TYPING IS MOMENTARY.</b> One frame every four seconds at most (the apps' own throttle,
/// inside the server's three-second one), and never while the socket is down: a typing frame is
/// not worth queueing, because by the time it went it would be a lie.
/// </para>
/// </remarks>
public sealed class ConversationModel
{
    /// <summary>How many messages a page is, here and on the wire.</summary>
    public const int Page = 50;

    /// <summary>At most one <c>typing</c> frame per chat per this long.</summary>
    public static readonly TimeSpan TypingEvery = TimeSpan.FromSeconds(4);

    private readonly ChatStore chats;
    private readonly ApiClient api;
    private readonly SendPipeline sending;
    private readonly IFrameSender socket;
    private readonly Func<DateTimeOffset> clock;
    private readonly HashSet<long> revealed = [];

    private int window = Page;
    private DateTimeOffset? saidTyping;
    private long reported;

    public ConversationModel(
        long chatId,
        ChatStore chats,
        ApiClient api,
        SendPipeline sending,
        IFrameSender socket,
        Func<DateTimeOffset>? clock = null)
    {
        ChatId = chatId;
        this.chats = chats;
        this.api = api;
        this.sending = sending;
        this.socket = socket;
        this.clock = clock ?? (() => DateTimeOffset.UtcNow);
        reported = chats.Chat(chatId)?.LastReadMessageId ?? 0;
    }

    public long ChatId { get; }

    /// <summary>Whether there may be older messages than the ones held: the button's enabled state.</summary>
    public bool MayHaveOlder { get; private set; } = true;

    /// <summary>The messages the reader is looking at, oldest first.</summary>
    public IReadOnlyList<Bubble> Bubbles()
    {
        var me = chats.Reader;
        var held = chats.Messages(ChatId, limit: window);
        var bubbles = new List<Bubble>(held.Count);
        // The store answers newest first, because that is the cheap end of the index; a
        // conversation reads the other way.
        for (var at = held.Count - 1; at >= 0; at--)
        {
            var message = held[at];
            var hidden = message.SenderId != me && chats.IsBlocked(message.SenderId);
            bubbles.Add(new Bubble(
                message, hidden, hidden && revealed.Contains(message.Id), message.SenderId == me));
        }
        return bubbles;
    }

    /// <summary>
    /// Open it: the newest page. Nothing is fetched when the cache already holds some of this
    /// chat — the resync's own catch-up brings it up to date, and a read here would race it.
    /// </summary>
    public async Task<ApiError?> OpenAsync(CancellationToken ct = default)
    {
        window = Page;
        if (chats.Messages(ChatId, limit: 1).Count > 0)
        {
            return null;
        }
        var page = await api.Messages(ChatId, limit: Page, ct: ct).ConfigureAwait(false);
        if (!page.Ok || page.Value is null)
        {
            return page.Error ?? ApiError.Transport("no answer");
        }
        var messages = page.Value.Messages ?? [];
        chats.Apply(messages);
        // A short page IS the beginning of the chat: there is nothing older to ask for.
        MayHaveOlder = messages.Length >= Page;
        return null;
    }

    /// <summary>
    /// Page BACKWARDS: older than the oldest message held. A history read must never move a
    /// catch-up cursor — it is older news by definition, and a cursor stepped back by it would
    /// re-read what has already been applied (or, worse, skip what has not).
    /// </summary>
    public async Task<ApiError?> OlderAsync(CancellationToken ct = default)
    {
        var held = chats.Messages(ChatId, limit: window);
        var oldest = held.Count > 0 ? held[^1].Id : (long?)null;
        if (oldest is null)
        {
            return await OpenAsync(ct).ConfigureAwait(false);
        }
        // Widen the window first: what the cache already holds below the oldest DRAWN message is
        // paged in without a request at all, which is what makes reopening a chat cheap.
        var deeper = chats.Messages(ChatId, beforeId: oldest, limit: Page);
        window += Page;
        if (deeper.Count > 0)
        {
            MayHaveOlder = true;
            return null;
        }
        var page = await api.Messages(ChatId, beforeId: oldest, limit: Page, ct: ct)
            .ConfigureAwait(false);
        if (!page.Ok || page.Value is null)
        {
            return page.Error ?? ApiError.Transport("no answer");
        }
        var messages = page.Value.Messages ?? [];
        chats.Apply(messages);
        MayHaveOlder = messages.Length >= Page;
        return null;
    }

    /// <summary>
    /// Report what the reader has read, at most once per newest message. Answers whether it
    /// reported anything — a device with nothing new to say makes no request.
    /// </summary>
    /// <param name="seen">
    /// The newest message the reader has actually BEEN SHOWN, for a window that is not at the
    /// bottom. Null means "everything held", which is what a window scrolled to the newest
    /// message means — and what a window scrolled up the history most certainly does not: a
    /// reader who opened a chat at an old position and never came down has not read the rest of
    /// it, and a client that said so would clear a badge nobody looked at. Clamped to what is
    /// held, so a window that names an id from somewhere else cannot report a message this
    /// device has never seen.
    /// </param>
    public async Task<bool> ReadAsync(long? seen = null, CancellationToken ct = default)
    {
        var held = chats.Newest(ChatId)?.Id ?? 0;
        var newest = seen is { } shown ? Math.Min(shown, held) : held;
        var marker = Math.Max(reported, chats.Chat(ChatId)?.LastReadMessageId ?? 0);
        if (newest <= marker)
        {
            return false;
        }
        // Locally FIRST: the badge goes out as the reader looks at it, whatever the network is
        // doing. The server keeps the maximum ever reported, so a repeat is harmless and a
        // failure costs nothing that the next report will not fix.
        chats.MarkRead(ChatId, newest);
        reported = newest;
        await api.MarkRead(ChatId, newest, ct).ConfigureAwait(false);
        return true;
    }

    /// <summary>
    /// Say that this reader is typing, if it is worth saying. Answers whether a frame went.
    /// </summary>
    public async Task<bool> TypingAsync(CancellationToken ct = default)
    {
        if (!socket.IsConnected)
        {
            return false;
        }
        var now = clock();
        if (saidTyping is { } last && now - last < TypingEvery)
        {
            return false;
        }
        saidTyping = now;
        return await socket.TrySend(ClientFrames.Typing(ChatId), ct).ConfigureAwait(false);
    }

    /// <summary>
    /// Send. The row is written down BEFORE anything is tried and the outbox owns it from there,
    /// so an interrupted send is a bubble that can be finished rather than nothing at all.
    /// </summary>
    public OutboxRow Send(
        string body,
        long? replyToMessageId = null,
        IReadOnlyList<long>? attachmentIds = null,
        IReadOnlyList<string>? pendingFiles = null,
        IReadOnlyList<string>? pollOptions = null,
        IReadOnlyList<MentionDto>? mentions = null) =>
        sending.Enqueue(
            ChatId, body, replyToMessageId, attachmentIds, pendingFiles, pollOptions, mentions);

    /// <summary>Show a hidden bubble after all. Per message, and it outlives the redraw.</summary>
    public void Reveal(long messageId) => revealed.Add(messageId);

    /// <summary>Hide it again.</summary>
    public void Hide(long messageId) => revealed.Remove(messageId);
}
