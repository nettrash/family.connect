namespace FamilyConnect.App.Services;

/// <summary>Where this user's copy of the app keeps its files.</summary>
/// <remarks>
/// LocalApplicationData rather than ApplicationData.Current: the latter needs package identity and
/// throws in an unpackaged run, while a packaged app's LocalApplicationData is already redirected
/// into its own package folder — so one path is right both ways.
/// </remarks>
internal static class AppFolders
{
    public static string Root { get; } = Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "FamilyConnect");

    /// <summary>The SQLite cache: history, the board, and the outbox this device still owes.</summary>
    public static string CachePath
    {
        get
        {
            Directory.CreateDirectory(Root);
            return Path.Combine(Root, "cache.db");
        }
    }
}
