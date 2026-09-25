using FamilyConnect.App.Services;
using Microsoft.UI.Xaml.Controls;

namespace FamilyConnect.App.Views;

/// <summary>
/// Signed in, and the server cannot be reached at launch. Not the sign-in form: a flaky network is
/// not a sign-out, and asking somebody for a password they already gave would say it was.
/// </summary>
public sealed partial class OfflineView : UserControl
{
    internal OfflineView(AppServices services, Action retry, Action changeServer)
    {
        InitializeComponent();
        var say = services.Say;
        HeadingText.Text = say.Get("Offline");
        BodyText.Text = say.Get("Can't reach the server. Check your connection.");
        RetryButton.Content = say.Get("Try Again");
        ChangeServerButton.Content = say.Get("Change server…");
        RetryButton.Click += (_, _) => retry();
        ChangeServerButton.Click += (_, _) => changeServer();
    }
}
