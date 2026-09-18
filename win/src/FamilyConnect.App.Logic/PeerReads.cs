namespace FamilyConnect.App.Logic;

/// <summary>
/// How far the other person in each direct chat has read — the seen tick's marker (docs/protocol.md, the
/// <c>read</c> frame).
/// </summary>
/// <remarks>
/// <para>
/// <b>ONLY A LIVE FRAME CARRIES IT.</b> <c>GET /chats</c> answers the reader's OWN marker and nobody else's, so this
/// is what the frames said since this device connected, kept in memory as the web client keeps it. The router
/// already passes only a direct chat's.
/// </para>
/// <para>
/// <b>MONOTONIC</b>, as every read marker is: a frame delayed behind a newer one must not take a tick back.
/// </para>
/// </remarks>
public sealed class PeerReads
{
    private readonly object gate = new();
    private readonly Dictionary<long, long> markers = [];

    /// <summary>A chat's marker moved forward (raised on whatever thread the frame arrived on).</summary>
    public event Action<long>? Changed;

    /// <summary>Take a marker; answers whether it moved anything.</summary>
    public bool Apply(long chatId, long lastReadMessageId)
    {
        lock (gate)
        {
            if (markers.TryGetValue(chatId, out var held) && held >= lastReadMessageId)
            {
                return false;
            }
            markers[chatId] = lastReadMessageId;
        }
        Changed?.Invoke(chatId);
        return true;
    }

    /// <summary>The marker for a chat: 0 while no frame has said.</summary>
    public long UpTo(long chatId)
    {
        lock (gate)
        {
            return markers.GetValueOrDefault(chatId);
        }
    }

    /// <summary>A session ended: somebody else's markers are not the next person's.</summary>
    public void Clear()
    {
        lock (gate)
        {
            markers.Clear();
        }
    }
}
