using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>Where the video-message recorder is (docs/audio-video-messages-2026-10-04.md, S3.4) — the plan's three states and the ways around them.</summary>
public enum RecorderStage
{
    /// <summary>The camera is being opened — and, the first time, Windows is asking for it.</summary>
    Opening,

    /// <summary>The camera or the microphone was refused (S3.2): the camera is off and the sentence says where to mend it.</summary>
    Refused,

    /// <summary>The camera cannot be used — another app has it, it went, or its preview would stay blank (S3.6).</summary>
    Unavailable,

    /// <summary>The live front camera, mirrored; the microphone not opened; Record.</summary>
    Preview,

    /// <summary>Recording: the camera and the microphone on, the ring filling, Stop.</summary>
    Recording,

    /// <summary>The clip as it will be sent, the camera off; Send, Delete, Retake.</summary>
    Review,

    /// <summary>Gone: the camera off and nothing held.</summary>
    Closed,
}

/// <summary>A question the recorder has put up, and what its answer is about (S3.4).</summary>
public enum RecorderQuestion
{
    None,

    /// <summary>Delete during a take of ten seconds or more: it stopped first; Keep goes to REVIEW.</summary>
    DeleteWhileRecording,

    /// <summary>Delete in REVIEW, ten seconds or more: Delete closes the recorder.</summary>
    DeleteReview,

    /// <summary>Retake in REVIEW, ten seconds or more: Delete turns the camera back on.</summary>
    Retake,

    /// <summary>Esc in REVIEW, which always asks: Delete closes the recorder.</summary>
    Escape,

    /// <summary>The window really closing over a clip (S4): Delete lets it close; Keep cancels the close.</summary>
    CloseWindow,
}

/// <summary>Something other than the person that reaches the recorder (S4's three video columns).</summary>
public enum RecorderInterruption
{
    /// <summary>A call rings, starts or is placed.</summary>
    Call,

    /// <summary>The window minimised, or hidden to the notification area.</summary>
    WindowHidden,

    /// <summary>The session locked, the screen saver started, or the computer is going to sleep.</summary>
    SessionLocked,

    /// <summary>The window only lost focus and is still visible.</summary>
    FocusLost,

    /// <summary>The window really closing: Quit, or the close button with Keep running off.</summary>
    WindowClosing,

    /// <summary>The session ended — signed out, the family left, the server changed.</summary>
    SignedOut,
}

/// <summary>What the window does for one step of the recorder.</summary>
public enum RecorderAction
{
    /// <summary>Open the camera (PREVIEW again after a Retake, or after a clip that could not be read).</summary>
    OpenCamera,

    /// <summary>Let go of the camera and the microphone — the light goes out.</summary>
    CloseCamera,

    /// <summary>Open the microphone and start writing the take.</summary>
    StartRecording,

    /// <summary>Stop the take and keep it: finish the file and make the square.</summary>
    StopAndKeep,

    /// <summary>Stop the take and throw it away.</summary>
    StopAndDiscard,

    /// <summary>Throw away the finished clip.</summary>
    DiscardClip,

    /// <summary>Pause the clip REVIEW is playing back — a call, a lock, a hidden window, the output gone (S4's last column).</summary>
    PauseClip,

    /// <summary>Put up "Delete video message?" [Delete] [Keep].</summary>
    Ask,

    /// <summary>Take the question away unanswered — another one replaces it.</summary>
    Dismiss,

    /// <summary>Send the clip — round or not, as its plan says — with the composer's primed reply.</summary>
    Send,

    /// <summary>Start a hands-free voice recording in the composer.</summary>
    StartVoice,

    /// <summary>Close the window now — the person deleted the clip that was holding it.</summary>
    CloseWindow,

    /// <summary>Take the recorder away and give focus back to whatever opened it.</summary>
    Closed,

    /// <summary>Say <see cref="RecorderEffect.Text"/> to a screen reader — politely, and only state changes (S6).</summary>
    Announce,
}

/// <summary>One thing for the window to do, with what it says where it says something.</summary>
public readonly record struct RecorderEffect(RecorderAction Action, string? Text = null);

/// <summary>What the status line says (S3.4): the line itself, and the one under it — a first-time hint, a warning or a notice.</summary>
public readonly record struct RecorderStatus(string Line, string? Below, bool Red, bool Warning);

/// <summary>
/// The video-message recorder as a machine (docs/audio-video-messages-2026-10-04.md, S3.4, S3.6, S4): every press, tick and
/// interruption goes in with the time on one monotonic clock, and what the window must do comes out — so the rules of the
/// three states are tested here, on a Mac, and the window only draws and runs the camera.
/// </summary>
/// <remarks>
/// <para>
/// <b>THE CAMERA IS OFF IN EVERY REFUSAL AND IN REVIEW, AND THE MICROPHONE OPENS ONLY AT RECORD</b> (S3.2, S3.4): a closed
/// recorder holds nothing, and PREVIEW never records.
/// </para>
/// <para>
/// <b>NOTHING IS SENT BUT BY SEND</b> (S1.7): the length limit stops into REVIEW and says so; an interruption stops into
/// REVIEW, or closes PREVIEW; a real close over a clip asks first.
/// </para>
/// <para>
/// <b>THE SLOT GUARDS ITS OWN PRESSES</b> (S1.1): for 600 ms after Record or Stop changed it, a click on the slot is ignored, so
/// a double click on Record cannot stop a take and a double click on Stop cannot send one.
/// </para>
/// </remarks>
public sealed class RoundRecorder(RoundVideoLimits limits, IStringCatalog say)
{
    private long recordingAt;
    private bool takeStarted;
    private bool stoppedByInterruption;
    private long lastUsed;
    private long guardUntil = long.MinValue;
    private bool warned;

    public RecorderStage Stage { get; private set; } = RecorderStage.Closed;

    public RecorderQuestion Question { get; private set; }

    /// <summary>The camera has delivered its first frame: Record is live (S3.4).</summary>
    public bool HasPicture { get; private set; }

    /// <summary>The first two seconds stayed near-black (S3.6).</summary>
    public bool LooksCovered { get; private set; }

    /// <summary>Windows is asking for the camera and microphone right now — losing focus to that is not leaving (S4).</summary>
    public bool Asking { get; private set; }

    /// <summary>The finished clip is ready to play and send; false while its square is still being made.</summary>
    public bool ClipReady { get; private set; }

    /// <summary>How long the clip in REVIEW is.</summary>
    public long ClipMs { get; private set; }

    /// <summary>What was refused (<see cref="RecorderStage.Refused"/>) and why the camera cannot be used (<see cref="RecorderStage.Unavailable"/>).</summary>
    public RecorderRefusal? Refused { get; private set; }

    public CameraTrouble? Trouble { get; private set; }

    /// <summary>What the clip will be sent as, once it is known.</summary>
    public RoundSendPlan? Plan { get; private set; }

    /// <summary>A sentence shown under the status line until the next change: "That video was too short.", "Recording stopped at one minute."</summary>
    public string? Notice { get; private set; }

    /// <summary>Whether something recorded would be lost by closing: a take running, or a clip in REVIEW (S4).</summary>
    public bool HoldsClip => Stage is RecorderStage.Recording or RecorderStage.Review;

    public bool IsOpen => Stage != RecorderStage.Closed;

    /// <summary>
    /// Space is the recorder's, caught before the focused control (S3.4): in REVIEW it plays and pauses wherever focus is —
    /// while the square is still being made too, when it does nothing — and never sends, deletes or retakes. Not while the
    /// question is up: that is the question's.
    /// </summary>
    public bool CatchesSpace => Stage == RecorderStage.Review && Question == RecorderQuestion.None;

    /// <summary>
    /// The take's length so far, or the clip's. A take counts from <see cref="TakeStarted"/> — when the file really began,
    /// after a screen reader's lead-in and the camera's own start — never from the press (S6: the timer starts with it).
    /// </summary>
    public long ElapsedMs(long now) => Stage switch
    {
        RecorderStage.Recording => takeStarted ? Math.Max(0, now - recordingAt) : 0,
        RecorderStage.Review => ClipMs,
        _ => 0,
    };

    // ---- opening ---------------------------------------------------------------------------------------------

    /// <summary>
    /// Opened from the video button, the paperclip or the menu (S3.1). Something already refused opens straight to it and
    /// asks Windows nothing (S3.2); otherwise the camera is opened, with the neutral line while Windows asks.
    /// </summary>
    public IReadOnlyList<RecorderEffect> Open(long now, RecorderRefusal? refusedAlready, bool windowsWillAsk)
    {
        Reset(now);
        if (refusedAlready is { } refused)
        {
            Stage = RecorderStage.Refused;
            Refused = refused;
            return [];
        }
        Stage = RecorderStage.Opening;
        Asking = windowsWillAsk;
        return [new(RecorderAction.OpenCamera)];
    }

    /// <summary>The camera is on and its preview attached; the picture follows (<see cref="FirstFrame"/>).</summary>
    public IReadOnlyList<RecorderEffect> CameraOn(long now)
    {
        if (Stage != RecorderStage.Opening)
        {
            // Opened after the recorder closed or moved on: let go of it again.
            return Stage is RecorderStage.Closed or RecorderStage.Review ? [new(RecorderAction.CloseCamera)] : [];
        }
        Stage = RecorderStage.Preview;
        Asking = false;
        lastUsed = now;
        return [];
    }

    /// <summary>The first frame arrived: Record comes alive and "Camera ready" is said.</summary>
    public IReadOnlyList<RecorderEffect> FirstFrame()
    {
        if (Stage != RecorderStage.Preview || HasPicture)
        {
            return [];
        }
        HasPicture = true;
        return [Say(say.Get("Camera ready"))];
    }

    /// <summary>The picture stayed near-black for two seconds (S3.6): said in the status line; Record stays usable.</summary>
    public void Covered()
    {
        if (Stage == RecorderStage.Preview)
        {
            LooksCovered = true;
        }
    }

    /// <summary>Windows refused the camera or the microphone while opening (S3.2): the camera is off.</summary>
    public IReadOnlyList<RecorderEffect> Refuse(RecorderRefusal refused)
    {
        if (Stage is not (RecorderStage.Opening or RecorderStage.Preview))
        {
            return [];
        }
        Stage = RecorderStage.Refused;
        Refused = refused;
        Asking = false;
        return [new(RecorderAction.CloseCamera)];
    }

    /// <summary>
    /// The camera cannot be used, or stopped being usable (S3.6): PREVIEW says why in place of the picture, Record dimmed and
    /// a voice message offered; a take in progress stops into REVIEW with what was recorded and the same sentence.
    /// </summary>
    public IReadOnlyList<RecorderEffect> CameraTroubled(CameraTrouble trouble, long now)
    {
        switch (Stage)
        {
            case RecorderStage.Opening or RecorderStage.Preview:
                Stage = RecorderStage.Unavailable;
                Trouble = trouble;
                Asking = false;
                HasPicture = false;
                return [new(RecorderAction.CloseCamera)];
            case RecorderStage.Recording:
                // Taken by another app: the camera's own sentence (S3.6); gone or failed: what S4 says of a recorder that fails.
                return StopInto(
                    now,
                    trouble == CameraTrouble.InUse ? RoundVideoRules.TroubleSentence(trouble, say) : say.Get("The recording stopped unexpectedly."),
                    interrupted: true);
            default:
                return [];
        }
    }

    // ---- the slot and the controls (S3.4) -----------------------------------------------------------------------

    /// <summary>
    /// The slot — Record, Stop or Send — clicked or Return pressed on it. Ignored for 600 ms after its own Record or Stop
    /// (S1.1), and Record until the first frame.
    /// </summary>
    public IReadOnlyList<RecorderEffect> Slot(long now)
    {
        if (now < guardUntil)
        {
            return [];
        }
        lastUsed = now;
        switch (Stage)
        {
            case RecorderStage.Preview when HasPicture:
                Stage = RecorderStage.Recording;
                recordingAt = now;
                takeStarted = false;
                warned = false;
                Notice = null;
                LooksCovered = false;
                guardUntil = now + ComposerButton.ActivationGuardMs;
                return [new(RecorderAction.StartRecording), Say(say.Get("Recording video"))];
            case RecorderStage.Recording:
                guardUntil = now + ComposerButton.ActivationGuardMs;
                return Stop(now, notice: null);
            case RecorderStage.Review when ClipReady && Plan is not null:
                return Close([new(RecorderAction.Send), Say(say.Get("Video message sent"))], cameraOn: false);
            default:
                return [];
        }
    }

    /// <summary>Close (PREVIEW, and every state that has no picture to keep): the camera goes off.</summary>
    public IReadOnlyList<RecorderEffect> CloseButton() =>
        Stage is RecorderStage.Opening or RecorderStage.Refused or RecorderStage.Unavailable or RecorderStage.Preview
            ? Close([], cameraOn: true)
            : [];

    /// <summary>
    /// Delete: during a take under ten seconds it is gone at once and PREVIEW comes back; from ten seconds the take stops
    /// first and the question is put. In REVIEW the same, closing the recorder.
    /// </summary>
    public IReadOnlyList<RecorderEffect> Delete(long now)
    {
        lastUsed = now;
        switch (Stage)
        {
            case RecorderStage.Recording when ElapsedMs(now) < ComposerButton.DeleteAsksFromMs:
                Stage = RecorderStage.Preview;
                Notice = null;
                return [new(RecorderAction.StopAndDiscard), Say(say.Get("Recording deleted"))];
            case RecorderStage.Recording:
                // Stopped while the question is up: nothing more is recorded. The camera stays on for a Delete that goes
                // back to PREVIEW; Keep turns it off.
                ClipMs = ElapsedMs(now);
                Stage = RecorderStage.Review;
                ClipReady = false;
                stoppedByInterruption = false;
                return [new(RecorderAction.StopAndKeep), .. Ask(RecorderQuestion.DeleteWhileRecording)];
            case RecorderStage.Review when ClipMs < ComposerButton.DeleteAsksFromMs:
                return Close([new(RecorderAction.DiscardClip)], cameraOn: false);
            case RecorderStage.Review:
                return Ask(RecorderQuestion.DeleteReview);
            default:
                return [];
        }
    }

    /// <summary>Retake (REVIEW): from ten seconds it asks first; then the clip goes and the camera comes back on.</summary>
    public IReadOnlyList<RecorderEffect> Retake(long now)
    {
        if (Stage != RecorderStage.Review)
        {
            return [];
        }
        lastUsed = now;
        return ClipMs >= ComposerButton.DeleteAsksFromMs ? Ask(RecorderQuestion.Retake) : BackToPreview(now);
    }

    /// <summary>
    /// Esc (and Narrator's way of leaving): PREVIEW and the states with no picture close; RECORDING stops; REVIEW always
    /// asks "Delete video message?".
    /// </summary>
    public IReadOnlyList<RecorderEffect> Escape(long now)
    {
        if (Question != RecorderQuestion.None)
        {
            return [];
        }
        return Stage switch
        {
            RecorderStage.Recording => Stop(now, notice: null),
            RecorderStage.Review => Ask(RecorderQuestion.Escape),
            _ => CloseButton(),
        };
    }

    /// <summary>
    /// "Record a voice message instead" (S3.4): the camera closes and a hands-free voice recording starts — unless a voice
    /// message that was not sent waits, which it says instead (row 9).
    /// </summary>
    public IReadOnlyList<RecorderEffect> VoiceInstead(bool notSentWaits)
    {
        if (Stage is not (RecorderStage.Opening or RecorderStage.Preview or RecorderStage.Refused or RecorderStage.Unavailable)
            || Refused == RecorderRefusal.Microphone)
        {
            return [];
        }
        if (notSentWaits)
        {
            Notice = say.Get("Send or delete the voice message that wasn't sent first.");
            return [Say(Notice)];
        }
        return Close([new(RecorderAction.StartVoice)], cameraOn: true);
    }

    /// <summary>
    /// Another camera chosen from "Choose camera" (S3.5) — in PREVIEW, or in place of one that cannot be used: this one goes
    /// off and that one opens.
    /// </summary>
    public IReadOnlyList<RecorderEffect> SwitchCamera(long now)
    {
        if (Stage is not (RecorderStage.Preview or RecorderStage.Unavailable))
        {
            return [];
        }
        lastUsed = now;
        Stage = RecorderStage.Opening;
        HasPicture = false;
        LooksCovered = false;
        Trouble = null;
        Notice = null;
        return [new(RecorderAction.CloseCamera), new(RecorderAction.OpenCamera)];
    }

    /// <summary>
    /// The take really began — the microphone open and the file being written, after the screen reader's lead-in (S6): the
    /// clock, the ring, the warning, the limit and the 1.0 s floor all count from here, so what is measured is what the
    /// file holds. Before it a Stop finds nothing recorded — "too short" — and the clock shows 0:00.
    /// </summary>
    public void TakeStarted(long now)
    {
        if (Stage == RecorderStage.Recording && !takeStarted)
        {
            takeStarted = true;
            recordingAt = now;
        }
    }

    /// <summary>The take would not start — the microphone refused to open, the disk full: back to PREVIEW, saying so.</summary>
    public IReadOnlyList<RecorderEffect> RecordFailed()
    {
        if (Stage != RecorderStage.Recording)
        {
            return [];
        }
        Stage = RecorderStage.Preview;
        Notice = say.Get("Couldn't start recording.");
        return [Say(Notice)];
    }

    /// <summary>Any other control used — the camera menu, the reply's ✕ — which keeps PREVIEW from timing out.</summary>
    public void Used(long now) => lastUsed = now;

    /// <summary>The answer to the question that is up: Delete (true) or Keep (false).</summary>
    public IReadOnlyList<RecorderEffect> Answer(bool delete, long now)
    {
        var question = Question;
        Question = RecorderQuestion.None;
        lastUsed = now;
        switch (question)
        {
            case RecorderQuestion.DeleteWhileRecording when delete:
                ClipReady = false;
                ClipMs = 0;
                Plan = null;
                Stage = RecorderStage.Preview;
                return [new(RecorderAction.DiscardClip), Say(say.Get("Recording deleted"))];
            case RecorderQuestion.DeleteWhileRecording:
                // Keep: REVIEW, and the light goes out.
                return [new(RecorderAction.CloseCamera)];
            case RecorderQuestion.DeleteReview or RecorderQuestion.Escape when delete:
                return Close([new(RecorderAction.DiscardClip)], cameraOn: false);
            case RecorderQuestion.Retake when delete:
                return BackToPreview(now);
            case RecorderQuestion.CloseWindow when delete:
                return Close([new(RecorderAction.DiscardClip), new(RecorderAction.CloseWindow)], cameraOn: false);
            default:
                return [];
        }
    }

    // ---- the take's end -------------------------------------------------------------------------------------------

    /// <summary>
    /// The take's file is finished and its square made — or not (<paramref name="plan"/> says what is sent), and how long it
    /// is. A take nothing could be read back from goes back to PREVIEW with "The recording stopped unexpectedly." (S4).
    /// A clip that reads back shorter than 1.0 s is never sent (S1.1): the person's Stop goes back to PREVIEW with "That
    /// video was too short."; one an interruption stopped is deleted and the recorder closes (S4: under 1.0 s, deleted);
    /// one a closing window asked about lets the close go ahead.
    /// </summary>
    public IReadOnlyList<RecorderEffect> ClipFinished(long now, RoundSendPlan? plan, long? durationMs)
    {
        if (Stage != RecorderStage.Review || ClipReady)
        {
            return [];
        }
        if (plan is not null && durationMs is { } length && length < ComposerButton.ShortestRecordingMs)
        {
            var asked = Question;
            List<RecorderEffect> dismissed = asked != RecorderQuestion.None ? [new(RecorderAction.Dismiss)] : [];
            Question = RecorderQuestion.None;
            if (asked == RecorderQuestion.CloseWindow)
            {
                return Close([.. dismissed, new(RecorderAction.DiscardClip), new(RecorderAction.CloseWindow)], cameraOn: false);
            }
            if (stoppedByInterruption)
            {
                return Close([.. dismissed, new(RecorderAction.DiscardClip)], cameraOn: false);
            }
            var again = BackToPreview(now);
            Notice = say.Get("That video was too short.");
            return [.. dismissed, .. again, Say(Notice)];
        }
        if (plan is null)
        {
            Question = RecorderQuestion.None;
            var gone = BackToPreview(now);
            Notice = say.Get("The recording stopped unexpectedly.");
            return [new(RecorderAction.Dismiss), .. gone, Say(Notice)];
        }
        ClipReady = true;
        Plan = plan;
        if (durationMs is > 0)
        {
            ClipMs = durationMs.Value;
        }
        if (plan.Notice is { } notice)
        {
            Notice = notice;
            return [Say(notice)];
        }
        return [Say(RoundVideoRules.ReviewLine(ClipMs, say))];
    }

    /// <summary>
    /// The clock: PREVIEW closes after a minute untouched ("Camera turned off"); a take says "10 seconds left" at the warning
    /// and stops itself at the limit, into REVIEW, with "Recording stopped at one minute." — never sent by it.
    /// </summary>
    public IReadOnlyList<RecorderEffect> Tick(long now)
    {
        switch (Stage)
        {
            case RecorderStage.Preview or RecorderStage.Opening when !Asking && now - lastUsed >= RoundVideoRules.PreviewIdleMs:
                return Close([Say(say.Get("Camera turned off"))], cameraOn: true);
            case RecorderStage.Recording when ElapsedMs(now) >= RoundVideoRules.CapMs(limits):
                return Stop(now, say.Get("Recording stopped at one minute."));
            case RecorderStage.Recording when !warned && ElapsedMs(now) >= RoundVideoRules.WarningMs(limits):
                warned = true;
                return [Say(say.Get("10 seconds left"))];
            default:
                return [];
        }
    }

    /// <summary>Whether the ring has turned orange: from the warning on (S3.4) — with the words beside it, never colour alone.</summary>
    public bool Warns(long now) => Stage == RecorderStage.Recording && ElapsedMs(now) >= RoundVideoRules.WarningMs(limits);

    /// <summary>How far round the red ring has filled, over the length limit (S3.4).</summary>
    public double RingFraction(long now)
    {
        if (Stage != RecorderStage.Recording)
        {
            return 0;
        }
        var cap = RoundVideoRules.CapMs(limits);
        return cap <= 0 ? 1 : Math.Clamp((double)ElapsedMs(now) / cap, 0, 1);
    }

    // ---- interruptions (S4) ------------------------------------------------------------------------------------------

    /// <summary>
    /// Something other than the person (S4): PREVIEW closes — but not for focus lost to a permission prompt it raised;
    /// a take stops into REVIEW — but keeps recording while the window is merely unfocused; REVIEW is kept, and a real
    /// close over it asks first. A sign-out deletes everything.
    /// </summary>
    public IReadOnlyList<RecorderEffect> Interrupt(RecorderInterruption why, long now)
    {
        if (Stage == RecorderStage.Closed)
        {
            return [];
        }
        if (why == RecorderInterruption.SignedOut)
        {
            List<RecorderEffect> gone = Question != RecorderQuestion.None ? [new(RecorderAction.Dismiss)] : [];
            Question = RecorderQuestion.None;
            gone.Add(new(Stage == RecorderStage.Recording ? RecorderAction.StopAndDiscard : RecorderAction.DiscardClip));
            return Close(gone, cameraOn: Stage != RecorderStage.Review);
        }
        switch (Stage)
        {
            case RecorderStage.Opening when why == RecorderInterruption.FocusLost && Asking:
                return [];
            case RecorderStage.Opening or RecorderStage.Preview or RecorderStage.Refused or RecorderStage.Unavailable:
                return Close([], cameraOn: true);
            case RecorderStage.Recording when why == RecorderInterruption.FocusLost:
                return [];
            case RecorderStage.Recording when why == RecorderInterruption.WindowClosing:
                return [.. StopInto(now, null, interrupted: true), .. Ask(RecorderQuestion.CloseWindow)];
            case RecorderStage.Recording:
                return StopInto(now, null, interrupted: true);
            case RecorderStage.Review when Question == RecorderQuestion.DeleteWhileRecording && why != RecorderInterruption.FocusLost:
            {
                // A take stopped to ask about deleting it kept the camera on for a Delete that goes back to PREVIEW. Taken
                // out of the question's hands, it is kept — and the light goes out, before a call reaches for the camera.
                Question = RecorderQuestion.None;
                List<RecorderEffect> kept = [new(RecorderAction.Dismiss), new(RecorderAction.CloseCamera)];
                if (why == RecorderInterruption.WindowClosing)
                {
                    kept.AddRange(Ask(RecorderQuestion.CloseWindow));
                }
                return kept;
            }
            case RecorderStage.Review when why == RecorderInterruption.WindowClosing:
                return Question == RecorderQuestion.CloseWindow
                    ? []
                    : [.. Question != RecorderQuestion.None ? [new RecorderEffect(RecorderAction.Dismiss)] : Array.Empty<RecorderEffect>(),
                       .. Ask(RecorderQuestion.CloseWindow)];
            default:
                return [];
        }
    }

    /// <summary>
    /// Something that pauses what plays (S4's last column, <see cref="PlaybackPauses"/>) while REVIEW may be playing its
    /// clip back: a call, a lock, a hidden window or the output device going pause it as they pause any circle; losing focus
    /// lets it play on.
    /// </summary>
    public IReadOnlyList<RecorderEffect> PlaybackInterrupted(PlaybackEvent happened) =>
        Stage == RecorderStage.Review && ClipReady && PlaybackPauses.Pauses(happened, Playing.RoundVideo)
            ? [new(RecorderAction.PauseClip)]
            : [];

    // ---- what the window draws ------------------------------------------------------------------------------------

    /// <summary>
    /// The status line (S3.4): "Starting camera…" or, while Windows asks, "Video messages need the camera and the
    /// microphone."; the refusal or the camera's trouble; "Not recording" — "Camera and microphone on · Not recording"
    /// where the microphone is held — with the first-time line or the covered-camera question under it; the red clock with
    /// "10 seconds left"; "Video message · 0:23".
    /// </summary>
    public RecorderStatus Status(long now, bool firstTimeOnDevice, bool microphoneOnInPreview)
    {
        switch (Stage)
        {
            case RecorderStage.Opening:
                return new(Asking ? say.Get("Video messages need the camera and the microphone.") : say.Get("Starting camera…"), Notice, false, false);
            case RecorderStage.Refused:
                return new(RoundVideoRules.Refusal(Refused ?? RecorderRefusal.Camera, CapabilityAccess.Unknown, say).Sentence, Notice, false, false);
            case RecorderStage.Unavailable:
                return new(RoundVideoRules.TroubleSentence(Trouble ?? CameraTrouble.Failed, say), Notice, false, false);
            case RecorderStage.Preview when !HasPicture:
                return new(say.Get("Starting camera…"), Notice, false, false);
            case RecorderStage.Preview:
            {
                var line = microphoneOnInPreview ? say.Get("Camera and microphone on · Not recording") : say.Get("Not recording");
                var below = Notice
                    ?? (LooksCovered ? say.Get("We can't see anything. Is the camera turned off or covered?")
                        : firstTimeOnDevice ? say.Get("Only you can see this until you start recording.")
                        : null);
                return new(line, below, false, false);
            }
            case RecorderStage.Recording:
            {
                var warns = Warns(now);
                return new(VoiceNotes.Clock(TimeSpan.FromMilliseconds(ElapsedMs(now))), warns ? say.Get("10 seconds left") : null, true, warns);
            }
            case RecorderStage.Review:
                return new(RoundVideoRules.ReviewLine(ClipMs, say), Notice, false, false);
            default:
                return new(string.Empty, null, false, false);
        }
    }

    // ---- inside -------------------------------------------------------------------------------------------------

    private void Reset(long now)
    {
        Stage = RecorderStage.Closed;
        Question = RecorderQuestion.None;
        HasPicture = false;
        LooksCovered = false;
        Asking = false;
        ClipReady = false;
        ClipMs = 0;
        Refused = null;
        Trouble = null;
        Plan = null;
        Notice = null;
        warned = false;
        takeStarted = false;
        stoppedByInterruption = false;
        lastUsed = now;
        guardUntil = long.MinValue;
    }

    /// <summary>The person's Stop (or the limit's): under a second goes back to PREVIEW, "That video was too short."</summary>
    private IReadOnlyList<RecorderEffect> Stop(long now, string? notice)
    {
        if (ElapsedMs(now) < ComposerButton.ShortestRecordingMs)
        {
            Stage = RecorderStage.Preview;
            Notice = say.Get("That video was too short.");
            return [new(RecorderAction.StopAndDiscard), Say(Notice)];
        }
        return StopInto(now, notice, interrupted: false);
    }

    /// <summary>
    /// Into REVIEW with what was recorded — the camera off, the square being made. An interruption under a second deletes
    /// instead: there is nothing worth keeping (S4).
    /// </summary>
    private IReadOnlyList<RecorderEffect> StopInto(long now, string? notice, bool interrupted)
    {
        var elapsed = ElapsedMs(now);
        if (interrupted && elapsed < ComposerButton.ShortestRecordingMs)
        {
            return Close([new(RecorderAction.StopAndDiscard)], cameraOn: true);
        }
        ClipMs = elapsed;
        Stage = RecorderStage.Review;
        ClipReady = false;
        Plan = null;
        Notice = notice;
        stoppedByInterruption = interrupted;
        List<RecorderEffect> effects = [new(RecorderAction.StopAndKeep), new(RecorderAction.CloseCamera)];
        if (notice is not null)
        {
            effects.Add(Say(notice));
        }
        return effects;
    }

    private IReadOnlyList<RecorderEffect> BackToPreview(long now)
    {
        Stage = RecorderStage.Opening;
        HasPicture = false;
        LooksCovered = false;
        ClipReady = false;
        ClipMs = 0;
        Plan = null;
        Notice = null;
        lastUsed = now;
        return [new(RecorderAction.DiscardClip), new(RecorderAction.OpenCamera)];
    }

    private IReadOnlyList<RecorderEffect> Ask(RecorderQuestion question)
    {
        Question = question;
        return [new(RecorderAction.Ask)];
    }

    private IReadOnlyList<RecorderEffect> Close(IReadOnlyList<RecorderEffect> first, bool cameraOn)
    {
        Stage = RecorderStage.Closed;
        Question = RecorderQuestion.None;
        List<RecorderEffect> effects = [.. first];
        if (cameraOn)
        {
            effects.Add(new(RecorderAction.CloseCamera));
        }
        effects.Add(new(RecorderAction.Closed));
        return effects;
    }

    /// <summary>Said to a screen reader: text already in the reader's language.</summary>
    private static RecorderEffect Say(string text) => new(RecorderAction.Announce, text);
}
