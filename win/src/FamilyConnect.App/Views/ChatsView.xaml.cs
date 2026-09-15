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
    private readonly Action<long> onMarks;
    /// <summary>A link's card landed, or the reader switched cards on or off.</summary>
    private readonly Action onPreviews;
    /// <summary>Card pictures decoded once, so a redraw reuses them instead of decoding (and flashing) again.</summary>
    private readonly Dictionary<string, BitmapImage?> previewPictures = new(StringComparer.Ordinal);
    private readonly AvatarFaces faces;

    private readonly Dictionary<string, BitmapImage> pictures = [];

    /// <summary>What each chat has staged for its next message, kept while the reader looks elsewhere.</summary>
    private readonly Dictionary<long, ComposerStaging> strips = [];

    /// <summary>And what each chat's composer was saying; <see cref="restoredDraft"/> is a draft just put back, which is not typing.</summary>
    private readonly ComposerDrafts drafts = new();
    private string? restoredDraft;

    /// <summary>A voice note being recorded, and the clock that redraws its bar.</summary>
    private VoiceRecorder? recorder;
    private DispatcherQueueTimer? recordingTimer;
    /// <summary>A link clicked a beat ago and waiting to open: a double click on it is the heart, which cancels it.</summary>
    private DispatcherQueueTimer? pendingLink;
    /// <summary>The album the viewer is showing, and a count that makes a load for an earlier page land nowhere.</summary>
    private MediaAlbum? viewing;
    private int viewerShown;
    private long lastHeart;

    /// <summary>
    /// The one recording playing — one at a time — and the latest row drawn for each, which a redraw replaces; ended, it
    /// sits at its end rather than looking paused at the start.
    /// </summary>
    private readonly Windows.Media.Playback.MediaPlayer audio = new();
    private readonly Dictionary<long, AudioRow> audioRows = new();
    private long? playingAudio;
    private long fetchingAudio;
    private bool audioAtEnd;
    private bool movingTrack;
    private bool scrubbingAudio;

    /// <summary>A place being shared: the whole flow (one at a time), the part spent looking, and what ends that look.</summary>
    private bool locating;
    private bool finding;
    private CancellationTokenSource? locationHunt;

    /// <summary>The chain open beside the conversation, if any, and what its panel last drew.</summary>
    private ThreadModel? thread;
    private string threadDrawn = string.Empty;

    /// <summary>Whether a call is on — the window's to say — and where a call this chat asks for goes.</summary>
    private bool callBusy;

    internal event Action<long, long, bool>? CallRequested;
    private bool sendingMedia;

    /// <summary>The names a half-typed @ could mean, and which of them Enter or Tab would take.</summary>
    private IReadOnlyList<MemberDto> offered = [];
    private int activeName;
    private bool gone;
    private ConversationModel? open;

    /// <summary>The open chat's open polls, when it is the family chat — the only one that holds any.</summary>
    private OpenPollsModel? openPolls;
    private MessageDto? replyingTo;
    private MessageDto? editing;
    private int redrawQueued;
    private bool drawingList;
    private bool atNewest = true;
    private bool pagingBack;
    private string listDrawn = string.Empty;

    /// <summary>What each row of the list was last drawn from, by position — so an unchanged row keeps its element.</summary>
    private readonly List<string> rowsDrawn = [];
    private string conversationDrawn = string.Empty;

    /// <summary>Where the open chat opens — its first unread message — decided once per open, and what the divider counts.</summary>
    private long? anchor;
    private bool anchorDecided;
    private bool scrollToAnchor;
    private int unreadAtOpen;
    private long lastReadAtOpen;

    internal ChatsView(AppServices services, Connection connection)
    {
        this.services = services;
        this.connection = connection;
        InitializeComponent();
        faces = new AvatarFaces(connection);
        var say = services.Say;
        list = new ChatListModel(connection.Chats, () => connection.Session.State.Me?.Id ?? 0, say);
        typing = new TypingRoster(connection.Chats, words: say);

        ChatsHeading.Text = say.Get("Chats");
        EmptyListText.Text = say.Get("No chats yet");
        SendButton.Content = say.Get("Send");
        ComposerBox.PlaceholderText = say.Get("Message");
        ToolTipService.SetToolTip(AttachButton, say.Get("Attach"));
        AutomationProperties.SetName(AttachButton, say.Get("Attach"));
        OpenPollsText.Text = say.Get("Open polls");

        ChatList.SelectionChanged += OnChatPicked;
        SendButton.Click += (_, _) => Send();
        AttachButton.Click += (_, _) => ShowAttachMenu();
        AskAssistantButton.Content = "✨";
        AskPictureButton.Content = "🎨";
        ToolTipService.SetToolTip(AskAssistantButton, say.Get("Ask the assistant"));
        AutomationProperties.SetName(AskAssistantButton, say.Get("Ask the assistant"));
        ToolTipService.SetToolTip(AskPictureButton, say.Get("Ask for a picture"));
        AutomationProperties.SetName(AskPictureButton, say.Get("Ask for a picture"));
        AskAssistantButton.Click += (_, _) => PutInComposer(AssistantText.WithAssistantMention(ComposerBox.Text));
        AskPictureButton.Click += (_, _) => PutInComposer(AssistantText.WithDrawToken(ComposerBox.Text));

        ViewerSave.Content = say.Get("Save…");
        ToolTipService.SetToolTip(ViewerSave, say.Get("Save a copy"));
        ViewerShare.Content = say.Get("Share…");
        ToolTipService.SetToolTip(ViewerShare, say.Get("Share"));
        ViewerShare.Click += (_, _) => _ = ShareViewedAsync();
        foreach (var (button, name) in new (Button, string)[]
        {
            (ViewerClose, say.Get("Close")),
            (ViewerPrevious, say.Get("Previous")),
            (ViewerNext, say.Get("Next")),
            (ViewerZoomIn, say.Get("Zoom in")),
            (ViewerZoomOut, say.Get("Zoom out")),
        })
        {
            ToolTipService.SetToolTip(button, name);
            AutomationProperties.SetName(button, name);
        }
        ViewerClose.Click += (_, _) => CloseViewer();
        ViewerPrevious.Click += (_, _) => StepViewer(-1);
        ViewerNext.Click += (_, _) => StepViewer(1);
        ViewerZoomIn.Click += (_, _) => ZoomViewer(album => album.ZoomIn());
        ViewerZoomOut.Click += (_, _) => ZoomViewer(album => album.ZoomOut());
        ViewerSave.Click += (_, _) => _ = SaveViewedAsync();
        ViewerImage.DoubleTapped += (_, e) =>
        {
            e.Handled = true;
            ZoomViewer(album => album.ToggleZoom());
        };
        ViewerScroller.ViewChanged += (_, e) =>
        {
            if (!e.IsIntermediate && viewing is { IsVideo: false } album)
            {
                album.ZoomedTo(ViewerScroller.ZoomFactor);
                ShowZoom();
            }
        };
        ViewerScroller.SizeChanged += (_, _) => FitViewerImage();
        ViewerOverlay.PreviewKeyDown += OnViewerKey;
        OpenPollsButton.Click += (_, _) => _ = ShowOpenPollsAsync();
        ComposerPanel.DragOver += OnDragOver;
        ComposerPanel.Drop += OnDrop;
        BannerCancel.Click += (_, _) => EndComposerMode(clear: editing is not null);
        ComposerBox.PreviewKeyDown += OnComposerKey;
        ComposerBox.Paste += OnComposerPaste;
        ComposerBox.TextChanged += (_, _) =>
        {
            // A draft put back as the chat opened was typed some other time: it says nobody is typing now.
            var restored = restoredDraft is not null && ComposerBox.Text == restoredDraft;
            restoredDraft = null;
            if (open is { } chat && editing is null && ComposerBox.Text.Length > 0 && !restored)
            {
                _ = chat.TypingAsync();
            }
            activeName = 0;
            DrawSuggestions();
            DrawPictureNotice();
        };
        MessageScroller.ViewChanged += OnScrolled;
        JumpButton.Click += (_, _) => JumpToNewest();
        RecordingStop.Content = say.Get("Stop");
        RecordingCancel.Content = say.Get("Cancel");
        RecordingStop.Click += (_, _) => _ = StopRecordingAsync();
        RecordingCancel.Click += (_, _) => CancelRecording();
        LocationText.Text = say.Get("Finding your location…");
        ThreadTitle.Text = say.Get("Thread");
        ThreadSend.Content = say.Get("Send");
        ThreadComposer.PlaceholderText = say.Get("Reply in thread");
        AutomationProperties.SetName(ThreadClose, say.Get("Close"));
        ToolTipService.SetToolTip(ThreadClose, say.Get("Close"));
        ThreadClose.Click += (_, _) => CloseThread();
        ThreadSend.Click += (_, _) => SendInThread();
        ThreadComposer.PreviewKeyDown += OnThreadComposerKey;
        ToolTipService.SetToolTip(CallButton, say.Get("Call"));
        AutomationProperties.SetName(CallButton, say.Get("Call"));
        ToolTipService.SetToolTip(VideoCallButton, say.Get("Video Call"));
        AutomationProperties.SetName(VideoCallButton, say.Get("Video Call"));
        CallButton.Click += (_, _) => RequestCall(video: false);
        VideoCallButton.Click += (_, _) => RequestCall(video: true);
        ThreadComposer.TextChanged += (_, _) =>
        {
            if (open is { } typed && ThreadComposer.Text.Length > 0)
            {
                _ = typed.TypingAsync();
            }
        };
        // The player's events arrive on its own thread and may still be queued when the view is let go.
        audio.PlaybackSession.PositionChanged += (_, _) => DispatcherQueue.TryEnqueue(() =>
        {
            if (!gone)
            {
                ShowPlayback();
            }
        });
        audio.PlaybackSession.PlaybackStateChanged += (_, _) => DispatcherQueue.TryEnqueue(() =>
        {
            if (!gone)
            {
                ShowPlayback();
            }
        });
        audio.MediaEnded += (_, _) => DispatcherQueue.TryEnqueue(() =>
        {
            if (!gone)
            {
                AudioEnded();
            }
        });
        ToolTipService.SetToolTip(JumpButton, say.Get("Jump to the newest message"));
        AutomationProperties.SetName(JumpButton, say.Get("Jump to the newest message"));

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
        // A peer's read marker and an answer mid-stream live outside the cache; they redraw all the same.
        onMarks = _ => QueueRedraw();
        connection.PeerReads.Changed += onMarks;
        connection.Answers.Changed += onMarks;
        onPreviews = QueueRedraw;
        connection.Previews.Landed += onPreviews;
        LinkPreviewSetting.Changed += onPreviews;
        MapPreviewSetting.Changed += onPreviews;
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
        connection.PeerReads.Changed -= onMarks;
        connection.Answers.Changed -= onMarks;
        connection.Previews.Landed -= onPreviews;
        LinkPreviewSetting.Changed -= onPreviews;
        MapPreviewSetting.Changed -= onPreviews;
        // A microphone nothing can reach is a microphone left on.
        CancelRecording();
        recordingTimer?.Stop();
        pendingLink?.Stop();
        pendingLink = null;
        StopAudio();
        locationHunt?.Cancel();
        CloseViewer();
        thread = null;
        // Gone BEFORE the player goes: a playback event already queued finds a view with nothing left to draw into.
        gone = true;
        audio.Dispose();
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

    /// <summary>
    /// Files shared into the app from elsewhere in Windows: that chat opens, and they land staged in its composer exactly as
    /// picked or dropped files do — the reader still presses Send.
    /// </summary>
    internal async Task StageSharedAsync(long chatId, IReadOnlyList<StorageFile> files)
    {
        if (gone)
        {
            return;
        }
        OpenChat(chatId);
        if (open is { } chat && chat.ChatId == chatId)
        {
            await IngestAsync(chat, Staging(chatId), files);
        }
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
        // A redraw queued just before the window let this view go lands on a connection it no longer holds.
        if (gone)
        {
            return;
        }
        DrawList();
        DrawConversation(keepFromBottom: null);
        DrawThread();
        ShowPollsBadge();
        ShowCallButtons();
        ShowAssistantButtons();
        ShowTyping();
        _ = ReportReadAsync();
    }

    /// <summary>The composer's assistant doors, for this chat, this server's assistant, and whether an edit is open.</summary>
    private void ShowAssistantButtons()
    {
        var kind = open is { } chat ? connection.Chats.Chat(chat.ChatId)?.Chat.Kind : null;
        var (ask, picture) = AssistantButtons.Offered(kind, connection.Session.State.Assistant, editing is not null);
        AskAssistantButton.Visibility = ask ? Visibility.Visible : Visibility.Collapsed;
        AskPictureButton.Visibility = picture ? Visibility.Visible : Visibility.Collapsed;
    }

    /// <summary>
    /// A draft an assistant door rewrote: appended or put first, never inserted at the caret, and the caret put back at the
    /// end with the box focused — so the next thing typed goes where the request needs it.
    /// </summary>
    private void PutInComposer(string draft)
    {
        ComposerBox.Text = draft;
        ComposerBox.SelectionStart = draft.Length;
        ComposerBox.Focus(FocusState.Programmatic);
    }

    // ---- the list ------------------------------------------------------------------------------

    private void DrawList()
    {
        var rows = list.Rows(DateTimeOffset.Now);
        // A chat that is gone takes its draft with it.
        drafts.Retain(rows.Select(row => row.Chat.Id));
        var times = rows
            .Select(row => RowTimeText.Format(row.When, row.At, services.Culture, services.Say))
            .ToList();
        // Read once for the whole list: a direct chat's circle is its peer's picture.
        var versions = connection.Chats.Members().ToDictionary(member => member.Id, member => member.AvatarVersion);
        // Unchanged is not redrawn: rebuilding the rows would drop the keyboard focus and the scroll
        // position of a list nothing happened to.
        var signatures = rows.Select((row, at) => string.Join(Field,
            row.Chat.Id, row.Title, row.Preview, row.Unread, row.Mentioned, row.Hidden, times[at],
            versions.GetValueOrDefault(row.Chat.PeerUserId ?? 0))).ToList();
        var drawn = string.Join(Row, signatures);
        EmptyListText.Visibility = rows.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        if (drawn == listDrawn)
        {
            return;
        }
        if (drawingList)
        {
            // Asked again from inside its own update: once more, afterwards, never nested in the middle of one.
            QueueRedraw();
            return;
        }
        listDrawn = drawn;
        drawingList = true;
        try
        {
            // IN PLACE. A row whose words have not changed keeps its element, and one that has is replaced where it stands.
            // Emptying the list and filling it again moved the keyboard focus and the selection on every change — which the
            // ListView answers by raising SelectionChanged in the middle of the update — and a row is never MOVED, because
            // an element leaving one item container for another is what WinUI refuses natively.
            for (var at = 0; at < rows.Count; at++)
            {
                if (at < ChatList.Items.Count && at < rowsDrawn.Count && rowsDrawn[at] == signatures[at])
                {
                    continue;
                }
                var element = RowElement(rows[at], times[at], versions);
                if (at < ChatList.Items.Count)
                {
                    ChatList.Items[at] = element;
                    rowsDrawn[at] = signatures[at];
                }
                else
                {
                    ChatList.Items.Add(element);
                    rowsDrawn.Add(signatures[at]);
                }
            }
            while (ChatList.Items.Count > rows.Count)
            {
                ChatList.Items.RemoveAt(ChatList.Items.Count - 1);
            }
            if (rowsDrawn.Count > rows.Count)
            {
                rowsDrawn.RemoveRange(rows.Count, rowsDrawn.Count - rows.Count);
            }
            // The selection once, after the items are settled.
            var selected = ChatList.Items.OfType<FrameworkElement>().FirstOrDefault(item => item.Tag is long id && id == open?.ChatId);
            if (!ReferenceEquals(ChatList.SelectedItem, selected))
            {
                ChatList.SelectedItem = selected;
            }
        }
        finally
        {
            drawingList = false;
        }
    }

    /// <summary>
    /// One row, as the Mac's sidebar draws it: the circle, the name — bold while there is something unread, two lines for
    /// the family's own name — the time, the one line under it, and the marks: "@" when an unread message names the
    /// reader, and the count.
    /// </summary>
    private FrameworkElement RowElement(ChatRow row, string time, IReadOnlyDictionary<long, int> versions)
    {
        var resources = Application.Current.Resources;
        var secondary = (Brush)resources["TextFillColorSecondaryBrush"];
        var grid = new Grid { Padding = new Thickness(0, 10, 0, 10), ColumnSpacing = 12, Tag = row.Chat.Id };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.Children.Add(Face(row.Chat, row.Title, 40, versions));

        var lines = new Grid { RowSpacing = 2, ColumnSpacing = 8, VerticalAlignment = VerticalAlignment.Center };
        lines.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        lines.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        lines.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        lines.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        lines.Children.Add(new TextBlock
        {
            Text = row.Title,
            FontWeight = row.Unread > 0 ? FontWeights.SemiBold : FontWeights.Normal,
            TextWrapping = row.IsFamily ? TextWrapping.Wrap : TextWrapping.NoWrap,
            MaxLines = row.IsFamily ? 2 : 1,
            TextTrimming = TextTrimming.CharacterEllipsis,
        });
        var clock = new TextBlock
        {
            Text = time,
            FontSize = 12,
            Foreground = (Brush)resources["TextFillColorTertiaryBrush"],
            VerticalAlignment = VerticalAlignment.Top,
            Margin = new Thickness(0, 2, 0, 0),
        };
        Grid.SetColumn(clock, 1);
        lines.Children.Add(clock);
        var preview = new TextBlock
        {
            Text = row.Preview,
            FontSize = 13,
            Foreground = secondary,
            MaxLines = 1,
            TextTrimming = TextTrimming.CharacterEllipsis,
            FontStyle = row.Hidden ? Windows.UI.Text.FontStyle.Italic : Windows.UI.Text.FontStyle.Normal,
        };
        Grid.SetRow(preview, 1);
        lines.Children.Add(preview);
        var marks = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4, VerticalAlignment = VerticalAlignment.Center };
        if (row.Mentioned)
        {
            var named = new Border
            {
                CornerRadius = new CornerRadius(8),
                Padding = new Thickness(6, 0, 6, 1),
                Background = (Brush)resources["AccentFillColorDefaultBrush"],
                Child = new TextBlock
                {
                    Text = "@",
                    FontSize = 11,
                    FontWeight = FontWeights.SemiBold,
                    Foreground = (Brush)resources["TextOnAccentFillColorPrimaryBrush"],
                },
            };
            AutomationProperties.SetName(named, services.Say.Get("Mentions you"));
            marks.Children.Add(named);
        }
        if (row.Unread > 0)
        {
            marks.Children.Add(new InfoBadge { Value = row.Unread });
        }
        Grid.SetRow(marks, 1);
        Grid.SetColumn(marks, 1);
        lines.Children.Add(marks);
        Grid.SetColumn(lines, 1);
        grid.Children.Add(lines);
        return grid;
    }

    /// <summary>A chat's circle: the family's house, a direct chat's peer, the assistant's initials.</summary>
    private FrameworkElement Face(ChatDto chat, string title, double size, IReadOnlyDictionary<long, int>? versions = null)
    {
        var peer = chat.Kind == "direct" ? chat.PeerUserId : null;
        var version = peer is { } id
            ? versions?.GetValueOrDefault(id) ?? connection.Chats.Member(id)?.AvatarVersion ?? 0
            : 0;
        return faces.Face(title, chat.Kind == "family", peer, version, size);
    }

    private void OnChatPicked(object sender, SelectionChangedEventArgs e)
    {
        if (drawingList
            || ChatList.SelectedItem is not FrameworkElement { Tag: long chatId }
            || open?.ChatId == chatId)
        {
            return;
        }
        // NOT FROM INSIDE THE LIST'S OWN SELECTION CHANGE: opening redraws the list, and changing a ListView's items while it
        // is still raising SelectionChanged is a native failure no handler sees. A beat later it is an ordinary update.
        DispatcherQueue.TryEnqueue(() =>
        {
            if (!gone && open?.ChatId != chatId)
            {
                _ = OpenAsync(chatId);
            }
        });
    }

    // ---- the conversation ----------------------------------------------------------------------

    private async Task OpenAsync(long chatId)
    {
        if (open is { } leaving && editing is null)
        {
            // Half a thought stays with the chat it was written in.
            drafts.Save(leaving.ChatId, ComposerBox.Text);
        }
        // A recording belongs to the chat it was started in, one playing goes quiet with its bubble, and a place being
        // found was asked for there.
        CancelRecording();
        StopAudio();
        locationHunt?.Cancel();
        // What was being looked at belongs to the chat it came from.
        CloseViewer();
        // A chain belongs to the chat it was opened in.
        CloseThread();
        var chat = new ConversationModel(
            chatId, connection.Chats, connection.Api, connection.Sending, connection.Socket,
            outbox: connection.Outbox);
        open = chat;
        atNewest = true;
        // What was said here has been seen now: its notifications go.
        Toasts.Clear(NotificationRules.ChatTag(chatId));
        conversationDrawn = string.Empty;
        EndComposerMode(clear: true);
        var draft = drafts.Of(chatId);
        if (draft.Length > 0)
        {
            restoredDraft = draft;
            ComposerBox.Text = draft;
            ComposerBox.SelectionStart = draft.Length;
        }
        DrawStaging();
        ComposerError.Visibility = Visibility.Collapsed;
        var held = connection.Chats.Chat(chatId);
        ConversationTitle.Text = held is { } row ? list.Title(row.Chat) : string.Empty;
        ConversationFace.Content = held is { } shown ? Face(shown.Chat, ConversationTitle.Text, 32) : null;
        ConversationHeader.Visibility = Visibility.Visible;
        ComposerPanel.Visibility = Visibility.Visible;
        // Where it opens, decided once per open from the counts as they stood before anything here was read.
        unreadAtOpen = held?.UnreadCount ?? 0;
        lastReadAtOpen = held?.LastReadMessageId ?? 0;
        DecideAnchor(chat);
        // Polls are the family chat's alone: anywhere else the server answers invalid_poll.
        var family = connection.Chats.Chat(chatId)?.Chat.Kind == "family";
        openPolls = family ? new OpenPollsModel(chatId, connection.Chats, connection.Api) : null;
        OpenPollsButton.Visibility = family ? Visibility.Visible : Visibility.Collapsed;
        ShowPollsBadge();
        ShowCallButtons();
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
        if (!anchorDecided)
        {
            // Nothing was held when it opened: the first page decides.
            DecideAnchor(chat);
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
            bubble.Message.Id, bubble.Message.EditSeq, bubble.Message.ReactionSeq, bubble.Message.Poll?.PollSeq, bubble.Reads,
            connection.Chats.IsBlocked(bubble.Message.ReplyTo?.SenderId ?? 0))));
        drawn += Row + string.Join(Row, pending.Select(row => string.Join(Field, row.ClientMsgId, row.Failed)));
        // A poll draws voters' names and "N of M voted": a block or a roster change redraws it.
        drawn = $"{chat.ChatId}{Field}{string.Join(',', connection.Chats.Blocked())}{Field}{connection.Chats.Members().Count}" +
            $"{Field}{connection.PeerReads.UpTo(chat.ChatId)}{Field}{connection.Answers.Version}{Field}{anchor}" +
            // A call record's "Call back" follows whether a call is on, and what the server carries.
            $"{Field}{callBusy}{Field}{connection.Session.State.CallsEnabled}{Field}{connection.Session.State.VideoCallsEnabled}" +
            // A card under a link appears once its fetch lands, and not at all while the reader has cards switched off —
            // THIS chat's cards: one landing for a message somewhere else is no reason to rebuild this one.
            $"{Field}{PreviewMarks(bubbles)}{Field}{MapPreviewSetting.Enabled}" +
            $"{Field}{DateOnly.FromDateTime(DateTime.Now)}{Row}{drawn}";
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
        var today = DateOnly.FromDateTime(DateTime.Now);
        FrameworkElement? divider = null;
        foreach (var line in Timeline.Rows(bubbles, family, Reader, anchor, TimeZoneInfo.Local))
        {
            if (line.DayAbove is { } day)
            {
                MessageStack.Children.Add(DayPill(Timeline.DayLabel(day, today, services.Culture, services.Say)));
            }
            if (line.UnreadDividerAbove)
            {
                divider = Divider(Timeline.DividerText(unreadAtOpen, services.Say));
                MessageStack.Children.Add(divider);
            }
            MessageStack.Children.Add(BubbleElement(chat, line));
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
        else if (scrollToAnchor)
        {
            scrollToAnchor = false;
            if (divider is null)
            {
                atNewest = true;
                MessageScroller.ChangeView(null, MessageScroller.ScrollableHeight, null, disableAnimation: true);
            }
            else
            {
                // OPENING AT THE DIVIDER leaves the newest below the fold, and the chat unread until the reader gets
                // there — unless it all fits, which is read where it stands.
                var top = Math.Max(0, divider.TransformToVisual(MessageStack).TransformPoint(new Windows.Foundation.Point(0, 0)).Y - 12);
                MessageScroller.ChangeView(null, top, null, disableAnimation: true);
                atNewest = MessageScroller.ScrollableHeight - top <= 24;
            }
        }
        else if (atNewest)
        {
            MessageScroller.ChangeView(null, MessageScroller.ScrollableHeight, null, disableAnimation: true);
        }
        ShowJump();
    }

    /// <summary>Decide where the open chat opens, from what it holds now.</summary>
    private void DecideAnchor(ConversationModel chat)
    {
        var messages = chat.Bubbles().Select(bubble => bubble.Message).ToList();
        anchorDecided = messages.Count > 0;
        anchor = Timeline.OpenAnchor(messages, unreadAtOpen, lastReadAtOpen, Reader);
        scrollToAnchor = anchor is not null;
        if (anchor is not null)
        {
            atNewest = false;
        }
    }

    private void ShowJump() =>
        JumpButton.Visibility = open is not null && !atNewest ? Visibility.Visible : Visibility.Collapsed;

    private void JumpToNewest()
    {
        atNewest = true;
        MessageScroller.ChangeView(null, MessageScroller.ScrollableHeight, null, disableAnimation: false);
        ShowJump();
        _ = ReportReadAsync();
    }

    /// <summary>The pill between day sections.</summary>
    private static FrameworkElement DayPill(string label)
    {
        var resources = Application.Current.Resources;
        return new Border
        {
            HorizontalAlignment = HorizontalAlignment.Center,
            Margin = new Thickness(0, 14, 0, 4),
            Padding = new Thickness(10, 3, 10, 4),
            CornerRadius = new CornerRadius(10),
            Background = (Brush)resources["SubtleFillColorSecondaryBrush"],
            Child = new TextBlock
            {
                Text = label,
                FontSize = 11,
                FontWeight = FontWeights.SemiBold,
                Foreground = (Brush)resources["TextFillColorSecondaryBrush"],
            },
        };
    }

    /// <summary>"N new messages", across the conversation in the accent, above the first of them.</summary>
    private static FrameworkElement Divider(string words)
    {
        var resources = Application.Current.Resources;
        var accent = (Brush)resources["AccentTextFillColorPrimaryBrush"];
        var grid = new Grid { Margin = new Thickness(0, 12, 0, 6), ColumnSpacing = 10 };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        var left = new Border { Height = 1, Background = accent, Opacity = 0.4, VerticalAlignment = VerticalAlignment.Center };
        var text = new TextBlock { Text = words, FontSize = 12, Foreground = accent };
        var right = new Border { Height = 1, Background = accent, Opacity = 0.4, VerticalAlignment = VerticalAlignment.Center };
        Grid.SetColumn(text, 1);
        Grid.SetColumn(right, 2);
        grid.Children.Add(left);
        grid.Children.Add(text);
        grid.Children.Add(right);
        return grid;
    }

    /// <summary>
    /// A balloon's corners: round, except where it meets its run mates on the sender's side — the shape language the
    /// apps use, which is what makes a burst read as one turn in the conversation rather than four.
    /// </summary>
    private static CornerRadius BalloonCorners(bool mine, bool runStart, bool runEnd)
    {
        const double Round = 14;
        const double Tight = 4;
        // CornerRadius runs top-left, top-right, bottom-right, bottom-left.
        return mine
            ? new CornerRadius(Round, runStart ? Round : Tight, runEnd ? Round : Tight, Round)
            : new CornerRadius(runStart ? Round : Tight, Round, Round, runEnd ? Round : Tight);
    }

    private double DistanceFromBottom => MessageScroller.ExtentHeight - MessageScroller.VerticalOffset;

    /// <summary>
    /// One message, as the apps draw it: the run's head (a face and a tinted name) where the sender changes, the balloon
    /// with its run-shaped corners — none at all round nothing but pictures — and under it the reactions and, at the end
    /// of a run, the time.
    /// </summary>
    private FrameworkElement BubbleElement(ConversationModel chat, TimelineRow row, ThreadModel? inThread = null)
    {
        var bubble = row.Bubble;
        var say = services.Say;
        var resources = Application.Current.Resources;
        var secondary = (Brush)resources["TextFillColorSecondaryBrush"];
        var message = bubble.Message;
        var mine = bubble.Mine;
        var kind = connection.Chats.Chat(chat.ChatId)?.Chat.Kind;
        var assistantChat = kind == "ai";
        var familyChat = kind == "family";
        var assistantId = connection.Session.State.Assistant?.UserId;
        // The body as drawn: an answer still being written shows what has streamed so far.
        var body = connection.Answers.BodyOf(message);
        var awaited = BubbleRules.Awaited(message, body, Reader, assistantChat, assistantId);
        var failed = connection.Answers.Failed(message);
        var shown = bubble with { Message = message with { Body = body } };
        // One to four emoji and nothing else: drawn large and bare — the apps' ladder, scaled to this window's 14-pixel body.
        var emojiSize = !awaited && bubble.Reads && message.Call is null && message.Poll is null && message.Media.Count == 0 && message.ReplyTo is null
            ? Emoji.DisplayFontSizeForBody(body, 14)
            : null;

        var column = new StackPanel
        {
            Spacing = 3,
            MaxWidth = 600,
            HorizontalAlignment = mine ? HorizontalAlignment.Right : HorizontalAlignment.Left,
            // Runs breathe less than turns do.
            Margin = new Thickness(mine ? 72 : 0, row.RunStart ? 8 : 1, mine ? 0 : 72, 0),
        };
        if (row.ShowsSender)
        {
            // The run's head: a face beside a tinted name — the one place a family thread tells its speakers apart at a glance.
            var name = BubbleText.Sender(bubble, connection.Chats, say);
            var head = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6, Margin = new Thickness(4, 0, 0, 0) };
            head.Children.Add(faces.Face(name, false, message.SenderId, connection.Chats.Member(message.SenderId)?.AvatarVersion ?? 0, 18));
            head.Children.Add(new TextBlock
            {
                Text = name,
                FontSize = 12,
                FontWeight = FontWeights.SemiBold,
                Foreground = (Brush)resources["AccentTextFillColorPrimaryBrush"],
                VerticalAlignment = VerticalAlignment.Center,
            });
            column.Children.Add(head);
        }

        var stack = new StackPanel { Spacing = 4 };
        if (bubble.Reads && Quotes.Of(message, connection.Chats, say) is { } quote)
        {
            stack.Children.Add(QuoteElement(quote));
        }
        if (bubble.Reads && message.Media.Count > 0)
        {
            stack.Children.Add(MediaElement(message, mine));
        }
        var words = new TextBlock
        {
            // An answer not written yet is a cursor, not a blank bubble — or says it stopped.
            Text = awaited ? (failed ? say.Get("Couldn't answer that. Ask again.") : "▍") : BubbleText.Words(shown, list, say),
            TextWrapping = TextWrapping.Wrap,
            IsTextSelectionEnabled = bubble.Reads && !awaited,
            FontStyle = bubble.Reads && !(awaited && failed) ? Windows.UI.Text.FontStyle.Normal : Windows.UI.Text.FontStyle.Italic,
        };
        if (awaited && !failed)
        {
            AutomationProperties.SetName(words, say.Get("The assistant is answering"));
        }
        // The body as the apps draw it — markdown, links, the assistant's tokens and names (MessageBody). Only a table makes
        // it more than the one block of words.
        FrameworkElement? laidOut = null;
        if (!awaited && bubble.Reads && message.Call is null && body.Length > 0)
        {
            laidOut = BodyElement(words, body, message.Mentions, mine);
        }
        // A call record draws its own row instead of its placeholder body; a caption-less photo is its picture, and the
        // words "Photo" under it would only repeat it.
        if (bubble.Reads && message.Call is { } call)
        {
            stack.Children.Add(CallRecordElement(chat, call, mine, inThread));
        }
        else if (!bubble.Reads || body.Length > 0 || message.Call is not null || message.Media.Count == 0)
        {
            stack.Children.Add(laidOut ?? words);
        }
        if (bubble.Reads && !awaited && failed)
        {
            // It stopped part-way: what arrived stays, and the row says so.
            stack.Children.Add(new TextBlock
            {
                Text = say.Get("Couldn't answer that. Ask again."),
                FontSize = 12,
                FontStyle = Windows.UI.Text.FontStyle.Italic,
                TextWrapping = TextWrapping.Wrap,
                Opacity = 0.8,
            });
        }
        // The card under the first https link the body draws. ASKED FOR whether or not the row reads — a hidden row fetches
        // exactly what a visible one would and draws none of it (docs/protocol.md, "A hidden row still fetches") — and drawn
        // once its fetch has landed, never as a placeholder that would grow the bubble twice.
        if (!awaited && message.Call is null
            && BubbleBody.PreviewLink(body, emojiOnly: emojiSize is not null) is { } previewLink
            && connection.Previews.State(previewLink) is { Status: PreviewStatus.Loaded, Preview: { } preview }
            && bubble.Reads)
        {
            stack.Children.Add(PreviewCard(preview));
        }
        if (bubble.Reads && message.Poll is not null)
        {
            // The question is the body, drawn above: the options go under it.
            var poll = message.Id;
            stack.Children.Add(PollCard.Build(
                message, mine, PollSeen(),
                option => ActAsync(() => chat.VoteAsync(poll, option)),
                () => ActAsync(() => chat.ClosePollAsync(poll))));
        }

        // Nothing but photos and videos, and nothing above them: the pictures ARE the message, and draw without a balloon — as
        // do a few emoji, which are themselves.
        var bare = emojiSize is not null || (bubble.Reads
            && !awaited
            && body.Length == 0
            && message.ReplyTo is null
            && message.Poll is null
            && message.Call is null
            && message.Media.Count > 0
            && message.Media.All(attachment => MediaText.IsMedia(attachment.Kind)));
        if (emojiSize is { } size)
        {
            words.FontSize = size;
        }
        var balloon = new Border
        {
            Child = stack,
            Padding = bare ? new Thickness(0) : new Thickness(12, 8, 12, 8),
            CornerRadius = BalloonCorners(mine, row.RunStart, row.RunEnd),
            HorizontalAlignment = mine ? HorizontalAlignment.Right : HorizontalAlignment.Left,
        };
        if (!bubble.Reads)
        {
            // The collapsed stand-in: the words on a quiet ground, and a click shows it.
            balloon.Background = (Brush)resources["SubtleFillColorSecondaryBrush"];
            balloon.CornerRadius = new CornerRadius(10);
            words.FontSize = 12;
            words.Foreground = secondary;
            ToolTipService.SetToolTip(balloon, say.Get("Click to show"));
            // A screen reader hears what the stand-in is and what a click does — never the words it hides (ios MacMessageRow).
            AutomationProperties.SetName(balloon, say.Get("Hidden message from a blocked member. Click to show it."));
        }
        else if (!bare)
        {
            balloon.Background = (Brush)resources[mine ? "AccentFillColorDefaultBrush" : "CardBackgroundFillColorDefaultBrush"];
            if (mine)
            {
                words.Foreground = (Brush)resources["TextOnAccentFillColorPrimaryBrush"];
            }
            else
            {
                balloon.BorderBrush = (Brush)resources["CardStrokeColorDefaultBrush"];
                balloon.BorderThickness = new Thickness(1);
            }
        }
        column.Children.Add(balloon);

        if (bubble.Reads && message.Reactions is { Length: > 0 } reactions)
        {
            var chips = ChipsElement(chat, message, reactions, inThread);
            chips.HorizontalAlignment = mine ? HorizontalAlignment.Right : HorizontalAlignment.Left;
            column.Children.Add(chips);
        }
        // Under a root somebody answered: how many, and a click that opens the chain beside the conversation — drawn with
        // the message rather than as a bubble of its own (docs/protocol.md, "What a client shows").
        if (inThread is null && bubble.Reads && message.ReplyCount is { } replyCount)
        {
            var label = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
            // Speech bubbles, in Segoe Fluent Icons.
            label.Children.Add(new FontIcon { Glyph = ((char)0xE8BD).ToString(), FontSize = 12 });
            label.Children.Add(new TextBlock { Text = say.Plural("%lld replies", replyCount, replyCount), FontSize = 12 });
            var threadLink = new HyperlinkButton
            {
                Content = label,
                Padding = new Thickness(6, 2, 6, 2),
                HorizontalAlignment = mine ? HorizontalAlignment.Right : HorizontalAlignment.Left,
            };
            ToolTipService.SetToolTip(threadLink, say.Get("Opens the thread"));
            var rootId = message.Id;
            threadLink.Click += (_, _) => _ = OpenThreadAsync(rootId);
            column.Children.Add(threadLink);
        }
        // The time at the end of a run — and always on an edited row, which says so, and on a hidden one, or three hidden
        // messages in a row would draw two stand-ins that are literally nothing.
        if (row.RunEnd || message.EditedAt is not null || !bubble.Reads)
        {
            var clock = BubbleText.When(message, services.Culture, say);
            var when = new TextBlock
            {
                Text = clock,
                FontSize = 11,
                Foreground = secondary,
                HorizontalAlignment = mine ? HorizontalAlignment.Right : HorizontalAlignment.Left,
                Margin = new Thickness(4, 0, 4, 0),
            };
            if (BubbleRules.ShowsTick(message, Reader, familyChat))
            {
                // One tick sent, two seen — a direct chat's fact, from the other person's live marker.
                var seen = BubbleRules.Seen(message, Reader, familyChat, connection.PeerReads.UpTo(chat.ChatId));
                when.Text = $"{clock} {(seen ? "✓✓" : "✓")}";
                AutomationProperties.SetName(when, $"{clock} {(seen ? say.Get("Read") : say.Get("Sent"))}");
            }
            column.Children.Add(when);
        }

        if (bubble.Hidden)
        {
            // A hidden bubble CAN be revealed here — unlike the list's — and the reveal belongs to the
            // conversation, so it outlives the next redraw.
            var id = message.Id;
            var revealed = bubble.Revealed;
            balloon.Tapped += (_, _) =>
            {
                // On the thread's panel the reveal is the panel's own.
                if (inThread is not null)
                {
                    if (revealed)
                    {
                        inThread.Hide(id);
                    }
                    else
                    {
                        inThread.Reveal(id);
                    }
                    threadDrawn = string.Empty;
                    DrawThread();
                    return;
                }
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
            balloon.ContextFlyout = MenuFor(chat, bubble, assistantChat, assistantId, balloon, inThread);
            // The Tapback-heart idiom, the same emoji on every client — on the balloon, not the row, so a double click beside
            // a message leaves nothing on it.
            balloon.DoubleTapped += (_, _) =>
            {
                CancelLinkForHeart();
                _ = ActAsync(() => React(chat, inThread, message.Id, Reactions.DoubleTap));
            };
        }
        else if (BubbleRules.IsOtherMember(message, Reader, assistantChat, assistantId))
        {
            // The hidden row's menu is Safety and nothing more: Copy would put the hidden words on the clipboard.
            var menu = new MenuFlyout();
            AddSafety(menu, message, assistantChat, assistantId, mayReport: inThread is null);
            balloon.ContextFlyout = menu;
        }
        return column;
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

    private FrameworkElement ChipsElement(ConversationModel chat, MessageDto message, ReactionDto[] reactions, ThreadModel? inThread)
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
                    _ = ActAsync(() => React(chat, inThread, message.Id, emoji));
                }
                else
                {
                    WhoReacted(chat, message, reactions, emoji, button, inThread);
                }
            };
            row.Children.Add(button);
        }
        return row;
    }

    private void WhoReacted(ConversationModel chat, MessageDto message, ReactionDto[] reactions, string emoji, FrameworkElement anchor, ThreadModel? inThread)
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
                _ = ActAsync(() => React(chat, inThread, message.Id, emoji));
            };
            panel.Children.Add(remove);
        }
        flyout.ShowAt(anchor);
    }

    private MenuFlyout MenuFor(ConversationModel chat, Bubble bubble, bool assistantChat, long? assistantId, FrameworkElement anchor, ThreadModel? inThread = null)
    {
        var say = services.Say;
        var message = bubble.Message;
        var menu = new MenuFlyout();

        var reply = new MenuFlyoutItem { Text = say.Get("Reply") };
        // On the thread's panel a reply is its composer: whatever the row, a send from there answers the root.
        reply.Click += (_, _) =>
        {
            if (inThread is not null)
            {
                ThreadComposer.Focus(FocusState.Programmatic);
            }
            else
            {
                StartReply(message);
            }
        };
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
            item.Click += (_, _) => _ = ActAsync(() => React(chat, inThread, message.Id, choice));
            react.Items.Add(item);
        }
        react.Items.Add(new MenuFlyoutSeparator());
        var more = new MenuFlyoutItem { Text = say.Get("More reactions…") };
        // Any emoji of the catalogue — the one already held takes it off, as a tap on it anywhere does.
        more.Click += (_, _) => EmojiPicker.Show(anchor, say, emoji => _ = ActAsync(() => React(chat, inThread, message.Id, emoji)));
        react.Items.Add(more);
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
        // "View thread" on any member of a chain — the root and every reply — opens the same chain.
        if (inThread is null && (message.ThreadRootId is not null || message.ReplyCount is not null))
        {
            var chain = new MenuFlyoutItem { Text = say.Get("View thread") };
            chain.Click += (_, _) => _ = OpenThreadAsync(message.Id);
            menu.Items.Add(chain);
        }
        // Editing is the chat's, and not offered on the panel rather than offered inert.
        if (inThread is null && ConversationModel.MayEdit(bubble))
        {
            var edit = new MenuFlyoutItem { Text = say.Get("Edit") };
            edit.Click += (_, _) => StartEdit(message);
            menu.Items.Add(edit);
        }
        AddSafety(menu, message, assistantChat, assistantId, mayReport: inThread is null);
        return menu;
    }

    /// <summary>
    /// Report… and Block — or Unblock — about another member's message, grouped as the family's own member rows
    /// group them. Never on the reader's own, and never on the assistant's.
    /// </summary>
    private void AddSafety(MenuFlyout menu, MessageDto message, bool assistantChat, long? assistantId, bool mayReport = true)
    {
        if (!BubbleRules.IsOtherMember(message, Reader, assistantChat, assistantId))
        {
            return;
        }
        var say = services.Say;
        var safety = new MenuFlyoutSubItem { Text = say.Get("Safety") };
        if (mayReport && BubbleRules.MayReport(message, Reader, assistantChat, assistantId))
        {
            var report = new MenuFlyoutItem { Text = say.Get("Report…") };
            report.Click += (_, _) => _ = ReportMessageAsync(message);
            safety.Items.Add(report);
        }
        var sender = message.SenderId;
        var blocked = connection.Chats.IsBlocked(sender);
        var block = new MenuFlyoutItem { Text = blocked ? say.Get("Unblock") : say.Get("Block") };
        block.Click += (_, _) => _ = BlockSenderAsync(sender, !blocked);
        safety.Items.Add(block);
        if (menu.Items.Count > 0)
        {
            menu.Items.Add(new MenuFlyoutSeparator());
        }
        menu.Items.Add(safety);
    }

    /// <summary>One message reported: the four reasons, and the disclosure that the owner will read it.</summary>
    private async Task ReportMessageAsync(MessageDto message)
    {
        var say = services.Say;
        var name = connection.Chats.Member(message.SenderId) switch
        {
            { Deleted: true } => say.Get("Deleted account"),
            { DisplayName: { Length: > 0 } display } => display,
            _ => say.Get("Someone"),
        };
        try
        {
            var reason = await Dialogs.ReportAsync(
                XamlRoot, say, name, aboutMessage: true, connection.Session.State.SupportContact);
            if (reason is null)
            {
                return;
            }
            var answer = await connection.Api.Report(message.SenderId, reason, message.Id);
            if (answer.Ok)
            {
                ShowProblem(say.Get("Report sent."));
                return;
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"reporting a message: {e.GetType().Name}");
        }
        ShowProblem(say.Get("Couldn't send the report. Try again."));
    }

    /// <summary>Block or unblock a message's sender — applied here once the server has it, as the family console does.</summary>
    private async Task BlockSenderAsync(long userId, bool blocked)
    {
        ApiError? error;
        try
        {
            error = await new FamilyModel(connection.Api, connection.Chats).BlockAsync(userId, blocked);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"blocking from a message: {e.GetType().Name}");
            error = ApiError.Transport(e.GetType().Name);
        }
        if (error is not null)
        {
            ShowProblem(services.Say.Get("Couldn't change that right now. Try again."));
        }
        conversationDrawn = string.Empty;
        listDrawn = string.Empty;
        DrawList();
        DrawConversation(keepFromBottom: atNewest ? null : DistanceFromBottom);
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
                threadDrawn = string.Empty;
                DrawThread();
                _ = connection.Live.FlushAsync(SendRules.FlushTrigger.UserRetried);
            };
            var discard = new HyperlinkButton { Content = say.Get("Delete") };
            discard.Click += (_, _) =>
            {
                chat.Discard(row.ClientMsgId);
                conversationDrawn = string.Empty;
                DrawConversation(keepFromBottom: null);
                threadDrawn = string.Empty;
                DrawThread();
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
            CornerRadius = new CornerRadius(14),
            MaxWidth = 600,
            HorizontalAlignment = HorizontalAlignment.Right,
            Margin = new Thickness(72, 8, 0, 0),
            Background = (Brush)resources["AccentFillColorDefaultBrush"],
            Opacity = row.Failed ? 0.9 : 0.6,
        };
    }

    // ---- attachments ---------------------------------------------------------------------------

    /// <summary>
    /// What a message carries: the pictures first — one at its own shape, several as a grid of four with
    /// the rest counted — and then the rows that are read rather than looked at.
    /// </summary>
    private FrameworkElement MediaElement(MessageDto message, bool mine)
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
            panel.Children.Add(attachment.Kind switch
            {
                "location" => LocationElement(attachment, mine),
                "audio" => AudioElement(attachment, mine),
                _ => FileElement(attachment),
            });
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
        frame.Tapped += (_, _) => OpenViewer(album, index);
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
    // ---- the viewer ---------------------------------------------------------------------------------

    /// <summary>Open a message's photos and videos at full size, at the one clicked.</summary>
    private void OpenViewer(IReadOnlyList<AttachmentDto> items, int index)
    {
        viewing = new MediaAlbum(items, index);
        ViewerOverlay.Visibility = Visibility.Visible;
        ShowViewerItem();
        ViewerClose.Focus(FocusState.Programmatic);
    }

    /// <summary>Close it — and a video with it, or the stream keeps running behind a closed viewer.</summary>
    private void CloseViewer()
    {
        viewing = null;
        viewerShown++;
        StopViewerVideo();
        ViewerImage.Source = null;
        ViewerOverlay.Visibility = Visibility.Collapsed;
    }

    private void StepViewer(int direction)
    {
        if (viewing is { } album && album.Step(direction))
        {
            ShowViewerItem();
            // An arrow that just reached the end is disabled, and a disabled button hands its focus to whatever comes next —
            // the composer under the viewer. The keys have to stay here.
            if (FocusManager.GetFocusedElement(XamlRoot) is not Control { IsEnabled: true })
            {
                ViewerClose.Focus(FocusState.Programmatic);
            }
        }
    }

    private void OnViewerKey(object sender, KeyRoutedEventArgs e)
    {
        if (viewing is not { } album)
        {
            return;
        }
        switch (album.Key(e.Key.ToString()))
        {
            case ViewerKey.Close:
                e.Handled = true;
                CloseViewer();
                break;
            case ViewerKey.Previous:
                e.Handled = true;
                StepViewer(-1);
                break;
            case ViewerKey.Next:
                e.Handled = true;
                StepViewer(1);
                break;
        }
    }

    /// <summary>The page the album is on: its words and arrows at once, its bytes as they come — the preview first when held.</summary>
    private void ShowViewerItem()
    {
        if (viewing is not { } album)
        {
            return;
        }
        var say = services.Say;
        var token = ++viewerShown;
        var paged = album.Count > 1 ? Visibility.Visible : Visibility.Collapsed;
        ViewerTitle.Text = album.Title(say);
        ViewerPosition.Text = album.Position(say) ?? string.Empty;
        ViewerPosition.Visibility = paged;
        ViewerPrevious.Visibility = paged;
        ViewerNext.Visibility = paged;
        ViewerPrevious.IsEnabled = album.HasPrevious;
        ViewerNext.IsEnabled = album.HasNext;
        ViewerProblem.Visibility = Visibility.Collapsed;
        StopViewerVideo();
        ViewerImage.Source = null;
        ViewerScroller.ChangeView(0, 0, 1, disableAnimation: true);
        ViewerScroller.Visibility = album.IsVideo ? Visibility.Collapsed : Visibility.Visible;
        ViewerVideo.Visibility = album.IsVideo ? Visibility.Visible : Visibility.Collapsed;
        ViewerZoomBar.Visibility = album.IsVideo ? Visibility.Collapsed : Visibility.Visible;
        ShowZoom();
        ViewerLoading.IsActive = true;
        _ = LoadViewerItemAsync(album.Current, album.IsVideo, token);
    }

    private async Task LoadViewerItemAsync(AttachmentDto item, bool video, int token)
    {
        try
        {
            if (item.HasPreview && pictures.TryGetValue(AttachmentCache.KeyFor(item.Id, preview: true), out var small))
            {
                if (video)
                {
                    ViewerVideo.PosterSource = small;
                }
                else
                {
                    ViewerImage.Source = small;
                }
            }
            // The ORIGINAL, never the preview a bubble draws.
            var (bytes, _) = await connection.Attachments.BytesAsync(item);
            if (token != viewerShown)
            {
                return;
            }
            if (bytes is null)
            {
                ViewerFailed();
                return;
            }
            if (video)
            {
                var stream = new InMemoryRandomAccessStream();
                using (var writer = new DataWriter(stream))
                {
                    writer.WriteBytes(bytes);
                    await writer.StoreAsync();
                    await writer.FlushAsync();
                    writer.DetachStream();
                }
                stream.Seek(0);
                if (token != viewerShown)
                {
                    stream.Dispose();
                    return;
                }
                ViewerVideo.Source = Windows.Media.Core.MediaSource.CreateFromStream(stream, item.Mime ?? "video/mp4");
                ViewerLoading.IsActive = false;
                return;
            }
            var picture = await DecodeAsync(bytes);
            if (token != viewerShown)
            {
                return;
            }
            if (picture is null)
            {
                ViewerFailed();
                return;
            }
            ViewerImage.Source = picture;
            ViewerLoading.IsActive = false;
            FitViewerImage();
        }
        catch (Exception e)
        {
            Diagnostics.Write($"viewing an attachment: {e.GetType().Name}");
            if (token == viewerShown)
            {
                ViewerFailed();
            }
        }
    }

    private void ViewerFailed()
    {
        ViewerLoading.IsActive = false;
        ViewerProblem.Text = services.Say.Get("The file could not be downloaded.");
        ViewerProblem.Visibility = Visibility.Visible;
    }

    /// <summary>The picture sized to the stage, times the zoom the scroller applies — which is what makes 1× show all of it.</summary>
    private void FitViewerImage()
    {
        if (ViewerScroller.ActualWidth > 0 && ViewerScroller.ActualHeight > 0)
        {
            ViewerImage.Width = ViewerScroller.ActualWidth;
            ViewerImage.Height = ViewerScroller.ActualHeight;
        }
    }

    private void ZoomViewer(Action<MediaAlbum> change)
    {
        if (viewing is not { IsVideo: false } album)
        {
            return;
        }
        change(album);
        ViewerScroller.ChangeView(null, null, (float)album.Zoom);
        ShowZoom();
    }

    private void ShowZoom()
    {
        if (viewing is not { } album)
        {
            return;
        }
        ViewerZoomText.Text = album.ZoomText(services.Culture);
        ViewerZoomIn.IsEnabled = album.CanZoomIn;
        ViewerZoomOut.IsEnabled = album.CanZoomOut;
    }

    private void StopViewerVideo()
    {
        ViewerVideo.MediaPlayer?.Pause();
        (ViewerVideo.Source as Windows.Media.Core.MediaSource)?.Dispose();
        ViewerVideo.Source = null;
        ViewerVideo.PosterSource = null;
    }

    /// <summary>Share — Windows' own share window, as the Mac's viewer offers its sharing menu — with the original's bytes.</summary>
    private async Task ShareViewedAsync()
    {
        if (viewing is not { } album)
        {
            return;
        }
        var item = album.Current;
        var (bytes, _) = await connection.Attachments.BytesAsync(item);
        if (bytes is null)
        {
            ViewerFailed();
            return;
        }
        try
        {
            await ShareSheet.ShareFileAsync(services.WindowHandle, AttachmentFiles.FileName(item), bytes);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"sharing an attachment: {e.GetType().Name}");
            ViewerProblem.Text = services.Say.Get("Something went wrong. Try again.");
            ViewerProblem.Visibility = Visibility.Visible;
        }
    }

    /// <summary>Save hands over the page that is up when it is clicked — the original's bytes.</summary>
    private async Task SaveViewedAsync()
    {
        if (viewing is not { } album)
        {
            return;
        }
        var item = album.Current;
        var (bytes, _) = await connection.Attachments.BytesAsync(item);
        if (bytes is null)
        {
            ViewerFailed();
            return;
        }
        await SaveBytesAsync(item, bytes);
    }

    // ---- calls --------------------------------------------------------------------------------------

    /// <summary>A call from this chat, to its other member: the window places it and shows it (docs/protocol.md, "Voice calls").</summary>
    private void RequestCall(bool video)
    {
        if (open is { } chat && connection.Chats.Chat(chat.ChatId)?.Chat is { Kind: "direct", PeerUserId: { } peer })
        {
            CallRequested?.Invoke(chat.ChatId, peer, video);
        }
    }

    /// <summary>Whether a call is on: while one is, a second is not placed.</summary>
    internal void ShowCallBusy(bool busy)
    {
        callBusy = busy;
        ShowCallButtons();
        DrawConversation(keepFromBottom: atNewest ? null : DistanceFromBottom);
    }

    /// <summary>
    /// A call record (ios CallRecordView, web bubble.rs): a phone or a camera — red for a call the reader missed — the record's
    /// own words in place of its placeholder body, and "Call back" wherever a call can be placed from here.
    /// </summary>
    private FrameworkElement CallRecordElement(ConversationModel chat, CallRecordDto call, bool mine, ThreadModel? inThread)
    {
        var say = services.Say;
        var resources = Application.Current.Resources;
        var ink = (Brush)resources[mine ? "TextOnAccentFillColorPrimaryBrush" : "TextFillColorPrimaryBrush"];
        var label = CallRecordText.Label(call.Outcome, call.DurationSecs, call.Video, mine, say);
        // A camera or a phone, in Segoe Fluent Icons.
        var glyph = new FontIcon
        {
            Glyph = ((char)(call.Video ? 0xE714 : 0xE717)).ToString(),
            FontSize = 18,
            Foreground = CallRecords.IsMissed(call, mine) ? (Brush)resources["SystemFillColorCriticalBrush"] : ink,
            VerticalAlignment = VerticalAlignment.Center,
        };
        var lines = new StackPanel { VerticalAlignment = VerticalAlignment.Center };
        lines.Children.Add(new TextBlock { Text = label, Foreground = ink, TextWrapping = TextWrapping.Wrap });
        var state = connection.Session.State;
        var direct = connection.Chats.Chat(chat.ChatId)?.Chat is { Kind: "direct", PeerUserId: not null };
        var (offered, video) = CallRecords.CallBack(call, direct, state.CallsEnabled, state.VideoCallsEnabled, inThread is not null);
        if (offered)
        {
            var back = new HyperlinkButton { Content = say.Get("Call back"), Padding = new Thickness(0, 2, 0, 0), IsEnabled = !callBusy };
            if (mine)
            {
                back.Foreground = ink;
            }
            AutomationProperties.SetHelpText(back, say.Get("Calls back"));
            back.Click += (_, _) => RequestCall(video);
            lines.Children.Add(back);
        }
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10 };
        row.Children.Add(glyph);
        row.Children.Add(lines);
        AutomationProperties.SetName(row, label);
        return row;
    }

    /// <summary>
    /// The call buttons: in a direct chat only — a family chat has nobody to ring, and the assistant has no ears — and only on
    /// a server that carries calls, and video calls, at all (<c>calls_enabled</c>, <c>video_calls_enabled</c> on <c>GET /me</c>).
    /// </summary>
    private void ShowCallButtons()
    {
        var state = connection.Session.State;
        var direct = open is { } chat && connection.Chats.Chat(chat.ChatId)?.Chat is { Kind: "direct", PeerUserId: not null };
        CallButton.Visibility = direct && state.CallsEnabled ? Visibility.Visible : Visibility.Collapsed;
        VideoCallButton.Visibility = direct && state.CallsEnabled && state.VideoCallsEnabled ? Visibility.Visible : Visibility.Collapsed;
        CallButton.IsEnabled = !callBusy;
        VideoCallButton.IsEnabled = !callBusy;
    }

    // ---- threads ------------------------------------------------------------------------------------

    /// <summary>A reaction from the conversation or the thread's panel, whose rows may be ones the cache does not hold.</summary>
    private Task<ApiError?> React(ConversationModel chat, ThreadModel? inThread, long messageId, string emoji) =>
        inThread is not null ? inThread.ReactAsync(messageId, emoji) : chat.ReactAsync(messageId, emoji);

    /// <summary>
    /// Open a chain beside the conversation (docs/protocol.md, "Threads"): at once on what is held, then read whole. A read
    /// still running when another chain opens belongs to nobody any more, and lands nowhere.
    /// </summary>
    private async Task OpenThreadAsync(long messageId)
    {
        if (open is not { } chat)
        {
            return;
        }
        var opening = new ThreadModel(chat.ChatId, messageId, connection.Chats, connection.Api, connection.Outbox);
        thread = opening;
        threadDrawn = string.Empty;
        ThreadComposer.Text = string.Empty;
        ThreadPanel.Visibility = Visibility.Visible;
        DrawThread(scrollToEnd: true);
        var error = await opening.LoadAsync();
        if (thread != opening || gone)
        {
            return;
        }
        if (error is not null)
        {
            Diagnostics.Write($"reading a thread: {error.Code} {error.Status}");
        }
        threadDrawn = string.Empty;
        DrawThread(scrollToEnd: true);
        ThreadComposer.Focus(FocusState.Programmatic);
    }

    private void CloseThread()
    {
        thread = null;
        threadDrawn = string.Empty;
        ThreadStack.Children.Clear();
        ThreadPanel.Visibility = Visibility.Collapsed;
    }

    /// <summary>
    /// The chain, drawn exactly as the chat draws it — the same bubbles, quotes, hidden-row rule and reactions — with the
    /// day pills a chain running over several days needs as much as the chat does.
    /// </summary>
    private void DrawThread(bool scrollToEnd = false)
    {
        if (thread is not { } chain || open is not { } chat || chain.ChatId != chat.ChatId)
        {
            return;
        }
        var say = services.Say;
        var bubbles = chain.Bubbles();
        var pending = chain.Pending();
        var drawn = string.Join(Row, bubbles.Select(bubble => string.Join(Field,
            bubble.Message.Id, bubble.Message.EditSeq, bubble.Message.ReactionSeq, bubble.Message.Poll?.PollSeq,
            bubble.Message.ReplyCount, bubble.Reads, connection.Chats.IsBlocked(bubble.Message.ReplyTo?.SenderId ?? 0))));
        drawn = $"{chain.RootId}{Field}{chain.Loaded}{Field}{chain.Failure?.Code}{Field}{string.Join(',', connection.Chats.Blocked())}" +
            $"{Field}{connection.Chats.Members().Count}{Field}{connection.Answers.Version}{Field}{DateOnly.FromDateTime(DateTime.Now)}" +
            $"{Field}{PreviewMarks(bubbles)}{Field}{MapPreviewSetting.Enabled}" +
            $"{Row}{drawn}{Row}{string.Join(Row, pending.Select(row => string.Join(Field, row.ClientMsgId, row.Failed)))}";
        if (drawn == threadDrawn)
        {
            return;
        }
        threadDrawn = drawn;
        var atEnd = scrollToEnd || ThreadScroller.VerticalOffset >= ThreadScroller.ScrollableHeight - 4;
        ThreadStack.Children.Clear();
        var replies = chain.Replies(bubbles);
        ThreadReplies.Text = say.Plural("%lld replies", replies, replies);
        var hasRoot = bubbles.Any(bubble => bubble.Message.Id == chain.RootId);
        if (bubbles.Count == 0)
        {
            if (chain.Failure is not null)
            {
                ThreadStack.Children.Add(new TextBlock
                {
                    Text = say.Get("Couldn't load the thread"),
                    FontWeight = FontWeights.SemiBold,
                    Margin = new Thickness(0, 16, 0, 2),
                });
                ThreadStack.Children.Add(new TextBlock { Text = say.Get("Try again in a moment."), Opacity = 0.7, TextWrapping = TextWrapping.Wrap });
            }
            else
            {
                ThreadStack.Children.Add(new ProgressRing { IsActive = true, Width = 24, Height = 24, Margin = new Thickness(0, 24, 0, 0) });
            }
        }
        // What is held is drawn; what could not be fetched is said, rather than passed off as the whole chain.
        ThreadNotice.Text = say.Get("Couldn't load the whole thread.");
        ThreadNotice.Visibility = bubbles.Count > 0 && chain.Failure is not null ? Visibility.Visible : Visibility.Collapsed;
        var family = connection.Chats.Chat(chat.ChatId)?.Chat.Kind == "family";
        var today = DateOnly.FromDateTime(DateTime.Now);
        foreach (var line in Timeline.Rows(bubbles, family, Reader, null, TimeZoneInfo.Local))
        {
            if (line.DayAbove is { } day)
            {
                ThreadStack.Children.Add(DayPill(Timeline.DayLabel(day, today, services.Culture, say)));
            }
            ThreadStack.Children.Add(BubbleElement(chat, line, chain));
        }
        foreach (var row in pending)
        {
            ThreadStack.Children.Add(PendingElement(chat, row));
        }
        // A reply needs a root to answer.
        ThreadComposer.IsEnabled = hasRoot;
        ThreadSend.IsEnabled = hasRoot;
        ThreadScroller.UpdateLayout();
        if (atEnd)
        {
            ThreadScroller.ChangeView(null, ThreadScroller.ScrollableHeight, null, disableAnimation: true);
        }
    }

    private void OnThreadComposerKey(object sender, KeyRoutedEventArgs e)
    {
        // Escape closes the panel, as it closes the web client's.
        if (e.Key == Windows.System.VirtualKey.Escape)
        {
            e.Handled = true;
            CloseThread();
            return;
        }
        if (e.Key != Windows.System.VirtualKey.Enter)
        {
            return;
        }
        // Enter sends and Shift+Enter is a new line, as in the conversation's own composer.
        var shift = InputKeyboardSource.GetKeyStateForCurrentThread(Windows.System.VirtualKey.Shift)
            .HasFlag(Windows.UI.Core.CoreVirtualKeyStates.Down);
        if (!shift)
        {
            e.Handled = true;
            SendInThread();
        }
    }

    /// <summary>
    /// A reply to the ROOT, whatever the reader was looking at — the iMessage rule, and the one that keeps the root's count
    /// honest (docs/protocol.md, "Answering from the thread"). Written down first, like every send.
    /// </summary>
    private void SendInThread()
    {
        if (open is not { } chat || thread is not { } chain || chain.ChatId != chat.ChatId || string.IsNullOrWhiteSpace(ThreadComposer.Text))
        {
            return;
        }
        var body = ThreadComposer.Text.Trim();
        chat.Send(body, replyToMessageId: chain.RootId,
            mentions: ComposerMentions.ForSend(body, connection.Chats.Members(), IsFamily(chat)));
        ThreadComposer.Text = string.Empty;
        threadDrawn = string.Empty;
        DrawThread(scrollToEnd: true);
        Queued();
    }

    // ---- sharing a place ------------------------------------------------------------------------

    /// <summary>
    /// Share where this device is, once (docs/protocol.md, "Locations"). Permission is settled FIRST, so somebody reading
    /// Windows's prompt never finds the composer busy (#41). Then a fix no older than two minutes: 100 m goes at once,
    /// and after twenty seconds the best fresh one does. It goes alone, with the draft as its caption and the reply it
    /// was primed for, and whatever is staged stays staged (MacConversationView.shareLocation, web location.rs).
    /// </summary>
    private async Task ShareLocationAsync()
    {
        if (open is not { } chat || locating || sendingMedia)
        {
            return;
        }
        var say = services.Say;
        if (Staging(chat.ChatId).BusyReason(editing is not null, say) is { } busy)
        {
            ShowProblem(busy);
            return;
        }
        var refused = say.Get("Family needs permission to use your location. Turn it on in Settings.");
        using var hunt = new CancellationTokenSource();
        locating = true;
        locationHunt = hunt;
        try
        {
            var permission = await LocationFinder.RequestPermissionAsync();
            if (gone || open != chat || hunt.IsCancellationRequested)
            {
                return;
            }
            switch (permission)
            {
                case LocationPermission.Denied:
                    ShowProblem(refused);
                    return;
                case LocationPermission.Failed:
                    ShowProblem(say.Get("Could not find your location."));
                    return;
                case LocationPermission.Unanswered:
                    // Nothing was taken and nothing runs: asking again is one click.
                    return;
            }
            ComposerError.Visibility = Visibility.Collapsed;
            finding = true;
            LocationRing.IsActive = true;
            LocationBar.Visibility = Visibility.Visible;
            AttachButton.IsEnabled = false;
            SendButton.IsEnabled = false;
            var (fix, denied) = await LocationFinder.CurrentFixAsync(hunt.Token);
            finding = false;
            if (gone || open != chat || hunt.IsCancellationRequested)
            {
                return;
            }
            if (fix is not { } found)
            {
                ShowProblem(denied ? refused : say.Get("Could not find your location."));
                return;
            }
            await SendLocationAsync(chat, found);
        }
        finally
        {
            locating = false;
            finding = false;
            if (ReferenceEquals(locationHunt, hunt))
            {
                locationHunt = null;
            }
            if (!gone)
            {
                LocationRing.IsActive = false;
                LocationBar.Visibility = Visibility.Collapsed;
                AttachButton.IsEnabled = recorder is null;
                SendButton.IsEnabled = !sendingMedia;
            }
        }
    }

    /// <summary>
    /// The place, queued like any attachment — through the staging folder, so a send interrupted by anything at all is a
    /// bubble that can be finished. An edit begun while it was being found keeps its words: the place then goes captionless.
    /// </summary>
    private async Task SendLocationAsync(ConversationModel chat, LocationFix fix)
    {
        var editingNow = editing is not null;
        var body = editingNow || string.IsNullOrWhiteSpace(ComposerBox.Text) ? string.Empty : ComposerBox.Text.TrimEnd();
        var replyTo = editingNow ? null : replyingTo?.Id;
        var place = new StagedMedia("location", string.Empty, ReadOnlyMemory<byte>.Empty,
            Latitude: fix.Latitude, Longitude: fix.Longitude, AccuracyM: fix.AccuracyM);
        var store = connection.Staging;
        var handles = new List<string>();
        sendingMedia = true;
        try
        {
            handles.Add(await Task.Run(() => store.Stage(place)));
            if (gone || open != chat)
            {
                return;
            }
            chat.Send(
                body, replyToMessageId: replyTo, pendingFiles: handles,
                mentions: ComposerMentions.ForSend(body, connection.Chats.Members(), IsFamily(chat)));
            if (!editingNow)
            {
                drafts.Sent(chat.ChatId);
                EndComposerMode(clear: true);
            }
            Queued();
        }
        catch (Exception e)
        {
            Diagnostics.Write($"sharing a location: {e.GetType().Name}");
            if (!gone)
            {
                ShowProblem(services.Say.Get("Could not share your location."));
            }
        }
        finally
        {
            store.Release(handles);
            sendingMedia = false;
        }
    }

    // ---- voice notes ----------------------------------------------------------------------------

    /// <summary>
    /// Record a voice note (docs/protocol.md, "Audio"): the microphone into an M4A, a bar with the time so far, Cancel and
    /// Stop — and staged when it stops, so a caption can be added and an accident thrown away.
    /// </summary>
    private async Task StartRecordingAsync()
    {
        if (open is not { } chat || recorder is not null)
        {
            return;
        }
        var say = services.Say;
        var strip = Staging(chat.ChatId);
        if (strip.BusyReason(editing is not null, say) is { } busy)
        {
            ShowProblem(busy);
            return;
        }
        if (!strip.CanStage)
        {
            ShowProblem(ComposerStaging.CapSentence(say));
            return;
        }
        var (started, failure) = await VoiceRecorder.StartAsync();
        if (started is null)
        {
            ShowProblem(failure == RecordingFailure.MicrophoneDenied
                ? say.Get("Family needs permission to use your microphone. Turn it on in Settings.")
                : say.Get("Couldn't start recording."));
            return;
        }
        if (open != chat || gone || recorder is not null)
        {
            // Granted after the chat went, or beside one already running: let go of the microphone.
            await started.DisposeAsync();
            return;
        }
        recorder = started;
        ComposerError.Visibility = Visibility.Collapsed;
        recordingTimer ??= RecordingClock();
        recordingTimer.Start();
        ShowRecording();
        RecordingBar.Visibility = Visibility.Visible;
        AttachButton.IsEnabled = false;
        RecordingStop.Focus(FocusState.Programmatic);
    }

    /// <summary>The bar's clock — and the five-minute ceiling, which presses Stop by itself.</summary>
    private DispatcherQueueTimer RecordingClock()
    {
        var timer = DispatcherQueue.CreateTimer();
        timer.Interval = TimeSpan.FromMilliseconds(250);
        timer.Tick += (_, _) =>
        {
            if (recorder is not { } running)
            {
                timer.Stop();
                return;
            }
            if (VoiceNotes.IsDone(running.Elapsed))
            {
                _ = StopRecordingAsync();
                return;
            }
            ShowRecording();
        };
        return timer;
    }

    private void ShowRecording() =>
        RecordingText.Text = VoiceNotes.RecordingLine(recorder?.Elapsed ?? TimeSpan.Zero, services.Say);

    private async Task StopRecordingAsync()
    {
        if (recorder is not { } stopping || open is not { } chat)
        {
            return;
        }
        recorder = null;
        recordingTimer?.Stop();
        RecordingBar.Visibility = Visibility.Collapsed;
        AttachButton.IsEnabled = true;
        var recorded = await stopping.StopAsync();
        if (recorded is not { } done || VoiceNotes.Staged(done.Bytes, done.Elapsed) is not { } staged)
        {
            ShowProblem(services.Say.Get("That recording was too short."));
            return;
        }
        if (open != chat || gone)
        {
            return;
        }
        Staging(chat.ChatId).Add(staged);
        DrawStaging();
        ComposerBox.Focus(FocusState.Programmatic);
    }

    private void CancelRecording()
    {
        if (recorder is not { } abandoned)
        {
            return;
        }
        recorder = null;
        recordingTimer?.Stop();
        RecordingBar.Visibility = Visibility.Collapsed;
        AttachButton.IsEnabled = true;
        _ = abandoned.DisposeAsync().AsTask();
    }

    /// <summary>One recording's row as drawn, so playback can keep it up to date.</summary>
    private sealed record AudioRow(long Id, Button Toggle, FontIcon Glyph, Slider Track, TextBlock Elapsed, double Total);

    /// <summary>
    /// A recording, drawn the way the Apple apps draw one (ios <c>AudioPlayerView</c>): a round play button, a scrubber, and
    /// where it is and how long it is under that — deliberately no waveform (docs/protocol.md, "Audio"). Downloaded rather
    /// than streamed: a player here cannot put the session's Authorization on the requests it would make.
    /// </summary>
    private FrameworkElement AudioElement(AttachmentDto attachment, bool mine)
    {
        var say = services.Say;
        var resources = Application.Current.Resources;
        var total = VoiceNotes.TotalSeconds(attachment.DurationMs);
        // An own balloon is filled with the accent, so nothing in it may be drawn in the accent too.
        var ink = (Brush)resources[mine ? "TextOnAccentFillColorPrimaryBrush" : "AccentFillColorDefaultBrush"];
        var glyph = new FontIcon
        {
            FontSize = 14,
            Foreground = (Brush)resources[mine ? "AccentFillColorDefaultBrush" : "TextOnAccentFillColorPrimaryBrush"],
        };
        var disc = new Grid { Width = 32, Height = 32 };
        disc.Children.Add(new Microsoft.UI.Xaml.Shapes.Ellipse { Fill = ink });
        disc.Children.Add(glyph);
        // The disc is 32; the target around it is 44.
        var toggle = new Button
        {
            Content = disc,
            Width = 44,
            Height = 44,
            Padding = new Thickness(0),
            CornerRadius = new CornerRadius(22),
            Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent),
            BorderThickness = new Thickness(0),
            VerticalAlignment = VerticalAlignment.Center,
        };
        var track = new Slider
        {
            Minimum = 0,
            Maximum = total,
            StepFrequency = 0.1,
            IsThumbToolTipEnabled = false,
        };
        AutomationProperties.SetName(track, say.Get("Position"));
        if (mine)
        {
            foreach (var key in new[] { "SliderTrackValueFill", "SliderTrackValueFillPointerOver", "SliderTrackValueFillPressed", "SliderThumbBackground", "SliderThumbBackgroundPointerOver", "SliderThumbBackgroundPressed" })
            {
                track.Resources[key] = ink;
            }
        }
        var elapsed = new TextBlock { FontSize = 11, Opacity = 0.75 };
        var length = new TextBlock { FontSize = 11, Opacity = 0.75, Text = MediaText.TimeLabel(total), HorizontalAlignment = HorizontalAlignment.Right };
        if (mine)
        {
            elapsed.Foreground = ink;
            length.Foreground = ink;
        }
        var times = new Grid();
        times.Children.Add(elapsed);
        times.Children.Add(length);
        var lines = new StackPanel { VerticalAlignment = VerticalAlignment.Center };
        lines.Children.Add(track);
        lines.Children.Add(times);
        var row = new Grid { ColumnSpacing = 10 };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.Children.Add(toggle);
        Grid.SetColumn(lines, 1);
        row.Children.Add(lines);
        var card = new Border
        {
            Child = row,
            Width = 260,
            Padding = new Thickness(8, 6, 8, 6),
            CornerRadius = new CornerRadius(12),
            BorderThickness = new Thickness(1),
            Background = mine
                ? new SolidColorBrush(Microsoft.UI.ColorHelper.FromArgb(0x24, 0xFF, 0xFF, 0xFF))
                : (Brush)resources["SubtleFillColorSecondaryBrush"],
            BorderBrush = mine
                ? new SolidColorBrush(Microsoft.UI.ColorHelper.FromArgb(0x29, 0xFF, 0xFF, 0xFF))
                : (Brush)resources["CardStrokeColorDefaultBrush"],
        };
        AutomationProperties.SetName(card, say.Format("Audio, %@", MediaText.TimeLabel(total)));

        var drawn = new AudioRow(attachment.Id, toggle, glyph, track, elapsed, total);
        audioRows[attachment.Id] = drawn;
        toggle.Click += (_, _) => _ = ToggleAudioAsync(attachment);
        // While the thumb is held, playback does not fight it for the position.
        track.AddHandler(UIElement.PointerPressedEvent, new PointerEventHandler((_, _) => scrubbingAudio = true), true);
        track.AddHandler(UIElement.PointerReleasedEvent, new PointerEventHandler((_, _) => scrubbingAudio = false), true);
        track.AddHandler(UIElement.PointerCaptureLostEvent, new PointerEventHandler((_, _) => scrubbingAudio = false), true);
        track.ValueChanged += (_, e) =>
        {
            if (movingTrack)
            {
                return;
            }
            elapsed.Text = MediaText.TimeLabel(e.NewValue);
            if (playingAudio == drawn.Id)
            {
                audioAtEnd = false;
                audio.PlaybackSession.Position = TimeSpan.FromSeconds(e.NewValue);
            }
        };
        ShowAudio(drawn.Id);
        return card;
    }

    private bool AudioRunning => audio.PlaybackSession.PlaybackState is Windows.Media.Playback.MediaPlaybackState.Playing
        or Windows.Media.Playback.MediaPlaybackState.Opening or Windows.Media.Playback.MediaPlaybackState.Buffering;

    private async Task ToggleAudioAsync(AttachmentDto attachment)
    {
        if (playingAudio == attachment.Id)
        {
            if (AudioRunning)
            {
                audio.Pause();
            }
            else
            {
                // Played through: Play starts it again, rather than doing nothing at the end.
                var total = VoiceNotes.TotalSeconds(attachment.DurationMs);
                if (audioAtEnd || VoiceNotes.ReplaysFromStart(audio.PlaybackSession.Position.TotalSeconds, total))
                {
                    audio.PlaybackSession.Position = TimeSpan.Zero;
                }
                audioAtEnd = false;
                audio.Play();
            }
            ShowPlayback();
            return;
        }
        // One at a time: whatever was playing stops, and says so.
        fetchingAudio = attachment.Id;
        audio.Pause();
        audioAtEnd = false;
        if (playingAudio is { } stopped)
        {
            playingAudio = null;
            ShowAudio(stopped);
        }
        byte[]? bytes;
        try
        {
            (bytes, _) = await connection.Attachments.BytesAsync(attachment);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"fetching a recording: {e.GetType().Name}");
            bytes = null;
        }
        if (gone || fetchingAudio != attachment.Id)
        {
            // Gone, or another recording was pressed while this one downloaded: that one plays.
            return;
        }
        if (bytes is null)
        {
            ShowProblem(services.Say.Get("The file could not be downloaded."));
            return;
        }
        var stream = new InMemoryRandomAccessStream();
        using (var writer = new DataWriter(stream))
        {
            writer.WriteBytes(bytes);
            await writer.StoreAsync();
            await writer.FlushAsync();
            writer.DetachStream();
        }
        stream.Seek(0);
        if (gone || fetchingAudio != attachment.Id)
        {
            stream.Dispose();
            return;
        }
        (audio.Source as Windows.Media.Core.MediaSource)?.Dispose();
        audio.Source = Windows.Media.Core.MediaSource.CreateFromStream(stream, attachment.Mime ?? VoiceNotes.Mime);
        playingAudio = attachment.Id;
        audio.Play();
        ShowPlayback();
    }

    /// <summary>The row for one recording: its glyph, and where it is — the start unless it is the one playing.</summary>
    private void ShowAudio(long id)
    {
        if (gone || !audioRows.TryGetValue(id, out var row))
        {
            return;
        }
        var active = playingAudio == id;
        var running = active && AudioRunning;
        // Play and Pause, in Segoe Fluent Icons.
        row.Glyph.Glyph = ((char)(running ? 0xE769 : 0xE768)).ToString();
        AutomationProperties.SetName(row.Toggle, running ? services.Say.Get("Pause") : services.Say.Get("Play"));
        if (active && scrubbingAudio)
        {
            return;
        }
        var at = !active ? 0 : audioAtEnd ? row.Total : Math.Min(audio.PlaybackSession.Position.TotalSeconds, row.Total);
        movingTrack = true;
        row.Track.Value = at;
        movingTrack = false;
        row.Elapsed.Text = MediaText.TimeLabel(at);
    }

    private void ShowPlayback()
    {
        if (playingAudio is { } id)
        {
            ShowAudio(id);
        }
    }

    /// <summary>Played to the end: it stops there, showing its whole length, and Play starts it again.</summary>
    private void AudioEnded()
    {
        if (gone)
        {
            return;
        }
        audio.Pause();
        audioAtEnd = true;
        ShowPlayback();
    }

    /// <summary>Leaving the chat, or the window: nothing keeps playing out of a bubble that is not there.</summary>
    private void StopAudio()
    {
        fetchingAudio = 0;
        playingAudio = null;
        audioAtEnd = false;
        scrubbingAudio = false;
        audio.Pause();
        (audio.Source as Windows.Media.Core.MediaSource)?.Dispose();
        audio.Source = null;
        audioRows.Clear();
    }

    /// <summary>A document: its name and its size, and a click that saves it. A recording plays in <see cref="AudioElement"/>.</summary>
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
        row.Children.Add(new TextBlock { Text = "📄", FontSize = 20, VerticalAlignment = VerticalAlignment.Center });
        Grid.SetColumn(lines, 1);
        row.Children.Add(lines);
        var button = new Button
        {
            Content = row,
            HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Left,
            Padding = new Thickness(8),
        };
        button.Click += (_, _) => _ = SaveAttachmentAsync(attachment);
        return button;
    }

    /// <summary>
    /// A place, labelled the way the Apple apps label one (ios <c>LocationAttachmentView</c>): a pin, the label or
    /// "Location", the coordinates with the sender's accuracy, and a click that hands it to the system map app. No map is
    /// drawn, so no tile provider is asked on the family's behalf (docs/protocol.md, "Locations" — the web client's choice
    /// too). With no coordinate it still says "Location", rather than drawing an empty balloon.
    /// </summary>
    private FrameworkElement LocationElement(AttachmentDto attachment, bool mine)
    {
        var say = services.Say;
        var resources = Application.Current.Resources;
        var name = AttachmentText.DisplayName("location", attachment.Name, say);
        var place = attachment.Latitude is { } latitude && attachment.Longitude is { } longitude
            ? (Latitude: latitude, Longitude: longitude)
            : ((double Latitude, double Longitude)?)null;
        var line = place is { } known ? MediaText.LocationLine(known.Latitude, known.Longitude, attachment.AccuracyM) : string.Empty;
        // An own balloon is filled with the accent, so nothing in it may be drawn in the accent too.
        var ink = (Brush)resources[mine ? "TextOnAccentFillColorPrimaryBrush" : "AccentFillColorDefaultBrush"];
        var disc = new Grid { Width = 36, Height = 36, VerticalAlignment = VerticalAlignment.Center };
        disc.Children.Add(new Microsoft.UI.Xaml.Shapes.Ellipse { Fill = ink });
        // A map pin, in Segoe Fluent Icons.
        disc.Children.Add(new FontIcon
        {
            Glyph = ((char)0xE707).ToString(),
            FontSize = 16,
            Foreground = (Brush)resources[mine ? "AccentFillColorDefaultBrush" : "TextOnAccentFillColorPrimaryBrush"],
        });
        var title = new TextBlock { Text = name, FontSize = 14, TextTrimming = TextTrimming.CharacterEllipsis };
        var lines = new StackPanel { VerticalAlignment = VerticalAlignment.Center };
        lines.Children.Add(title);
        var numbers = new TextBlock { Text = line, FontSize = 11, Opacity = 0.75, TextTrimming = TextTrimming.CharacterEllipsis };
        if (line.Length > 0)
        {
            lines.Children.Add(numbers);
        }
        var row = new Grid { ColumnSpacing = 10 };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        row.Children.Add(disc);
        Grid.SetColumn(lines, 1);
        row.Children.Add(lines);
        // "Opens elsewhere", in Segoe Fluent Icons: the click hands the place to the map app.
        var away = new FontIcon { Glyph = ((char)0xE8A7).ToString(), FontSize = 12, Opacity = 0.6, VerticalAlignment = VerticalAlignment.Center };
        if (place is not null)
        {
            Grid.SetColumn(away, 2);
            row.Children.Add(away);
        }
        if (mine)
        {
            title.Foreground = ink;
            numbers.Foreground = ink;
            away.Foreground = ink;
        }
        // The map above the row, where there is a place and the reader has maps on: drawing it is asking OpenStreetMap.
        FrameworkElement content = row;
        if (place is { } mapped && MapPreviewSetting.Enabled)
        {
            var stacked = new StackPanel { Spacing = 8 };
            stacked.Children.Add(MapElement(mapped.Latitude, mapped.Longitude));
            stacked.Children.Add(row);
            content = stacked;
        }
        var button = new Button
        {
            Content = content,
            Width = 260,
            HorizontalContentAlignment = HorizontalAlignment.Stretch,
            Padding = new Thickness(8, 6, 10, 6),
            CornerRadius = new CornerRadius(12),
            BorderThickness = new Thickness(1),
            Background = mine
                ? new SolidColorBrush(Microsoft.UI.ColorHelper.FromArgb(0x24, 0xFF, 0xFF, 0xFF))
                : (Brush)resources["SubtleFillColorSecondaryBrush"],
            BorderBrush = mine
                ? new SolidColorBrush(Microsoft.UI.ColorHelper.FromArgb(0x29, 0xFF, 0xFF, 0xFF))
                : (Brush)resources["CardStrokeColorDefaultBrush"],
            IsEnabled = place is not null,
        };
        AutomationProperties.SetName(button, line.Length > 0 ? $"{name}. {line}" : name);
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

    /// <summary>The map's size inside a location's 260-pixel card, less the card's padding and border.</summary>
    private const double MapWidth = 240;
    private const double MapHeight = 132;

    /// <summary>Map tiles decoded once, so a redraw reuses them instead of decoding (and flashing) again.</summary>
    private readonly Dictionary<string, BitmapImage> mapPictures = new(StringComparer.Ordinal);

    /// <summary>
    /// A shared place's map, as the Apple apps draw it in the bubble: the streets around it with the place at the middle,
    /// rounded, not interactive — a pannable map inside a scrolling conversation fights every drag — and credited to
    /// OpenStreetMap, as its licence requires. Tiles appear as they arrive; before then the square is a quiet grey.
    /// </summary>
    private FrameworkElement MapElement(double latitude, double longitude)
    {
        var resources = Application.Current.Resources;
        var canvas = new Canvas
        {
            Width = MapWidth,
            Height = MapHeight,
            Background = (Brush)resources["SubtleFillColorSecondaryBrush"],
            Clip = new RectangleGeometry { Rect = new Windows.Foundation.Rect(0, 0, MapWidth, MapHeight) },
        };
        foreach (var tile in MapView.Tiles(latitude, longitude, MapWidth, MapHeight))
        {
            var image = new Image { Width = MapView.TileSize, Height = MapView.TileSize, Stretch = Stretch.Fill };
            Canvas.SetLeft(image, tile.Left);
            Canvas.SetTop(image, tile.Top);
            canvas.Children.Add(image);
            _ = ShowTileAsync(image, tile);
        }
        // The place: a red dot where the pin stands, ringed in white so it reads on any colour of map.
        var ring = new Microsoft.UI.Xaml.Shapes.Ellipse
        {
            Width = 18,
            Height = 18,
            Fill = new SolidColorBrush(Microsoft.UI.ColorHelper.FromArgb(0xFF, 0xFF, 0xFF, 0xFF)),
        };
        Canvas.SetLeft(ring, (MapWidth / 2) - 9);
        Canvas.SetTop(ring, (MapHeight / 2) - 9);
        canvas.Children.Add(ring);
        var dot = new Microsoft.UI.Xaml.Shapes.Ellipse
        {
            Width = 12,
            Height = 12,
            Fill = new SolidColorBrush(Microsoft.UI.ColorHelper.FromArgb(0xFF, 0xE5, 0x39, 0x35)),
        };
        Canvas.SetLeft(dot, (MapWidth / 2) - 6);
        Canvas.SetTop(dot, (MapHeight / 2) - 6);
        canvas.Children.Add(dot);

        var map = new Grid { Width = MapWidth, Height = MapHeight };
        map.Children.Add(canvas);
        // OpenStreetMap's licence asks for its name on every map drawn from it. A proper name, the same in every language.
        map.Children.Add(new Border
        {
            HorizontalAlignment = HorizontalAlignment.Right,
            VerticalAlignment = VerticalAlignment.Bottom,
            Padding = new Thickness(5, 1, 5, 2),
            CornerRadius = new CornerRadius(4, 0, 0, 0),
            Background = new SolidColorBrush(Microsoft.UI.ColorHelper.FromArgb(0xC8, 0xFF, 0xFF, 0xFF)),
            Child = new TextBlock
            {
                Text = "© OpenStreetMap contributors",
                FontSize = 9,
                Foreground = new SolidColorBrush(Microsoft.UI.ColorHelper.FromArgb(0xFF, 0x33, 0x33, 0x33)),
            },
        });
        try
        {
            // Rounded like every other picture in a bubble. A Grid's corner radius rounds its background only, so the tiles
            // are clipped by the composition layer instead.
            var visual = Microsoft.UI.Xaml.Hosting.ElementCompositionPreview.GetElementVisual(map);
            var geometry = visual.Compositor.CreateRoundedRectangleGeometry();
            geometry.Size = new System.Numerics.Vector2((float)MapWidth, (float)MapHeight);
            geometry.CornerRadius = new System.Numerics.Vector2(8, 8);
            visual.Clip = visual.Compositor.CreateGeometricClip(geometry);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"rounding a map: {e.GetType().Name}");
        }
        // Decorative: the card's own name already says the place and its coordinates.
        AutomationProperties.SetAccessibilityView(map, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        return map;
    }

    private async Task ShowTileAsync(Image image, MapTile tile)
    {
        try
        {
            if (!mapPictures.TryGetValue(tile.Key, out var picture))
            {
                if (await services.Maps.BytesAsync(tile) is not { } bytes || gone)
                {
                    return;
                }
                if (await DecodeAsync(bytes) is not { } decoded)
                {
                    return;
                }
                if (mapPictures.Count >= 96)
                {
                    mapPictures.Remove(mapPictures.Keys.First());
                }
                picture = mapPictures[tile.Key] = decoded;
            }
            image.Source = picture;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"drawing a map tile: {e.GetType().Name}");
        }
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
        threadDrawn = string.Empty;
        DrawThread();
    }

    private async void OnScrolled(object? sender, ScrollViewerViewChangedEventArgs e)
    {
        if (e.IsIntermediate)
        {
            return;
        }
        atNewest = MessageScroller.VerticalOffset >= MessageScroller.ScrollableHeight - 24;
        ShowJump();
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
        // A photo being replied to may now go to the assistant with the draft.
        DrawPictureNotice();
    }

    private void StartEdit(MessageDto message)
    {
        replyingTo = null;
        editing = message;
        BannerText.Text = services.Say.Get("Editing message");
        BannerCancel.Content = services.Say.Get("Cancel editing");
        BannerPanel.Visibility = Visibility.Visible;
        SendButton.Content = services.Say.Get("Save");
        ShowAssistantButtons();
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
        ShowAssistantButtons();
        if (clear)
        {
            ComposerBox.Text = string.Empty;
        }
        DrawPictureNotice();
    }

    // ---- polls ---------------------------------------------------------------------------------------

    private PollCard.Seen PollSeen() => new(
        services.Say, Reader, connection.Chats.Member, connection.Chats.IsBlocked,
        PollText.MemberCount(connection.Chats.Members()));

    /// <summary>How many open polls the reader has still to answer — nothing drawn at none.</summary>
    private void ShowPollsBadge()
    {
        var unanswered = openPolls?.Unanswered() ?? 0;
        OpenPollsBadge.Value = unanswered;
        OpenPollsBadge.Visibility = unanswered > 0 ? Visibility.Visible : Visibility.Collapsed;
    }

    /// <summary>
    /// A poll: the question is the message and the options ride with it, through the outbox like any other
    /// message. It names members the way a message does, and answers the reply being written, if any.
    /// </summary>
    private async Task AskPollAsync()
    {
        if (open is not { } chat || !IsFamily(chat))
        {
            return;
        }
        (string Question, string[] Options)? asked;
        try
        {
            asked = await PollComposer.AskAsync(XamlRoot, services.Say);
        }
        catch (Exception e)
        {
            // Only one dialog may be up at a time.
            Diagnostics.Write($"asking a poll: {e.GetType().Name}");
            return;
        }
        if (asked is not { } poll || open != chat)
        {
            return;
        }
        chat.Send(
            poll.Question, replyToMessageId: replyingTo?.Id, pollOptions: poll.Options,
            mentions: ComposerMentions.ForSend(poll.Question, connection.Chats.Members(), familyChat: true));
        if (replyingTo is not null)
        {
            EndComposerMode(clear: false);
        }
        Queued();
    }

    private async Task ShowOpenPollsAsync()
    {
        if (openPolls is not { } model)
        {
            return;
        }
        try
        {
            await OpenPollsSheet.ShowAsync(XamlRoot, model, PollSeen, connection);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"showing the open polls: {e.GetType().Name}");
        }
        if (openPolls != model)
        {
            return;
        }
        ShowPollsBadge();
        conversationDrawn = string.Empty;
        DrawConversation(keepFromBottom: atNewest ? null : DistanceFromBottom);
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
    /// A body laid out (<see cref="BubbleBody"/>): one text block fills the bubble's words and answers null; a body with a
    /// table answers its blocks, text and tables in turn.
    /// </summary>
    private FrameworkElement? BodyElement(TextBlock words, string body, MentionDto[]? named, bool mine)
    {
        var blocks = BubbleBody.Lay(body, named, connection.Chats.Members(), Reader, connection.Chats.IsBlocked);
        if (blocks is [BodyTextBlock { Runs: var only }])
        {
            FillRuns(words, only, mine);
            return null;
        }
        var ink = (Brush)Application.Current.Resources[mine ? "TextOnAccentFillColorPrimaryBrush" : "TextFillColorPrimaryBrush"];
        var panel = new StackPanel { Spacing = 6 };
        foreach (var block in blocks)
        {
            if (block is BodyTableBlock { Table: var table })
            {
                panel.Children.Add(TableElement(table, ink));
            }
            else if (block is BodyTextBlock { Runs: var runs })
            {
                var text = new TextBlock { TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true, Foreground = ink };
                FillRuns(text, runs, mine);
                panel.Children.Add(text);
            }
        }
        return panel;
    }

    /// <summary>
    /// A text block's runs as inlines. Links in the accent — underlined in the bubble's own ink on an own bubble — opening a
    /// beat late; names and the assistant's tokens BOLD in the bubble's own ink, never a colour of their own, which a tinted
    /// ground would swallow; and a name this reader can message, a door onto that chat.
    /// </summary>
    private void FillRuns(TextBlock words, IReadOnlyList<BodyRun> runs, bool mine)
    {
        var ink = (Brush)Application.Current.Resources[mine ? "TextOnAccentFillColorPrimaryBrush" : "TextFillColorPrimaryBrush"];
        words.Text = string.Empty;
        words.Inlines.Clear();
        foreach (var run in runs)
        {
            var styled = StyledRun(run.Text, run.Style, run.Marked);
            if (run.MemberId is { } id && run.Opens)
            {
                var door = new Hyperlink { UnderlineStyle = UnderlineStyle.None, Foreground = ink };
                door.Inlines.Add(styled);
                door.Click += (_, _) => _ = OpenDirectAsync(id);
                words.Inlines.Add(door);
            }
            else if (BubbleBody.Openable(run.Link) is { } uri)
            {
                var link = new Hyperlink();
                if (mine)
                {
                    link.Foreground = ink;
                }
                link.Inlines.Add(styled);
                link.Click += (_, _) => OpenLinkSoon(uri);
                words.Inlines.Add(link);
            }
            else
            {
                words.Inlines.Add(styled);
            }
        }
    }

    /// <summary>
    /// One run in its markdown style: bold, italic, struck, code in a monospaced face, and a heading on the apps' ladder
    /// (1.29, 1.18 and 1.00 of the body). A name or an assistant token is semibold whatever it sits in.
    /// </summary>
    private static Run StyledRun(string text, MarkdownStyle style, bool marked)
    {
        var run = new Run { Text = text };
        if (marked)
        {
            run.FontWeight = FontWeights.SemiBold;
        }
        else if (style.Strong)
        {
            run.FontWeight = FontWeights.Bold;
        }
        if (style.Emphasis)
        {
            run.FontStyle = Windows.UI.Text.FontStyle.Italic;
        }
        if (style.Strikethrough)
        {
            run.TextDecorations = Windows.UI.Text.TextDecorations.Strikethrough;
        }
        if (style.Code || style.Face == MarkdownFace.Monospaced)
        {
            run.FontFamily = new FontFamily("Cascadia Mono, Consolas");
        }
        switch (style.Face)
        {
            case MarkdownFace.Heading1:
                run.FontSize = 18;
                run.FontWeight = FontWeights.Bold;
                break;
            case MarkdownFace.Heading2:
                run.FontSize = 16.5;
                run.FontWeight = FontWeights.Bold;
                break;
            case MarkdownFace.Heading3:
                run.FontWeight = FontWeights.SemiBold;
                break;
        }
        return run;
    }

    /// <summary>
    /// A table (ios MessageBodyView): the header semibold above a one-pixel rule, the columns sharing the width, cells
    /// wrapping and aligned as the delimiter row says. Cells carry their styles and nothing else — no links, no names.
    /// </summary>
    private static FrameworkElement TableElement(MarkdownTable table, Brush ink)
    {
        var grid = new Grid { ColumnSpacing = 12, RowSpacing = 4, Margin = new Thickness(0, 2, 0, 2) };
        for (var column = 0; column < table.ColumnCount; column++)
        {
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        }
        void AddRow(IReadOnlyList<MarkdownText> cells, bool header)
        {
            var row = grid.RowDefinitions.Count;
            grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            for (var column = 0; column < cells.Count && column < table.ColumnCount; column++)
            {
                var cell = new TextBlock
                {
                    TextWrapping = TextWrapping.Wrap,
                    IsTextSelectionEnabled = true,
                    Foreground = ink,
                    TextAlignment = table.Alignment(column) switch
                    {
                        MarkdownAlignment.Center => TextAlignment.Center,
                        MarkdownAlignment.Trailing => TextAlignment.Right,
                        _ => TextAlignment.Left,
                    },
                };
                if (header)
                {
                    cell.FontWeight = FontWeights.SemiBold;
                }
                foreach (var run in cells[column].Runs)
                {
                    cell.Inlines.Add(StyledRun(run.Text, run.Style, marked: false));
                }
                Grid.SetRow(cell, row);
                Grid.SetColumn(cell, column);
                grid.Children.Add(cell);
            }
        }
        AddRow(table.Header, header: true);
        grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        var rule = new Border { Height = 1, Background = ink, Opacity = 0.3 };
        Grid.SetRow(rule, 1);
        Grid.SetColumnSpan(rule, Math.Max(1, table.ColumnCount));
        grid.Children.Add(rule);
        foreach (var cells in table.Rows)
        {
            AddRow(cells, header: false);
        }
        return grid;
    }

    /// <summary>
    /// The preview under a link (ios LinkPreviewCard): the page's picture when it offered one, the site, the title in two lines
    /// and the description in two. Its own solid ground with a hairline, so it reads the same on the accent of an own bubble as
    /// on a card. A click opens the link a beat late, like the link itself, so a double click is still the heart.
    /// </summary>
    private FrameworkElement PreviewCard(LinkPreview preview)
    {
        var resources = Application.Current.Resources;
        var column = new StackPanel();
        if (connection.Previews.Image(preview.Url) is { } bytes && PreviewPicture(preview.Url.AbsoluteUri, bytes) is { } picture)
        {
            column.Children.Add(new Image { Source = picture, Height = 120, Stretch = Stretch.UniformToFill, HorizontalAlignment = HorizontalAlignment.Stretch });
        }
        var words = new StackPanel { Spacing = 2, Padding = new Thickness(10, 8, 10, 8) };
        words.Children.Add(new TextBlock
        {
            Text = preview.SiteName,
            FontSize = 11,
            MaxLines = 1,
            TextTrimming = TextTrimming.CharacterEllipsis,
            Foreground = (Brush)resources["TextFillColorSecondaryBrush"],
        });
        words.Children.Add(new TextBlock
        {
            Text = preview.Title,
            FontSize = 13,
            FontWeight = FontWeights.SemiBold,
            MaxLines = 2,
            TextWrapping = TextWrapping.Wrap,
            TextTrimming = TextTrimming.CharacterEllipsis,
            Foreground = (Brush)resources["TextFillColorPrimaryBrush"],
        });
        if (preview.Description is { } description)
        {
            words.Children.Add(new TextBlock
            {
                Text = description,
                FontSize = 11,
                MaxLines = 2,
                TextWrapping = TextWrapping.Wrap,
                TextTrimming = TextTrimming.CharacterEllipsis,
                Foreground = (Brush)resources["TextFillColorSecondaryBrush"],
            });
        }
        column.Children.Add(words);
        var card = new Border
        {
            Child = column,
            MaxWidth = 360,
            Margin = new Thickness(0, 4, 0, 0),
            CornerRadius = new CornerRadius(12),
            Background = (Brush)resources["SolidBackgroundFillColorTertiaryBrush"],
            BorderBrush = (Brush)resources["CardStrokeColorDefaultBrush"],
            BorderThickness = new Thickness(1),
            HorizontalAlignment = HorizontalAlignment.Left,
        };
        AutomationProperties.SetName(card, $"{preview.Title}, {preview.SiteName}");
        AutomationProperties.SetHelpText(card, services.Say.Get("Opens the link"));
        ToolTipService.SetToolTip(card, preview.Url.AbsoluteUri);
        var url = preview.Url;
        card.Tapped += (_, e) =>
        {
            e.Handled = true;
            OpenLinkSoon(url);
        };
        return card;
    }

    /// <summary>Card pictures that turned out not to decode: a redraw draws those cards without one.</summary>
    private int previewPicturesRefused;

    /// <summary>
    /// What the cards under these bubbles' links are doing — the part of the previews a drawing depends on. Asks exactly what
    /// <see cref="BubbleElement"/> asks, for the same links, and nothing about any other chat's.
    /// </summary>
    private string PreviewMarks(IEnumerable<Bubble> bubbles)
    {
        if (!LinkPreviewSetting.Enabled)
        {
            return "off";
        }
        var marks = bubbles.Select(bubble =>
            bubble.Message.Call is null
            && BubbleBody.PreviewLink(connection.Answers.BodyOf(bubble.Message), emojiOnly: false) is { } link
            && connection.Previews.State(link) is { } state
                ? (int)state.Status
                : -1);
        return $"{string.Join(',', marks)}{Field}{previewPicturesRefused}";
    }

    /// <summary>
    /// A card's picture, decoded once and at most 1200 pixels on its longest edge — a 6000 by 4000 photo under the byte cap
    /// would otherwise cost ~96 MB decoded. Null when the bytes are no picture.
    /// </summary>
    private BitmapImage? PreviewPicture(string key, byte[] bytes)
    {
        if (previewPictures.TryGetValue(key, out var known))
        {
            return known;
        }
        if (previewPictures.Count >= 64)
        {
            // The oldest goes, never all of them: emptying the table re-decoded every card on screen at the next redraw, and a
            // card whose picture failed asked for that redraw again.
            previewPictures.Remove(previewPictures.Keys.First());
        }
        var picture = new BitmapImage();
        previewPictures[key] = picture;
        _ = DecodePreviewPictureAsync(key, picture, bytes);
        return picture;
    }

    private async Task DecodePreviewPictureAsync(string key, BitmapImage picture, byte[] bytes)
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
            var decoder = await Windows.Graphics.Imaging.BitmapDecoder.CreateAsync(stream);
            const uint Longest = 1200;
            if (decoder.PixelWidth >= decoder.PixelHeight && decoder.PixelWidth > Longest)
            {
                picture.DecodePixelWidth = (int)Longest;
            }
            else if (decoder.PixelHeight > decoder.PixelWidth && decoder.PixelHeight > Longest)
            {
                picture.DecodePixelHeight = (int)Longest;
            }
            stream.Seek(0);
            await picture.SetSourceAsync(stream);
        }
        catch (Exception e)
        {
            // Not a picture after all: the card keeps its words, and the next redraw draws it without one.
            Diagnostics.Write($"decoding a link preview picture: {e.GetType().Name}");
            previewPictures[key] = null;
            previewPicturesRefused++;
            QueueRedraw();
        }
    }

    /// <summary>Open a link a beat late — the Mac's 350 ms — unless the click was the second half of a heart.</summary>
    private void OpenLinkSoon(Uri uri)
    {
        pendingLink?.Stop();
        pendingLink = null;
        // A double click reaches the link twice: once the heart has landed, the second click opens nothing.
        if (Environment.TickCount64 - lastHeart < BubbleBody.LinkDelay.TotalMilliseconds)
        {
            return;
        }
        var timer = DispatcherQueue.CreateTimer();
        timer.Interval = BubbleBody.LinkDelay;
        timer.IsRepeating = false;
        timer.Tick += (sender, _) =>
        {
            sender.Stop();
            if (pendingLink != sender)
            {
                return;
            }
            pendingLink = null;
            _ = Windows.System.Launcher.LaunchUriAsync(uri);
        };
        pendingLink = timer;
        timer.Start();
    }

    private void CancelLinkForHeart()
    {
        lastHeart = Environment.TickCount64;
        pendingLink?.Stop();
        pendingLink = null;
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
        // While a place is being found, Send waits for it: the words typed meanwhile are its caption.
        if (open is not { } chat || sendingMedia || finding)
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
        drafts.Sent(chat.ChatId);
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
            drafts.Sent(chat.ChatId);
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

    /// <summary>
    /// The composer's menu of what a message can carry, in the Mac's order: a photo for the assistant where all three locks
    /// allow it, a file, the clipboard, a recording, a place — and, in the family chat alone, a poll.
    /// </summary>
    private void ShowAttachMenu()
    {
        if (open is not { } chat)
        {
            return;
        }
        var say = services.Say;
        var menu = new MenuFlyout { Placement = Microsoft.UI.Xaml.Controls.Primitives.FlyoutPlacementMode.TopEdgeAlignedLeft };
        var state = connection.Session.State;
        // The assistant's own chat, a server that can see, and a family that allows it — all three, or no door that lies.
        if (AssistantPictures.OffersPictureAttach(
                connection.Chats.Chat(chat.ChatId)?.Chat.Kind == "ai", state.Assistant?.Vision == true, state.Family?.AiVision == true))
        {
            var pictures = new MenuFlyoutItem { Text = say.Get("Show the Assistant a Photo…"), Icon = new SymbolIcon(Symbol.Pictures) };
            pictures.Click += (_, _) => _ = PickPicturesAsync();
            menu.Items.Add(pictures);
        }
        var file = new MenuFlyoutItem { Text = say.Get("Attach a File…"), Icon = new SymbolIcon(Symbol.Attach) };
        file.Click += (_, _) => _ = PickAsync();
        menu.Items.Add(file);
        var paste = new MenuFlyoutItem { Text = say.Get("Paste"), Icon = new SymbolIcon(Symbol.Paste) };
        paste.Click += (_, _) => _ = PasteAsync(fromMenu: true);
        menu.Items.Add(paste);
        // A microphone, in Segoe Fluent Icons.
        var record = new MenuFlyoutItem { Text = say.Get("Record Audio"), Icon = new FontIcon { Glyph = ((char)0xE720).ToString() } };
        record.Click += (_, _) => _ = StartRecordingAsync();
        menu.Items.Add(record);
        // A map pin, in Segoe Fluent Icons.
        var place = new MenuFlyoutItem { Text = say.Get("Location"), Icon = new FontIcon { Glyph = ((char)0xE707).ToString() }, IsEnabled = !locating };
        place.Click += (_, _) => _ = ShareLocationAsync();
        menu.Items.Add(place);
        if (IsFamily(chat))
        {
            // A bulleted list, in Segoe Fluent Icons.
            var poll = new MenuFlyoutItem { Text = say.Get("Poll"), Icon = new FontIcon { Glyph = ((char)0xE8FD).ToString() } };
            poll.Click += (_, _) => _ = AskPollAsync();
            menu.Items.Add(poll);
        }
        menu.ShowAt(AttachButton);
    }

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

    /// <summary>Photos for the assistant: a picker of pictures, and the first four taken — the four the model is shown.</summary>
    private async Task PickPicturesAsync()
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
        IReadOnlyList<StorageFile> files;
        try
        {
            var picker = new FileOpenPicker { SuggestedStartLocation = PickerLocationId.PicturesLibrary, ViewMode = PickerViewMode.Thumbnail };
            foreach (var type in new[] { ".jpg", ".jpeg", ".png", ".heic", ".heif", ".webp", ".gif", ".bmp", ".tif", ".tiff" })
            {
                picker.FileTypeFilter.Add(type);
            }
            WinRT.Interop.InitializeWithWindow.Initialize(picker, services.WindowHandle);
            files = await picker.PickMultipleFilesAsync();
        }
        catch (Exception e)
        {
            Diagnostics.Write($"picking photos for the assistant: {e.GetType().Name}");
            ShowProblem(services.Say.Get("Something went wrong. Try again."));
            return;
        }
        if (open == chat)
        {
            await IngestAsync(chat, strip, [.. files.Take(AssistantPictures.MaxPerQuestion)]);
        }
    }

    /// <summary>
    /// What the strip over the composer says before a photograph goes to the assistant: in its own chat, of every photo
    /// staged; in the family chat, of an @ai draft that carries one or replies to one (docs/protocol.md, "Pictures").
    /// </summary>
    private void DrawPictureNotice()
    {
        string? said = null;
        if (open is { } chat)
        {
            var state = connection.Session.State;
            var kind = connection.Chats.Chat(chat.ChatId)?.Chat.Kind;
            var serverCanSee = state.Assistant?.Vision == true;
            var familyAllows = state.Family?.AiVision == true;
            // What travels of a photo staged here is the photo as prepared.
            List<PictureCandidate> staged =
                [.. Staging(chat.ChatId).Items.Select(item => new PictureCandidate(item.Kind, item.Mime, item.Bytes.Length))];
            if (kind == "ai")
            {
                said = AssistantPictures.PrivateNotice(
                    staged, AssistantPictures.OffersPictureAttach(true, serverCanSee, familyAllows), services.Say);
            }
            else if (kind == "family" && editing is null && state.Assistant is { } assistant)
            {
                List<PictureCandidate> quoted =
                [
                    .. (replyingTo?.Media ?? []).Select(attachment =>
                        PictureCandidate.OfAttachment(attachment.Kind, attachment.Mime ?? string.Empty, attachment.Size, attachment.HasPreview)),
                ];
                said = MentionPictureNotice.Of(
                    ComposerBox.Text, staged, quoted,
                    new PictureSwitches(serverCanSee, familyAllows, state.Family?.AiHistory == true, state.Family?.AiHistoryPhotos == true, assistant.Images))
                    ?.Sentence(services.Say);
            }
        }
        PictureNoticeText.Text = said ?? string.Empty;
        PictureNoticeText.Visibility = said is null ? Visibility.Collapsed : Visibility.Visible;
    }

    /// <summary>
    /// A paste into the box. WORDS WIN — an ordinary text paste stays one, and the box pastes it itself — but copied FILES,
    /// which bring their names along as text, and a picture copied on its own are staged as the attach button stages them.
    /// </summary>
    private void OnComposerPaste(object sender, TextControlPasteEventArgs e)
    {
        DataPackageView content;
        try
        {
            content = Clipboard.GetContent();
        }
        catch (Exception exception)
        {
            Diagnostics.Write($"reading the clipboard: {exception.GetType().Name}");
            return;
        }
        var files = content.Contains(StandardDataFormats.StorageItems);
        var picture = content.Contains(StandardDataFormats.Bitmap) && !content.Contains(StandardDataFormats.Text);
        if (!files && !picture)
        {
            return;
        }
        e.Handled = true;
        _ = PasteAsync(fromMenu: false, content);
    }

    /// <summary>What the clipboard holds, taken the web client's way (<see cref="PasteRules.Decision"/>): files staged, words typed, a lone picture staged as one.</summary>
    private async Task PasteAsync(bool fromMenu, DataPackageView? content = null)
    {
        if (open is not { } chat)
        {
            return;
        }
        var say = services.Say;
        try
        {
            content ??= Clipboard.GetContent();
            var files = content.Contains(StandardDataFormats.StorageItems)
                ? (await content.GetStorageItemsAsync()).OfType<StorageFile>().ToList()
                : [];
            var text = content.Contains(StandardDataFormats.Text) ? await content.GetTextAsync() : string.Empty;
            switch (PasteRules.Decision([.. files.Select(file => file.Name)], text))
            {
                case PasteDecision.Attach:
                    await IngestAsync(chat, Staging(chat.ChatId), files);
                    return;
                case PasteDecision.Type:
                    Type(text);
                    return;
            }
            if (content.Contains(StandardDataFormats.Bitmap))
            {
                var picture = await PictureFileAsync(await content.GetBitmapAsync());
                await IngestAsync(chat, Staging(chat.ChatId), [picture]);
                return;
            }
            if (fromMenu)
            {
                ShowProblem(say.Get("There's nothing to paste."));
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"pasting: {e.GetType().Name}");
            ShowProblem(say.Get("Something went wrong. Try again."));
        }
    }

    /// <summary>Words into the draft where the caret is, over whatever was selected.</summary>
    private void Type(string text)
    {
        var at = Math.Min(ComposerBox.SelectionStart, ComposerBox.Text.Length);
        var until = Math.Min(at + ComposerBox.SelectionLength, ComposerBox.Text.Length);
        ComposerBox.Text = ComposerBox.Text[..at] + text + ComposerBox.Text[until..];
        ComposerBox.SelectionStart = at + text.Length;
        ComposerBox.Focus(FocusState.Programmatic);
    }

    /// <summary>A picture that came with no file of its own, written to one — named as every client names a pasted item.</summary>
    private static async Task<StorageFile> PictureFileAsync(RandomAccessStreamReference reference)
    {
        using var source = await reference.OpenReadAsync();
        var mime = source.ContentType is { Length: > 0 } type && type.StartsWith("image/", StringComparison.Ordinal) ? type : "image/png";
        var file = await ApplicationData.Current.TemporaryFolder.CreateFileAsync(
            PasteRules.PastedName(mime), CreationCollisionOption.GenerateUniqueName);
        using (var target = await file.OpenAsync(FileAccessMode.ReadWrite))
        {
            await RandomAccessStream.CopyAndCloseAsync(source.GetInputStreamAt(0), target.GetOutputStreamAt(0));
        }
        return file;
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
        DrawPictureNotice();
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
