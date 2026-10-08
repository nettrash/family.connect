using FamilyConnect.Core;

namespace FamilyConnect.App.Logic;

/// <summary>
/// What Windows says about this app's use of the microphone — <c>AppCapability.CheckAccess</c>'s answer, read on the
/// window's side and named here so the decision it feeds can be tested anywhere.
/// </summary>
public enum CapabilityAccess
{
    Allowed,

    /// <summary>Never asked: Windows asks the person the next time the microphone is opened.</summary>
    UserPromptRequired,

    /// <summary>The person turned this app off in Settings.</summary>
    DeniedByUser,

    /// <summary>Windows itself: the microphone switched off for every app, or a policy.</summary>
    DeniedBySystem,

    /// <summary>The manifest does not ask for it — a build fault, which no switch in Settings can mend.</summary>
    NotDeclaredByApp,

    /// <summary>Windows could not be asked.</summary>
    Unknown,
}

/// <summary>
/// A voice note recorded here (docs/protocol.md, "Audio" and "A browser is a client too"): MP4 with AAC in it — never
/// WebM, which the server refuses — five minutes at most, sent from the Send slot, or staged when the member stops it to
/// listen or add words, so a recording made by accident can still be thrown away (docs/audio-video-messages-2026-10-04.md,
/// S2.5).
/// </summary>
/// <remarks>
/// <b>A SECOND IS THE FLOOR PEOPLE SEE</b> (docs/audio-video-messages-2026-10-04.md, S1.1 and decision 15): nothing
/// shorter is ever staged, sent or kept. The 1024 bytes stay as what they always were — a recording with nothing in
/// it — beside it.
/// </remarks>
public static class VoiceNotes
{
    /// <summary>The longest a voice note may run; the recording stops itself there.</summary>
    public static readonly TimeSpan Longest = TimeSpan.FromMinutes(5);

    /// <summary>From here the clock turns orange and "30 seconds left" is shown beside it and announced (S2.5): 4:30.</summary>
    public static readonly TimeSpan WarnFrom = TimeSpan.FromSeconds(270);

    /// <summary>
    /// With a screen reader running, how long the microphone waits after "Recording" is announced (S6: a fixed second where
    /// the platform cannot say when its speech ended), so the app's own voice does not open every note.
    /// </summary>
    public static readonly TimeSpan ScreenReaderLeadIn = TimeSpan.FromSeconds(1);

    /// <summary>The shortest recording that is anything at all (S1.1's "shortest recording"): under it, nothing is kept.</summary>
    public static readonly TimeSpan Shortest = TimeSpan.FromSeconds(1);

    /// <summary>From this long, deleting a recording asks first (S1.1's "delete asks"): it cannot be made again.</summary>
    public static readonly TimeSpan DeleteAsksFrom = TimeSpan.FromSeconds(10);

    /// <summary>A recording this small is nothing — ios <c>AudioRecorder</c>'s 1024 bytes or less.</summary>
    public const int NothingAtOrBelowBytes = 1024;

    /// <summary>What a voice note is uploaded as: the container the server checks, and every family phone plays.</summary>
    public const string Mime = "audio/mp4";

    /// <summary>Where Windows lets the person give this app the microphone back (S2.2).</summary>
    public const string MicrophoneSettingsPage = "ms-settings:privacy-microphone";

    /// <summary>Whether a recording has run as long as a voice note may.</summary>
    public static bool IsDone(TimeSpan elapsed) => elapsed >= Longest;

    /// <summary>Whether a recording has reached its last thirty seconds: words as well as colour (WCAG 1.4.1).</summary>
    public static bool Warns(TimeSpan elapsed) => elapsed >= WarnFrom;

    /// <summary>How long the microphone waits after "Recording" is said — a second when a screen reader runs, else none.</summary>
    public static TimeSpan LeadIn(bool screenReader) => screenReader ? ScreenReaderLeadIn : TimeSpan.Zero;

    /// <summary>Whether deleting a recording this long asks "Delete this recording?" first.</summary>
    public static bool AsksBeforeDeleting(int durationMs) => durationMs >= (int)DeleteAsksFrom.TotalMilliseconds;

    /// <summary>
    /// Whether a staged item is a voice note recorded HERE rather than a sound file picked from disk: both are
    /// <c>kind=audio</c>, and a recording is the one with no name — its duration is its identity (<see cref="Staged"/>).
    /// </summary>
    public static bool IsRecorded(StagedMedia item) => item.Kind == "audio" && item.Name is null;

    /// <summary>
    /// The recording as a staged attachment — its duration its identity, and no name — or null when it is too short to be
    /// anything: under a second, or no more than 1024 bytes. The composer says which.
    /// </summary>
    public static StagedMedia? Staged(ReadOnlyMemory<byte> bytes, TimeSpan elapsed) =>
        bytes.Length <= NothingAtOrBelowBytes || elapsed < Shortest
            ? null
            : new StagedMedia("audio", Mime, bytes, DurationMs: DurationMs(elapsed));

    /// <summary>A recording's length as the attachment carries it: whole milliseconds, half rounded away from zero, at most five minutes.</summary>
    public static int DurationMs(TimeSpan elapsed) =>
        (int)Math.Round(Math.Clamp(elapsed.TotalMilliseconds, 0, Longest.TotalMilliseconds), MidpointRounding.AwayFromZero);

    /// <summary>
    /// Why a recording cannot start right now, or null when it can — in the order the Send slot's own rows put them
    /// (S1.3: an edit, a call, an attachment on its way, a voice message that was not sent), and then the strip's cap.
    /// </summary>
    /// <param name="editing">An edit is open.</param>
    /// <param name="call">A call is on, in any phase (Windows' <c>callBusy</c>).</param>
    /// <param name="busy">Files are being prepared, or a send's files are being written down.</param>
    /// <param name="notSent">This chat holds a voice message that was not sent (S2.8).</param>
    /// <param name="full">The strip holds as many items as one message can.</param>
    public static string? Refusal(bool editing, bool call, bool busy, bool notSent, bool full, IStringCatalog say) =>
        editing ? say.Get("Finish editing before attaching something.")
        : call ? say.Get("You can record a message after the call.")
        : busy ? say.Get("Wait until the current attachment is done.")
        : notSent ? say.Get("Send or delete the voice message that wasn't sent first.")
        : full ? ComposerStaging.CapSentence(say)
        : null;

    /// <summary>
    /// What to say when Windows refused the microphone, and whether to offer the Settings page (S2.2: the iOS sentence
    /// with [Open Settings], after <c>AppCapability.CheckAccess</c>). The page is offered only where a switch on it can
    /// help — the person's own, or Windows' — and when Windows could not say, because the refusal was a refusal. A prompt
    /// closed without an answer says nothing: nothing was taken, and asking again is one click.
    /// </summary>
    public static (string? Sentence, bool OffersSettings) MicrophoneRefusal(CapabilityAccess access, IStringCatalog say) =>
        access switch
        {
            CapabilityAccess.DeniedByUser or CapabilityAccess.DeniedBySystem or CapabilityAccess.Unknown =>
                (say.Get("Family needs permission to use your microphone. Turn it on in Settings."), true),
            CapabilityAccess.UserPromptRequired => (null, false),
            _ => (say.Get("Couldn't start recording."), false),
        };

    /// <summary>
    /// The recording row's clock, "0:42" (S2.4, S2.9): whole seconds gone by, as a stopwatch counts them — so it reads 5:00
    /// only when five minutes have really run, and the row stops there.
    /// </summary>
    public static string Clock(TimeSpan elapsed) =>
        MediaText.TimeLabel(Math.Floor(Math.Max(0, elapsed.TotalSeconds)));

    /// <summary>
    /// Whether a sentence the composer shows is also SAID — its polite live region raised. Never while a recording runs: the
    /// app's own speech stays out of the note (S6), so only the recording's own few announcements are made then, on the
    /// hidden status line. The sentence is on the screen either way, and a dimmed control carries it as its HelpText.
    /// </summary>
    public static bool NoticeSaid(bool recording) => !recording;

    /// <summary>"0:12 / 0:42": a staged or not-sent note while it plays (S2.7) — where it is, and how long it is.</summary>
    public static string PlayingLabel(double elapsedSeconds, double totalSeconds) =>
        $"{MediaText.TimeLabel(Math.Min(elapsedSeconds, totalSeconds))} / {MediaText.TimeLabel(totalSeconds)}";

    /// <summary>
    /// How long a recording is, in seconds, from the attachment — so the scrubber is right before a byte arrives — and
    /// never zero, which would give the scrubber no length at all (ios <c>AudioPlayerView.total</c>).
    /// </summary>
    public static double TotalSeconds(int? durationMs) => Math.Max(0.1, (durationMs ?? 0) / 1000.0);

    /// <summary>
    /// Whether Play starts again from the beginning: at the end, or within a fifth of a second of it — without which the
    /// button would do nothing on a recording that has played through.
    /// </summary>
    public static bool ReplaysFromStart(double elapsedSeconds, double totalSeconds) => elapsedSeconds >= totalSeconds - 0.2;
}

/// <summary>
/// The viewer's video while something records (docs/audio-video-messages-2026-10-04.md, S1.7: no app sound plays while
/// recording). Its controls go and it says why, and its player is muted — so nothing a key or the system's media buttons
/// start can be heard in the note — and as the recording ends the player is left as muted, or not, as it was before.
/// </summary>
public sealed class QuietWhileRecording
{
    private bool? mutedBefore;

    /// <summary>
    /// Whether the player is to be muted now: always while a recording runs — remembering, the first time, whether it was
    /// muted already — and once it has ended, exactly as it was before it began.
    /// </summary>
    /// <param name="recording">A recording runs.</param>
    /// <param name="mutedNow">Whether the player is muted as this is asked.</param>
    public bool Muted(bool recording, bool mutedNow)
    {
        if (recording)
        {
            mutedBefore ??= mutedNow;
            return true;
        }
        var before = mutedBefore ?? mutedNow;
        mutedBefore = null;
        return before;
    }
}

/// <summary>
/// A recording being started (docs/audio-video-messages-2026-10-04.md, S1.7 and S4): from the press until the recorder
/// runs — or the start gives up — there is no recorder yet, and that can be long: Windows' first-time microphone prompt, a
/// screen reader's one-second lead-in.
/// </summary>
/// <remarks>
/// <para>
/// <b>NO APP SOUND FROM THE START ON</b> (S1.7): a play pressed while the microphone is being opened would run on into the
/// note, so playback is refused while a start is under way as it is while a recording runs (<see cref="Quiet"/>).
/// </para>
/// <para>
/// <b>AN INTERRUPTION DURING THE START IS REMEMBERED</b> (S4): a lock, sleep, a call, the window hidden or closed, leaving
/// the chat or a sign-out finds no recorder to stop — so the start notes it, and lets go of the microphone it is granted
/// rather than recording behind a lock screen.
/// </para>
/// </remarks>
public sealed class RecordingStart
{
    /// <summary>Whether a start is under way — so a second press starts nothing more.</summary>
    public bool Starting { get; private set; }

    /// <summary>Whether something interrupted the start under way.</summary>
    public bool WasInterrupted { get; private set; }

    /// <summary>A start begins: nothing heard before it counts against it, since only a start under way notes anything.</summary>
    public void Begin() => Starting = true;

    /// <summary>Something other than the person ends what is recorded: noted against the start under way, if there is one.</summary>
    public void Interrupted()
    {
        if (Starting)
        {
            WasInterrupted = true;
        }
    }

    /// <summary>The start is over — the recorder running, or given up.</summary>
    public void End()
    {
        Starting = false;
        WasInterrupted = false;
    }

    /// <summary>Whether app sound is refused now: while a recording runs or one is being started.</summary>
    public bool Quiet(bool recording) => recording || Starting;
}
