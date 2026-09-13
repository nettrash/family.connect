using FamilyConnect.App.Logic;
using FamilyConnect.App.Services;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;

namespace FamilyConnect.App.Views;

/// <summary>Signed in and in no family: join one with a code, or — where the server allows — start one.</summary>
public sealed partial class DoorView : UserControl
{
    private readonly AppServices services;
    private readonly FamilyDoor door;

    internal DoorView(AppServices services, Connection connection)
    {
        this.services = services;
        door = new FamilyDoor(connection.Api, connection.Session, services.Say);
        InitializeComponent();
        var say = services.Say;

        JoinHeading.Text = say.Get("Join a Family");
        CodeBox.Header = say.Get("Invite code");
        CodeHelp.Text = say.Get("Any family member can read the code to you; the owner finds it in Settings.");
        CreateHeading.Text = say.Get("Create a Family");
        NameBox.Header = say.Get("Family name");
        NameHelp.Text = say.Get("This names your family chat too. 1–64 characters.");
        LogOutButton.Content = say.Get("Log Out");
        // The door a closed server has shut is not drawn at all (docs/protocol.md, "Starting a family").
        CreatePanel.Visibility = connection.Session.State.FamilyRegistrationEnabled
            ? Visibility.Visible
            : Visibility.Collapsed;

        CodeBox.TextChanged += (_, _) => Arrange();
        NameBox.TextChanged += (_, _) => Arrange();
        CodeBox.KeyDown += (_, e) => OnEnter(e, () => _ = JoinAsync());
        NameBox.KeyDown += (_, e) => OnEnter(e, () => _ = CreateAsync());
        JoinButton.Click += (_, _) => _ = JoinAsync();
        CreateButton.Click += (_, _) => _ = CreateAsync();
        LogOutButton.Click += async (_, _) => await connection.Session.SignOutAsync();
        Loaded += (_, _) => CodeBox.Focus(FocusState.Programmatic);
        Arrange();
    }

    private static void OnEnter(KeyRoutedEventArgs e, Action then)
    {
        if (e.Key == Windows.System.VirtualKey.Enter)
        {
            e.Handled = true;
            then();
        }
    }

    private void Arrange()
    {
        var say = services.Say;
        JoinButton.Content = JoinWorking.IsActive ? say.Get("Joining…") : say.Get("Join");
        CreateButton.Content = CreateWorking.IsActive ? say.Get("Creating…") : say.Get("Create Family");
        var busy = JoinWorking.IsActive || CreateWorking.IsActive;
        JoinButton.IsEnabled = !busy && FamilyDoor.MayJoin(CodeBox.Text);
        CreateButton.IsEnabled = !busy && FamilyDoor.MayCreate(NameBox.Text);
    }

    private async Task JoinAsync()
    {
        if (JoinWorking.IsActive || CreateWorking.IsActive || !FamilyDoor.MayJoin(CodeBox.Text))
        {
            return;
        }
        JoinError.Visibility = Visibility.Collapsed;
        JoinWorking.IsActive = true;
        Arrange();
        var refused = await door.JoinAsync(CodeBox.Text);
        JoinWorking.IsActive = false;
        Show(JoinError, refused);
        Arrange();
    }

    private async Task CreateAsync()
    {
        if (JoinWorking.IsActive || CreateWorking.IsActive || !FamilyDoor.MayCreate(NameBox.Text))
        {
            return;
        }
        CreateError.Visibility = Visibility.Collapsed;
        CreateWorking.IsActive = true;
        Arrange();
        var refused = await door.CreateAsync(NameBox.Text);
        CreateWorking.IsActive = false;
        Show(CreateError, refused);
        Arrange();
    }

    private static void Show(TextBlock where, string? refused)
    {
        where.Text = refused ?? string.Empty;
        where.Visibility = refused is null ? Visibility.Collapsed : Visibility.Visible;
    }
}
