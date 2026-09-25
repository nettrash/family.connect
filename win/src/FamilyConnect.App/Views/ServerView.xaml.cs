using FamilyConnect.App.Logic;
using FamilyConnect.App.Services;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace FamilyConnect.App.Views;

/// <summary>First run: point the app at the family's server.</summary>
public sealed partial class ServerView : UserControl
{
    private readonly AppServices services;
    private readonly Func<Uri, Task> useServer;

    internal ServerView(AppServices services, Uri? prefill, Func<Uri, Task> useServer)
    {
        this.services = services;
        this.useServer = useServer;
        InitializeComponent();
        var say = services.Say;

        // The product's name, not a sentence: it is the same in every language.
        HeadingText.Text = "Family Connect";
        AddressBox.Header = say.Get("Server address");
        HelpText.Text = say.Get("Ask the family member who runs the server for its address.");
        WarningText.Text = say.Get(
            "This address uses plain http. That's fine for a server on your home network, but anyone on the same network can read the traffic.");
        ConnectButton.Content = say.Get("Connect");

        AddressBox.Text = prefill?.ToString() ?? string.Empty;
        AddressBox.TextChanged += (_, _) => Assess();
        AddressBox.KeyDown += (_, e) =>
        {
            if (e.Key == Windows.System.VirtualKey.Enter)
            {
                e.Handled = true;
                _ = ConnectAsync();
            }
        };
        ConnectButton.Click += (_, _) => _ = ConnectAsync();
        Loaded += (_, _) => AddressBox.Focus(FocusState.Programmatic);
        Assess();
    }

    private void Assess()
    {
        var url = ServerCheck.Parse(AddressBox.Text);
        WarningText.Visibility = url is not null && ServerCheck.IsPlainText(url)
            ? Visibility.Visible
            : Visibility.Collapsed;
        ConnectButton.IsEnabled = !Working.IsActive && !string.IsNullOrWhiteSpace(AddressBox.Text);
    }

    private async Task ConnectAsync()
    {
        if (Working.IsActive || string.IsNullOrWhiteSpace(AddressBox.Text))
        {
            return;
        }
        var say = services.Say;
        ErrorText.Visibility = Visibility.Collapsed;
        var url = ServerCheck.Parse(AddressBox.Text);
        var answers = false;
        if (url is not null)
        {
            Busy(true);
            try
            {
                answers = await ServerCheck.AnswersAsync(services.Probe, url);
            }
            catch (Exception e)
            {
                Diagnostics.Write($"server probe: {e.GetType().Name}");
            }
            finally
            {
                Busy(false);
            }
        }
        if (url is null || !answers)
        {
            ErrorText.Text = say.Format(
                "No Family Connect server answered at %@. Check the address and your network.",
                url?.ToString() ?? AddressBox.Text.Trim());
            ErrorText.Visibility = Visibility.Visible;
            return;
        }
        await useServer(url);
    }

    private void Busy(bool busy)
    {
        Working.IsActive = busy;
        ConnectButton.Content = busy ? services.Say.Get("Checking…") : services.Say.Get("Connect");
        Assess();
    }
}
