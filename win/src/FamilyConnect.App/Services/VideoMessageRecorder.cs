using System.Diagnostics;
using System.Runtime.InteropServices.WindowsRuntime;
using FamilyConnect.App.Logic;
using Windows.Devices.Enumeration;
using Windows.Graphics.Imaging;
using Windows.Media.Capture;
using Windows.Media.Capture.Frames;
using Windows.Media.Core;
using Windows.Media.Devices;
using Windows.Media.MediaProperties;
using Windows.Media.Playback;
using Windows.Security.Authorization.AppCapabilityAccess;
using Windows.Storage;

namespace FamilyConnect.App.Services;

/// <summary>Why the camera did not open: refused (and which), or troubled (and how).</summary>
internal sealed record CameraOpenFailure(RecorderRefusal? Refused, CameraTrouble? Trouble);

/// <summary>A finished take: the camera's own MP4 on disk, and how long it ran by a monotonic clock.</summary>
internal sealed record VideoTake(string Path, TimeSpan Elapsed);

/// <summary>
/// The camera of a VIDEO MESSAGE (docs/audio-video-messages-2026-10-04.md, S3; "Where it plugs in", Windows Phase 3): the
/// front-panel camera through <c>MediaCapture</c> initialised for <c>AudioAndVideo</c>, its live picture for the recorder's
/// circle through <see cref="MediaSource.CreateFromMediaFrameSource"/>, and a take written to a temporary MP4 at the camera's
/// own size — which <see cref="MediaPreparing.RoundAsync"/> then makes square.
/// </summary>
/// <remarks>
/// <para>
/// <b>HOWEVER IT ENDS — CLOSED, REFUSED, FAILED, OR THE WINDOW GOING — THE CAMERA AND THE MICROPHONE ARE LET GO OF</b>, as
/// <see cref="VoiceRecorder"/> lets go of the microphone: every path ends in <see cref="DisposeAsync"/>, which stops a take,
/// stops the frame reader, takes the picture off the player and disposes the capture. A camera light nothing can reach is a
/// camera left on.
/// </para>
/// <para>
/// <b>NONE OF THIS HAS RUN.</b> Nothing at run time can be checked on a Mac or in CI (Blocked 1). T1 checks that the frame
/// source's preview shows the webcam's picture and whether initialising for <c>AudioAndVideo</c> shows the microphone in
/// use before Record; T3 checks the square; T6 opens the camera while a call has it. Until they pass,
/// <see cref="RoundVideoRules.RecordingEnabled"/> keeps every way in hidden, and nothing here is ever constructed.
/// </para>
/// <para>
/// <b>A CAMERA WHOSE PREVIEW WOULD STAY BLANK IS NEVER RECORDED WITH</b> (#9756: RGB24, UYVY and I420): a mode in another
/// format is chosen where the camera offers one, and otherwise the camera is refused with "Video messages can't be recorded
/// with this camera." — a <c>MediaFrameReader</c> fallback for such cameras waits until one is at hand to try it on.
/// </para>
/// </remarks>
internal sealed class VideoMessageRecorder : IAsyncDisposable
{
    private static readonly string Folder = Path.Combine(Path.GetTempPath(), "FamilyConnect", "round");

    private MediaCapture? capture;
    private MediaFrameReader? reader;
    private MediaPlayer? player;
    private LowLagMediaRecording? recording;
    private string? takePath;
    private readonly Stopwatch clock = new();
    private readonly Stopwatch sinceFirstFrame = new();
    private int sawFrame;

    private VideoMessageRecorder()
    {
    }

    /// <summary>The live picture, for the circle's <c>MediaPlayerElement</c>: muted, real-time, the frame source's.</summary>
    public MediaPlayer? Preview => player;

    /// <summary>The first frame arrived (S3.4: Record comes alive). Raised once, on whatever thread Windows delivered it.</summary>
    public event Action? FirstFrame;

    /// <summary>A frame's mean luma, 0 to 1, for the first seconds of the picture (S3.6's covered camera); any thread.</summary>
    public event Action<double>? Brightness;

    /// <summary>Windows blocked the picture for privacy — a shutter, the system's camera switch (S3.6); any thread.</summary>
    public event Action? Blocked;

    /// <summary>The camera failed, was taken, or went (S3.6, S4); any thread, at most once.</summary>
    public event Action<CameraTrouble>? Troubled;

    /// <summary>How long the take has run, by a monotonic clock.</summary>
    public TimeSpan Elapsed => clock.Elapsed;

    /// <summary>What Windows says about this app and the camera (<c>AppCapability.CheckAccess</c>, S3.2).</summary>
    public static CapabilityAccess CameraAccess()
    {
        try
        {
            // The manifest's own spelling: <DeviceCapability Name="webcam" />.
            return AppCapability.Create("webcam").CheckAccess() switch
            {
                AppCapabilityAccessStatus.Allowed => CapabilityAccess.Allowed,
                AppCapabilityAccessStatus.UserPromptRequired => CapabilityAccess.UserPromptRequired,
                AppCapabilityAccessStatus.DeniedByUser => CapabilityAccess.DeniedByUser,
                AppCapabilityAccessStatus.DeniedBySystem => CapabilityAccess.DeniedBySystem,
                AppCapabilityAccessStatus.NotDeclaredByApp => CapabilityAccess.NotDeclaredByApp,
                _ => CapabilityAccess.Unknown,
            };
        }
        catch (Exception e)
        {
            Diagnostics.Write($"asking about the camera: {e.GetType().Name} 0x{e.HResult:X8}");
            return CapabilityAccess.Unknown;
        }
    }

    /// <summary>The cameras Windows lists, with the names "Choose camera" shows and which way each faces (S3.5). Empty when it cannot say.</summary>
    public static async Task<IReadOnlyList<CameraChoice>> CamerasAsync()
    {
        try
        {
            var found = await DeviceInformation.FindAllAsync(DeviceClass.VideoCapture);
            return [.. found.Where(device => device.IsEnabled).Select(device => new CameraChoice(
                device.Id,
                device.Name,
                device.EnclosureLocation?.Panel switch
                {
                    Windows.Devices.Enumeration.Panel.Front => CameraPanel.Front,
                    Windows.Devices.Enumeration.Panel.Back => CameraPanel.Back,
                    _ => CameraPanel.Unknown,
                }))];
        }
        catch (Exception e)
        {
            Diagnostics.Write($"listing cameras: {e.GetType().Name} 0x{e.HResult:X8}");
            return [];
        }
    }

    /// <summary>
    /// Open <paramref name="deviceId"/> for a video message: Windows asks for the camera and the microphone the first time
    /// (S3.2), the mode is chosen (<see cref="RoundVideoRules.PickMode"/>), and the live picture starts. NOTHING RECORDS:
    /// the take begins at <see cref="StartAsync"/>.
    /// </summary>
    public static async Task<(VideoMessageRecorder? Recorder, CameraOpenFailure? Failure)> OpenAsync(string deviceId)
    {
        var recorder = new VideoMessageRecorder { capture = new MediaCapture() };
        try
        {
            await recorder.capture.InitializeAsync(new MediaCaptureInitializationSettings
            {
                VideoDeviceId = deviceId,
                StreamingCaptureMode = StreamingCaptureMode.AudioAndVideo,
                SharingMode = MediaCaptureSharingMode.ExclusiveControl,
                // Frames in system memory, so the first seconds can be read for a covered camera (S3.6).
                MemoryPreference = MediaCaptureMemoryPreference.Cpu,
            });
        }
        catch (UnauthorizedAccessException)
        {
            await recorder.DisposeAsync();
            return (null, new CameraOpenFailure(
                RoundVideoRules.RefusedAfter(CameraAccess(), VoiceRecorder.MicrophoneAccess()), null));
        }
        catch (Exception e)
        {
            Diagnostics.Write($"opening the camera: {e.GetType().Name} 0x{e.HResult:X8}");
            await recorder.DisposeAsync();
            return (null, new CameraOpenFailure(null, RoundVideoRules.Trouble(e.HResult)));
        }
        try
        {
            if (await recorder.StartPictureAsync() is { } trouble)
            {
                await recorder.DisposeAsync();
                return (null, new CameraOpenFailure(null, trouble));
            }
            recorder.capture.Failed += recorder.OnCaptureFailed;
            recorder.capture.CameraStreamStateChanged += recorder.OnStreamStateChanged;
            recorder.capture.CaptureDeviceExclusiveControlStatusChanged += recorder.OnControlChanged;
            return (recorder, null);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"starting the camera's picture: {e.GetType().Name} 0x{e.HResult:X8}");
            await recorder.DisposeAsync();
            return (null, new CameraOpenFailure(null, RoundVideoRules.Trouble(e.HResult)));
        }
    }

    /// <summary>
    /// The frame source the circle shows — colour, the preview stream where the camera has one, else the record stream —
    /// in a mode whose preview draws, and the record stream in the same shape; then the player, and the frame reader that
    /// says when the picture arrives. A trouble when no mode draws (#9756).
    /// </summary>
    private async Task<CameraTrouble?> StartPictureAsync()
    {
        var sources = capture!.FrameSources.Values.Where(source => source.Info.SourceKind == MediaFrameSourceKind.Color).ToList();
        var source = sources.FirstOrDefault(source => source.Info.MediaStreamType == MediaStreamType.VideoPreview)
            ?? sources.FirstOrDefault(source => source.Info.MediaStreamType == MediaStreamType.VideoRecord);
        if (source is null)
        {
            Diagnostics.Write("a camera with no colour frame source");
            return CameraTrouble.Unsupported;
        }
        var formats = source.SupportedFormats.Where(format => format.VideoFormat is not null).ToList();
        var modes = formats.Select(Mode).ToList();
        if (RoundVideoRules.PickMode(modes) is not { } chosen)
        {
            Diagnostics.Write($"a camera offers no mode whose preview draws: {string.Join(", ", formats.Select(format => format.Subtype).Distinct())}");
            return CameraTrouble.Unsupported;
        }
        if (source.CurrentFormat is not { } current || Mode(current) != chosen)
        {
            await source.SetFormatAsync(formats[modes.IndexOf(chosen)]);
        }
        await MatchRecordStreamAsync((double)chosen.Width / chosen.Height);
        Diagnostics.Write($"the camera shows {chosen.Width}x{chosen.Height} {chosen.Subtype} at {chosen.FrameRate:0.##} fps");

        try
        {
            reader = await capture.CreateFrameReaderAsync(source);
            reader.AcquisitionMode = MediaFrameReaderAcquisitionMode.Realtime;
            reader.FrameArrived += OnFrameArrived;
            if (await reader.StartAsync() != MediaFrameReaderStartStatus.Success)
            {
                Diagnostics.Write("the camera's frame reader did not start; the first frame is the player's");
                reader.FrameArrived -= OnFrameArrived;
                reader.Dispose();
                reader = null;
            }
        }
        catch (Exception e)
        {
            // The picture does not need it: without it the player says when it plays, and nothing asks whether it is black.
            Diagnostics.Write($"the camera's frame reader: {e.GetType().Name} 0x{e.HResult:X8}");
            reader = null;
        }

        player = new MediaPlayer { RealTimePlayback = true, AutoPlay = true, IsMuted = true };
        if (reader is null)
        {
            player.PlaybackSession.PlaybackStateChanged += OnPlaybackStateChanged;
        }
        player.Source = MediaSource.CreateFromMediaFrameSource(source);
        return null;
    }

    private static CaptureMode Mode(MediaFrameFormat format) => new(
        format.VideoFormat.Width,
        format.VideoFormat.Height,
        format.FrameRate is { Denominator: > 0 } rate ? (double)rate.Numerator / rate.Denominator : 0,
        format.Subtype);

    /// <summary>
    /// The record stream in the preview's shape, where the camera keeps the two apart — so the square cut from the take is
    /// the square the circle showed. A camera that refuses keeps its own, and the take is still cut from its middle.
    /// </summary>
    private async Task MatchRecordStreamAsync(double aspect)
    {
        try
        {
            var controller = capture!.VideoDeviceController;
            var offered = controller.GetAvailableMediaStreamProperties(MediaStreamType.VideoRecord)
                .OfType<VideoEncodingProperties>()
                .ToList();
            var modes = offered.Select(properties => new CaptureMode(
                properties.Width,
                properties.Height,
                properties.FrameRate is { Denominator: > 0 } rate ? (double)rate.Numerator / rate.Denominator : 0,
                properties.Subtype)).ToList();
            if (RoundVideoRules.PickMode(modes, aspect) is { } chosen)
            {
                await controller.SetMediaStreamPropertiesAsync(MediaStreamType.VideoRecord, offered[modes.IndexOf(chosen)]);
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"choosing the camera's record mode: {e.GetType().Name} 0x{e.HResult:X8}");
        }
    }

    private void OnFrameArrived(MediaFrameReader sender, MediaFrameArrivedEventArgs args)
    {
        using var frame = sender.TryAcquireLatestFrame();
        if (frame?.VideoMediaFrame is not { } video)
        {
            return;
        }
        if (Interlocked.Exchange(ref sawFrame, 1) == 0)
        {
            sinceFirstFrame.Start();
            FirstFrame?.Invoke();
        }
        // Only for as long as anybody asks (S3.6's first two seconds, and a little over): reading frames costs a CPU.
        if (Brightness is { } listen && sinceFirstFrame.ElapsedMilliseconds <= RoundVideoRules.BlackCheckMs + 500
            && video.SoftwareBitmap is { } bitmap && MeanLuma(bitmap) is { } luma)
        {
            listen(luma);
        }
    }

    private void OnPlaybackStateChanged(MediaPlaybackSession session, object args)
    {
        if (session.PlaybackState == MediaPlaybackState.Playing && Interlocked.Exchange(ref sawFrame, 1) == 0)
        {
            FirstFrame?.Invoke();
        }
    }

    /// <summary>A frame's mean brightness, 0 to 1, from every eighth pixel of its grey copy. Null when Windows cannot convert it.</summary>
    private static double? MeanLuma(SoftwareBitmap bitmap)
    {
        try
        {
            using var grey = SoftwareBitmap.Convert(bitmap, BitmapPixelFormat.Gray8);
            var bytes = new byte[grey.PixelWidth * grey.PixelHeight];
            grey.CopyToBuffer(bytes.AsBuffer());
            long sum = 0;
            var count = 0;
            for (var at = 0; at < bytes.Length; at += 8)
            {
                sum += bytes[at];
                count++;
            }
            return count == 0 ? null : sum / (255.0 * count);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"reading a camera frame's brightness: {e.GetType().Name}");
            return null;
        }
    }

    /// <summary>
    /// Open the microphone and start the take (S3.4: the clip begins at Record), into a temporary MP4 at the camera's own
    /// size and Windows' own quality — it is made square and re-encoded to the profile afterwards. False when it would not
    /// start; the camera stays on.
    /// </summary>
    public async Task<bool> StartAsync()
    {
        if (capture is null || recording is not null)
        {
            return false;
        }
        try
        {
            Directory.CreateDirectory(Folder);
            Sweep();
            takePath = Path.Combine(Folder, $"{Guid.NewGuid():N}.mp4");
            await File.WriteAllBytesAsync(takePath, []);
            var file = await StorageFile.GetFileFromPathAsync(takePath);
            recording = await capture.PrepareLowLagRecordToStorageFileAsync(MediaEncodingProfile.CreateMp4(VideoEncodingQuality.Auto), file);
            await recording.StartAsync();
            clock.Restart();
            capture.RecordLimitationExceeded += OnLimitReached;
            return true;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"starting a video message: {e.GetType().Name} 0x{e.HResult:X8}");
            await AbandonTakeAsync();
            return false;
        }
    }

    /// <summary>
    /// Stop the take and hand it over — the file is the caller's from here — or null when nothing could be finished.
    /// The camera stays on: <see cref="DisposeAsync"/> turns it off (REVIEW), or a Delete that goes back to PREVIEW keeps it.
    /// </summary>
    public async Task<VideoTake?> StopAsync()
    {
        var elapsed = clock.Elapsed;
        clock.Stop();
        if (recording is not { } running || takePath is not { } path)
        {
            return null;
        }
        recording = null;
        takePath = null;
        if (capture is { } owned)
        {
            owned.RecordLimitationExceeded -= OnLimitReached;
        }
        try
        {
            await running.StopAsync();
            await running.FinishAsync();
            return new FileInfo(path) is { Exists: true, Length: > 0 } ? new VideoTake(path, elapsed) : Gone(path);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"finishing a video message: {e.GetType().Name} 0x{e.HResult:X8}");
            return Gone(path);
        }

        static VideoTake? Gone(string path)
        {
            Delete(path);
            return null;
        }
    }

    /// <summary>Stop the take and throw it away; the camera stays on.</summary>
    public async Task DiscardAsync() => await AbandonTakeAsync();

    private async Task AbandonTakeAsync()
    {
        clock.Stop();
        if (capture is { } owned)
        {
            owned.RecordLimitationExceeded -= OnLimitReached;
        }
        if (recording is { } running)
        {
            recording = null;
            try
            {
                await running.StopAsync();
                await running.FinishAsync();
            }
            catch (Exception e)
            {
                Diagnostics.Write($"abandoning a video message: {e.GetType().Name}");
            }
        }
        if (takePath is { } path)
        {
            takePath = null;
            Delete(path);
        }
    }

    /// <summary>A take or a square nobody will send: gone from the disk.</summary>
    public static void Delete(string? path)
    {
        if (path is null)
        {
            return;
        }
        try
        {
            File.Delete(path);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"removing a video message's file: {e.GetType().Name}");
        }
    }

    /// <summary>Where a REVIEW clip is played from: beside the takes, gone with the recorder.</summary>
    public static async Task<string?> WriteClipAsync(ReadOnlyMemory<byte> bytes)
    {
        try
        {
            Directory.CreateDirectory(Folder);
            var path = Path.Combine(Folder, $"{Guid.NewGuid():N}.mp4");
            await File.WriteAllBytesAsync(path, bytes.ToArray());
            return path;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"writing a video message to play: {e.GetType().Name}");
            return null;
        }
    }

    /// <summary>What an earlier run left — a crash mid-take — gone after a day. A REVIEW clip lives only while the app runs (S4).</summary>
    private static void Sweep()
    {
        foreach (var stale in Directory.EnumerateFiles(Folder))
        {
            try
            {
                if (File.GetLastWriteTimeUtc(stale) < DateTime.UtcNow.AddDays(-1))
                {
                    File.Delete(stale);
                }
            }
            catch (Exception e)
            {
                Diagnostics.Write($"sweeping a video message: {e.GetType().Name}");
            }
        }
    }

    private void OnCaptureFailed(MediaCapture sender, MediaCaptureFailedEventArgs args)
    {
        // The code only: the message is Windows' words, and may name the device.
        Diagnostics.Write($"the camera failed: 0x{args.Code:X8}");
        RaiseTroubled(RoundVideoRules.Trouble(unchecked((int)args.Code)));
    }

    private void OnStreamStateChanged(MediaCapture sender, object args)
    {
        // Another app taking the camera shuts this stream down; a privacy shutter or the system's camera switch blocks it.
        // UNCONFIRMED until trial T6 — which of these Windows raises, and when.
        switch (sender.CameraStreamState)
        {
            case CameraStreamState.Shutdown:
                Diagnostics.Write("the camera's stream was shut down");
                RaiseTroubled(CameraTrouble.InUse);
                break;
            case CameraStreamState.BlockedForPrivacy:
                Diagnostics.Write("the camera's stream is blocked for privacy");
                Blocked?.Invoke();
                break;
        }
    }

    private void OnControlChanged(MediaCapture sender, MediaCaptureDeviceExclusiveControlStatusChangedEventArgs args)
    {
        if (args.Status == MediaCaptureDeviceExclusiveControlStatus.SharedReadOnlyAvailable)
        {
            Diagnostics.Write("another app took the camera");
            RaiseTroubled(CameraTrouble.InUse);
        }
    }

    private void OnLimitReached(MediaCapture sender)
    {
        Diagnostics.Write("a video message reached Windows' own limit");
        RaiseTroubled(CameraTrouble.Failed);
    }

    private void RaiseTroubled(CameraTrouble trouble)
    {
        // Once: a failure followed by a shutdown is one camera that went.
        if (Interlocked.Exchange(ref Troubled, null) is { } troubled)
        {
            troubled(trouble);
        }
    }

    /// <summary>
    /// Let go of everything — the take (thrown away unless <see cref="StopAsync"/> handed it over), the frame reader, the
    /// picture, the camera and the microphone. Safe to call more than once, and from every path.
    /// </summary>
    public async ValueTask DisposeAsync()
    {
        FirstFrame = null;
        Brightness = null;
        Blocked = null;
        Troubled = null;
        await AbandonTakeAsync();
        if (capture is { } owned)
        {
            owned.Failed -= OnCaptureFailed;
            owned.CameraStreamStateChanged -= OnStreamStateChanged;
            owned.CaptureDeviceExclusiveControlStatusChanged -= OnControlChanged;
        }
        if (reader is { } frames)
        {
            reader = null;
            frames.FrameArrived -= OnFrameArrived;
            try
            {
                await frames.StopAsync();
            }
            catch (Exception e)
            {
                Diagnostics.Write($"stopping the camera's frame reader: {e.GetType().Name}");
            }
            frames.Dispose();
        }
        if (player is { } shown)
        {
            player = null;
            shown.PlaybackSession.PlaybackStateChanged -= OnPlaybackStateChanged;
            try
            {
                shown.Pause();
                (shown.Source as IDisposable)?.Dispose();
                shown.Source = null;
            }
            catch (Exception e)
            {
                Diagnostics.Write($"taking the camera's picture away: {e.GetType().Name}");
            }
            shown.Dispose();
        }
        capture?.Dispose();
        capture = null;
    }
}
