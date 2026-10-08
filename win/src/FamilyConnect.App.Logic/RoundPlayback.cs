using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

// Playing a VIDEO MESSAGE IN PLACE on Windows (docs/audio-video-messages-2026-10-04.md, S5.2–S5.4, S6): a tap plays it inside
// its own circle, at the same size, with sound — what every other client does. Everything about it that needs no player is
// here: the circle's states and what a tap does in each, when its dot goes, how far round its one ring is, which of its
// pieces show, when it is out of view, and how often a frame is copied into the circle. The player itself is the App's
// RoundFramePlayer: a MediaPlayer in FRAME-SERVER mode whose frames are copied into the very ellipse that draws the poster,
// so the picture is round by construction and never by trusting a control to clip a video (microsoft-ui-xaml #8264).

/// <summary>Where one circle is (iOS <c>RoundVideoPlayback.Phase</c>).</summary>
public enum RoundPhase
{
    /// <summary>The poster with the play disc: before the first tap, after the end, after a load given up, after a stop.</summary>
    Rest,

    /// <summary>Tapped and waiting — for the bytes, for the player, or for frames in a stall: the loading ring.</summary>
    Loading,

    /// <summary>Frames are moving, with sound.</summary>
    Playing,

    /// <summary>Paused part way — by a tap, or by something else starting, a call, a lock, a hidden window.</summary>
    Paused,

    /// <summary>"Couldn't load the video. Tap to try again."</summary>
    Failed,
}

/// <summary>What a tap on a circle did — the view says the one that needs words.</summary>
public enum RoundTap
{
    /// <summary>It began to load (from rest, after a failure, or instead of another circle).</summary>
    Started,

    /// <summary>A paused circle plays on.</summary>
    Resumed,

    /// <summary>A playing circle paused.</summary>
    Paused,

    /// <summary>A second tap while it loads gives up (S5.3): back to the poster.</summary>
    GaveUp,

    /// <summary>A recording runs: nothing of the app's plays under it (S1.7) — "You can play this after recording."</summary>
    Refused,
}

/// <summary>What the view does to the one player for a step of the machine.</summary>
public enum RoundAct
{
    /// <summary>Nothing.</summary>
    None,

    /// <summary>Let go of any player, then fetch this circle's video and open a new one (the load is <see cref="RoundPlayback.Load"/>).</summary>
    Load,

    /// <summary>Play the player there is — after pausing whatever else plays.</summary>
    Play,

    /// <summary>Pause the player, keeping it and its last frame.</summary>
    Pause,

    /// <summary>Let go of the player: the circle is back to its poster.</summary>
    Stop,
}

/// <summary>One step of a circle's machine.</summary>
/// <param name="Tap">What the tap did, when it was a tap.</param>
/// <param name="Act">What the view does to the player.</param>
/// <param name="Replaced">Another circle this step took the player from — it goes back to its poster and is redrawn.</param>
public readonly record struct RoundStep(RoundTap Tap, RoundAct Act, long? Replaced = null);

/// <summary>
/// The ONE circle that plays in place, and every rule of moving it between <see cref="RoundPhase"/>s — the iPhone's
/// <c>RoundVideoPlayback</c> and the web's round tile, as a machine the window drives: a tap plays, a tap pauses, a second tap
/// while it loads gives up, a failure waits for a tap to try again, the end returns it to its poster (and says whether its dot
/// goes), one thing plays at a time (<see cref="Yield"/>), a call, a lock or a hidden window pauses it (<see cref="Interrupt"/>),
/// and its row leaving the screen or the chat closing stops it (<see cref="Stop"/>).
/// </summary>
/// <remarks>
/// <para>
/// <b>ONE AT A TIME, BY CONSTRUCTION.</b> There is one machine per conversation view and it names one circle
/// (<see cref="Id"/>); tapping another takes the player from the first, which goes back to its poster
/// (<see cref="RoundStep.Replaced"/>).
/// </para>
/// <para>
/// <b>STALE NEWS IS IGNORED.</b> Every load has its own number (<see cref="Load"/>); whatever the player reports carries the
/// number of the load it belongs to, and news of a load that was given up, replaced or stopped changes nothing — a download
/// that lands after a second tap never starts playing.
/// </para>
/// </remarks>
public sealed class RoundPlayback
{
    /// <summary>The circle — the attachment's id — this machine is about, or null when none is.</summary>
    public long? Id { get; private set; }

    /// <summary>Where it is.</summary>
    public RoundPhase Phase { get; private set; } = RoundPhase.Rest;

    /// <summary>The number of the current load; news carrying another is stale.</summary>
    public int Load { get; private set; }

    /// <summary>Frames have been drawn into the circle for this load, so the picture is worth showing over the poster.</summary>
    public bool HasFrames { get; private set; }

    /// <summary>This load has been seen playing — what the dot and the ring wait for.</summary>
    public bool SeenPlaying { get; private set; }

    /// <summary>Where it is, in seconds.</summary>
    public double Position { get; private set; }

    /// <summary>How long it is, in seconds; 0 while unknown.</summary>
    public double Total { get; private set; }

    /// <summary>Loading, playing or paused: a player is (or is about to be) held for it.</summary>
    public bool Active => Phase is RoundPhase.Loading or RoundPhase.Playing or RoundPhase.Paused;

    /// <summary>Where the circle <paramref name="id"/> is: its own phase when it is this machine's, at rest otherwise.</summary>
    public RoundPhase PhaseOf(long id) => Id == id ? Phase : RoundPhase.Rest;

    /// <summary>
    /// A tap on the circle <paramref name="id"/> (the single tap, after the double-click window that is the heart):
    /// playing, it pauses; loading, it gives up; paused, it plays on; at rest or failed, it loads — taking the player from any
    /// other circle. Nothing STARTS while a recording runs (S1.7), but a pause is always allowed.
    /// </summary>
    /// <param name="id">The circle tapped.</param>
    /// <param name="recording">A voice recording runs, or is being started.</param>
    public RoundStep Tap(long id, bool recording)
    {
        if (Id == id)
        {
            switch (Phase)
            {
                case RoundPhase.Playing:
                    Phase = RoundPhase.Paused;
                    return new(RoundTap.Paused, RoundAct.Pause);
                case RoundPhase.Loading:
                    // "A second tap while it loads gives up" (S5.3) — the bytes, if they land, land on nothing.
                    Rest();
                    return new(RoundTap.GaveUp, RoundAct.Stop);
                case RoundPhase.Paused:
                    if (recording)
                    {
                        return new(RoundTap.Refused, RoundAct.None);
                    }
                    // A stall's frames are still there; until the player says it plays, it is loading.
                    Phase = HasFrames ? RoundPhase.Playing : RoundPhase.Loading;
                    return new(RoundTap.Resumed, RoundAct.Play);
            }
        }
        if (recording)
        {
            return new(RoundTap.Refused, RoundAct.None);
        }
        var replaced = Id is { } other && other != id ? other : (long?)null;
        Id = id;
        Phase = RoundPhase.Loading;
        Load++;
        HasFrames = false;
        SeenPlaying = false;
        Position = 0;
        Total = 0;
        return new(RoundTap.Started, RoundAct.Load, replaced);
    }

    /// <summary>
    /// The player has opened load <paramref name="load"/>: whether to play it now — only when it is still this machine's load
    /// and still waited for (a pause meanwhile keeps it paused).
    /// </summary>
    public bool Opened(int load, double total)
    {
        if (load != Load || Phase != RoundPhase.Loading)
        {
            return false;
        }
        Total = Finite(total);
        return true;
    }

    /// <summary>The player says load <paramref name="load"/> is playing.</summary>
    public void Playing(int load)
    {
        if (load != Load || !Active)
        {
            return;
        }
        SeenPlaying = true;
        Phase = RoundPhase.Playing;
    }

    /// <summary>The player paused load <paramref name="load"/> by itself — the system, not a tap.</summary>
    public void PausedByPlayer(int load)
    {
        if (load == Load && Phase == RoundPhase.Playing)
        {
            Phase = RoundPhase.Paused;
        }
    }

    /// <summary>The player is waiting for bytes in the middle of load <paramref name="load"/>: the loading ring again.</summary>
    public void Waiting(int load)
    {
        if (load == Load && Phase == RoundPhase.Playing)
        {
            Phase = RoundPhase.Loading;
        }
    }

    /// <summary>A frame of load <paramref name="load"/> is in the circle: from now the picture shows over the poster.</summary>
    public void FirstFrame(int load)
    {
        if (load == Load && Active)
        {
            HasFrames = true;
        }
    }

    /// <summary>Where load <paramref name="load"/> is: the ring and the clock follow it.</summary>
    public void Progress(int load, double position, double total)
    {
        if (load != Load || !Active)
        {
            return;
        }
        if (double.IsFinite(total) && total > 0)
        {
            Total = total;
        }
        Position = Math.Clamp(Finite(position), 0, Total > 0 ? Total : double.MaxValue);
    }

    /// <summary>
    /// Load <paramref name="load"/> played to its end: back to the poster and the play disc (S5.3), the player let go — and
    /// whether this was a PLAY-THROUGH, so the circle's dot goes: it was seen playing here. A clip that never started (an end
    /// reported straight after opening) does not count.
    /// </summary>
    public (RoundAct Act, bool PlayedThrough) Ended(int load)
    {
        if (load != Load || !Active)
        {
            return (RoundAct.None, false);
        }
        var through = SeenPlaying;
        Rest();
        return (RoundAct.Stop, through);
    }

    /// <summary>Load <paramref name="load"/> could not be fetched or played: "Couldn't load the video. Tap to try again."</summary>
    public RoundAct Failed(int load)
    {
        if (load != Load || !Active)
        {
            return RoundAct.None;
        }
        Phase = RoundPhase.Failed;
        HasFrames = false;
        SeenPlaying = false;
        Position = 0;
        return RoundAct.Stop;
    }

    /// <summary>
    /// Something else began to play — a voice note, the viewer, a recording — so this one makes way (one thing at a time,
    /// S5.3): playing, or stalled with frames up, it pauses where it is; still loading with nothing drawn, it gives up.
    /// </summary>
    public RoundAct Yield()
    {
        switch (Phase)
        {
            case RoundPhase.Loading when !HasFrames:
                Rest();
                return RoundAct.Stop;
            case RoundPhase.Playing or RoundPhase.Loading:
                Phase = RoundPhase.Paused;
                return RoundAct.Pause;
            default:
                return RoundAct.None;
        }
    }

    /// <summary>
    /// Something happened to the app (S4's last column, <see cref="PlaybackPauses"/>): a call, a lock, a change of output
    /// device or a hidden window makes it <see cref="Yield"/>; anything else leaves it.
    /// </summary>
    public RoundAct Interrupt(PlaybackEvent happened) =>
        PlaybackPauses.Pauses(happened, Logic.Playing.RoundVideo) ? Yield() : RoundAct.None;

    /// <summary>
    /// Its row left the screen, its chat closed, the window let go of the view: back to the poster, the player let go. Returns
    /// the circle that was stopped, or null when there was nothing to stop.
    /// </summary>
    public long? Stop()
    {
        if (Id is not { } id)
        {
            return null;
        }
        Rest();
        Id = null;
        return id;
    }

    private void Rest()
    {
        Phase = RoundPhase.Rest;
        // A load given up must not be revived by its own late news.
        Load++;
        HasFrames = false;
        SeenPlaying = false;
        Position = 0;
    }

    private static double Finite(double value) => double.IsFinite(value) && value > 0 ? value : 0;
}

/// <summary>How a circle that plays in place is drawn (S5.2, S5.3, S6) — what shows in each <see cref="RoundPhase"/>.</summary>
public static class RoundInline
{
    /// <summary>
    /// THE FLAG. True: a tap plays the circle in place, through the frame-server player, and only Open Full Screen opens the
    /// viewer. False: the interim of 2026-10-04, a tap opens the viewer (S5.3's old "Windows in this version"). It is also what
    /// a machine whose frame server fails falls back to, circle by circle, on its own.
    /// </summary>
    public static bool PlaysInPlace { get; } = true;

    /// <summary>How often the circle's clock redraws the ring and the time while it plays, in milliseconds.</summary>
    public const int TickMs = 100;

    /// <summary>The expand control's glyph: a 28 dark disc carrying Segoe's FullScreen (S5.4).</summary>
    public const double ExpandGlyph = 28;

    /// <summary>The expand control's target (S5.4, S1.1).</summary>
    public const double ExpandTarget = 44;

    /// <summary>The gap between the circle's edge and its ring, which runs OUTSIDE the picture, never over a face.</summary>
    public const double RingGap = 3;

    /// <summary>How far the ring reaches past the circle on every side.</summary>
    public const double RingReach = RingGap + RoundLook.Ring;

    /// <summary>
    /// How far round the ONE accent ring is, 0 to 1 (S5.3): as far as it has played while it plays, is paused or stalls —
    /// stepping once a second, not sweeping, where Windows' animations are off (S6, Reduce Motion) — and nothing at rest, while
    /// loading before it ever played, or after a failure.
    /// </summary>
    public static double Ring(RoundPhase phase, bool seenPlaying, double position, double total, bool reducedMotion)
    {
        if (phase is RoundPhase.Rest or RoundPhase.Failed || !seenPlaying
            || !double.IsFinite(total) || total <= 0 || !double.IsFinite(position))
        {
            return 0;
        }
        var at = reducedMotion ? Math.Floor(position) : position;
        return Math.Clamp(at / total, 0, 1);
    }

    /// <summary>The play disc: on the poster at rest, paused or failed; faded out while it loads or plays (S5.3).</summary>
    public static bool ShowsDisc(RoundPhase phase) => phase is RoundPhase.Rest or RoundPhase.Paused or RoundPhase.Failed;

    /// <summary>The loading ring over the poster: from the tap until it plays, and in a stall (S5.3).</summary>
    public static bool ShowsSpinner(RoundPhase phase) => phase == RoundPhase.Loading;

    /// <summary>The frames, not the poster, fill the circle: once one has been drawn, until it ends, fails or stops.</summary>
    public static bool ShowsFrames(RoundPhase phase, bool hasFrames) =>
        hasFrames && phase is RoundPhase.Playing or RoundPhase.Paused or RoundPhase.Loading;

    /// <summary>The expand control at the top trailing edge — Open Full Screen — while it plays or is paused part way (S5.4).</summary>
    public static bool ShowsExpand(RoundPhase phase, bool hasFrames) => ShowsFrames(phase, hasFrames);

    /// <summary>"Couldn't load the video. Tap to try again." under the circle.</summary>
    public static bool ShowsFailure(RoundPhase phase) => phase == RoundPhase.Failed;

    /// <summary>The capsule: how far it has played once it has started, its whole length otherwise ("0:23").</summary>
    public static string? Capsule(AttachmentDto video, RoundPhase phase, bool seenPlaying, double position) =>
        seenPlaying && phase is RoundPhase.Playing or RoundPhase.Paused or RoundPhase.Loading
            ? MediaText.TimeLabel(Math.Max(0, double.IsFinite(position) ? position : 0))
            : RoundLook.Capsule(video);

    /// <summary>
    /// What a press does next, for a screen reader (S6: Play/Pause is the default action) and the tooltip: "Pause" while it
    /// plays or loads — a press there pauses or gives up — "Play" otherwise.
    /// </summary>
    public static string Action(RoundPhase phase, IStringCatalog say) =>
        phase is RoundPhase.Playing or RoundPhase.Loading ? say.Get("Pause") : say.Get("Play");

    /// <summary>
    /// Whether any of a circle is on the screen: its effective viewport — the part of its parent scrollers it is seen through,
    /// in its own coordinates — overlaps its own <paramref name="width"/> × <paramref name="height"/>. An empty viewport is off
    /// the screen; an unbounded one (no scroller round it) is on it. A circle wholly out of view stops (S5.3), as the web's
    /// IntersectionObserver does.
    /// </summary>
    public static bool InView(double x, double y, double viewWidth, double viewHeight, double width, double height) =>
        Overlaps(x, viewWidth, width) && Overlaps(y, viewHeight, height);

    private static bool Overlaps(double start, double length, double size)
    {
        if (double.IsNaN(start) || double.IsNaN(length) || length <= 0 || size <= 0)
        {
            return false;
        }
        if (double.IsPositiveInfinity(length))
        {
            return true;
        }
        if (!double.IsFinite(start))
        {
            return false;
        }
        return start < size && start + length > 0;
    }
}

/// <summary>
/// How the frame-server player's frames are copied into the circle: at the clip's own rate and never faster than the profile's
/// 30 (S5.2's 480 × 480 at 30 fps — "The recording profile for a round video"), one copy at a time, into a surface the size of
/// the circle on this screen.
/// </summary>
public static class RoundFrames
{
    /// <summary>The fastest the circle is redrawn: the profile's own highest rate.</summary>
    public const double MaxRate = RoundVideoRules.MaxFrameRate;

    /// <summary>The rate a clip that does not say is drawn at.</summary>
    public const double DefaultRate = 30;

    /// <summary>The least share of the interval that must have passed, so a frame arriving a little early is not skipped.</summary>
    public const double Slack = 0.75;

    /// <summary>Copies in a row that may fail before the circle gives up on the frame server and opens the viewer instead.</summary>
    public const int FailuresBeforeViewer = 3;

    /// <summary>
    /// How far a clip may play, in seconds, with no frame drawn into the circle before it gives up on the frame server and opens
    /// the viewer instead — sound with a still poster is not playing a video.
    /// </summary>
    public const double FirstFrameWithinSeconds = 2;

    /// <summary>Whether the circle is playing blind: playing for <see cref="FirstFrameWithinSeconds"/> or more, and no frame yet.</summary>
    public static bool Starved(RoundPhase phase, bool hasFrames, double position) =>
        phase == RoundPhase.Playing && !hasFrames && double.IsFinite(position) && position >= FirstFrameWithinSeconds;

    /// <summary>The largest frame surface, in pixels — twice the recording's 480, for a 240 circle on a 400 % screen.</summary>
    public const int LargestSide = 960;

    /// <summary>The time between two copies, in milliseconds: the clip's rate — the default when it says none — never over 30.</summary>
    public static double IntervalMs(double? clipRate)
    {
        var rate = clipRate is { } given && double.IsFinite(given) && given > 0 ? given : DefaultRate;
        return 1000.0 / Math.Min(rate, MaxRate);
    }

    /// <summary>
    /// Whether the frame the player just offered is copied: never while a copy is still under way, always for the first, and
    /// otherwise once most of an interval has passed since the last copy (<see cref="Slack"/>) — so a 30 fps clip is drawn at
    /// 30 and a 60 fps one at 30, not 15.
    /// </summary>
    /// <param name="now">The clock now, in milliseconds.</param>
    /// <param name="last">When the last copy began, or null before the first.</param>
    /// <param name="intervalMs">From <see cref="IntervalMs"/>.</param>
    /// <param name="copying">A copy is still under way.</param>
    public static bool Due(long now, long? last, double intervalMs, bool copying)
    {
        if (copying)
        {
            return false;
        }
        if (last is not { } before)
        {
            return true;
        }
        var interval = double.IsFinite(intervalMs) && intervalMs > 0 ? intervalMs : IntervalMs(null);
        // A clock that went backwards copies rather than freezing the picture.
        return now < before || now - before >= interval * Slack;
    }

    /// <summary>
    /// The frame surface's side in physical pixels: the circle at this screen's scale, even (the decoder's 4:2:0 never splits
    /// a sample), at least 16 and at most <see cref="LargestSide"/>.
    /// </summary>
    public static int Side(double diameter, double scale)
    {
        var s = double.IsFinite(scale) && scale > 0 ? scale : 1;
        var d = double.IsFinite(diameter) && diameter > 0 ? diameter : RoundLook.Diameter;
        var side = (int)Math.Round(d * s);
        side = Math.Clamp(side, 16, LargestSide);
        return side & ~1;
    }

    /// <summary>
    /// A copied frame made OPAQUE, in place: every fourth byte of premultiplied BGRA — the alpha — set to 255. A video has no
    /// transparency, but a Direct3D surface the player copied into need not say so, and a frame drawn with the alpha it came
    /// with could show the grey disc through a face. Opaque premultiplied BGRA is its own colour, so nothing else changes.
    /// </summary>
    public static void Opaque(Span<byte> bgra)
    {
        for (var i = 3; i < bgra.Length; i += 4)
        {
            bgra[i] = 0xFF;
        }
    }

    /// <summary>A clip's frame rate from its encoding properties' ratio, or null when it does not say.</summary>
    public static double? Rate(uint numerator, uint denominator) =>
        numerator > 0 && denominator > 0 ? (double)numerator / denominator : null;
}

/// <summary>
/// The circle that played in place took the whole app down last time: a native failure inside XAML or the media stack ends the
/// process with no exception any handler sees (0xc000027b), so the view writes a MARK before it starts a circle in place and
/// clears it when it lets the player go. A mark still there at the next launch, left by THIS SAME BUILD, means that build died
/// playing in place, and its circles open the viewer from then on; a new build tries in place again.
/// </summary>
public static class RoundCrashGuard
{
    /// <summary>The mark's file name, beside diagnostics.log.</summary>
    public const string FileName = "round-inline.mark";

    /// <summary>What the mark says: which build was playing a circle in place.</summary>
    public static string Mark(string build) => $"playing in place: {build}";

    /// <summary>Whether a mark left behind says this very build died playing in place.</summary>
    public static bool Tripped(string? held, string build) =>
        !string.IsNullOrWhiteSpace(build) && string.Equals(held?.Trim(), Mark(build), StringComparison.Ordinal);
}
