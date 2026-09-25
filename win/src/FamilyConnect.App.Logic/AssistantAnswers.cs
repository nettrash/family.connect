using System.Text;
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
/// </remarks>
public sealed class AssistantAnswers
{
    private readonly object gate = new();
    private readonly Dictionary<long, StringBuilder> written = [];
    private readonly HashSet<long> stopped = [];
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

    /// <summary>It stopped early: the row keeps what arrived, and says so.</summary>
    public void Stopped(long chatId, long messageId)
    {
        lock (gate)
        {
            stopped.Add(messageId);
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

    /// <summary>Whether the row says "Couldn't answer that. Ask again."</summary>
    public bool Failed(MessageDto message)
    {
        if (message.EditSeq is not null)
        {
            return false;
        }
        lock (gate)
        {
            return stopped.Contains(message.Id);
        }
    }

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
