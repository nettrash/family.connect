using System.Globalization;
using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>The web client's timeline tests, ported: day pills, the sender caption, runs, the divider and the opening anchor.</summary>
public sealed class TimelineTests
{
    private const long Me = 7;
    private const long Anna = 9;
    private const long Bob = 11;
    private static readonly TimeZoneInfo Utc = TimeZoneInfo.Utc;

    private static string At(int day, int hour) => $"2026-09-{day:00}T{hour:00}:00:00Z";

    private static Bubble Row(long id, long sender, string at, bool hidden = false) =>
        new(new MessageDto(id, 42, sender, null, $"message {id}", at), hidden, false, sender == Me);

    [Fact]
    public void EachLocalDayStartsASectionWithItsOwnPill()
    {
        var rows = Timeline.Rows([Row(1, Anna, At(9, 10)), Row(2, Anna, At(9, 11)), Row(3, Anna, At(10, 9))], familyChat: true, Me, null, Utc);

        Assert.Equal(new DateOnly(2026, 9, 9), rows[0].DayAbove);
        Assert.Null(rows[1].DayAbove);
        Assert.Equal(new DateOnly(2026, 9, 10), rows[2].DayAbove);
        // A run never crosses a day: the name shows again after the pill, and the day ends the run.
        Assert.Equal([true, false, true], rows.Select(row => row.ShowsSender));
        Assert.True(rows[1].RunEnd);
        Assert.True(rows[2].RunStart);
    }

    /// <summary>The day is the READER's: an evening message in New York is the next day's in UTC.</summary>
    [Fact]
    public void TheDayIsTheReadersOwn()
    {
        var zone = TimeZoneInfo.CreateCustomTimeZone("minus5", TimeSpan.FromHours(-5), "minus5", "minus5");
        var rows = Timeline.Rows([Row(1, Anna, At(10, 2)), Row(2, Anna, At(10, 6))], true, Me, null, zone);
        Assert.Equal(new DateOnly(2026, 9, 9), rows[0].DayAbove);
        Assert.Equal(new DateOnly(2026, 9, 10), rows[1].DayAbove);
    }

    /// <summary>Family chat only, never mine, never on a hidden row, and only where the sender changes.</summary>
    [Fact]
    public void TheSenderShowsWhereTheSenderChangesInTheFamilyChat()
    {
        Bubble[] bubbles =
        [
            Row(1, Anna, At(9, 10)), Row(2, Anna, At(9, 10)), Row(3, Me, At(9, 10)), Row(4, Bob, At(9, 10), hidden: true),
            Row(5, Anna, At(9, 10)),
        ];
        var rows = Timeline.Rows(bubbles, familyChat: true, Me, null, Utc);
        Assert.Equal([true, false, false, false, true], rows.Select(row => row.ShowsSender));
        Assert.DoesNotContain(Timeline.Rows(bubbles, familyChat: false, Me, null, Utc), row => row.ShowsSender);

        Assert.Equal([true, false, true, true, true], rows.Select(row => row.RunStart));
        Assert.Equal([false, true, true, true, true], rows.Select(row => row.RunEnd));
    }

    /// <summary>A stamp that does not parse belongs to the day it sits at the end of — no pill, and the run goes on.</summary>
    [Fact]
    public void AnUnparseableStampJoinsTheDayItFollows()
    {
        var rows = Timeline.Rows([Row(1, Anna, At(9, 10)), Row(2, Anna, "not a time"), Row(3, Anna, At(9, 12))], true, Me, null, Utc);
        Assert.Equal([true, false, false], rows.Select(row => row.DayAbove is not null));
        Assert.Equal([true, false, false], rows.Select(row => row.ShowsSender));
        Assert.Null(Timeline.Rows([Row(1, Anna, "nope")], true, Me, null, Utc)[0].DayAbove);
    }

    [Fact]
    public void ThereIsAtMostOneDivider()
    {
        var rows = Timeline.Rows([Row(1, Anna, At(9, 10)), Row(2, Anna, At(9, 10)), Row(2, Anna, At(9, 10))], true, Me, firstUnreadId: 2, Utc);
        Assert.Equal([false, true, false], rows.Select(row => row.UnreadDividerAbove));
        Assert.DoesNotContain(Timeline.Rows([Row(1, Anna, At(9, 10))], true, Me, firstUnreadId: null, Utc), row => row.UnreadDividerAbove);
    }

    private static MessageDto M(long id, long sender) => new(id, 42, sender, null, "x", At(9, 10));

    /// <summary>With a marker: the oldest message from somebody else above it — or nowhere, if not every unread one is held.</summary>
    [Fact]
    public void AnOpeningAnchorsAtTheOldestUnreadAboveTheMarker()
    {
        MessageDto[] held = [M(1, Anna), M(2, Anna), M(3, Me), M(4, Anna), M(5, Bob), M(0, Anna)];
        Assert.Null(Timeline.OpenAnchor(held, unreadCount: 0, lastRead: 2, Me));
        Assert.Equal(4, Timeline.OpenAnchor(held, unreadCount: 2, lastRead: 2, Me));
        Assert.Equal(4, Timeline.OpenAnchor(held, unreadCount: 1, lastRead: 2, Me));
        Assert.Null(Timeline.OpenAnchor(held, unreadCount: 3, lastRead: 2, Me));
    }

    /// <summary>With no marker: the unread count back from the newest message from somebody else, and nowhere on a short count.</summary>
    [Fact]
    public void WithNoMarkerTheCountIsCountedBack()
    {
        MessageDto[] held = [M(1, Anna), M(2, Anna), M(3, Me), M(4, Bob)];
        Assert.Equal(4, Timeline.OpenAnchor(held, unreadCount: 1, lastRead: 0, Me));
        Assert.Equal(2, Timeline.OpenAnchor(held, unreadCount: 2, lastRead: 0, Me));
        Assert.Equal(1, Timeline.OpenAnchor(held, unreadCount: 3, lastRead: 0, Me));
        Assert.Null(Timeline.OpenAnchor(held, unreadCount: 4, lastRead: 0, Me));
        // A row the server has not numbered is nobody's unread message, whoever it seems to be from.
        Assert.Equal(1, Timeline.OpenAnchor([M(1, Anna), M(0, Anna)], unreadCount: 1, lastRead: 0, Me));
    }

    /// <summary>Never past the window: an anchor 301 rows back is no anchor, 300 is.</summary>
    [Fact]
    public void AnAnchorBeyondTheCapIsNone()
    {
        var held = new List<MessageDto> { M(1, Anna) };
        held.AddRange(Enumerable.Range(2, Timeline.AnchorCap).Select(id => M(id, Me)));
        Assert.Equal(1, Timeline.OpenAnchor(held, unreadCount: 1, lastRead: 0, Me));
        held.Add(M(999, Me));
        Assert.Null(Timeline.OpenAnchor(held, unreadCount: 1, lastRead: 0, Me));
    }

    [Fact]
    public void ThePillSaysTodayYesterdayOrTheShortDate()
    {
        var say = EnglishCatalog.Instance;
        var today = new DateOnly(2026, 9, 17);
        var english = CultureInfo.GetCultureInfo("en-US");
        Assert.Equal("Today", Timeline.DayLabel(today, today, english, say));
        Assert.Equal("Yesterday", Timeline.DayLabel(new DateOnly(2026, 9, 16), today, english, say));
        Assert.Equal("Mon, Aug 17", Timeline.DayLabel(new DateOnly(2026, 8, 17), today, english, say));
        // Across a month and a year boundary the calendar, not 24 hours, decides.
        Assert.Equal("Yesterday", Timeline.DayLabel(new DateOnly(2025, 12, 31), new DateOnly(2026, 1, 1), english, say));
        Assert.StartsWith("Mo, 17.", Timeline.DayLabel(new DateOnly(2026, 8, 17), today, CultureInfo.GetCultureInfo("de-DE"), say));

        Assert.Equal("1 new message", Timeline.DividerText(1, say));
        Assert.Equal("3 new messages", Timeline.DividerText(3, say));
    }
}
