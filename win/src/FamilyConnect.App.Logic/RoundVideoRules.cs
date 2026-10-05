using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

// Recording a VIDEO MESSAGE on Windows (docs/audio-video-messages-2026-10-04.md — "the plan" below — Phase 3d, S3, S4, S6,
// S8.6; docs/protocol.md, "Video messages"): everything about it that needs no camera — the switch, the square, the
// circle's size, which camera and which of its modes, the encodes, what a refusal or a busy camera says, what is sent, and
// the recorder's three states as a machine the window drives (RoundRecorder). The camera itself is VideoMessageRecorder's.

/// <summary>How the live preview is cut round (S3.3, Blocked 1's trial T2) — the first one the trial finds working.</summary>
public enum PreviewClip
{
    /// <summary><c>CornerRadius = D/2</c> on the <c>MediaPlayerElement</c> itself (#8264, reported fixed in WASDK 1.4).</summary>
    CornerRadius,

    /// <summary>An ellipse geometric clip on the element's composition visual.</summary>
    Composition,

    /// <summary>A ring painted over an opaque card: the corners masked in the card's own colour.</summary>
    Mask,
}

/// <summary>Where the recorder's controls stand (S3.3): along the bottom, or — in a pane shorter than 480 — in a column at the trailing edge.</summary>
public enum RecorderLayout
{
    Row,
    Column,
}

/// <summary>The recorder's circle and where its controls go, for one pane.</summary>
public readonly record struct RecorderFit(double Diameter, RecorderLayout Layout);

/// <summary>One mode a camera offers: its size, its frame rate, and the pixel format it delivers it in.</summary>
public readonly record struct CaptureMode(uint Width, uint Height, double FrameRate, string Subtype = "NV12");

/// <summary>The square cut from the middle of a frame — even, so a 4:2:0 picture is never split mid-sample.</summary>
public readonly record struct SquareCrop(uint X, uint Y, uint Side);

/// <summary>Which way a camera faces, as Windows' enclosure location says it.</summary>
public enum CameraPanel
{
    Unknown,
    Front,
    Back,
}

/// <summary>One camera as Windows lists it: its id, the name it is shown by in "Choose camera", and which way it faces.</summary>
public sealed record CameraChoice(string Id, string Name, CameraPanel Panel);

/// <summary>What was already refused before the recorder could start (S3.2).</summary>
public enum RecorderRefusal
{
    Camera,
    Microphone,
}

/// <summary>Why the camera cannot be used although nothing was refused (S3.6).</summary>
public enum CameraTrouble
{
    /// <summary>Another app holds it — a call in another app, the Camera app — or took it away mid-take.</summary>
    InUse,

    /// <summary>It went away, or there is none: a USB webcam pulled out.</summary>
    Missing,

    /// <summary>A camera whose frame-source preview stays blank (#9756: RGB24, UYVY, I420) — never a recording without a picture.</summary>
    Unsupported,

    /// <summary>Anything else Windows threw.</summary>
    Failed,
}

/// <summary>What a finished clip is sent as (S3.6): a video message, or — after the person has been told why — a regular video.</summary>
/// <param name="Round">Sent with <c>round: true</c>.</param>
/// <param name="FromSquare">The square clip is what goes; false when the square pass failed and the camera's own file goes instead.</param>
/// <param name="Notice">What REVIEW says about it, or null when it is a video message as planned.</param>
public sealed record RoundSendPlan(bool Round, bool FromSquare, string? Notice);

/// <summary>The rules of recording a video message on Windows that need no camera (the plan's S3, S3.6, S8.6, the profile).</summary>
/// <remarks>
/// <para>
/// <b>BUILT, AND SWITCHED OFF</b> (<see cref="RecordingEnabled"/>). Nothing at run time can be checked on a Mac or in CI —
/// CI builds the MSIX and never runs it — so Windows records no video message until the owner has run trials T1–T3 on a
/// real machine (Blocked 1, Decision 28). With the switch off the video button, the paperclip's and the microphone menu's
/// "Record Video Message" and the microphone's Narrator hint are never drawn, and nothing here touches a camera: a build
/// that can only RECEIVE circles shows no way of recording one (Decision 40). win/README.md says how to switch it on.
/// </para>
/// <para>
/// <b>RECORDED TO THE PROFILE, NEVER RE-PLANNED</b> ("The recording profile for a round video"): 480 × 480, upright, NOT
/// mirrored, H.264 at 500 000 bit/s and at most 30 fps, AAC-LC mono at 64 000 — 96 000 where Media Foundation's encoder
/// refuses 64 000 (<see cref="MediaEncoding.VoiceNoteAttempts"/>) — <c>moov</c> first, a 480 × 480 poster.
/// </para>
/// </remarks>
public static class RoundVideoRules
{
    /// <summary>
    /// THE SWITCH. False until the owner's trials pass on a real Windows machine (Blocked 1):
    /// T1 — the <c>MediaFrameSource</c> preview shows the webcam's picture, and whether <c>MediaCapture</c> initialised for
    /// <c>AudioAndVideo</c> holds the microphone in PREVIEW (then <see cref="MicrophoneOnInPreview"/> is set true);
    /// T2 — <c>CornerRadius = D/2</c> clips the <c>MediaPlayerElement</c>, else the composition ellipse, else the mask
    /// (<see cref="Clip"/>);
    /// T3 — the square pass writes a 480 × 480 H.264 file with 64 kbit/s AAC that Faststart puts <c>moov</c>-first.
    /// Flip it to true to run them; leave it true only once they pass.
    /// </summary>
    public const bool RecordingEnabled = false;

    /// <summary>
    /// Trial T1's second answer: whether Windows shows the microphone in use while the recorder only previews. When it does,
    /// PREVIEW must not claim the opposite, and says "Camera and microphone on · Not recording" (S3.4).
    /// </summary>
    public const bool MicrophoneOnInPreview = false;

    /// <summary>Trial T2's answer: how the live preview is cut round. The plan's first choice until the trial says otherwise.</summary>
    public static PreviewClip Clip { get; } = PreviewClip.CornerRadius;

    /// <summary>The square's side, in pixels: the centre of the 640 × 480 mode front cameras offer, H.264 level 3.0.</summary>
    public const uint Edge = 480;

    /// <summary>The video bitrate: 2 000 000 × (230 400 ÷ 921 600), the profile's step 3 at the 30 fps target.</summary>
    public const uint VideoBitrate = 500_000;

    /// <summary>The highest frame rate written; a camera delivering fewer is kept as it is.</summary>
    public const double MaxFrameRate = 30;

    /// <summary>The profile's longest gap between keyframes, in seconds (the web's <c>KEYFRAME_SECONDS</c>).</summary>
    public const double KeyframeSeconds = 2;

    /// <summary>The recorder's circle, in effective pixels (S3.3).</summary>
    public const double LargestCircle = 320;
    public const double SmallestCircle = 160;

    /// <summary>The ring just OUTSIDE the circle, so it never covers a face (S3.3).</summary>
    public const double RingWidth = 4;

    /// <summary>PREVIEW turns the camera off after this long with no control used (S1.1).</summary>
    public const long PreviewIdleMs = 60_000;

    /// <summary>How long a picture may stay near-black before PREVIEW asks whether the camera is covered (S3.6).</summary>
    public const long BlackCheckMs = 2_000;

    /// <summary>A frame whose mean luma (0 to 1) is at or under this is near-black — a privacy shutter, not a dark room.</summary>
    public const double BlackLuma = 0.06;

    /// <summary>The pixel formats whose frame-source preview stays blank (microsoft-ui-xaml #9756).</summary>
    public static IReadOnlySet<string> BlankPreviewFormats { get; } =
        new HashSet<string>(["RGB24", "UYVY", "I420"], StringComparer.OrdinalIgnoreCase);

    /// <summary>Settings' camera page, offered on a refusal where a switch there can help (S3.2).</summary>
    public const string CameraSettings = "ms-settings:privacy-webcam";
    public const string MicrophoneSettings = "ms-settings:privacy-microphone";

    // ---- whether there is a way in at all (S1.2, S1.4) ---------------------------------------------------

    /// <summary>
    /// S1.2's <b>round available</b> on Windows: this build records (<see cref="RecordingEnabled"/>), the server named the
    /// limits, and the machine has a camera.
    /// </summary>
    public static bool Available(bool recordingEnabled, RoundVideoLimits? limits, bool hasCamera) =>
        recordingEnabled && limits is not null && hasCamera;

    /// <summary>
    /// What the recorder hears of what ends a voice recording (S4): a call, a hidden or minimised window and a lock as
    /// themselves; leaving the chat — which the recorder covering the window allows only from outside it — as a hidden
    /// window, stopping a take into REVIEW and keeping it; the app going or the session ending as a sign-out, which deletes
    /// (a real close over a clip has asked before it gets here). Null for the person's own endings, which are not the
    /// recorder's business.
    /// </summary>
    public static RecorderInterruption? InterruptionOf(RecordingEnd why) => why switch
    {
        RecordingEnd.Call => RecorderInterruption.Call,
        RecordingEnd.WindowHidden or RecordingEnd.LeftChat => RecorderInterruption.WindowHidden,
        RecordingEnd.SessionLocked => RecorderInterruption.SessionLocked,
        RecordingEnd.WindowClosed or RecordingEnd.SignedOut => RecorderInterruption.SignedOut,
        _ => null,
    };

    /// <summary>Where a take stops on its own: <c>max_round_video_ms</c> − 500, 59.5 s at 60 000.</summary>
    public static long CapMs(RoundVideoLimits limits) => ComposerButton.RoundCapMs(limits.MaxMs);

    /// <summary>Where "10 seconds left" is shown and said: <c>max_round_video_ms</c> − 10 000, 50 s at 60 000.</summary>
    public static long WarningMs(RoundVideoLimits limits) => ComposerButton.RoundWarningMs(limits.MaxMs);

    // ---- the picture ---------------------------------------------------------------------------------------

    /// <summary>
    /// The largest square in the middle of a frame (the profile: a centre crop, never a squash), its corner and side even.
    /// Null for a frame with no pixels.
    /// </summary>
    public static SquareCrop? CentreSquare(uint width, uint height)
    {
        var shortest = Math.Min(width, height);
        if (shortest == 0)
        {
            return null;
        }
        var side = shortest >= 2 ? shortest & ~1u : shortest;
        return new SquareCrop(((width - side) / 2) & ~1u, ((height - side) / 2) & ~1u, side);
    }

    /// <summary>
    /// The recorder's circle for a pane (S3.3): D = min(320, width − 48, height − 240 − the reply banner), at least 160, with
    /// the controls along the bottom; in a pane shorter than 480 the controls stand in a column at the trailing edge and
    /// D = min(320, height − 96, width − 200), at least 160.
    /// </summary>
    public static RecorderFit Fit(double paneWidth, double paneHeight, double bannerHeight)
    {
        if (!double.IsFinite(paneWidth) || !double.IsFinite(paneHeight) || paneWidth <= 0 || paneHeight <= 0)
        {
            return new(SmallestCircle, RecorderLayout.Row);
        }
        var banner = double.IsFinite(bannerHeight) ? Math.Max(0, bannerHeight) : 0;
        if (paneHeight < 480)
        {
            var column = Math.Min(LargestCircle, Math.Min(paneHeight - 96, paneWidth - 200));
            return new(Math.Floor(Math.Max(SmallestCircle, column)), RecorderLayout.Column);
        }
        var row = Math.Min(LargestCircle, Math.Min(paneWidth - 48, paneHeight - 240 - banner));
        return new(Math.Floor(Math.Max(SmallestCircle, row)), RecorderLayout.Row);
    }

    /// <summary>
    /// A point on the ring, clockwise from 12 o'clock (S3.4): where the arc of <paramref name="fraction"/> ends, on a circle of
    /// <paramref name="radius"/> about (<paramref name="centre"/>, <paramref name="centre"/>).
    /// </summary>
    public static (double X, double Y) RingPoint(double fraction, double radius, double centre)
    {
        var turn = 2 * Math.PI * Math.Clamp(double.IsFinite(fraction) ? fraction : 0, 0, 1);
        return (centre + radius * Math.Sin(turn), centre - radius * Math.Cos(turn));
    }

    /// <summary>
    /// Whether a mode's frame-source preview draws at all: every format but the three #9756 leaves blank. A camera that
    /// offers nothing else gets "Video messages can't be recorded with this camera." (S3.6).
    /// </summary>
    public static bool PreviewDraws(string subtype) => !BlankPreviewFormats.Contains(subtype);

    /// <summary>
    /// The mode to capture in: one whose preview draws (<see cref="PreviewDraws"/>), of <paramref name="aspect"/> where one
    /// is asked for and offered; a short side of 480 or more where there is one — the smallest such, so nothing is
    /// upscaled and nothing larger than needed is moved — and otherwise the largest; 24 fps or more where offered; and the
    /// rate nearest 30 without going over, then the nearest over it. Null when nothing draws.
    /// </summary>
    public static CaptureMode? PickMode(IReadOnlyList<CaptureMode> modes, double? aspect = null)
    {
        var drawing = modes.Where(mode => mode.Width > 0 && mode.Height > 0 && PreviewDraws(mode.Subtype)).ToList();
        if (drawing.Count == 0)
        {
            return null;
        }
        if (aspect is { } wanted && drawing.Where(mode => Math.Abs((double)mode.Width / mode.Height - wanted) < 0.01).ToList() is { Count: > 0 } same)
        {
            drawing = same;
        }
        var enough = drawing.Where(mode => Math.Min(mode.Width, mode.Height) >= Edge).ToList();
        var pool = enough.Count > 0 ? enough : drawing;
        var smooth = pool.Where(mode => mode.FrameRate >= 24).ToList();
        if (smooth.Count > 0)
        {
            pool = smooth;
        }
        var bySize = enough.Count > 0
            ? pool.OrderBy(mode => (long)mode.Width * mode.Height)
            : pool.OrderByDescending(mode => (long)mode.Width * mode.Height);
        return bySize
            .ThenBy(mode => mode.FrameRate <= MaxFrameRate + 0.01 ? 0 : 1)
            .ThenBy(mode => Math.Abs(MaxFrameRate - mode.FrameRate))
            .First();
    }

    /// <summary>
    /// The camera to open (S3.5): the one chosen before on this device while it is still there, else the first on the
    /// front panel, else the first Windows lists. Null with no camera.
    /// </summary>
    public static CameraChoice? PickCamera(IReadOnlyList<CameraChoice> cameras, string? remembered) =>
        cameras.FirstOrDefault(camera => remembered is not null && camera.Id == remembered)
        ?? cameras.FirstOrDefault(camera => camera.Panel == CameraPanel.Front)
        ?? cameras.FirstOrDefault();

    /// <summary>"Choose camera" is offered only with more than one to choose from (S3.4, S3.5).</summary>
    public static bool OffersCameraChoice(int cameras) => cameras > 1;

    // ---- the encode ------------------------------------------------------------------------------------------

    /// <summary>
    /// The square's encodes, in the order to ask for them — the profile's H.264 High at 480 × 480, 500 000 bit/s and the
    /// camera's own rate up to 30 fps, upright, SDR, with AAC-LC mono at 64 000 and then the encoder's 96 000; then Main with
    /// the last of those, "Main where an encoder offers nothing else". A clip with no sound track — a microphone that gave
    /// nothing — is asked for without one: a transcode does not invent silence.
    /// </summary>
    /// <remarks>
    /// Each asks for a keyframe at least every <see cref="KeyframeSeconds"/> — at the rate encoded, in whole frames. Media
    /// Foundation's H.264 encoder takes that as <c>MF_MT_MAX_KEYFRAME_SPACING</c> on its output type (trial T3 checks the
    /// gaps); in case one refuses the type for it, the last way is asked once more without it — the encoder's own spacing
    /// rather than no video message at all.
    /// </remarks>
    public static IReadOnlyList<VideoEncoding> Encodes(uint rateNumerator, uint rateDenominator, bool hasAudio)
    {
        var rate = rateNumerator > 0 && rateDenominator > 0 ? (double)rateNumerator / rateDenominator : MaxFrameRate;
        var (numerator, denominator) = MediaEncoding.FrameRateRatio(Math.Min(rate, MaxFrameRate), rateNumerator, rateDenominator);
        var spacing = (uint)Math.Max(1, Math.Floor(KeyframeSeconds * numerator / denominator));
        var first = new VideoEncoding(
            Edge, Edge, 0, VideoBitrate, numerator, denominator, H264Profile.High, null, ToSdr: false, KeyframeSpacing: spacing);
        List<VideoEncoding> attempts;
        if (!hasAudio)
        {
            attempts = [first, first with { Profile = H264Profile.Main }];
        }
        else
        {
            var audio = MediaEncoding.VoiceNoteAttempts();
            attempts = [.. audio.Select(aac => first with { Audio = aac })];
            attempts.Add(first with { Profile = H264Profile.Main, Audio = audio[^1] });
        }
        attempts.Add(attempts[^1] with { KeyframeSpacing = null });
        return attempts;
    }

    /// <summary>
    /// Whether the square pass made what was asked (trial T3 checks the same on a real machine): H.264, exactly 480 × 480,
    /// no turn, its sound track there exactly when one was asked for and AAC, and no faster than asked.
    /// </summary>
    public static bool CameOut(
        VideoEncoding asked, string videoCodec, uint width, uint height, uint rotation, string? audioCodec, double? frameRate) =>
        videoCodec == "h264"
        && MediaEncoding.Matches(asked, width, height, rotation)
        && (asked.Audio is null ? audioCodec is null : audioCodec == "aac")
        && MediaEncoding.FrameRateCameOut(asked, frameRate);

    // ---- what is refused, and what the camera says (S3.2, S3.6) ----------------------------------------------

    /// <summary>
    /// What was refused already, read BEFORE anything is opened (S3.2): either one refused opens the recorder straight to it
    /// and asks Windows nothing. The microphone is named first: with it off, the camera refusal's way out — a voice message
    /// instead — would lead to the same refusal.
    /// </summary>
    public static RecorderRefusal? RefusedBefore(CapabilityAccess camera, CapabilityAccess microphone) =>
        Denied(microphone) ? RecorderRefusal.Microphone : Denied(camera) ? RecorderRefusal.Camera : null;

    /// <summary>
    /// What a refused open was refused for, read once Windows has answered: the one now denied, the microphone first; the
    /// camera when Windows says neither — the camera is the one this recorder is for.
    /// </summary>
    public static RecorderRefusal RefusedAfter(CapabilityAccess camera, CapabilityAccess microphone) =>
        RefusedBefore(camera, microphone) ?? RecorderRefusal.Camera;

    /// <summary>Whether Windows will ask the person when the camera and microphone are opened — and so the neutral "needs" line shows (S3.2).</summary>
    public static bool WillAsk(CapabilityAccess camera, CapabilityAccess microphone) =>
        camera == CapabilityAccess.UserPromptRequired || microphone == CapabilityAccess.UserPromptRequired;

    private static bool Denied(CapabilityAccess access) => access is CapabilityAccess.DeniedByUser or CapabilityAccess.DeniedBySystem;

    /// <summary>
    /// A refusal's sentence, and the Settings page that can mend it (S3.2) — offered unless the manifest itself is at fault,
    /// which no switch in Settings mends.
    /// </summary>
    public static (string Sentence, string? Settings) Refusal(RecorderRefusal refused, CapabilityAccess access, IStringCatalog say) =>
        refused == RecorderRefusal.Camera
            ? (say.Get("Family needs permission to use your camera. Turn it on in Settings."),
               access == CapabilityAccess.NotDeclaredByApp ? null : CameraSettings)
            : (say.Get("Family needs permission to use your microphone. Turn it on in Settings."),
               access == CapabilityAccess.NotDeclaredByApp ? null : MicrophoneSettings);

    /// <summary>
    /// What Media Foundation's HRESULT says about the camera (S3.6) — the codes it raises for a camera another app holds or
    /// took, and for one that is gone. UNCONFIRMED on a real machine: trial T6 opens the camera while a call has it.
    /// </summary>
    public static CameraTrouble Trouble(int hresult) => unchecked((uint)hresult) switch
    {
        // MF_E_HW_MFT_FAILED_START_STREAMING, MF_E_VIDEO_RECORDING_DEVICE_PREEMPTED, ERROR_SHARING_VIOLATION, ERROR_BUSY.
        0xC00D3704 or 0xC00DABE4 or 0x80070020 or 0x800700AA => CameraTrouble.InUse,
        // MF_E_NO_CAPTURE_DEVICES_AVAILABLE, MF_E_VIDEO_RECORDING_DEVICE_INVALIDATED, ERROR_DEVICE_NOT_CONNECTED.
        0xC00DABE0 or 0xC00DABE3 or 0x8007048F => CameraTrouble.Missing,
        _ => CameraTrouble.Failed,
    };

    /// <summary>What PREVIEW says in place of the picture for a camera that cannot be used (S3.6).</summary>
    public static string TroubleSentence(CameraTrouble trouble, IStringCatalog say) => trouble switch
    {
        CameraTrouble.InUse => say.Get("The camera is being used by another app."),
        CameraTrouble.Unsupported => say.Get("Video messages can't be recorded with this camera."),
        _ => say.Get("Something went wrong. Try again."),
    };

    /// <summary>A frame is near-black (S3.6): a privacy shutter or a camera switched off in Settings, never merely a dark room.</summary>
    public static bool LooksBlack(double meanLuma) => double.IsFinite(meanLuma) && meanLuma <= BlackLuma;

    // ---- what is sent (S3.6, "On the wire") -------------------------------------------------------------------

    /// <summary>
    /// What a finished take is sent as: a video message when the square pass made it, it is 1 ms to
    /// <c>max_round_video_ms</c> long and within <c>max_round_video_bytes</c>; the camera's own file as a regular video —
    /// "Couldn't make it round." — when the pass failed or the length cannot be sent round; the square itself as a regular
    /// video — "Too big for a video message." — over the byte ceiling (Rule C: an optimisation never turns a send that
    /// would have worked into one that does not).
    /// </summary>
    public static RoundSendPlan Plan(long? squareBytes, long? durationMs, RoundVideoLimits limits, IStringCatalog say)
    {
        if (squareBytes is not > 0 || durationMs is not { } length || length < 1 || length > limits.MaxMs)
        {
            return new(false, false, say.Get("Couldn't make it round. It will be sent as a regular video."));
        }
        return squareBytes > limits.MaxBytes
            ? new(false, true, say.Get("Too big for a video message. It will be sent as a regular video."))
            : new(true, true, null);
    }

    /// <summary>
    /// The square as the outbox stages it: <c>kind=video</c>, <c>video/mp4</c>, 480 × 480, its length, and its 480 × 480
    /// poster — which the upload sends with <c>PUT /attachments/{id}/preview</c>.
    /// </summary>
    public static StagedMedia Staged(ReadOnlyMemory<byte> square, int durationMs, ReadOnlyMemory<byte>? poster) =>
        new("video", "video/mp4", square, (int)Edge, (int)Edge, durationMs, Preview: poster);

    /// <summary>"Video message · 0:23" — REVIEW's status line (S3.4).</summary>
    public static string ReviewLine(long durationMs, IStringCatalog say) =>
        say.Format("Video message · %@", MediaText.TimeLabel(Math.Max(0, durationMs) / 1000.0));
}

/// <summary>
/// The video button's own guard (S1.1): it ignores activation for 600 ms after it appears, because it appears beside the
/// slot the moment a text Send empties the field, and a second click that drifts left must not turn the camera on.
/// </summary>
public sealed class DoorGuard
{
    private long shownAt = long.MinValue;
    private bool shown;

    /// <summary>The button is drawn (true) or taken away (false) at <paramref name="now"/>; only the moment it APPEARS arms the guard.</summary>
    public void Showing(bool showing, long now)
    {
        if (showing && !shown)
        {
            shownAt = now;
        }
        shown = showing;
    }

    /// <summary>Whether a click at <paramref name="now"/> may open the recorder.</summary>
    public bool Accepts(long now) => shown && now - shownAt >= ComposerButton.ActivationGuardMs;
}

/// <summary>Whether the first two seconds of PREVIEW stayed near-black (S3.6) — decided once, from the frames that came.</summary>
public sealed class PictureCheck
{
    private long? first;
    private bool sawLight;

    /// <summary>The verdict, once made: true for a picture that stayed black.</summary>
    public bool? Black { get; private set; }

    /// <summary>
    /// One frame's mean luma at <paramref name="now"/>. Answers true exactly once, when two seconds of frames were all
    /// near-black; a single lit frame settles it the other way.
    /// </summary>
    public bool Frame(long now, double meanLuma)
    {
        if (Black is not null)
        {
            return false;
        }
        first ??= now;
        sawLight |= !RoundVideoRules.LooksBlack(meanLuma);
        if (sawLight)
        {
            Black = false;
            return false;
        }
        if (now - first.Value < RoundVideoRules.BlackCheckMs)
        {
            return false;
        }
        Black = true;
        return true;
    }
}
