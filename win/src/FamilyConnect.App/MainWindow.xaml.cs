using FamilyConnect.App.Logic;
using FamilyConnect.App.Services;
using FamilyConnect.App.Views;
using FamilyConnect.Core.Protocol;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
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

    /// <summary>Calls: the media page and the card are the window's, kept across servers; the engine and its frames are the connection's.</summary>
    private readonly Services.WebViewCallMedia callMedia;
    private readonly Views.CallCardView callCard;
    private FamilyConnect.App.Logic.CallEngine? calls;
    private Action<FamilyConnect.Core.Protocol.ServerFrame>? onCall;
    private string? toastedCall;
    private Attention? attention;
    private int attentionQueued;
    private ChatsView? chats;
    private Gate? shown;
    private NavigationViewItem chatsItem = null!;
    private NavigationViewItem boardItem = null!;
    private NavigationViewItem familyItem = null!;
    private NavigationViewItem settingsItem = null!;
    private bool selecting;

    internal MainWindow(AppServices services)
    {
        this.services = services;
        InitializeComponent();
        Startup.Step("window XAML loaded");
        services.WindowHandle = WinRT.Interop.WindowNative.GetWindowHandle(this);

        // The product's name, which is the same in every language.
        Title = "Family Connect";
        TitleText.Text = "Family Connect";
        ExtendsContentIntoTitleBar = true;
        SetTitleBar(TitleBar);
        SystemBackdrop = new MicaBackdrop();
        AppWindow.Resize(new Windows.Graphics.SizeInt32(1100, 760));
        WindowIcon.Apply(AppWindow);
        BuildRail();
        callMedia = new Services.WebViewCallMedia(CallMediaView);
        callCard = new Views.CallCardView(services, CallCard, CallFace, CallName, CallStatus, CallPicture, CallActions, DispatcherQueue);
        Startup.Step("call card ready");

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
        // The calls of this server: its frames in, on this thread, and the card and the chats told of every change.
        var engine = new FamilyConnect.App.Logic.CallEngine(next.Socket, ct => next.Api.IceServers(ct), callMedia);
        calls = engine;
        onCall = frame => DispatcherQueue.TryEnqueue(() => engine.Hear(frame));
        next.Router.Call += onCall;
        engine.Changed += () => DispatcherQueue.TryEnqueue(() => ShowCall(engine));
        callCard.Attach(next, engine);
        attention = new Attention(services, next, QueueAttention);
        RefreshAttention();
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
            Nav.IsPaneVisible = false;
            Screen.Content = new OfflineView(
                services,
                retry: () => _ = RefreshAsync(current),
                changeServer: () => ShowServer(current.Server));
            return;
        }
        Show(current.Session.State.Gate);
    }

    private void OnSessionChanged(SessionState state) =>
        DispatcherQueue.TryEnqueue(() =>
        {
            Show(state.Gate);
            RefreshAttention();
        });

    /// <summary>The unread count may have moved, on whatever thread noticed: one refresh, on this one.</summary>
    private void QueueAttention()
    {
        if (Interlocked.Exchange(ref attentionQueued, 1) == 1)
        {
            return;
        }
        DispatcherQueue.TryEnqueue(() =>
        {
            Interlocked.Exchange(ref attentionQueued, 0);
            RefreshAttention();
        });
    }

    /// <summary>The count in the title and on the taskbar icon — and neither while nobody is chatting.</summary>
    private void RefreshAttention()
    {
        var chatting = connection?.Session.State.CanChat == true && attention is not null;
        var title = chatting ? attention!.Title : "Family Connect";
        Title = title;
        TitleText.Text = title;
        Toasts.Badge(chatting ? attention!.Unread : 0);
        Badge(chatsItem, chatting ? attention!.Unread : 0);
        Badge(boardItem, chatting && connection is { } live ? live.Board.Unread() : 0);
    }

    /// <summary>A clicked notification: to the front, and to the chat it was about.</summary>
    internal void OpenFromToast(IReadOnlyDictionary<string, string> arguments)
    {
        Activate();
        if (ToastArguments.Parse(arguments) is { ChatId: { } chatId })
        {
            ShowChats();
            chats?.OpenChat(chatId);
        }
    }

    private void Show(Gate gate)
    {
        if (connection is not { } current || gate == shown)
        {
            return;
        }
        shown = gate;
        Startup.Step($"screen {gate}");
        var member = gate is Gate.Member or Gate.Owner;
        if (!member)
        {
            chats?.Detach();
            chats = null;
        }
        // The rail's places are a family's: before there is one, the door or the sign-in fills the window.
        Nav.IsPaneVisible = member;
        if (member)
        {
            Select(chatsItem);
        }
        Screen.Content = gate switch
        {
            Gate.NoFamily => new DoorView(services, current),
            Gate.Pending => new PendingView(services, current),
            Gate.Member or Gate.Owner => chats ??= NewChats(current),
            _ => new SignInView(services, current, changeServer: () => ShowServer(current.Server)),
        };
    }

    /// <summary>
    /// Settings, over the chats: they go on listening underneath, and Done puts them back — unless
    /// the gate moved meanwhile (a family left, an account deleted), which has already replaced both.
    /// </summary>
    private void ShowSettings()
    {
        if (connection is not { } current || chats is null)
        {
            return;
        }
        Screen.Content = new SettingsView(services, current, close: () =>
        {
            if (connection == current && chats is { } open && shown is Gate.Member or Gate.Owner)
            {
                Select(chatsItem);
                Screen.Content = open;
            }
        });
    }

    /// <summary>
    /// The family, over the chats as Settings is. "Message" on a member puts the chats back and opens
    /// that conversation in them.
    /// </summary>
    private void ShowFamily()
    {
        if (connection is not { } current || chats is null)
        {
            return;
        }
        void Back()
        {
            if (connection == current && chats is { } open && shown is Gate.Member or Gate.Owner)
            {
                Select(chatsItem);
                Screen.Content = open;
            }
        }
        Screen.Content = new FamilyView(services, current, close: Back, openChat: chatId =>
        {
            Back();
            chats?.OpenChat(chatId);
        });
    }

    /// <summary>
    /// The wall, over the chats as Settings is. Drawn, it moves the board's marks, and the badge follows. A name in an
    /// opened note puts the chats back and opens that conversation, as "Message" in the family does.
    /// </summary>
    private void ShowBoard()
    {
        if (connection is not { } current || chats is null)
        {
            return;
        }
        void Back()
        {
            if (connection == current && chats is { } open && shown is Gate.Member or Gate.Owner)
            {
                Select(chatsItem);
                Screen.Content = open;
            }
        }
        Screen.Content = new BoardView(services, current, close: Back, shown: RefreshAttention, openChat: chatId =>
        {
            Back();
            chats?.OpenChat(chatId);
        });
    }

    /// <summary>
    /// The rail: the chats, the board, the family, and settings at its foot — each drawn with its Segoe Fluent glyph
    /// and named by the catalogue, the name doubling as the tooltip a compact rail needs.
    /// </summary>
    private void BuildRail()
    {
        var say = services.Say;
        NavigationViewItem Item(string tag, string words, int glyph)
        {
            var item = new NavigationViewItem { Content = words, Tag = tag, Icon = new FontIcon { Glyph = ((char)glyph).ToString() } };
            ToolTipService.SetToolTip(item, words);
            return item;
        }
        chatsItem = Item("chats", say.Get("Chats"), 0xE8BD);
        boardItem = Item("board", say.Get("Board"), 0xE840);
        familyItem = Item("family", say.Get("Family"), 0xE716);
        settingsItem = Item("settings", say.Get("Settings"), 0xE713);
        Nav.MenuItems.Add(chatsItem);
        Nav.MenuItems.Add(boardItem);
        Nav.MenuItems.Add(familyItem);
        Nav.FooterMenuItems.Add(settingsItem);
        Nav.SelectionChanged += (_, args) =>
        {
            if (selecting)
            {
                return;
            }
            switch ((args.SelectedItem as NavigationViewItem)?.Tag as string)
            {
                case "chats":
                    ShowChats();
                    break;
                case "board":
                    ShowBoard();
                    break;
                case "family":
                    ShowFamily();
                    break;
                case "settings":
                    ShowSettings();
                    break;
            }
        };
    }

    /// <summary>The rail's selection, moved without being taken for a click on it.</summary>
    private void Select(NavigationViewItem item)
    {
        selecting = true;
        try
        {
            Nav.SelectedItem = item;
        }
        finally
        {
            selecting = false;
        }
    }

    private static void Badge(NavigationViewItem item, int count) =>
        item.InfoBadge = count > 0 ? new InfoBadge { Value = count } : null;

    /// <summary>Back to the chats, which went on listening while another place was in front.</summary>
    private void ShowChats()
    {
        if (connection is not null && chats is { } open && shown is Gate.Member or Gate.Owner)
        {
            Select(chatsItem);
            Screen.Content = open;
        }
    }

    private void ShowServer(Uri? prefill)
    {
        shown = null;
        Nav.IsPaneVisible = false;
        Screen.Content = new ServerView(services, prefill, UseServerAsync);
    }

    private void Detach()
    {
        if (calls is { } engine)
        {
            // A window leaving its server leaves its call: the far side is told, and the microphone goes out.
            _ = engine.LeaveAsync();
            engine.Detach();
            calls = null;
        }
        callCard.Attach(null, null);
        chats?.Detach();
        chats = null;
        shown = null;
        attention?.Dispose();
        attention = null;
        if (connection is { } old)
        {
            old.Session.Changed -= OnSessionChanged;
            if (onCall is not null)
            {
                old.Router.Call -= onCall;
            }
        }
        onCall = null;
        connection = null;
    }

    /// <summary>The chats, with their call buttons wired to this window's calls — and told whether one is on.</summary>
    private ChatsView NewChats(Connection current)
    {
        var view = new ChatsView(services, current);
        view.CallRequested += (chatId, peerUserId, video) => _ = calls?.PlaceAsync(chatId, peerUserId, video);
        view.ShowCallBusy(calls?.Busy == true);
        return view;
    }

    /// <summary>
    /// A call changed: the card, the chats' buttons — and, for a call ringing while the window is behind others, a
    /// notification that brings it forward, taken away again once the ringing stops.
    /// </summary>
    private void ShowCall(FamilyConnect.App.Logic.CallEngine engine)
    {
        if (engine != calls)
        {
            return;
        }
        callCard.Draw();
        chats?.ShowCallBusy(engine.Busy);
        var call = engine.Call;
        if (toastedCall is { } toasted && (call is null || call.CallId != toasted || call.Stage != FamilyConnect.App.Logic.CallStage.Incoming))
        {
            Toasts.Clear(CallTag(toasted));
            toastedCall = null;
        }
        if (call is { Stage: FamilyConnect.App.Logic.CallStage.Incoming } ringing && toastedCall is null && !services.Foreground && connection is { } current)
        {
            toastedCall = ringing.CallId;
            var say = services.Say;
            var who = current.Chats.Member(ringing.PeerUserId)?.DisplayName is { Length: > 0 } display ? display : say.Get("Someone");
            Toasts.Show(new FamilyConnect.App.Logic.Toast(
                CallTag(ringing.CallId), who, ringing.Video ? say.Get("Incoming video call") : say.Get("Incoming call"), ringing.ChatId));
        }
    }

    private static string CallTag(string callId) => $"call-{callId}";

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
            (Screen.Content as BoardView)?.ReaderReturned();
            RefreshAttention();
        }
    }

    private void OnClosed(object sender, WindowEventArgs args)
    {
        flushTimer.Stop();
        Detach();
        Toasts.Badge(0);
        Toasts.Unregister();
        _ = services.DisposeAsync().AsTask();
    }
}
