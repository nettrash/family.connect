using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Media;

namespace FamilyConnect.App.Views;

/// <summary>
/// The colours this app chooses for itself, where WinUI's own would be wrong.
/// </summary>
/// <remarks>
/// <para>
/// Nearly everything drawn here comes from the theme's brushes and should: they are what makes a
/// window look like Windows. This holds the exceptions, and there is one reason for all of them —
/// <b>the surfaces the app fills with its own accent and writes its own words on</b>, above all the
/// reader's own message balloon.
/// </para>
/// <para>
/// <c>AccentFillColorDefaultBrush</c> is the deep accent shade in the light theme and the PALE one
/// in the dark theme, where the ink it pairs with (<c>TextOnAccentFillColorPrimary</c>) is BLACK.
/// For an accent button that is right and this file has no opinion about it. For a chat it is not:
/// it turned the reader's own words black in the dark theme while everybody else's stayed white,
/// and made this client disagree with the Mac about the same message. So the ink is white and the
/// ground is chosen to carry it — 8.5:1 in the light theme, 6.2:1 in the dark one, where the pale
/// shade would have been 3.7:1 and is exactly why the platform flips to black instead.
/// </para>
/// <para>
/// A colour read here is resolved when an element is built, like every other brush these views
/// read from code, so a theme changed mid-session lands on the next redraw. And every lookup is
/// defended: a key that is not there would throw inside a click, where the App's own handler marks
/// it handled and nobody sees it — this app has already lost a whole screen to one unguarded XAML
/// failure.
/// </para>
/// </remarks>
internal static class Palette
{
    /// <summary>The ink on one of the app's accent surfaces: white, in both themes.</summary>
    public static Brush Ink() => new SolidColorBrush(Windows.UI.Color.FromArgb(0xFF, 0xFF, 0xFF, 0xFF));

    /// <summary>
    /// An accent surface of the app's own — the reader's balloon, the chips it fills: the deep
    /// shade in the light theme, the accent itself in the dark one, where the deep shade would go
    /// muddy against the window.
    /// </summary>
    public static Brush Surface(ElementTheme theme) => new SolidColorBrush(SurfaceColour(theme));

    /// <summary>That same colour, for the tints and flashes drawn over it.</summary>
    public static Windows.UI.Color SurfaceColour(ElementTheme theme) =>
        theme == ElementTheme.Dark ? Accent() : Shade("SystemAccentColorDark1", 0x0D, 0x47, 0xA1);

    /// <summary>The app's accent, as set in <see cref="App.ApplyAccent"/>.</summary>
    public static Windows.UI.Color Accent() => Shade("SystemAccentColor", 0x1E, 0x5B, 0xC6);

    /// <summary>
    /// The accent as THIS THEME draws it, which is not the same colour: a tint over a CARD has to
    /// come from here, because the base blue is darker than the card it would tint in the dark
    /// theme, and a tint nobody can see is the bug this file exists to prevent.
    /// </summary>
    public static Windows.UI.Color ThemeAccent() =>
        Application.Current.Resources.TryGetValue("AccentFillColorDefaultBrush", out var value)
            && value is SolidColorBrush brush
            ? brush.Color
            : Accent();

    /// <summary>One of the accent shades the app sets for itself, or the value it sets it to.</summary>
    private static Windows.UI.Color Shade(string key, byte red, byte green, byte blue) =>
        Application.Current.Resources.TryGetValue(key, out var value) && value is Windows.UI.Color colour
            ? colour
            : Windows.UI.Color.FromArgb(0xFF, red, green, blue);
}
