using FamilyConnect.App.Logic;

namespace FamilyConnect.App.Services;

/// <summary>
/// The file behind <see cref="RoundCrashGuard"/>: written before a circle starts playing in place, removed when the player is
/// let go, and read once at launch. THIS build is the compiled assembly's module id, so every new build tries in place again.
/// </summary>
internal static class RoundCrashMark
{
    private static readonly string Build = typeof(RoundCrashMark).Assembly.ManifestModule.ModuleVersionId.ToString("N");

    private static string Where => Path.Combine(AppFolders.Root, RoundCrashGuard.FileName);

    private static bool armed;

    /// <summary>Whether this build died playing a circle in place last time (and so opens the viewer from now on).</summary>
    public static bool DiedLastTime()
    {
        try
        {
            return File.Exists(Where) && RoundCrashGuard.Tripped(File.ReadAllText(Where), Build);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"reading the in-place mark: {e.GetType().Name}");
            return false;
        }
    }

    /// <summary>A circle is about to play in place.</summary>
    public static void Arm()
    {
        if (armed)
        {
            return;
        }
        try
        {
            Directory.CreateDirectory(AppFolders.Root);
            File.WriteAllText(Where, RoundCrashGuard.Mark(Build));
            armed = true;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"writing the in-place mark: {e.GetType().Name}");
        }
    }

    /// <summary>The player was let go: the app lived through it.</summary>
    public static void Disarm()
    {
        if (!armed)
        {
            return;
        }
        try
        {
            File.Delete(Where);
            armed = false;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"clearing the in-place mark: {e.GetType().Name}");
        }
    }
}
