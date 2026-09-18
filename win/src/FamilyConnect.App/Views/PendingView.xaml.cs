using FamilyConnect.App.Services;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml.Controls;

namespace FamilyConnect.App.Views;

/// <summary>
/// A join request waiting for an owner. The screen asks again on its own: an approval raises no
/// frame this device will hear — it has no family socket yet — so `GET /me` is how it finds out.
/// </summary>
public sealed partial class PendingView : UserControl
{
    private static readonly TimeSpan AskEvery = TimeSpan.FromSeconds(15);

    private readonly DispatcherQueueTimer timer;

    internal PendingView(AppServices services, Connection connection)
    {
        InitializeComponent();
        var say = services.Say;
        HeadingText.Text = say.Get("Almost There");
        StatusText.Text = say.Get("Waiting for approval");
        // The family's own name is data, not a sentence.
        FamilyText.Text = connection.Session.State.PendingFamilyName ?? string.Empty;
        LogOutButton.Content = say.Get("Log out");
        LogOutButton.Click += async (_, _) =>
        {
            // An async click handler that throws ends the process: written down instead.
            try
            {
                await connection.Session.SignOutAsync();
            }
            catch (Exception e)
            {
                Diagnostics.Write($"signing out: {e.GetType().Name}");
            }
        };

        timer = DispatcherQueue.CreateTimer();
        timer.Interval = AskEvery;
        timer.Tick += (_, _) => _ = connection.Session.RefreshAsync();
        Loaded += (_, _) => timer.Start();
        Unloaded += (_, _) => timer.Stop();
    }
}
