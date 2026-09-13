// The entry point. FamilyConnect.App.csproj defines DISABLE_XAML_GENERATED_MAIN and names this class
// as its StartupObject: since Windows App SDK 2.3.1 that switch renames the generated Main rather
// than deleting it, so StartupObject is what makes this one run (md.win paid for that).
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.Windows.AppLifecycle;

namespace FamilyConnect.App;

internal static class Program
{
    /// <summary>One window per user: a second launch brings the first one forward and exits.</summary>
    internal const string InstanceKey = "me.nettrash.familyconnect";

    [STAThread]
    private static int Main(string[] args)
    {
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
        var activation = AppInstance.GetCurrent().GetActivatedEventArgs();
        var main = AppInstance.FindOrRegisterForKey(InstanceKey);
        if (!main.IsCurrent)
        {
            Redirection.RedirectAndWait(main, activation);
            return 0;
        }
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
