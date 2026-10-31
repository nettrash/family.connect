using System.Globalization;
using System.Text;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>
/// The assistant's answers while they are being written: the text streamed so far, and the ones that stopped
/// early (docs/protocol.md, <c>ai_delta</c> and <c>ai_error</c>).
/// </summary>
/// <remarks>
/// <para>
/// <b>COSMETIC, AND NEVER IN THE CACHE.</b> The finished answer arrives as <c>message_edited</c> and replaces all
/// of it, so streamed text lives here and is drawn after the held body — and a delta for a row already finished
/// (it has an <c>edit_seq</c>), or for a row this device does not hold, is late and ignored.
/// </para>
/// <para>
/// <b>A FINISHED ANSWER IS A FINISHED ANSWER</b>, whatever went before it: an edit clears both the text and the
/// failure, and a row that carries an <c>edit_seq</c> is never drawn as failed, however it reached the cache.
/// </para>
/// <para>
/// <b>A FAILURE REMEMBERS ITS SENTENCE</b> (docs/protocol.md, <c>ai_error</c>'s <c>reason</c>): a refusal by the
/// provider's own filter says so, a family whose month is used up says when it renews (docs/protocol.md, "Paid families"),
/// and anything else says "ask again" — for exactly as long as the row is failed, so a redraw never turns one sentence into
/// the other.
/// </para>
/// </remarks>
public sealed class AssistantAnswers
{
    private readonly object gate = new();
    private readonly Dictionary<long, StringBuilder> written = [];
    // Each stopped answer, with the reason its frame gave — null for every failure the protocol has no word for — and, for a
    // used-up allowance, when it renews.
    private readonly Dictionary<long, (AiErrorReason? Reason, DateTimeOffset? RenewsAt)> stopped = [];
    private int version;

    /// <summary>Something about a chat's answers changed (raised on whatever thread the frame arrived on).</summary>
    public event Action<long>? Changed;

    /// <summary>Goes up on every change — part of what a conversation compares before drawing again.</summary>
    public int Version => Volatile.Read(ref version);

    public void Delta(long chatId, long messageId, string text, MessageDto? held)
    {
        if (held is null || held.EditSeq is not null || text.Length == 0)
        {
            return;
        }
        lock (gate)
        {
            if (!written.TryGetValue(messageId, out var so))
            {
                written[messageId] = so = new StringBuilder();
            }
            so.Append(text);
        }
        Touch(chatId);
    }

    /// <summary>It stopped early: the row keeps what arrived, and says so — and why, when the frame said.</summary>
    public void Stopped(long chatId, long messageId, AiErrorReason? reason = null, DateTimeOffset? renewsAt = null)
    {
        lock (gate)
        {
            stopped[messageId] = (reason, renewsAt);
        }
        Touch(chatId);
    }

    public void Finished(MessageDto message)
    {
        bool removed;
        lock (gate)
        {
            removed = written.Remove(message.Id) | stopped.Remove(message.Id);
        }
        if (removed)
        {
            Touch(message.ChatId);
        }
    }

    /// <summary>The body as drawn: the held one, and — until the answer is finished — what has streamed after it.</summary>
    public string BodyOf(MessageDto message)
    {
        if (message.EditSeq is not null)
        {
            return message.Body;
        }
        lock (gate)
        {
            return written.TryGetValue(message.Id, out var so) ? message.Body + so : message.Body;
        }
    }

    /// <summary>
    /// Whether what is drawn for this row includes streamed words the finished row has not yet replaced — the model's raw
    /// text, before the server's link filter (docs/protocol.md, "How sources are shown"). True for an answer that stopped
    /// part-way too, whose words are never replaced.
    /// </summary>
    public bool IsWriting(MessageDto message)
    {
        if (message.EditSeq is not null)
        {
            return false;
        }
        lock (gate)
        {
            return written.ContainsKey(message.Id);
        }
    }

    /// <summary>Whether the row says it failed — which sentence it says is <see cref="FailureSentence"/>.</summary>
    public bool Failed(MessageDto message)
    {
        if (message.EditSeq is not null)
        {
            return false;
        }
        lock (gate)
        {
            return stopped.ContainsKey(message.Id);
        }
    }

    /// <summary>
    /// What a failed row says — on the bubble, under what streamed, and to a screen reader — or null while it has not
    /// failed. A used-up allowance names its UTC renewal day in <paramref name="culture"/> (the current culture when not
    /// given).
    /// </summary>
    public string? FailureSentence(
        MessageDto message, IStringCatalog say, CultureInfo? culture = null, DateTimeOffset? now = null)
    {
        if (message.EditSeq is not null)
        {
            return null;
        }
        (AiErrorReason? Reason, DateTimeOffset? RenewsAt) failure;
        lock (gate)
        {
            if (!stopped.TryGetValue(message.Id, out failure))
            {
                return null;
            }
        }
        return Sentence(
            failure.Reason, failure.RenewsAt, say,
            culture ?? CultureInfo.CurrentCulture, now ?? DateTimeOffset.UtcNow);
    }

    /// <summary>The failure sentence for a reason: the provider's refusal is not worth asking again in the same words.</summary>
    public static string Sentence(AiErrorReason? reason, IStringCatalog say) =>
        Sentence(reason, null, say, CultureInfo.CurrentCulture, DateTimeOffset.UtcNow);

    /// <summary>
    /// The failure sentence for a reason, and for a used-up allowance the day it renews: the frame's <c>renews_at</c>, or —
    /// when it did not carry a readable one — the 1st of the next UTC month, which is what it always is — as its UTC day.
    /// </summary>
    public static string Sentence(
        AiErrorReason? reason, DateTimeOffset? renewsAt, IStringCatalog say, CultureInfo culture, DateTimeOffset now) => reason switch
    {
        AiErrorReason.Refused => say.Get("The assistant's provider refused that. Try putting it another way."),
        AiErrorReason.AllowanceUsed =>
            PaidFamilies.AiAllowanceUsed(renewsAt ?? PaidFamilies.NextMonthUtc(now), say, culture, now),
        _ => say.Get("Couldn't answer that. Ask again."),
    };

    public void Clear()
    {
        lock (gate)
        {
            written.Clear();
            stopped.Clear();
        }
        Interlocked.Increment(ref version);
    }

    private void Touch(long chatId)
    {
        Interlocked.Increment(ref version);
        Changed?.Invoke(chatId);
    }
}
