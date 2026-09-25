using Microsoft.Windows.AppNotifications;

namespace FamilyConnect.App;

/// <summary>
/// Clicked notifications, held until there is a window to open them in. A click that LAUNCHED the app
/// arrives before OnLaunched has made one, and a click must not be lost for being early.
/// </summary>
internal static class ToastActivation
{
    private static readonly object Gate = new();
    private static readonly Queue<IReadOnlyDictionary<string, string>> Waiting = new();
    private static Action<IReadOnlyDictionary<string, string>>? handler;

    /// <summary>Raised by Windows on a background thread.</summary>
    public static void Heard(AppNotificationManager sender, AppNotificationActivatedEventArgs args)
    {
        var arguments = new Dictionary<string, string>(args.Arguments);
        Action<IReadOnlyDictionary<string, string>>? ready;
        lock (Gate)
        {
            ready = handler;
            if (ready is null)
            {
                Waiting.Enqueue(arguments);
                return;
            }
        }
        ready(arguments);
    }

    /// <summary>The window exists: hand it what came early, then everything as it comes.</summary>
    public static void Attach(Action<IReadOnlyDictionary<string, string>> open)
    {
        List<IReadOnlyDictionary<string, string>> early;
        lock (Gate)
        {
            handler = open;
            early = [.. Waiting];
            Waiting.Clear();
        }
        foreach (var arguments in early)
        {
            open(arguments);
        }
    }
}
