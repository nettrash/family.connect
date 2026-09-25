using System.Globalization;

namespace FamilyConnect.App.Logic;

/// <summary>
/// How the app says which build it is, on the one line in Settings that says so.
/// </summary>
/// <remarks>
/// <para>
/// An MSIX version is <c>Major.Minor.Build.Revision</c>, every part a 16-bit number, and the
/// Store RESERVES the revision — a package whose fourth part is not 0 is refused at Partner
/// Center. So the auto-incrementing build number lives in the THIRD part, which means it is
/// also the part a person would read as a semantic patch release if it were printed inline.
/// It is not one: <c>1.1.437</c> is version 1.1, build 437.
/// </para>
/// <para>
/// So the line reads the way the Apple apps' does — "1.1 (437)", the version people talk about
/// and then the build in brackets — and drops the brackets entirely at build 0, which is what a
/// local build carries (CI is what stamps a number; see the <c>win-app</c> job). A bare "1.1 (0)"
/// would be a bug report quoting a number that means "nobody stamped this".
/// </para>
/// </remarks>
public static class AppVersionText
{
    /// <summary>The version line's value: "1.1 (437)", or "1.1" for an unstamped build.</summary>
    public static string For(int major, int minor, int build) =>
        build > 0
            ? string.Create(CultureInfo.InvariantCulture, $"{major}.{minor} ({build})")
            : string.Create(CultureInfo.InvariantCulture, $"{major}.{minor}");
}
