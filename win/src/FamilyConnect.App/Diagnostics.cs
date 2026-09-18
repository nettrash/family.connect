namespace FamilyConnect.App;

/// <summary>
/// A line-per-event log for the failures nothing else can show — a crash before the window, a
/// packaging slip, a handler that threw.
/// </summary>
/// <remarks>
/// <b>NEVER A MESSAGE, NEVER A TOKEN.</b> This file sits in the user's profile, and a family's
/// words and the key to their account have no business in it. Log what happened, not what was said.
/// </remarks>
internal static class Diagnostics
{
    private static readonly object Writing = new();

    public static void Write(string line)
    {
        try
        {
            lock (Writing)
            {
                Directory.CreateDirectory(Services.AppFolders.Root);
                // "O" is the round-trip format and culture-invariant: a Finnish machine writes the
                // same instant as an American one.
                File.AppendAllText(
                    Path.Combine(Services.AppFolders.Root, "diagnostics.log"),
                    $"{DateTimeOffset.UtcNow:O} {line}{Environment.NewLine}");
            }
        }
        catch (Exception)
        {
            // A log that cannot be written must never take the app down with it.
        }
    }
}
