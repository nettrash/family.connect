using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic;

/// <summary>
/// The family chat's open polls, as they stand — a list of its own, because a decision nobody can find
/// is a decision nobody makes (docs/protocol.md, "Finding the open ones").
/// </summary>
/// <remarks>
/// <para>
/// <b>OLDEST FIRST, BY MESSAGE ID</b>, and never by <c>poll_seq</c>: ordering by the last change would
/// reshuffle the list under the reader every time somebody voted.
/// </para>
/// <para>
/// <b>A PLAIN READ.</b> The rows are kept here and NOT written into the cache: they are messages from
/// anywhere in the chat's history, and the cache holds pages. A poll frame lands in the cache alone,
/// so each row is drawn with whichever state is newer, its own or the cache's.
/// </para>
/// <para>
/// <b>RE-READ AFTER EVERY ACTION.</b> The answer to a vote would patch one row, but a poll its author
/// closed while the list was open belongs off it, and one request gets both right.
/// </para>
/// </remarks>
public sealed class OpenPollsModel
{
    private readonly ChatStore chats;
    private readonly ApiClient api;
    private readonly HashSet<long> revealed = [];
    private List<MessageDto> listed = [];

    public OpenPollsModel(long chatId, ChatStore chats, ApiClient api)
    {
        ChatId = chatId;
        this.chats = chats;
        this.api = api;
    }

    public long ChatId { get; }

    /// <summary>True once a read has answered; a failed one leaves what was there before.</summary>
    public bool Loaded { get; private set; }

    /// <summary>Why the last read failed, or null.</summary>
    public ApiError? Failure { get; private set; }

    public async Task<ApiError?> LoadAsync(CancellationToken ct = default)
    {
        var answer = await api.OpenPolls(ChatId, ct).ConfigureAwait(false);
        if (!answer.Ok || answer.Value is null)
        {
            Failure = answer.Error ?? ApiError.Transport("no answer");
            return Failure;
        }
        listed = [.. (answer.Value.Messages ?? []).Where(message => message.Poll is not null).OrderBy(message => message.Id)];
        Failure = null;
        Loaded = true;
        return null;
    }

    /// <summary>The rows, each with the newest state this device knows.</summary>
    public IReadOnlyList<MessageDto> Messages() => [.. listed.Select(Freshest)];

    /// <summary>
    /// A poll by somebody this reader has blocked is the hidden row here too, or the block would have a
    /// second door. Revealing it is this list's, for as long as the list lives.
    /// </summary>
    public bool IsHidden(MessageDto message) =>
        message.SenderId != chats.Reader && chats.IsBlocked(message.SenderId) && !revealed.Contains(message.Id);

    public void Reveal(long messageId) => revealed.Add(messageId);

    /// <summary>The badge: every poll the cache holds for this chat and every one listed, counted once.</summary>
    public int Unanswered() => Polls.Unanswered(chats.Polls(ChatId), listed, chats.Reader, chats.IsBlocked);

    public Task<ApiError?> VoteAsync(long messageId, long optionId, CancellationToken ct = default) =>
        ActAsync(messageId, row => PollVoting.TapAsync(api, chats, ChatId, row, optionId, ct), ct);

    public Task<ApiError?> CloseAsync(long messageId, CancellationToken ct = default) =>
        ActAsync(messageId, row => PollVoting.CloseAsync(api, chats, ChatId, row, ct), ct);

    private async Task<ApiError?> ActAsync(
        long messageId, Func<MessageDto, Task<(ApiError? Error, PollDto? Poll)>> act, CancellationToken ct)
    {
        if (Messages().FirstOrDefault(row => row.Id == messageId) is not { } row)
        {
            return null;
        }
        var (error, poll) = await act(row).ConfigureAwait(false);
        if (error is null && poll is null)
        {
            // Nothing was sent — a closed poll, somebody else's to close — so there is nothing to re-read.
            return null;
        }
        if (poll is not null)
        {
            // Kept even if the re-read below fails: the vote happened.
            listed = [.. listed.Select(held => held.Id == messageId && poll.PollSeq >= held.Poll!.PollSeq ? held with { Poll = poll } : held)];
        }
        var reread = await LoadAsync(ct).ConfigureAwait(false);
        return error ?? reread;
    }

    private MessageDto Freshest(MessageDto row) =>
        chats.Message(row.Id) is { Poll: { } cached } && cached.PollSeq > row.Poll!.PollSeq
            ? row with { Poll = cached }
            : row;
}
