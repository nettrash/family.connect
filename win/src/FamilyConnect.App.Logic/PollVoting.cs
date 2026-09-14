using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic;

/// <summary>
/// A vote, a retraction and a close — for a poll in the conversation and for one on the open-polls
/// list, which may be a message this device has never paged back to (docs/protocol.md, "Polls").
/// </summary>
/// <remarks>
/// <para>
/// <b>THE TAP IS DECIDED HERE.</b> The server's vote is a state-set: re-PUTting the option already held
/// is a no-op, so a client that always PUT would draw a row as "yours, tap to clear" and then do
/// nothing when it was tapped. The option held means DELETE; any other means PUT.
/// </para>
/// <para>
/// <b>THE ANSWER IS EVIDENCE.</b> It is the poll's whole state, applied under the message's own
/// <c>poll_seq</c> guard and never moving the chat's catch-up cursor — the same rule a reaction's answer
/// keeps. Nothing is drawn before it arrives: an optimistic state would have no sequence the server
/// minted, and the next frame would be judged against it.
/// </para>
/// </remarks>
public static class PollVoting
{
    public static async Task<(ApiError? Error, PollDto? Poll)> TapAsync(
        ApiClient api, ChatStore chats, long chatId, MessageDto message, long optionId, CancellationToken ct = default)
    {
        if (!Polls.Votable(message) || message.Poll!.Options.All(option => option.Id != optionId))
        {
            return (null, null);
        }
        var answer = Polls.TapRetracts(message.Poll, optionId, chats.Reader)
            ? await api.Unvote(chatId, message.Id, ct).ConfigureAwait(false)
            : await api.Vote(chatId, message.Id, optionId, ct).ConfigureAwait(false);
        return Settle(chats, chatId, answer);
    }

    /// <summary>The author's alone — the family owner does not outrank authorship here — and one-way.</summary>
    public static async Task<(ApiError? Error, PollDto? Poll)> CloseAsync(
        ApiClient api, ChatStore chats, long chatId, MessageDto message, CancellationToken ct = default)
    {
        if (!MayClose(message, chats.Reader))
        {
            return (null, null);
        }
        return Settle(chats, chatId, await api.ClosePoll(chatId, message.Id, ct).ConfigureAwait(false));
    }

    /// <summary>Whether this reader is offered "Close poll": their own, numbered, still open.</summary>
    public static bool MayClose(MessageDto message, long reader) =>
        message.SenderId == reader && Polls.Votable(message);

    private static (ApiError? Error, PollDto? Poll) Settle(ChatStore chats, long chatId, ApiResult<MessagePollDto> answer)
    {
        if (!answer.Ok || answer.Value is null)
        {
            return (answer.Error ?? ApiError.Transport("no answer"), null);
        }
        chats.ApplyPoll(chatId, answer.Value.MessageId, answer.Value.Poll, SeqRoute.Evidence);
        return (null, answer.Value.Poll);
    }
}
