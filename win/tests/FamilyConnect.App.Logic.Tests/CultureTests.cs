using System.Globalization;
using FamilyConnect.App.Logic;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// What the app's own rules do in a reader's alphabet — which on Windows is whatever the machine
/// is set to.
/// </summary>
/// <remarks>
/// The culture is set HERE rather than by whoever launched the suite, so the check cannot be lost
/// by running the tests a different way; the collection is its own because the ambient culture is
/// process-wide and xUnit runs collections in parallel.
/// </remarks>
[Collection("culture")]
public class CultureTests : IDisposable
{
    private readonly CultureInfo was = CultureInfo.CurrentCulture;

    public void Dispose() => CultureInfo.CurrentCulture = was;

    /// <summary>
    /// THE TURKISH I. <c>"IMAGE/JPEG"</c> lower-cased in Turkish is <c>"ımage/jpeg"</c> with a
    /// dotless ı, so a type comparison that went through the reader's alphabet would refuse a
    /// perfectly good photograph because somebody's computer is set to Turkish.
    /// </summary>
    [Fact]
    public void AnImageTypeIsStillAllowedOnATurkishComputer()
    {
        CultureInfo.CurrentCulture = CultureInfo.GetCultureInfo("tr-TR");

        Assert.True(AvatarRules.IsAllowed("IMAGE/JPEG"));
        Assert.True(AvatarRules.IsAllowed("image/png"));
        Assert.False(AvatarRules.IsAllowed("image/heic"));
    }

    /// <summary>
    /// And a row's time is a DECISION here, not a format — so the model answers the same thing in
    /// any language, and the window draws it in the reader's own.
    /// </summary>
    [Fact]
    public void TheRowTimeIsTheSameDecisionInEveryLanguage()
    {
        var now = new DateTimeOffset(2026, 9, 12, 12, 0, 0, TimeSpan.Zero).ToLocalTime();
        var english = ChatListModel.When(now.AddDays(-1), now);

        CultureInfo.CurrentCulture = CultureInfo.GetCultureInfo("ja-JP");

        Assert.Equal(english, ChatListModel.When(now.AddDays(-1), now));
        Assert.Equal(RowTimeKind.Yesterday, english);
    }
}
