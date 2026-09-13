using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic;

/// <summary>Who is typing where — the line under a conversation's name.</summary>
/// <remarks>
/// <para>
/// <b>A TYPING FRAME IS MOMENTARY.</b> It stands for five seconds — the phones' and the web's TTL —
/// and is never stored. The reader's own frames (from another device) and a blocked member's are
/// not news, and a message from somebody ends their typing: they stopped, and said it.
/// </para>
/// <para>
/// <b>BY MEMBER, NOT BY WHOSE FRAME CAME LAST</b>, so "Anna, Bob" does not keep swapping while
/// somebody reads it. The sentences are the Apple client's — "%@ is typing…" and "%@ are typing…"
/// over the names — because those are the ones translated into all nine languages.
/// </para>
/// </remarks>
public sealed class TypingRoster(ChatStore chats, Func<DateTimeOffset>? clock = null, IStringCatalog? words = null)
{
    public static readonly TimeSpan Lasts = TimeSpan.FromSeconds(5);

    private readonly Func<DateTimeOffset> now = clock ?? (() => DateTimeOffset.UtcNow);
    private readonly IStringCatalog say = words ?? EnglishCatalog.Instance;
    private readonly object gate = new();
    private readonly Dictionary<long, Dictionary<long, DateTimeOffset>> typing = [];

    /// <summary>A <c>typing</c> frame. Raised on the socket's thread.</summary>
    public void Heard(long chatId, long userId)
    {
        if (userId == chats.Reader || chats.IsBlocked(userId))
        {
            return;
        }
        lock (gate)
        {
            if (!typing.TryGetValue(chatId, out var who))
            {
                typing[chatId] = who = [];
            }
            who[userId] = now();
        }
    }

    /// <summary>A message arrived: whoever sent it has stopped typing.</summary>
    public void Spoke(MessageDto message)
    {
        lock (gate)
        {
            if (typing.TryGetValue(message.ChatId, out var who))
            {
                who.Remove(message.SenderId);
            }
        }
    }

    /// <summary>Who is typing in this chat now, in member order.</summary>
    public IReadOnlyList<long> Typists(long chatId)
    {
        var at = now();
        lock (gate)
        {
            return typing.TryGetValue(chatId, out var who)
                ? who.Where(entry => at - entry.Value < Lasts).Select(entry => entry.Key).Order().ToList()
                : [];
        }
    }

    /// <summary>The line to draw, or nothing.</summary>
    public string Line(long chatId)
    {
        var names = Typists(chatId)
            .Select(id => chats.Member(id)?.DisplayName is { Length: > 0 } name ? name : say.Get("Someone"))
            .ToList();
        return names.Count switch
        {
            0 => string.Empty,
            1 => say.Format("%@ is typing…", names[0]),
            _ => say.Format("%@ are typing…", string.Join(", ", names)),
        };
    }
}
