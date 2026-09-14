namespace FamilyConnect.Core;

/// <summary>One reading of where this device is: the numbers, how sure it was, and when it was taken.</summary>
/// <param name="AccuracyM">Metres, or null when the device did not report a usable one — drawn as a plain pin, never as perfect precision.</param>
public readonly record struct LocationFix(double Latitude, double Longitude, double? AccuracyM, DateTimeOffset Taken);

/// <summary>What a fix that just arrived means for the wait.</summary>
public enum FixVerdict
{
    /// <summary>Fresh and good enough: send it.</summary>
    Accept,

    /// <summary>Fresh but coarse: keep it, in case nothing better comes.</summary>
    Hold,

    /// <summary>Too old to send at all.</summary>
    Stale,
}

/// <summary>
/// The bar a shared place must clear (docs/protocol.md, "Locations"; web <c>location.rs</c>, ios <c>LocationProvider</c>):
/// a fix older than two minutes is never sent, however quickly the device hands it over; 100 m is good enough to send at
/// once; and after twenty seconds the best FRESH fix seen goes, coarse or not, with its accuracy saying so honestly.
/// </summary>
public static class LocationFixes
{
    /// <summary>Good enough to stop looking, in metres: a street, which is what "where are you?" asks.</summary>
    public const double GoodEnoughMetres = 100;

    /// <summary>The oldest fix that may be sent.</summary>
    public static readonly TimeSpan FreshEnough = TimeSpan.FromMinutes(2);

    /// <summary>How long to look — once looking is allowed, never counting somebody reading the prompt.</summary>
    public static readonly TimeSpan Timeout = TimeSpan.FromSeconds(20);

    /// <summary>
    /// How old, and how good. Staleness is decided FIRST: a pinpoint fix from an hour ago is precisely wrong, which is
    /// worse than roughly right.
    /// </summary>
    public static FixVerdict Judge(LocationFix fix, DateTimeOffset now) =>
        now - fix.Taken > FreshEnough ? FixVerdict.Stale
        : fix.AccuracyM is { } accuracy && accuracy <= GoodEnoughMetres ? FixVerdict.Accept
        : FixVerdict.Hold;

    /// <summary>The better of two fresh fixes: the smaller circle, and a known one over an unknown one.</summary>
    public static LocationFix Better(LocationFix? held, LocationFix fix) => held switch
    {
        null => fix,
        { AccuracyM: { } was } when fix.AccuracyM is { } now && now < was => fix,
        { AccuracyM: null } when fix.AccuracyM is not null => fix,
        { } kept => kept,
    };

    /// <summary>An accuracy the device reported, if it is a measurement at all: finite and not negative.</summary>
    public static double? UsableAccuracy(double accuracy) => double.IsFinite(accuracy) && accuracy >= 0 ? accuracy : null;
}

/// <summary>
/// One wait for a fix: a good one ends it, a coarse fresh one is held for the deadline, and a stale one is nothing.
/// Per wait, never per device — the next "where are you?" must not be answered with a fix held from the last one.
/// </summary>
/// <remarks>Offered from the platform's thread and read at the deadline from another, so it holds a lock.</remarks>
public sealed class LocationHunt
{
    private readonly object gate = new();
    private LocationFix? best;

    /// <summary>The best fresh fix held so far — what the deadline sends, or null for "could not find".</summary>
    public LocationFix? Best
    {
        get
        {
            lock (gate)
            {
                return best;
            }
        }
    }

    /// <summary>One delivery: the fix itself when it ends the wait, otherwise null.</summary>
    public LocationFix? Offer(LocationFix fix, DateTimeOffset now)
    {
        switch (LocationFixes.Judge(fix, now))
        {
            case FixVerdict.Accept:
                return fix;
            case FixVerdict.Hold:
                lock (gate)
                {
                    best = LocationFixes.Better(best, fix);
                }
                return null;
            default:
                return null;
        }
    }
}
