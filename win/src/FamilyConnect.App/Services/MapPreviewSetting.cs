namespace FamilyConnect.App.Services;

/// <summary>
/// Whether a shared location draws a map (ios <c>AppSettings.mapPreviewsEnabled</c>) — this device's choice, on unless the
/// reader turned it off, as on the Mac. Drawing one asks OpenStreetMap for the tiles around the place, which is why it can be
/// switched off at all; off, the bubble keeps its pin, its name and the way into a map the reader opens themselves.
/// </summary>
internal static class MapPreviewSetting
{
    private static readonly object Gate = new();
    private static bool? known;

    /// <summary>The switch moved: every location drawn with the old answer is drawn again.</summary>
    public static event Action? Changed;

    private static string Path => System.IO.Path.Combine(AppFolders.Root, "map-previews.txt");

    /// <summary>Asked for every location on every redraw, so read from disk once and remembered.</summary>
    public static bool Enabled
    {
        get
        {
            lock (Gate)
            {
                if (known is { } value)
                {
                    return value;
                }
                try
                {
                    known = !File.Exists(Path) || File.ReadAllText(Path).Trim() != "off";
                }
                catch (Exception e)
                {
                    Diagnostics.Write($"map preview setting could not be read: {e.GetType().Name}");
                    known = true;
                }
                return known.Value;
            }
        }
        set
        {
            lock (Gate)
            {
                known = value;
                try
                {
                    Directory.CreateDirectory(AppFolders.Root);
                    File.WriteAllText(Path, value ? "on" : "off");
                }
                catch (Exception e)
                {
                    Diagnostics.Write($"map preview setting could not be written: {e.GetType().Name}");
                }
            }
            Changed?.Invoke();
        }
    }
}
