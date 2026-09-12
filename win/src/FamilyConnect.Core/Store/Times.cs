using System.Globalization;

namespace FamilyConnect.Core.Store;

/// <summary>
/// The wire carries RFC3339 instants; the cache stores milliseconds since the epoch.
/// </summary>
/// <remarks>
/// An INSTANT either way, never a local time: the wire stores the moment and every client draws it
/// in the reader's own zone, which is the whole reason a family spread across two countries sees
/// the same event at the time it is for them (docs/protocol.md, "Board").
/// </remarks>
public static class Times
{
    /// <summary>Milliseconds since the epoch, or null for an absent or unparseable stamp.</summary>
    public static long? Instant(string? rfc3339)
    {
        if (string.IsNullOrEmpty(rfc3339))
        {
            return null;
        }
        return DateTimeOffset.TryParse(
            rfc3339, CultureInfo.InvariantCulture,
            DateTimeStyles.AdjustToUniversal | DateTimeStyles.AssumeUniversal, out var when)
            ? when.ToUnixTimeMilliseconds()
            : null;
    }

    /// <summary>
    /// Back to the wire's spelling: UTC, with a `Z`. Null and 0 both mean "no stamp" — 0 is this
    /// cache's spelling of absent, not 1970.
    /// </summary>
    public static string? Rfc3339(long? milliseconds) =>
        milliseconds is null or 0
            ? null
            : DateTimeOffset.FromUnixTimeMilliseconds(milliseconds.Value)
                .UtcDateTime
                .ToString("yyyy-MM-ddTHH:mm:ss.fffZ", CultureInfo.InvariantCulture);
}
