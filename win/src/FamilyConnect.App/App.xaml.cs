using FamilyConnect.App.Services;
using Microsoft.UI.Xaml;
using Microsoft.Windows.AppLifecycle;

namespace FamilyConnect.App;

public partial class App : Application
{
    private AppServices? services;
    private MainWindow? window;

    public App()
    {
        InitializeComponent();
        UnhandledException += (_, e) => Diagnostics.Write($"unhandled (XAML): {e.Exception}");
        AppDomain.CurrentDomain.UnhandledException +=
            (_, e) => Diagnostics.Write($"unhandled (domain): {e.ExceptionObject}");
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        services = new AppServices();
        var shown = window = new MainWindow(services);
        // A second launch was redirected here (Program.Main): bring the one window forward. Raised
        // on a background thread, so the window is reached through its own queue.
        AppInstance.GetCurrent().Activated += (_, _) =>
            shown.DispatcherQueue.TryEnqueue(() => shown.Activate());
        shown.Activate();
    }
}
