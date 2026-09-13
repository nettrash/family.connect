using FamilyConnect.App.Logic;
using FamilyConnect.App.Services;
using FamilyConnect.Core.Protocol;
using Microsoft.UI.Input;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;

namespace FamilyConnect.App.Views;

/// <summary>
/// The chat list and one open conversation. Every decision is App.Logic's — the order, the counts,
/// the hidden rows, the read marker, paging — and this class only draws what those models answer.
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
    private readonly Action<MessageDto> onMessage;
    private readonly Action<long> onChat;
    private readonly Action onRoster;
    private readonly Action<long, bool> onBlock;
    private readonly Action<Resync.Report> onResync;
    private readonly Action<Link> onLink;

    private ConversationModel? open;
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

        ChatsHeading.Text = say.Get("Chats");
        EmptyListText.Text = say.Get("No chats yet");
        LogOutButton.Content = say.Get("Log Out");
        SendButton.Content = say.Get("Send");
        ComposerBox.PlaceholderText = say.Get("Message");

        ChatList.SelectionChanged += OnChatPicked;
        SendButton.Click += (_, _) => Send();
        ComposerBox.PreviewKeyDown += OnComposerKey;
        ComposerBox.TextChanged += (_, _) =>
        {
            if (open is { } chat && ComposerBox.Text.Length > 0)
            {
                _ = chat.TypingAsync();
            }
        };
        MessageScroller.ViewChanged += OnScrolled;
        LogOutButton.Click += async (_, _) => await connection.Session.SignOutAsync();

        onMessage = _ => QueueRedraw();
        onChat = _ => QueueRedraw();
        onRoster = QueueRedraw;
        onBlock = (_, _) => QueueRedraw();
        onResync = _ => QueueRedraw();
        onLink = link => DispatcherQueue.TryEnqueue(() => ShowLink(link));
        connection.Router.Arrived += onMessage;
        connection.Router.Edited += onMessage;
        connection.Router.ChatChanged += onChat;
        connection.Router.RosterChanged += onRoster;
        connection.Router.BlockChanged += onBlock;
        connection.Live.Resynced += onResync;
        connection.Live.LinkChanged += onLink;

        ShowLink(connection.Live.Link);
        Redraw();
    }

    /// <summary>The window is going away from this connection: stop listening to it.</summary>
    internal void Detach()
    {
        connection.Router.Arrived -= onMessage;
        connection.Router.Edited -= onMessage;
        connection.Router.ChatChanged -= onChat;
        connection.Router.RosterChanged -= onRoster;
        connection.Router.BlockChanged -= onBlock;
        connection.Live.Resynced -= onResync;
        connection.Live.LinkChanged -= onLink;
    }

    /// <summary>The window came to the front: what is on screen may now count as read.</summary>
    internal void ReaderReturned() => _ = ReportReadAsync();

    private void ShowLink(Link link) =>
        LinkText.Text = link switch
        {
            Link.Up => string.Empty,
            Link.Connecting => services.Say.Get("Connecting…"),
            _ => services.Say.Get("Offline"),
        };

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
            chatId, connection.Chats, connection.Api, connection.Sending, connection.Socket);
        open = chat;
        atNewest = true;
        conversationDrawn = string.Empty;
        ConversationTitle.Text = connection.Chats.Chat(chatId) is { } row ? list.Title(row.Chat) : string.Empty;
        ComposerPanel.Visibility = Visibility.Visible;
        DrawConversation(keepFromBottom: null);
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
        var drawn = string.Join(Row, bubbles.Select(bubble => string.Join(Field,
            bubble.Message.Id, bubble.Message.EditSeq, bubble.Reads)));
        drawn = $"{chat.ChatId}{Row}{drawn}";
        if (drawn == conversationDrawn)
        {
            return;
        }
        conversationDrawn = drawn;
        MessageStack.Children.Clear();
        if (bubbles.Count == 0)
        {
            MessageStack.Children.Add(new TextBlock
            {
                Text = services.Say.Get("No messages yet"),
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

    private FrameworkElement BubbleElement(ConversationModel chat, Bubble bubble, bool showSender)
    {
        var say = services.Say;
        var resources = Application.Current.Resources;
        var stack = new StackPanel { Spacing = 2 };
        if (showSender && !bubble.Mine)
        {
            stack.Children.Add(new TextBlock
            {
                Text = BubbleText.Sender(bubble, connection.Chats, say),
                FontSize = 12,
                FontWeight = FontWeights.SemiBold,
            });
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
            Text = BubbleText.When(bubble.Message, services.Culture, say),
            FontSize = 11,
            Opacity = 0.7,
            HorizontalAlignment = HorizontalAlignment.Right,
        };
        stack.Children.Add(words);
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
            var id = bubble.Message.Id;
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
                DrawConversation(keepFromBottom: MessageScroller.ExtentHeight - MessageScroller.VerticalOffset);
            };
        }
        return border;
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
                var fromBottom = MessageScroller.ExtentHeight - MessageScroller.VerticalOffset;
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

    private void OnComposerKey(object sender, KeyRoutedEventArgs e)
    {
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
        // Written down first; the outbox owns it from here, and a send interrupted by anything at
        // all is a message that can be finished rather than one that never happened.
        chat.Send(ComposerBox.Text.TrimEnd());
        ComposerBox.Text = string.Empty;
        atNewest = true;
        _ = connection.Live.FlushAsync(SendRules.FlushTrigger.Queued);
    }
}
