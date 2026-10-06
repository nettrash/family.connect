using System.Numerics;
using System.Runtime.InteropServices.WindowsRuntime;
using FamilyConnect.App.Logic;
using FamilyConnect.App.Services;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using Microsoft.UI;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Hosting;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Microsoft.UI.Xaml.Shapes;
using Windows.Foundation;
using Windows.Media.Core;
using Windows.Media.Playback;
using Windows.Storage;
using Windows.Storage.Streams;
using VirtualKey = Windows.System.VirtualKey;
using Launcher = Windows.System.Launcher;
using Windows.UI.ViewManagement;

namespace FamilyConnect.App.Views;

/// <summary>
/// The video-message recorder on Windows (docs/audio-video-messages-2026-10-04.md, S3.3, S3.4, S8.6): a layer over the whole
/// window below the title bar — the rail, the list and the conversation — with the circle, its status line, the reply it will
/// carry and a control row exactly where the composer's row is, the slot on the Send button. It draws what
/// <see cref="RoundRecorder"/> says and runs what it asks for; the camera is <see cref="VideoMessageRecorder"/>'s.
/// </summary>
/// <remarks>
/// <para>
/// <b>NEVER CONSTRUCTED WHILE <see cref="RoundVideoRules.RecordingEnabled"/> IS FALSE</b>: every way in is hidden. Nothing
/// here has run — T1 (the preview), T2 (the clip), T3 (the square) and T5 (Narrator) are the owner's trials.
/// </para>
/// <para>
/// <b>IT TAKES ALL INPUT UNDER IT</b>: the scrim is hit-tested, the window beneath is disabled while it is up (so neither a
/// pointer nor Tab reaches the conversation), Tab cycles inside it, and closing gives focus back to what opened it. The call
/// card stays above it — a call arriving is an interruption it hears (S4).
/// </para>
/// <para>
/// <b>EFFECTS RUN ONE AFTER ANOTHER</b>, in the order the machine gave them, so the camera is never opened and closed at the
/// same time; making the square runs beside them, and lands as one more step of the machine.
/// </para>
/// </remarks>
internal sealed class RoundRecorderLayer
{
    /// <summary>What the layer asks of the conversation that opened it.</summary>
    /// <param name="ReplyText">The primed reply's banner text, or null without one.</param>
    /// <param name="DropReply">The banner's ✕: the reply is dropped from the composer.</param>
    /// <param name="NotSentWaits">A voice message that was not sent waits in this chat (row 9).</param>
    /// <param name="QuietEverything">Whatever plays goes quiet — a take or REVIEW is starting to make or play sound (S1.7).</param>
    /// <param name="StartVoice">"Record a voice message instead": a hands-free voice recording in the composer.</param>
    /// <param name="SendAsync">The clip, staged, sent with the reply — with <c>round</c> or as a regular video.</param>
    /// <param name="Cover">The window under the layer disabled (true) or given back (false).</param>
    /// <param name="Closed">The layer is gone.</param>
    internal sealed record Hooks(
        Func<string?> ReplyText,
        Action DropReply,
        Func<bool> NotSentWaits,
        Action QuietEverything,
        Action StartVoice,
        Func<StagedMedia, bool, Task> SendAsync,
        Action<bool> Cover,
        Action Closed);

    private readonly IStringCatalog say;
    private readonly Grid host;
    private readonly FrameworkElement pane;
    private readonly FrameworkElement composer;
    private readonly FrameworkElement sendButton;
    private readonly DispatcherQueue queue;
    private readonly RoundVideoLimits limits;
    private readonly Hooks hooks;
    private readonly RoundRecorder flow;
    private readonly KeepAwake keepAwake = new();
    private readonly DispatcherQueueTimer clock;

    private VideoMessageRecorder? camera;
    private IReadOnlyList<CameraChoice> cameras = [];
    private PictureCheck pictureCheck = new();
    private bool firstTime;
    private UIElement? opener;
    private Task running = Task.CompletedTask;
    private bool closed;
    private bool voiceAfterClose;
    private bool spokeRecording;
    private RecorderFit fit = new(RoundVideoRules.LargestCircle, RecorderLayout.Row);
    private RecorderFit? fitAtRecord;
    private TaskCompletionSource<bool>? closeAnswer;

    // The clip: its generation (a clip thrown away makes a later landing land nowhere), the files, what is sent, its player.
    private int clipGeneration;
    private CancellationTokenSource? making;
    private string? takePath;
    private string? clipPath;
    private StagedMedia? media;
    private MediaPlayer? reviewPlayer;
    private bool reviewStarted;
    private ContentDialog? question;

    // ---- what is drawn ----
    private readonly Grid root = new();
    private readonly Microsoft.UI.Xaml.Shapes.Path scrimAround = new();
    private readonly Rectangle scrimPane = new() { HorizontalAlignment = HorizontalAlignment.Left, VerticalAlignment = VerticalAlignment.Top };
    private readonly Grid content = new();
    private readonly StackPanel stack = new() { Spacing = 12, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Bottom };
    private readonly Border statusBacking = new() { CornerRadius = new CornerRadius(8), Padding = new Thickness(12, 6, 12, 6), HorizontalAlignment = HorizontalAlignment.Center };
    private readonly TextBlock statusLine = new() { Foreground = new SolidColorBrush(Colors.White), TextAlignment = TextAlignment.Center };
    private readonly TextBlock statusBelow = new() { TextWrapping = TextWrapping.Wrap, TextAlignment = TextAlignment.Center, MaxWidth = 320 };
    private readonly Ellipse redDot = new() { Width = 8, Height = 8, VerticalAlignment = VerticalAlignment.Center };
    private readonly Button circleButton = new() { Padding = new Thickness(0), BorderThickness = new Thickness(0), Background = new SolidColorBrush(Colors.Transparent), HorizontalAlignment = HorizontalAlignment.Center };
    private readonly Grid circle = new();
    private readonly MediaPlayerElement previewElement = new() { AreTransportControlsEnabled = false, Stretch = Stretch.UniformToFill, IsTabStop = false };
    private readonly MediaPlayerElement reviewElement = new() { AreTransportControlsEnabled = false, Stretch = Stretch.UniformToFill, IsTabStop = false };
    private readonly Ellipse poster = new();
    private readonly ImageBrush posterBrush = new() { Stretch = Stretch.UniformToFill };
    private readonly Ellipse disc = new();
    private readonly FontIcon discGlyph = new() { Glyph = "", FontSize = 40 };
    private readonly Line slash = new() { StrokeThickness = 3 };
    private readonly ProgressRing spinner = new() { Width = 36, Height = 36, IsActive = false };
    private readonly Border playDisc = new() { Width = 56, Height = 56, CornerRadius = new CornerRadius(28), Background = new SolidColorBrush(Windows.UI.Color.FromArgb(0x99, 0, 0, 0)) };
    private readonly Border maskCard = new();
    private readonly Microsoft.UI.Xaml.Shapes.Path mask = new();
    private readonly Ellipse ringThin = new() { StrokeThickness = RecorderLook.Track, Stroke = new SolidColorBrush(Windows.UI.Color.FromArgb(0x40, 0xFF, 0xFF, 0xFF)) };
    private readonly Microsoft.UI.Xaml.Shapes.Path ringArc = new() { StrokeThickness = RoundVideoRules.RingWidth, StrokeStartLineCap = PenLineCap.Round, StrokeEndLineCap = PenLineCap.Round };
    private readonly Ellipse ringFull = new() { StrokeThickness = RoundVideoRules.RingWidth };
    private readonly Grid banner = new() { ColumnSpacing = 8, Padding = new Thickness(12, 6, 6, 6), CornerRadius = new CornerRadius(8), MaxWidth = 420 };
    private readonly TextBlock bannerText = new() { VerticalAlignment = VerticalAlignment.Center, TextTrimming = TextTrimming.CharacterEllipsis };
    private readonly Button bannerDrop = new() { Background = new SolidColorBrush(Colors.Transparent), BorderThickness = new Thickness(0), Padding = new Thickness(10) };
    private readonly Grid controls = new() { ColumnSpacing = 8, RowSpacing = 8, Padding = new Thickness(6, 0, 0, 0) };
    private readonly StackPanel leading = new() { Orientation = Orientation.Horizontal, Spacing = 8, VerticalAlignment = VerticalAlignment.Center };
    private readonly StackPanel middle = new() { Orientation = Orientation.Horizontal, Spacing = 8, VerticalAlignment = VerticalAlignment.Center, HorizontalAlignment = HorizontalAlignment.Center };
    private readonly Button closeButton = new();
    private readonly Button deleteButton = new();
    private readonly Button retakeButton = new();
    private readonly Button cameraButton = new();
    private readonly Button voiceButton = new();
    private readonly Button settingsButton = new();
    private readonly Button slot = new() { Width = SlotTarget, Height = SlotTarget, Padding = new Thickness(0), CornerRadius = new CornerRadius(SlotTarget / 2), Background = new SolidColorBrush(Colors.Transparent), BorderThickness = new Thickness(0), IsHoldingEnabled = false };
    private readonly Ellipse slotHalo = new() { Width = SlotTarget, Height = SlotTarget, Fill = new SolidColorBrush(Windows.UI.Color.FromArgb(0x2E, 0xFF, 0xFF, 0xFF)) };
    private readonly Ellipse slotDisc = new() { Width = RecorderLook.Slot, Height = RecorderLook.Slot };
    private readonly Rectangle slotSquare = new() { Width = RecorderLook.StopSquare, Height = RecorderLook.StopSquare, RadiusX = RecorderLook.StopCorner, RadiusY = RecorderLook.StopCorner, Fill = new SolidColorBrush(Colors.White) };
    private readonly FontIcon slotGlyph = new() { FontSize = 24, Foreground = new SolidColorBrush(Colors.White) };
    private readonly TextBlock slotCaption = new() { FontSize = 11, HorizontalAlignment = HorizontalAlignment.Center, Foreground = new SolidColorBrush(Windows.UI.Color.FromArgb(0xFF, 0x9D, 0xA0, 0xAD)) };

    /// <summary>The slot's target: the 64 disc and the faint halo round it (the approved design).</summary>
    private const double SlotTarget = RecorderLook.SlotTarget;
    private readonly TextBlock liveLine = new() { Opacity = 0, IsHitTestVisible = false, Width = 1, Height = 1 };
    private string? settingsPage;

    public RoundRecorderLayer(
        AppServices services,
        Grid host,
        FrameworkElement pane,
        FrameworkElement composer,
        FrameworkElement sendButton,
        DispatcherQueue queue,
        RoundVideoLimits limits,
        Hooks hooks)
    {
        say = services.Say;
        this.host = host;
        this.pane = pane;
        this.composer = composer;
        this.sendButton = sendButton;
        this.queue = queue;
        this.limits = limits;
        this.hooks = hooks;
        flow = new RoundRecorder(limits, say);
        clock = queue.CreateTimer();
        clock.Interval = TimeSpan.FromMilliseconds(250);
        clock.Tick += (_, _) => Run(flow.Tick(Now));
        Build();
    }

    private static long Now => Environment.TickCount64;

    /// <summary>The recorder is up — it owns the composer's row (S1.3 row 1).</summary>
    public bool IsOpen => !closed && flow.IsOpen;

    /// <summary>A take running, or a clip in REVIEW: a real close asks first (S4).</summary>
    public bool HoldsClip => IsOpen && flow.HoldsClip;

    // ---- opening and closing ---------------------------------------------------------------------------------

    /// <summary>
    /// Up over the window, focus on the slot (S3.4), and the camera asked for — or the refusal shown at once when one is
    /// already on record (S3.2). <paramref name="opener"/> gets focus back when it closes.
    /// </summary>
    public async Task OpenAsync(UIElement? opener)
    {
        this.opener = opener;
        if (!host.Children.Contains(root))
        {
            host.Children.Add(root);
        }
        hooks.Cover(true);
        firstTime = !RoundVideoSetting.PreviewSeen;
        var cameraAccess = VideoMessageRecorder.CameraAccess();
        var microphoneAccess = VoiceRecorder.MicrophoneAccess();
        if (RoundVideoRules.RefusedBefore(cameraAccess, microphoneAccess) is { } refused)
        {
            settingsPage = RoundVideoRules.Refusal(
                refused, refused == RecorderRefusal.Camera ? cameraAccess : microphoneAccess, say).Settings;
        }
        Run(flow.Open(Now, RoundVideoRules.RefusedBefore(cameraAccess, microphoneAccess), RoundVideoRules.WillAsk(cameraAccess, microphoneAccess)));
        clock.Start();
        Layout();
        Draw();
        slot.Focus(FocusState.Programmatic);
        await running;
    }

    /// <summary>Something other than the person (S4): PREVIEW closes, a take stops into REVIEW, a sign-out deletes everything.</summary>
    public void Interrupt(RecorderInterruption why)
    {
        if (!closed)
        {
            Run(flow.Interrupt(why, Now));
        }
    }

    /// <summary>
    /// Something that pauses what plays (S4's last column): REVIEW's clip pauses for a call, a lock, a hidden window or the
    /// output device going, as any circle does.
    /// </summary>
    public void PausePlayback(PlaybackEvent happened)
    {
        if (!closed)
        {
            Run(flow.PlaybackInterrupted(happened));
        }
    }

    /// <summary>
    /// A real close over a take or a clip (S4): the take stops into REVIEW, and "Delete video message?" is asked. True once
    /// the close may go ahead — the person chose Delete, or nothing was held after all; false when they chose Keep.
    /// </summary>
    public Task<bool> AskBeforeClosingAsync()
    {
        if (!HoldsClip)
        {
            return Task.FromResult(true);
        }
        closeAnswer ??= new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
        var answer = closeAnswer.Task;
        Run(flow.Interrupt(RecorderInterruption.WindowClosing, Now));
        return answer;
    }

    // ---- the machine's steps -----------------------------------------------------------------------------------

    private void Run(IReadOnlyList<RecorderEffect> effects)
    {
        if (effects.Count == 0)
        {
            Draw();
            return;
        }
        running = RunAsync(running, effects);
    }

    private async Task RunAsync(Task before, IReadOnlyList<RecorderEffect> effects)
    {
        try
        {
            await before;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"the video recorder's last step: {e.GetType().Name}");
        }
        foreach (var effect in effects)
        {
            try
            {
                await DoAsync(effect);
            }
            catch (Exception e)
            {
                Diagnostics.Write($"the video recorder's {effect.Action}: {e.GetType().Name} 0x{e.HResult:X8}");
            }
        }
        Draw();
    }

    private async Task DoAsync(RecorderEffect effect)
    {
        switch (effect.Action)
        {
            case RecorderAction.OpenCamera:
                await OpenCameraAsync();
                break;
            case RecorderAction.CloseCamera:
                await CloseCameraAsync();
                break;
            case RecorderAction.StartRecording:
                // No app sound while recording (S1.7), the screen kept on, and the layout held until Stop (S3.3).
                hooks.QuietEverything();
                keepAwake.Hold();
                fitAtRecord = fit;
                // With a screen reader running, "Recording video" is said BEFORE the microphone opens, so the app's own
                // voice is not the first thing in the clip (S6) — a second, as for a voice note.
                var leadIn = VoiceNotes.LeadIn(ScreenReader.Running());
                if (leadIn > TimeSpan.Zero)
                {
                    spokeRecording = true;
                    Announce(say.Get("Recording video"));
                    await Task.Delay(leadIn);
                }
                if (camera is not { } recording || flow.Stage != RecorderStage.Recording || !await recording.StartAsync())
                {
                    keepAwake.Release();
                    fitAtRecord = null;
                    Run(flow.RecordFailed());
                    break;
                }
                // The clock, the ring and the 1.0 s floor count from here — what the file holds — not from the press (S6).
                flow.TakeStarted(Now);
                break;
            case RecorderAction.StopAndKeep:
            {
                var take = camera is { } stopping ? await stopping.StopAsync() : null;
                keepAwake.Release();
                fitAtRecord = null;
                // Beside the steps, not in their line: a question put up while the square is made must not wait for it.
                _ = MakeClipAsync(take, clipGeneration);
                break;
            }
            case RecorderAction.StopAndDiscard:
                if (camera is { } discarding)
                {
                    await discarding.DiscardAsync();
                }
                keepAwake.Release();
                fitAtRecord = null;
                break;
            case RecorderAction.DiscardClip:
                DiscardClip();
                break;
            case RecorderAction.PauseClip:
                PauseReview();
                break;
            case RecorderAction.Ask:
                Ask();
                break;
            case RecorderAction.Dismiss:
                if (question is { } up)
                {
                    question = null;
                    up.Hide();
                }
                break;
            case RecorderAction.Send:
                if (media is { } clip && flow.Plan is { } plan)
                {
                    media = null;
                    await hooks.SendAsync(clip, plan.Round);
                }
                break;
            case RecorderAction.StartVoice:
                voiceAfterClose = true;
                break;
            case RecorderAction.CloseWindow:
                closeAnswer?.TrySetResult(true);
                closeAnswer = null;
                break;
            case RecorderAction.Closed:
                Teardown();
                break;
            case RecorderAction.Announce:
                // Said already, before the microphone opened: not again into the clip.
                if (spokeRecording && effect.Text == say.Get("Recording video"))
                {
                    spokeRecording = false;
                    break;
                }
                Announce(effect.Text);
                break;
        }
    }

    /// <summary>
    /// The camera chosen on this device, else the front-panel one, else any (S3.5) — opened, its picture put in the circle,
    /// and its first frame, brightness and trouble wired to the machine. A camera that opens after the recorder moved on is
    /// let go of at once.
    /// </summary>
    private async Task OpenCameraAsync()
    {
        // Never two at once: whatever is still open — a take stopped to ask about deleting it kept its camera — goes first.
        await CloseCameraAsync();
        cameras = await VideoMessageRecorder.CamerasAsync();
        if (closed || flow.Stage != RecorderStage.Opening)
        {
            return;
        }
        if (RoundVideoRules.PickCamera(cameras, RoundVideoSetting.Camera) is not { } chosen)
        {
            Run(flow.CameraTroubled(CameraTrouble.Missing, Now));
            return;
        }
        var (opened, failure) = await VideoMessageRecorder.OpenAsync(chosen.Id);
        if (opened is null)
        {
            Run(failure?.Refused is { } refused
                ? flow.Refuse(refused)
                : flow.CameraTroubled(failure?.Trouble ?? CameraTrouble.Failed, Now));
            if (failure?.Refused is { } why)
            {
                var access = why == RecorderRefusal.Camera ? VideoMessageRecorder.CameraAccess() : VoiceRecorder.MicrophoneAccess();
                settingsPage = RoundVideoRules.Refusal(why, access, say).Settings;
            }
            return;
        }
        if (closed || flow.Stage != RecorderStage.Opening)
        {
            await opened.DisposeAsync();
            return;
        }
        camera = opened;
        pictureCheck = new PictureCheck();
        var check = pictureCheck;
        opened.FirstFrame += () => queue.TryEnqueue(() =>
        {
            if (ReferenceEquals(camera, opened))
            {
                Run(flow.FirstFrame());
            }
        });
        opened.Brightness += luma => queue.TryEnqueue(() =>
        {
            if (ReferenceEquals(camera, opened) && check.Frame(Now, luma))
            {
                flow.Covered();
                Draw();
            }
        });
        opened.Blocked += () => queue.TryEnqueue(() =>
        {
            if (ReferenceEquals(camera, opened))
            {
                flow.Covered();
                Draw();
            }
        });
        opened.Troubled += trouble => queue.TryEnqueue(() =>
        {
            if (ReferenceEquals(camera, opened))
            {
                Run(flow.CameraTroubled(trouble, Now));
            }
        });
        previewElement.SetMediaPlayer(opened.Preview);
        Run(flow.CameraOn(Now));
        if (firstTime)
        {
            // Said once on this device (S7.5): written now, read only when the recorder next opens.
            RoundVideoSetting.PreviewSeen = true;
        }
    }

    private async Task CloseCameraAsync()
    {
        if (camera is not { } owned)
        {
            return;
        }
        camera = null;
        Unplug(previewElement);
        await owned.DisposeAsync();
    }

    /// <summary>
    /// The take into what is sent (S3.6): the square (<see cref="MediaPreparing.RoundAsync"/>) — round, or as a regular video
    /// when it is too big — or, when no square could be made, the take itself through the ordinary video path, as a regular
    /// video. Then REVIEW plays what will be sent. A take thrown away meanwhile lands nowhere.
    /// </summary>
    private async Task MakeClipAsync(VideoTake? take, int generation)
    {
        making?.Cancel();
        var cancel = new CancellationTokenSource();
        making = cancel;
        if (take is null)
        {
            Run(flow.ClipFinished(Now, null, null));
            return;
        }
        takePath = take.Path;
        try
        {
            var file = await StorageFile.GetFileFromPathAsync(take.Path);
            var made = await MediaPreparing.RoundAsync(file, cancel.Token);
            var duration = made?.DurationMs ?? (int)Math.Min(int.MaxValue, take.Elapsed.TotalMilliseconds);
            var plan = RoundVideoRules.Plan(made?.Bytes.LongLength, duration, limits, say);
            StagedMedia? sent;
            string? playing;
            if (plan.FromSquare && made is not null)
            {
                sent = plan.Round
                    ? RoundVideoRules.Staged(made.Bytes, duration, made.Poster)
                    : new StagedMedia("video", "video/mp4", made.Bytes, (int)RoundVideoRules.Edge, (int)RoundVideoRules.Edge, duration, Preview: made.Poster);
                playing = await VideoMessageRecorder.WriteClipAsync(made.Bytes);
            }
            else
            {
                // Rule C: the take as any picked video is sent — brought to the profile by the planner, with its own poster.
                sent = (await MediaPreparing.PrepareAsync(file, cancel.Token)).Media;
                playing = take.Path;
            }
            if (generation != clipGeneration || closed)
            {
                if (playing != take.Path)
                {
                    VideoMessageRecorder.Delete(playing);
                }
                return;
            }
            if (sent is null || playing is null)
            {
                Run(flow.ClipFinished(Now, null, null));
                return;
            }
            media = sent;
            clipPath = playing;
            await ShowClipAsync(playing, sent.Preview);
            Run(flow.ClipFinished(Now, plan, sent.DurationMs ?? duration));
        }
        catch (OperationCanceledException)
        {
            // Thrown away while it was being made.
        }
        catch (Exception e)
        {
            Diagnostics.Write($"making a video message: {e.GetType().Name} 0x{e.HResult:X8}");
            if (generation == clipGeneration && !closed)
            {
                Run(flow.ClipFinished(Now, null, null));
            }
        }
    }

    /// <summary>REVIEW's circle: the poster with a play glyph, and the clip AS IT WILL BE SENT — never mirrored (S3.4).</summary>
    private async Task ShowClipAsync(string path, ReadOnlyMemory<byte>? posterJpeg)
    {
        StopReview();
        var player = new MediaPlayer { AutoPlay = false };
        player.Source = MediaSource.CreateFromUri(new Uri(path));
        player.PlaybackSession.PlaybackStateChanged += (_, _) => queue.TryEnqueue(Draw);
        player.MediaEnded += (_, _) => queue.TryEnqueue(() =>
        {
            if (ReferenceEquals(reviewPlayer, player))
            {
                reviewStarted = false;
                Draw();
            }
        });
        reviewPlayer = player;
        reviewStarted = false;
        reviewElement.SetMediaPlayer(player);
        posterBrush.ImageSource = null;
        if (posterJpeg is { Length: > 0 } jpeg)
        {
            try
            {
                var image = new BitmapImage();
                using var stream = new InMemoryRandomAccessStream();
                await stream.WriteAsync(jpeg.ToArray().AsBuffer());
                stream.Seek(0);
                await image.SetSourceAsync(stream);
                posterBrush.ImageSource = image;
            }
            catch (Exception e)
            {
                Diagnostics.Write($"drawing a video message's poster: {e.GetType().Name}");
            }
        }
    }

    /// <summary>A tap on the circle, or Space anywhere in REVIEW (S3.4): play with sound, or pause.</summary>
    private void TogglePlay()
    {
        if (flow.Stage != RecorderStage.Review || !flow.ClipReady || reviewPlayer is not { } player)
        {
            return;
        }
        flow.Used(Now);
        try
        {
            if (player.PlaybackSession.PlaybackState == MediaPlaybackState.Playing)
            {
                player.Pause();
            }
            else
            {
                hooks.QuietEverything();
                if (!reviewStarted)
                {
                    player.PlaybackSession.Position = TimeSpan.Zero;
                }
                reviewStarted = true;
                player.Play();
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"playing a video message to review: {e.GetType().Name}");
        }
        Draw();
    }

    /// <summary>REVIEW's clip paused where it is, when it is playing — the play glyph back, the place kept.</summary>
    private void PauseReview()
    {
        if (reviewPlayer is not { } player)
        {
            return;
        }
        try
        {
            if (player.PlaybackSession.PlaybackState == MediaPlaybackState.Playing)
            {
                player.Pause();
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"pausing a video message's review: {e.GetType().Name}");
        }
    }

    private void StopReview()
    {
        if (reviewPlayer is not { } player)
        {
            return;
        }
        reviewPlayer = null;
        reviewStarted = false;
        Unplug(reviewElement);
        try
        {
            player.Pause();
            (player.Source as IDisposable)?.Dispose();
            player.Source = null;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"stopping a video message's review: {e.GetType().Name}");
        }
        player.Dispose();
    }

    /// <summary>A player taken out of its element — guarded, because a teardown must reach the camera after it whatever this does.</summary>
    private static void Unplug(MediaPlayerElement element)
    {
        try
        {
            element.SetMediaPlayer(null);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"taking a player out of the video recorder: {e.GetType().Name}");
        }
    }

    private void DiscardClip()
    {
        clipGeneration++;
        making?.Cancel();
        making = null;
        StopReview();
        posterBrush.ImageSource = null;
        media = null;
        if (clipPath is { } played && played != takePath)
        {
            VideoMessageRecorder.Delete(played);
        }
        VideoMessageRecorder.Delete(takePath);
        clipPath = null;
        takePath = null;
    }

    /// <summary>"Delete video message?" [Delete] [Keep] — Keep the default, so a reflex Enter keeps what cannot be recorded again.</summary>
    private void Ask()
    {
        if (question is not null || root.XamlRoot is null)
        {
            return;
        }
        var dialog = Dialogs.Create(root.XamlRoot, say.Get("Delete video message?"), string.Empty);
        dialog.PrimaryButtonText = say.Get("Delete");
        dialog.CloseButtonText = say.Get("Keep");
        dialog.DefaultButton = ContentDialogButton.Close;
        question = dialog;
        _ = AnswerAsync(dialog);
    }

    private async Task AnswerAsync(ContentDialog dialog)
    {
        var delete = false;
        try
        {
            delete = await dialog.ShowAsync() == ContentDialogResult.Primary;
        }
        catch (Exception e)
        {
            // Another dialog already up: nothing is deleted on a question nobody saw.
            Diagnostics.Write($"asking about a video message: {e.GetType().Name}");
        }
        if (!ReferenceEquals(question, dialog))
        {
            // Taken away by something else, which decided instead.
            return;
        }
        question = null;
        var asked = flow.Question;
        Run(flow.Answer(delete, Now));
        if (asked == RecorderQuestion.CloseWindow && !delete)
        {
            // Keep cancels the close (S4).
            closeAnswer?.TrySetResult(false);
            closeAnswer = null;
        }
    }

    /// <summary>
    /// Gone: the camera, the clip and its files, the screen's keep-awake; the window given back and focus with it — then a
    /// voice recording, when that is why it closed.
    /// </summary>
    private void Teardown()
    {
        if (closed)
        {
            return;
        }
        closed = true;
        clock.Stop();
        if (question is { } up)
        {
            question = null;
            up.Hide();
        }
        DiscardClip();
        if (camera is { } owned)
        {
            camera = null;
            Unplug(previewElement);
            _ = owned.DisposeAsync().AsTask();
        }
        keepAwake.Release();
        host.Children.Remove(root);
        hooks.Cover(false);
        closeAnswer?.TrySetResult(true);
        closeAnswer = null;
        if (opener is Control control)
        {
            control.Focus(FocusState.Programmatic);
        }
        hooks.Closed();
        if (voiceAfterClose)
        {
            hooks.StartVoice();
        }
    }

    /// <summary>Said to a screen reader — politely, state changes only (S6) — on the layer's own hidden live region.</summary>
    private void Announce(string? sentence)
    {
        if (string.IsNullOrEmpty(sentence) || closed)
        {
            return;
        }
        liveLine.Text = sentence;
        try
        {
            (FrameworkElementAutomationPeer.FromElement(liveLine) ?? FrameworkElementAutomationPeer.CreatePeerForElement(liveLine))
                ?.RaiseAutomationEvent(AutomationEvents.LiveRegionChanged);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"announcing in the video recorder: {e.GetType().Name}");
        }
    }

    // ---- building and laying out ---------------------------------------------------------------------------------

    private void Build()
    {
        var resources = Application.Current.Resources;
        // Opaque where Windows' transparency effects are off (S3.3's Reduce Transparency).
        var transparent = true;
        try
        {
            transparent = new UISettings().AdvancedEffectsEnabled;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"asking about transparency effects: {e.GetType().Name}");
        }
        // The whole window darkened, the conversation as much as the rest (the approved design): what is recorded is the
        // circle, and nothing behind it should compete with it.
        scrimAround.Fill = new SolidColorBrush(Windows.UI.Color.FromArgb(transparent ? (byte)0xD9 : (byte)0xFF, 0x0B, 0x0C, 0x10));
        scrimPane.Fill = new SolidColorBrush(Windows.UI.Color.FromArgb(transparent ? (byte)0xD9 : (byte)0xFF, 0x0B, 0x0C, 0x10));
        root.Children.Add(scrimAround);
        root.Children.Add(scrimPane);
        root.Children.Add(content);
        root.Children.Add(liveLine);
        root.TabFocusNavigation = KeyboardNavigationMode.Cycle;
        AutomationProperties.SetName(root, say.Get("Video message"));
        AutomationProperties.SetLiveSetting(liveLine, AutomationLiveSetting.Polite);
        root.PreviewKeyDown += OnPreviewKey;
        root.PreviewKeyUp += OnPreviewKeyUp;
        root.KeyDown += OnKey;
        root.SizeChanged += (_, _) => Layout();

        // The status line, on its own backing.
        statusBacking.Background = new SolidColorBrush(Windows.UI.Color.FromArgb(0xB3, 0, 0, 0));
        redDot.Fill = (Brush)resources["SystemFillColorCriticalBrush"];
        var line = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Center };
        line.Children.Add(redDot);
        line.Children.Add(statusLine);
        var lines = new StackPanel { Spacing = 2 };
        lines.Children.Add(line);
        lines.Children.Add(statusBelow);
        statusBacking.Child = lines;

        // The circle, its ring just outside it.
        poster.Fill = posterBrush;
        disc.Fill = (Brush)resources["ControlFillColorSecondaryBrush"];
        discGlyph.Foreground = new SolidColorBrush(Colors.White);
        slash.Stroke = new SolidColorBrush(Colors.White);
        slash.HorizontalAlignment = HorizontalAlignment.Center;
        slash.VerticalAlignment = VerticalAlignment.Center;
        slash.X1 = 0;
        slash.Y1 = 0;
        slash.X2 = 44;
        slash.Y2 = 44;
        spinner.Foreground = new SolidColorBrush(Colors.White);
        playDisc.Child = new FontIcon { Glyph = "", FontSize = 22, Foreground = new SolidColorBrush(Colors.White) };
        playDisc.HorizontalAlignment = HorizontalAlignment.Center;
        playDisc.VerticalAlignment = VerticalAlignment.Center;
        // The preview is mirrored, like a mirror and a call's self-view; the file is not (S3.5).
        previewElement.RenderTransformOrigin = new Point(0.5, 0.5);
        previewElement.RenderTransform = new ScaleTransform { ScaleX = -1 };
        var card = (Brush)resources["SolidBackgroundFillColorBaseBrush"];
        maskCard.Background = card;
        mask.Fill = card;
        foreach (var part in new UIElement[] { maskCard, disc, previewElement, reviewElement, poster, mask, discGlyph, slash, spinner, playDisc, ringThin, ringArc, ringFull })
        {
            circle.Children.Add(part);
        }
        foreach (var centred in new FrameworkElement[] { maskCard, disc, previewElement, reviewElement, poster, mask, ringThin, ringFull })
        {
            centred.HorizontalAlignment = HorizontalAlignment.Center;
            centred.VerticalAlignment = VerticalAlignment.Center;
        }
        discGlyph.HorizontalAlignment = HorizontalAlignment.Center;
        discGlyph.VerticalAlignment = VerticalAlignment.Center;
        circleButton.Content = circle;
        circleButton.Click += (_, _) => TogglePlay();

        // The reply it will carry, with its ✕ (S3.3).
        banner.Background = (Brush)resources["AcrylicInAppFillColorDefaultBrush"];
        banner.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        banner.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        banner.Children.Add(bannerText);
        bannerDrop.Content = new FontIcon { Glyph = "", FontSize = 12 };
        AutomationProperties.SetName(bannerDrop, say.Get("Cancel reply"));
        ToolTipService.SetToolTip(bannerDrop, say.Get("Cancel reply"));
        Grid.SetColumn(bannerDrop, 1);
        banner.Children.Add(bannerDrop);
        bannerDrop.Click += (_, _) =>
        {
            flow.Used(Now);
            hooks.DropReply();
            Layout();
            Draw();
        };

        stack.Children.Add(statusBacking);
        stack.Children.Add(circleButton);
        stack.Children.Add(banner);
        content.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        content.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        content.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        content.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        content.Children.Add(stack);
        content.Children.Add(controls);

        // The control row: the leading control, the middle ones, and the slot where Send is — round buttons with captions under
        // them, the slot the one big one (the approved design).
        Round(closeButton, 0xE711, say.Get("Close"), say.Get("Close"));
        closeButton.Click += (_, _) => Run(flow.CloseButton());
        Round(deleteButton, 0xE74D, say.Get("Delete"), say.Get("Delete recording"));
        deleteButton.Click += (_, _) => Run(flow.Delete(Now));
        Round(retakeButton, 0xE72C, say.Get("Retake"), say.Get("Retake"));
        retakeButton.Click += (_, _) => Run(flow.Retake(Now));
        leading.Children.Add(Captioned(closeButton));
        leading.Children.Add(Captioned(deleteButton));
        // Retake stands where the camera button does: the middle of the row, as the design has it.
        middle.Children.Add(Captioned(retakeButton));
        // It opens the cameras by their names (S3.5) — on a desktop there may be more than two. A desktop's camera button opens a LIST of its cameras (S3.5: "Choose camera"), so that is its name; it is captioned
        // with what it is about rather than the phones' "Switch", which would promise a flip.
        Round(cameraButton, 0xE89E, say.Get("Camera"), say.Get("Choose camera"));
        cameraButton.Click += (_, _) => ChooseCamera();
        Round(voiceButton, ComposerButton.MicrophoneGlyph, string.Empty, say.Get("Record a voice message instead"));
        voiceButton.Click += (_, _) => Run(flow.VoiceInstead(hooks.NotSentWaits()));
        Round(settingsButton, 0xE713, say.Get("Settings"), say.Get("Open Settings"));
        settingsButton.Click += (_, _) =>
        {
            if (settingsPage is { } page)
            {
                _ = Launcher.LaunchUriAsync(new Uri(page));
            }
        };
        middle.Children.Add(Captioned(cameraButton));
        middle.Children.Add(Captioned(voiceButton));
        middle.Children.Add(Captioned(settingsButton));
        var face = new Grid { Width = SlotTarget, Height = SlotTarget };
        face.Children.Add(slotHalo);
        face.Children.Add(slotDisc);
        foreach (var centred in new FrameworkElement[] { slotHalo, slotDisc, slotSquare, slotGlyph })
        {
            centred.HorizontalAlignment = HorizontalAlignment.Center;
            centred.VerticalAlignment = VerticalAlignment.Center;
        }
        face.Children.Add(slotSquare);
        face.Children.Add(slotGlyph);
        slot.Content = face;
        slot.Click += (_, _) => Run(flow.Slot(Now));
        AutomationProperties.SetAccessibilityView(slotCaption, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        slotColumn.Children.Add(slot);
        slotColumn.Children.Add(slotCaption);
        controls.Children.Add(leading);
        controls.Children.Add(middle);
        controls.Children.Add(slotColumn);
    }

    /// <summary>The slot and its caption, one column: what the layout places where Send is.</summary>
    private readonly StackPanel slotColumn = new() { Spacing = RecorderLook.CaptionGap, VerticalAlignment = VerticalAlignment.Bottom };

    /// <summary>
    /// One of the smaller round buttons: a glyph in Segoe Fluent Icons on the design's faint disc, 44 across, its caption kept
    /// in its <see cref="Button.Tag"/> for <see cref="Captioned"/>, and the name a screen reader hears.
    /// </summary>
    private static void Round(Button button, int glyph, string caption, string name)
    {
        button.Content = new FontIcon { Glyph = ((char)glyph).ToString(), FontSize = 16 };
        button.Width = RecorderLook.Side;
        button.Height = RecorderLook.Side;
        button.Padding = new Thickness(0);
        button.CornerRadius = new CornerRadius(RecorderLook.Side / 2);
        button.BorderThickness = new Thickness(0);
        button.Background = new SolidColorBrush(Windows.UI.Color.FromArgb(0x14, 0xFF, 0xFF, 0xFF));
        button.Foreground = new SolidColorBrush(Windows.UI.Color.FromArgb(0xFF, 0xE8, 0xE9, 0xEF));
        button.HorizontalAlignment = HorizontalAlignment.Center;
        button.Tag = caption;
        AutomationProperties.SetName(button, name);
        ToolTipService.SetToolTip(button, name);
    }

    /// <summary>A round button with its caption under it — a caption a screen reader never meets twice: the button says its name.</summary>
    private static StackPanel Captioned(Button button)
    {
        var caption = new TextBlock
        {
            // An uncaptioned button keeps an empty line (a no-break space) so every button in the row stands as high.
            Text = button.Tag is string { Length: > 0 } words ? words : "\u00A0",
            FontSize = 11,
            // Its own height at the reader's text size — a fixed one cut the bottom off from 150 %.
            HorizontalAlignment = HorizontalAlignment.Center,
            TextAlignment = TextAlignment.Center,
            Foreground = new SolidColorBrush(Windows.UI.Color.FromArgb(0xFF, 0x9D, 0xA0, 0xAD)),
        };
        AutomationProperties.SetAccessibilityView(caption, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        var column = new StackPanel { Spacing = RecorderLook.CaptionGap, MinWidth = 56, VerticalAlignment = VerticalAlignment.Bottom };
        column.Children.Add(button);
        column.Children.Add(caption);
        return column;
    }

    /// <summary>A round button shown or not — with its caption, which is the column it stands in.</summary>
    private static void Show(Button button, bool shown)
    {
        var visibility = shown ? Visibility.Visible : Visibility.Collapsed;
        if (button.Parent is FrameworkElement column)
        {
            column.Visibility = visibility;
        }
        button.Visibility = visibility;
    }

    /// <summary>"Choose camera" (S3.5): the cameras by their system names, the one in use checked; the choice is remembered here.</summary>
    private void ChooseCamera()
    {
        flow.Used(Now);
        var menu = new MenuFlyout();
        var current = RoundVideoRules.PickCamera(cameras, RoundVideoSetting.Camera);
        foreach (var choice in cameras)
        {
            var item = new ToggleMenuFlyoutItem { Text = choice.Name, IsChecked = choice.Id == current?.Id };
            item.Click += (_, _) =>
            {
                if (choice.Id == current?.Id)
                {
                    return;
                }
                RoundVideoSetting.Camera = choice.Id;
                Run(flow.SwitchCamera(Now));
            };
            menu.Items.Add(item);
        }
        menu.ShowAt(cameraButton);
    }

    /// <summary>Where the window's parts are, in the layer's own coordinates; null while one is not laid out.</summary>
    private Rect? Bounds(FrameworkElement element)
    {
        try
        {
            if (element.ActualWidth <= 0 || element.ActualHeight <= 0 || element.XamlRoot is null)
            {
                return null;
            }
            return element.TransformToVisual(host).TransformBounds(new Rect(0, 0, element.ActualWidth, element.ActualHeight));
        }
        catch (Exception e)
        {
            Diagnostics.Write($"placing the video recorder: {e.GetType().Name}");
            return null;
        }
    }

    /// <summary>
    /// The scrim — 70 % over the rail and the list, 30 % over the conversation, still taking every click — and the circle
    /// and controls over the conversation pane: the control row on the composer's row, the slot on the Send button (S3.3).
    /// A pane shorter than 480 stands the controls in a column at the trailing edge. Kept as chosen at Record until Stop.
    /// </summary>
    private void Layout()
    {
        var width = host.ActualWidth;
        var height = host.ActualHeight;
        if (width <= 0 || height <= 0)
        {
            return;
        }
        var conversation = Bounds(pane) ?? new Rect(0, 0, width, height);
        var around = new GeometryGroup { FillRule = FillRule.EvenOdd };
        around.Children.Add(new RectangleGeometry { Rect = new Rect(0, 0, width, height) });
        around.Children.Add(new RectangleGeometry { Rect = conversation });
        scrimAround.Data = around;
        var inset = new Thickness(conversation.X, conversation.Y, Math.Max(0, width - conversation.Right), Math.Max(0, height - conversation.Bottom));
        scrimPane.Margin = inset;
        scrimPane.Width = conversation.Width;
        scrimPane.Height = conversation.Height;
        content.Margin = inset;

        var replying = hooks.ReplyText() is not null;
        banner.Measure(new Size(420, double.PositiveInfinity));
        var bannerHeight = replying ? Math.Max(40, banner.DesiredSize.Height) : 0;
        fit = fitAtRecord ?? RoundVideoRules.Fit(conversation.Width, conversation.Height, bannerHeight);
        SizeCircle(fit.Diameter);

        var send = Bounds(sendButton);
        var row = Bounds(composer);
        if (fit.Layout == RecorderLayout.Row)
        {
            Grid.SetRowSpan(stack, 1);
            Grid.SetColumnSpan(stack, 2);
            Grid.SetRow(controls, 1);
            Grid.SetColumn(controls, 0);
            Grid.SetColumnSpan(controls, 2);
            controls.RowDefinitions.Clear();
            controls.ColumnDefinitions.Clear();
            controls.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            controls.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            controls.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            Place(leading, 0, 0);
            Place(middle, 0, 1);
            Place(slotColumn, 0, 2);
            controls.VerticalAlignment = VerticalAlignment.Bottom;
            controls.HorizontalAlignment = HorizontalAlignment.Stretch;
            // On the composer's row, the big slot CENTRED where Send is, so the pointer never moves (S3.3), and its caption
            // under it.
            var left = row is { } card ? card.X + 6 - conversation.X : 16;
            var right = send is { } at ? conversation.Right - (at.X + at.Width / 2) - SlotTarget / 2 : 22;
            // The slot's caption as tall as it is at the reader's text size, MEASURED, so the slot itself is centred on Send.
            slotCaption.Measure(new Size(double.PositiveInfinity, double.PositiveInfinity));
            var caption = slotCaption.DesiredSize.Height;
            var bottom = send is { } there ? RecorderLook.SlotColumnBottom(conversation.Bottom - (there.Y + there.Height / 2), caption) : 4;
            controls.Margin = new Thickness(Math.Max(0, left), 0, Math.Max(0, right), Math.Max(0, bottom));
            controls.Height = double.NaN;
            controls.Width = double.NaN;
            stack.Margin = new Thickness(24, 12, 24, Math.Max(0, bottom) + SlotTarget + RecorderLook.CaptionGap + caption + 16);
            stack.VerticalAlignment = VerticalAlignment.Bottom;
            leading.Orientation = Orientation.Horizontal;
            middle.Orientation = Orientation.Horizontal;
            if (banner.Parent != stack)
            {
                (banner.Parent as Panel)?.Children.Remove(banner);
                stack.Children.Add(banner);
            }
        }
        else
        {
            Grid.SetRowSpan(stack, 2);
            Grid.SetColumnSpan(stack, 1);
            Grid.SetRow(controls, 0);
            Grid.SetColumn(controls, 1);
            Grid.SetColumnSpan(controls, 1);
            Grid.SetRowSpan(controls, 2);
            controls.ColumnDefinitions.Clear();
            controls.RowDefinitions.Clear();
            for (var at = 0; at < 4; at++)
            {
                controls.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            }
            controls.VerticalAlignment = VerticalAlignment.Center;
            controls.HorizontalAlignment = HorizontalAlignment.Right;
            controls.Margin = new Thickness(0, 0, 16, 0);
            controls.Height = double.NaN;
            controls.Width = 184;
            stack.Margin = new Thickness(16, 8, 16, 8);
            stack.VerticalAlignment = VerticalAlignment.Center;
            leading.Orientation = Orientation.Vertical;
            middle.Orientation = Orientation.Vertical;
            if (banner.Parent != controls)
            {
                (banner.Parent as Panel)?.Children.Remove(banner);
                controls.Children.Insert(0, banner);
            }
            Place(banner, 0, 0);
            Place(leading, 1, 0);
            Place(middle, 2, 0);
            Place(slotColumn, 3, 0);
        }
    }

    private static void Place(FrameworkElement element, int row, int column)
    {
        Grid.SetRow(element, row);
        Grid.SetColumn(element, column);
    }

    /// <summary>The circle at diameter <paramref name="diameter"/>, cut round as trial T2 settles (<see cref="RoundVideoRules.Clip"/>).</summary>
    private void SizeCircle(double diameter)
    {
        var outer = diameter + 2 * (RoundVideoRules.RingWidth + 2);
        circle.Width = outer;
        circle.Height = outer;
        foreach (var inner in new FrameworkElement[] { disc, previewElement, reviewElement, poster, maskCard, mask })
        {
            inner.Width = diameter;
            inner.Height = diameter;
        }
        // The thin track lies exactly where the progress ring runs: OUTSIDE the edge, the arc drawn over it.
        ringThin.Width = diameter + RoundVideoRules.RingWidth;
        ringThin.Height = diameter + RoundVideoRules.RingWidth;
        ringFull.Width = diameter + 2 * RoundVideoRules.RingWidth;
        ringFull.Height = diameter + 2 * RoundVideoRules.RingWidth;
        var clip = RoundVideoRules.Clip;
        foreach (var video in new MediaPlayerElement[] { previewElement, reviewElement })
        {
            if (clip == PreviewClip.CornerRadius)
            {
                video.CornerRadius = new CornerRadius(diameter / 2);
            }
            else if (clip == PreviewClip.Composition)
            {
                var visual = ElementCompositionPreview.GetElementVisual(video);
                var ellipse = visual.Compositor.CreateEllipseGeometry();
                ellipse.Center = new Vector2((float)(diameter / 2));
                ellipse.Radius = new Vector2((float)(diameter / 2));
                visual.Clip = visual.Compositor.CreateGeometricClip(ellipse);
            }
        }
        // The fallback: the square on an opaque card, its corners painted in the card's own colour.
        var masked = clip == PreviewClip.Mask;
        maskCard.Visibility = masked ? Visibility.Visible : Visibility.Collapsed;
        mask.Visibility = masked ? Visibility.Visible : Visibility.Collapsed;
        if (masked)
        {
            var corners = new GeometryGroup { FillRule = FillRule.EvenOdd };
            corners.Children.Add(new RectangleGeometry { Rect = new Rect(0, 0, diameter, diameter) });
            corners.Children.Add(new EllipseGeometry { Center = new Point(diameter / 2, diameter / 2), RadiusX = diameter / 2, RadiusY = diameter / 2 });
            mask.Data = corners;
        }
    }

    // ---- drawing -------------------------------------------------------------------------------------------------

    private void Draw()
    {
        if (closed)
        {
            return;
        }
        var now = Now;
        var stage = flow.Stage;
        var resources = Application.Current.Resources;

        // The status line (S3.4).
        var status = flow.Status(now, firstTime, RoundVideoRules.MicrophoneOnInPreview);
        statusLine.Text = status.Line;
        redDot.Visibility = status.Red ? Visibility.Visible : Visibility.Collapsed;
        statusBelow.Text = status.Below ?? string.Empty;
        statusBelow.Visibility = status.Below is null ? Visibility.Collapsed : Visibility.Visible;
        statusBelow.Foreground = status.Warning
            ? (Brush)resources["SystemFillColorCautionBrush"]
            : new SolidColorBrush(Windows.UI.Color.FromArgb(0xE6, 0xFF, 0xFF, 0xFF));

        // The circle.
        var live = stage is RecorderStage.Preview or RecorderStage.Recording && camera is not null;
        var review = stage == RecorderStage.Review && flow.ClipReady;
        var playing = review && reviewPlayer?.PlaybackSession.PlaybackState == MediaPlaybackState.Playing;
        previewElement.Visibility = live ? Visibility.Visible : Visibility.Collapsed;
        reviewElement.Visibility = review && reviewStarted ? Visibility.Visible : Visibility.Collapsed;
        poster.Visibility = review && !reviewStarted ? Visibility.Visible : Visibility.Collapsed;
        var waiting = stage == RecorderStage.Opening
            || (stage == RecorderStage.Preview && !flow.HasPicture)
            || (stage == RecorderStage.Review && !flow.ClipReady);
        var refused = stage is RecorderStage.Refused or RecorderStage.Unavailable;
        disc.Visibility = waiting || refused || (stage == RecorderStage.Preview && !live) ? Visibility.Visible : Visibility.Collapsed;
        discGlyph.Visibility = refused ? Visibility.Visible : Visibility.Collapsed;
        discGlyph.Glyph = refused && flow.Refused == RecorderRefusal.Microphone ? ((char)ComposerButton.MicrophoneGlyph).ToString() : "";
        slash.Visibility = refused ? Visibility.Visible : Visibility.Collapsed;
        spinner.IsActive = waiting;
        spinner.Visibility = waiting ? Visibility.Visible : Visibility.Collapsed;
        playDisc.Visibility = review && !playing ? Visibility.Visible : Visibility.Collapsed;
        circleButton.IsTabStop = review;
        circleButton.IsHitTestVisible = review;
        AutomationProperties.SetName(circleButton, review ? (playing ? say.Get("Pause") : say.Get("Play")) : say.Get("Video message"));
        if (!review)
        {
            AutomationProperties.SetAccessibilityView(circleButton, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        }
        else
        {
            AutomationProperties.SetAccessibilityView(circleButton, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Content);
        }

        // The ring (S3.4): thin white in PREVIEW; red filling clockwise from 12, orange from the warning; the accent while playing.
        ringThin.Visibility = RecorderLook.ShowsTrack(stage) ? Visibility.Visible : Visibility.Collapsed;
        double? fraction = null;
        Brush? ringBrush = null;
        if (stage == RecorderStage.Recording)
        {
            fraction = flow.RingFraction(now);
            ringBrush = (Brush)resources[flow.Warns(now) ? "SystemFillColorCautionBrush" : "SystemFillColorCriticalBrush"];
        }
        else if (review && reviewStarted && reviewPlayer?.PlaybackSession is { NaturalDuration.TotalMilliseconds: > 0 } session)
        {
            fraction = Math.Clamp(session.Position / session.NaturalDuration, 0, 1);
            ringBrush = (Brush)resources["AccentFillColorDefaultBrush"];
        }
        DrawRing(fraction, ringBrush);

        // The reply it will carry.
        var reply = hooks.ReplyText();
        bannerText.Text = reply ?? string.Empty;
        banner.Visibility = reply is null ? Visibility.Collapsed : Visibility.Visible;

        // The control row (S3.4).
        var noPicture = stage is RecorderStage.Opening or RecorderStage.Refused or RecorderStage.Unavailable or RecorderStage.Preview;
        Show(closeButton, noPicture);
        Show(deleteButton, stage is RecorderStage.Recording or RecorderStage.Review);
        Show(retakeButton, stage == RecorderStage.Review);
        Show(cameraButton, stage is RecorderStage.Preview or RecorderStage.Unavailable && RoundVideoRules.OffersCameraChoice(cameras.Count));
        var voice = noPicture && flow.Refused != RecorderRefusal.Microphone;
        Show(voiceButton, voice);
        var notSent = voice && hooks.NotSentWaits();
        voiceButton.Opacity = notSent ? 0.4 : 1;
        AutomationProperties.SetHelpText(voiceButton, notSent ? say.Get("Send or delete the voice message that wasn't sent first.") : string.Empty);
        Show(settingsButton, stage == RecorderStage.Refused && settingsPage is not null);
        DrawSlot(stage);
        KeepFocus();
    }

    /// <summary>
    /// Focus stays on the slot as the states change (S3.4): a control that had it and is now gone — Delete as a short take
    /// goes back to PREVIEW, Retake as REVIEW turns into Opening — would leave it nowhere, with the window beneath
    /// disabled, and Esc and Return, which the layer hears, deaf until a click. Focus somewhere else that is still there —
    /// the reply's ✕, a camera menu, the call card above — is left where it is.
    /// </summary>
    private void KeepFocus()
    {
        if (question is not null || root.XamlRoot is not { } xamlRoot)
        {
            return;
        }
        try
        {
            var focused = FocusManager.GetFocusedElement(xamlRoot) as DependencyObject;
            if (focused is not null && !LostInside(focused))
            {
                return;
            }
            slot.Focus(FocusState.Programmatic);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"keeping focus in the video recorder: {e.GetType().Name}");
        }
    }

    /// <summary>Whether <paramref name="element"/> is inside the layer but no longer shown — itself or a parent collapsed.</summary>
    private bool LostInside(DependencyObject element)
    {
        var hidden = false;
        for (var at = element; at is not null; at = VisualTreeHelper.GetParent(at))
        {
            if (ReferenceEquals(at, root))
            {
                return hidden;
            }
            hidden |= at is UIElement { Visibility: Visibility.Collapsed };
        }
        return false;
    }

    /// <summary>
    /// The one big slot (S3.4, S8.6; the approved design): a red disc to Record — dimmed until the first frame — the same
    /// disc with a white rounded square on it to Stop, and the accent disc with the Send arrow — dimmed until the clip is
    /// ready — with "Record", "Stop" or "Send" under it. Red is the system's critical fill and the accent the app's own.
    /// </summary>
    private void DrawSlot(RecorderStage stage)
    {
        var resources = Application.Current.Resources;
        var shape = RecorderLook.Shape(stage);
        var (name, live, fill) = shape switch
        {
            SlotShape.StopSquare => (say.Get("Stop recording"), true, (Brush)resources["SystemFillColorCriticalBrush"]),
            SlotShape.SendArrow => (say.Get("Send video message"), flow.ClipReady, (Brush)resources["AccentFillColorDefaultBrush"]),
            _ => (say.Get("Record"), stage == RecorderStage.Preview && flow.HasPicture, (Brush)resources["SystemFillColorCriticalBrush"]),
        };
        slotDisc.Fill = fill;
        slotSquare.Visibility = shape == SlotShape.StopSquare ? Visibility.Visible : Visibility.Collapsed;
        slotGlyph.Visibility = shape == SlotShape.SendArrow ? Visibility.Visible : Visibility.Collapsed;
        slotGlyph.Glyph = ((char)ComposerButton.SendGlyph).ToString();
        // On the accent, the ink the accent pairs with — white on the deep shade, black on the pale one the dark theme draws.
        slotGlyph.Foreground = shape == SlotShape.SendArrow && resources.TryGetValue("TextOnAccentFillColorPrimaryBrush", out var ink) && ink is Brush onAccent
            ? onAccent
            : new SolidColorBrush(Colors.White);
        slotCaption.Text = RecorderLook.Caption(stage, say);
        slot.Opacity = live ? 1 : 0.4;
        if (!Equals(AutomationProperties.GetName(slot), name))
        {
            AutomationProperties.SetName(slot, name);
            ToolTipService.SetToolTip(slot, name);
        }
    }

    /// <summary>The ring's arc, clockwise from 12 o'clock to <paramref name="fraction"/>; a whole ring at 1.</summary>
    private void DrawRing(double? fraction, Brush? brush)
    {
        if (fraction is not { } part || brush is null || part <= 0)
        {
            ringArc.Visibility = Visibility.Collapsed;
            ringFull.Visibility = Visibility.Collapsed;
            return;
        }
        var radius = (circle.Width - RoundVideoRules.RingWidth) / 2 - 2;
        var centre = circle.Width / 2;
        if (part >= 0.999)
        {
            ringArc.Visibility = Visibility.Collapsed;
            ringFull.Stroke = brush;
            ringFull.Visibility = Visibility.Visible;
            return;
        }
        var (x, y) = RoundVideoRules.RingPoint(part, radius, centre);
        var figure = new PathFigure { StartPoint = new Point(centre, centre - radius), IsClosed = false };
        figure.Segments.Add(new ArcSegment
        {
            Point = new Point(x, y),
            Size = new Size(radius, radius),
            IsLargeArc = part > 0.5,
            SweepDirection = SweepDirection.Clockwise,
        });
        var geometry = new PathGeometry();
        geometry.Figures.Add(figure);
        ringArc.Data = geometry;
        ringArc.Stroke = brush;
        ringArc.Visibility = Visibility.Visible;
        ringFull.Visibility = Visibility.Collapsed;
    }

    // ---- keys (S3.4, S6) ----------------------------------------------------------------------------------------

    /// <summary>
    /// Caught before the focused control: Esc — Close in PREVIEW, Stop while recording, the question in REVIEW — and Space in
    /// REVIEW, which plays and pauses wherever focus is and never sends, deletes or retakes.
    /// </summary>
    private void OnPreviewKey(object sender, KeyRoutedEventArgs e)
    {
        if (question is not null)
        {
            return;
        }
        if (e.Key == VirtualKey.Escape)
        {
            e.Handled = true;
            Run(flow.Escape(Now));
        }
        else if (e.Key == VirtualKey.Space && flow.CatchesSpace)
        {
            // While the square is still being made too: then it plays nothing, but it must not reach a focused Delete or
            // Retake either (S3.4: Space never sends, deletes or retakes).
            e.Handled = true;
            TogglePlay();
        }
    }

    /// <summary>A button clicks on Space's key-up: in REVIEW that half of the key is the recorder's as well.</summary>
    private void OnPreviewKeyUp(object sender, KeyRoutedEventArgs e)
    {
        if (e.Key == VirtualKey.Space && question is null && flow.CatchesSpace)
        {
            e.Handled = true;
        }
    }

    /// <summary>Return with focus on nothing that takes it — the layer itself — is the slot (S3.4); a focused button answers its own.</summary>
    private void OnKey(object sender, KeyRoutedEventArgs e)
    {
        if (e.Key == VirtualKey.Enter && question is null)
        {
            e.Handled = true;
            Run(flow.Slot(Now));
        }
    }
}
