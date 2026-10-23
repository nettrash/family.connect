using System.Globalization;
using FamilyConnect.App.Logic;

namespace FamilyConnect.App.Services;

/// <summary>
/// Which family this device last showed each account on this server (docs/protocol.md, "Several families": the current family
/// is "persisted per account on the device"), kept on disk beside the cache — and NOT in it, because the cache is wiped at
/// every sign-in and the reader signing back in expects to land where they were.
/// </summary>
/// <remarks>One line per account: the server, the account's id and the family's, tab-separated. Nothing else is in it.</remarks>
internal sealed class CurrentFamilyFile(Uri server) : ICurrentFamily
{
    private static string Path => System.IO.Path.Combine(AppFolders.Root, "current-family.txt");

    public long? For(long userId)
    {
        try
        {
            return Read().TryGetValue(userId, out var family) ? family : null;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"the current family could not be read: {e.GetType().Name}");
            return null;
        }
    }

    public void Remember(long userId, long familyId)
    {
        try
        {
            var held = Read();
            if (held.TryGetValue(userId, out var family) && family == familyId)
            {
                return;
            }
            held[userId] = familyId;
            var others = File.Exists(Path)
                ? File.ReadAllLines(Path).Where(line => line.Split('\t') is not [var host, _, _] || host != server.AbsoluteUri)
                : [];
            Directory.CreateDirectory(AppFolders.Root);
            File.WriteAllLines(Path, others.Concat(held.Select(pair => string.Join('\t',
                server.AbsoluteUri,
                pair.Key.ToString(CultureInfo.InvariantCulture),
                pair.Value.ToString(CultureInfo.InvariantCulture)))));
        }
        catch (Exception e)
        {
            Diagnostics.Write($"the current family could not be written: {e.GetType().Name}");
        }
    }

    /// <summary>This server's lines, by account.</summary>
    private Dictionary<long, long> Read()
    {
        var held = new Dictionary<long, long>();
        if (!File.Exists(Path))
        {
            return held;
        }
        foreach (var line in File.ReadAllLines(Path))
        {
            if (line.Split('\t') is [var host, var user, var family]
                && host == server.AbsoluteUri
                && long.TryParse(user, NumberStyles.None, CultureInfo.InvariantCulture, out var userId)
                && long.TryParse(family, NumberStyles.None, CultureInfo.InvariantCulture, out var familyId))
            {
                held[userId] = familyId;
            }
        }
        return held;
    }
}
