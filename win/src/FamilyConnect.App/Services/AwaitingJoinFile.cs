using FamilyConnect.App.Logic;

namespace FamilyConnect.App.Services;

/// <summary>
/// The join request this device is waiting on, kept on disk with the server it was asked of — so a relaunch after the owner
/// said no still tells the reader so on the family door.
/// </summary>
internal sealed class AwaitingJoinFile(Uri server) : IAwaitingJoin
{
    private static string Path => System.IO.Path.Combine(AppFolders.Root, "awaiting-join.txt");

    public long? UserId
    {
        get
        {
            try
            {
                if (!File.Exists(Path))
                {
                    return null;
                }
                var lines = File.ReadAllLines(Path);
                return lines.Length == 2 && lines[0] == server.AbsoluteUri
                    && long.TryParse(lines[1], System.Globalization.NumberStyles.None, System.Globalization.CultureInfo.InvariantCulture, out var id)
                    ? id
                    : null;
            }
            catch (Exception e)
            {
                Diagnostics.Write($"awaiting join could not be read: {e.GetType().Name}");
                return null;
            }
        }
        set
        {
            try
            {
                if (value is { } id)
                {
                    Directory.CreateDirectory(AppFolders.Root);
                    File.WriteAllLines(Path, [server.AbsoluteUri, id.ToString(System.Globalization.CultureInfo.InvariantCulture)]);
                }
                else if (File.Exists(Path))
                {
                    File.Delete(Path);
                }
            }
            catch (Exception e)
            {
                Diagnostics.Write($"awaiting join could not be written: {e.GetType().Name}");
            }
        }
    }
}
