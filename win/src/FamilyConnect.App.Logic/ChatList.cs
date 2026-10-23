using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic;

/// <summary>
/// WHICH time a row shows, decided here; HOW it reads is the window's, because a date is drawn in
/// the reader's own zone, calendar and language and none of those belong in a rule.
/// </summary>
public enum RowTimeKind
{
    /// <summary>Nothing to show: a chat with no messages.</summary>
    None,

    /// <summary>Today: the clock.</summary>
    Clock,

    /// <summary>Yesterday: the word.</summary>
    Yesterday,

    /// <summary>Within the last week: the weekday, short.</summary>
    Weekday,

    /// <summary>Older: the month and the day.</summary>
    Date,
}

/// <summary>One row of the chat list, as the window draws it and with nothing left to decide.</summary>
public sealed record ChatRow(
    ChatDto Chat,
    string Title,
    string Preview,
    int Unread,
    bool Mentioned,
    /// <summary>
    /// The newest message is from somebody this reader has blocked, so the line is the hidden row
    /// and there is nothing to reveal — a list is not where a person peeks.
    /// </summary>
    bool Hidden,
    RowTimeKind When,
    DateTimeOffset? At)
{
    public bool IsFamily => Chat.Kind == ChatKinds.Family;

    public bool IsDirect => Chat.Kind == ChatKinds.Direct;

    public bool IsAssistant => Chat.Kind == ChatKinds.Assistant;

    /// <summary>A group (docs/protocol.md, "Groups"): listed below the family chat, by its last message like any chat.</summary>
    public bool IsGroup => ChatKinds.IsGroup(Chat.Kind);

    /// <summary>What its circle shows.</summary>
    public ChatFace Face => ChatFaces.For(Chat.Kind);
}

/// <summary>Which circle a chat is drawn with, by its kind — the list's rows and the conversation's header alike.</summary>
public static class ChatFaces
{
    public static ChatFace For(string? kind) => kind switch
    {
        ChatKinds.Family => ChatFace.House,
        ChatKinds.Group => ChatFace.People,
        _ => ChatFace.Initials,
    };
}

/// <summary>
/// The chat list: the rows, in the order something last happened in them, each with the one line
/// under its name.
/// </summary>
/// <remarks>
/// <para>
/// <b>THE ORDER AND THE COUNTS ARE THE SERVER'S.</b> The list read carries the authoritative
/// unread counts and the caller's own read marker, and this model reads them back out of the cache
/// rather than counting anything itself — two devices that each counted would eventually disagree
/// about the same family.
/// </para>
/// <para>
/// <b>A BLOCKED MEMBER'S MESSAGE IS THE HIDDEN ROW HERE TOO</b>, with no name and no reveal. In
/// the family chat their message still arrives, still counts and may still BE the preview, because
/// the count is the other half of the read marker and projecting one without the other
/// desynchronises them — so the row shows that something was said and not what
/// (docs/protocol.md, "Blocking a member").
/// </para>
/// <para>
/// <b>A CALL READS AS A CALL</b> and never as the English placeholder the server writes into a
/// record's body for clients that predate calls; a caption-less attachment reads as what it is,
/// because a preview with nothing in it is a row that looks like nothing happened.
/// </para>
/// </remarks>
public sealed class ChatListModel(ChatStore chats, Func<long> me, IStringCatalog? words = null)
{
    private readonly IStringCatalog say = words ?? EnglishCatalog.Instance;

    /// <summary>
    /// Every chat on the list as the last <see cref="Rows"/> read it — of EVERY family, before the shown one was picked out —
    /// so the family switcher can count each family's unread without reading the list a second time.
    /// </summary>
    public IReadOnlyList<ChatRowDto> Held { get; private set; } = [];

    /// <summary>The rows, as of <paramref name="now"/> — which only the times depend on.</summary>
    /// <remarks>
    /// THE FAMILY CHAT IS FIRST, always, and everything else — groups included (docs/protocol.md,
    /// "Groups") — follows it newest conversation first. It is the room everybody is in and the one a reader looks for — Apple pins it, the
    /// web pins it, and a list that let it slide under a busy direct chat would be a different
    /// app on Windows. The rest go by the id of their newest message rather than by its time: ids
    /// are monotonic, where a stamp can be moved by a server's clock and is deliberately NOT
    /// moved by an edit.
    /// </remarks>
    public IReadOnlyList<ChatRow> Rows(DateTimeOffset now)
    {
        var rows = new List<ChatRow>();
        var newestId = new Dictionary<long, long>();
        // The family shown: its chats and no other family's (docs/protocol.md, "Several families") — so "the family chat"
        // pinned first below is the shown family's, and so is the assistant's.
        var shown = chats.Family;
        Held = chats.Chats();
        foreach (var held in Held.Where(row => FamilyRules.Lists(row.Chat, shown)))
        {
            var newest = chats.Newest(held.Chat.Id);
            newestId[held.Chat.Id] = newest?.Id ?? 0;
            var hidden = newest is not null && IsHidden(newest);
            var at = newest is null ? null : Instant(newest.CreatedAt);
            rows.Add(new ChatRow(
                held.Chat,
                Title(held.Chat),
                newest is null ? say.Get("No messages yet") : Preview(newest, hidden),
                held.UnreadCount,
                held.Mentioned == true,
                hidden,
                at is null ? RowTimeKind.None : When(at.Value, now),
                at));
        }
        return [.. rows
            .OrderByDescending(row => row.IsFamily)
            .ThenByDescending(row => newestId[row.Chat.Id])
            .ThenBy(row => row.Chat.Id)];
    }

    /// <summary>
    /// The name on the row. The wire's title is the family's name, the peer's display name, the
    /// assistant's or the group's, recomputed by the server on every read — so it wins; the roster is the
    /// fallback for a direct chat whose title arrived empty, and "Chat" is the floor, because a
    /// row with no name at all is a row nobody can tap with intent.
    /// </summary>
    public string Title(ChatDto chat)
    {
        if (!string.IsNullOrEmpty(chat.Title))
        {
            return chat.Title;
        }
        if (chat.PeerUserId is { } peer && chats.Member(peer)?.DisplayName is { Length: > 0 } name)
        {
            return name;
        }
        return say.Get("Chat");
    }

    /// <summary>The one line under the name.</summary>
    public string Preview(MessageDto message, bool hidden)
    {
        if (hidden)
        {
            return say.Get("Hidden — blocked member");
        }
        if (message.Call is { } call)
        {
            return CallRecordText.Label(
                call.Outcome, call.DurationSecs, call.Video, message.SenderId == me(), say);
        }
        if (message.Body.Length > 0)
        {
            // The FIRST LINE only: a row is one line high, and a body with a newline in it would
            // otherwise push every row below it down the list.
            var end = message.Body.IndexOfAny(['\n', '\r']);
            return end < 0 ? message.Body : message.Body[..end];
        }
        var media = message.Media;
        if (media.Count == 0)
        {
            return string.Empty;
        }
        if (message.StickerPicture is not null)
        {
            // The word, in the reader's language, where a photo's row says so of a photo.
            return say.Get("Sticker");
        }
        if (message.RoundVideo is not null)
        {
            // Asked BEFORE "Video" (S5.7): a circle is a video on the wire and a video message to the reader.
            return say.Get("Video message");
        }
        if (message.AnimationVideo is not null)
        {
            // An animation says "GIF" where a video says "Video" (docs/protocol.md, "Animations") — and its caption first,
            // as any video's row does.
            return say.Get("GIF");
        }
        return media[0].Kind switch
        {
            "photo" when media.Count > 1 => say.Format("%lld Photos", media.Count),
            "photo" => say.Get("Photo"),
            "video" => say.Get("Video"),
            // The preview says "Voice message" where an attachment's own name would say "Audio":
            // this is the row for a message that IS the recording.
            "audio" => say.Get("Voice message"),
            "location" => say.Get("Location"),
            "file" => AttachmentText.DisplayName("file", media[0].Name, say),
            // A kind this build never heard of still says something happened.
            _ => AttachmentText.DisplayName(media[0].Kind, media[0].Name, say),
        };
    }

    /// <summary>Whether this message is drawn hidden: somebody else's, and blocked.</summary>
    public bool IsHidden(MessageDto message) =>
        message.SenderId != me() && chats.IsBlocked(message.SenderId);

    /// <summary>
    /// Which time a row shows. Days are compared in the READER's zone, not in UTC: a message sent
    /// at 23:30 last night is "Yesterday" to the person reading it, whatever the clock says in
    /// Greenwich.
    /// </summary>
    public static RowTimeKind When(DateTimeOffset at, DateTimeOffset now)
    {
        var day = at.ToLocalTime().Date;
        var today = now.ToLocalTime().Date;
        if (day == today)
        {
            return RowTimeKind.Clock;
        }
        if (day == today.AddDays(-1))
        {
            return RowTimeKind.Yesterday;
        }
        var age = now - at;
        return age >= TimeSpan.Zero && age < TimeSpan.FromDays(7)
            ? RowTimeKind.Weekday
            : RowTimeKind.Date;
    }

    private static DateTimeOffset? Instant(string rfc3339) =>
        Times.Instant(rfc3339) is { } milliseconds
            ? DateTimeOffset.FromUnixTimeMilliseconds(milliseconds)
            : null;
}
