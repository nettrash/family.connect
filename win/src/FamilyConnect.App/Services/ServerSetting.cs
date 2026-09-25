using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Services;

/// <summary>Which server this app talks to. The address only — never a token.</summary>
internal static class ServerSetting
{
    private static string Path => System.IO.Path.Combine(AppFolders.Root, "server.txt");

    public static Uri? Read()
    {
        try
        {
            return File.Exists(Path) ? ServerUrl.Normalise(File.ReadAllText(Path)) : null;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"server setting could not be read: {e.GetType().Name}");
            return null;
        }
    }

    public static void Write(Uri server)
    {
        Directory.CreateDirectory(AppFolders.Root);
        File.WriteAllText(Path, server.ToString());
    }
}
