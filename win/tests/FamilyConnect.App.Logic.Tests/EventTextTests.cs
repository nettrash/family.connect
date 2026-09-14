using System.Globalization;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

public sealed class EventTextTests
{
    private static readonly CultureInfo British = CultureInfo.GetCultureInfo("en-GB");
    private static readonly TimeZoneInfo Utc = TimeZoneInfo.Utc;
    private static readonly TimeZoneInfo PlusThree = TimeZoneInfo.CreateCustomTimeZone("fc+3", TimeSpan.FromHours(3), "fc+3", "fc+3");

    [Fact]
    public void TheBlockIsTheDaysNumberOverItsShortMonthInTheReadersZone()
    {
        Assert.Equal(("24", "Dec"), EventText.DateBlock("2026-12-24T21:30:00Z", British, Utc));
        // Half past nine on Christmas Eve in London is half past midnight on Christmas Day three hours east.
        Assert.Equal(("25", "Dec"), EventText.DateBlock("2026-12-24T21:30:00Z", British, PlusThree));
        Assert.Null(EventText.DateBlock("not a time", British, Utc));
        Assert.Equal(("5", "Dec"), EventText.DateBlock("2026-12-05T10:00:00Z", British, Utc));
    }

    [Fact]
    public void TheClockSaysTheEndAndTheEndsDayWhenItIsAnotherDay()
    {
        Assert.Equal("16:00", EventText.Clock("2026-12-24T16:00:00Z", null, British, Utc));
        Assert.Equal("16:00 – 20:00", EventText.Clock("2026-12-24T16:00:00Z", "2026-12-24T20:00:00Z", British, Utc));
        Assert.Equal("16:00 – 25 Dec 02:00", EventText.Clock("2026-12-24T16:00:00Z", "2026-12-25T02:00:00Z", British, Utc));
        // The same pair, three hours east, starts and ends on the same local day.
        Assert.Equal("01:00 – 02:30", EventText.Clock("2026-12-24T22:00:00Z", "2026-12-24T23:30:00Z", British, PlusThree));
        Assert.Equal("16:00", EventText.Clock("2026-12-24T16:00:00Z", "garbage", British, Utc));
        Assert.Equal(string.Empty, EventText.Clock("", "2026-12-24T20:00:00Z", British, Utc));
        Assert.Contains("PM", EventText.Clock("2026-12-24T16:00:00Z", null, CultureInfo.GetCultureInfo("en-US"), Utc));
    }

    [Fact]
    public void TheWholeOfItHasTheWeekdayToo()
    {
        Assert.Equal("Thu, 24 Dec, 16:00 – 20:00", EventText.When("2026-12-24T16:00:00Z", "2026-12-24T20:00:00Z", British, Utc));
        Assert.Equal(string.Empty, EventText.When("never", null, British, Utc));
    }

    [Fact]
    public void AnEventIsOverWhenItsEndHasPassedOrWithNoEndItsStart()
    {
        var now = new DateTimeOffset(2026, 12, 24, 18, 0, 0, TimeSpan.Zero);
        Assert.True(EventText.IsPast("2026-12-24T16:00:00Z", null, now));
        Assert.False(EventText.IsPast("2026-12-24T16:00:00Z", "2026-12-24T20:00:00Z", now));
        Assert.True(EventText.IsPast("2026-12-24T12:00:00Z", "2026-12-24T17:00:00Z", now));
        Assert.False(EventText.IsPast("2026-12-25T10:00:00Z", null, now));
        Assert.False(EventText.IsPast("broken", null, now));
        // Ending this very minute is not over yet.
        Assert.False(EventText.IsPast("2026-12-24T16:00:00Z", "2026-12-24T18:00:00Z", now));
        Assert.Equal(new DateTimeOffset(2026, 12, 24, 16, 0, 0, TimeSpan.Zero), EventText.Instant("2026-12-24T18:00:00+02:00"));
    }
}
