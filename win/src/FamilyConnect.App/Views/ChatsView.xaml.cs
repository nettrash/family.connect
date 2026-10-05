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
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Documents;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Animation = Microsoft.UI.Xaml.Media.Animation;
using Ellipse = Microsoft.UI.Xaml.Shapes.Ellipse;
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

    /// <summary>
    /// The family's CHAT stickers (docs/protocol.md, "Sticker pack") — not the board's cards, which this code also calls
    /// stickers: the pack, the one timer that moves the animated ones, and what has been decoded so a redraw does not
    /// decode (and flash) again — by attachment id at the conversation's size, by item id at the panel's, and by staging
    /// handle for one still on its way.
    /// </summary>
    /// <remarks>
    /// The conversation's are the only ones that MOVE, so they are the only ones with frames to account for: they sit
    /// on a <see cref="StickerShelf{TPicture}"/>, which holds them to one budget between them, remembers a sticker
    /// nothing here decodes, and keeps each one's clock across a redraw. The other two are still pictures at a cell's
    /// size; a null in either is the same remembered "nothing here decodes it".
    /// </remarks>
    private readonly PackModel pack;
    private readonly StickerAnimator stickers;
    private readonly StickerShelf<StickerPicture> stickerPictures;
    private readonly Dictionary<long, StickerPicture?> stickerThumbs = [];
    private readonly Dictionary<string, StickerPicture?> stagedStickers = new(StringComparer.Ordinal);

    /// <summary>
    /// One sticker is decoded at a time. Frames are decoded BEFORE the shelf can make room for them, so ten decodes
    /// running side by side would hold ten stickers' frames over the budget at once — and two elements of the same
    /// sticker (a conversation redrawn while the first was still decoding) would each decode it.
    /// </summary>
    private readonly SemaphoreSlim stickerDecoding = new(1, 1);

    /// <summary>The sticker the viewer is showing, which nothing else keeps: given back when the viewer moves on.</summary>
    private StickerPicture? viewerSticker;
    private readonly Action<PackItemDto> onPack;
    private bool sendingSticker;

    /// <summary>What each chat has staged for its next message, kept while the reader looks elsewhere.</summary>
    private readonly Dictionary<long, ComposerStaging> strips = [];

    /// <summary>And what each chat's composer was saying; <see cref="restoredDraft"/> is a draft just put back, which is not typing.</summary>
    private readonly ComposerDrafts drafts = new();
    private string? restoredDraft;

    /// <summary>A voice note being recorded, the chat it belongs to — the one it was started in — and the clock that redraws its row.</summary>
    private VoiceRecorder? recorder;
    private long recordingChat;
    private DispatcherQueueTimer? recordingTimer;

    /// <summary>
    /// Whether the recording began beside words or staged items — from the paperclip or the shortcut — so the slot is Stop and
    /// Stop stages the note beside them (S1.3 row 3); otherwise the composer was empty and the slot is the Send arrow (row 2).
    /// </summary>
    private bool recordingBesideDraft;

    /// <summary>
    /// A start under way — Windows' permission prompt, a screen reader's second — so a second press starts nothing more,
    /// nothing plays meanwhile (S1.7), and an interruption meanwhile lets go of the microphone once it is granted (S4).
    /// </summary>
    private readonly RecordingStart recordingStart = new();

    /// <summary>"30 seconds left" said once a recording, as it is shown (S2.5).</summary>
    private bool warnedThirtySeconds;

    /// <summary>The red dot's pulse while something records (S2.9), and whether the row's buttons are icons for want of room (S2.4).</summary>
    private Animation.Storyboard? recordingPulse;
    private bool? recordingRowNarrow;

    /// <summary>
    /// The trailing slot (S1.3, S8.6): its 600 ms guard and the press reaching it — the latest pointer's and key's, for their
    /// ends — Ctrl+Shift+R's held-down repeats, and how the slot was drawn last, so a redraw on every keystroke changes
    /// nothing that has not changed.
    /// </summary>
    private readonly SlotGuard slotGuard = new();
    private long slotPointerPress;
    private long slotKeyPress;
    private readonly ShortcutPresses recordShortcut = new();
    private int? slotGlyph;
    private string? slotName;
    private string? slotHelp;
    private string? slotTooltip;
    private Animation.Storyboard? slotFade;

    /// <summary>How many voice messages that were not sent the open chat's rows show: the microphone's row 9 (S1.3).</summary>
    private int notSentShown;

    /// <summary>Takes the last thing said to a screen reader off the hidden status line a moment later.</summary>
    private DispatcherQueueTimer? statusClear;

    /// <summary>A recording stopped to ask "Delete this recording?", while the question is up: an interruption keeps it.</summary>
    private AskedRecording? asking;

    /// <summary>
    /// Recordings stopped and not yet put anywhere — the moment between Stop and review, while Windows finishes the file —
    /// which a real close waits for, so that what lands there is kept too.
    /// </summary>
    private readonly List<Task> settling = [];

    /// <summary>The screen, kept on while something records (S1.7).</summary>
    private readonly KeepAwake keepAwake = new();

    /// <summary>
    /// The voice messages that were not sent (docs/audio-video-messages-2026-10-04.md, S2.8) — this account's, on this
    /// server — and whether what is recorded is still kept at all: not once the session has ended (<see cref="Detach"/>).
    /// </summary>
    private readonly ParkedRecordings parked;
    private bool keepsRecordings = true;
    /// <summary>A link clicked a beat ago and waiting to open: a double click on it is the heart, which cancels it.</summary>
    private DispatcherQueueTimer? pendingLink;
    /// <summary>The album the viewer is showing, and a count that makes a load for an earlier page land nowhere.</summary>
    private MediaAlbum? viewing;
    private int viewerShown;
    private long lastHeart;

    /// <summary>The viewer's video, muted while something records and given back as it was (S1.7).</summary>
    private readonly QuietWhileRecording viewerQuiet = new();

    /// <summary>
    /// Video messages (docs/audio-video-messages-2026-10-04.md, S5): which ones this device has played — the dot's own
    /// knowledge, kept in the cache and so this account's; whether the viewer is showing one in its circle, whether it has
    /// played there yet, and whether it failed to load; the clock that keeps the circle's bar up to date and a guard for the
    /// bar moving itself; the backdrop the viewer had before it went solid; and the posters of the reader's own circles
    /// still on their way, by staging handle.
    /// </summary>
    private readonly PlayedRoundStore playedRounds;
    private bool viewerRound;
    private bool viewerRoundPlayed;
    private bool viewerRoundStarted; // this opening has seen it playing: half of being played through (S5.3)
    private bool viewerRoundFailed;
    private bool movingRoundSeek;
    private DispatcherQueueTimer? viewerRoundClock;
    private Brush? viewerBackdrop;
    private readonly Dictionary<string, BitmapImage?> stagedPosters = new(StringComparer.Ordinal);

    /// <summary>The default output device changing — headphones pulled, a headset gone — which pauses what plays (S4).</summary>
    private readonly Windows.Foundation.TypedEventHandler<object, Windows.Media.Devices.DefaultAudioRenderDeviceChangedEventArgs> onOutputChanged;

    /// <summary>
    /// Recording a video message (docs/audio-video-messages-2026-10-04.md, S1.4–S1.6, S3; Windows Phase 3d): where the window
    /// lets the recorder lie and how it covers itself while it does, the recorder while it is open, whether this machine has a
    /// camera (asked only where a video message could be recorded at all), the video button's own 600 ms guard, the field's
    /// trailing room the button takes in it, a chat a notification asked for while the recorder was up, and the session
    /// change that brings the server's limits. Every way in is drawn only where <see cref="RoundVideoRules.Available"/> —
    /// never while <see cref="RoundVideoRules.RecordingEnabled"/> is off.
    /// </summary>
    private Grid? recorderHost;
    private Action<bool>? coverWindow;
    private Action? recorderClosed;
    private RoundRecorderLayer? roundRecorder;
    private bool hasCamera;
    private bool askingForCamera;
    private readonly DoorGuard doorGuard = new();
    private bool fieldPadded;
    private long? chatAfterRecorder;
    private readonly Action<SessionState> onSession;

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

    /// <summary>
    /// A voice note playing from THIS DEVICE — a staged one (S2.7) or one that was not sent (S2.8) — through the same one
    /// player, so it is never beside a bubble's recording; the rows drawn for them, by the note itself and by the not-sent
    /// entry's id; and a count that makes a read for an earlier press land nowhere.
    /// </summary>
    private StagedMedia? playingStaged;
    private string? playingParked;
    private int fetchingLocal;
    private readonly Dictionary<StagedMedia, LocalRow> stagedRows = new(ReferenceEqualityComparer.Instance);
    private readonly Dictionary<string, LocalRow> parkedRows = new(StringComparer.Ordinal);

    /// <summary>
    /// The place under each drawn recording where its text goes (docs/protocol.md, "Transcripts on request"), by
    /// attachment — every one on screen, since the conversation and the thread panel can both draw the same recording —
    /// and what tells this view that one of them changed. What is SHOWN is the model's, so a redraw draws it again.
    /// </summary>
    private readonly Dictionary<long, List<TranscriptHost>> transcriptHosts = [];
    private readonly Action<long> onTranscript;

    /// <summary>A place being shared: the whole flow (one at a time), the part spent looking, and what ends that look.</summary>
    private bool locating;
    private bool finding;
    private CancellationTokenSource? locationHunt;

    /// <summary>The chain open beside the conversation, if any, and what its panel last drew.</summary>
    private ThreadModel? thread;
    private string threadDrawn = string.Empty;

    /// <summary>
    /// The balloon drawn for each message, on each surface: what a click on a quote scrolls to.
    /// Rebuilt with the rows, because that is when the elements are.
    /// </summary>
    private readonly Dictionary<long, (Border Balloon, Brush Flash)> balloons = [];
    private readonly Dictionary<long, (Border Balloon, Brush Flash)> threadBalloons = [];

    /// <summary>Guards the tint's own timer, so a second jump does not clear the first one's.</summary>
    private int tintToken;

    /// <summary>Whether a call is on — the window's to say — and where a call this chat asks for goes.</summary>
    private bool callBusy;

    /// <summary>The call records' "Call back" links drawn in the conversation, switched off with the toolbar's call buttons.</summary>
    private readonly List<HyperlinkButton> callBackLinks = [];

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
        pack = connection.Stickers;
        stickers = new StickerAnimator(DispatcherQueue);
        stickerPictures = new StickerShelf<StickerPicture>(
            StickerAnimation.MaxHeldBytes, StickerAnimation.MaxKept,
            stickers.IsShowing, stickers.MakeStill, stickers.Release);
        var say = services.Say;
        list = new ChatListModel(connection.Chats, () => connection.Session.State.Me?.Id ?? 0, say);
        typing = new TypingRoster(connection.Chats, words: say);
        // The account is known by now: the chats are drawn only for a member, and `GET /me` said who that is.
        parked = ParkedRecordings.For(AppFolders.ParkedPath, connection.Server, connection.Session.State.Me?.Id ?? 0);
        try
        {
            // What a crash left half-written is no recording: it goes, and nothing an entry names does.
            parked.Sweep();
        }
        catch (Exception e)
        {
            Diagnostics.Write($"sweeping voice messages that were not sent: {e.GetType().Name}");
        }
        playedRounds = new PlayedRoundStore(connection.Cache);

        ChatsHeading.Text = say.Get("Chats");
        EmptyListText.Text = say.Get("No chats yet");
        ComposerBox.PlaceholderText = say.Get("Message");
        ToolTipService.SetToolTip(AttachButton, say.Get("Attach"));
        AutomationProperties.SetName(AttachButton, say.Get("Attach"));
        OpenPollsText.Text = say.Get("Open polls");

        ChatList.SelectionChanged += OnChatPicked;
        WireSlot();
        AutomationProperties.SetName(VideoDoorButton, say.Get("Record video message"));
        ToolTipService.SetToolTip(VideoDoorButton, say.Get("Record a video message"));
        VideoDoorButton.Click += (_, _) => OnVideoDoorClick();
        // The server's video-message limits arrive with the family's own document, after the chats are drawn.
        onSession = _ => DispatcherQueue.TryEnqueue(() =>
        {
            if (!gone)
            {
                LearnCamera();
                DrawSlot();
            }
        });
        connection.Session.Changed += onSession;
        ConsentReview.Click += (_, _) => _ = ReviewAssistantConsentAsync(Send);
        AttachButton.Click += (_, _) => ShowAttachMenu();
        ToolTipService.SetToolTip(StickerButton, say.Get("Stickers"));
        AutomationProperties.SetName(StickerButton, say.Get("Stickers"));
        StickerButton.Click += (_, _) => ShowStickerPanel(StickerButton, inThread: null);
        ToolTipService.SetToolTip(ThreadStickerButton, say.Get("Stickers"));
        AutomationProperties.SetName(ThreadStickerButton, say.Get("Stickers"));
        ThreadStickerButton.Click += (_, _) =>
        {
            if (thread is { } chain)
            {
                ShowStickerPanel(ThreadStickerButton, chain);
            }
        };
        ViewerAddSticker.Content = say.Get("Add to family stickers");
        ViewerAddSticker.Click += (_, _) => _ = AddViewedStickerAsync();
        AskAssistantButton.Content = "✨";
        AskPictureButton.Content = "🎨";
        ToolTipService.SetToolTip(AskAssistantButton, say.Get("Ask the assistant"));
        AutomationProperties.SetName(AskAssistantButton, say.Get("Ask the assistant"));
        ToolTipService.SetToolTip(AskPictureButton, say.Get("Ask for a picture"));
        AutomationProperties.SetName(AskPictureButton, say.Get("Ask for a picture"));
        AskAssistantButton.Click += (_, _) => PutInComposer(AssistantText.WithAssistantMention(ComposerBox.Text));
        AskPictureButton.Click += (_, _) => PutInComposer(AssistantText.WithDrawToken(ComposerBox.Text));
        PictureHintText.Text = PictureHint.Sentence(say);

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
        // A video message's own controls under its circle (S5.3, S5.4): play and pause, and scrubbing.
        ViewerRoundPlay.Click += (_, _) => ToggleViewerRound();
        AutomationProperties.SetName(ViewerRoundSeek, say.Get("Position"));
        ViewerRoundSeek.ValueChanged += (_, e) =>
        {
            if (movingRoundSeek || !viewerRound)
            {
                return;
            }
            try
            {
                if (ViewerVideo.MediaPlayer is { } player)
                {
                    player.PlaybackSession.Position = TimeSpan.FromSeconds(e.NewValue);
                }
            }
            catch (Exception failure)
            {
                Diagnostics.Write($"moving a video message: {failure.GetType().Name}");
            }
        };
        ViewerRoundRetryText.Text = say.Get("Couldn't load the video. Tap to try again.");
        AutomationProperties.SetName(ViewerRoundRetry, say.Get("Couldn't load the video. Tap to try again."));
        ViewerRoundRetry.Click += (_, _) => ShowViewerItem();
        ViewerOverlay.SizeChanged += (_, _) =>
        {
            if (viewerRound)
            {
                SizeViewerRound();
            }
        };
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
            // Words the person typed, deleted or pasted lift the slot's guard (S1.1, fc_text::record's OtherAction).
            slotGuard.TextChanged(ComposerBox.Text, recording: recorder is not null);
            // The first character makes the microphone Send, and the last one deleted makes it the microphone again.
            DrawSlot();
        };
        MessageScroller.ViewChanged += OnScrolled;
        JumpButton.Click += (_, _) => JumpToNewest();
        // The recording row's own buttons (S2.4): real buttons, named for what they do to the recording (S6).
        ToolTipService.SetToolTip(RecordingDelete, say.Get("Delete recording"));
        AutomationProperties.SetName(RecordingDelete, say.Get("Delete recording"));
        ToolTipService.SetToolTip(RecordingStop, say.Get("Stop recording"));
        AutomationProperties.SetName(RecordingStop, say.Get("Stop recording"));
        RecordingStop.Click += (_, _) => _ = EndRecordingAsync(RecordingEnd.Stopped);
        RecordingDelete.Click += (_, _) => _ = DeleteRecordingAsync();
        RecordingRow.SizeChanged += (_, _) => FitRecordingRow();
        LocationText.Text = say.Get("Finding your location…");
        PreparingText.Text = say.Get("Preparing…");
        PreparingCancel.Content = say.Get("Cancel");
        PreparingCancel.Click += (_, _) =>
        {
            if (open is { } chat)
            {
                Staging(chat.ChatId).CancelPreparing();
            }
        };
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
                if (recordingStart.Quiet(recorder is not null) && AudioRunning)
                {
                    // No app sound while something records (S1.7) — not even one the system's media keys start: every press
                    // here is refused, and this is what a key outside the app reaches.
                    audio.Pause();
                }
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
        // Headphones pulled or a headset gone: what was playing in the ear must not carry on out of the speakers (S4). Raised on
        // a thread of its own, and only the default role's device is the one this app plays through.
        onOutputChanged = (_, args) =>
        {
            if (args.Role != Windows.Media.Devices.AudioDeviceRole.Default)
            {
                return;
            }
            DispatcherQueue.TryEnqueue(() =>
            {
                if (!gone)
                {
                    PausePlayback(PlaybackEvent.OutputChanged);
                }
            });
        };
        try
        {
            Windows.Media.Devices.MediaDevice.DefaultAudioRenderDeviceChanged += onOutputChanged;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"listening for the output device: {e.GetType().Name}");
        }
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
        // A pack frame changes no conversation — a sent sticker is its own copy — but it can be what makes the button
        // appear, and an item it removed must not be drawn from a stale thumbnail if its id ever came back.
        onPack = item => DispatcherQueue.TryEnqueue(() =>
        {
            if (gone)
            {
                return;
            }
            if (item.Deleted)
            {
                stickerThumbs.Remove(item.Id);
            }
            ShowStickerButton();
        });
        connection.Router.PackChanged += onPack;
        onPreviews = QueueRedraw;
        connection.Previews.Landed += onPreviews;
        // A recording's text changed on its own clock — asked, answered, folded — and only its place under the player redraws.
        onTranscript = attachmentId => DispatcherQueue.TryEnqueue(() =>
        {
            if (!gone)
            {
                DrawTranscripts(attachmentId);
            }
        });
        connection.Transcripts.Changed += onTranscript;
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

    /// <summary>
    /// The window is going away from this connection: stop listening to it. <paramref name="keep"/> is false when the
    /// session itself ended — signed out, expired, the family left — which takes everything recorded and not sent with it
    /// (S4's last row); otherwise, the window closing or another server, what is being recorded waits as "not sent".
    /// </summary>
    internal void Detach(bool keep)
    {
        typingTimer.Stop();
        pictures.Clear();
        stickers.Stop();
        // After the stop, so nothing is "showing" and every frame is given back rather than left to a finalizer.
        ForgetViewerSticker();
        stickerPictures.Clear();
        stickerThumbs.Clear();
        stagedStickers.Clear();
        connection.Session.Changed -= onSession;
        connection.Router.PackChanged -= onPack;
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
        connection.Transcripts.Changed -= onTranscript;
        transcriptHosts.Clear();
        LinkPreviewSetting.Changed -= onPreviews;
        MapPreviewSetting.Changed -= onPreviews;
        // A microphone nothing can reach is a microphone left on — let go of NOW, whatever becomes of what it recorded.
        if (!keep)
        {
            keepsRecordings = false;
        }
        _ = Interrupt(keep ? RecordingEnd.WindowClosed : RecordingEnd.SignedOut);
        if (!keep)
        {
            WipeParked();
        }
        recordingTimer?.Stop();
        keepAwake.Release();
        pendingLink?.Stop();
        pendingLink = null;
        StopAudio();
        try
        {
            Windows.Media.Devices.MediaDevice.DefaultAudioRenderDeviceChanged -= onOutputChanged;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"letting go of the output device: {e.GetType().Name}");
        }
        stagedPosters.Clear();
        locationHunt?.Cancel();
        // A transcode nobody is waiting for any more is minutes of an encoder for nothing: every chat's is called off.
        foreach (var strip in strips.Values)
        {
            strip.CancelPreparing();
        }
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
        if (RecorderOpen)
        {
            // The recorder covers the window: the chat a notification named opens once it closes (S4).
            chatAfterRecorder = chatId;
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
    internal void ReaderReturned()
    {
        _ = ReportReadAsync();
        // A webcam plugged in while the window was behind others.
        LearnCamera(again: true);
    }

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
        ShowStickerButton();
        DrawPictureHint();
    }

    /// <summary>
    /// The line under a picture request that says what the images model refuses (<see cref="PictureHint"/>): drawn with the
    /// draft, and told to a screen reader as it appears — the member is typing, not looking for it — and as the composer's
    /// help text for as long as it stands.
    /// </summary>
    private void DrawPictureHint()
    {
        var kind = open is { } chat ? connection.Chats.Chat(chat.ChatId)?.Chat.Kind : null;
        var shown = PictureHint.ForComposer(kind, ComposerBox.Text, connection.Session.State.Assistant, editing is not null);
        var appearing = shown && PictureHintText.Visibility != Visibility.Visible;
        PictureHintText.Visibility = shown ? Visibility.Visible : Visibility.Collapsed;
        AutomationProperties.SetHelpText(ComposerBox, shown ? PictureHintText.Text : string.Empty);
        if (!appearing)
        {
            return;
        }
        try
        {
            FrameworkElementAutomationPeer.FromElement(PictureHintText)?.RaiseAutomationEvent(AutomationEvents.LiveRegionChanged);
        }
        catch (Exception e)
        {
            // An announcement is a courtesy: the line is on the screen and in the composer's help text either way.
            Diagnostics.Write($"announcing the picture hint: {e.GetType().Name}");
        }
    }

    /// <summary>
    /// The sticker buttons: on a server that has packs (one that predates them names no ceilings, and is offered nothing
    /// rather than a 404), in EVERY chat a message can be sent in — the family chat, a one-to-one chat, the assistant's
    /// chat — and by the thread's composer wherever that composer can send (<see cref="PackSending"/>). Not while a
    /// message is being edited: a sticker is its own message and an edit is somebody else's.
    /// </summary>
    /// <remarks>
    /// IN THE ASSISTANT'S OWN CHAT TOO. To the assistant a sticker is a photo, so the one click that sends it is asked
    /// about exactly as the Send button is there (<see cref="SendStickerAsync"/>) — through the consent question, never
    /// around it.
    /// </remarks>
    private void ShowStickerButton()
    {
        var kind = open is { } chat ? connection.Chats.Chat(chat.ChatId)?.Chat.Kind : null;
        var offered = PackSending.Offered(kind, editing is not null, pack.Offered);
        StickerButton.Visibility = offered ? Visibility.Visible : Visibility.Collapsed;
        // The thread's composer is enabled exactly when there is a root to answer (DrawThread).
        var inThread = PackSending.OfferedInThread(thread is not null && ThreadComposer.IsEnabled, pack.Offered);
        ThreadStickerButton.Visibility = inThread ? Visibility.Visible : Visibility.Collapsed;
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
                Background = Palette.Surface(ActualTheme),
                Child = new TextBlock
                {
                    Text = "@",
                    FontSize = 11,
                    FontWeight = FontWeights.SemiBold,
                    Foreground = Palette.Ink(),
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
        // Leaving: a recording belongs to the chat it was started in, and waits there as "not sent" — and so does a voice
        // note still in review, taking the words in the field as its caption (S2.8, S4). BEFORE the draft is kept, so the
        // words are not kept twice.
        _ = Interrupt(RecordingEnd.LeftChat);
        if (open is { } leaving && editing is null)
        {
            // Half a thought stays with the chat it was written in.
            drafts.Save(leaving.ChatId, ComposerBox.Text);
        }
        // One playing goes quiet with its bubble, and a place being found was asked for there.
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
        DrawNotSent();
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
            callBackLinks.Clear();
            conversationDrawn = string.Empty;
            return;
        }
        var bubbles = chat.Bubbles();
        var pending = chat.Pending();
        var drawn = string.Join(Row, bubbles.Select(bubble => string.Join(Field,
            bubble.Message.Id, bubble.Message.EditSeq, bubble.Message.ReactionSeq, bubble.Message.Poll?.PollSeq, bubble.Reads,
            connection.Chats.IsBlocked(bubble.Message.ReplyTo?.SenderId ?? 0),
            connection.Chats.IsBlocked(bubble.Message.ReplyTo?.Parent?.SenderId ?? 0))));
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
        balloons.Clear();
        callBackLinks.Clear();
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
        // What a failed answer says, remembered with the failure: the provider's refusal, or "ask again".
        var failure = connection.Answers.FailureSentence(message, say);
        var failed = failure is not null;
        var shown = bubble with { Message = message with { Body = body } };
        // A STICKER (docs/protocol.md, "How it is drawn"): one flagged photo and no words. A message from before stickers,
        // or from a server that ignores the flag, is not one, and is drawn as the photo in a balloon it always was.
        var sticker = bubble.Reads ? message.StickerPicture : null;
        if (!bubble.Reads && message.StickerPicture is { } unseen)
        {
            // A HIDDEN ROW STILL FETCHES, AND DRAWS NONE OF IT (docs/protocol.md, "Blocking a member"): the bytes a
            // visible sticker would have asked for are asked for here too, on the same schedule, and kept for the reveal.
            _ = FetchHiddenStickerAsync(unseen);
        }
        // A VIDEO MESSAGE (docs/protocol.md, "Video messages"; S5.1): one flagged video, drawn as a circle with no balloon.
        // Anything else — two attachments, an old server that ignores the flag — is the ordinary message it otherwise is.
        var round = bubble.Reads && sticker is null ? message.RoundVideo : null;
        if (!bubble.Reads && message.RoundVideo is { } unseenRound)
        {
            // The same rule for a circle: its poster — all a visible one fetches — is fetched and drawn nowhere.
            _ = FetchHiddenPosterAsync(unseenRound);
        }
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
        // Which hidden levels of this bubble's quote the reader has asked to see, on THIS surface:
        // the thread panel's reveals are its own, exactly as its hidden bubbles are.
        bool Revealed(QuoteLevel level) => inThread is not null
            ? inThread.QuoteRevealed(message.Id, level)
            : chat.QuoteRevealed(message.Id, level);
        if (bubble.Reads && Quotes.Of(message, connection.Chats, say, Revealed) is { } quote)
        {
            // Over a sticker there is no balloon for the quote to sit on, so it is tinted for the window's own ground —
            // the reader's white-on-accent quote would be white on nothing.
            stack.Children.Add(QuoteElement(quote, mine && sticker is null && round is null, say, () => QuoteClicked(chat, inThread, message.Id, quote)));
        }
        if (sticker is not null)
        {
            stack.Children.Add(StickerElement(sticker));
        }
        else if (round is not null)
        {
            stack.Children.Add(RoundVideoElement(round, mine, () =>
            {
                CancelLinkForHeart();
                _ = ActAsync(() => React(chat, inThread, message.Id, Reactions.DoubleTap));
            }));
            // "Show text" under the circle, outside its gestures (S5.5) — the sticker's branch has no footer, so this one
            // adds its own — in the ink that reads on the chat background, since there is no balloon under it.
            if (TranscriptPanel(round, mine: false, message, kind) is { } text)
            {
                if (mine)
                {
                    text.HorizontalAlignment = HorizontalAlignment.Right;
                }
                stack.Children.Add(text);
            }
        }
        else if (bubble.Reads && message.Media.Count > 0)
        {
            stack.Children.Add(MediaElement(message, mine, kind));
        }
        var words = new TextBlock
        {
            // An answer not written yet is a cursor, not a blank bubble — or says it stopped.
            Text = awaited ? (failure ?? "▍") : BubbleText.Words(shown, list, say),
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
        if (bubble.Reads && !awaited && failure is { } stoppedPartWay)
        {
            // It stopped part-way: what arrived stays, and the row says so.
            stack.Children.Add(new TextBlock
            {
                Text = stoppedPartWay,
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
            && PreviewLinkOf(message, body, emojiOnly: emojiSize is not null) is { } previewLink
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
        // A sticker has NO BUBBLE, a reply or not: the picture alone, its transparency showing the chat behind it. Nor has a
        // video message: the circle alone on the chat background (S5.2).
        var bare = emojiSize is not null || sticker is not null || round is not null || (bubble.Reads
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
        (inThread is not null ? threadBalloons : balloons)[message.Id] = (balloon, FlashFor(mine));
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
            balloon.Background = mine ? Palette.Surface(ActualTheme) : (Brush)resources["CardBackgroundFillColorDefaultBrush"];
            if (mine)
            {
                words.Foreground = Palette.Ink();
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

    /// <summary>
    /// The quote over a reply: the level under it above, the message it answers below, and the
    /// whole thing one control a click acts on (web <c>bubble.rs</c>, ios <c>MessageBubbleView</c>).
    /// </summary>
    /// <remarks>
    /// A BUTTON AND NOT A BORDER WITH A TAP. Narrator has to announce it and the keyboard has to
    /// reach it: a click here reveals a hidden level or goes to the message, and an affordance
    /// only a mouse can use is not one this product ships (the apps paid for that lesson on
    /// their own quote — a bare tap gesture publishes no accessibility action at all).
    /// </remarks>
    private FrameworkElement QuoteElement(Quote quote, bool mine, IStringCatalog say, Action clicked)
    {
        var resources = Application.Current.Resources;
        // TINTED FROM THE BALLOON IT SITS ON, because there are two very different grounds: the
        // reader's own balloon is the accent colour with white words on it, everybody else's is a
        // card with the window's own text. One brush for both is how a quote ends up invisible —
        // the first version used the accent's own secondary shade, which on an accent balloon is
        // the balloon. So: white over the accent, the accent over a card, and an inset panel
        // rather than a bare rule, which is what says "this is the message I am answering".
        var accent = Palette.ThemeAccent();
        var onAccent = Palette.Ink();
        // White over an own balloon, because that balloon is one blue in both themes (OwnBalloon)
        // and its words are white — so the overlay that makes the quote read as INSET is white too.
        Brush Ink(byte alpha) => new SolidColorBrush(Windows.UI.Color.FromArgb(alpha, 0xFF, 0xFF, 0xFF));
        Brush Tinted(byte alpha) => new SolidColorBrush(Windows.UI.Color.FromArgb(alpha, accent.R, accent.G, accent.B));
        var fill = mine ? Ink(0x2A) : Tinted(0x20);
        var lifted = mine ? Ink(0x40) : Tinted(0x38);
        var lines = new StackPanel { Spacing = 0 };
        // The parent goes ABOVE: read downwards it is the older half of the exchange.
        if (quote.Parent is { } parent)
        {
            Draw(parent, 0.8);
        }
        Draw(quote.Reply, 1);
        var frame = new Border
        {
            Child = lines,
            Background = fill,
            // The stripe carries the colour: white on the accent balloon, the accent on a card.
            BorderThickness = new Thickness(3, 0, 0, 0),
            BorderBrush = mine ? Ink(0xD8) : (Brush)resources["AccentFillColorDefaultBrush"],
            CornerRadius = new CornerRadius(4),
            Padding = new Thickness(9, 5, 9, 5),
            Margin = new Thickness(0, 0, 0, 2),
        };
        var button = new Button
        {
            Content = frame,
            Background = null,
            BorderThickness = new Thickness(0),
            Padding = new Thickness(0),
            MinWidth = 0,
            MinHeight = 0,
            HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Left,
        };
        // The panel answers the pointer itself: a Button's own hover lands on the content presenter
        // BEHIND this frame, so without it the one clickable thing in a bubble looks inert.
        button.PointerEntered += (_, _) => frame.Background = lifted;
        button.PointerExited += (_, _) => frame.Background = fill;
        // What it does when clicked, which is not the same thing twice: a hidden level is shown
        // first, and only a quote with nothing left to reveal goes to the message.
        ToolTipService.SetToolTip(button, Quotes.ClickOn(quote) != QuoteClick.GoToMessage
            ? say.Get("Hidden message from a blocked member. Click to show it.")
            : say.Get("Opens the message this answers"));
        AutomationProperties.SetName(button, string.Join(
            ' ',
            new[] { quote.Parent, quote.Reply }
                .Where(line => line is not null)
                .Select(line => $"{line!.Name} {line.Excerpt}".Trim())));
        button.Click += (_, _) => clicked();
        return button;

        void Draw(QuoteLine line, double opacity)
        {
            var row = new StackPanel { Spacing = 0, Opacity = opacity };
            if (!line.Hidden)
            {
                row.Children.Add(new TextBlock
                {
                    Text = line.Name,
                    FontSize = 12,
                    FontWeight = FontWeights.SemiBold,
                    // The name is the coloured half of the quote: the accent where there is a
                    // card behind it, white where the accent itself is.
                    Foreground = mine ? onAccent : (Brush)resources["AccentTextFillColorPrimaryBrush"],
                });
            }
            row.Children.Add(new TextBlock
            {
                Text = line.Excerpt,
                FontSize = 12,
                MaxLines = 2,
                TextWrapping = TextWrapping.Wrap,
                TextTrimming = TextTrimming.CharacterEllipsis,
                FontStyle = line.Hidden ? Windows.UI.Text.FontStyle.Italic : Windows.UI.Text.FontStyle.Normal,
                Foreground = mine ? Ink(0xC8) : (Brush)resources["TextFillColorSecondaryBrush"],
            });
            lines.Children.Add(row);
        }
    }

    /// <summary>
    /// The colour a balloon wears for the moment after a quote's click lands on it — one per
    /// ground, because a pale wash that reads on a card is invisible on the accent balloon. There,
    /// the accent itself is lifted towards white, and kept opaque so the white words stay readable.
    /// </summary>
    private Brush FlashFor(bool mine)
    {
        if (!mine)
        {
            return Application.Current.Resources.TryGetValue("SystemFillColorAttentionBackgroundBrush", out var themed)
                && themed is Brush brush
                ? brush
                : new SolidColorBrush(Windows.UI.Color.FromArgb(0x30, 0x1E, 0x5B, 0xC6));
        }
        // DEEPER, not brighter. Lifting the blue towards white takes white text with it — a 38%
        // lift measures 3.2:1, under the 4.5:1 that ordinary text needs — while deepening it keeps
        // the words past 9:1 and is just as plainly a change of colour. Deepened from whichever
        // ground this theme draws, so it follows the balloon rather than assuming one blue.
        var ground = Palette.SurfaceColour(ActualTheme);
        static byte Deepen(byte channel) => (byte)(channel * 0.72);
        return new SolidColorBrush(
            Windows.UI.Color.FromArgb(0xFF, Deepen(ground.R), Deepen(ground.G), Deepen(ground.B)));
    }

    /// <summary>
    /// A click on a quote. A hidden level is revealed first — outermost, then the one under it,
    /// the same one-tap rule a hidden bubble follows (docs/protocol.md, "Blocking a member") —
    /// and once there is nothing masked left, it goes to the message the reply answers.
    /// </summary>
    private void QuoteClicked(ConversationModel chat, ThreadModel? inThread, long messageId, Quote quote)
    {
        var asked = Quotes.ClickOn(quote);
        if (asked != QuoteClick.GoToMessage)
        {
            var level = asked == QuoteClick.RevealReply ? QuoteLevel.Reply : QuoteLevel.Parent;
            if (inThread is not null)
            {
                inThread.RevealQuote(messageId, level);
                // Nothing the signature watches has changed — the bubble still reads the same —
                // so the redraw has to be asked for outright.
                threadDrawn = string.Empty;
                DrawThread();
            }
            else
            {
                chat.RevealQuote(messageId, level);
                conversationDrawn = string.Empty;
                DrawConversation(keepFromBottom: DistanceFromBottom);
            }
            return;
        }
        GoToMessage(chat, inThread, quote.Reply.MessageId);
    }

    /// <summary>
    /// Show the message a quote names: scrolled to, and tinted for a moment so the eye lands on
    /// it (web <c>conversation.rs</c>, ios <c>jumpToMessage</c>).
    /// </summary>
    /// <remarks>
    /// <para>
    /// FROM WHAT THIS DEVICE HOLDS, WITHOUT A REQUEST. The window widens over rows the cache
    /// already has (<see cref="ConversationModel.DrawTo"/>); a quote can also name a message
    /// retention has swept or one older than anything this install held, and then the reader is
    /// told rather than left clicking a control that does nothing.
    /// </para>
    /// <para>
    /// From the THREAD PANEL, a message that is not in the chain is shown in the conversation
    /// behind it: the panel closes rather than the click being refused, because the message is
    /// there and going to it is what was asked for.
    /// </para>
    /// </remarks>
    private void GoToMessage(ConversationModel chat, ThreadModel? inThread, long messageId)
    {
        var say = services.Say;
        if (inThread is not null)
        {
            if (inThread.Draws(messageId) && threadBalloons.TryGetValue(messageId, out var inChain))
            {
                ScrollTo(ThreadScroller, ThreadStack, inChain);
                return;
            }
            CloseThread();
        }
        if (!chat.DrawTo(messageId))
        {
            ShowProblem(chat.MayHaveOlder
                ? say.Get("That message is not loaded yet. Scroll up to read further back.")
                : say.Get("That message is not here any more."));
            return;
        }
        // Widening the window draws rows above the reader: this redraw is what puts the element
        // being scrolled to in the tree at all.
        DrawConversation(keepFromBottom: DistanceFromBottom);
        if (!balloons.TryGetValue(messageId, out var drawn))
        {
            ShowProblem(say.Get("That message is not here any more."));
            return;
        }
        ComposerError.Visibility = Visibility.Collapsed;
        // From the offset asked for, not from the scroller: ChangeView has not happened yet when
        // it returns, and the divider's own scroll reads it the same way for the same reason.
        var top = ScrollTo(MessageScroller, MessageStack, drawn);
        atNewest = MessageScroller.ScrollableHeight - top <= 24;
        ShowJump();
    }

    /// <summary>
    /// Scroll one surface to a balloon it drew, tint it briefly, and answer the offset asked for.
    /// </summary>
    private double ScrollTo(ScrollViewer scroller, FrameworkElement stack, (Border Balloon, Brush Flash) drawn)
    {
        scroller.UpdateLayout();
        var top = Math.Max(
            0,
            drawn.Balloon.TransformToVisual(stack).TransformPoint(new Windows.Foundation.Point(0, 0)).Y - 12);
        scroller.ChangeView(null, top, null, disableAnimation: false);
        Tint(drawn);
        return top;
    }

    /// <summary>
    /// The moment of colour that says the view moved. Restored by its own timer, and the token
    /// means a second jump before the first fades does not put back a background that has since
    /// been replaced.
    /// </summary>
    private void Tint((Border Balloon, Brush Flash) drawn)
    {
        var balloon = drawn.Balloon;
        var was = balloon.Background;
        balloon.Background = drawn.Flash;
        var token = ++tintToken;
        var timer = DispatcherQueue.CreateTimer();
        timer.Interval = TimeSpan.FromMilliseconds(1200);
        timer.IsRepeating = false;
        timer.Tick += (_, _) =>
        {
            if (token == tintToken)
            {
                balloon.Background = was;
            }
        };
        timer.Start();
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

        // A circle opens full screen from its menu too (S5.4) — the viewer, with scrubbing; and it has no Edit, which
        // MayEdit already says.
        if (message.RoundVideo is { } circle)
        {
            var full = new MenuFlyoutItem { Text = say.Get("Open Full Screen") };
            full.Click += (_, _) => OpenRound(circle);
            menu.Items.Add(full);
        }

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
        // THE ASSISTANT'S OWN SAFETY ITEM, and only the one: a reply can be
        // reported, and there is nothing to block — the assistant is not a
        // member, and whether it speaks at all is the owner's `ai_greeting`
        // and the operator's `[ai]` switch. The report goes to the people who
        // run the server rather than to the family owner, for the reason
        // docs/protocol.md gives under "Reporting the assistant".
        if (BubbleRules.MayReportAssistant(message, Reader, assistantChat, assistantId))
        {
            var assistantSafety = new MenuFlyoutSubItem { Text = services.Say.Get("Safety") };
            var reportReply = new MenuFlyoutItem { Text = services.Say.Get("Report this reply…") };
            reportReply.Click += (_, _) => _ = ReportAssistantReplyAsync(message);
            assistantSafety.Items.Add(reportReply);
            if (menu.Items.Count > 0)
            {
                menu.Items.Add(new MenuFlyoutSeparator());
            }
            menu.Items.Add(assistantSafety);
            return;
        }
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

    /// <summary>
    /// One assistant reply reported. The operator reads it, not the owner, and the sheet says so
    /// before it is sent (docs/protocol.md, "Reporting the assistant").
    /// </summary>
    private async Task ReportAssistantReplyAsync(MessageDto message)
    {
        var say = services.Say;
        try
        {
            var chosen = await Dialogs.ReportAssistantAsync(
                XamlRoot, say, connection.Session.State.SupportContact);
            if (chosen is not { } report)
            {
                return;
            }
            var answer = await connection.Api.ReportAssistant(message.Id, report.Reason, report.Note);
            if (answer.Ok)
            {
                ShowProblem(say.Get("Report sent."));
                return;
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"reporting an assistant reply: {e.GetType().Name}");
        }
        ShowProblem(say.Get("Couldn't send the report. Try again."));
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
        if (row.Sticker)
        {
            return PendingStickerElement(chat, row);
        }
        if (row.Round)
        {
            return PendingRoundElement(chat, row);
        }
        var say = services.Say;
        var resources = Application.Current.Resources;
        var stack = new StackPanel { Spacing = 4 };
        if (row.Body.Length > 0)
        {
            stack.Children.Add(new TextBlock
            {
                Text = row.Body,
                TextWrapping = TextWrapping.Wrap,
                Foreground = Palette.Ink(),
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
                Foreground = Palette.Ink(),
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
                Foreground = Palette.Ink(),
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
            Background = Palette.Surface(ActualTheme),
            Opacity = row.Failed ? 0.9 : 0.6,
        };
    }

    // ---- attachments ---------------------------------------------------------------------------

    /// <summary>
    /// What a message carries: the pictures first — one at its own shape, several as a grid of four with
    /// the rest counted — and then the rows that are read rather than looked at.
    /// </summary>
    private FrameworkElement MediaElement(MessageDto message, bool mine, string? chatKind)
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
        // A video's text goes under the pictures, one place per video; where there are several, each says which it is.
        var videos = looked.Where(attachment => attachment.Kind == "video").ToList();
        for (var at = 0; at < videos.Count; at++)
        {
            var video = videos[at];
            if (TranscriptPanel(video, mine, message, chatKind) is not { } text)
            {
                continue;
            }
            if (videos.Count > 1)
            {
                var label = $"{services.Say.Get("Video")} {(at + 1).ToString(services.Culture)}";
                var caption = new TextBlock { Text = label, FontSize = 11, Opacity = 0.75, Margin = new Thickness(4, 2, 4, 0) };
                if (mine)
                {
                    caption.Foreground = Palette.Ink();
                }
                AutomationProperties.SetName(text, label);
                panel.Children.Add(caption);
            }
            panel.Children.Add(text);
        }
        foreach (var attachment in message.Media.Where(attachment => !MediaText.IsMedia(attachment.Kind)))
        {
            panel.Children.Add(attachment.Kind switch
            {
                "location" => LocationElement(attachment, mine),
                "audio" => AudioElement(attachment, mine, message, chatKind),
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
    private void OpenViewer(IReadOnlyList<AttachmentDto> items, int index, bool round = false)
    {
        viewing = new MediaAlbum(items, index, round);
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
        var played = viewerRoundPlayed;
        SetViewerRound(false);
        if (played)
        {
            // Played here: its dot goes (S5.2) — on the next turn, and never into a view the window has already let go of
            // (Detach closes the viewer too).
            DispatcherQueue.TryEnqueue(() =>
            {
                if (gone)
                {
                    return;
                }
                conversationDrawn = string.Empty;
                threadDrawn = string.Empty;
                DrawConversation(keepFromBottom: atNewest ? null : DistanceFromBottom);
                DrawThread();
            });
        }
        ForgetViewerSticker();
        ViewerAddSticker.Visibility = Visibility.Collapsed;
        ViewerNotice.Visibility = Visibility.Collapsed;
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
        ViewerNotice.Visibility = Visibility.Collapsed;
        // Offered once the page has loaded and the pack has been asked — never on a guess.
        ViewerAddSticker.Visibility = Visibility.Collapsed;
        StopViewerVideo();
        ForgetViewerSticker();
        ViewerScroller.ChangeView(0, 0, 1, disableAnimation: true);
        ViewerScroller.Visibility = album.IsVideo ? Visibility.Collapsed : Visibility.Visible;
        ViewerVideo.Visibility = album.IsVideo ? Visibility.Visible : Visibility.Collapsed;
        ViewerZoomBar.Visibility = album.IsVideo ? Visibility.Collapsed : Visibility.Visible;
        // Only a video message — opened from its circle or its menu, S5.1's test of the whole message — is shown in its circle.
        SetViewerRound(album.IsRound);
        ShowZoom();
        ViewerLoading.IsActive = true;
        _ = LoadViewerItemAsync(album.Current, album.IsVideo, token);
    }

    private async Task LoadViewerItemAsync(AttachmentDto item, bool video, int token)
    {
        try
        {
            if (item.Sticker)
            {
                await LoadViewerStickerAsync(item, token);
                return;
            }
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
                // Not by itself under a running microphone, and not at all until it stops (S1.7).
                ViewerVideo.AutoPlay = recorder is null;
                ViewerVideo.Source = Windows.Media.Core.MediaSource.CreateFromStream(stream, item.Mime ?? "video/mp4");
                QuietViewerVideo();
                ViewerLoading.IsActive = false;
                if (viewerRound)
                {
                    // One thing plays at a time (S5.3): a voice note stops for the circle.
                    if (AudioRunning)
                    {
                        audio.Pause();
                        ShowPlayback();
                    }
                    ShowViewerRound();
                }
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

    /// <summary>
    /// The viewer is about to show something else, or nothing: its image is emptied FIRST, and only then are the frames
    /// of the sticker it was showing given back — the viewer's picture is kept nowhere else, and a moving one is the
    /// largest thing this view decodes.
    /// </summary>
    private void ForgetViewerSticker()
    {
        stickers.Forget(ViewerImage);
        try
        {
            ViewerImage.Source = null;
        }
        catch (Exception e)
        {
            // Still showing a frame, then: the frames are the collector's.
            Diagnostics.Write($"emptying the viewer: {e.GetType().Name}");
            viewerSticker = null;
            return;
        }
        if (viewerSticker is { } shown)
        {
            viewerSticker = null;
            stickers.Release(shown);
        }
    }

    /// <summary>The size a STILL sticker is decoded for when it is looked at on its own: the pack's own 512.</summary>
    private const double ViewerStickerBox = StickerFile.Edge;

    /// <summary>
    /// And a MOVING one: twice the conversation's box, in plain pixels. Every frame is held decoded, and at 512 a
    /// two-second sticker would be past the budget — drawn still in the very place somebody opened it to watch.
    /// </summary>
    private const double ViewerMovingBox = StickerLook.Box * 2;

    /// <summary>
    /// A sticker shown larger (docs/protocol.md, "Tapping one shows it larger"): its ORIGINAL bytes, moving where this
    /// machine can — and "Add to family stickers" when the family's pack does not hold it.
    /// </summary>
    private async Task LoadViewerStickerAsync(AttachmentDto item, int token)
    {
        var (bytes, _) = await connection.Attachments.BytesAsync(item, preview: false);
        if (token != viewerShown)
        {
            return;
        }
        if (bytes is null)
        {
            ViewerFailed();
            return;
        }
        var moves = StickerFile.IsAnimated(bytes) && StickerImaging.AnimationsWanted();
        var picture = moves
            ? await StickerImaging.DecodeAsync(bytes, ViewerMovingBox, 1, animate: true)
            : await StickerImaging.DecodeAsync(bytes, ViewerStickerBox, RasterScale(), animate: false);
        if (token != viewerShown)
        {
            // The viewer moved on while this was decoding: nothing will ever draw it.
            picture?.Release();
            return;
        }
        if (picture is null)
        {
            ViewerFailed();
            return;
        }
        ForgetViewerSticker();
        viewerSticker = picture;
        stickers.Show(ViewerImage, picture);
        ViewerLoading.IsActive = false;
        FitViewerImage();
        // Decided HERE, from bytes this device already has: nothing on the wire names the item a message was sent from.
        // A wrong "no" only offers what the pack already holds, and the server answers that with the item that was there.
        if (pack.Offered && !await pack.HoldsAsync(item) && token == viewerShown)
        {
            ViewerAddSticker.IsEnabled = true;
            ViewerAddSticker.Visibility = Visibility.Visible;
        }
    }

    /// <summary>"Add to family stickers": the pack's own flow, with the message's bytes — uploaded again, unprepared, and claimed.</summary>
    private async Task AddViewedStickerAsync()
    {
        if (viewing is not { Current: { Sticker: true } item })
        {
            return;
        }
        var say = services.Say;
        var token = viewerShown;
        ViewerAddSticker.IsEnabled = false;
        ViewerProblem.Visibility = Visibility.Collapsed;
        ViewerNotice.Visibility = Visibility.Collapsed;
        (PackAdded? Added, ApiError? Error) answer;
        try
        {
            // The few words it may be given, as for a sticker added from disk: optional, and fixed once it is added.
            var (add, label) = await Dialogs.StickerLabelAsync(XamlRoot, say);
            if (!add || token != viewerShown)
            {
                if (token == viewerShown)
                {
                    ViewerAddSticker.IsEnabled = true;
                }
                return;
            }
            answer = await pack.AddFromMessageAsync(item, label);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"adding a sent sticker to the pack: {e.GetType().Name}");
            answer = (null, ApiError.Transport(e.GetType().Name));
        }
        if (token != viewerShown)
        {
            return;
        }
        if (answer.Added is { } added)
        {
            // Either way the pack holds it now, so there is nothing left to offer.
            ViewerAddSticker.Visibility = Visibility.Collapsed;
            ViewerNotice.Text = PackText.Sentence(added, say);
            ViewerNotice.Visibility = Visibility.Visible;
            ShowStickerButton();
            return;
        }
        ViewerAddSticker.IsEnabled = true;
        ViewerProblem.Text = PackText.Sentence(answer.Error ?? ApiError.Transport("no answer"), say);
        ViewerProblem.Visibility = Visibility.Visible;
    }

    private void ViewerFailed()
    {
        ViewerLoading.IsActive = false;
        if (viewerRound)
        {
            // Over its poster, with the way back (S5.3).
            viewerRoundFailed = true;
            ViewerRoundRetry.Visibility = Visibility.Visible;
            ViewerRoundBar.Visibility = Visibility.Collapsed;
            return;
        }
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

    /// <summary>
    /// No app sound while something records (S1.7) — the viewer's video included. While a recording runs its controls go and
    /// it says why, as every dimmed play button does, and its player is paused and muted, so nothing a key or the system's
    /// media buttons start is heard in the note; once the recording ends the controls come back and the player is left muted,
    /// or not, as it was before (<see cref="QuietWhileRecording"/>).
    /// </summary>
    private void QuietViewerVideo()
    {
        if (gone)
        {
            return;
        }
        var recording = recorder is not null;
        // A circle has its own bar under it, which the same rule dims.
        ViewerVideo.AreTransportControlsEnabled = !recording && !viewerRound;
        ViewerRoundPlay.IsEnabled = !recording;
        ViewerRoundSeek.IsEnabled = !recording;
        try
        {
            if (ViewerVideo.MediaPlayer is { } player)
            {
                if (recording)
                {
                    player.Pause();
                }
                player.IsMuted = viewerQuiet.Muted(recording, player.IsMuted);
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"quieting the viewer's video: {e.GetType().Name}");
        }
        var sentence = services.Say.Get("You can play this after recording.");
        if (recording && viewing is { IsVideo: true })
        {
            ViewerNotice.Text = sentence;
            ViewerNotice.Visibility = Visibility.Visible;
        }
        else if (!recording && ViewerNotice.Visibility == Visibility.Visible && ViewerNotice.Text == sentence)
        {
            ViewerNotice.Visibility = Visibility.Collapsed;
        }
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

    /// <summary>
    /// A call from this chat, to its other member: the window places it and shows it (docs/protocol.md, "Voice calls") — never
    /// over a recording, whatever button asked (S1.7: the call buttons are off while something records).
    /// </summary>
    private void RequestCall(bool video)
    {
        if (!CallRecords.CanPlaceCall(callBusy, recorder is not null))
        {
            return;
        }
        if (open is { } chat && connection.Chats.Chat(chat.ChatId)?.Chat is { Kind: "direct", PeerUserId: { } peer })
        {
            CallRequested?.Invoke(chat.ChatId, peer, video);
        }
    }

    /// <summary>
    /// Whether a call is on: while one is, a second is not placed — and nothing is recorded. A call in any phase stops what
    /// is being recorded, which waits as "not sent", never sent (S4).
    /// </summary>
    internal void ShowCallBusy(bool busy)
    {
        callBusy = busy;
        if (busy)
        {
            _ = Interrupt(RecordingEnd.Call);
        }
        ShowCallButtons();
        // A call dims the microphone, and says so when it is pressed (S1.3 row 7).
        DrawSlot();
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
        var ink = mine ? Palette.Ink() : (Brush)resources["TextFillColorPrimaryBrush"];
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
            // Off with the toolbar's call buttons: while a call is on, and while something records (S1.7).
            var back = new HyperlinkButton
            {
                Content = say.Get("Call back"),
                Padding = new Thickness(0, 2, 0, 0),
                IsEnabled = CallRecords.CanPlaceCall(callBusy, recorder is not null),
            };
            if (mine)
            {
                back.Foreground = ink;
            }
            AutomationProperties.SetHelpText(back, say.Get("Calls back"));
            back.Click += (_, _) => RequestCall(video);
            lines.Children.Add(back);
            callBackLinks.Add(back);
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
        // Off while something records too (S1.7): a call over a recording has no right answer — and so is every record's
        // "Call back" already drawn, which a recording starting or ending does not redraw.
        var live = CallRecords.CanPlaceCall(callBusy, recorder is not null);
        CallButton.IsEnabled = live;
        VideoCallButton.IsEnabled = live;
        foreach (var link in callBackLinks)
        {
            link.IsEnabled = live;
        }
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
        threadBalloons.Clear();
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
            bubble.Message.ReplyCount, bubble.Reads, connection.Chats.IsBlocked(bubble.Message.ReplyTo?.SenderId ?? 0),
            connection.Chats.IsBlocked(bubble.Message.ReplyTo?.Parent?.SenderId ?? 0))));
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
        threadBalloons.Clear();
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
        ShowStickerButton();
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
        DrawSlot();
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
            // Send waits for the place: the words typed meanwhile are its caption.
            DrawSlot();
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
                DrawSlot();
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
            DrawSlot();
        }
    }

    // ---- the Send slot ---------------------------------------------------------------------------
    //
    // Voice and video messages from the Send button (docs/audio-video-messages-2026-10-04.md, Phase 1; issue #79): the
    // composer's trailing control is Send when there is something to send and a microphone when there is not, in one fixed
    // place. Which it is, how it is drawn and what a press on it does are ComposerButton's; this only wires and draws.

    /// <summary>
    /// The slot's presses (S8.6): a click of any length with any input; a mouse right-click, a pen tap with the barrel button
    /// down, Shift+F10 or the Menu key for the microphone's menu — never a hold; the press that is reaching it, from its going
    /// down to its end, for the 600 ms guard (<see cref="SlotGuard"/>); and Ctrl+Shift+R, the app's first keyboard accelerator,
    /// once per press however long it is held, and Esc while something records.
    /// </summary>
    private void WireSlot()
    {
        SendButton.Click += (_, _) => OnSlotClick();
        SendButton.RightTapped += (_, e) =>
        {
            e.Handled = true;
            OnSlotMenuPress(slotGuard.RightTapped(DeviceOf(e.PointerDeviceType), CurrentSlot().IsMicrophone));
        };
        SendButton.ContextRequested += (sender, e) =>
        {
            e.Handled = true;
            // No position: the keyboard's Shift+F10 or Menu key. At a position: a pointer's, which RightTapped answers.
            var press = e.TryGetPosition(SendButton, out _) ? SlotPress.ContextAtPointer : SlotPress.ContextKey;
            OnSlotMenuPress(ComposerButton.Respond(press, SlotDevice.None, barrelAtPress: false, CurrentSlot().IsMicrophone));
        };
        // The button takes the pointer's press for itself; what it was — and when, and when it ended — is still read, after it.
        SendButton.AddHandler(UIElement.PointerPressedEvent, new PointerEventHandler(OnSlotPointerPressed), true);
        SendButton.AddHandler(UIElement.PointerReleasedEvent, new PointerEventHandler(OnSlotPointerEnded), true);
        SendButton.AddHandler(UIElement.PointerCanceledEvent, new PointerEventHandler(OnSlotPointerEnded), true);
        SendButton.AddHandler(UIElement.PointerCaptureLostEvent, new PointerEventHandler(OnSlotPointerEnded), true);
        SendButton.PreviewKeyDown += OnSlotKey;
        SendButton.AddHandler(UIElement.KeyUpEvent, new KeyEventHandler(OnSlotKeyUp), true);
        SendButton.LostFocus += (_, _) => EndSlotPress(slotKeyPress);
        var record = new KeyboardAccelerator
        {
            Key = Windows.System.VirtualKey.R,
            Modifiers = Windows.System.VirtualKeyModifiers.Control | Windows.System.VirtualKeyModifiers.Shift,
        };
        record.Invoked += (_, args) =>
        {
            args.Handled = true;
            if (recordShortcut.Fresh(Environment.TickCount64))
            {
                OnRecordShortcut();
            }
        };
        ComposerPanel.KeyboardAccelerators.Add(record);
        // An accelerator repeats while its key is held, and that cannot be changed: the chord's repeats are swallowed as they
        // come in — PreviewKeyDown comes before any accelerator — so a held Ctrl+Shift+R starts a recording, or stops one, once.
        AddHandler(UIElement.PreviewKeyDownEvent, new KeyEventHandler(OnShortcutKey), true);
        // Inside the recording row Esc is Stop — never Delete (S2.4, decision 13).
        ComposerPanel.KeyDown += (_, e) =>
        {
            if (e.Key == Windows.System.VirtualKey.Escape && recorder is not null)
            {
                e.Handled = true;
                _ = EndRecordingAsync(RecordingEnd.Stopped);
            }
        };
        DrawSlot();
    }

    private static SlotDevice DeviceOf(Microsoft.UI.Input.PointerDeviceType type) => type switch
    {
        Microsoft.UI.Input.PointerDeviceType.Touch => SlotDevice.Touch,
        Microsoft.UI.Input.PointerDeviceType.Pen => SlotDevice.Pen,
        // A mouse — and a touchpad, whose two-finger click is the mouse's right one.
        _ => SlotDevice.Mouse,
    };

    private static bool KeyHeld(Windows.System.VirtualKey key) =>
        InputKeyboardSource.GetKeyStateForCurrentThread(key).HasFlag(Windows.UI.Core.CoreVirtualKeyStates.Down);

    /// <summary>A press goes down on the slot: with what — and whether a pen's barrel button was held as it did (S8.6).</summary>
    private void OnSlotPointerPressed(object sender, PointerRoutedEventArgs e)
    {
        var device = DeviceOf(e.Pointer.PointerDeviceType);
        var barrel = device == SlotDevice.Pen && e.GetCurrentPoint(SendButton).Properties.IsBarrelButtonPressed;
        slotPointerPress = slotGuard.PointerDown(Environment.TickCount64, device, barrel);
    }

    /// <summary>The pointer's press is over — lifted, cancelled or its capture lost — with or without a click.</summary>
    private void OnSlotPointerEnded(object sender, PointerRoutedEventArgs e) => EndSlotPress(slotPointerPress);

    /// <summary>
    /// A press ends — after the click it made, if it made one: the button's own handling comes first, and this goes to the
    /// back of the queue besides, so whatever that press raises in the same breath still reads it. A later press is not
    /// touched (<see cref="SlotGuard.PressEnded"/>).
    /// </summary>
    private void EndSlotPress(long press) =>
        DispatcherQueue.TryEnqueue(Microsoft.UI.Dispatching.DispatcherQueuePriority.Low, () => slotGuard.PressEnded(press));

    /// <summary>
    /// Enter or Space on the focused slot: the press begins at the key's going down — and a key held down is ONE press, so its
    /// repeats are swallowed rather than let through once the guard has run out.
    /// </summary>
    private void OnSlotKey(object sender, KeyRoutedEventArgs e)
    {
        if (e.Key is not (Windows.System.VirtualKey.Enter or Windows.System.VirtualKey.Space))
        {
            return;
        }
        if (e.KeyStatus.WasKeyDown)
        {
            e.Handled = true;
            return;
        }
        slotKeyPress = slotGuard.KeyDown(Environment.TickCount64);
    }

    private void OnSlotKeyUp(object sender, KeyRoutedEventArgs e)
    {
        if (e.Key is Windows.System.VirtualKey.Enter or Windows.System.VirtualKey.Space)
        {
            EndSlotPress(slotKeyPress);
        }
    }

    /// <summary>Ctrl+Shift+R going down anywhere in the view: a repeat is swallowed before the accelerator sees it (S1.6).</summary>
    private void OnShortcutKey(object sender, KeyRoutedEventArgs e)
    {
        if (e.Key == Windows.System.VirtualKey.R && KeyHeld(Windows.System.VirtualKey.Control) && KeyHeld(Windows.System.VirtualKey.Shift)
            && recordShortcut.KeyDown(Environment.TickCount64, e.KeyStatus.WasKeyDown))
        {
            e.Handled = true;
        }
    }

    /// <summary>A right-click, a barrel tap or the menu key reached the slot: the microphone's menu, or nothing (S1.6, S8.6).</summary>
    private void OnSlotMenuPress(SlotResponse response)
    {
        if (response == SlotResponse.Menu && !gone && open is not null)
        {
            ShowSlotMenu();
        }
    }

    /// <summary>
    /// A click reached the slot. What it does is <see cref="SlotGuard.Click"/>'s (S8.6) and the row's (S1.3): the recording's
    /// Send or Stop, today's Send or Save, a dimmed microphone's reason, or a hands-free recording — none of them while the
    /// guard runs, or for a press that went down while it ran (S1.1).
    /// </summary>
    private void OnSlotClick()
    {
        var now = Environment.TickCount64;
        // Asked even with nothing open: the press is used up either way, and the next click brings its own or none at all.
        var slot = CurrentSlot();
        var response = slotGuard.Click(now, slot, ComposerBox.Text);
        if (gone || open is null)
        {
            return;
        }
        if (response == SlotResponse.Menu)
        {
            ShowSlotMenu();
            return;
        }
        if (response == SlotResponse.Ignore)
        {
            return;
        }
        switch (slot.Kind)
        {
            case SlotKind.SendVoice:
                slotGuard.Arm(now, ComposerBox.Text);
                _ = EndRecordingAsync(RecordingEnd.Sent);
                break;
            case SlotKind.StopRecording:
                slotGuard.Arm(now, ComposerBox.Text);
                _ = EndRecordingAsync(RecordingEnd.Stopped);
                break;
            case SlotKind.Save or SlotKind.Send:
                Send();
                break;
            case SlotKind.Dimmed:
                Explain(ComposerButton.Notice(slot.Reason, services.Say)!);
                break;
            case SlotKind.Microphone:
                _ = StartRecordingAsync(fromSlot: true);
                break;
        }
    }

    /// <summary>
    /// The microphone's secondary menu (S1.6): "Record Voice Message" — which, in rows 7 to 9, says why it cannot instead. Its
    /// "Record Video Message" arrives with round video on Windows (Phase 3d).
    /// </summary>
    private void ShowSlotMenu()
    {
        var menu = new MenuFlyout { Placement = Microsoft.UI.Xaml.Controls.Primitives.FlyoutPlacementMode.TopEdgeAlignedRight };
        var record = new MenuFlyoutItem
        {
            Text = services.Say.Get("Record Voice Message"),
            Icon = new FontIcon { Glyph = ((char)ComposerButton.MicrophoneGlyph).ToString() },
            KeyboardAcceleratorTextOverride = ComposerButton.RecordShortcut,
        };
        record.Click += (_, _) =>
        {
            if (CurrentSlot() is { Kind: SlotKind.Dimmed } dimmed)
            {
                Explain(ComposerButton.Notice(dimmed.Reason, services.Say)!);
                return;
            }
            _ = StartRecordingAsync(fromSlot: false);
        };
        menu.Items.Add(record);
        // "Record Video Message" (S1.6), where one can be recorded: in rows 7 and 8 it says why not; in row 9 it opens the
        // recorder, as the video button does — the not-sent rule is about voice.
        if (RoundAvailable() && open is { } chat && Kind(chat) != "ai")
        {
            var video = new MenuFlyoutItem
            {
                Text = services.Say.Get("Record Video Message"),
                Icon = new FontIcon { Glyph = ((char)RoundVideoGlyph).ToString() },
            };
            video.Click += (_, _) =>
            {
                if (CurrentSlot() is { Kind: SlotKind.Dimmed, Reason: Dimmed.Call or Dimmed.Busy } dimmed)
                {
                    Explain(ComposerButton.Notice(dimmed.Reason, services.Say)!);
                    return;
                }
                _ = OpenRoundRecorderAsync(SendButton);
            };
            menu.Items.Add(video);
        }
        menu.ShowAt(SendButton);
    }

    /// <summary>
    /// Ctrl+Shift+R (S1.6): records — beside the draft when there is one — and, pressed while a recording runs, stops it into
    /// review. A shortcut never sends, and never opens a camera. Nothing in the assistant's chat, and nothing new under the
    /// viewer, where the row could not be seen.
    /// </summary>
    private void OnRecordShortcut()
    {
        if (gone || open is not { } chat)
        {
            return;
        }
        if (recorder is not null)
        {
            _ = EndRecordingAsync(RecordingEnd.Stopped);
            return;
        }
        if (recordingStart.Starting || asking is not null || ViewerOverlay.Visibility == Visibility.Visible || Kind(chat) == "ai")
        {
            return;
        }
        _ = StartRecordingAsync(fromSlot: false);
    }

    /// <summary>The composer's attachment guard (S1.2's <b>busy</b>): files being prepared or written down, or a place being found.</summary>
    private bool Busy(ComposerStaging strip) => strip.Preparing || sendingMedia || locating;

    /// <summary>Which row of S1.3 the open chat's slot is in, from what the composer is now.</summary>
    private Slot CurrentSlot() =>
        SlotInputsNow() is { } inputs ? ComposerButton.ComposerSlot(inputs) : new(SlotKind.SendDisabled);

    /// <summary>What the open chat's composer is now, as S1.2 names it — or null with no chat open.</summary>
    private SlotInputs? SlotInputsNow()
    {
        if (open is not { } chat)
        {
            return null;
        }
        var strip = Staging(chat.ChatId);
        return new SlotInputs(
            RecorderOpen: RecorderOpen,
            Recording: recorder is null ? Recording.None : recordingBesideDraft ? Recording.HandsFreeBesideDraft : Recording.HandsFree,
            Editing: editing is not null,
            DraftBlank: string.IsNullOrWhiteSpace(ComposerBox.Text),
            Staged: strip.Items.Count > 0,
            AssistantChat: Kind(chat) == "ai",
            // Whether this machine has a microphone at all is only learnt by trying: a press then says it could not start.
            CanRecord: true,
            Call: callBusy,
            Busy: Busy(strip),
            NotSent: notSentShown > 0);
    }

    /// <summary>
    /// The slot as its row draws it (S8.6): the glyph, cross-faded over 150 ms when it changes (at once with Windows'
    /// animations off), the name, the tooltip and Narrator's HelpText; dimmed is drawn faded and left enabled.
    /// </summary>
    private void DrawSlot()
    {
        if (gone)
        {
            return;
        }
        var slot = CurrentSlot();
        var face = ComposerButton.Face(slot, sendHeld: sendingMedia || finding, services.Say);
        // The microphone's video message is behind its menu (S6): Narrator says how to reach it, where there is one.
        if (face.HelpText.Length == 0 && slot.Kind == SlotKind.Microphone && RoundAvailable())
        {
            face = face with { HelpText = services.Say.Get("Press Shift+F10 for a video message.") };
        }
        if (SendButton.IsEnabled != face.Enabled)
        {
            SendButton.IsEnabled = face.Enabled;
        }
        // Set only when they change: the slot is redrawn on every keystroke, and a screen reader is told of every setting.
        if (slotName != face.Name)
        {
            slotName = face.Name;
            AutomationProperties.SetName(SendButton, face.Name);
        }
        if (slotHelp != face.HelpText)
        {
            slotHelp = face.HelpText;
            AutomationProperties.SetHelpText(SendButton, face.HelpText);
        }
        if (slotTooltip != face.Tooltip)
        {
            slotTooltip = face.Tooltip;
            ToolTipService.SetToolTip(SendButton, face.Tooltip);
        }
        SlotLook.Opacity = face.Enabled && !face.LooksDimmed ? 1 : 0.4;
        ShowSlotGlyph(face.Glyph);
        DrawVideoDoor();
    }

    private void ShowSlotGlyph(int glyph)
    {
        if (slotGlyph == glyph)
        {
            return;
        }
        var before = slotGlyph;
        slotGlyph = glyph;
        slotFade?.Stop();
        slotFade = null;
        SlotGlyph.Glyph = ((char)glyph).ToString();
        // At once the first time, before the view is on screen, and when Windows' animations are off (S1.3, S6).
        if (before is not { } leaving || !SendButton.IsLoaded || !StickerImaging.AnimationsWanted())
        {
            SlotGlyph.Opacity = 1;
            SlotGlyphLeaving.Opacity = 0;
            return;
        }
        SlotGlyphLeaving.Glyph = ((char)leaving).ToString();
        SlotGlyph.Opacity = 1;
        SlotGlyphLeaving.Opacity = 0;
        var length = new Duration(TimeSpan.FromMilliseconds(ComposerButton.SlotCrossfadeMs));
        var coming = new Animation.DoubleAnimation { From = 0, To = 1, Duration = length };
        var going = new Animation.DoubleAnimation { From = 1, To = 0, Duration = length };
        Animation.Storyboard.SetTarget(coming, SlotGlyph);
        Animation.Storyboard.SetTargetProperty(coming, "Opacity");
        Animation.Storyboard.SetTarget(going, SlotGlyphLeaving);
        Animation.Storyboard.SetTargetProperty(going, "Opacity");
        slotFade = new Animation.Storyboard();
        slotFade.Children.Add(coming);
        slotFade.Children.Add(going);
        slotFade.Begin();
    }

    /// <summary>
    /// A sentence on the composer's notice line, said as it appears: a dimmed control explaining itself (S1.3), a refusal —
    /// shown but never said while a recording runs, when the app's own speech would be in the note (S6): a play button
    /// pressed then has the sentence as its HelpText already.
    /// </summary>
    private void Explain(string sentence)
    {
        ShowProblem(sentence);
        if (VoiceNotes.NoticeSaid(recording: recorder is not null))
        {
            Announce(ComposerError);
        }
    }

    /// <summary>
    /// Said to a screen reader and never drawn (S6): "Recording", "Recording deleted", "Voice message sent", "Ready to review,
    /// 0:42", "30 seconds left" — on the hidden status line, a polite live region, which is emptied a moment later so a
    /// reader walking the window does not come upon an old one.
    /// </summary>
    private void SayAloud(string sentence)
    {
        if (gone)
        {
            return;
        }
        RecordingStatus.Text = sentence;
        Announce(RecordingStatus);
        if (statusClear is null)
        {
            statusClear = DispatcherQueue.CreateTimer();
            statusClear.Interval = TimeSpan.FromSeconds(5);
            statusClear.IsRepeating = false;
            statusClear.Tick += (_, _) =>
            {
                if (!gone)
                {
                    RecordingStatus.Text = string.Empty;
                }
            };
        }
        statusClear.Stop();
        statusClear.Start();
    }

    /// <summary>
    /// No app sound while something records (S1.7): every play button — a bubble's recording, a staged or not-sent note — is
    /// drawn dimmed and says why, as HelpText and when pressed; it stays a button, because dimmed is not disabled.
    /// </summary>
    private void ShowPlayDimming()
    {
        foreach (var toggle in audioRows.Values.Select(row => row.Toggle)
                     .Concat(stagedRows.Values.Select(row => row.Toggle))
                     .Concat(parkedRows.Values.Select(row => row.Toggle)))
        {
            DimPlay(toggle);
        }
    }

    private void DimPlay(Button toggle)
    {
        var recording = recorder is not null;
        toggle.Opacity = recording ? 0.4 : 1;
        AutomationProperties.SetHelpText(toggle, recording ? services.Say.Get("You can play this after recording.") : string.Empty);
    }

    // ---- voice notes ----------------------------------------------------------------------------
    //
    // The recorder made safe (docs/audio-video-messages-2026-10-04.md, Phase 0) and put in the Send slot (Phase 1): whatever
    // ends a recording lets go of the microphone at once; the person's Send sends it, their Stop stages it for review, their
    // Delete deletes it; anything else STOPS AND KEEPS it as a voice message that was not sent, in its own row, never sent
    // by anything but that row and deleted only by the person or a sign-out.

    /// <summary>A recording stopped to ask "Delete this recording?": what it recorded, while the question is up.</summary>
    private sealed class AskedRecording(long chatId, Recorded recorded, ContentDialog question)
    {
        public long ChatId { get; } = chatId;

        public Recorded Recorded { get; } = recorded;

        public ContentDialog Question { get; } = question;

        /// <summary>Taken out of the question's hands by an interruption, which kept the recording: the answer is moot.</summary>
        public bool Settled { get; set; }
    }

    /// <summary>
    /// Whether something recorded is held only in memory: a recording running, one being asked about, a voice note in
    /// review, or one not sent that the disk refused. A real close keeps them first (MainWindow), because nothing awaited
    /// after the window goes would finish.
    /// </summary>
    internal bool HoldsRecordings =>
        recorder is not null || asking is not null || settling.Count > 0 || strips.Values.Any(strip => strip.HoldsRecordings)
        || parked.HoldsInMemory;

    /// <summary>
    /// Something other than the person ends what is being recorded — leaving the chat, a call, the window hidden, minimised
    /// or closing, the session locking, the computer sleeping (S4). It STOPS AND KEEPS: the recording waits in its chat as a
    /// voice message that was not sent; a question about deleting one is taken away and the recording kept; and where the
    /// chat is left — or the window really closes — a voice note still in review goes the same way, taking the words in the
    /// field as its caption. Everything that reads or changes the composer happens before this returns, so a caller that
    /// goes on to change the composer changes it after.
    /// </summary>
    internal Task Interrupt(RecordingEnd why)
    {
        // The video recorder hears the same interruptions, its own way (S4's three video columns).
        if (roundRecorder is { IsOpen: true } layer && RoundVideoRules.InterruptionOf(why) is { } heard)
        {
            layer.Interrupt(heard);
        }
        // What PLAYS hears the same interruptions (S4's last column): a call, a lock, a hidden window.
        if (PlaybackPauses.Of(why) is { } happened)
        {
            PausePlayback(happened);
        }
        // A start still waiting on Windows' prompt or the lead-in has no recorder to stop: it lets go of the microphone
        // once it is granted, rather than recording behind a lock screen (S4).
        recordingStart.Interrupted();
        var work = new List<Task>();
        if (asking is { } question)
        {
            work.Add(KeepAskedAsync(question, why));
        }
        if (recorder is not null)
        {
            work.Add(EndRecordingAsync(why));
        }
        if (NotSent.ParksReview(why))
        {
            foreach (var chatId in strips.Keys.ToList())
            {
                work.Add(ParkReviewNotesAsync(chatId));
            }
            if (settling.Count > 0)
            {
                // A Stop or a Delete already under way lands in review after this: it is kept when it does.
                work.Add(ParkReviewAfterAsync([.. settling]));
            }
        }
        // The app is going: what the disk refused and memory held is offered to the disk once more, after the rest.
        return why == RecordingEnd.WindowClosed ? WriteHeldAfterAsync(Task.WhenAll(work)) : Task.WhenAll(work);
    }

    /// <summary>
    /// A real close: once everything else is kept, the voice messages the disk refused (S4) are tried on it again — the last
    /// chance before the app goes and memory with it.
    /// </summary>
    private async Task WriteHeldAfterAsync(Task keeping)
    {
        try
        {
            await keeping;
        }
        finally
        {
            if (parked.HoldsInMemory)
            {
                var left = await Task.Run(parked.WriteHeld);
                if (left > 0)
                {
                    Diagnostics.Write($"voice messages that were not sent and could not be written before closing: {left}");
                }
            }
        }
    }

    /// <summary>What recordings still settling put in review, kept as "not sent" once they have landed.</summary>
    private async Task ParkReviewAfterAsync(IReadOnlyList<Task> landing)
    {
        await Task.WhenAll(landing);
        foreach (var chatId in strips.Keys.ToList())
        {
            await ParkReviewNotesAsync(chatId);
        }
    }

    /// <summary>One stop's settling, from the microphone let go of until what it recorded is wherever it goes.</summary>
    private TaskCompletionSource Settling()
    {
        var landed = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        settling.Add(landed.Task);
        return landed;
    }

    private void Settled(TaskCompletionSource landed)
    {
        settling.Remove(landed.Task);
        landed.TrySetResult();
    }

    /// <summary>
    /// Record a voice note (docs/protocol.md, "Audio"; docs/audio-video-messages-2026-10-04.md, S2.2): the microphone into an
    /// M4A, the recording row in place of the field — Delete, the clock, Stop — and the slot beside it the Send arrow, or
    /// Stop when it began beside words or staged items.
    /// </summary>
    /// <remarks>
    /// Refused, with the reason, the way the Send slot's rows refuse (S1.3): in an edit, during a call, while an attachment
    /// is on its way, and while this chat holds a voice message that was not sent — and never in the assistant's chat
    /// (decision 24). Whatever is playing goes quiet first, so the app's own sound does not open the note; with a screen
    /// reader running, "Recording" is said BEFORE the microphone records, and the recording and its clock start a second
    /// later (S6); the screen is kept on and the call buttons are off until it ends.
    /// </remarks>
    /// <param name="fromSlot">The slot's own click (row 10), which arms its 600 ms guard (S1.1) — the menu and the shortcut do not.</param>
    private async Task StartRecordingAsync(bool fromSlot)
    {
        // Not while the last one is still landing — a moment after its Send, Stop or Delete — so what that one says is never
        // said into this one, and its review never arrives under this one's row.
        if (open is not { } chat || recorder is not null || asking is not null || recordingStart.Starting || settling.Count > 0
            || Kind(chat) == "ai")
        {
            return;
        }
        var say = services.Say;
        var strip = Staging(chat.ChatId);
        if (VoiceNotes.Refusal(
                editing is not null, callBusy, Busy(strip), HasNotSent(chat.ChatId), !strip.CanStage, say) is { } refused)
        {
            Explain(refused);
            return;
        }
        recordingStart.Begin();
        if (fromSlot)
        {
            slotGuard.Arm(Environment.TickCount64, ComposerBox.Text);
        }
        QuietForRecording();
        try
        {
            var leadIn = VoiceNotes.LeadIn(ScreenReader.Running());
            var spoken = false;
            Func<Task>? speakFirst = leadIn > TimeSpan.Zero
                ? async () =>
                {
                    spoken = true;
                    SayAloud(say.Get("Recording"));
                    await Task.Delay(leadIn);
                }
                : null;
            var (started, failure) = await VoiceRecorder.StartAsync(speakFirst);
            if (started is null)
            {
                if (gone || open != chat)
                {
                    return;
                }
                if (failure != RecordingFailure.MicrophoneDenied)
                {
                    Explain(say.Get("Couldn't start recording."));
                    return;
                }
                // Settings is offered only where a switch there can help — asked of Windows first (S2.2).
                var (sentence, offersSettings) = VoiceNotes.MicrophoneRefusal(VoiceRecorder.MicrophoneAccess(), say);
                if (sentence is not null)
                {
                    ShowProblem(sentence, offersSettings ? (say.Get("Open Settings"), OpenMicrophoneSettings) : null);
                    Announce(ComposerError);
                }
                return;
            }
            // Granted after the chat went, beside one already running, while another is being asked about, with a call or an
            // edit that began while Windows asked, behind a window minimised or hidden meanwhile, or after the session locked,
            // the computer slept or the screen saver started (S4): let go of the microphone.
            if (open != chat || gone || recorder is not null || asking is not null || callBusy || services.WindowAway || editing is not null
                || recordingStart.WasInterrupted || SessionWatch.ScreenSaverRunning())
            {
                await started.DisposeAsync();
                if (callBusy && !gone && open == chat)
                {
                    Explain(say.Get("You can record a message after the call."));
                }
                return;
            }
            recorder = started;
            recordingChat = chat.ChatId;
            // Whatever began playing while the microphone was being opened — the system's media keys reach the player
            // whatever the app refuses — goes quiet before the recording does (S1.7).
            QuietForRecording();
            // Beside words or staged items — the paperclip, the menu, the shortcut — the words are hidden behind the row and
            // must not leave unseen: the slot is Stop, and Stop stages the note beside them (row 3).
            recordingBesideDraft = !string.IsNullOrWhiteSpace(ComposerBox.Text) || strip.Items.Count > 0;
            warnedThirtySeconds = false;
            // The microphone pulled out or taken away mid-recording: what can be read back is kept (S4).
            started.Failed += () => DispatcherQueue.TryEnqueue(() =>
            {
                if (!gone && ReferenceEquals(recorder, started))
                {
                    _ = EndRecordingAsync(RecordingEnd.RecorderFailed);
                }
            });
            keepAwake.Hold();
            ComposerError.Visibility = Visibility.Collapsed;
            recordingTimer ??= RecordingClock();
            recordingTimer.Start();
            if (fromSlot)
            {
                // The slot just turned into the Send arrow under the pointer: a double click on the microphone cannot send.
                slotGuard.Arm(Environment.TickCount64, ComposerBox.Text);
            }
            ShowRecordingRow();
            if (!spoken)
            {
                SayAloud(say.Get("Recording"));
            }
        }
        finally
        {
            recordingStart.End();
            DrawSlot();
        }
    }

    /// <summary>
    /// The row's clock — and the five-minute ceiling, which stops into review by itself and says so; and the screen saver,
    /// which says nothing to an app and so is asked about, and stops and keeps (S4).
    /// </summary>
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
                _ = EndRecordingAsync(RecordingEnd.Capped);
                return;
            }
            if (SessionWatch.ScreenSaverRunning())
            {
                _ = EndRecordingAsync(RecordingEnd.SessionLocked);
                return;
            }
            ShowRecording();
        };
        return timer;
    }

    /// <summary>
    /// The clock, "0:42" — never announced (S6) — and from 4:30 "30 seconds left" beside it in orange, said once: words as well
    /// as colour (S2.5, WCAG 1.4.1). Windows draws no level meter in this version (S2.9), so the words take its place.
    /// </summary>
    private void ShowRecording()
    {
        var elapsed = recorder?.Elapsed ?? TimeSpan.Zero;
        RecordingTime.Text = VoiceNotes.Clock(elapsed);
        if (!VoiceNotes.Warns(elapsed))
        {
            return;
        }
        if (RecordingWarning.Visibility != Visibility.Visible)
        {
            RecordingWarning.Text = services.Say.Get("30 seconds left");
            RecordingWarning.Visibility = Visibility.Visible;
            RecordingTime.Foreground = (Brush)Application.Current.Resources["SystemFillColorCautionBrush"];
        }
        if (!warnedThirtySeconds)
        {
            warnedThirtySeconds = true;
            SayAloud(services.Say.Get("30 seconds left"));
        }
    }

    /// <summary>
    /// The recording row in place of the field and the buttons beside it (S2.4) — at the height the row had, so nothing above
    /// it moves — with its middle Stop only when the slot is not Stop already, and focus on the slot, where it stays.
    /// </summary>
    private void ShowRecordingRow()
    {
        RecordingRow.MinHeight = Math.Max(ComposerBox.ActualHeight, ComposerTools.ActualHeight);
        RecordingStop.Visibility = recordingBesideDraft ? Visibility.Collapsed : Visibility.Visible;
        RecordingWarning.Visibility = Visibility.Collapsed;
        RecordingTime.ClearValue(TextBlock.ForegroundProperty);
        ShowRecording();
        ComposerTools.Visibility = Visibility.Collapsed;
        ComposerBox.Visibility = Visibility.Collapsed;
        SuggestionScroller.Visibility = Visibility.Collapsed;
        RecordingRow.Visibility = Visibility.Visible;
        FitRecordingRow();
        StartPulse();
        AttachButton.IsEnabled = false;
        ShowCallButtons();
        ShowPlayDimming();
        QuietViewerVideo();
        DrawSlot();
        SendButton.Focus(FocusState.Programmatic);
    }

    /// <summary>Too narrow for words (S2.4): Delete and Stop become icons, with the same names.</summary>
    private void FitRecordingRow()
    {
        var narrow = RecordingRow.ActualWidth is > 0 and < 300;
        if (narrow == recordingRowNarrow)
        {
            return;
        }
        recordingRowNarrow = narrow;
        var say = services.Say;
        // Delete and Stop, in Segoe Fluent Icons.
        RecordingDelete.Content = narrow ? new FontIcon { Glyph = ((char)0xE74D).ToString(), FontSize = 14 } : say.Get("Delete");
        RecordingStop.Content = narrow ? new FontIcon { Glyph = ((char)ComposerButton.StopGlyph).ToString(), FontSize = 14 } : say.Get("Stop");
    }

    /// <summary>The red dot pulses between 100 % and 40 % once a second while it records — and stays still when Windows' animations are off (S2.9).</summary>
    private void StartPulse()
    {
        StopPulse();
        if (!StickerImaging.AnimationsWanted())
        {
            return;
        }
        var pulse = new Animation.DoubleAnimation
        {
            From = 1,
            To = 0.4,
            Duration = new Duration(TimeSpan.FromMilliseconds(500)),
            AutoReverse = true,
            RepeatBehavior = Animation.RepeatBehavior.Forever,
        };
        Animation.Storyboard.SetTarget(pulse, RecordingDot);
        Animation.Storyboard.SetTargetProperty(pulse, "Opacity");
        recordingPulse = new Animation.Storyboard();
        recordingPulse.Children.Add(pulse);
        recordingPulse.Begin();
    }

    private void StopPulse()
    {
        recordingPulse?.Stop();
        recordingPulse = null;
    }

    /// <summary>
    /// However a recording ends — Send, Stop, five minutes, Delete, or something that is not the person — the microphone is
    /// let go of at once, and what it recorded goes where <see cref="NotSent.Ended"/> says: sent, review, its not-sent row,
    /// or nowhere. What the person did is said to a screen reader (S6), and their own ending gives the keyboard back to the
    /// field, so a second Enter cannot open the microphone again (S2.4).
    /// </summary>
    private async Task EndRecordingAsync(RecordingEnd why)
    {
        if (recorder is not { } stopping)
        {
            return;
        }
        var chatId = recordingChat;
        // Read NOW, before leaving the chat ends the composer's mode: the reply it was recorded under goes with it — and the
        // slot's Send carries the reply the composer is primed with at the moment it is pressed, shown above the row.
        var take = TakeForInterrupted(chatId, why);
        var sending = why == RecordingEnd.Sent && open?.ChatId == chatId ? open : null;
        var replyTo = sending is not null && editing is null ? replyingTo?.Id : null;
        RecordingStopped();
        if (!gone && open?.ChatId == chatId && NotSent.GivesFocusBack(why))
        {
            ComposerBox.Focus(FocusState.Programmatic);
        }
        var landed = Settling();
        try
        {
            var recorded = await stopping.StopAsync();
            var outcome = NotSent.Ended(keepsRecordings ? why : RecordingEnd.SignedOut, recorded.Elapsed, recorded.Bytes?.Length, services.Say);
            ReleaseReply(chatId, take, outcome.Fate);
            var fate = outcome.Fate == RecordingFate.Send
                ? await SendRecordedAsync(sending, chatId, recorded, replyTo)
                : await SettleAsync(chatId, recorded, outcome.Fate, take.ReplyTo, caption: null);
            if (gone || open?.ChatId != chatId)
            {
                return;
            }
            if (outcome.Sentence is { } sentence)
            {
                Explain(sentence);
            }
            else if (NotSent.Said(why, fate, VoiceNotes.DurationMs(recorded.Elapsed), services.Say) is { } said)
            {
                SayAloud(said);
            }
        }
        finally
        {
            Settled(landed);
        }
    }

    /// <summary>
    /// The slot's Send in row 2 (S2.5): the note goes through the outbox like any media send — its row written before its
    /// first byte, its bytes kept until the ack — with the reply the composer was primed with. The person pressed Send, so a
    /// chat switched in the same breath does not stop it. A note that cannot be written down lands in review with the
    /// error, and one the window let go of meanwhile waits as not sent: never lost. Answers where it went.
    /// </summary>
    /// <param name="chat">The conversation the Send was pressed in, or null when it was not open — kept as not sent instead.</param>
    private async Task<RecordingFate> SendRecordedAsync(ConversationModel? chat, long chatId, Recorded recorded, long? replyTo)
    {
        if (!keepsRecordings || recorded.Bytes is not { } bytes || VoiceNotes.Staged(bytes, recorded.Elapsed) is not { } note)
        {
            return RecordingFate.Discard;
        }
        if (chat is null || gone)
        {
            await ParkAsync(chatId, bytes, note.DurationMs ?? 0, replyTo, caption: null);
            return RecordingFate.Park;
        }
        var store = connection.Staging;
        string? handle = null;
        sendingMedia = true;
        DrawSlot();
        try
        {
            handle = await Task.Run(() => store.Stage(note));
            chat.Send(
                string.Empty, replyToMessageId: replyTo, pendingFiles: [handle],
                mentions: ComposerMentions.ForSend(string.Empty, connection.Chats.Members(), IsFamily(chat)));
        }
        catch (Exception e)
        {
            Diagnostics.Write($"sending a voice message: {e.GetType().Name}");
            var kept = await SettleAsync(chatId, recorded, RecordingFate.Review, replyTo, caption: null);
            if (!gone && open?.ChatId == chatId)
            {
                Explain(services.Say.Get("Something went wrong. Try again."));
            }
            return kept;
        }
        finally
        {
            if (handle is not null)
            {
                // The row that names it is in the outbox now (or never will be): the sweep may judge it.
                store.Release([handle]);
            }
            sendingMedia = false;
            DrawSlot();
        }
        // Written down: from here it is the outbox's, whatever the window does next.
        if (!gone && open == chat && replyTo is not null && editing is null && replyingTo?.Id == replyTo)
        {
            // The reply went with the note: the composer is primed for nothing now.
            EndComposerMode(clear: false);
        }
        if (!gone && open == chat)
        {
            Queued();
        }
        else
        {
            _ = connection.Live.FlushAsync(SendRules.FlushTrigger.Queued);
        }
        return RecordingFate.Send;
    }

    /// <summary>
    /// The reply went with a recording an interruption kept, and nothing else in the composer was going to carry it: the
    /// composer, empty, is primed for nothing now — unless it has been primed for something else since.
    /// </summary>
    private void ReleaseReply(long chatId, ComposerTake take, RecordingFate fate)
    {
        if (fate == RecordingFate.Park && take.ClearsReply && !gone && open?.ChatId == chatId
            && editing is null && replyingTo?.Id == take.ReplyTo)
        {
            EndComposerMode(clear: false);
        }
    }

    /// <summary>What a recording an interruption keeps takes from the composer — nothing, when the person ended it.</summary>
    private ComposerTake TakeForInterrupted(long chatId, RecordingEnd why) =>
        why is RecordingEnd.Stopped or RecordingEnd.Sent or RecordingEnd.Capped or RecordingEnd.Deleted or RecordingEnd.SignedOut
        || !keepsRecordings || gone || open?.ChatId != chatId
            ? ComposerTake.Nothing
            : NotSent.ForRecording(ComposerBox.Text, replyingTo?.Id, editing is not null, Staging(chatId).Items.Count > 0);

    /// <summary>
    /// The microphone's part of every ending: the row gives the field and its buttons back, and the clock, the screen, the
    /// call buttons, the play buttons, the viewer's video and the slot go back to how they were.
    /// </summary>
    private void RecordingStopped()
    {
        recorder = null;
        recordingTimer?.Stop();
        keepAwake.Release();
        if (gone)
        {
            return;
        }
        StopPulse();
        RecordingRow.Visibility = Visibility.Collapsed;
        ComposerBox.Visibility = Visibility.Visible;
        ComposerTools.Visibility = Visibility.Visible;
        AttachButton.IsEnabled = !finding;
        DrawSuggestions();
        ShowCallButtons();
        ShowPlayDimming();
        QuietViewerVideo();
        DrawSlot();
    }

    /// <summary>
    /// The row's Delete (S2.5). Under ten seconds the recording is deleted at once; from ten seconds it STOPS FIRST — nothing
    /// more is recorded while the question is up — and then asks "Delete this recording?" [Delete] [Keep], and Keep stages it
    /// for review. An interruption meanwhile takes the question away and keeps the recording as "not sent".
    /// </summary>
    private async Task DeleteRecordingAsync()
    {
        if (recorder is not { } running)
        {
            return;
        }
        if (running.Elapsed < VoiceNotes.DeleteAsksFrom)
        {
            await EndRecordingAsync(RecordingEnd.Deleted);
            return;
        }
        var chatId = recordingChat;
        RecordingStopped();
        var landed = Settling();
        try
        {
            await AskBeforeDeletingAsync(chatId, await running.StopAsync());
        }
        finally
        {
            Settled(landed);
        }
    }

    /// <summary>The question itself, about a recording that has stopped and been read back.</summary>
    private async Task AskBeforeDeletingAsync(long chatId, Recorded recorded)
    {
        var outcome = NotSent.Ended(RecordingEnd.Stopped, recorded.Elapsed, recorded.Bytes?.Length, services.Say);
        if (outcome.Fate != RecordingFate.Review)
        {
            // Nothing that could be read back: nothing to ask about, and the composer says what happened.
            if (outcome.Sentence is { } sentence && !gone && open?.ChatId == chatId)
            {
                ShowProblem(sentence);
                Announce(ComposerError);
            }
            return;
        }
        if (gone || !keepsRecordings || open?.ChatId != chatId)
        {
            // Left, closed or signed out while it stopped: an interruption's rule, not a question nobody is there to answer.
            await SettleAsync(chatId, recorded, keepsRecordings ? RecordingFate.Park : RecordingFate.Discard, replyTo: null, caption: null);
            return;
        }
        var question = new AskedRecording(chatId, recorded, Dialogs.DeleteRecording(XamlRoot, services.Say));
        asking = question;
        ContentDialogResult answer;
        try
        {
            answer = await question.Question.ShowAsync();
        }
        catch (Exception e)
        {
            // Another dialog was up: nothing was asked, so nothing is deleted — it goes to review.
            Diagnostics.Write($"asking before deleting a recording: {e.GetType().Name}");
            answer = ContentDialogResult.None;
        }
        if (question.Settled)
        {
            return;
        }
        asking = null;
        // Delete: gone. Keep: to review, as the person's Stop would have put it. Either is said (S6).
        var (why, went) = answer == ContentDialogResult.Primary
            ? (RecordingEnd.Deleted, RecordingFate.Discard)
            : (RecordingEnd.Stopped, await SettleAsync(chatId, recorded, RecordingFate.Review, replyTo: null, caption: null));
        if (gone || open?.ChatId != chatId)
        {
            return;
        }
        if (NotSent.GivesFocusBack(why))
        {
            ComposerBox.Focus(FocusState.Programmatic);
        }
        if (NotSent.Said(why, went, VoiceNotes.DurationMs(recorded.Elapsed), services.Say) is { } said)
        {
            SayAloud(said);
        }
    }

    /// <summary>An interruption while "Delete this recording?" is up: the question goes, and the recording is kept as "not sent".</summary>
    private async Task KeepAskedAsync(AskedRecording question, RecordingEnd why)
    {
        asking = null;
        question.Settled = true;
        var take = TakeForInterrupted(question.ChatId, why);
        try
        {
            question.Question.Hide();
        }
        catch (Exception e)
        {
            Diagnostics.Write($"taking a question away: {e.GetType().Name}");
        }
        var outcome = NotSent.Ended(keepsRecordings ? why : RecordingEnd.SignedOut, question.Recorded.Elapsed, question.Recorded.Bytes?.Length, services.Say);
        ReleaseReply(question.ChatId, take, outcome.Fate);
        await SettleAsync(question.ChatId, question.Recorded, outcome.Fate, take.ReplyTo, caption: null);
    }

    /// <summary>
    /// A stopped recording, put where it goes, and where it went answered. REVIEW is the strip of the chat it was recorded in
    /// while that chat is open — and its not-sent row when it is not: a note in review in a chat somebody left is exactly a
    /// note not sent (S2.8).
    /// </summary>
    private async Task<RecordingFate> SettleAsync(long chatId, Recorded recorded, RecordingFate fate, long? replyTo, string? caption)
    {
        // A send is SendRecordedAsync's, never this; anything else that is not a discard is kept somewhere.
        if (fate == RecordingFate.Discard || recorded.Bytes is not { } bytes || !keepsRecordings)
        {
            return RecordingFate.Discard;
        }
        if (fate == RecordingFate.Review && !gone && open?.ChatId == chatId
            && VoiceNotes.Staged(bytes, recorded.Elapsed) is { } staged && Staging(chatId).Add(staged))
        {
            DrawStaging();
            // The field comes back, focused, for an optional caption (S2.7).
            ComposerBox.Focus(FocusState.Programmatic);
            return RecordingFate.Review;
        }
        await ParkAsync(chatId, bytes, VoiceNotes.DurationMs(recorded.Elapsed), replyTo, caption);
        return RecordingFate.Park;
    }

    /// <summary>
    /// Leaving a chat — or the window really closing — with a voice note still in review (S2.8): it waits as "not sent",
    /// taking the words in the field as its caption and the reply with them, and the field is left empty. Taken NOW, before
    /// the composer is handed to another chat or its draft is kept; only the writing waits.
    /// </summary>
    private Task ParkReviewNotesAsync(long chatId)
    {
        if (!keepsRecordings || !strips.TryGetValue(chatId, out var strip) || !strip.HoldsRecordings)
        {
            return Task.CompletedTask;
        }
        var here = !gone && open?.ChatId == chatId;
        var take = here ? NotSent.ForReview(ComposerBox.Text, replyingTo?.Id, editing is not null) : ComposerTake.Nothing;
        var notes = strip.TakeRecordings();
        if (take.ClearsWords)
        {
            ComposerBox.Text = string.Empty;
        }
        if (take.ClearsReply)
        {
            EndComposerMode(clear: false);
        }
        if (here)
        {
            DrawStaging();
        }
        return ParkNotesAsync(chatId, notes, take);
    }

    /// <summary>Several notes in review, one row each: every one keeps the reply, and only the first the words — one message's words, never two.</summary>
    private async Task ParkNotesAsync(long chatId, IReadOnlyList<StagedMedia> notes, ComposerTake take)
    {
        for (var at = 0; at < notes.Count; at++)
        {
            await ParkAsync(chatId, notes[at].Bytes.ToArray(), notes[at].DurationMs ?? 0, take.ReplyTo, at == 0 ? take.Caption : null);
        }
    }

    /// <summary>
    /// Written down as a voice message that was not sent. Where it cannot be written — a full disk (S4) — it is held in
    /// memory instead, as the same row with the same reply and caption, never carried by another Send (S2.8): kept for as
    /// long as the app runs, and the disk tried again as the window really closes.
    /// </summary>
    private async Task ParkAsync(long chatId, byte[] bytes, int durationMs, long? replyTo, string? caption)
    {
        try
        {
            await Task.Run(() => parked.Park(chatId, bytes, durationMs, replyTo, caption, DateTimeOffset.UtcNow));
        }
        catch (Exception e)
        {
            Diagnostics.Write($"keeping a voice message that was not sent: {e.GetType().Name}");
            if (!keepsRecordings)
            {
                return;
            }
            parked.Hold(chatId, bytes, durationMs, replyTo, caption, DateTimeOffset.UtcNow);
            if (!gone && open?.ChatId == chatId)
            {
                DrawNotSent();
            }
            return;
        }
        if (!keepsRecordings)
        {
            // The session ended while it was being written: it goes with everything else recorded and not sent.
            WipeParked();
            return;
        }
        if (!gone && open?.ChatId == chatId)
        {
            DrawNotSent();
        }
    }

    /// <summary>Whether this chat holds a voice message that was not sent; a disk that cannot be read blocks nothing.</summary>
    private bool HasNotSent(long chatId)
    {
        try
        {
            return parked.Any(chatId);
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            Diagnostics.Write($"reading voice messages that were not sent: {e.GetType().Name}");
            return false;
        }
    }

    /// <summary>Everything recorded and not sent, gone with the session — written, or held in memory for want of a disk.</summary>
    private void WipeParked()
    {
        parked.ForgetHeld();
        try
        {
            ParkedRecordings.WipeAll(AppFolders.ParkedPath);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"wiping voice messages that were not sent: {e.GetType().Name}");
        }
    }

    /// <summary>
    /// Starting a recording pauses whatever is playing (S1.7) — the one recording player, and a video in the viewer — and a
    /// recording on its way to playing is called off, so its bytes landing do not start it under the microphone.
    /// </summary>
    private void QuietForRecording()
    {
        fetchingAudio = 0;
        fetchingLocal++;
        if (AudioRunning)
        {
            audio.Pause();
        }
        ShowPlayback();
        try
        {
            ViewerVideo.MediaPlayer?.Pause();
        }
        catch (Exception e)
        {
            Diagnostics.Write($"pausing the viewer's video: {e.GetType().Name}");
        }
    }

    /// <summary>Windows' own microphone page — offered only after <c>AppCapability</c> said a switch there can help.</summary>
    private void OpenMicrophoneSettings()
    {
        try
        {
            _ = Windows.System.Launcher.LaunchUriAsync(new Uri(VoiceNotes.MicrophoneSettingsPage));
        }
        catch (Exception e)
        {
            Diagnostics.Write($"opening the microphone settings: {e.GetType().Name}");
        }
    }

    /// <summary>
    /// The open chat's voice messages that were not sent (S2.8), a row each above the composer: "Voice message not sent ·
    /// 0:42", the reply it was recorded under and its caption, then ▶, Send and ✕.
    /// </summary>
    private void DrawNotSent()
    {
        NotSentPanel.Children.Clear();
        parkedRows.Clear();
        IReadOnlyList<ParkedRecording> waiting = [];
        if (!gone && open is { } chat)
        {
            try
            {
                waiting = parked.Of(chat.ChatId);
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException)
            {
                Diagnostics.Write($"reading voice messages that were not sent: {e.GetType().Name}");
            }
            foreach (var entry in waiting)
            {
                NotSentPanel.Children.Add(NotSentRow(chat, entry));
            }
        }
        NotSentPanel.Visibility = waiting.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
        // While one waits, the microphone is dimmed and says so (S1.3 row 9).
        notSentShown = waiting.Count;
        ReconcileLocal();
        DrawSlot();
    }

    private FrameworkElement NotSentRow(ConversationModel chat, ParkedRecording entry)
    {
        var say = services.Say;
        var resources = Application.Current.Resources;
        var secondary = (Brush)resources["TextFillColorSecondaryBrush"];
        var grid = new Grid { ColumnSpacing = 10 };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        // A microphone, in Segoe Fluent Icons.
        grid.Children.Add(new FontIcon
        {
            Glyph = ((char)0xE720).ToString(),
            FontSize = 16,
            Foreground = secondary,
            VerticalAlignment = VerticalAlignment.Center,
        });
        var lines = new StackPanel { Spacing = 2, VerticalAlignment = VerticalAlignment.Center };
        var idle = NotSent.Line(entry.DurationMs, say);
        var line = new TextBlock { Text = idle, TextTrimming = TextTrimming.CharacterEllipsis };
        Typography.SetNumeralAlignment(line, FontNumeralAlignment.Tabular);
        lines.Children.Add(line);
        // The reply it was recorded under — which is the one it goes with, whatever the composer is primed with now.
        if (entry.ReplyToMessageId is { } replyId && connection.Chats.Message(replyId) is { } answered)
        {
            lines.Children.Add(new TextBlock
            {
                Text = Quotes.Banner(answered, connection.Chats, say),
                FontSize = 12,
                Foreground = secondary,
                TextTrimming = TextTrimming.CharacterEllipsis,
            });
        }
        if (entry.Caption is { } caption)
        {
            lines.Children.Add(new TextBlock
            {
                Text = caption,
                FontSize = 12,
                Foreground = secondary,
                TextWrapping = TextWrapping.Wrap,
                MaxLines = 2,
                TextTrimming = TextTrimming.CharacterEllipsis,
            });
        }
        Grid.SetColumn(lines, 1);
        grid.Children.Add(lines);
        // Its ▶ (S2.8): the note as it would be sent, from this device.
        var (play, glyph) = PlayButton();
        play.Click += (_, _) => _ = ToggleLocalAsync(null, entry);
        parkedRows[entry.Id] = new LocalRow(play, glyph, line, idle, VoiceNotes.TotalSeconds(entry.DurationMs));
        DimPlay(play);
        Grid.SetColumn(play, 2);
        grid.Children.Add(play);
        var send = new Button { Content = say.Get("Send"), VerticalAlignment = VerticalAlignment.Center };
        if (resources.TryGetValue("AccentButtonStyle", out var accent) && accent is Style accented)
        {
            send.Style = accented;
        }
        ToolTipService.SetToolTip(send, say.Get("Send voice message"));
        AutomationProperties.SetName(send, say.Get("Send voice message"));
        send.Click += (_, _) => _ = SendParkedAsync(chat, entry);
        Grid.SetColumn(send, 3);
        grid.Children.Add(send);
        var delete = new Button { Content = "✕", Padding = new Thickness(8, 4, 8, 4), VerticalAlignment = VerticalAlignment.Center };
        ToolTipService.SetToolTip(delete, say.Get("Delete recording"));
        AutomationProperties.SetName(delete, say.Get("Delete recording"));
        delete.Click += (_, _) => _ = DeleteParkedAsync(entry);
        Grid.SetColumn(delete, 4);
        grid.Children.Add(delete);
        return new Border
        {
            Child = grid,
            Padding = new Thickness(10, 6, 6, 6),
            CornerRadius = new CornerRadius(8),
            Background = (Brush)resources["CardBackgroundFillColorDefaultBrush"],
            BorderBrush = (Brush)resources["CardStrokeColorDefaultBrush"],
            BorderThickness = new Thickness(1),
        };
    }

    /// <summary>
    /// A not-sent row's Send: THAT recording, with THAT reply and caption, and nothing else — never the composer's words,
    /// staged items or reply, and never carried by another Send (S2.8). Asked about exactly as the Send button is where the
    /// assistant would hear it, and written down in the outbox BEFORE it leaves the store, so a crash in between can never
    /// lose it.
    /// </summary>
    private async Task SendParkedAsync(ConversationModel chat, ParkedRecording entry)
    {
        if (gone || open != chat || sendingMedia || !HasEntry(chat.ChatId, entry))
        {
            return;
        }
        var say = services.Say;
        var caption = entry.Caption ?? string.Empty;
        var session = connection.Session.State;
        var chatKind = Kind(chat);
        if (AssistantConsent.IsRequired(chatKind, caption, session.Assistant?.Processor, session.AssistantConsentAt))
        {
            _ = ReviewAssistantConsentAsync(() =>
            {
                // A yes is for the chat it was asked in.
                if (!gone && open == chat)
                {
                    _ = SendParkedAsync(chat, entry);
                }
            });
            return;
        }
        if (AssistantConsent.IsWithheldFromAnUnnamedAssistant(chatKind, caption, session.Assistant is not null, session.Assistant?.Processor))
        {
            ShowProblem(say.Get("This server hasn't said which service answers, so nothing can be sent to the assistant here."));
            return;
        }
        var store = connection.Staging;
        string? handle = null;
        sendingMedia = true;
        DrawSlot();
        ComposerError.Visibility = Visibility.Collapsed;
        try
        {
            var media = await Task.Run(() => parked.Staged(entry));
            if (media is null)
            {
                ShowProblem(say.Get("Something went wrong. Try again."));
                return;
            }
            handle = await Task.Run(() => store.Stage(media));
            if (gone || open != chat)
            {
                // Not sent after all: it stays in its row.
                return;
            }
            chat.Send(
                caption, replyToMessageId: entry.ReplyToMessageId, pendingFiles: [handle],
                mentions: ComposerMentions.ForSend(caption, connection.Chats.Members(), IsFamily(chat)));
            if (!await Task.Run(() => parked.Remove(entry)))
            {
                Diagnostics.Write("a voice message that was sent could not be taken out of the not-sent store");
            }
            DrawNotSent();
            Queued();
            if (!gone && open == chat)
            {
                // Its Send went with its row: the keyboard goes back to the field, and a screen reader hears what happened.
                RowEnded(sent: true, entry.DurationMs);
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"sending a voice message that was not sent: {e.GetType().Name}");
            if (!gone)
            {
                ShowProblem(say.Get("Something went wrong. Try again."));
            }
        }
        finally
        {
            if (handle is not null)
            {
                // The row that names it is in the outbox now (or never will be): the sweep may judge it.
                store.Release([handle]);
            }
            sendingMedia = false;
            DrawSlot();
        }
    }

    private bool HasEntry(long chatId, ParkedRecording entry)
    {
        try
        {
            return parked.Of(chatId).Any(held => held.Id == entry.Id);
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            Diagnostics.Write($"reading voice messages that were not sent: {e.GetType().Name}");
            return false;
        }
    }

    /// <summary>A not-sent row's ✕: deleted — after "Delete this recording?" from ten seconds, because it cannot be made again.</summary>
    private async Task DeleteParkedAsync(ParkedRecording entry)
    {
        if (VoiceNotes.AsksBeforeDeleting(entry.DurationMs))
        {
            ContentDialogResult answer;
            try
            {
                answer = await Dialogs.DeleteRecording(XamlRoot, services.Say).ShowAsync();
            }
            catch (Exception e)
            {
                // Only one dialog may be up at a time: nothing was asked, so nothing is deleted.
                Diagnostics.Write($"asking before deleting a recording: {e.GetType().Name}");
                return;
            }
            if (answer != ContentDialogResult.Primary || gone)
            {
                return;
            }
        }
        await Task.Run(() => parked.Remove(entry));
        if (!gone)
        {
            DrawNotSent();
            // Its ✕ went with its row: the keyboard goes back to the field, and a screen reader hears what happened (S6).
            RowEnded(sent: false, entry.DurationMs);
        }
    }

    /// <summary>
    /// A not-sent row's own Send or ✕ took it, and its focused button with it: the keyboard goes back to the field — never left
    /// nowhere, which loses a screen reader's place — and what happened is said (<see cref="NotSent.RowEnded"/>).
    /// </summary>
    private void RowEnded(bool sent, int durationMs)
    {
        var (focusField, said) = NotSent.RowEnded(sent, durationMs, services.Say);
        if (focusField)
        {
            ComposerBox.Focus(FocusState.Programmatic);
        }
        SayAloud(said);
    }

    /// <summary>A sentence on the composer's notice line, said to a screen reader as it appears (S6): the line is a polite live region.</summary>
    private static void Announce(FrameworkElement line)
    {
        try
        {
            (FrameworkElementAutomationPeer.FromElement(line) ?? FrameworkElementAutomationPeer.CreatePeerForElement(line))
                ?.RaiseAutomationEvent(AutomationEvents.LiveRegionChanged);
        }
        catch (Exception e)
        {
            // An announcement is a courtesy: the sentence is on the screen either way.
            Diagnostics.Write($"announcing a notice: {e.GetType().Name}");
        }
    }

    /// <summary>One recording's row as drawn, so playback can keep it up to date.</summary>
    private sealed record AudioRow(long Id, Button Toggle, FontIcon Glyph, Slider Track, TextBlock Elapsed, double Total);

    /// <summary>
    /// A recording, drawn the way the Apple apps draw one (ios <c>AudioPlayerView</c>): a round play button, a scrubber, and
    /// where it is and how long it is under that — deliberately no waveform (docs/protocol.md, "Audio"). Downloaded rather
    /// than streamed: a player here cannot put the session's Authorization on the requests it would make.
    /// </summary>
    private FrameworkElement AudioElement(AttachmentDto attachment, bool mine, MessageDto message, string? chatKind)
    {
        var say = services.Say;
        var resources = Application.Current.Resources;
        var total = VoiceNotes.TotalSeconds(attachment.DurationMs);
        // An own balloon is filled with the accent, so nothing in it may be drawn in the accent too.
        var ink = mine ? Palette.Ink() : (Brush)resources["AccentFillColorDefaultBrush"];
        var glyph = new FontIcon
        {
            FontSize = 14,
            Foreground = mine ? Palette.Surface(ActualTheme) : (Brush)resources["TextOnAccentFillColorPrimaryBrush"],
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
        DimPlay(toggle);
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
        return WithTranscript(card, attachment, mine, message, chatKind);
    }

    /// <summary>
    /// One recording's place for its text, and what it needs to ask: the chat and the message it is on, and the form the
    /// rules chose with the ceiling the device's own sound must fit.
    /// </summary>
    private sealed record TranscriptHost(
        StackPanel Panel, long ChatId, long MessageId, AttachmentDto Attachment, bool Mine, TranscriptForm Form, long MaxBytes);

    /// <summary>
    /// Media Foundation is how this machine takes a sound track out of a file it holds (<see cref="SoundTracks"/>), and
    /// every Windows this app runs on has it — the player beside "Show text" is Media Foundation too. A file it cannot
    /// read is "Couldn't read the sound in this file.", and one too long to fit "This recording is too long to turn into
    /// text.", decided per file once the click lands (<see cref="TranscriptSound"/>).
    /// </summary>
    private const bool ExtractsSound = true;

    /// <summary>
    /// The player with "Show text" under it, where this member may ask (docs/protocol.md, "Transcripts on request").
    /// </summary>
    private FrameworkElement WithTranscript(
        FrameworkElement player, AttachmentDto attachment, bool mine, MessageDto message, string? chatKind)
    {
        if (TranscriptPanel(attachment, mine, message, chatKind) is not { } panel)
        {
            return player;
        }
        var both = new StackPanel { Spacing = 2 };
        both.Children.Add(player);
        both.Children.Add(panel);
        return both;
    }

    /// <summary>
    /// The place for one recording's text — a voice note's, an audio file's or a video's — where this member may ask:
    /// a server that transcribes, the rule for this member and this message, and a form to ask in (the server's stored
    /// copy, or sound this device makes) — <see cref="TranscriptRules.Offers"/> decides, and text this device already
    /// holds is always shown. Null where nothing is offered.
    /// </summary>
    private StackPanel? TranscriptPanel(AttachmentDto attachment, bool mine, MessageDto message, string? chatKind)
    {
        var state = connection.Session.State;
        if (message.Id <= 0
            || !TranscriptRules.Offers(
                state.Assistant, state.Family, chatKind, message.SenderId, Reader, attachment,
                held: connection.Transcripts.Holds(attachment.Id), extracts: ExtractsSound))
        {
            return null;
        }
        // Held text with no server to ask any more is still shown; it is then never asked for again.
        var form = state.Assistant is { } assistant && TranscriptRules.IsAvailable(assistant)
            ? TranscriptRules.Form(attachment, assistant, ExtractsSound)
            : TranscriptForm.None;
        var maxBytes = state.Assistant is { } stated ? TranscriptRules.MaxBytes(stated) : TranscriptRules.CeilingBytes;
        var panel = new StackPanel { Spacing = 2, Margin = new Thickness(4, 2, 4, 0), MaxWidth = 520, HorizontalAlignment = HorizontalAlignment.Left };
        var host = new TranscriptHost(panel, message.ChatId, message.Id, attachment, mine, form, maxBytes);
        // Registered while it is on screen, so a change redraws every copy of this recording that is showing, and none
        // that a redraw has already thrown away.
        panel.Loaded += (_, _) =>
        {
            if (!transcriptHosts.TryGetValue(attachment.Id, out var hosts))
            {
                transcriptHosts[attachment.Id] = hosts = [];
            }
            if (!hosts.Contains(host))
            {
                hosts.Add(host);
            }
            DrawTranscript(host);
        };
        panel.Unloaded += (_, _) =>
        {
            if (transcriptHosts.TryGetValue(attachment.Id, out var hosts) && hosts.Remove(host) && hosts.Count == 0)
            {
                transcriptHosts.Remove(attachment.Id);
            }
        };
        DrawTranscript(host);
        return panel;
    }

    private void DrawTranscripts(long attachmentId)
    {
        if (transcriptHosts.TryGetValue(attachmentId, out var hosts))
        {
            foreach (var host in hosts.ToList())
            {
                DrawTranscript(host);
            }
        }
    }

    /// <summary>
    /// What is under one player: "Show text"; "Getting the text…" while it is asked for; the text itself — selectable, so
    /// it can be copied — with "Hide text"; "No speech" for silence; or why it could not be had, with "Try Again" only
    /// where trying again could help.
    /// </summary>
    private void DrawTranscript(TranscriptHost host)
    {
        var say = services.Say;
        var resources = Application.Current.Resources;
        var panel = host.Panel;
        var id = host.Attachment.Id;
        // An own balloon is filled with the accent, so nothing in it may be drawn in the accent too.
        var actionInk = host.Mine ? Palette.Ink() : (Brush)resources["AccentTextFillColorPrimaryBrush"];
        Brush? quietInk = host.Mine ? Palette.Ink() : null;
        var look = connection.Transcripts.Look(id);
        panel.Children.Clear();
        switch (look.Phase)
        {
            case TranscriptPhase.Asking:
            {
                var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
                row.Children.Add(new ProgressRing { IsActive = true, Width = 14, Height = 14 });
                var line = new TextBlock { Text = say.Get("Getting the text…"), FontSize = 12, Opacity = 0.75, VerticalAlignment = VerticalAlignment.Center };
                if (quietInk is not null)
                {
                    line.Foreground = quietInk;
                }
                row.Children.Add(line);
                panel.Children.Add(row);
                break;
            }
            case TranscriptPhase.Open:
            {
                var words = new TextBlock
                {
                    Text = TranscriptRules.Shown(look.Text, say),
                    TextWrapping = TextWrapping.Wrap,
                    // The words of a recording, as words: selectable, and so copyable. Silence is a label, not text to copy.
                    IsTextSelectionEnabled = !look.NoSpeech,
                    FontStyle = look.NoSpeech ? Windows.UI.Text.FontStyle.Italic : Windows.UI.Text.FontStyle.Normal,
                };
                if (quietInk is not null)
                {
                    words.Foreground = quietInk;
                }
                // A screen reader hears what this block IS, so it is not read as the message itself.
                AutomationProperties.SetLocalizedControlType(words, say.Get("Text of the recording"));
                panel.Children.Add(words);
                panel.Children.Add(TranscriptAction(say.Get("Hide text"), actionInk, () => connection.Transcripts.Hide(id)));
                break;
            }
            case TranscriptPhase.Failed:
            {
                var error = look.Error ?? ApiError.Transport("no answer");
                var line = new TextBlock
                {
                    Text = TranscriptRules.FailureSentence(error, say),
                    FontSize = 12,
                    TextWrapping = TextWrapping.Wrap,
                    Opacity = 0.85,
                };
                if (quietInk is not null)
                {
                    line.Foreground = quietInk;
                }
                panel.Children.Add(line);
                if (TranscriptRules.MayRetry(error))
                {
                    panel.Children.Add(TranscriptAction(say.Get("Try Again"), actionInk, () => _ = AskTranscriptAsync(host)));
                }
                break;
            }
            default:
                panel.Children.Add(TranscriptAction(say.Get("Show text"), actionInk, () => _ = AskTranscriptAsync(host)));
                break;
        }
    }

    /// <summary>A small text button under the player, in the balloon's own ink.</summary>
    private static Button TranscriptAction(string label, Brush ink, Action click)
    {
        var button = new Button
        {
            Content = new TextBlock { Text = label, FontSize = 12, Foreground = ink },
            Padding = new Thickness(0, 2, 0, 2),
            MinHeight = 0,
            Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent),
            BorderThickness = new Thickness(0),
            HorizontalAlignment = HorizontalAlignment.Left,
        };
        AutomationProperties.SetName(button, label);
        button.Click += (_, _) => click();
        return button;
    }

    /// <summary>
    /// "Show text" pressed: the kept text at once, or the question when <c>/me</c> says this member has not agreed, then
    /// the request — and the question again, once, if the server says agree first (the board backdrop's order). The
    /// recording's sound is the ASKER's to send, so it is the asker who is asked.
    /// </summary>
    private async Task AskTranscriptAsync(TranscriptHost host)
    {
        try
        {
            var attachment = host.Attachment;
            // The device's own sound, made only after the consent question — and, for the stored form, only if the server
            // then says it cannot send its copy.
            var sound = ExtractsSound
                ? new SoundSource(
                    host.MaxBytes,
                    ct => SoundTracks.MakeAsync(connection.Attachments, attachment, host.MaxBytes, ct),
                    attachment.DurationMs)
                : null;
            await connection.Transcripts.ShowAsync(
                host.ChatId, host.MessageId, attachment.Id, host.Form, sound,
                () => BackdropConsent.AsksFirst(
                    connection.Session.State.Assistant?.Processor, connection.Session.State.AssistantConsentAt),
                AgreeToTheAssistantAsync);
        }
        catch (Exception e)
        {
            // The type only: never the text of anybody's recording.
            Diagnostics.Write($"asking for a recording's text: {e.GetType().Name}");
        }
    }

    /// <summary>
    /// The assistant question, asked for something other than a send — a recording's text — with the same disclosure the
    /// composer shows, the answer written to the server and <c>/me</c> read again. True only on a yes the server kept.
    /// </summary>
    private async Task<bool> AgreeToTheAssistantAsync()
    {
        var state = connection.Session.State;
        if (state.Assistant?.Processor is not { } processor || !AssistantConsent.IsAvailable(processor) || XamlRoot is null)
        {
            return false;
        }
        ConsentAnswer agreed;
        try
        {
            agreed = await Dialogs.AssistantConsentAsync(
                XamlRoot, services.Say, processor,
                state.Family?.AiHistory == true, state.Family?.AiVision == true, state.Assistant?.Transcribe == true,
                Lookups.Offered(state.Assistant) ? Lookups.Providers(state.Assistant) : null);
        }
        catch (Exception e)
        {
            // Another dialog already open: nothing was asked, so nothing is sent.
            Diagnostics.Write($"asking about the assistant: {e.GetType().Name}");
            return false;
        }
        if (agreed == ConsentAnswer.NotNow)
        {
            return false;
        }
        var error = await Lookups.RecordAsync(connection.Api, agreed, assistantAgreed: false);
        await connection.Session.RefreshAsync();
        DrawConsentBar();
        if (error is not null)
        {
            ShowProblem(services.Say.Get("Couldn't save your answer. Try again."));
        }
        // What was asked for is the recording's text, which needs the assistant's consent and nothing more.
        return !string.IsNullOrWhiteSpace(connection.Session.State.AssistantConsentAt);
    }

    private bool AudioRunning => audio.PlaybackSession.PlaybackState is Windows.Media.Playback.MediaPlaybackState.Playing
        or Windows.Media.Playback.MediaPlaybackState.Opening or Windows.Media.Playback.MediaPlaybackState.Buffering;

    private async Task ToggleAudioAsync(AttachmentDto attachment)
    {
        // No app sound while something records (S1.7): it would be in the recording.
        if (recordingStart.Quiet(recorder is not null))
        {
            Explain(services.Say.Get("You can play this after recording."));
            return;
        }
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
        // One at a time: whatever was playing stops, and says so — a staged or not-sent note too.
        fetchingAudio = attachment.Id;
        fetchingLocal++;
        audio.Pause();
        audioAtEnd = false;
        if (playingAudio is { } stopped)
        {
            playingAudio = null;
            ShowAudio(stopped);
        }
        ReleaseLocal();
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
        ShowLocal();
    }

    /// <summary>
    /// Played to the end: a bubble's recording stops there, showing its whole length, and Play starts it again; a staged or
    /// not-sent note goes back to its name.
    /// </summary>
    private void AudioEnded()
    {
        if (gone)
        {
            return;
        }
        audio.Pause();
        if (LocalActive)
        {
            ReleaseLocal();
            return;
        }
        audioAtEnd = true;
        ShowPlayback();
    }

    /// <summary>Leaving the chat, or the window: nothing keeps playing out of a bubble that is not there.</summary>
    private void StopAudio()
    {
        fetchingAudio = 0;
        fetchingLocal++;
        playingAudio = null;
        playingStaged = null;
        playingParked = null;
        audioAtEnd = false;
        scrubbingAudio = false;
        audio.Pause();
        (audio.Source as Windows.Media.Core.MediaSource)?.Dispose();
        audio.Source = null;
        audioRows.Clear();
        stagedRows.Clear();
        parkedRows.Clear();
    }

    /// <summary>A staged or not-sent note's row as drawn — its ▶, the line that becomes "0:12 / 0:42", its name and length — so playback can keep it up to date.</summary>
    private sealed record LocalRow(Button Toggle, FontIcon Glyph, TextBlock Line, string Idle, double Total);

    private bool LocalActive => playingStaged is not null || playingParked is not null;

    private LocalRow? ActiveLocalRow() =>
        playingStaged is { } staged && stagedRows.TryGetValue(staged, out var review) ? review
        : playingParked is { } id && parkedRows.TryGetValue(id, out var waiting) ? waiting
        : null;

    /// <summary>
    /// ▶ on a voice note in review (S2.7) or one that was not sent (S2.8): it plays the note from this device — the local
    /// bytes, the very ones a Send would upload — through the one player, so whatever else plays stops first; ❚❚ pauses it.
    /// Nothing plays while something records (S1.7).
    /// </summary>
    private async Task ToggleLocalAsync(StagedMedia? staged, ParkedRecording? entry)
    {
        if (recordingStart.Quiet(recorder is not null))
        {
            Explain(services.Say.Get("You can play this after recording."));
            return;
        }
        var same = staged is not null ? ReferenceEquals(playingStaged, staged) : entry is not null && playingParked == entry.Id;
        if (same)
        {
            if (AudioRunning)
            {
                audio.Pause();
            }
            else
            {
                audio.Play();
            }
            ShowLocal();
            return;
        }
        // One at a time: a bubble's recording, or another note, stops — and says so.
        fetchingAudio = 0;
        audio.Pause();
        audioAtEnd = false;
        if (playingAudio is { } stopped)
        {
            playingAudio = null;
            ShowAudio(stopped);
        }
        ReleaseLocal();
        var token = ++fetchingLocal;
        byte[]? bytes;
        try
        {
            bytes = staged is not null
                ? staged.Bytes.ToArray()
                : await Task.Run(() => parked.Staged(entry!)?.Bytes.ToArray());
        }
        catch (Exception e)
        {
            Diagnostics.Write($"reading a voice message to play: {e.GetType().Name}");
            bytes = null;
        }
        if (gone || token != fetchingLocal)
        {
            // Gone, or something else was pressed meanwhile: that one plays.
            return;
        }
        if (bytes is null)
        {
            Explain(services.Say.Get("Something went wrong. Try again."));
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
        if (gone || token != fetchingLocal || recordingStart.Quiet(recorder is not null))
        {
            stream.Dispose();
            return;
        }
        (audio.Source as Windows.Media.Core.MediaSource)?.Dispose();
        audio.Source = Windows.Media.Core.MediaSource.CreateFromStream(stream, VoiceNotes.Mime);
        playingStaged = staged;
        playingParked = staged is null ? entry?.Id : null;
        audio.Play();
        ShowLocal();
    }

    /// <summary>The playing note's row: ❚❚ or ▶, named for what a press does next, and "0:12 / 0:42" while it is the one.</summary>
    private void ShowLocal()
    {
        if (gone || ActiveLocalRow() is not { } row)
        {
            return;
        }
        var running = AudioRunning;
        // Pause and Play, in Segoe Fluent Icons.
        row.Glyph.Glyph = ((char)(running ? 0xE769 : 0xE768)).ToString();
        var name = running ? services.Say.Get("Pause") : services.Say.Get("Play");
        AutomationProperties.SetName(row.Toggle, name);
        ToolTipService.SetToolTip(row.Toggle, name);
        row.Line.Text = VoiceNotes.PlayingLabel(audio.PlaybackSession.Position.TotalSeconds, row.Total);
    }

    /// <summary>The note that was playing is not the one any more — ended, or something else pressed: its row goes back to ▶ and its name.</summary>
    private void ReleaseLocal()
    {
        var row = ActiveLocalRow();
        playingStaged = null;
        playingParked = null;
        if (row is null || gone)
        {
            return;
        }
        row.Glyph.Glyph = ((char)0xE768).ToString();
        AutomationProperties.SetName(row.Toggle, services.Say.Get("Play"));
        ToolTipService.SetToolTip(row.Toggle, services.Say.Get("Play"));
        row.Line.Text = row.Idle;
    }

    /// <summary>
    /// After the strip or the not-sent rows were drawn again: a note still there goes on showing that it plays; one that went
    /// — sent, deleted, kept as not sent, another chat opened — goes quiet with it.
    /// </summary>
    private void ReconcileLocal()
    {
        var drawn = playingStaged is { } staged ? stagedRows.ContainsKey(staged)
            : playingParked is { } id ? parkedRows.ContainsKey(id)
            : true;
        if (drawn)
        {
            ShowLocal();
            return;
        }
        fetchingLocal++;
        audio.Pause();
        playingStaged = null;
        playingParked = null;
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
        var ink = mine ? Palette.Ink() : (Brush)resources["AccentFillColorDefaultBrush"];
        var disc = new Grid { Width = 36, Height = 36, VerticalAlignment = VerticalAlignment.Center };
        disc.Children.Add(new Microsoft.UI.Xaml.Shapes.Ellipse { Fill = ink });
        // A map pin, in Segoe Fluent Icons.
        disc.Children.Add(new FontIcon
        {
            Glyph = ((char)0xE707).ToString(),
            FontSize = 16,
            Foreground = mine ? Palette.Surface(ActualTheme) : (Brush)resources["TextOnAccentFillColorPrimaryBrush"],
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

    private void ShowProblem(string sentence) => ShowProblem(sentence, null);

    /// <summary>
    /// A sentence on the composer's notice line, with a link after it that does something about it — Settings, for a
    /// refused microphone (S2.2). The link lives in the line itself, so the next sentence, which replaces the text,
    /// takes it away, and hiding the line hides it.
    /// </summary>
    private void ShowProblem(string sentence, (string Words, Action Act)? link)
    {
        ComposerError.Text = sentence;
        if (link is { } action)
        {
            var hyperlink = new Hyperlink();
            hyperlink.Inlines.Add(new Run { Text = action.Words });
            hyperlink.Click += (_, _) => action.Act();
            ComposerError.Inlines.Add(new Run { Text = " " });
            ComposerError.Inlines.Add(hyperlink);
        }
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
        DrawSlot();
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
        ShowAssistantButtons();
        ComposerBox.Text = message.Body;
        ComposerBox.SelectionStart = ComposerBox.Text.Length;
        ComposerBox.Focus(FocusState.Programmatic);
        // Save — never a microphone, even with the field cleared (S1.3 row 4) — whether or not the words changed.
        DrawSlot();
    }

    /// <summary>Back to writing a new message. An abandoned edit takes its words with it.</summary>
    private void EndComposerMode(bool clear)
    {
        replyingTo = null;
        editing = null;
        BannerPanel.Visibility = Visibility.Collapsed;
        ShowAssistantButtons();
        if (clear)
        {
            ComposerBox.Text = string.Empty;
        }
        DrawPictureNotice();
        DrawSlot();
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
    /// What KIND of chat this is, in the protocol's own words — "family", "direct", "ai" — or
    /// null while the list has not caught up with it. The assistant's rules are written against
    /// it (docs/protocol.md, "Consenting to the assistant").
    /// </summary>
    private string? Kind(ConversationModel chat) => connection.Chats.Chat(chat.ChatId)?.Chat.Kind;

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
        var ink = mine ? Palette.Ink() : (Brush)Application.Current.Resources["TextFillColorPrimaryBrush"];
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
        var ink = mine ? Palette.Ink() : (Brush)Application.Current.Resources["TextFillColorPrimaryBrush"];
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

    /// <summary>
    /// The link a bubble's card would describe, or null where it draws none: the first https link its body draws, except
    /// under an assistant answer still being written or one carrying the server's sources footer, whose links stay out of
    /// preview cards (<see cref="Lookups.MayPreview"/>; design decision 7 of docs/information-streams-2026-10-03.md). The
    /// one place both the drawing and its signature ask, so the two cannot disagree.
    /// </summary>
    private string? PreviewLinkOf(MessageDto message, string body, bool emojiOnly)
    {
        var assistantChat = connection.Chats.Chat(message.ChatId)?.Chat.Kind == "ai";
        return Lookups.MayPreview(
            message, body, Reader, assistantChat, connection.Session.State.Assistant?.UserId, connection.Answers.IsWriting(message))
            ? BubbleBody.PreviewLink(body, emojiOnly)
            : null;
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
            && PreviewLinkOf(bubble.Message, connection.Answers.BodyOf(bubble.Message), emojiOnly: false) is { } link
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
    private void OpenLinkSoon(Uri uri) => Soon(() => _ = Windows.System.Launcher.LaunchUriAsync(uri));

    /// <summary>
    /// Do what a click asked a beat late, unless the click was the first half of a double click — which is the heart, and
    /// calls it off (<see cref="CancelLinkForHeart"/>): a link opening, or a circle (S5.3: the single tap waits out the
    /// double-tap window).
    /// </summary>
    private void Soon(Action act)
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
            act();
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
        // A key held down is ONE press (S2.4): its first may have been Esc stopping a recording, Enter on the slot's Stop or
        // Send, or on the recording row's Stop or Delete — each of which hands focus to this field — and its repeats here must
        // neither drop the reply the note was recorded under nor send what that first press staged or uncovered.
        if (e.Key == Windows.System.VirtualKey.Escape && BannerPanel.Visibility == Visibility.Visible)
        {
            e.Handled = true;
            if (!slotGuard.IgnoresFieldKey(Environment.TickCount64, e.KeyStatus.WasKeyDown, sends: false, ComposerBox.Text))
            {
                EndComposerMode(clear: editing is not null);
            }
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
            // Enter is the slot's activation in rows 2 to 5 (S1.3), so it waits out the slot's guard as a click does: a second
            // Enter after the slot's Stop in row 3 must not send the words the note is being staged beside (S1.1). Words typed
            // since are never guarded, so "ok" sent at once still goes.
            if (!slotGuard.IgnoresFieldKey(Environment.TickCount64, e.KeyStatus.WasKeyDown, sends: true, ComposerBox.Text))
            {
                Send();
            }
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
            // The slot was Save and is the microphone now: a second click in the same breath must not record (S1.1).
            slotGuard.Arm(Environment.TickCount64, ComposerBox.Text);
            _ = ActAsync(() => chat.EditAsync(target.Id, words));
            return;
        }
        if (!hasWords && strip.Items.Count == 0)
        {
            return;
        }
        // NOTHING REACHES THE MODEL UNASKED. The server refuses this with
        // `assistant_consent_required` anyway; asking here is what turns that refusal into a
        // question with the message still in the box (docs/protocol.md, "Consenting to the
        // assistant").
        var session = connection.Session.State;
        var chatKind = Kind(chat);
        if (AssistantConsent.IsRequired(
            chatKind, ComposerBox.Text, session.Assistant?.Processor, session.AssistantConsentAt))
        {
            _ = ReviewAssistantConsentAsync(Send);
            return;
        }
        // And a server that will not say WHO answers gets nothing at all: there is no honest way
        // to ask, so there is nothing to send. The strip above the box says so.
        if (AssistantConsent.IsWithheldFromAnUnnamedAssistant(
            chatKind, ComposerBox.Text, session.Assistant is not null, session.Assistant?.Processor))
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
            // Its first steps empty the composer before it awaits anything: the slot is the microphone from here.
            _ = SendWithMediaAsync(chat, strip, body, replyTo);
            slotGuard.Arm(Environment.TickCount64, ComposerBox.Text);
            return;
        }
        // Written down first; the outbox owns it from here, and a send interrupted by anything at
        // all is a message that can be finished rather than one that never happened.
        // The names are resolved from the text at send: a name typed by hand mentions too.
        chat.Send(body, replyToMessageId: replyTo, mentions: ComposerMentions.ForSend(body, connection.Chats.Members(), IsFamily(chat)));
        drafts.Sent(chat.ChatId);
        EndComposerMode(clear: true);
        // A send that empties the composer turns Send into the microphone under the pointer: a double click on Send cannot
        // start a recording (S1.1). Typing never arms it, so "ok" followed at once by Send still sends.
        slotGuard.Arm(Environment.TickCount64, ComposerBox.Text);
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
            DrawSlot();
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
        // "Record Voice Message" (S1.5): in a family or a direct chat — never the assistant's, where every message is a
        // consented model call (decision 24). Off while an attachment is on its way, in an edit, during a call, and while
        // this chat holds a voice message that was not sent — its row is right there. With words typed or items staged it
        // records beside them, and the slot is Stop (S1.3 row 3).
        if (Kind(chat) != "ai")
        {
            var strip = Staging(chat.ChatId);
            var record = new MenuFlyoutItem
            {
                Text = say.Get("Record Voice Message"),
                Icon = new FontIcon { Glyph = ((char)ComposerButton.MicrophoneGlyph).ToString() },
                KeyboardAcceleratorTextOverride = ComposerButton.RecordShortcut,
                IsEnabled = !Busy(strip) && editing is null && !callBusy && !HasNotSent(chat.ChatId),
            };
            record.Click += (_, _) => _ = StartRecordingAsync(fromSlot: false);
            menu.Items.Add(record);
            // "Record Video Message" (S1.5), right below, where one can be recorded: off while an attachment is on its way,
            // in an edit and during a call. It works beside words or staged items — a video message travels alone, and they
            // stay in the composer.
            if (RoundAvailable())
            {
                var video = new MenuFlyoutItem
                {
                    Text = say.Get("Record Video Message"),
                    Icon = new FontIcon { Glyph = ((char)RoundVideoGlyph).ToString() },
                    IsEnabled = !Busy(strip) && editing is null && !callBusy && recorder is null,
                };
                video.Click += (_, _) => _ = OpenRoundRecorderAsync(AttachButton);
                menu.Items.Add(video);
            }
        }
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

    // ---- video messages, recorded -----------------------------------------------------------------
    //
    // Phase 3d (docs/audio-video-messages-2026-10-04.md, S1.4–S1.6, S3, S4, S8.6): the video button inside the empty field,
    // "Record Video Message" in the paperclip's menu and the microphone's, and the recorder they open over the window. All of
    // it is drawn only where RoundVideoRules.Available — this build records (RoundVideoRules.RecordingEnabled, off until the
    // owner's trials T1–T3), the server named the limits, and the machine has a camera — so a build that can only RECEIVE
    // circles shows no way of recording one (Decision 40).

    /// <summary>The video button's glyph: Segoe Fluent E714, a video camera (S1.4).</summary>
    private const int RoundVideoGlyph = 0xE714;

    /// <summary>
    /// Where the recorder lies — over the rail and the page, under the call card — and how the window under it is covered
    /// while it does; and what the window does once it has gone (a share that waited for it).
    /// </summary>
    internal void UseRecorderHost(Grid host, Action<bool> cover, Action closed)
    {
        recorderHost = host;
        coverWindow = cover;
        recorderClosed = closed;
        LearnCamera();
        DrawSlot();
    }

    /// <summary>The recorder is up: it owns the composer's row (S1.3 row 1), and a notification's chat waits for it.</summary>
    internal bool RecorderOpen => roundRecorder?.IsOpen == true;

    /// <summary>A take running or a clip in REVIEW: a real close asks "Delete video message?" first (S4, S8.6).</summary>
    internal bool HoldsRoundClip => roundRecorder?.HoldsClip == true;

    /// <summary>The ask before a real close (S4): true when the close may go ahead, false when the person kept the clip.</summary>
    internal Task<bool> AskBeforeClosingAsync() => roundRecorder?.AskBeforeClosingAsync() ?? Task.FromResult(true);

    /// <summary>The window lost focus and is still visible: PREVIEW closes — but not to a prompt it raised; a take goes on (S4).</summary>
    internal void WindowDeactivated() => roundRecorder?.Interrupt(RecorderInterruption.FocusLost);

    /// <summary>S1.2's <b>round available</b> on Windows, and somewhere for the recorder to lie.</summary>
    private bool RoundAvailable() =>
        recorderHost is not null && RoundVideoRules.Available(RoundVideoRules.RecordingEnabled, connection.Session.State.RoundVideo, hasCamera);

    /// <summary>
    /// Whether this machine has a camera — asked of Windows only where a video message could otherwise be recorded, so a build
    /// with recording switched off never enumerates a camera at all.
    /// </summary>
    private void LearnCamera(bool again = false)
    {
        if (askingForCamera || (hasCamera && !again) || recorderHost is null
            || !RoundVideoRules.Available(RoundVideoRules.RecordingEnabled, connection.Session.State.RoundVideo, hasCamera: true))
        {
            return;
        }
        askingForCamera = true;
        _ = LearnCameraAsync();
    }

    private async Task LearnCameraAsync()
    {
        try
        {
            var found = (await VideoMessageRecorder.CamerasAsync()).Count > 0;
            if (!gone && found != hasCamera)
            {
                hasCamera = found;
                DrawSlot();
            }
        }
        finally
        {
            askingForCamera = false;
        }
    }

    /// <summary>The video button as S1.4 has it for the open chat's composer now.</summary>
    private Door CurrentDoor()
    {
        if (open is not { } chat || SlotInputsNow() is not { } inputs)
        {
            return Door.Hidden;
        }
        return ComposerButton.VideoDoor(new DoorInputs(
            inputs,
            FamilyOrDirectChat: Kind(chat) is "family" or "direct",
            UndoWindow: false,
            ServerOffersRound: connection.Session.State.RoundVideo is not null,
            HasCamera: hasCamera,
            EncoderProbePasses: true,
            RecordsRoundVideo: RoundVideoRules.RecordingEnabled && recorderHost is not null));
    }

    /// <summary>
    /// The video button inside the empty field (S1.4): drawn — dimmed with the slot's sentence in rows 7 and 8 — or not, and
    /// the field given trailing room while it is, so a placeholder never runs under it. Its 600 ms guard starts when it appears.
    /// </summary>
    private void DrawVideoDoor()
    {
        var door = CurrentDoor();
        var showing = door.Kind != DoorKind.Hidden && ComposerBox.Visibility == Visibility.Visible;
        doorGuard.Showing(showing, Environment.TickCount64);
        var shown = showing ? Visibility.Visible : Visibility.Collapsed;
        if (VideoDoorButton.Visibility != shown)
        {
            VideoDoorButton.Visibility = shown;
        }
        // Room for it at the field's trailing edge while it shows; the style's own padding back when it goes.
        if (showing && !fieldPadded)
        {
            var padding = ComposerBox.Padding;
            ComposerBox.Padding = new Thickness(padding.Left, padding.Top, padding.Right + ComposerButton.MinTargetWindowsEpx, padding.Bottom);
            fieldPadded = true;
        }
        else if (!showing && fieldPadded)
        {
            ComposerBox.ClearValue(Control.PaddingProperty);
            fieldPadded = false;
        }
        if (!showing)
        {
            return;
        }
        var dimmed = door.Kind == DoorKind.Dimmed;
        VideoDoorButton.Opacity = dimmed ? 0.4 : 1;
        AutomationProperties.SetHelpText(VideoDoorButton, dimmed ? ComposerButton.Notice(door.Reason, services.Say) ?? string.Empty : string.Empty);
    }

    /// <summary>The video button clicked: nothing for 600 ms after it appeared; its sentence when dimmed; else the recorder.</summary>
    private void OnVideoDoorClick()
    {
        if (!doorGuard.Accepts(Environment.TickCount64))
        {
            return;
        }
        var door = CurrentDoor();
        if (door.Kind == DoorKind.Dimmed)
        {
            Explain(ComposerButton.Notice(door.Reason, services.Say)!);
            return;
        }
        if (door.Kind == DoorKind.Shown)
        {
            _ = OpenRoundRecorderAsync(VideoDoorButton);
        }
    }

    /// <summary>
    /// The recorder, over the window (S3): never in the assistant's chat, during a call, while an attachment is on its way, in
    /// an edit or beside a voice recording — those say why where they are reachable at all. Words typed and items staged stay
    /// in the composer; the primed reply goes with the video (S1.5).
    /// </summary>
    private async Task OpenRoundRecorderAsync(UIElement opener)
    {
        if (gone || open is not { } chat || RecorderOpen || recorderHost is not { } host || coverWindow is not { } cover
            || connection.Session.State.RoundVideo is not { } limits || !RoundAvailable() || Kind(chat) == "ai"
            || editing is not null || recorder is not null || recordingStart.Starting || asking is not null)
        {
            return;
        }
        if (callBusy || Busy(Staging(chat.ChatId)))
        {
            Explain(ComposerButton.Notice(callBusy ? Dimmed.Call : Dimmed.Busy, services.Say)!);
            return;
        }
        RoundRecorderLayer? layer = null;
        layer = new RoundRecorderLayer(
            services,
            host,
            ConversationPane,
            ComposerPanel,
            SendButton,
            DispatcherQueue,
            limits,
            new RoundRecorderLayer.Hooks(
                ReplyText: () => replyingTo is not null && BannerPanel.Visibility == Visibility.Visible ? BannerText.Text : null,
                DropReply: () => EndComposerMode(clear: false),
                NotSentWaits: () => open is { } here && HasNotSent(here.ChatId),
                QuietEverything: QuietForRecording,
                StartVoice: () => _ = StartRecordingAsync(fromSlot: false),
                SendAsync: (media, round) => SendVideoMessageAsync(chat, media, round),
                Cover: covered => cover(covered),
                Closed: () =>
                {
                    if (ReferenceEquals(roundRecorder, layer))
                    {
                        roundRecorder = null;
                    }
                    if (gone)
                    {
                        return;
                    }
                    DrawSlot();
                    recorderClosed?.Invoke();
                    if (chatAfterRecorder is { } waiting)
                    {
                        chatAfterRecorder = null;
                        OpenChat(waiting);
                    }
                }));
        roundRecorder = layer;
        DrawSlot();
        await layer.OpenAsync(opener);
    }

    /// <summary>
    /// A video message's Send (S3.4): the clip staged and queued as one message — with <c>round</c>, or as the regular video
    /// REVIEW said it would be (S3.6) — carrying the primed reply, the row written before the first byte moves, the bytes kept
    /// until the ack; the circle shows in the conversation at once from this device's own poster (S5.6). The words in the
    /// field stay where they are.
    /// </summary>
    private async Task SendVideoMessageAsync(ConversationModel chat, StagedMedia media, bool round)
    {
        var replyTo = replyingTo?.Id;
        var store = connection.Staging;
        string? handle = null;
        try
        {
            handle = await Task.Run(() => store.Stage(media));
            if (round)
            {
                chat.SendRound(handle, replyTo);
            }
            else
            {
                chat.Send(string.Empty, replyTo, pendingFiles: [handle]);
            }
            if (gone)
            {
                return;
            }
            if (open == chat)
            {
                if (replyTo is not null && replyingTo?.Id == replyTo)
                {
                    EndComposerMode(clear: false);
                }
                Queued();
            }
            else
            {
                _ = connection.Live.FlushAsync(SendRules.FlushTrigger.Queued);
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"sending a video message: {e.GetType().Name}");
            if (!gone)
            {
                ShowProblem(services.Say.Get("Something went wrong. Try again."));
            }
        }
        finally
        {
            if (handle is not null)
            {
                // The row that names it is in the outbox now (or never will be): the sweep may judge it.
                store.Release([handle]);
            }
        }
    }

    // ---- video messages ------------------------------------------------------------------------
    //
    // A VIDEO MESSAGE (docs/protocol.md, "Video messages"; docs/audio-video-messages-2026-10-04.md, S5): one square video
    // sent to be drawn round. In this version a click plays it in the viewer, inside a ring painted over its corners
    // (S5.3); playing inside the thread waits for the clipping trial (Blocked 1).

    /// <summary>
    /// A circle in a conversation (S5.2): NO BALLOON, the square poster filling an ellipse of the one size — a neutral
    /// disc of that size until the poster lands, so the row never changes height — a play disc in the middle, the
    /// duration in a capsule at the bottom and, until this device has played it, an accent dot beside it. Only the poster
    /// is fetched to draw it; the video itself only when it is opened. A click opens it a beat late, because a double
    /// click on it is the heart (S5.3).
    /// </summary>
    private FrameworkElement RoundVideoElement(AttachmentDto video, bool mine, Action heart)
    {
        var say = services.Say;
        var resources = Application.Current.Resources;
        const double Diameter = RoundLook.Diameter;
        var played = HasPlayed(video.Id);
        var white = new SolidColorBrush(Microsoft.UI.Colors.White);
        var face = new Grid { Width = Diameter, Height = Diameter };
        face.Children.Add(new Ellipse { Fill = (Brush)resources["ControlFillColorSecondaryBrush"] });
        var poster = new Ellipse();
        face.Children.Add(poster);
        var disc = new Grid
        {
            Width = RoundLook.PlayDisc,
            Height = RoundLook.PlayDisc,
            HorizontalAlignment = HorizontalAlignment.Center,
            VerticalAlignment = VerticalAlignment.Center,
        };
        disc.Children.Add(new Ellipse { Fill = new SolidColorBrush(Windows.UI.Color.FromArgb(140, 0, 0, 0)) });
        // Play, in Segoe Fluent Icons — a glyph, not a sentence; nudged right so the triangle looks centred.
        disc.Children.Add(new FontIcon
        {
            Glyph = ((char)0xE768).ToString(),
            FontSize = 18,
            Foreground = white,
            Margin = new Thickness(3, 0, 0, 0),
            HorizontalAlignment = HorizontalAlignment.Center,
            VerticalAlignment = VerticalAlignment.Center,
        });
        face.Children.Add(disc);
        var foot = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Spacing = 6,
            HorizontalAlignment = HorizontalAlignment.Center,
            VerticalAlignment = VerticalAlignment.Bottom,
            Margin = new Thickness(0, 0, 0, 18),
        };
        if (RoundLook.Capsule(video) is { } length)
        {
            foot.Children.Add(new Border
            {
                CornerRadius = new CornerRadius(10),
                Padding = new Thickness(8, 2, 8, 3),
                Background = new SolidColorBrush(Windows.UI.Color.FromArgb(153, 0, 0, 0)),
                Child = new TextBlock { Text = length, FontSize = 12, Foreground = white },
            });
        }
        if (RoundLook.ShowsDot(mine, played))
        {
            // THIS DEVICE's own knowledge: kept per account, never sent, wiped at sign-out (S5.2) — and never on the
            // reader's own circles.
            foot.Children.Add(new Ellipse
            {
                Width = RoundLook.Dot,
                Height = RoundLook.Dot,
                Fill = (Brush)resources["AccentFillColorDefaultBrush"],
                VerticalAlignment = VerticalAlignment.Center,
            });
        }
        if (foot.Children.Count > 0)
        {
            face.Children.Add(foot);
        }
        // A button, so Tab reaches it and Enter or Space opens it like a click; drawn as nothing but the circle.
        var circle = new Button
        {
            Content = face,
            Padding = new Thickness(0),
            BorderThickness = new Thickness(0),
            Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent),
            CornerRadius = new CornerRadius(Diameter / 2),
            HorizontalAlignment = mine ? HorizontalAlignment.Right : HorizontalAlignment.Left,
        };
        AutomationProperties.SetName(circle, RoundLook.Name(video, say));
        if (RoundLook.Value(mine, played, say) is { } status)
        {
            AutomationProperties.SetItemStatus(circle, status);
        }
        circle.Click += (_, _) => Soon(() => OpenRound(video));
        // The heart, here and handled: a button may keep the gesture from the balloon, and it must not land twice.
        circle.DoubleTapped += (_, e) =>
        {
            e.Handled = true;
            heart();
        };
        _ = ShowPosterAsync(poster, video);
        return circle;
    }

    /// <summary>The square poster, filling its ellipse — the same small copy the viewer shows while the video loads.</summary>
    private async Task ShowPosterAsync(Ellipse poster, AttachmentDto video)
    {
        if (!video.HasPreview)
        {
            // No poster was ever made: the neutral disc stays, and the video is never downloaded to draw one.
            return;
        }
        try
        {
            var key = AttachmentCache.KeyFor(video.Id, preview: true);
            if (!pictures.TryGetValue(key, out var picture))
            {
                var (bytes, error) = await connection.Attachments.BytesAsync(video, preview: true);
                if (gone)
                {
                    return;
                }
                if (bytes is null)
                {
                    if (error is not null)
                    {
                        Diagnostics.Write($"a video message's poster: {error.Code} {error.Status}");
                    }
                    return;
                }
                if (await DecodeAsync(bytes) is not { } decoded)
                {
                    return;
                }
                picture = pictures[key] = decoded;
            }
            poster.Fill = new ImageBrush { ImageSource = picture, Stretch = Stretch.UniformToFill };
        }
        catch (Exception e)
        {
            Diagnostics.Write($"drawing a video message: {e.GetType().Name}");
        }
    }

    /// <summary>What a visible circle fetches — its poster — fetched for a hidden one and drawn nowhere.</summary>
    private async Task FetchHiddenPosterAsync(AttachmentDto video)
    {
        try
        {
            if (video.HasPreview && !pictures.ContainsKey(AttachmentCache.KeyFor(video.Id, preview: true)))
            {
                await connection.Attachments.BytesAsync(video, preview: true);
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"fetching a hidden video message: {e.GetType().Name}");
        }
    }

    /// <summary>Whether this device has played it; a cache that cannot be read draws no dot rather than a wrong one.</summary>
    private bool HasPlayed(long attachmentId)
    {
        try
        {
            return playedRounds.Played(attachmentId);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"reading what was played: {e.GetType().Name}");
            return true;
        }
    }

    /// <summary>A circle, in the viewer: its circle, and its own bar under it.</summary>
    /// <remarks>Only ever handed what <see cref="MessageDto.RoundVideo"/> found — S5.1's test of the whole message.</remarks>
    private void OpenRound(AttachmentDto video) => OpenViewer([video], 0, round: true);

    /// <summary>
    /// The viewer shows a video message, or stops showing one: a SOLID backdrop, the player square and filled, the ring
    /// and its corner mask over it — painted in that same backdrop — and the bar under it in place of the player's own
    /// controls, which would sit in the corners the ring covers.
    /// </summary>
    private void SetViewerRound(bool on)
    {
        viewerRound = on;
        viewerRoundPlayed = false;
        viewerRoundStarted = false;
        viewerRoundFailed = false;
        viewerRoundClock?.Stop();
        ViewerRoundRetry.Visibility = Visibility.Collapsed;
        ViewerRoundBar.Visibility = Visibility.Collapsed;
        ViewerRoundFrame.Visibility = on ? Visibility.Visible : Visibility.Collapsed;
        ViewerVideo.AreTransportControlsEnabled = !on && recorder is null;
        if (on)
        {
            viewerBackdrop ??= ViewerOverlay.Background;
            var solid = new SolidColorBrush(Microsoft.UI.Colors.Black);
            ViewerOverlay.Background = solid;
            ViewerRoundMask.Fill = solid;
            ViewerVideo.Stretch = Stretch.UniformToFill;
            ViewerVideo.Margin = new Thickness(0);
            ViewerVideo.HorizontalAlignment = HorizontalAlignment.Center;
            ViewerVideo.VerticalAlignment = VerticalAlignment.Center;
            SizeViewerRound();
            return;
        }
        if (viewerBackdrop is { } before)
        {
            ViewerOverlay.Background = before;
            viewerBackdrop = null;
        }
        ViewerVideo.Stretch = Stretch.Uniform;
        ViewerVideo.Margin = new Thickness(24);
        ViewerVideo.Width = double.NaN;
        ViewerVideo.Height = double.NaN;
        ViewerVideo.HorizontalAlignment = HorizontalAlignment.Stretch;
        ViewerVideo.VerticalAlignment = VerticalAlignment.Stretch;
    }

    /// <summary>What the viewer's bars above and below the circle take of its height.</summary>
    private const double ViewerRoundChrome = 140;

    /// <summary>The circle at the size the viewer has room for (<see cref="RoundLook.ViewerDiameter"/>), its mask cut to it.</summary>
    private void SizeViewerRound()
    {
        var diameter = RoundLook.ViewerDiameter(ViewerOverlay.ActualWidth, ViewerOverlay.ActualHeight, ViewerRoundChrome);
        ViewerVideo.Width = diameter;
        ViewerVideo.Height = diameter;
        ViewerRoundFrame.Width = diameter;
        ViewerRoundFrame.Height = diameter;
        ViewerRoundRing.Width = diameter;
        ViewerRoundRing.Height = diameter;
        ViewerRoundRetryText.MaxWidth = Math.Max(120, diameter - 60);
        // Everything in the square that is not the circle: the corners, filled with the backdrop.
        var corners = new GeometryGroup { FillRule = FillRule.EvenOdd };
        corners.Children.Add(new RectangleGeometry { Rect = new Windows.Foundation.Rect(0, 0, diameter, diameter) });
        corners.Children.Add(new EllipseGeometry
        {
            Center = new Windows.Foundation.Point(diameter / 2, diameter / 2),
            RadiusX = diameter / 2,
            RadiusY = diameter / 2,
        });
        ViewerRoundMask.Data = corners;
    }

    /// <summary>The clip is in: its bar comes up, and a clock keeps it — and the played dot — up to date.</summary>
    private void ShowViewerRound()
    {
        if (viewerRoundFailed)
        {
            return;
        }
        ViewerRoundBar.Visibility = Visibility.Visible;
        if (viewerRoundClock is null)
        {
            viewerRoundClock = DispatcherQueue.CreateTimer();
            viewerRoundClock.Interval = TimeSpan.FromMilliseconds(250);
            viewerRoundClock.Tick += (_, _) => TickViewerRound();
        }
        viewerRoundClock.Start();
        TickViewerRound();
    }

    /// <summary>Where the circle is: the bar's position and time, Play or Pause — and, once played through here, no dot.</summary>
    private void TickViewerRound()
    {
        if (gone || !viewerRound || viewing is not { } album)
        {
            viewerRoundClock?.Stop();
            return;
        }
        if (ViewerVideo.MediaPlayer is not { } player)
        {
            return;
        }
        try
        {
            var session = player.PlaybackSession;
            var total = session.NaturalDuration.TotalSeconds;
            if (total <= 0)
            {
                total = (album.Current.DurationMs ?? 0) / 1000.0;
            }
            var at = Math.Clamp(session.Position.TotalSeconds, 0, Math.Max(total, 0));
            var playing = session.PlaybackState == Windows.Media.Playback.MediaPlaybackState.Playing;
            viewerRoundStarted |= playing;
            // Its dot goes at the END, not at the start (S5.3), as on every other client.
            if (!viewerRoundPlayed && RoundLook.PlayedThrough(viewerRoundStarted, at, total))
            {
                viewerRoundPlayed = true;
                try
                {
                    playedRounds.MarkPlayed(album.Current.Id);
                }
                catch (Exception e)
                {
                    Diagnostics.Write($"remembering what was played: {e.GetType().Name}");
                }
            }
            movingRoundSeek = true;
            try
            {
                ViewerRoundSeek.Maximum = Math.Max(total, 0.1);
                ViewerRoundSeek.Value = at;
            }
            finally
            {
                movingRoundSeek = false;
            }
            ViewerRoundTime.Text = $"{MediaText.TimeLabel(at)} / {MediaText.TimeLabel(total)}";
            ViewerRoundGlyph.Glyph = ((char)(playing ? 0xE769 : 0xE768)).ToString();
            var label = services.Say.Get(playing ? "Pause" : "Play");
            if (!string.Equals(AutomationProperties.GetName(ViewerRoundPlay), label, StringComparison.Ordinal))
            {
                AutomationProperties.SetName(ViewerRoundPlay, label);
                ToolTipService.SetToolTip(ViewerRoundPlay, label);
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"following a video message: {e.GetType().Name}");
        }
    }

    /// <summary>The circle's Play and Pause: played through, Play starts it again; and one thing plays at a time.</summary>
    private void ToggleViewerRound()
    {
        if (ViewerVideo.MediaPlayer is not { } player)
        {
            return;
        }
        // No app sound while something records (S1.7); the button is dimmed then too.
        if (recordingStart.Quiet(recorder is not null))
        {
            return;
        }
        try
        {
            var session = player.PlaybackSession;
            if (session.PlaybackState == Windows.Media.Playback.MediaPlaybackState.Playing)
            {
                player.Pause();
            }
            else
            {
                if (session.NaturalDuration > TimeSpan.Zero
                    && session.Position >= session.NaturalDuration - TimeSpan.FromMilliseconds(250))
                {
                    session.Position = TimeSpan.Zero;
                }
                if (AudioRunning)
                {
                    audio.Pause();
                    ShowPlayback();
                }
                player.Play();
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"playing a video message: {e.GetType().Name}");
        }
        TickViewerRound();
    }

    /// <summary>
    /// Something happened that pauses what plays (S4's last column; <see cref="PlaybackPauses"/>): the one recording
    /// player, and the viewer's video when the rule says so — a circle on a hidden window, but a voice note plays on.
    /// </summary>
    private void PausePlayback(PlaybackEvent happened)
    {
        try
        {
            if (PlaybackPauses.Pauses(happened, Playing.VoiceNote) && AudioRunning)
            {
                audio.Pause();
                ShowPlayback();
            }
            if (viewing is { IsVideo: true }
                && PlaybackPauses.Pauses(happened, viewerRound ? Playing.RoundVideo : Playing.Video))
            {
                ViewerVideo.MediaPlayer?.Pause();
            }
            // And the clip the recorder is playing back in REVIEW, which is a circle like any other.
            roundRecorder?.PausePlayback(happened);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"pausing for {happened}: {e.GetType().Name}");
        }
    }

    /// <summary>
    /// The reader's own circle on its way (S5.6): drawn round at once from the poster this device made, fainter, with a
    /// thin neutral ring while it goes up and "Sending…" — and, refused, the failed row's Try Again and Delete.
    /// </summary>
    private FrameworkElement PendingRoundElement(ConversationModel chat, OutboxRow row)
    {
        var say = services.Say;
        var resources = Application.Current.Resources;
        var column = new StackPanel
        {
            Spacing = 3,
            HorizontalAlignment = HorizontalAlignment.Right,
            Margin = new Thickness(72, 8, 0, 0),
        };
        var face = new Grid
        {
            Width = RoundLook.Diameter,
            Height = RoundLook.Diameter,
            HorizontalAlignment = HorizontalAlignment.Right,
            Opacity = row.Failed ? 0.9 : 0.6,
        };
        face.Children.Add(new Ellipse { Fill = (Brush)resources["ControlFillColorSecondaryBrush"] });
        var poster = new Ellipse();
        face.Children.Add(poster);
        face.Children.Add(new Ellipse
        {
            Stroke = (Brush)resources["ControlStrongStrokeColorDefaultBrush"],
            StrokeThickness = RoundLook.Ring,
        });
        AutomationProperties.SetName(face, say.Get("Video message"));
        if ((row.StagedFiles ?? row.PendingFiles) is [var handle, ..])
        {
            _ = ShowStagedPosterAsync(poster, handle);
        }
        column.Children.Add(face);
        column.Children.Add(PendingFoot(chat, row));
        return column;
    }

    /// <summary>The poster staged with a circle on its way — this device's own JPEG, never asked of the server.</summary>
    private async Task ShowStagedPosterAsync(Ellipse poster, string handle)
    {
        try
        {
            if (!stagedPosters.TryGetValue(handle, out var picture))
            {
                var store = connection.Staging;
                var staged = await Task.Run(() => store.Read(handle));
                if (gone || staged?.Preview is not { IsEmpty: false } jpeg)
                {
                    return;
                }
                picture = await DecodeAsync(jpeg.ToArray());
                if (gone)
                {
                    return;
                }
                if (stagedPosters.Count >= 16)
                {
                    stagedPosters.Clear();
                }
                // A null is kept as well: bytes nothing here decodes will not decode on the next redraw either.
                stagedPosters[handle] = picture;
            }
            if (picture is not null)
            {
                poster.Fill = new ImageBrush { ImageSource = picture, Stretch = Stretch.UniformToFill };
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"drawing a video message on its way: {e.GetType().Name}");
        }
    }

    /// <summary>
    /// Under a bare send on its way — a sticker, a circle — with no balloon to sit in: "Sending…" in the window's own caption
    /// colour, or, refused, the two things a person can do about it.
    /// </summary>
    private FrameworkElement PendingFoot(ConversationModel chat, OutboxRow row)
    {
        var say = services.Say;
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
            return actions;
        }
        return new TextBlock
        {
            Text = say.Get("Sending…"),
            FontSize = 11,
            HorizontalAlignment = HorizontalAlignment.Right,
            Margin = new Thickness(4, 0, 4, 0),
            // No balloon under it, so the window's own caption colour — not the white a balloon carries.
            Foreground = Palette.SecondaryText(),
        };
    }

    // ---- stickers ------------------------------------------------------------------------------
    //
    // The CHAT sticker: a small picture sent as its own message (docs/protocol.md, "Sticker pack"). Nothing here is
    // about the board, whose cards this code also calls stickers (StickerFace).

    /// <summary>The screen's own scale, for decoding a picture at the pixels it will be drawn with; 1 where it cannot be asked.</summary>
    private double RasterScale()
    {
        try
        {
            return XamlRoot?.RasterizationScale ?? 1;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"reading the screen's scale: {e.GetType().Name}");
            return 1;
        }
    }

    /// <summary>
    /// A sticker in a conversation: NO BUBBLE, in the one box every sticker on this client gets — fitted whole, never
    /// cropped, never at its own pixel size — and the shape taken from metadata, so the row does not jump when the
    /// picture lands. A click shows it larger.
    /// </summary>
    private FrameworkElement StickerElement(AttachmentDto picture)
    {
        var say = services.Say;
        var (width, height) = StickerLook.Fit(picture.Width, picture.Height);
        var frame = new Grid
        {
            Width = width,
            Height = height,
            // Transparent, not absent: the whole box answers a click, not just the pixels that happen to be opaque.
            Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent),
        };
        // Until the bytes land — and for good where nothing on this machine decodes them — the word says what is here.
        var word = new TextBlock
        {
            Text = say.Get("Sticker"),
            Opacity = 0.6,
            HorizontalAlignment = HorizontalAlignment.Center,
            VerticalAlignment = VerticalAlignment.Center,
        };
        frame.Children.Add(word);
        var image = new Image { Stretch = Stretch.Uniform };
        frame.Children.Add(image);
        AutomationProperties.SetName(frame, say.Get("Sticker"));
        _ = ShowStickerAsync(image, word, picture);
        IReadOnlyList<AttachmentDto> album = [picture];
        frame.Tapped += (_, _) => OpenViewer(album, 0);
        return frame;
    }

    /// <summary>
    /// A sent sticker's picture: its ORIGINAL bytes, never the preview, whatever <c>has_preview</c> says — a preview is a
    /// JPEG, and the flag can be inherited through dedup. Animated where this machine can, frame zero where it cannot.
    /// </summary>
    private async Task ShowStickerAsync(Image image, TextBlock word, AttachmentDto picture)
    {
        try
        {
            if (!stickerPictures.TryGet(picture.Id, out var decoded))
            {
                var (bytes, error) = await connection.Attachments.BytesAsync(picture, preview: false);
                if (gone)
                {
                    return;
                }
                if (bytes is null)
                {
                    // NOT remembered: the bytes may well arrive next time, and then there is a sticker to draw.
                    if (error is not null)
                    {
                        Diagnostics.Write($"a sticker: {error.Code} {error.Status}");
                    }
                    return;
                }
                await stickerDecoding.WaitAsync();
                try
                {
                    if (gone)
                    {
                        return;
                    }
                    // Asked again: another element of this same sticker may have decoded it while this one waited.
                    if (!stickerPictures.TryGet(picture.Id, out decoded))
                    {
                        decoded = await StickerImaging.DecodeAsync(bytes, StickerLook.Box, RasterScale(), StickerImaging.AnimationsWanted());
                        if (gone)
                        {
                            decoded?.Release();
                            return;
                        }
                        // Frames are memory, and the shelf holds every sticker here to one budget: what was drawn longest
                        // ago makes room for this one. A null is kept too — "nothing on this machine decodes it" is an
                        // answer, and asking again on every redraw would read the file and fail the decode every time.
                        stickerPictures.Put(picture.Id, decoded, decoded?.Bytes ?? 0);
                    }
                }
                finally
                {
                    stickerDecoding.Release();
                }
            }
            if (decoded is null)
            {
                // The word stays: it is what says a sticker is here.
                return;
            }
            // The clock is the STICKER's: an image made by a redraw joins the animation where it was.
            stickers.Show(image, decoded, stickerPictures.Started(picture.Id, stickers.Now));
            word.Visibility = Visibility.Collapsed;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"drawing a sticker: {e.GetType().Name}");
        }
    }

    /// <summary>What a visible sticker fetches, fetched for a hidden one and drawn nowhere.</summary>
    private async Task FetchHiddenStickerAsync(AttachmentDto picture)
    {
        try
        {
            if (!stickerPictures.TryGet(picture.Id, out _))
            {
                await connection.Attachments.BytesAsync(picture, preview: false);
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"fetching a hidden sticker: {e.GetType().Name}");
        }
    }

    /// <summary>
    /// A sticker on its way: drawn as the sticker it will be, fainter, from the bytes staged for it — so one click in
    /// the panel puts it in the conversation at once, whatever the network is doing — and, refused, with the two things
    /// a person can do about it.
    /// </summary>
    private FrameworkElement PendingStickerElement(ConversationModel chat, OutboxRow row)
    {
        var say = services.Say;
        var column = new StackPanel
        {
            Spacing = 3,
            HorizontalAlignment = HorizontalAlignment.Right,
            Margin = new Thickness(72, 8, 0, 0),
        };
        var frame = new Grid
        {
            Width = StickerLook.Box,
            Height = StickerLook.Box,
            HorizontalAlignment = HorizontalAlignment.Right,
            Opacity = row.Failed ? 0.9 : 0.6,
        };
        var word = new TextBlock
        {
            Text = say.Get("Sticker"),
            Opacity = 0.6,
            HorizontalAlignment = HorizontalAlignment.Center,
            VerticalAlignment = VerticalAlignment.Center,
        };
        frame.Children.Add(word);
        var image = new Image { Stretch = Stretch.Uniform, HorizontalAlignment = HorizontalAlignment.Right };
        frame.Children.Add(image);
        AutomationProperties.SetName(frame, say.Get("Sticker"));
        if ((row.StagedFiles ?? row.PendingFiles) is [var handle, ..])
        {
            _ = ShowStagedStickerAsync(image, word, handle);
        }
        column.Children.Add(frame);
        column.Children.Add(PendingFoot(chat, row));
        return column;
    }

    private async Task ShowStagedStickerAsync(Image image, TextBlock word, string handle)
    {
        try
        {
            if (!stagedStickers.TryGetValue(handle, out var decoded))
            {
                var store = connection.Staging;
                var staged = await Task.Run(() => store.Read(handle));
                if (staged is null || gone)
                {
                    return;
                }
                // Still: it is on screen for as long as a send takes, and the message that replaces it moves.
                decoded = await StickerImaging.DecodeAsync(staged.Bytes.ToArray(), StickerLook.Box, RasterScale(), animate: false);
                if (gone)
                {
                    return;
                }
                if (stagedStickers.Count >= 16)
                {
                    stagedStickers.Clear();
                }
                // A null is kept as well: bytes nothing here decodes will not decode on the next redraw either.
                stagedStickers[handle] = decoded;
            }
            if (decoded is null)
            {
                return;
            }
            stickers.Show(image, decoded);
            word.Visibility = Visibility.Collapsed;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"drawing a sticker on its way: {e.GetType().Name}");
        }
    }

    /// <summary>
    /// The panel over the composer: what this device sent lately, then the family's whole pack in the order it was
    /// added — and ONE CLICK SENDS. No caption and no confirmation: the sticker is the message.
    /// </summary>
    /// <param name="anchor">The button that opened it: the conversation's, or the thread's.</param>
    /// <param name="inThread">The chain whose composer asked, or null for the conversation's own. A sticker sent from it answers the chain's ROOT.</param>
    private void ShowStickerPanel(FrameworkElement anchor, ThreadModel? inThread)
    {
        if (open is not { } chat || !pack.Offered)
        {
            return;
        }
        if (inThread is null ? editing is not null : thread != inThread || inThread.ChatId != chat.ChatId || !ThreadComposer.IsEnabled)
        {
            return;
        }
        try
        {
            var say = services.Say;
            var panel = pack.Panel();
            var flyout = new Flyout { Placement = Microsoft.UI.Xaml.Controls.Primitives.FlyoutPlacementMode.TopEdgeAlignedLeft };
            var content = new StackPanel { Spacing = 8, Width = StickerLook.PanelColumns * StickerLook.PanelCell };
            content.Children.Add(new TextBlock { Text = say.Get("Stickers"), FontWeight = FontWeights.SemiBold });
            if (panel.IsEmpty)
            {
                content.Children.Add(new TextBlock { Text = say.Get("No stickers yet"), TextWrapping = TextWrapping.Wrap });
                // Where they come from: the pack is managed on the Family screen, by anybody in the family.
                content.Children.Add(new TextBlock
                {
                    Text = say.Get("Add a picture and everyone in the family can send it as a sticker."),
                    TextWrapping = TextWrapping.Wrap,
                    Foreground = Palette.SecondaryText(),
                });
            }
            else
            {
                if (panel.Recent.Count > 0)
                {
                    // What THIS DEVICE sent lately, first. Never on the wire, and gone with this device's cache.
                    content.Children.Add(PanelHeading(say.Get("Recently used")));
                    content.Children.Add(StickerCells(chat, panel.Recent, flyout, inThread));
                    content.Children.Add(new Border
                    {
                        Height = 1,
                        Background = Palette.Themed("DividerStrokeColorDefaultBrush", 0x30, 0x80, 0x80, 0x80),
                    });
                    content.Children.Add(PanelHeading(say.Get("All stickers")));
                }
                content.Children.Add(StickerCells(chat, panel.All, flyout, inThread));
            }
            flyout.Content = new ScrollViewer
            {
                Content = content,
                MaxHeight = 380,
                HorizontalScrollMode = ScrollMode.Disabled,
                HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
                VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            };
            flyout.ShowAt(anchor);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"opening the sticker panel: {e.GetType().Name}");
            ShowProblem(services.Say.Get("Something went wrong. Try again."));
        }
    }

    private static TextBlock PanelHeading(string words) => new()
    {
        Text = words,
        FontSize = 12,
        Foreground = Palette.SecondaryText(),
    };

    /// <summary>One grid of the panel: a cell per item, each a button a screen reader and the keyboard can reach.</summary>
    private FrameworkElement StickerCells(ConversationModel chat, IReadOnlyList<PackItemDto> items, Flyout flyout, ThreadModel? inThread)
    {
        var say = services.Say;
        var cells = new VariableSizedWrapGrid
        {
            Orientation = Orientation.Horizontal,
            MaximumRowsOrColumns = StickerLook.PanelColumns,
            ItemWidth = StickerLook.PanelCell,
            ItemHeight = StickerLook.PanelCell,
        };
        foreach (var item in items)
        {
            // Still in the panel, whatever the file is: two hundred cells each cycling frames is a panel nobody can read.
            var image = new Image { Stretch = Stretch.Uniform };
            // ONE CLICK SENDS, so a cell may never be a blank square: until its picture lands — and for good where
            // nothing on this machine decodes it (WebP without its extension) — it says what it would send, in the
            // words whoever added it gave, or "Sticker". Under the picture, never over it.
            var word = new TextBlock
            {
                Text = item.Label is { Length: > 0 } given ? given : say.Get("Sticker"),
                FontSize = 11,
                Opacity = 0.6,
                TextAlignment = TextAlignment.Center,
                TextWrapping = TextWrapping.Wrap,
                TextTrimming = TextTrimming.CharacterEllipsis,
                MaxLines = 3,
                HorizontalAlignment = HorizontalAlignment.Center,
                VerticalAlignment = VerticalAlignment.Center,
            };
            var face = new Grid();
            face.Children.Add(word);
            face.Children.Add(image);
            var cell = new Button
            {
                Content = face,
                Width = StickerLook.PanelCell,
                Height = StickerLook.PanelCell,
                Padding = new Thickness(6),
                BorderThickness = new Thickness(0),
                Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent),
                HorizontalContentAlignment = HorizontalAlignment.Center,
                VerticalContentAlignment = VerticalAlignment.Center,
            };
            // The few words whoever added it gave — for a screen reader, whether or not the picture drew.
            var name = PackText.Name(item, say);
            AutomationProperties.SetName(cell, name);
            // One click sends, with no second step to catch a slip: a screen reader is told so before the click.
            AutomationProperties.SetHelpText(cell, say.Get("Sends this sticker"));
            if (item.Label is { Length: > 0 } label)
            {
                ToolTipService.SetToolTip(cell, label);
            }
            var chosen = item;
            cell.Click += (_, _) =>
            {
                flyout.Hide();
                _ = SendStickerAsync(chat, chosen, inThread);
            };
            _ = ShowStickerThumbAsync(image, word, item);
            cells.Children.Add(cell);
        }
        return cells;
    }

    /// <summary>
    /// A pack item in a panel cell: the item's ORIGINAL bytes, fetched once and kept under its attachment id. The cell's
    /// word goes only when there is a picture to put in its place.
    /// </summary>
    private async Task ShowStickerThumbAsync(Image image, TextBlock word, PackItemDto item)
    {
        try
        {
            if (!stickerThumbs.TryGetValue(item.Id, out var decoded))
            {
                var (bytes, error) = await pack.BytesAsync(item);
                if (gone)
                {
                    return;
                }
                if (bytes is null)
                {
                    if (error is not null)
                    {
                        Diagnostics.Write($"a pack item: {error.Code} {error.Status}");
                    }
                    return;
                }
                decoded = await StickerImaging.DecodeAsync(bytes, StickerLook.PanelCell, RasterScale(), animate: false);
                if (gone)
                {
                    return;
                }
                // A null is kept: the panel is opened again and again, and a codec that is not there is still not there.
                stickerThumbs[item.Id] = decoded;
            }
            if (decoded is null)
            {
                return;
            }
            stickers.Show(image, decoded);
            word.Visibility = Visibility.Collapsed;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"drawing a pack item: {e.GetType().Name}");
        }
    }

    /// <summary>
    /// One click in the panel: the item's bytes AS CACHED, staged as one photo with no preview, and a row queued with
    /// the sticker flag — a message from there on, so it is written down before anything moves and lands whenever the
    /// network lets it (docs/protocol.md, "Sending one"). The words in the box are left alone: a sticker carries none.
    /// </summary>
    /// <remarks>
    /// <para>
    /// NEVER THROUGH <see cref="MediaPreparing"/>. Everything that does to a photograph — the downscale, the JPEG, the
    /// preview — would cost a sticker its transparency and its animation.
    /// </para>
    /// <para>
    /// NOTHING REACHES THE MODEL UNASKED, and one click is no exception. In the assistant's chat the sticker goes through
    /// the same question the Send button does (docs/protocol.md, "Consenting to the assistant"): asked first, sent only
    /// on a yes the server has recorded, and not sent at all to an assistant whose owner this server will not name.
    /// </para>
    /// </remarks>
    /// <param name="inThread">The chain whose composer it was sent from: the sticker answers that chain's ROOT, as every reply from there does.</param>
    private async Task SendStickerAsync(ConversationModel chat, PackItemDto item, ThreadModel? inThread = null)
    {
        if (sendingSticker)
        {
            return;
        }
        var say = services.Say;
        var session = connection.Session.State;
        switch (PackSending.For(Kind(chat), session.Assistant is not null, session.Assistant?.Processor, session.AssistantConsentAt))
        {
            case PackSending.Gate.Ask:
                _ = ReviewAssistantConsentAsync(() =>
                {
                    // A yes is for the chat it was asked in: somebody who walked away while reading has not sent.
                    if (!gone && open == chat)
                    {
                        _ = SendStickerAsync(chat, item, inThread);
                    }
                });
                return;
            case PackSending.Gate.Withheld:
                ShowProblem(say.Get("This server hasn't said which service answers, so nothing can be sent to the assistant here."));
                return;
        }
        sendingSticker = true;
        ComposerError.Visibility = Visibility.Collapsed;
        // A sticker may be a reply — which is how one answers something with a sticker. Taken now: the banner is the
        // reader's to cancel while the bytes are fetched. From a thread it answers the ROOT, whatever was being looked at.
        var replyTo = inThread is not null ? inThread.RootId : replyingTo?.Id;
        var store = connection.Staging;
        string? handle = null;
        try
        {
            var (media, error) = await pack.ToSendAsync(item);
            if (media is null)
            {
                if (!gone)
                {
                    var refused = error ?? ApiError.Transport("no answer");
                    // A sticker is a COPY of the item's bytes, and this device has not fetched them yet.
                    ShowProblem(refused.Transient
                        ? say.Get("Couldn't send that sticker. Check your connection and try again.")
                        : PackText.Sentence(refused, say));
                }
                return;
            }
            handle = await Task.Run(() => store.Stage(media));
            chat.SendSticker(handle, replyTo);
            pack.Sent(item.Id);
            if (gone)
            {
                return;
            }
            if (open == chat)
            {
                if (inThread is not null)
                {
                    // Drawn under the chain it answers, as a reply typed there is (SendInThread).
                    if (thread == inThread)
                    {
                        threadDrawn = string.Empty;
                        DrawThread(scrollToEnd: true);
                    }
                }
                else if (replyTo is not null && replyingTo?.Id == replyTo)
                {
                    // The reply has been sent; what is typed in the box stays, for the message it belongs to.
                    EndComposerMode(clear: false);
                }
                Queued();
            }
            else
            {
                _ = connection.Live.FlushAsync(SendRules.FlushTrigger.Queued);
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"sending a sticker: {e.GetType().Name}");
            if (!gone)
            {
                ShowProblem(say.Get("Something went wrong. Try again."));
            }
        }
        finally
        {
            if (handle is not null)
            {
                // The row that names it is in the outbox now (or never will be): the sweep may judge it.
                store.Release([handle]);
            }
            sendingSticker = false;
        }
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
        DrawPictureHint();
        DrawConsentBar();
    }

    /// <summary>
    /// The line above the box while the assistant question is unanswered — and the line where it
    /// cannot be asked at all, because this server named nobody (docs/protocol.md, "Consenting to
    /// the assistant").
    /// </summary>
    /// <remarks>
    /// Drawn from the same place the picture notice is, and for the same reason: it has to be
    /// there on the keystroke that makes the draft one which would travel — in the assistant's
    /// own chat from the first character, in the family chat the moment <c>@ai</c> is typed.
    /// </remarks>
    private void DrawConsentBar()
    {
        var say = services.Say;
        var state = connection.Session.State;
        var kind = open is { } chat ? Kind(chat) : null;
        var processor = state.Assistant?.Processor;
        var needed = editing is null && AssistantConsent.IsRequired(
            kind, ComposerBox.Text, processor, state.AssistantConsentAt);
        var unnamed = editing is null && AssistantConsent.IsWithheldFromAnUnnamedAssistant(
            kind, ComposerBox.Text, state.Assistant is not null, processor);
        if (needed && processor is { } named)
        {
            ConsentText.Text = say.Format("This goes to %@. You haven't agreed to that yet.", named);
            ConsentReview.Content = say.Get("Review…");
            ConsentReview.Visibility = Visibility.Visible;
            ConsentBar.Visibility = Visibility.Visible;
        }
        else if (unnamed)
        {
            ConsentText.Text = say.Get("This server hasn't said which service answers, so nothing can be sent to the assistant here.");
            ConsentReview.Visibility = Visibility.Collapsed;
            ConsentBar.Visibility = Visibility.Visible;
        }
        else
        {
            ConsentBar.Visibility = Visibility.Collapsed;
        }
    }

    /// <summary>
    /// Ask the assistant question, and finish the send it interrupted. The draft never left the
    /// box, so agreeing completes what the person already asked for; "Not Now" leaves it there.
    /// </summary>
    /// <param name="then">
    /// The send that was interrupted, run only on a yes the server has recorded: the composer's own, or the sticker
    /// somebody clicked in the panel.
    /// </param>
    private async Task ReviewAssistantConsentAsync(Action then)
    {
        var state = connection.Session.State;
        if (state.Assistant?.Processor is not { } processor || string.IsNullOrWhiteSpace(processor))
        {
            return;
        }
        var agreed = await Dialogs.AssistantConsentAsync(
            XamlRoot, services.Say, processor,
            state.Family?.AiHistory == true, state.Family?.AiVision == true, state.Assistant?.Transcribe == true,
            Lookups.Offered(state.Assistant) ? Lookups.Providers(state.Assistant) : null);
        if (agreed == ConsentAnswer.NotNow)
        {
            return;
        }
        var error = await Lookups.RecordAsync(connection.Api, agreed, assistantAgreed: false);
        // The session is the state: `/me` is what the composer reads, so it is re-read rather
        // than patched here, and the strip and the send follow from what the server says — also
        // after a failed lookup write, when the assistant's own consent may already have landed.
        await connection.Session.RefreshAsync();
        DrawConsentBar();
        if (error is null)
        {
            then();
        }
        else
        {
            ShowProblem(services.Say.Get("Couldn't save your answer. Try again."));
        }
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
        ComposerError.Visibility = Visibility.Collapsed;
        var stagedBefore = strip.Items.Count;
        string? said;
        try
        {
            // The strip is marked as preparing before its first await, so the bar can be drawn from it straight away.
            var ingest = strip.IngestAsync(files, MediaPreparing.PrepareAsync, () => !gone, services.Say);
            ShowPreparing();
            said = await ingest;
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
        ShowPreparing();
        ComposerError.Visibility = Visibility.Collapsed;
        // By its id: a chat left and come back to while a video was transcoded is a new model over the same strip.
        if (open?.ChatId == chat.ChatId)
        {
            if (strip.Items.Count > stagedBefore)
            {
                // The person's staging is never guarded (S1.1): a picture pasted just after a send goes at the next Send.
                slotGuard.Changed(recording: recorder is not null);
            }
            if (said is not null)
            {
                ShowProblem(said);
            }
            DrawStaging();
        }
    }

    /// <summary>What the open chat has staged, each with its own ✕.</summary>
    /// <summary>
    /// "Preparing…" and its Cancel, for the chat that is OPEN: a batch goes on for the chat it was dropped into whichever
    /// chat is looked at meanwhile, so the bar follows the open chat's own strip rather than whoever started last.
    /// </summary>
    private void ShowPreparing()
    {
        var preparing = open is { } chat && strips.TryGetValue(chat.ChatId, out var strip) && strip.Preparing;
        PreparingRing.IsActive = preparing;
        PreparingBar.Visibility = preparing ? Visibility.Visible : Visibility.Collapsed;
        // An attachment on its way dims the microphone (S1.3 row 8).
        DrawSlot();
    }

    private void DrawStaging()
    {
        ShowPreparing();
        DrawPictureNotice();
        StagingStrip.Children.Clear();
        stagedRows.Clear();
        if (open is not { } chat || Staging(chat.ChatId) is not { Items.Count: > 0 } strip)
        {
            StagingScroller.Visibility = Visibility.Collapsed;
            ReconcileLocal();
            DrawSlot();
            return;
        }
        var say = services.Say;
        for (var index = 0; index < strip.Items.Count; index++)
        {
            var item = strip.Items[index];
            if (VoiceNotes.IsRecorded(item))
            {
                StagingStrip.Children.Add(ChipBox(StagedNoteChip(strip, item)));
                continue;
            }
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
                // Taken off by the person: a change of theirs, never guarded (S1.1, fc_text::record's OtherAction).
                slotGuard.Changed(recording: recorder is not null);
                DrawStaging();
            };
            row.Children.Add(remove);
            StagingStrip.Children.Add(ChipBox(row));
        }
        StagingScroller.Visibility = Visibility.Visible;
        ReconcileLocal();
        DrawSlot();
    }

    private static Border ChipBox(FrameworkElement row) => new()
    {
        Child = row,
        Padding = new Thickness(6, 4, 4, 4),
        CornerRadius = new CornerRadius(8),
        Background = (Brush)Application.Current.Resources["CardBackgroundFillColorDefaultBrush"],
    };

    /// <summary>
    /// A voice note in review (S2.7): "[▶] Voice message · 0:42 [✕]" — ▶ plays it from this device, "[❚❚] 0:12 / 0:42" while
    /// it does, and ✕ ("Delete recording") deletes it, asking first from ten seconds, because it cannot be made again.
    /// </summary>
    private FrameworkElement StagedNoteChip(ComposerStaging strip, StagedMedia item)
    {
        var say = services.Say;
        var idle = ComposerStaging.Label(item, say, services.Culture);
        var (toggle, glyph) = PlayButton();
        var line = new TextBlock
        {
            Text = idle,
            VerticalAlignment = VerticalAlignment.Center,
            MaxWidth = 220,
            TextTrimming = TextTrimming.CharacterEllipsis,
        };
        Typography.SetNumeralAlignment(line, FontNumeralAlignment.Tabular);
        var remove = new Button { Content = "✕", Padding = new Thickness(6, 2, 6, 2), VerticalAlignment = VerticalAlignment.Center };
        ToolTipService.SetToolTip(remove, say.Get("Delete recording"));
        AutomationProperties.SetName(remove, say.Get("Delete recording"));
        toggle.Click += (_, _) => _ = ToggleLocalAsync(item, null);
        remove.Click += (_, _) => _ = DeleteStagedNoteAsync(strip, item);
        stagedRows[item] = new LocalRow(toggle, glyph, line, idle, VoiceNotes.TotalSeconds(item.DurationMs));
        DimPlay(toggle);
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
        row.Children.Add(toggle);
        row.Children.Add(line);
        row.Children.Add(remove);
        return row;
    }

    /// <summary>
    /// A small ▶ that becomes ❚❚ while its note plays, named for what it does next — in a 44-epx target that reaches past
    /// the row it sits in rather than making it taller (S1.1: the hit area grows, the visual does not).
    /// </summary>
    private (Button Toggle, FontIcon Glyph) PlayButton()
    {
        var say = services.Say;
        // Play, in Segoe Fluent Icons.
        var glyph = new FontIcon { Glyph = ((char)0xE768).ToString(), FontSize = 14 };
        var toggle = new Button
        {
            Content = glyph,
            Width = ComposerButton.MinTargetWindowsEpx,
            Height = ComposerButton.MinTargetWindowsEpx,
            Margin = new Thickness(-6),
            Padding = new Thickness(0),
            CornerRadius = new CornerRadius(ComposerButton.MinTargetWindowsEpx / 2.0),
            Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent),
            BorderThickness = new Thickness(0),
            VerticalAlignment = VerticalAlignment.Center,
        };
        ToolTipService.SetToolTip(toggle, say.Get("Play"));
        AutomationProperties.SetName(toggle, say.Get("Play"));
        return (toggle, glyph);
    }

    /// <summary>A note in review's ✕: gone — after "Delete this recording?" from ten seconds — and said (S2.7, S6).</summary>
    private async Task DeleteStagedNoteAsync(ComposerStaging strip, StagedMedia item)
    {
        if (VoiceNotes.AsksBeforeDeleting(item.DurationMs ?? 0))
        {
            ContentDialogResult answer;
            try
            {
                answer = await Dialogs.DeleteRecording(XamlRoot, services.Say).ShowAsync();
            }
            catch (Exception e)
            {
                // Only one dialog may be up at a time: nothing was asked, so nothing is deleted.
                Diagnostics.Write($"asking before deleting a recording: {e.GetType().Name}");
                return;
            }
            if (answer != ContentDialogResult.Primary || gone)
            {
                return;
            }
        }
        // By reference: it may have been sent, kept as not sent or moved meanwhile, and then this deletes nothing.
        if (!strip.Remove(item))
        {
            return;
        }
        // Taken off by the person, as any staged item's ✕ (S1.1): the guard lifts.
        slotGuard.Changed(recording: recorder is not null);
        DrawStaging();
        ComposerBox.Focus(FocusState.Programmatic);
        SayAloud(NotSent.Said(RecordingEnd.Deleted, RecordingFate.Discard, item.DurationMs ?? 0, services.Say)!);
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
