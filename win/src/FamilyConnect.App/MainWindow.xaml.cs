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
    private readonly WindowPlacement placement;

    /// <summary>The notification area icon a closed window leaves behind, and whether a close is the app quitting.</summary>
    private readonly TrayIcon? tray;
    private bool quitting;

    /// <summary>A share waiting for the chats to exist, and whether the reader is choosing where one goes right now.</summary>
    private bool sharePending;
    private bool choosingShare;
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
        // Where it was left, or a first size scaled for this screen — never a pixel count, which at 200% is half a window.
        placement = WindowPlacement.Apply(AppWindow, services.WindowHandle);
        // Closing the window hides it while the app goes on listening — as a Mac app stays in the Dock — and the icon in
        // the notification area brings it back or quits. Without the icon a close is a quit, never a window nobody can reach.
        try
        {
            tray = new TrayIcon(
                "Family Connect", services.Say.Get("Open Family Connect"), services.Say.Get("Quit Family Connect"), BringForward, Quit);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"the notification area icon: {e.GetType().Name}");
        }
        AppWindow.Closing += OnClosing;
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
            _ = Logged(UseServerAsync(server), "opening the saved server");
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
        if (tray is not null)
        {
            tray.Tip = title;
        }
        Toasts.Badge(chatting ? attention!.Unread : 0);
        Badge(chatsItem, chatting ? attention!.Unread : 0);
        Badge(boardItem, chatting && connection is { } live ? live.Board.Unread() : 0);
    }

    /// <summary>A clicked notification: to the front, and to the chat it was about.</summary>
    internal void OpenFromToast(IReadOnlyDictionary<string, string> arguments)
    {
        BringForward();
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
        if (member && sharePending)
        {
            // Something was shared in before there were chats to put it in: now there are.
            DispatcherQueue.TryEnqueue(ReceiveShared);
        }
    }

    /// <summary>
    /// Settings, over the chats: they go on listening underneath, and Done puts them back — unless
    /// the gate moved meanwhile (a family left, an account deleted), which has already replaced both.
    /// </summary>
    private void ShowSettings() => OpenOverChats(
        "settings",
        current => new SettingsView(services, current, close: () =>
        {
            if (connection == current)
            {
                BackToChats();
            }
        }));

    /// <summary>
    /// The family, over the chats as Settings is. "Message" on a member puts the chats back and opens
    /// that conversation in them.
    /// </summary>
    private void ShowFamily() => OpenOverChats("the family", current =>
    {
        void Back()
        {
            if (connection == current)
            {
                BackToChats();
            }
        }
        return new FamilyView(services, current, close: Back, openChat: chatId =>
        {
            Back();
            chats?.OpenChat(chatId);
        });
    });

    /// <summary>
    /// The wall, over the chats as Settings is. Drawn, it moves the board's marks, and the badge follows. A name in an
    /// opened note puts the chats back and opens that conversation, as "Message" in the family does.
    /// </summary>
    private void ShowBoard() => OpenOverChats("the board", current =>
    {
        void Back()
        {
            if (connection == current)
            {
                BackToChats();
            }
        }
        return new BoardView(services, current, close: Back, shown: RefreshAttention, openChat: chatId =>
        {
            Back();
            chats?.OpenChat(chatId);
        });
    });

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

    /// <summary>
    /// A rail item's count. <b>THE BADGE IS MADE ONCE AND ONLY ITS NUMBER CHANGES</b>: a new InfoBadge on every refresh — every
    /// message, every resync, every activation — swapped the NavigationView's native parts under it many times a minute, and
    /// a failure in there is a fail-fast no managed handler ever sees.
    /// </summary>
    private static void Badge(NavigationViewItem item, int count)
    {
        if (item.InfoBadge is not { } badge)
        {
            if (count <= 0)
            {
                return;
            }
            badge = new InfoBadge();
            item.InfoBadge = badge;
        }
        var visibility = count > 0 ? Visibility.Visible : Visibility.Collapsed;
        if (badge.Visibility != visibility)
        {
            badge.Visibility = visibility;
        }
        if (count > 0 && badge.Value != count)
        {
            badge.Value = count;
        }
    }

    /// <summary>Back to the chats, which went on listening while another place was in front.</summary>
    private void ShowChats() => BackToChats();

    /// <summary>
    /// The chats, and the rail's selection with them. The way back from every screen that opens
    /// over them, and where a screen that could not open leaves the reader.
    /// </summary>
    private void BackToChats()
    {
        if (connection is not null && chats is { } open && shown is Gate.Member or Gate.Owner)
        {
            Select(chatsItem);
            Screen.Content = open;
        }
    }

    /// <summary>
    /// Open one of the rail's screens over the chats, or leave the reader where they can try again.
    /// </summary>
    /// <remarks>
    /// <b>A CLICK THAT DOES NOTHING MUST NOT BE THE END OF IT.</b> The rail has already moved to
    /// the item by the time this runs, and <c>NavigationView</c> raises nothing for a click on the
    /// item that is already selected — so a screen that refuses to open, or whose constructor
    /// throws (caught by the App's own handler, which marks it handled), leaves the old content on
    /// screen and every later click on that item doing NOTHING AT ALL. That reads as a dead menu
    /// and cannot be retried. So: the reason is written down, and the selection goes back to the
    /// chats, where the same click can be made again.
    /// </remarks>
    private void OpenOverChats(string what, Func<Connection, UIElement> build)
    {
        if (connection is not { } current || chats is null)
        {
            Diagnostics.Write($"opening {what}: no connection or no chats to open it over");
            BackToChats();
            return;
        }
        try
        {
            Screen.Content = build(current);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"opening {what}: {e.GetType().FullName} 0x{e.HResult:X8} {e.StackTrace}");
            BackToChats();
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

    /// <summary>Whether there is an icon in the notification area to reach this window from.</summary>
    internal bool HasNotificationAreaIcon => tray is { Shown: true };

    /// <summary>To the front — out of the notification area first, when that is where the window went.</summary>
    internal void BringForward()
    {
        if (!AppWindow.IsVisible)
        {
            AppWindow.Show();
        }
        Activate();
    }

    /// <summary>
    /// Files shared into the app (<see cref="ShareInbox"/>): to the front, asked which chat, and staged there. Before there are
    /// chats to choose from — signed out, or still asking the server who is signed in — it waits for them.
    /// </summary>
    internal void ReceiveShared()
    {
        if (ShareInbox.Peek() is null)
        {
            return;
        }
        if (connection is null || chats is null || shown is not (Gate.Member or Gate.Owner))
        {
            sharePending = true;
            return;
        }
        sharePending = false;
        _ = Logged(ReceiveSharedAsync(), "receiving a share");
    }

    /// <summary>
    /// "Send to": every chat a file may land in, in the list's own order — the family first, and never the assistant's chat,
    /// which takes no attachments (ios ShareTargetPicker). Choosing one opens it with the files staged; nothing is sent.
    /// </summary>
    private async Task ReceiveSharedAsync()
    {
        if (choosingShare || connection is not { } current || chats is not { } view || ShareInbox.Peek() is not { } share)
        {
            return;
        }
        choosingShare = true;
        try
        {
            BringForward();
            ShowChats();
            var say = services.Say;
            var rows = new ChatListModel(current.Chats, () => current.Session.State.Me?.Id ?? 0, say)
                .Rows(DateTimeOffset.Now)
                .Where(row => !row.IsAssistant)
                .ToList();
            if (rows.Count == 0)
            {
                ShareInbox.Discard(share.Folder);
                return;
            }
            long? chosen = null;
            ContentDialog? dialog = null;
            var list = new ListView { SelectionMode = ListViewSelectionMode.None, IsItemClickEnabled = true, MaxHeight = 420, MinWidth = 340 };
            foreach (var row in rows)
            {
                var line = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12, Padding = new Thickness(0, 6, 0, 6), Tag = row.Chat.Id };
                line.Children.Add(new PersonPicture { Width = 32, Height = 32, DisplayName = row.Title });
                line.Children.Add(new TextBlock { Text = row.Title, VerticalAlignment = VerticalAlignment.Center, TextTrimming = TextTrimming.CharacterEllipsis, MaxWidth = 260 });
                list.Items.Add(line);
            }
            list.ItemClick += (_, e) =>
            {
                if (e.ClickedItem is FrameworkElement { Tag: long id })
                {
                    chosen = id;
                    dialog?.Hide();
                }
            };
            dialog = Dialogs.Create(Content.XamlRoot, say.Get("Send to"), list);
            dialog.CloseButtonText = say.Get("Cancel");
            dialog.DefaultButton = ContentDialogButton.Close;
            await dialog.ShowAsync();
            if (chosen is not { } chatId || chats != view)
            {
                // Sent nowhere: the copies go, as the Mac's Cancel drops its parked files.
                ShareInbox.Discard(share.Folder);
                return;
            }
            var files = new List<Windows.Storage.StorageFile>();
            foreach (var path in share.Files)
            {
                files.Add(await Windows.Storage.StorageFile.GetFileFromPathAsync(path));
            }
            // Staging reads each file into memory as it goes (ComposerStaging), so the copies are not needed afterwards.
            await view.StageSharedAsync(chatId, files);
            ShareInbox.Discard(share.Folder);
        }
        finally
        {
            choosingShare = false;
        }
        // Another share may have arrived while this one was being placed.
        if (!Directory.Exists(share.Folder) && ShareInbox.Peek() is not null)
        {
            DispatcherQueue.TryEnqueue(ReceiveShared);
        }
    }

    /// <summary>The icon's Quit: the one close that really closes.</summary>
    private void Quit()
    {
        quitting = true;
        Close();
    }

    /// <summary>
    /// The title bar's close, Alt+F4, the taskbar's Close window: hidden, still listening — unless the reader turned that
    /// off, or there is no icon to come back from.
    /// </summary>
    private void OnClosing(Microsoft.UI.Windowing.AppWindow sender, Microsoft.UI.Windowing.AppWindowClosingEventArgs args)
    {
        if (quitting || tray is not { Shown: true } || !KeepRunningSetting.Enabled)
        {
            return;
        }
        args.Cancel = true;
        placement.Save();
        sender.Hide();
    }

    /// <summary>A step nobody awaits, whose failure is written down rather than lost with its task.</summary>
    private static async Task Logged(Task work, string what)
    {
        try
        {
            await work;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"{what}: {e.GetType().Name} 0x{e.HResult:X8}");
        }
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
            (Screen.Content as BoardView)?.ReaderReturned();
            RefreshAttention();
        }
    }

    private void OnClosed(object sender, WindowEventArgs args)
    {
        placement.Save();
        tray?.Dispose();
        flushTimer.Stop();
        Detach();
        Toasts.Badge(0);
        Toasts.Unregister();
        _ = services.DisposeAsync().AsTask();
    }
}
