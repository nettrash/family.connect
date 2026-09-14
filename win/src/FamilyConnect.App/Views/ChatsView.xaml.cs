using FamilyConnect.App.Logic;
using FamilyConnect.App.Services;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Input;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Documents;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Storage;
using Windows.Storage.Pickers;
using Windows.Storage.Streams;
using Windows.ApplicationModel.DataTransfer;

namespace FamilyConnect.App.Views;

/// <summary>
/// The chat list and one open conversation. Every decision is App.Logic's — the order, the counts,
/// the hidden rows, the read marker, paging, what a tap on a reaction means, who is typing — and
/// this class only draws what those models answer.
/// </summary>
/// <remarks>
/// <para>
/// <b>THE CACHE IS THE STATE.</b> A live frame lands in the store on the socket's thread, and all this
/// view does about it is ask for one redraw on its own thread. Redraws are COALESCED: a resync that
/// applies two hundred messages is one redraw, not two hundred.
/// </para>
/// <para>
/// <b>A READ IS REPORTED ONLY WHEN IT WAS READ</b>: the window in front, and the conversation scrolled
/// to its newest message. Scrolled up the history, the reader has not read what is below — the rule
/// every client keeps (docs/protocol.md, "A browser is a client too").
/// </para>
/// </remarks>
public sealed partial class ChatsView : UserControl
{
    // Separators for the "has anything changed" signatures: characters no title or message contains.
    private const char Field = (char)31;
    private const char Row = (char)30;

    private readonly AppServices services;
    private readonly Connection connection;
    private readonly ChatListModel list;
    private readonly TypingRoster typing;
    private readonly DispatcherQueueTimer typingTimer;
    private readonly Action<MessageDto> onArrived;
    private readonly Action<MessageDto> onEdited;
    private readonly Action<long> onChat;
    private readonly Action onRoster;
    private readonly Action<long, bool> onBlock;
    private readonly Action<long, long> onTyping;
    private readonly Action<Resync.Report> onResync;
    private readonly Action<Link> onLink;
    private readonly Action<OutboxRow, ApiError> onRefused;
    private readonly Action openSettings;
    private readonly Action openFamily;
    private readonly Action openBoard;

    private readonly Dictionary<string, BitmapImage> pictures = [];

    /// <summary>What each chat has staged for its next message, kept while the reader looks elsewhere.</summary>
    private readonly Dictionary<long, ComposerStaging> strips = [];
    private bool sendingMedia;

    /// <summary>The names a half-typed @ could mean, and which of them Enter or Tab would take.</summary>
    private IReadOnlyList<MemberDto> offered = [];
    private int activeName;
    private bool gone;
    private ConversationModel? open;
    private MessageDto? replyingTo;
    private MessageDto? editing;
    private int redrawQueued;
    private bool drawingList;
    private bool atNewest = true;
    private bool pagingBack;
    private string listDrawn = string.Empty;
    private string conversationDrawn = string.Empty;

    internal ChatsView(AppServices services, Connection connection, Action openSettings, Action openFamily, Action openBoard)
    {
        this.services = services;
        this.openSettings = openSettings;
        this.openFamily = openFamily;
        this.openBoard = openBoard;
        this.connection = connection;
        InitializeComponent();
        var say = services.Say;
        list = new ChatListModel(connection.Chats, () => connection.Session.State.Me?.Id ?? 0, say);
        typing = new TypingRoster(connection.Chats, words: say);

        ChatsHeading.Text = say.Get("Chats");
        EmptyListText.Text = say.Get("No chats yet");
        BoardText.Text = say.Get("Board");
        FamilyButton.Content = say.Get("Family");
        SettingsButton.Content = say.Get("Settings");
        SendButton.Content = say.Get("Send");
        ComposerBox.PlaceholderText = say.Get("Message");
        ToolTipService.SetToolTip(AttachButton, say.Get("Attach a photo, video or file"));
        AutomationProperties.SetName(AttachButton, say.Get("Attach a photo, video or file"));

        ChatList.SelectionChanged += OnChatPicked;
        SendButton.Click += (_, _) => Send();
        AttachButton.Click += (_, _) => _ = PickAsync();
        ComposerPanel.DragOver += OnDragOver;
        ComposerPanel.Drop += OnDrop;
        BannerCancel.Click += (_, _) => EndComposerMode(clear: editing is not null);
        ComposerBox.PreviewKeyDown += OnComposerKey;
        ComposerBox.TextChanged += (_, _) =>
        {
            if (open is { } chat && editing is null && ComposerBox.Text.Length > 0)
            {
                _ = chat.TypingAsync();
            }
            activeName = 0;
            DrawSuggestions();
        };
        MessageScroller.ViewChanged += OnScrolled;
        // Logging out lives in Settings, where it asks first.
        SettingsButton.Click += (_, _) => this.openSettings();
        FamilyButton.Click += (_, _) => this.openFamily();
        BoardButton.Click += (_, _) => this.openBoard();

        onArrived = message =>
        {
            typing.Spoke(message);
            QueueRedraw();
        };
        onEdited = _ => QueueRedraw();
        onChat = _ => QueueRedraw();
        onRoster = QueueRedraw;
        onBlock = (_, _) => QueueRedraw();
        onTyping = (chatId, userId) =>
        {
            typing.Heard(chatId, userId);
            DispatcherQueue.TryEnqueue(ShowTyping);
        };
        onResync = _ => QueueRedraw();
        onLink = link => DispatcherQueue.TryEnqueue(() => ShowLink(link));
        onRefused = (_, _) => QueueRedraw();
        connection.Router.Arrived += onArrived;
        connection.Router.Edited += onEdited;
        connection.Router.ChatChanged += onChat;
        connection.Router.RosterChanged += onRoster;
        connection.Router.BlockChanged += onBlock;
        connection.Router.Typing += onTyping;
        connection.Live.Resynced += onResync;
        connection.Live.LinkChanged += onLink;
        connection.Sending.Refused += onRefused;
        connection.Media.Refused += onRefused;

        // A typing line expires on its own; something has to look again when it does.
        typingTimer = DispatcherQueue.CreateTimer();
        typingTimer.Interval = TimeSpan.FromSeconds(1);
        typingTimer.Tick += (_, _) => ShowTyping();
        typingTimer.Start();

        ShowLink(connection.Live.Link);
        Redraw();
    }

    /// <summary>The window is going away from this connection: stop listening to it.</summary>
    internal void Detach()
    {
        typingTimer.Stop();
        pictures.Clear();
        connection.Router.Arrived -= onArrived;
        connection.Router.Edited -= onEdited;
        connection.Router.ChatChanged -= onChat;
        connection.Router.RosterChanged -= onRoster;
        connection.Router.BlockChanged -= onBlock;
        connection.Router.Typing -= onTyping;
        connection.Live.Resynced -= onResync;
        connection.Live.LinkChanged -= onLink;
        connection.Sending.Refused -= onRefused;
        connection.Media.Refused -= onRefused;
        gone = true;
    }

    /// <summary>A clicked notification: open that chat, whatever was open before.</summary>
    internal void OpenChat(long chatId)
    {
        if (open?.ChatId == chatId)
        {
            return;
        }
        // The list is drawn again so the row the notification named is the one selected.
        listDrawn = string.Empty;
        _ = OpenAsync(chatId);
    }

    /// <summary>How many notes on the wall have something new to read — nothing drawn at none.</summary>
    internal void ShowBoardBadge(int unread)
    {
        BoardBadge.Value = unread;
        BoardBadge.Visibility = unread > 0 ? Visibility.Visible : Visibility.Collapsed;
    }

    /// <summary>The window came to the front: what is on screen may now count as read.</summary>
    internal void ReaderReturned() => _ = ReportReadAsync();

    private long Reader => connection.Chats.Reader;

    private void ShowLink(Link link) =>
        LinkText.Text = link switch
        {
            Link.Up => string.Empty,
            Link.Connecting => services.Say.Get("Connecting…"),
            _ => services.Say.Get("Offline"),
        };

    private void ShowTyping() =>
        TypingText.Text = open is { } chat ? typing.Line(chat.ChatId) : string.Empty;

    private void QueueRedraw()
    {
        if (Interlocked.Exchange(ref redrawQueued, 1) == 1)
        {
            return;
        }
        DispatcherQueue.TryEnqueue(() =>
        {
            Interlocked.Exchange(ref redrawQueued, 0);
            Redraw();
        });
    }

    private void Redraw()
    {
        DrawList();
        DrawConversation(keepFromBottom: null);
        ShowTyping();
        _ = ReportReadAsync();
    }

    // ---- the list ------------------------------------------------------------------------------

    private void DrawList()
    {
        var rows = list.Rows(DateTimeOffset.Now);
        var times = rows
            .Select(row => RowTimeText.Format(row.When, row.At, services.Culture, services.Say))
            .ToList();
        // Unchanged is not redrawn: rebuilding the rows would drop the keyboard focus and the scroll
        // position of a list nothing happened to.
        var drawn = string.Join(Row, rows.Select((row, at) => string.Join(Field,
            row.Chat.Id, row.Title, row.Preview, row.Unread, row.Hidden, times[at])));
        EmptyListText.Visibility = rows.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        if (drawn == listDrawn)
        {
            return;
        }
        listDrawn = drawn;
        drawingList = true;
        try
        {
            ChatList.Items.Clear();
            for (var at = 0; at < rows.Count; at++)
            {
                var element = RowElement(rows[at], times[at]);
                ChatList.Items.Add(element);
                if (open?.ChatId == rows[at].Chat.Id)
                {
                    ChatList.SelectedItem = element;
                }
            }
        }
        finally
        {
            drawingList = false;
        }
    }

    private static FrameworkElement RowElement(ChatRow row, string time)
    {
        var grid = new Grid { Padding = new Thickness(4, 8, 4, 8), ColumnSpacing = 8, Tag = row.Chat.Id };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var words = new StackPanel { Spacing = 2 };
        words.Children.Add(new TextBlock
        {
            Text = row.Title,
            FontWeight = FontWeights.SemiBold,
            TextTrimming = TextTrimming.CharacterEllipsis,
        });
        words.Children.Add(new TextBlock
        {
            Text = row.Preview,
            Opacity = 0.7,
            MaxLines = 1,
            TextTrimming = TextTrimming.CharacterEllipsis,
            FontStyle = row.Hidden ? Windows.UI.Text.FontStyle.Italic : Windows.UI.Text.FontStyle.Normal,
        });

        var side = new StackPanel { Spacing = 4, HorizontalAlignment = HorizontalAlignment.Right };
        side.Children.Add(new TextBlock
        {
            Text = time,
            FontSize = 12,
            Opacity = 0.7,
            HorizontalAlignment = HorizontalAlignment.Right,
        });
        if (row.Unread > 0)
        {
            side.Children.Add(new InfoBadge { Value = row.Unread, HorizontalAlignment = HorizontalAlignment.Right });
        }
        Grid.SetColumn(side, 1);
        grid.Children.Add(words);
        grid.Children.Add(side);
        return grid;
    }

    private void OnChatPicked(object sender, SelectionChangedEventArgs e)
    {
        if (drawingList
            || ChatList.SelectedItem is not FrameworkElement { Tag: long chatId }
            || open?.ChatId == chatId)
        {
            return;
        }
        _ = OpenAsync(chatId);
    }

    // ---- the conversation ----------------------------------------------------------------------

    private async Task OpenAsync(long chatId)
    {
        var chat = new ConversationModel(
            chatId, connection.Chats, connection.Api, connection.Sending, connection.Socket,
            outbox: connection.Outbox);
        open = chat;
        atNewest = true;
        // What was said here has been seen now: its notifications go.
        Toasts.Clear(NotificationRules.ChatTag(chatId));
        conversationDrawn = string.Empty;
        EndComposerMode(clear: true);
        DrawStaging();
        ComposerError.Visibility = Visibility.Collapsed;
        ConversationTitle.Text = connection.Chats.Chat(chatId) is { } row ? list.Title(row.Chat) : string.Empty;
        ComposerPanel.Visibility = Visibility.Visible;
        DrawList();
        DrawConversation(keepFromBottom: null);
        ShowTyping();
        var error = await chat.OpenAsync();
        if (open != chat)
        {
            return;
        }
        if (error is not null)
        {
            Diagnostics.Write($"opening a chat: {error.Code} {error.Status}");
        }
        DrawConversation(keepFromBottom: null);
        await ReportReadAsync();
    }

    private void DrawConversation(double? keepFromBottom)
    {
        if (open is not { } chat)
        {
            MessageStack.Children.Clear();
            conversationDrawn = string.Empty;
            return;
        }
        var bubbles = chat.Bubbles();
        var pending = chat.Pending();
        var drawn = string.Join(Row, bubbles.Select(bubble => string.Join(Field,
            bubble.Message.Id, bubble.Message.EditSeq, bubble.Message.ReactionSeq, bubble.Reads,
            connection.Chats.IsBlocked(bubble.Message.ReplyTo?.SenderId ?? 0))));
        drawn += Row + string.Join(Row, pending.Select(row => string.Join(Field, row.ClientMsgId, row.Failed)));
        drawn = $"{chat.ChatId}{Row}{drawn}";
        if (drawn == conversationDrawn)
        {
            return;
        }
        conversationDrawn = drawn;
        MessageStack.Children.Clear();
        if (bubbles.Count == 0 && pending.Count == 0)
        {
            MessageStack.Children.Add(new TextBlock
            {
                Text = services.Say.Get("Say something to get started."),
                Opacity = 0.7,
                Margin = new Thickness(0, 12, 0, 0),
            });
        }
        // Names over bubbles only where there is more than one other person to tell apart.
        var family = connection.Chats.Chat(chat.ChatId)?.Chat.Kind == "family";
        foreach (var bubble in bubbles)
        {
            MessageStack.Children.Add(BubbleElement(chat, bubble, showSender: family));
        }
        foreach (var row in pending)
        {
            MessageStack.Children.Add(PendingElement(chat, row));
        }
        MessageScroller.UpdateLayout();
        if (keepFromBottom is { } distance)
        {
            // Paging back must not move what the reader is looking at: the page lands ABOVE it.
            MessageScroller.ChangeView(null, MessageScroller.ExtentHeight - distance, null, disableAnimation: true);
        }
        else if (atNewest)
        {
            MessageScroller.ChangeView(null, MessageScroller.ScrollableHeight, null, disableAnimation: true);
        }
    }

    private double DistanceFromBottom => MessageScroller.ExtentHeight - MessageScroller.VerticalOffset;

    private FrameworkElement BubbleElement(ConversationModel chat, Bubble bubble, bool showSender)
    {
        var say = services.Say;
        var resources = Application.Current.Resources;
        var message = bubble.Message;
        var stack = new StackPanel { Spacing = 4 };
        if (showSender && !bubble.Mine)
        {
            stack.Children.Add(new TextBlock
            {
                Text = BubbleText.Sender(bubble, connection.Chats, say),
                FontSize = 12,
                FontWeight = FontWeights.SemiBold,
            });
        }
        if (bubble.Reads && Quotes.Of(message, connection.Chats, say) is { } quote)
        {
            stack.Children.Add(QuoteElement(quote));
        }
        if (bubble.Reads && message.Media.Count > 0)
        {
            stack.Children.Add(MediaElement(message));
        }
        var words = new TextBlock
        {
            Text = BubbleText.Words(bubble, list, say),
            TextWrapping = TextWrapping.Wrap,
            IsTextSelectionEnabled = bubble.Reads,
            FontStyle = bubble.Reads ? Windows.UI.Text.FontStyle.Normal : Windows.UI.Text.FontStyle.Italic,
        };
        if (bubble.Reads && message.Call is null && message.Body.Length > 0 && message.Mentions is { Length: > 0 } named)
        {
            NamedRuns(words, message.Body, named, bubble.Mine);
        }
        var when = new TextBlock
        {
            Text = BubbleText.When(message, services.Culture, say),
            FontSize = 11,
            Opacity = 0.7,
            HorizontalAlignment = HorizontalAlignment.Right,
        };
        // A caption-less photo is its picture: the words "Photo" under it would only repeat it.
        if (!bubble.Reads || message.Body.Length > 0 || message.Call is not null || message.Media.Count == 0)
        {
            stack.Children.Add(words);
        }
        if (bubble.Reads && message.Reactions is { Length: > 0 } reactions)
        {
            stack.Children.Add(ChipsElement(chat, message, reactions));
        }
        stack.Children.Add(when);

        var border = new Border
        {
            Child = stack,
            Padding = new Thickness(12, 8, 12, 8),
            CornerRadius = new CornerRadius(12),
            MaxWidth = 560,
            HorizontalAlignment = bubble.Mine ? HorizontalAlignment.Right : HorizontalAlignment.Left,
            Margin = new Thickness(bubble.Mine ? 64 : 0, 2, bubble.Mine ? 0 : 64, 2),
            Background = (Brush)resources[bubble.Mine ? "AccentFillColorDefaultBrush" : "CardBackgroundFillColorDefaultBrush"],
        };
        if (bubble.Mine)
        {
            var ink = (Brush)resources["TextOnAccentFillColorPrimaryBrush"];
            words.Foreground = ink;
            when.Foreground = ink;
        }
        if (bubble.Hidden)
        {
            // A hidden bubble CAN be revealed here — unlike the list's — and the reveal belongs to the
            // conversation, so it outlives the next redraw.
            var id = message.Id;
            var revealed = bubble.Revealed;
            border.Tapped += (_, _) =>
            {
                if (revealed)
                {
                    chat.Hide(id);
                }
                else
                {
                    chat.Reveal(id);
                }
                DrawConversation(keepFromBottom: DistanceFromBottom);
            };
        }
        if (bubble.Reads)
        {
            border.ContextFlyout = MenuFor(chat, bubble);
            // The Tapback-heart idiom, the same emoji on every client.
            border.DoubleTapped += (_, _) => _ = ActAsync(() => chat.ReactAsync(message.Id, Reactions.DoubleTap));
        }
        return border;
    }

    private static FrameworkElement QuoteElement(Quote quote)
    {
        var lines = new StackPanel { Spacing = 0 };
        if (!quote.Hidden)
        {
            lines.Children.Add(new TextBlock { Text = quote.Name, FontSize = 12, FontWeight = FontWeights.SemiBold });
        }
        lines.Children.Add(new TextBlock
        {
            Text = quote.Excerpt,
            FontSize = 12,
            MaxLines = 2,
            TextWrapping = TextWrapping.Wrap,
            TextTrimming = TextTrimming.CharacterEllipsis,
            FontStyle = quote.Hidden ? Windows.UI.Text.FontStyle.Italic : Windows.UI.Text.FontStyle.Normal,
        });
        return new Border
        {
            Child = lines,
            BorderThickness = new Thickness(3, 0, 0, 0),
            BorderBrush = (Brush)Application.Current.Resources["AccentFillColorSecondaryBrush"],
            Padding = new Thickness(8, 2, 0, 2),
            Opacity = 0.85,
        };
    }

    private FrameworkElement ChipsElement(ConversationModel chat, MessageDto message, ReactionDto[] reactions)
    {
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
        foreach (var chip in Reactions.Chips(reactions, Reader))
        {
            var button = new Button
            {
                Content = chip.ShowsCount ? $"{chip.Emoji} {chip.Count}" : chip.Emoji,
                Padding = new Thickness(6, 1, 6, 1),
                MinWidth = 0,
                MinHeight = 0,
            };
            if (chip.IncludesMe)
            {
                button.BorderBrush = (Brush)Application.Current.Resources["AccentFillColorDefaultBrush"];
                button.BorderThickness = new Thickness(2);
            }
            var emoji = chip.Emoji;
            // A tap JOINS and never removes: on the reader's own chip it shows who reacted, where their
            // row is the remove control (Reactions.ReactionChip.TapJoins).
            button.Click += (_, _) =>
            {
                if (chip.TapJoins)
                {
                    _ = ActAsync(() => chat.ReactAsync(message.Id, emoji));
                }
                else
                {
                    WhoReacted(chat, message, reactions, emoji, button);
                }
            };
            row.Children.Add(button);
        }
        return row;
    }

    private void WhoReacted(ConversationModel chat, MessageDto message, ReactionDto[] reactions, string emoji, FrameworkElement anchor)
    {
        var say = services.Say;
        var blocked = connection.Chats.Blocked().ToHashSet();
        var panel = new StackPanel { Spacing = 6, MaxWidth = 320 };
        panel.Children.Add(new TextBlock { Text = say.Get("See who reacted"), FontWeight = FontWeights.SemiBold });
        foreach (var detail in Reactions.Details(
                     reactions, id => connection.Chats.Member(id)?.DisplayName, Reader, blocked, say))
        {
            panel.Children.Add(new TextBlock
            {
                Text = $"{detail.Emoji}  {string.Join(", ", detail.Names)}",
                TextWrapping = TextWrapping.Wrap,
            });
        }
        var flyout = new Flyout { Content = panel };
        if (string.Equals(Reactions.Mine(reactions, Reader), emoji, StringComparison.Ordinal))
        {
            var remove = new HyperlinkButton { Content = say.Get("Remove reaction") };
            remove.Click += (_, _) =>
            {
                flyout.Hide();
                _ = ActAsync(() => chat.ReactAsync(message.Id, emoji));
            };
            panel.Children.Add(remove);
        }
        flyout.ShowAt(anchor);
    }

    private MenuFlyout MenuFor(ConversationModel chat, Bubble bubble)
    {
        var say = services.Say;
        var message = bubble.Message;
        var menu = new MenuFlyout();

        var reply = new MenuFlyoutItem { Text = say.Get("Reply") };
        reply.Click += (_, _) => StartReply(message);
        menu.Items.Add(reply);

        var react = new MenuFlyoutSubItem { Text = say.Get("React") };
        var mine = Reactions.Mine(message.Reactions ?? [], Reader);
        foreach (var emoji in Reactions.Capsule(mine))
        {
            var item = new ToggleMenuFlyoutItem
            {
                Text = emoji,
                IsChecked = string.Equals(emoji, mine, StringComparison.Ordinal),
            };
            var choice = emoji;
            item.Click += (_, _) => _ = ActAsync(() => chat.ReactAsync(message.Id, choice));
            react.Items.Add(item);
        }
        menu.Items.Add(react);

        if (message.Body.Length > 0 && message.Call is null)
        {
            var copy = new MenuFlyoutItem { Text = say.Get("Copy") };
            copy.Click += (_, _) =>
            {
                var package = new DataPackage();
                package.SetText(message.Body);
                Clipboard.SetContent(package);
            };
            menu.Items.Add(copy);
        }
        if (ConversationModel.MayEdit(bubble))
        {
            var edit = new MenuFlyoutItem { Text = say.Get("Edit") };
            edit.Click += (_, _) => StartEdit(message);
            menu.Items.Add(edit);
        }
        return menu;
    }

    /// <summary>
    /// One of this reader's messages that has not landed: "Sending…" while it is on its way, and —
    /// once refused — the two things a person can do about it.
    /// </summary>
    private FrameworkElement PendingElement(ConversationModel chat, OutboxRow row)
    {
        var say = services.Say;
        var resources = Application.Current.Resources;
        var stack = new StackPanel { Spacing = 4 };
        if (row.Body.Length > 0)
        {
            stack.Children.Add(new TextBlock
            {
                Text = row.Body,
                TextWrapping = TextWrapping.Wrap,
                Foreground = (Brush)resources["TextOnAccentFillColorPrimaryBrush"],
            });
        }
        // What it carries, counted: the files are this device's own until they land, and a number
        // needs no translating.
        var carried = row.StagedFiles?.Length ?? row.PendingFiles?.Length ?? row.AttachmentIds?.Length ?? 0;
        if (carried > 0)
        {
            stack.Children.Add(new TextBlock
            {
                Text = $"📎 {carried.ToString(services.Culture)}",
                Foreground = (Brush)resources["TextOnAccentFillColorPrimaryBrush"],
            });
        }
        if (row.Failed)
        {
            var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Right };
            var retry = new HyperlinkButton { Content = say.Get("Try Again") };
            retry.Click += (_, _) =>
            {
                chat.Retry(row.ClientMsgId);
                conversationDrawn = string.Empty;
                DrawConversation(keepFromBottom: null);
                _ = connection.Live.FlushAsync(SendRules.FlushTrigger.UserRetried);
            };
            var discard = new HyperlinkButton { Content = say.Get("Delete") };
            discard.Click += (_, _) =>
            {
                chat.Discard(row.ClientMsgId);
                conversationDrawn = string.Empty;
                DrawConversation(keepFromBottom: null);
            };
            actions.Children.Add(retry);
            actions.Children.Add(discard);
            stack.Children.Add(actions);
        }
        else
        {
            stack.Children.Add(new TextBlock
            {
                Text = say.Get("Sending…"),
                FontSize = 11,
                Opacity = 0.7,
                HorizontalAlignment = HorizontalAlignment.Right,
                Foreground = (Brush)resources["TextOnAccentFillColorPrimaryBrush"],
            });
        }
        return new Border
        {
            Child = stack,
            Padding = new Thickness(12, 8, 12, 8),
            CornerRadius = new CornerRadius(12),
            MaxWidth = 560,
            HorizontalAlignment = HorizontalAlignment.Right,
            Margin = new Thickness(64, 2, 0, 2),
            Background = (Brush)resources["AccentFillColorDefaultBrush"],
            Opacity = row.Failed ? 0.9 : 0.6,
        };
    }

    // ---- attachments ---------------------------------------------------------------------------

    /// <summary>
    /// What a message carries: the pictures first — one at its own shape, several as a grid of four with
    /// the rest counted — and then the rows that are read rather than looked at.
    /// </summary>
    private FrameworkElement MediaElement(MessageDto message)
    {
        var panel = new StackPanel { Spacing = 4 };
        var looked = message.Media.Where(attachment => MediaText.IsMedia(attachment.Kind)).ToList();
        if (looked.Count == 1)
        {
            var (width, height) = MediaText.TileSize(looked[0].Width, looked[0].Height);
            panel.Children.Add(TileElement(looked, 0, width, height));
        }
        else if (looked.Count > 1)
        {
            var grid = new Grid { ColumnSpacing = 4, RowSpacing = 4 };
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            var shown = Math.Min(looked.Count, 4);
            for (var at = 0; at < shown; at++)
            {
                if (at % 2 == 0)
                {
                    grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
                }
                var cell = TileElement(looked, at, 156, 156, more: at == shown - 1 ? looked.Count - shown : 0);
                Grid.SetRow(cell, at / 2);
                Grid.SetColumn(cell, at % 2);
                grid.Children.Add(cell);
            }
            panel.Children.Add(grid);
        }
        foreach (var attachment in message.Media.Where(attachment => !MediaText.IsMedia(attachment.Kind)))
        {
            panel.Children.Add(attachment.Kind == "location" ? LocationElement(attachment) : FileElement(attachment));
        }
        return panel;
    }

    private FrameworkElement TileElement(IReadOnlyList<AttachmentDto> album, int index, double width, double height, int more = 0)
    {
        var attachment = album[index];
        var say = services.Say;
        var video = attachment.Kind == "video";
        var frame = new Grid
        {
            Width = width,
            Height = height,
            CornerRadius = new CornerRadius(8),
            Background = (Brush)Application.Current.Resources["ControlFillColorSecondaryBrush"],
        };
        frame.Children.Add(new TextBlock
        {
            Text = video ? say.Get("Video") : say.Get("Photo"),
            Opacity = 0.6,
            HorizontalAlignment = HorizontalAlignment.Center,
            VerticalAlignment = VerticalAlignment.Center,
        });
        var image = new Image { Stretch = Stretch.UniformToFill };
        frame.Children.Add(image);
        if (video)
        {
            // A glyph, not a sentence.
            frame.Children.Add(new TextBlock
            {
                Text = "▶",
                FontSize = 28,
                Foreground = new SolidColorBrush(Microsoft.UI.Colors.White),
                HorizontalAlignment = HorizontalAlignment.Center,
                VerticalAlignment = VerticalAlignment.Center,
            });
        }
        if (more > 0)
        {
            frame.Children.Add(new Border
            {
                Background = new SolidColorBrush(Windows.UI.Color.FromArgb(128, 0, 0, 0)),
                Child = new TextBlock
                {
                    Text = "+" + more.ToString(services.Culture),
                    FontSize = 24,
                    Foreground = new SolidColorBrush(Microsoft.UI.Colors.White),
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                },
            });
        }
        var source = AttachmentFiles.SourceFor(attachment);
        if (source != AttachmentFiles.TileSource.None)
        {
            _ = ShowPictureAsync(image, attachment, preview: source == AttachmentFiles.TileSource.Preview);
        }
        frame.Tapped += (_, _) => _ = OpenMediaAsync(attachment);
        return frame;
    }

    private async Task ShowPictureAsync(Image image, AttachmentDto attachment, bool preview)
    {
        var key = AttachmentCache.KeyFor(attachment.Id, preview && attachment.HasPreview);
        if (!pictures.TryGetValue(key, out var picture))
        {
            var (bytes, error) = await connection.Attachments.BytesAsync(attachment, preview);
            if (bytes is null)
            {
                if (error is not null)
                {
                    Diagnostics.Write($"a picture: {error.Code} {error.Status}");
                }
                return;
            }
            if (await DecodeAsync(bytes) is not { } decoded)
            {
                return;
            }
            picture = pictures[key] = decoded;
        }
        image.Source = picture;
    }

    private static async Task<BitmapImage?> DecodeAsync(byte[] bytes)
    {
        try
        {
            using var stream = new InMemoryRandomAccessStream();
            using (var writer = new DataWriter(stream))
            {
                writer.WriteBytes(bytes);
                await writer.StoreAsync();
                await writer.FlushAsync();
                writer.DetachStream();
            }
            stream.Seek(0);
            var picture = new BitmapImage();
            await picture.SetSourceAsync(stream);
            return picture;
        }
        catch (Exception e)
        {
            // A format this machine cannot decode (a HEIC without its codec, say) stays a labelled tile.
            Diagnostics.Write($"decoding a picture: {e.GetType().Name}");
            return null;
        }
    }

    /// <summary>A photo opens whole, with a way to save it; a video opens in whatever plays videos here.</summary>
    private async Task OpenMediaAsync(AttachmentDto attachment)
    {
        var say = services.Say;
        if (attachment.Kind != "photo")
        {
            await OpenExternallyAsync(attachment);
            return;
        }
        var (bytes, _) = await connection.Attachments.BytesAsync(attachment);
        if (bytes is null || await DecodeAsync(bytes) is not { } picture)
        {
            ShowProblem(say.Get("The file could not be downloaded."));
            return;
        }
        var dialog = new ContentDialog
        {
            XamlRoot = XamlRoot,
            Content = new Image { Source = picture, Stretch = Stretch.Uniform, MaxWidth = 900, MaxHeight = 640 },
            PrimaryButtonText = say.Get("Save…"),
            CloseButtonText = say.Get("Close"),
            DefaultButton = ContentDialogButton.Close,
        };
        if (await dialog.ShowAsync() == ContentDialogResult.Primary)
        {
            await SaveBytesAsync(attachment, bytes);
        }
    }

    /// <summary>A document or a recording: its name and its size, and a click that saves or plays it.</summary>
    private FrameworkElement FileElement(AttachmentDto attachment)
    {
        var say = services.Say;
        var lines = new StackPanel();
        lines.Children.Add(new TextBlock
        {
            Text = AttachmentFiles.MiddleTruncate(AttachmentText.DisplayName(attachment.Kind, attachment.Name, say), 40),
            FontWeight = FontWeights.SemiBold,
        });
        if (attachment.Size is { } size)
        {
            lines.Children.Add(new TextBlock
            {
                Text = MediaText.DisplaySize(size, say, services.Culture),
                FontSize = 12,
                Opacity = 0.7,
            });
        }
        var row = new Grid { ColumnSpacing = 8 };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.Children.Add(new TextBlock { Text = attachment.Kind == "audio" ? "🎵" : "📄", FontSize = 20, VerticalAlignment = VerticalAlignment.Center });
        Grid.SetColumn(lines, 1);
        row.Children.Add(lines);
        var button = new Button
        {
            Content = row,
            HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Left,
            Padding = new Thickness(8),
        };
        button.Click += (_, _) => _ = attachment.Kind == "audio" ? OpenExternallyAsync(attachment) : SaveAttachmentAsync(attachment);
        return button;
    }

    /// <summary>A place: its label and its coordinates, and a click that opens it in Maps.</summary>
    private FrameworkElement LocationElement(AttachmentDto attachment)
    {
        var say = services.Say;
        var lines = new StackPanel();
        lines.Children.Add(new TextBlock
        {
            Text = AttachmentText.DisplayName("location", attachment.Name, say),
            FontWeight = FontWeights.SemiBold,
        });
        var place = attachment.Latitude is { } latitude && attachment.Longitude is { } longitude
            ? (Latitude: latitude, Longitude: longitude)
            : ((double Latitude, double Longitude)?)null;
        if (place is { } known)
        {
            lines.Children.Add(new TextBlock
            {
                Text = MediaText.LocationLine(known.Latitude, known.Longitude, attachment.AccuracyM),
                FontSize = 12,
                Opacity = 0.7,
            });
        }
        var row = new Grid { ColumnSpacing = 8 };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.Children.Add(new TextBlock { Text = "📍", FontSize = 20, VerticalAlignment = VerticalAlignment.Center });
        Grid.SetColumn(lines, 1);
        row.Children.Add(lines);
        var button = new Button
        {
            Content = row,
            HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Left,
            Padding = new Thickness(8),
            IsEnabled = place is not null,
        };
        button.Click += (_, _) =>
        {
            if (place is { } known)
            {
                _ = Windows.System.Launcher.LaunchUriAsync(
                    new Uri(MediaText.MapsUrl(known.Latitude, known.Longitude, attachment.Name, say)));
            }
        };
        return button;
    }

    private async Task SaveAttachmentAsync(AttachmentDto attachment)
    {
        var (bytes, _) = await connection.Attachments.BytesAsync(attachment);
        if (bytes is null)
        {
            ShowProblem(services.Say.Get("The file could not be downloaded."));
            return;
        }
        await SaveBytesAsync(attachment, bytes);
    }

    private async Task SaveBytesAsync(AttachmentDto attachment, byte[] bytes)
    {
        try
        {
            await AttachmentSaving.SaveAsync(services.WindowHandle, AttachmentFiles.FileName(attachment), bytes);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"saving an attachment: {e.GetType().Name}");
            ShowProblem(services.Say.Get("Something went wrong. Try again."));
        }
    }

    private async Task OpenExternallyAsync(AttachmentDto attachment)
    {
        var (bytes, _) = await connection.Attachments.BytesAsync(attachment);
        if (bytes is null)
        {
            ShowProblem(services.Say.Get("The file could not be downloaded."));
            return;
        }
        try
        {
            await AttachmentSaving.OpenAsync(AttachmentFiles.FileName(attachment), bytes);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"opening an attachment: {e.GetType().Name}");
            ShowProblem(services.Say.Get("Something went wrong. Try again."));
        }
    }

    private void ShowProblem(string sentence)
    {
        ComposerError.Text = sentence;
        ComposerError.Visibility = Visibility.Visible;
    }

    /// <summary>A reaction or an edit: done, then drawn again — or said to have failed.</summary>
    private async Task ActAsync(Func<Task<ApiError?>> act)
    {
        ComposerError.Visibility = Visibility.Collapsed;
        ApiError? error;
        try
        {
            error = await act();
        }
        catch (Exception exception)
        {
            Diagnostics.Write($"chat action: {exception.GetType().Name}");
            error = ApiError.Transport(exception.GetType().Name);
        }
        if (error is not null)
        {
            ComposerError.Text = services.Say.Get("Something went wrong. Try again.");
            ComposerError.Visibility = Visibility.Visible;
        }
        conversationDrawn = string.Empty;
        DrawConversation(keepFromBottom: atNewest ? null : DistanceFromBottom);
    }

    private async void OnScrolled(object? sender, ScrollViewerViewChangedEventArgs e)
    {
        if (e.IsIntermediate)
        {
            return;
        }
        atNewest = MessageScroller.VerticalOffset >= MessageScroller.ScrollableHeight - 24;
        try
        {
            if (atNewest)
            {
                await ReportReadAsync();
            }
            if (MessageScroller.VerticalOffset < 48 && open is { MayHaveOlder: true } chat && !pagingBack)
            {
                pagingBack = true;
                var fromBottom = DistanceFromBottom;
                await chat.OlderAsync();
                if (open == chat)
                {
                    DrawConversation(keepFromBottom: fromBottom);
                }
            }
        }
        catch (Exception exception)
        {
            Diagnostics.Write($"scrolling a chat: {exception.GetType().Name}");
        }
        finally
        {
            pagingBack = false;
        }
    }

    private async Task ReportReadAsync()
    {
        if (open is { } chat && atNewest && services.Foreground)
        {
            await chat.ReadAsync();
        }
    }

    // ---- the composer --------------------------------------------------------------------------

    private void StartReply(MessageDto message)
    {
        editing = null;
        replyingTo = message;
        BannerText.Text = Quotes.Banner(message, connection.Chats, services.Say);
        BannerCancel.Content = services.Say.Get("Cancel reply");
        BannerPanel.Visibility = Visibility.Visible;
        SendButton.Content = services.Say.Get("Send");
        ComposerBox.Focus(FocusState.Programmatic);
    }

    private void StartEdit(MessageDto message)
    {
        replyingTo = null;
        editing = message;
        BannerText.Text = services.Say.Get("Editing message");
        BannerCancel.Content = services.Say.Get("Cancel editing");
        BannerPanel.Visibility = Visibility.Visible;
        SendButton.Content = services.Say.Get("Save");
        ComposerBox.Text = message.Body;
        ComposerBox.SelectionStart = ComposerBox.Text.Length;
        ComposerBox.Focus(FocusState.Programmatic);
    }

    /// <summary>Back to writing a new message. An abandoned edit takes its words with it.</summary>
    private void EndComposerMode(bool clear)
    {
        replyingTo = null;
        editing = null;
        BannerPanel.Visibility = Visibility.Collapsed;
        SendButton.Content = services.Say.Get("Send");
        if (clear)
        {
            ComposerBox.Text = string.Empty;
        }
    }

    // ---- mentioning a member ---------------------------------------------------------------------

    private bool IsFamily(ConversationModel chat) => connection.Chats.Chat(chat.ChatId)?.Chat.Kind == "family";

    /// <summary>
    /// The strip over the composer: the names a half-typed @ could mean, in the family chat alone. The arrows walk
    /// it, Enter or Tab takes the highlighted name, and a click takes that one.
    /// </summary>
    private void DrawSuggestions()
    {
        offered = open is { } chat
            ? ComposerMentions.Offered(ComposerBox.Text, connection.Chats.Members(), Reader, connection.Chats.IsBlocked, IsFamily(chat), editing is not null)
            : [];
        SuggestionStrip.Children.Clear();
        SuggestionScroller.Visibility = offered.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
        AutomationProperties.SetName(SuggestionStrip, services.Say.Get("Members"));
        for (var index = 0; index < offered.Count; index++)
        {
            var member = offered[index];
            var label = new TextBlock();
            label.Inlines.Add(new Run { Text = member.DisplayName, FontWeight = FontWeights.SemiBold });
            if (member.Username.Length > 0)
            {
                label.Inlines.Add(new Run { Text = $"  @{member.Username}", Foreground = (Brush)Application.Current.Resources["TextFillColorSecondaryBrush"] });
            }
            var button = new Button { Content = label, IsTabStop = false };
            if (index == Math.Min(activeName, offered.Count - 1))
            {
                button.Style = (Style)Application.Current.Resources["AccentButtonStyle"];
            }
            button.Click += (_, _) => AcceptName(member.DisplayName);
            SuggestionStrip.Children.Add(button);
        }
    }

    /// <summary>The trailing @prefix replaced by the whole name — which is what makes the resolution at send find somebody.</summary>
    private void AcceptName(string name)
    {
        ComposerBox.Text = Mentions.Accept(ComposerBox.Text, name);
        ComposerBox.SelectionStart = ComposerBox.Text.Length;
        ComposerBox.Focus(FocusState.Programmatic);
    }

    /// <summary>
    /// A body's names drawn BOLD in the bubble's own ink — never a colour of their own, which a tinted ground would
    /// swallow — and, for somebody this reader can message, a door onto that chat.
    /// </summary>
    private void NamedRuns(TextBlock words, string body, MentionDto[] named, bool mine)
    {
        var ink = (Brush)Application.Current.Resources[mine ? "TextOnAccentFillColorPrimaryBrush" : "TextFillColorPrimaryBrush"];
        var members = connection.Chats.Members();
        words.Text = string.Empty;
        words.Inlines.Clear();
        foreach (var (text, userId) in Mentions.Runs(body, [.. named.Select(mention => new Named(mention.UserId, mention.Name))]))
        {
            if (userId is not { } id)
            {
                words.Inlines.Add(new Run { Text = text });
                continue;
            }
            if (!ComposerMentions.OpensChat(id, members, Reader, connection.Chats.IsBlocked))
            {
                words.Inlines.Add(new Run { Text = text, FontWeight = FontWeights.SemiBold });
                continue;
            }
            var door = new Hyperlink { UnderlineStyle = UnderlineStyle.None, Foreground = ink, FontWeight = FontWeights.SemiBold };
            door.Inlines.Add(new Run { Text = text });
            door.Click += (_, _) => _ = OpenDirectAsync(id);
            words.Inlines.Add(door);
        }
    }

    /// <summary>Get-or-create the chat with a member, put it in the list, and open it.</summary>
    private async Task OpenDirectAsync(long userId)
    {
        try
        {
            var answer = await connection.Api.DirectChat(userId);
            if (answer is not { Ok: true, Value: { } opened })
            {
                ShowProblem(FamilyText.GenericFailure(answer.Error ?? ApiError.Transport("no answer"), services.Say));
                return;
            }
            var chats = await connection.Api.Chats();
            if (chats is { Ok: true, Value.Chats: { } rows })
            {
                connection.Chats.Replace(rows);
            }
            OpenChat(opened.Chat.Id);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"opening a mentioned member's chat: {e.GetType().Name}");
            ShowProblem(services.Say.Get("Something went wrong. Try again."));
        }
    }

    private void OnComposerKey(object sender, KeyRoutedEventArgs e)
    {
        if (offered.Count > 0)
        {
            var shifted = InputKeyboardSource.GetKeyStateForCurrentThread(Windows.System.VirtualKey.Shift)
                .HasFlag(Windows.UI.Core.CoreVirtualKeyStates.Down);
            switch (e.Key)
            {
                case Windows.System.VirtualKey.Down:
                    e.Handled = true;
                    activeName = (activeName + 1) % offered.Count;
                    DrawSuggestions();
                    return;
                case Windows.System.VirtualKey.Up:
                    e.Handled = true;
                    activeName = (activeName + offered.Count - 1) % offered.Count;
                    DrawSuggestions();
                    return;
                case Windows.System.VirtualKey.Enter or Windows.System.VirtualKey.Tab when !shifted:
                    e.Handled = true;
                    AcceptName(offered[Math.Min(activeName, offered.Count - 1)].DisplayName);
                    return;
            }
        }
        if (e.Key == Windows.System.VirtualKey.Escape && BannerPanel.Visibility == Visibility.Visible)
        {
            e.Handled = true;
            EndComposerMode(clear: editing is not null);
            return;
        }
        if (e.Key != Windows.System.VirtualKey.Enter)
        {
            return;
        }
        // Enter sends and Shift+Enter is a new line — the convention of every desktop chat.
        var shift = InputKeyboardSource.GetKeyStateForCurrentThread(Windows.System.VirtualKey.Shift)
            .HasFlag(Windows.UI.Core.CoreVirtualKeyStates.Down);
        if (!shift)
        {
            e.Handled = true;
            Send();
        }
    }

    private void Send()
    {
        if (open is not { } chat || sendingMedia)
        {
            return;
        }
        var strip = Staging(chat.ChatId);
        var hasWords = !string.IsNullOrWhiteSpace(ComposerBox.Text);
        if (editing is { } target)
        {
            if (!hasWords)
            {
                return;
            }
            var words = ComposerBox.Text;
            EndComposerMode(clear: true);
            _ = ActAsync(() => chat.EditAsync(target.Id, words));
            return;
        }
        if (!hasWords && strip.Items.Count == 0)
        {
            return;
        }
        if (strip.Preparing)
        {
            ShowProblem(services.Say.Get("Wait until the current attachment is done."));
            return;
        }
        var body = ComposerBox.Text.TrimEnd();
        var replyTo = replyingTo?.Id;
        if (strip.Items.Count > 0)
        {
            _ = SendWithMediaAsync(chat, strip, body, replyTo);
            return;
        }
        // Written down first; the outbox owns it from here, and a send interrupted by anything at
        // all is a message that can be finished rather than one that never happened.
        // The names are resolved from the text at send: a name typed by hand mentions too.
        chat.Send(body, replyToMessageId: replyTo, mentions: ComposerMentions.ForSend(body, connection.Chats.Members(), IsFamily(chat)));
        EndComposerMode(clear: true);
        Queued();
    }

    /// <summary>
    /// A send with files: they go to the staging folder FIRST — off the window's thread, since ten
    /// files of up to 100 MB each are a write and not a click — and the row that names them is
    /// queued after. Until it is, the handles are pinned, so a flush sweeping on another thread
    /// cannot take them in between.
    /// </summary>
    private async Task SendWithMediaAsync(ConversationModel chat, ComposerStaging strip, string body, long? replyTo)
    {
        var items = strip.TakeAll();
        var words = ComposerBox.Text;
        var store = connection.Staging;
        var handles = new List<string>();
        // One at a time: a second send while these are written would be queued in front of them.
        sendingMedia = true;
        SendButton.IsEnabled = false;
        EndComposerMode(clear: true);
        DrawStaging();
        try
        {
            await Task.Run(() =>
            {
                foreach (var item in items)
                {
                    handles.Add(store.Stage(item));
                }
            });
            chat.Send(
                body, replyToMessageId: replyTo, pendingFiles: handles,
                mentions: ComposerMentions.ForSend(body, connection.Chats.Members(), IsFamily(chat)));
            Queued();
        }
        catch (Exception e)
        {
            Diagnostics.Write($"staging a send: {e.GetType().Name}");
            strip.Restore(items);
            if (open == chat)
            {
                if (ComposerBox.Text.Length == 0)
                {
                    ComposerBox.Text = words;
                }
                DrawStaging();
                ShowProblem(services.Say.Get("Something went wrong. Try again."));
            }
        }
        finally
        {
            store.Release(handles);
            sendingMedia = false;
            SendButton.IsEnabled = true;
        }
    }

    private void Queued()
    {
        atNewest = true;
        conversationDrawn = string.Empty;
        DrawConversation(keepFromBottom: null);
        _ = connection.Live.FlushAsync(SendRules.FlushTrigger.Queued);
    }

    // ---- attaching -----------------------------------------------------------------------------

    private ComposerStaging Staging(long chatId)
    {
        if (!strips.TryGetValue(chatId, out var strip))
        {
            strip = new ComposerStaging();
            strips[chatId] = strip;
        }
        return strip;
    }

    private async Task PickAsync()
    {
        if (open is not { } chat)
        {
            return;
        }
        var strip = Staging(chat.ChatId);
        if (strip.BusyReason(editing is not null, services.Say) is { } busy)
        {
            ShowProblem(busy);
            return;
        }
        if (!strip.CanStage)
        {
            ShowProblem(ComposerStaging.CapSentence(services.Say));
            return;
        }
        IReadOnlyList<StorageFile> files;
        try
        {
            var picker = new FileOpenPicker
            {
                SuggestedStartLocation = PickerLocationId.PicturesLibrary,
                ViewMode = PickerViewMode.Thumbnail,
            };
            picker.FileTypeFilter.Add("*");
            // A desktop app must name the window that owns the picker, or it throws.
            WinRT.Interop.InitializeWithWindow.Initialize(picker, services.WindowHandle);
            files = await picker.PickMultipleFilesAsync();
        }
        catch (Exception e)
        {
            Diagnostics.Write($"picking files: {e.GetType().Name}");
            ShowProblem(services.Say.Get("Something went wrong. Try again."));
            return;
        }
        await IngestAsync(chat, strip, files);
    }

    private void OnDragOver(object sender, DragEventArgs e)
    {
        if (open is not null && e.DataView.Contains(StandardDataFormats.StorageItems))
        {
            e.AcceptedOperation = DataPackageOperation.Copy;
        }
    }

    private async void OnDrop(object sender, DragEventArgs e)
    {
        if (open is not { } chat || !e.DataView.Contains(StandardDataFormats.StorageItems))
        {
            return;
        }
        List<StorageFile> files;
        var deferral = e.GetDeferral();
        try
        {
            // A dropped FOLDER is not a file, and is left where it is.
            files = (await e.DataView.GetStorageItemsAsync()).OfType<StorageFile>().ToList();
        }
        catch (Exception exception)
        {
            Diagnostics.Write($"reading a drop: {exception.GetType().Name}");
            return;
        }
        finally
        {
            deferral.Complete();
        }
        await IngestAsync(chat, Staging(chat.ChatId), files);
    }

    /// <summary>THE way files come in, whichever door they used: prepared one at a time, in order.</summary>
    private async Task IngestAsync(ConversationModel chat, ComposerStaging strip, IReadOnlyList<StorageFile> files)
    {
        if (files.Count == 0)
        {
            return;
        }
        if (strip.BusyReason(editing is not null, services.Say) is { } busy)
        {
            ShowProblem(busy);
            return;
        }
        ShowProblem(services.Say.Get("Preparing…"));
        string? said;
        try
        {
            said = await strip.IngestAsync(files, MediaPreparing.PrepareAsync, () => !gone, services.Say);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"preparing files: {e.GetType().Name}");
            said = services.Say.Get("Something went wrong. Try again.");
        }
        if (gone)
        {
            return;
        }
        ComposerError.Visibility = Visibility.Collapsed;
        if (open == chat)
        {
            if (said is not null)
            {
                ShowProblem(said);
            }
            DrawStaging();
        }
    }

    /// <summary>What the open chat has staged, each with its own ✕.</summary>
    private void DrawStaging()
    {
        StagingStrip.Children.Clear();
        if (open is not { } chat || Staging(chat.ChatId) is not { Items.Count: > 0 } strip)
        {
            StagingScroller.Visibility = Visibility.Collapsed;
            return;
        }
        var say = services.Say;
        for (var index = 0; index < strip.Items.Count; index++)
        {
            var item = strip.Items[index];
            var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
            row.Children.Add(Thumb(item));
            row.Children.Add(new TextBlock
            {
                Text = ComposerStaging.Label(item, say, services.Culture),
                VerticalAlignment = VerticalAlignment.Center,
                MaxWidth = 220,
                TextTrimming = TextTrimming.CharacterEllipsis,
            });
            var remove = new Button { Content = "✕", Padding = new Thickness(6, 2, 6, 2), VerticalAlignment = VerticalAlignment.Center };
            ToolTipService.SetToolTip(remove, say.Get("Remove attachment"));
            AutomationProperties.SetName(remove, say.Get("Remove attachment"));
            var at = index;
            remove.Click += (_, _) =>
            {
                strip.Remove(at);
                DrawStaging();
            };
            row.Children.Add(remove);
            StagingStrip.Children.Add(new Border
            {
                Child = row,
                Padding = new Thickness(6, 4, 4, 4),
                CornerRadius = new CornerRadius(8),
                Background = (Brush)Application.Current.Resources["CardBackgroundFillColorDefaultBrush"],
            });
        }
        StagingScroller.Visibility = Visibility.Visible;
    }

    /// <summary>A staged item's picture — its preview — or its kind's glyph.</summary>
    private static FrameworkElement Thumb(StagedMedia item)
    {
        if (item.Preview is { IsEmpty: false } preview)
        {
            var image = new Image { Width = 36, Height = 36, Stretch = Stretch.UniformToFill };
            _ = ShowThumbAsync(image, preview);
            return image;
        }
        return new TextBlock
        {
            Text = item.Kind switch { "audio" => "🎤", "video" => "🎬", "photo" => "🖼", _ => "📄" },
            FontSize = 20,
            VerticalAlignment = VerticalAlignment.Center,
        };
    }

    private static async Task ShowThumbAsync(Image image, ReadOnlyMemory<byte> jpeg)
    {
        try
        {
            using var stream = new InMemoryRandomAccessStream();
            using (var writer = new DataWriter(stream))
            {
                writer.WriteBytes(jpeg.ToArray());
                await writer.StoreAsync();
                writer.DetachStream();
            }
            stream.Seek(0);
            var bitmap = new BitmapImage { DecodePixelHeight = 72 };
            await bitmap.SetSourceAsync(stream);
            image.Source = bitmap;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"drawing a staged thumbnail: {e.GetType().Name}");
        }
    }
}
