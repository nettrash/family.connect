using FamilyConnect.App.Logic;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// The version line, and why the build number is in brackets rather than inline.
/// </summary>
public sealed class AppVersionTextTests
{
    /// <summary>
    /// A stamped build reads the way the Apple apps read — version, then build in brackets. The
    /// third MSIX part is the build number because the Store reserves the fourth, so printing it
    /// inline ("1.1.437") would advertise a patch release that does not exist.
    /// </summary>
    [Fact]
    public void AStampedBuildPutsItsNumberInBrackets()
    {
        Assert.Equal("1.1 (437)", AppVersionText.For(1, 1, 437));
        Assert.Equal("2.0 (1)", AppVersionText.For(2, 0, 1));
    }

    /// <summary>
    /// Build 0 is what a LOCAL build carries — CI is what stamps a number — and "1.1 (0)" would
    /// be a bug report quoting a number that means "nobody stamped this".
    /// </summary>
    [Fact]
    public void AnUnstampedBuildSaysOnlyTheVersion()
    {
        Assert.Equal("1.1", AppVersionText.For(1, 1, 0));
    }

    /// <summary>Invariant digits: a Settings line is not a place for locale-specific numerals.</summary>
    [Fact]
    public void TheDigitsAreInvariant()
    {
        var previous = System.Globalization.CultureInfo.CurrentCulture;
        try
        {
            System.Globalization.CultureInfo.CurrentCulture =
                new System.Globalization.CultureInfo("ar-EG");
            Assert.Equal("1.1 (437)", AppVersionText.For(1, 1, 437));
        }
        finally
        {
            System.Globalization.CultureInfo.CurrentCulture = previous;
        }
    }
}
