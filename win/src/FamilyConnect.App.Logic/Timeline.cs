using System.Globalization;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic;

/// <summary>One drawn row of a conversation, with nothing left to decide.</summary>
public sealed record TimelineRow(
    Bubble Bubble,
    /// <summary>The day pill above this row: the first row of each local day.</summary>
    DateOnly? DayAbove,
    /// <summary>"N new messages" goes above this row — at most one row in a chat.</summary>
    bool UnreadDividerAbove,
    /// <summary>The sender's face and name above the bubble.</summary>
    bool ShowsSender,
    /// <summary>The first of a run by one sender: the gap above it is a turn's, and its corner is round.</summary>
    bool RunStart,
    /// <summary>The last of a run by one sender: the time shows here, and its corner is round.</summary>
    bool RunEnd);

/// <summary>
/// How a chat's messages are laid out as rows — the web client's <c>timeline</c> and the apps' <c>MessagePresentation</c>:
/// day sections with one pill each, the single unread divider, whose name shows above a bubble, where a run starts and
/// ends, and where a chat opens.
/// </summary>
public static class Timeline
{
    /// <summary>How far back from the newest row an opening may anchor: the apps refuse to anchor past their window.</summary>
    public const int AnchorCap = 300;

    /// <summary>The local day a message was sent on, or null for a stamp that does not parse.</summary>
    public static DateOnly? DayOf(MessageDto message, TimeZoneInfo zone) =>
        Times.Instant(message.CreatedAt) is { } milliseconds
            ? DateOnly.FromDateTime(TimeZoneInfo.ConvertTime(DateTimeOffset.FromUnixTimeMilliseconds(milliseconds), zone).DateTime)
            : null;

    /// <summary>Lay a chat's bubbles (oldest first) out as rows.</summary>
    public static IReadOnlyList<TimelineRow> Rows(
        IReadOnlyList<Bubble> bubbles, bool familyChat, long me, long? firstUnreadId, TimeZoneInfo zone)
    {
        // A row whose stamp does not parse belongs to the day it sits at the end of.
        var days = new DateOnly?[bubbles.Count];
        DateOnly? running = null;
        for (var at = 0; at < bubbles.Count; at++)
        {
            running = DayOf(bubbles[at].Message, zone) ?? running;
            days[at] = running;
        }

        var rows = new List<TimelineRow>(bubbles.Count);
        DateOnly? section = null;
        var placed = false;
        for (var at = 0; at < bubbles.Count; at++)
        {
            var bubble = bubbles[at];
            var message = bubble.Message;
            var day = days[at];
            var newDay = day != section;
            if (newDay)
            {
                section = day;
            }
            // A run never crosses a day: the name shows again after the pill.
            var previous = at > 0 && !newDay ? bubbles[at - 1].Message : null;
            var next = at + 1 < bubbles.Count && days[at + 1] == section ? bubbles[at + 1].Message : null;
            // A hidden row still counts as its sender's run, so the next visible sender keeps their caption.
            var showsSender = familyChat
                && message.SenderId != me
                && !bubble.Hidden
                && (previous is null || previous.SenderId != message.SenderId);
            // At most one divider, structurally.
            var divider = !placed && firstUnreadId is { } first && message.Id == first;
            placed |= divider;
            rows.Add(new TimelineRow(
                bubble,
                newDay ? day : null,
                divider,
                showsSender,
                previous is null || previous.SenderId != message.SenderId,
                next is null || next.SenderId != message.SenderId));
        }
        return rows;
    }

    /// <summary>
    /// Where a chat OPENS: the first unread message, under the divider — or null, which is the ordinary open at the
    /// newest. Decided once per open (ios <c>UnreadAnchor</c>): nothing unread is the newest; with a read marker, the
    /// oldest message from somebody else above it, unless the ones held do not include every unread one; with no marker,
    /// the unread count back from the newest; and never past <see cref="AnchorCap"/> rows back.
    /// </summary>
    public static long? OpenAnchor(IReadOnlyList<MessageDto> messages, int unreadCount, long lastRead, long me)
    {
        if (unreadCount <= 0)
        {
            return null;
        }
        // Distance counts EVERY row back from the newest; only a numbered row from somebody else can be unread.
        var inbound = messages
            .Reverse()
            .Select((message, distance) => (Message: message, Distance: distance))
            .Where(row => row.Message.Id != 0 && row.Message.SenderId != me)
            .Select(row => (row.Distance, row.Message.Id))
            .ToList();
        (int Distance, long Id) hit;
        if (lastRead > 0)
        {
            var above = inbound.Where(row => row.Id > lastRead).ToList();
            if (above.Count < unreadCount)
            {
                return null;
            }
            hit = above[^1];
        }
        else
        {
            if (inbound.Count < unreadCount)
            {
                return null;
            }
            hit = inbound[unreadCount - 1];
        }
        return hit.Distance <= AnchorCap ? hit.Id : null;
    }

    /// <summary>
    /// The pill between day sections: "Today", "Yesterday", otherwise the short weekday and the month and day — no year —
    /// the three labels the apps draw.
    /// </summary>
    public static string DayLabel(DateOnly day, DateOnly today, CultureInfo culture, IStringCatalog say)
    {
        if (day == today)
        {
            return say.Get("Today");
        }
        if (day == today.AddDays(-1))
        {
            return say.Get("Yesterday");
        }
        var date = day.ToDateTime(TimeOnly.MinValue);
        // The culture's own month-and-day order, with the month shortened: "Aug 17", "17. Aug.", "8月17日".
        var monthDay = culture.DateTimeFormat.MonthDayPattern.Replace("MMMM", "MMM", StringComparison.Ordinal);
        return $"{culture.DateTimeFormat.GetAbbreviatedDayName(date.DayOfWeek)}, {date.ToString(monthDay, culture)}";
    }

    /// <summary>The divider's words.</summary>
    public static string DividerText(int unreadCount, IStringCatalog say) =>
        say.Plural("%lld new messages", unreadCount, unreadCount);
}
