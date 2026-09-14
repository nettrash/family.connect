using System.Globalization;

namespace FamilyConnect.App.Logic;

/// <summary>
/// When an event is, as the wall writes it — a CALENDAR ENTRY: the day's number over its short month,
/// and the time beside it (docs/protocol.md, "Board"). The reader's own language and zone: the wire
/// carries an instant, and "16:00" is only true somewhere.
/// </summary>
public static class EventText
{
    /// <summary>An instant off the wire, or null for one that cannot be read.</summary>
    public static DateTimeOffset? Instant(string? rfc3339) =>
        !string.IsNullOrWhiteSpace(rfc3339)
        && DateTimeOffset.TryParse(rfc3339, CultureInfo.InvariantCulture, DateTimeStyles.AssumeUniversal, out var at)
            ? at
            : null;

    /// <summary>The block: the day's number and its short month.</summary>
    public static (string Day, string Month)? DateBlock(string startsAt, CultureInfo culture, TimeZoneInfo zone) =>
        Local(startsAt, zone) is { } local
            ? (local.ToString("%d", culture), local.ToString("MMM", culture))
            : null;

    /// <summary>
    /// The time, beside the block that already says the date: "16:00", "16:00 – 20:00", or
    /// "16:00 – 25 Dec 02:00" when it ends on another day.
    /// </summary>
    public static string Clock(string startsAt, string? endsAt, CultureInfo culture, TimeZoneInfo zone)
    {
        if (Local(startsAt, zone) is not { } starts)
        {
            return string.Empty;
        }
        var from = starts.ToString("t", culture);
        if (Local(endsAt, zone) is not { } ends)
        {
            return from;
        }
        var to = ends.ToString("t", culture);
        return ends.Date == starts.Date
            ? $"{from} – {to}"
            : $"{from} – {ends.ToString(ShortMonthDay(culture), culture)} {to}";
    }

    /// <summary>The whole of it in one line, weekday included — for a label that stands in for the card.</summary>
    public static string When(string startsAt, string? endsAt, CultureInfo culture, TimeZoneInfo zone) =>
        Local(startsAt, zone) is { } starts
            ? $"{starts.ToString($"ddd, {ShortMonthDay(culture)}", culture)}, {Clock(startsAt, endsAt, culture, zone)}"
            : string.Empty;

    /// <summary>Over: its end has passed, or — with no end — its start.</summary>
    public static bool IsPast(string startsAt, string? endsAt, DateTimeOffset now) =>
        (Instant(endsAt) ?? Instant(startsAt)) is { } at && at < now;

    private static DateTimeOffset? Local(string? rfc3339, TimeZoneInfo zone) =>
        Instant(rfc3339) is { } at ? TimeZoneInfo.ConvertTime(at, zone) : null;

    /// <summary>The culture's own month-and-day, with the month abbreviated: "Dec 25", "25 Dec".</summary>
    private static string ShortMonthDay(CultureInfo culture) =>
        culture.DateTimeFormat.MonthDayPattern.Replace("MMMM", "MMM", StringComparison.Ordinal);
}
