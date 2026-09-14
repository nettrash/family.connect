// The entry point. FamilyConnect.App.csproj defines DISABLE_XAML_GENERATED_MAIN and names this class
// as its StartupObject: since Windows App SDK 2.3.1 that switch renames the generated Main rather
// than deleting it, so StartupObject is what makes this one run (md.win paid for that).
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.Windows.AppLifecycle;
using Microsoft.Windows.AppNotifications;

namespace FamilyConnect.App;

internal static class Program
{
    /// <summary>One window per user: a second launch brings the first one forward and exits.</summary>
    internal const string InstanceKey = "me.nettrash.familyconnect";

    [STAThread]
    private static int Main(string[] args)
    {
        Startup.Watch();
        Startup.Step("main");
        // Everything here can fail before a window exists, and WinUI's own handlers are installed
        // inside Application.Start — so a throw here would end the process with nothing written
        // down. The log is the only thing that can tell the next person why.
        try
        {
            return Run();
        }
        catch (Exception e)
        {
            Diagnostics.Write($"startup failed before any window: {e}");
            return 1;
        }
    }

    private static int Run()
    {
        WinRT.ComWrappersSupport.InitializeComWrappers();
        var main = AppInstance.FindOrRegisterForKey(InstanceKey);
        if (!main.IsCurrent)
        {
            Redirection.RedirectAndWait(main, AppInstance.GetCurrent().GetActivatedEventArgs());
            return 0;
        }
        // Notifications, in the order the Windows App SDK requires: the handler BEFORE Register, and
        // Register before anything reads this process's activation. A click while the app is not
        // running launches it as an ordinary launch and delivers the click through the handler,
        // which may be before any window exists — so ToastActivation holds it until one does.
        AppNotificationManager.Default.NotificationInvoked += ToastActivation.Heard;
        AppNotificationManager.Default.Register();
        Startup.Step("starting WinUI");
        Application.Start(callbackParams =>
        {
            // What the XAML-generated Main installs: without it every await in a view would resume
            // on a thread-pool thread and touch the window from the wrong one.
            var context = new DispatcherQueueSynchronizationContext(DispatcherQueue.GetForCurrentThread());
            SynchronizationContext.SetSynchronizationContext(context);
            _ = new App();
        });
        return 0;
    }
}

/// <summary>
/// What a launch leaves in the log: how far it got, and the first exceptions it met, thrown or caught.
/// </summary>
/// <remarks>
/// <b>A CRASH WINUI TURNS INTO A FAIL-FAST (0xc000027b) NEVER REACHES AN UNHANDLED-EXCEPTION HANDLER</b>, and without these the
/// log is silent about why a window never appeared. Only the first 45 seconds of a launch, and at most 40 exceptions — type,
/// HRESULT and where, NEVER the message, which can carry what somebody wrote.
/// </remarks>
internal static class Startup
{
    private static readonly DateTime Until = DateTime.UtcNow.AddSeconds(45);
    private static int logged;

    [ThreadStatic]
    private static bool writing;

    public static void Watch() => AppDomain.CurrentDomain.FirstChanceException += (_, e) =>
    {
        if (writing || DateTime.UtcNow > Until || Interlocked.Increment(ref logged) > 40)
        {
            return;
        }
        writing = true;
        try
        {
            var where = new System.Diagnostics.StackTrace(1, false).GetFrames()
                .Select(frame => frame.GetMethod())
                .Where(method => method is not null)
                .Take(10)
                .Select(method => $"{method!.DeclaringType?.Name}.{method.Name}");
            Diagnostics.Write($"launch exception: {e.Exception.GetType().FullName} 0x{e.Exception.HResult:X8} at {string.Join(" < ", where)}");
        }
        catch (Exception)
        {
            // Nothing a breadcrumb does may take the launch down.
        }
        finally
        {
            writing = false;
        }
    };

    public static void Step(string what)
    {
        if (DateTime.UtcNow <= Until)
        {
            Diagnostics.Write($"launch: {what}");
        }
    }
}
