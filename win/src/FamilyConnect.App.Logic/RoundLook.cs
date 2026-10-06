using FamilyConnect.Core.Protocol;
using FamilyConnect.Core;

namespace FamilyConnect.App.Logic;

/// <summary>
/// How a VIDEO MESSAGE is drawn on this client (docs/audio-video-messages-2026-10-04.md, S5.2, S5.3, S6) — a drawing
/// rule, not a wire one: the circle's size, what is written on it, what a screen reader hears, and the size of the
/// circle the viewer plays it in.
/// </summary>
/// <remarks>
/// <para>
/// <b>NO BALLOON, AND ONE SIZE.</b> The circle alone on the chat background, like a sticker — 240 on Windows, which is
/// never a compact width (S5.2): larger than a sticker (160), smaller than a video tile. The square poster fills it,
/// and until it lands a neutral disc of the same size holds the row's height.
/// </para>
/// <para>
/// <b>A TAP PLAYS IT IN PLACE</b> (S5.3; 2026-10-06 — <see cref="RoundInline"/>, <see cref="RoundPlayback"/>): inside its own
/// circle, at the same size, with sound. <b>OPEN FULL SCREEN PLAYS IT IN THE VIEWER</b> (S5.4): the square clip with its
/// corners painted in the viewer's solid backdrop, exactly one accent ring OUTSIDE the edge running round as it plays (the
/// approved design of 2026-10-05), and a play disc that fades out while it does.
/// </para>
/// </remarks>
public static class RoundLook
{
    /// <summary>The circle in a conversation, in effective pixels (S5.2: Windows is never compact).</summary>
    public const double Diameter = 240;

    /// <summary>The play disc in the middle of the circle: the approved design's 48 (the 44 target the circle itself exceeds).</summary>
    public const double PlayDisc = 48;

    /// <summary>The accent dot beside the duration while this device has not played it.</summary>
    public const double Dot = 8;

    /// <summary>The ring outside the edge: the accent as it plays — in place or in the viewer — a neutral one while one of the reader's own uploads.</summary>
    public const double Ring = 3;

    /// <summary>The largest circle the viewer draws: the recording's own 480 pixels.</summary>
    public const double ViewerLargest = 480;

    /// <summary>The smallest circle the viewer draws, however small the window: the conversation's own size and no less.</summary>
    public const double ViewerSmallest = 200;

    /// <summary>"0:23", the capsule at the bottom of the circle — or nothing when the sender did not say how long.</summary>
    public static string? Capsule(AttachmentDto video) =>
        video.DurationMs is > 0 and var ms ? MediaText.TimeLabel(ms / 1000.0) : null;

    /// <summary>What a screen reader hears for the circle: "Video message, 0:23" (S6).</summary>
    public static string Name(AttachmentDto video, IStringCatalog say) =>
        Capsule(video) is { } length ? say.Format("Video message, %@", length) : say.Get("Video message");

    /// <summary>
    /// Whether the accent dot shows (S5.2): only on SOMEONE ELSE'S circle that this device has not played — never on the
    /// reader's own, which no other client marks either.
    /// </summary>
    public static bool ShowsDot(bool mine, bool played) => !mine && !played;

    /// <summary>
    /// Its value: "Not played" while the dot shows and "Played" once this device has played it — on someone else's circle
    /// only; the reader's own say neither (S6), as a voice message's do not (<see cref="VoiceLook.Status"/>).
    /// </summary>
    public static string? Value(bool mine, bool played, IStringCatalog say) =>
        mine ? null : played ? say.Get("Played") : say.Get("Not played");

    /// <summary>How near the end a clip counts as played through: the viewer clock's own tick, and Play's restart rule.</summary>
    public const double EndSlackSeconds = 0.25;

    /// <summary>
    /// Whether this opening has PLAYED IT THROUGH, so its dot goes (S5.3: "at the end it returns to the poster and loses its
    /// dot") — it was seen playing here and its position has reached the end. Starting it is not enough, and nor is a seek
    /// to the end of a clip never played; a clip of unknown length is never through.
    /// </summary>
    /// <param name="seenPlaying">Whether this opening has seen it playing.</param>
    /// <param name="position">Where it is, in seconds.</param>
    /// <param name="total">How long it is, in seconds — 0 or less when unknown.</param>
    public static bool PlayedThrough(bool seenPlaying, double position, double total) =>
        seenPlaying && double.IsFinite(total) && total > 0 && double.IsFinite(position)
            && position >= total - EndSlackSeconds;

    /// <summary>
    /// How far round the viewer's ONE accent ring is drawn, 0 to 1: nothing before this opening has seen it play, as far as it
    /// has played while it plays or is paused part way — and NOTHING once it has finished, when the circle is back to its
    /// poster and play disc (the approved design hides the ring at the end rather than leaving a full one round it).
    /// </summary>
    /// <param name="seenPlaying">Whether this opening has seen it playing.</param>
    /// <param name="playing">Whether it is playing now.</param>
    /// <param name="position">Where it is, in seconds.</param>
    /// <param name="total">How long it is, in seconds — 0 or less when unknown.</param>
    public static double ViewerRing(bool seenPlaying, bool playing, double position, double total)
    {
        if (!seenPlaying || !double.IsFinite(total) || total <= 0 || !double.IsFinite(position))
        {
            return 0;
        }
        if (!playing && position >= total - EndSlackSeconds)
        {
            return 0;
        }
        return Math.Clamp(position / total, 0, 1);
    }

    /// <summary>
    /// The circle the viewer plays it in: as large as the room left by the viewer's own bars allows, never larger than
    /// the recording's 480 pixels and never smaller than the conversation's circle — a window too small for even that
    /// scrolls nothing and simply clips, as the viewer's video always has.
    /// </summary>
    /// <param name="width">The viewer's width.</param>
    /// <param name="height">The viewer's height.</param>
    /// <param name="chrome">What the bars above and below the circle take of the height.</param>
    public static double ViewerDiameter(double width, double height, double chrome)
    {
        const double Margin = 24;
        if (!double.IsFinite(width) || !double.IsFinite(height) || width <= 0 || height <= 0)
        {
            return ViewerLargest;
        }
        var room = Math.Min(width - 2 * Margin, height - Math.Max(0, chrome) - 2 * Margin);
        return Math.Floor(Math.Clamp(room, ViewerSmallest, ViewerLargest));
    }
}

/// <summary>The shape the recorder's one big slot button takes (the approved design: Record → Stop → Send).</summary>
public enum SlotShape
{
    /// <summary>A plain red disc: Record.</summary>
    RecordDisc,

    /// <summary>A red disc with a white rounded square on it: Stop.</summary>
    StopSquare,

    /// <summary>An accent disc with the Send arrow: Send.</summary>
    SendArrow,
}

/// <summary>
/// How the video-message recorder is drawn (the approved design of 2026-10-05, "Recording a video message"): ONE big slot
/// button where Send is, smaller round buttons with captions beside it, and the ring outside the circle — a thin track all
/// the way round, red filling over the minute while it records, the accent while the clip plays back in REVIEW.
/// </summary>
public static class RecorderLook
{
    /// <summary>The big slot button: 64 across, with a faint 4-wide halo round it.</summary>
    public const double Slot = 64;

    /// <summary>The halo round the slot.</summary>
    public const double Halo = 4;

    /// <summary>Stop's white rounded square on the red disc, and its corners.</summary>
    public const double StopSquare = 22;

    /// <summary>The corner radius of Stop's square.</summary>
    public const double StopCorner = 5;

    /// <summary>The smaller round buttons beside the slot — Close or Delete, Switch camera or Retake.</summary>
    public const double Side = 44;

    /// <summary>The thin track ring's stroke, drawn where the progress ring runs.</summary>
    public const double Track = 1;

    /// <summary>The slot's shape in each stage: Stop while recording, Send in review, Record otherwise.</summary>
    public static SlotShape Shape(RecorderStage stage) => stage switch
    {
        RecorderStage.Recording => SlotShape.StopSquare,
        RecorderStage.Review => SlotShape.SendArrow,
        _ => SlotShape.RecordDisc,
    };

    /// <summary>The caption under the slot: "Record", "Stop" or "Send".</summary>
    public static string Caption(RecorderStage stage, IStringCatalog say) => Shape(stage) switch
    {
        SlotShape.StopSquare => say.Get("Stop"),
        SlotShape.SendArrow => say.Get("Send"),
        _ => say.Get("Record"),
    };

    /// <summary>The slot's target: the disc and its halo.</summary>
    public const double SlotTarget = Slot + 2 * Halo;

    /// <summary>The gap between a round button and its caption.</summary>
    public const double CaptionGap = 2;

    /// <summary>Whether the thin track ring is drawn: wherever the circle shows a picture or a clip.</summary>
    public static bool ShowsTrack(RecorderStage stage) =>
        stage is RecorderStage.Preview or RecorderStage.Recording or RecorderStage.Review;
}

/// <summary>What is playing through this client's shared player, or its viewer.</summary>
public enum Playing
{
    /// <summary>A voice note or an audio file in a bubble, or a staged or not-sent note.</summary>
    VoiceNote,

    /// <summary>A video message — on Windows in place, in its own circle (S5.3), or in the viewer's circle (S5.4).</summary>
    RoundVideo,

    /// <summary>An ordinary video in the viewer.</summary>
    Video,
}

/// <summary>Something that happens to the app while something plays (S4's last column).</summary>
public enum PlaybackEvent
{
    /// <summary>A call rings, starts or is placed — any phase but idle.</summary>
    Call,

    /// <summary>The session locks, the screen saver starts, or the computer goes to sleep.</summary>
    SessionLocked,

    /// <summary>The window is minimised, or hidden to the notification area by its close button with Keep running on.</summary>
    WindowHidden,

    /// <summary>The default output device changed — headphones unplugged, a Bluetooth headset gone or come.</summary>
    OutputChanged,

    /// <summary>The window only lost focus and is still visible.</summary>
    FocusLost,
}

/// <summary>
/// Which interruptions pause what plays (docs/audio-video-messages-2026-10-04.md, S4, "Playing (a voice note or a
/// circle)"; S8.6): a call, a session lock and a change of the default output device pause a voice note and a circle; a
/// hidden window pauses a circle and lets a voice note play on; losing focus pauses nothing.
/// </summary>
/// <remarks>
/// An ORDINARY video in the viewer is not in S4's column and is left as it always was — this version changes nothing
/// about it. Leaving the chat, a real close and a sign-out STOP everything, which the view already does on its own.
/// </remarks>
public static class PlaybackPauses
{
    /// <summary>Whether <paramref name="happened"/> pauses <paramref name="playing"/>.</summary>
    public static bool Pauses(PlaybackEvent happened, Playing playing) =>
        playing != Playing.Video && happened switch
        {
            PlaybackEvent.Call or PlaybackEvent.SessionLocked or PlaybackEvent.OutputChanged => true,
            // A circle is something one looks at; a voice note is heard from anywhere (S4's minimised row).
            PlaybackEvent.WindowHidden => playing == Playing.RoundVideo,
            _ => false,
        };

    /// <summary>
    /// The playback side of what interrupts a recording — the one place the window hears a call, a lock or a hidden
    /// window — or null where the view already STOPS everything (leaving the chat, closing, signing out) or the end was
    /// the person's own.
    /// </summary>
    public static PlaybackEvent? Of(RecordingEnd why) => why switch
    {
        RecordingEnd.Call => PlaybackEvent.Call,
        RecordingEnd.SessionLocked => PlaybackEvent.SessionLocked,
        RecordingEnd.WindowHidden => PlaybackEvent.WindowHidden,
        _ => null,
    };
}
