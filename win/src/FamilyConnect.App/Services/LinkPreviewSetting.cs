namespace FamilyConnect.App.Services;

/// <summary>
/// Whether the reader wants a card under links (ios <c>AppSettings.linkPreviewsEnabled</c>) — this device's choice. On unless
/// they turned it off, as on the phones. Building a card asks the linked website for it, which is why it can be switched off at
/// all; the answer is the same for every row and every sender.
/// </summary>
internal static class LinkPreviewSetting
{
    private static readonly object Gate = new();
    private static bool? known;

    /// <summary>The switch moved: every bubble drawn with the old answer is drawn again.</summary>
    public static event Action? Changed;

    private static string Path => System.IO.Path.Combine(AppFolders.Root, "link-previews.txt");

    /// <summary>Asked for every bubble on every redraw, so read from disk once and remembered.</summary>
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
                    Diagnostics.Write($"link preview setting could not be read: {e.GetType().Name}");
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
                    Diagnostics.Write($"link preview setting could not be written: {e.GetType().Name}");
                }
            }
            Changed?.Invoke();
        }
    }
}
