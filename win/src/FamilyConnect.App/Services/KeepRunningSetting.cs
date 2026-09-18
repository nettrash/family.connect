namespace FamilyConnect.App.Services;

/// <summary>
/// Whether closing the window leaves the app running in the notification area — this device's choice, kept beside the
/// notification setting. On unless the reader turned it off: an app that stops listening the moment its window closes is
/// an app that misses the call it exists to ring for.
/// </summary>
internal static class KeepRunningSetting
{
    private static string Path => System.IO.Path.Combine(AppFolders.Root, "keep-running.txt");

    public static bool Enabled
    {
        get
        {
            try
            {
                return !File.Exists(Path) || File.ReadAllText(Path).Trim() != "off";
            }
            catch (Exception e)
            {
                Diagnostics.Write($"keep-running setting could not be read: {e.GetType().Name}");
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
                Diagnostics.Write($"keep-running setting could not be written: {e.GetType().Name}");
            }
        }
    }
}
