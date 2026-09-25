using FamilyConnect.App.Logic;
using FamilyConnect.App.Services;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;

namespace FamilyConnect.App.Views;

/// <summary>
/// Log in or register on one screen: a family app has exactly two entry stories — "I have an account
/// here" and "I'm new" — and both stay visible rather than one hiding behind a link.
/// </summary>
public sealed partial class SignInView : UserControl
{
    private readonly AppServices services;
    private readonly Connection connection;

    internal SignInView(AppServices services, Connection connection, Action changeServer)
    {
        this.services = services;
        this.connection = connection;
        InitializeComponent();
        var say = services.Say;

        LogInItem.Text = say.Get("Log In");
        RegisterItem.Text = say.Get("Register");
        UsernameBox.Header = say.Get("Username");
        DisplayNameBox.Header = say.Get("Display name");
        PasswordField.Header = say.Get("Password");
        RulesText.Text = say.Get(
            "Usernames are 3–32 letters, digits, dots or underscores. Passwords need at least 8 characters.");
        ChangeServerButton.Content = say.Get("Change server…");
        ServerText.Text = say.Format("Connected to %@", connection.Server.ToString());

        Modes.SelectedItem = LogInItem;
        Modes.SelectionChanged += (_, _) => Arrange();
        UsernameBox.TextChanged += (_, _) => Arrange();
        DisplayNameBox.TextChanged += (_, _) => Arrange();
        PasswordField.PasswordChanged += (_, _) => Arrange();
        // Return walks the fields, and the last one submits.
        UsernameBox.KeyDown += (_, e) => OnEnter(e, () =>
        {
            if (Registering)
            {
                DisplayNameBox.Focus(FocusState.Keyboard);
            }
            else
            {
                PasswordField.Focus(FocusState.Keyboard);
            }
        });
        DisplayNameBox.KeyDown += (_, e) => OnEnter(e, () => PasswordField.Focus(FocusState.Keyboard));
        PasswordField.KeyDown += (_, e) => OnEnter(e, () => _ = SubmitAsync());
        SubmitButton.Click += (_, _) => _ = SubmitAsync();
        ChangeServerButton.Click += (_, _) => changeServer();
        Loaded += (_, _) => UsernameBox.Focus(FocusState.Programmatic);
        Arrange();
    }

    private bool Registering => Modes.SelectedItem == RegisterItem;

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
        var registering = Registering;
        HeadingText.Text = registering ? say.Get("Join the Family") : say.Get("Welcome Back");
        DisplayNameBox.Visibility = registering ? Visibility.Visible : Visibility.Collapsed;
        RulesText.Visibility = registering && ErrorText.Visibility == Visibility.Collapsed
            ? Visibility.Visible
            : Visibility.Collapsed;
        SubmitButton.Content = Working.IsActive
            ? (registering ? say.Get("Creating account…") : say.Get("Logging in…"))
            : (registering ? say.Get("Create Account") : say.Get("Log In"));
        SubmitButton.IsEnabled = !Working.IsActive && SignInForm.MaySubmit(
            UsernameBox.Text, PasswordField.Password, DisplayNameBox.Text, registering);
    }

    private async Task SubmitAsync()
    {
        var registering = Registering;
        if (Working.IsActive
            || !SignInForm.MaySubmit(UsernameBox.Text, PasswordField.Password, DisplayNameBox.Text, registering))
        {
            return;
        }
        ErrorText.Visibility = Visibility.Collapsed;
        Working.IsActive = true;
        Arrange();
        var username = UsernameBox.Text.Trim();
        var error = registering
            ? await connection.Session.RegisterAsync(username, DisplayNameBox.Text.Trim(), PasswordField.Password)
            : await connection.Session.SignInAsync(username, PasswordField.Password);
        Working.IsActive = false;
        if (error is not null)
        {
            // On success the gate moves and the window replaces this screen; only a refusal stays.
            ErrorText.Text = DoorSentences.SignIn(error, registering, services.Say);
            ErrorText.Visibility = Visibility.Visible;
        }
        Arrange();
    }
}
