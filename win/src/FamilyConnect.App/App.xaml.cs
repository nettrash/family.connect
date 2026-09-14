using FamilyConnect.App.Services;
using Microsoft.UI;
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
        Startup.Step("app XAML loaded");
        // The apps' own accent (ios AccentColor): a deep blue on a light ground, a bright one on a dark ground.
        // WinUI's accent brushes read the Dark shades in the light theme and the Light shades in the dark one. Set
        // here, before any window, because a Color resource has no XAML element xamlcheck can resolve.
        Resources["SystemAccentColor"] = ColorHelper.FromArgb(0xFF, 0x1E, 0x5B, 0xC6);
        Resources["SystemAccentColorDark1"] = ColorHelper.FromArgb(0xFF, 0x0D, 0x47, 0xA1);
        Resources["SystemAccentColorDark2"] = ColorHelper.FromArgb(0xFF, 0x0A, 0x3B, 0x86);
        Resources["SystemAccentColorDark3"] = ColorHelper.FromArgb(0xFF, 0x07, 0x2F, 0x6B);
        Resources["SystemAccentColorLight1"] = ColorHelper.FromArgb(0xFF, 0x3C, 0x6F, 0xE8);
        Resources["SystemAccentColorLight2"] = ColorHelper.FromArgb(0xFF, 0x4D, 0x7D, 0xFC);
        Resources["SystemAccentColorLight3"] = ColorHelper.FromArgb(0xFF, 0x7F, 0xA3, 0xFD);
        Startup.Step("accent set");
        // A resource key XAML could not find is written down rather than only failing somewhere inside WinUI.
        DebugSettings.XamlResourceReferenceFailed += (_, e) => Diagnostics.Write($"xaml resource: {e.Message}");
        UnhandledException += (_, e) => Diagnostics.Write($"unhandled (XAML): {e.Exception}");
        AppDomain.CurrentDomain.UnhandledException +=
            (_, e) => Diagnostics.Write($"unhandled (domain): {e.ExceptionObject}");
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        Startup.Step("launched");
        services = new AppServices();
        var shown = window = new MainWindow(services);
        Startup.Step("window built");
        // A second launch was redirected here (Program.Main): bring the one window forward. Raised
        // on a background thread, so the window is reached through its own queue.
        AppInstance.GetCurrent().Activated += (_, _) =>
            shown.DispatcherQueue.TryEnqueue(() => shown.Activate());
        // A clicked notification opens its chat — including the click that launched the app.
        ToastActivation.Attach(arguments =>
            shown.DispatcherQueue.TryEnqueue(() => shown.OpenFromToast(arguments)));
        shown.Activate();
        Startup.Step("window shown");
    }
}
