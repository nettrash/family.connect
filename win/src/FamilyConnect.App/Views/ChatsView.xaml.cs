using FamilyConnect.App.Logic;
using FamilyConnect.App.Services;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Input;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
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

    private ConversationModel? open;
    private MessageDto? replyingTo;
    private MessageDto? editing;
    private int redrawQueued;
    private bool drawingList;
    private bool atNewest = true;
    private bool pagingBack;
    private string listDrawn = string.Empty;
    private string conversationDrawn = string.Empty;

    internal ChatsView(AppServices services, Connection connection)
    {
        this.services = services;
        this.connection = connection;
        InitializeComponent();
        var say = services.Say;
        list = new ChatListModel(connection.Chats, () => connection.Session.State.Me?.Id ?? 0, say);
        typing = new TypingRoster(connection.Chats, words: say);

        ChatsHeading.Text = say.Get("Chats");
        EmptyListText.Text = say.Get("No chats yet");
        LogOutButton.Content = say.Get("Log Out");
        SendButton.Content = say.Get("Send");
        ComposerBox.PlaceholderText = say.Get("Message");

        ChatList.SelectionChanged += OnChatPicked;
        SendButton.Click += (_, _) => Send();
        BannerCancel.Click += (_, _) => EndComposerMode(clear: editing is not null);
        ComposerBox.PreviewKeyDown += OnComposerKey;
        ComposerBox.TextChanged += (_, _) =>
        {
            if (open is { } chat && editing is null && ComposerBox.Text.Length > 0)
            {
                _ = chat.TypingAsync();
            }
        };
        MessageScroller.ViewChanged += OnScrolled;
        LogOutButton.Click += async (_, _) => await connection.Session.SignOutAsync();

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
        connection.Router.Arrived -= onArrived;
        connection.Router.Edited -= onEdited;
        connection.Router.ChatChanged -= onChat;
        connection.Router.RosterChanged -= onRoster;
        connection.Router.BlockChanged -= onBlock;
        connection.Router.Typing -= onTyping;
        connection.Live.Resynced -= onResync;
        connection.Live.LinkChanged -= onLink;
        connection.Sending.Refused -= onRefused;
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
        conversationDrawn = string.Empty;
        EndComposerMode(clear: true);
        ComposerError.Visibility = Visibility.Collapsed;
        ConversationTitle.Text = connection.Chats.Chat(chatId) is { } row ? list.Title(row.Chat) : string.Empty;
        ComposerPanel.Visibility = Visibility.Visible;
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
        var words = new TextBlock
        {
            Text = BubbleText.Words(bubble, list, say),
            TextWrapping = TextWrapping.Wrap,
            IsTextSelectionEnabled = bubble.Reads,
            FontStyle = bubble.Reads ? Windows.UI.Text.FontStyle.Normal : Windows.UI.Text.FontStyle.Italic,
        };
        var when = new TextBlock
        {
            Text = BubbleText.When(message, services.Culture, say),
            FontSize = 11,
            Opacity = 0.7,
            HorizontalAlignment = HorizontalAlignment.Right,
        };
        stack.Children.Add(words);
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
        stack.Children.Add(new TextBlock
        {
            Text = row.Body,
            TextWrapping = TextWrapping.Wrap,
            Foreground = (Brush)resources["TextOnAccentFillColorPrimaryBrush"],
        });
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

    private void OnComposerKey(object sender, KeyRoutedEventArgs e)
    {
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
        if (open is not { } chat || string.IsNullOrWhiteSpace(ComposerBox.Text))
        {
            return;
        }
        if (editing is { } target)
        {
            var words = ComposerBox.Text;
            EndComposerMode(clear: true);
            _ = ActAsync(() => chat.EditAsync(target.Id, words));
            return;
        }
        // Written down first; the outbox owns it from here, and a send interrupted by anything at
        // all is a message that can be finished rather than one that never happened.
        chat.Send(ComposerBox.Text.TrimEnd(), replyToMessageId: replyingTo?.Id);
        EndComposerMode(clear: true);
        atNewest = true;
        conversationDrawn = string.Empty;
        DrawConversation(keepFromBottom: null);
        _ = connection.Live.FlushAsync(SendRules.FlushTrigger.Queued);
    }
}
