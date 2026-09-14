using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic;

/// <summary>
/// One chain of replies on a surface of its own: the root at the top, the replies below it in order, and a composer whose
/// every send answers the root (docs/protocol.md, "Threads"; web <c>thread_panel.rs</c>, ios <c>ThreadView</c>).
/// </summary>
/// <remarks>
/// <para>
/// <b>OPENED AT ONCE ON WHAT IS HELD, THEN READ.</b> A reply names its root, so a chain opened from one is rooted before
/// anything is asked; the read's first page answers with the ROOT first, whichever message was named, and re-roots it.
/// </para>
/// <para>
/// <b>THE READ STAYS OUT OF THE CACHE.</b> The cache holds pages, and a root older than them or a reply newer would move
/// where the next page starts. Its rows are kept here, and the cache takes only its copies of rows it already holds —
/// their recomputed count and any edit (<see cref="ChatStore.Refresh"/>).
/// </para>
/// <para>
/// <b>EACH COPY OF THE ROOT COUNTS A REPLY ONCE.</b> The cache raises its own copy on the routes that count; this surface
/// raises its copy the first time IT sees a reply — whatever brought it — and a row both hold is drawn from the fresher.
/// </para>
/// </remarks>
public sealed class ThreadModel
{
    /// <summary>How many rows one thread read asks for.</summary>
    public const int Page = 50;

    private readonly ChatStore chats;
    private readonly ApiClient api;
    private readonly OutboxStore? outbox;
    private readonly long named;
    private readonly HashSet<long> revealed = [];
    private readonly Dictionary<long, MessageDto> read = [];

    public ThreadModel(long chatId, long messageId, ChatStore chats, ApiClient api, OutboxStore? outbox = null)
    {
        ChatId = chatId;
        named = messageId;
        this.chats = chats;
        this.api = api;
        this.outbox = outbox;
        // A reply names its root; anything else is one.
        RootId = chats.Message(messageId) is { ThreadRootId: { } root } ? root : messageId;
        // What the cache holds of the chain is where the surface starts — not news, so none of it raises the root.
        foreach (var message in chats.Thread(RootId).Where(message => message.ChatId == chatId))
        {
            read[message.Id] = message;
        }
    }

    public long ChatId { get; }

    /// <summary>The chain's root — decided from what is held, and settled by the read's first page.</summary>
    public long RootId { get; private set; }

    /// <summary>True once the whole chain has been read.</summary>
    public bool Loaded { get; private set; }

    /// <summary>Why the last read stopped, or null. What was drawn before stays drawn.</summary>
    public ApiError? Failure { get; private set; }

    /// <summary>The chain, page by page — root first, then every reply, <c>after_id</c> looped until a short page.</summary>
    public async Task<ApiError?> LoadAsync(CancellationToken ct = default)
    {
        long? after = null;
        while (true)
        {
            var answer = await api.Thread(ChatId, named, after, Page, ct).ConfigureAwait(false);
            if (!answer.Ok || answer.Value is null)
            {
                Failure = answer.Error ?? ApiError.Transport("no answer");
                return Failure;
            }
            var page = answer.Value.Messages ?? [];
            if (after is null && page.Length > 0)
            {
                // The ROOT comes first, whichever message was named.
                RootId = page[0].Id;
            }
            chats.Refresh(page);
            foreach (var message in page)
            {
                read[message.Id] = Merge(read.GetValueOrDefault(message.Id), message);
            }
            if (page.Length < Page)
            {
                break;
            }
            // A cursor that cannot move ends the loop, whatever the server says about the page being full.
            var next = page.Max(message => message.Id);
            if (after is { } previous && next <= previous)
            {
                break;
            }
            after = next;
        }
        Failure = null;
        Loaded = true;
        return null;
    }

    /// <summary>
    /// The rows, oldest first, each drawn from the fresher of this surface's copy and the cache's — folding in first
    /// whatever the cache gained since: a live reply, this reader's own delivered one, a catch-up.
    /// </summary>
    public IReadOnlyList<Bubble> Bubbles()
    {
        var held = chats.Thread(RootId).Where(message => message.ChatId == ChatId).ToDictionary(message => message.Id);
        foreach (var message in held.Values.OrderBy(message => message.Id))
        {
            if (!read.ContainsKey(message.Id))
            {
                Heard(message);
            }
        }
        var me = chats.Reader;
        return
        [
            .. read.Values
                .OrderBy(message => message.Id)
                .Select(copy => held.TryGetValue(copy.Id, out var cached) ? Merge(copy, cached) : copy)
                .Select(message =>
                {
                    var hidden = message.SenderId != me && chats.IsBlocked(message.SenderId);
                    return new Bubble(message, hidden, hidden && revealed.Contains(message.Id), message.SenderId == me);
                }),
        ];
    }

    /// <summary>
    /// A message this surface has not seen: on it if it belongs — the root, or a reply rooted at it — and a reply new
    /// HERE raises this surface's copy of the root.
    /// </summary>
    /// <returns>Whether it belonged.</returns>
    public bool Heard(MessageDto message)
    {
        if (message.ChatId != ChatId || (message.Id != RootId && message.ThreadRootId != RootId))
        {
            return false;
        }
        var isNew = !read.ContainsKey(message.Id);
        read[message.Id] = Merge(read.GetValueOrDefault(message.Id), message);
        if (isNew && message.ThreadRootId == RootId && read.TryGetValue(RootId, out var root))
        {
            read[RootId] = root with { ReplyCount = (root.ReplyCount ?? 0) + 1 };
        }
        return true;
    }

    /// <summary>How many replies the surface draws: every row but the root.</summary>
    public int Replies(IReadOnlyList<Bubble> bubbles) => bubbles.Count(bubble => bubble.Message.Id != RootId);

    /// <summary>This reader's replies still on their way that answer the chain — the root, or any row of it.</summary>
    public IReadOnlyList<OutboxRow> Pending()
    {
        var chain = read.Keys.Concat(chats.Thread(RootId).Select(message => message.Id)).Append(RootId).ToHashSet();
        return [.. (outbox?.ForChat(ChatId) ?? []).Where(row => row.ReplyToMessageId is { } answered && chain.Contains(answered))];
    }

    /// <summary>Show a hidden row after all — this surface's own, for as long as it lives.</summary>
    public void Reveal(long messageId) => revealed.Add(messageId);

    public void Hide(long messageId) => revealed.Remove(messageId);

    /// <summary>
    /// React on a row of the chain, held by the cache or not: decided against the freshest copy, and the answer applied to
    /// both — the cache as evidence, this surface's copy under its sequence.
    /// </summary>
    public async Task<ApiError?> ReactAsync(long messageId, string emoji, CancellationToken ct = default)
    {
        if ((chats.Message(messageId) ?? read.GetValueOrDefault(messageId)) is not { } copy)
        {
            return null;
        }
        var toggle = Reactions.Toggle(copy.Reactions ?? [], chats.Reader, emoji);
        var answer = toggle.Removing
            ? await api.Unreact(ChatId, messageId, ct).ConfigureAwait(false)
            : await api.React(ChatId, messageId, emoji, ct).ConfigureAwait(false);
        if (!answer.Ok || answer.Value is null)
        {
            return answer.Error ?? ApiError.Transport("no answer");
        }
        chats.ApplyReactions(ChatId, messageId, answer.Value.ReactionSeq, answer.Value.Reactions, SeqRoute.Evidence);
        if (read.TryGetValue(messageId, out var kept) && answer.Value.ReactionSeq >= (kept.ReactionSeq ?? 0))
        {
            read[messageId] = kept with { Reactions = answer.Value.Reactions, ReactionSeq = answer.Value.ReactionSeq };
        }
        return null;
    }

    /// <summary>
    /// Two copies of one row: the words from the one with the newer edit — an older copy never undoes an edit, though it
    /// still brings the true count — and the reactions from the one with the newer reaction sequence.
    /// </summary>
    private static MessageDto Merge(MessageDto? kept, MessageDto incoming)
    {
        if (kept is null)
        {
            return incoming;
        }
        var merged = (incoming.EditSeq ?? 0) >= (kept.EditSeq ?? 0) ? incoming : kept with { ReplyCount = incoming.ReplyCount };
        return (kept.ReactionSeq ?? 0) > (merged.ReactionSeq ?? 0)
            ? merged with { Reactions = kept.Reactions, ReactionSeq = kept.ReactionSeq }
            : merged;
    }
}
