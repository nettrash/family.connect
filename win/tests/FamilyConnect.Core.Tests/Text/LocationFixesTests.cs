using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using Xunit;

namespace FamilyConnect.Core.Tests;

/// <summary>The bar a shared place clears — web <c>location.rs</c>'s own tests, and the edges they leave out.</summary>
public sealed class LocationFixesTests
{
    private static readonly DateTimeOffset Now = new(2026, 9, 14, 12, 0, 0, TimeSpan.Zero);

    private static LocationFix Fix(double? accuracy, double ageMs = 0) =>
        new(55.75, 37.61, accuracy, Now.AddMilliseconds(-ageMs));

    [Fact]
    public void AFixIsSentOnlyWhileItIsFresh()
    {
        Assert.Equal(FixVerdict.Accept, LocationFixes.Judge(Fix(12, 1_000), Now));
        Assert.Equal(FixVerdict.Accept, LocationFixes.Judge(Fix(100), Now));
        // Coarse is held, not refused — and so is a fix that does not know how good it is.
        Assert.Equal(FixVerdict.Hold, LocationFixes.Judge(Fix(101), Now));
        Assert.Equal(FixVerdict.Hold, LocationFixes.Judge(Fix(null), Now));
        // Two minutes to the millisecond is still fresh; a millisecond more is not, however precise.
        Assert.Equal(FixVerdict.Accept, LocationFixes.Judge(Fix(5, 120_000), Now));
        Assert.Equal(FixVerdict.Stale, LocationFixes.Judge(Fix(5, 120_001), Now));
        Assert.Equal(FixVerdict.Stale, LocationFixes.Judge(Fix(null, 120_001), Now));
        Assert.Equal(FixVerdict.Stale, LocationFixes.Judge(Fix(500, 3_600_000), Now));
    }

    [Fact]
    public void TheBestFreshFixIsTheSmallestCircle()
    {
        var coarse = Fix(900);
        var finer = Fix(300);
        var unknown = Fix(null);
        Assert.Equal(coarse, LocationFixes.Better(null, coarse));
        Assert.Equal(finer, LocationFixes.Better(coarse, finer));
        Assert.Equal(finer, LocationFixes.Better(finer, coarse));
        // Known beats unknown, both ways round.
        Assert.Equal(coarse, LocationFixes.Better(unknown, coarse));
        Assert.Equal(coarse, LocationFixes.Better(coarse, unknown));
        // A tie keeps what was held.
        var sameCircle = coarse with { Latitude = 1 };
        Assert.Equal(coarse, LocationFixes.Better(coarse, sameCircle));
        Assert.Equal(unknown, LocationFixes.Better(unknown, unknown with { Latitude = 1 }));
    }

    [Fact]
    public void AnAccuracyIsKnownOnlyWhenItIsAMeasurement()
    {
        Assert.Equal(12.5, LocationFixes.UsableAccuracy(12.5));
        Assert.Equal(0, LocationFixes.UsableAccuracy(0));
        Assert.Null(LocationFixes.UsableAccuracy(-1));
        Assert.Null(LocationFixes.UsableAccuracy(double.NaN));
        Assert.Null(LocationFixes.UsableAccuracy(double.PositiveInfinity));
    }

    /// <summary>A good fix ends the wait; a coarse fresh one is held for the deadline; a stale one is nothing.</summary>
    [Fact]
    public void AHuntEndsOnAGoodFixAndHoldsTheBestCoarseOne()
    {
        var hunt = new LocationHunt();
        Assert.Null(hunt.Offer(Fix(5, 600_000), Now));
        Assert.Null(hunt.Best);

        Assert.Null(hunt.Offer(Fix(900), Now));
        Assert.Equal(900, hunt.Best?.AccuracyM);
        Assert.Null(hunt.Offer(Fix(300), Now));
        Assert.Null(hunt.Offer(Fix(900), Now));
        Assert.Equal(300, hunt.Best?.AccuracyM);

        var good = Fix(40);
        Assert.Equal(good, hunt.Offer(good, Now));

        Assert.Equal(100, LocationFixes.GoodEnoughMetres);
        Assert.Equal(TimeSpan.FromMinutes(2), LocationFixes.FreshEnough);
        Assert.Equal(TimeSpan.FromSeconds(20), LocationFixes.Timeout);
    }

    /// <summary>A label somebody typed rides escaped, after the numbers; an empty one is not sent.</summary>
    [Fact]
    public void APlacesLabelRidesEscapedAfterItsNumbers()
    {
        Assert.Equal(
            "kind=location&latitude=1.0000000&longitude=2.0000000&name=Home%20%26%20Caf%C3%A9",
            UploadQueries.Location(1, 2, null, "Home & Café"));
        Assert.Equal("kind=location&latitude=1.0000000&longitude=2.0000000", UploadQueries.Location(1, 2, null, ""));
    }
}
