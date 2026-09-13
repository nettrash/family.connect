using FamilyConnect.App.Logic;
using FamilyConnect.App.Services;
using FamilyConnect.App.Views;
using FamilyConnect.Core.Protocol;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Media;

namespace FamilyConnect.App;

/// <summary>
/// The one window: which screen is showing is the session's gate, and nothing else decides it.
/// </summary>
public sealed partial class MainWindow : Window
{
    private readonly AppServices services;
    private readonly DispatcherQueueTimer flushTimer;
    private Connection? connection;
    private ChatsView? chats;
    private Gate? shown;

    internal MainWindow(AppServices services)
    {
        this.services = services;
        InitializeComponent();

        // The product's name, which is the same in every language.
        Title = "Family Connect";
        TitleText.Text = "Family Connect";
        ExtendsContentIntoTitleBar = true;
        SetTitleBar(TitleBar);
        SystemBackdrop = new MicaBackdrop();
        AppWindow.Resize(new Windows.Graphics.SizeInt32(1100, 760));
        WindowIcon.Apply(AppWindow);

        // The outbox retries on its own schedule (SendRules); something has to ask it to.
        flushTimer = DispatcherQueue.CreateTimer();
        flushTimer.Interval = TimeSpan.FromSeconds(5);
        flushTimer.Tick += (_, _) => Flush(SendRules.FlushTrigger.Timer);
        flushTimer.Start();

        Activated += OnActivated;
        Closed += OnClosed;

        if (services.SavedServer is { } server)
        {
            _ = UseServerAsync(server);
        }
        else
        {
            ShowServer(prefill: null);
        }
    }

    private async Task UseServerAsync(Uri server)
    {
        Detach();
        var next = await services.UseAsync(server);
        connection = next;
        next.Session.Changed += OnSessionChanged;
        if (!next.HasToken)
        {
            Show(Gate.SignedOut);
            return;
        }
        Screen.Content = null;
        await RefreshAsync(next);
    }

    /// <summary>
    /// A stored token is not a session: `GET /me` decides. But A FLAKY NETWORK IS NOT A SIGN-OUT —
    /// a launch that cannot reach the server shows that, with a way to try again, rather than a
    /// sign-in form for somebody who is signed in.
    /// </summary>
    private async Task RefreshAsync(Connection current)
    {
        var error = await current.Session.RefreshAsync();
        if (current != connection)
        {
            return;
        }
        if (error is { SessionGone: false }
            && current.Session.State.Gate == Gate.SignedOut
            && current.HasToken)
        {
            shown = null;
            Screen.Content = new OfflineView(
                services,
                retry: () => _ = RefreshAsync(current),
                changeServer: () => ShowServer(current.Server));
            return;
        }
        Show(current.Session.State.Gate);
    }

    private void OnSessionChanged(SessionState state) =>
        DispatcherQueue.TryEnqueue(() => Show(state.Gate));

    private void Show(Gate gate)
    {
        if (connection is not { } current || gate == shown)
        {
            return;
        }
        shown = gate;
        if (gate is not (Gate.Member or Gate.Owner))
        {
            chats?.Detach();
            chats = null;
        }
        Screen.Content = gate switch
        {
            Gate.NoFamily => new DoorView(services, current),
            Gate.Pending => new PendingView(services, current),
            Gate.Member or Gate.Owner => chats ??= new ChatsView(services, current),
            _ => new SignInView(services, current, changeServer: () => ShowServer(current.Server)),
        };
    }

    private void ShowServer(Uri? prefill)
    {
        shown = null;
        Screen.Content = new ServerView(services, prefill, UseServerAsync);
    }

    private void Detach()
    {
        chats?.Detach();
        chats = null;
        shown = null;
        if (connection is { } old)
        {
            old.Session.Changed -= OnSessionChanged;
        }
        connection = null;
    }

    private void Flush(SendRules.FlushTrigger trigger)
    {
        if (connection is { } current && current.Session.State.CanChat)
        {
            _ = current.Live.FlushAsync(trigger);
        }
    }

    private void OnActivated(object sender, WindowActivatedEventArgs args)
    {
        services.Foreground = args.WindowActivationState != WindowActivationState.Deactivated;
        if (services.Foreground)
        {
            Flush(SendRules.FlushTrigger.WindowActivated);
            chats?.ReaderReturned();
        }
    }

    private void OnClosed(object sender, WindowEventArgs args)
    {
        flushTimer.Stop();
        Detach();
        _ = services.DisposeAsync().AsTask();
    }
}
