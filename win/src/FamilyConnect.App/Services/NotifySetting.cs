namespace FamilyConnect.App.Services;

/// <summary>
/// Whether the reader wants a notification when a message arrives — this device's choice, kept
/// beside the server setting. On unless they turned it off: Windows itself can still refuse, and
/// <see cref="Toasts.Available"/> is asked as well.
/// </summary>
internal static class NotifySetting
{
    private static string Path => System.IO.Path.Combine(AppFolders.Root, "notifications.txt");

    public static bool Wanted
    {
        get
        {
            try
            {
                return !File.Exists(Path) || File.ReadAllText(Path).Trim() != "off";
            }
            catch (Exception e)
            {
                Diagnostics.Write($"notification setting could not be read: {e.GetType().Name}");
                return true;
            }
        }
        set
        {
            try
            {
                Directory.CreateDirectory(AppFolders.Root);
                File.WriteAllText(Path, value ? "on" : "off");
            }
            catch (Exception e)
            {
                Diagnostics.Write($"notification setting could not be written: {e.GetType().Name}");
            }
        }
    }
}
