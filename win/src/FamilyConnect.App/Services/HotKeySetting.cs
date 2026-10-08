namespace FamilyConnect.App.Services;

/// <summary>
/// Whether Ctrl+Alt+Shift+F brings Family Connect forward from any app (#80) — this device's choice, kept beside the
/// keep-running setting, and on unless the reader turned it off.
/// </summary>
internal static class HotKeySetting
{
    private static string Path => System.IO.Path.Combine(AppFolders.Root, "hot-key.txt");

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
                Diagnostics.Write($"hot-key setting could not be read: {e.GetType().Name}");
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
                Diagnostics.Write($"hot-key setting could not be written: {e.GetType().Name}");
            }
        }
    }
}
