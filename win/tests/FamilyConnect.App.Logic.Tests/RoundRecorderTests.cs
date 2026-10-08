using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// The video-message recorder's three states as a machine (docs/audio-video-messages-2026-10-04.md, S3.2, S3.4, S3.6, S4,
/// S6): every press, tick and interruption, and what the window is told to do — the camera off in every refusal and in
/// REVIEW, the microphone only from Record, nothing sent but by Send, and closing over a clip asked about.
/// </summary>
public sealed class RoundRecorderTests
{
    private static readonly IStringCatalog Say = EnglishCatalog.Instance;
    private static readonly RoundVideoLimits Limits = new(60_000, 12_582_912);
    private static readonly RoundSendPlan Round = new(true, true, null);

    private static RecorderAction[] Actions(IReadOnlyList<RecorderEffect> effects) =>
        [.. effects.Where(effect => effect.Action != RecorderAction.Announce).Select(effect => effect.Action)];

    private static string[] Said(IReadOnlyList<RecorderEffect> effects) =>
        [.. effects.Where(effect => effect.Action == RecorderAction.Announce).Select(effect => effect.Text ?? string.Empty)];

    /// <summary>A recorder opened at 0 with the camera on and its first frame in: PREVIEW, ready to record.</summary>
    private static RoundRecorder Previewing(long at = 0)
    {
        var recorder = new RoundRecorder(Limits, Say);
        recorder.Open(at, refusedAlready: null, windowsWillAsk: false);
        recorder.CameraOn(at);
        recorder.FirstFrame();
        return recorder;
    }

    /// <summary>Recording from <paramref name="at"/> — pressed then, and the file begun then.</summary>
    private static RoundRecorder Recording(long at = 1_000)
    {
        var recorder = Previewing();
        recorder.Slot(at);
        recorder.TakeStarted(at);
        return recorder;
    }

    /// <summary>In REVIEW with a clip of <paramref name="length"/>, made and ready.</summary>
    private static RoundRecorder Reviewing(long length = 5_000)
    {
        var recorder = Recording(1_000);
        recorder.Slot(1_000 + length);
        recorder.ClipFinished(1_000 + length + 100, Round, length);
        return recorder;
    }

    // ---- opening (S3.2) ---------------------------------------------------------------------------------------------

    /// <summary>Opened: the camera is asked for, the neutral line while Windows asks, "Starting camera…" otherwise.</summary>
    [Fact]
    public void OpeningAsksForTheCamera()
    {
        var recorder = new RoundRecorder(Limits, Say);
        Assert.False(recorder.IsOpen);
        Assert.Equal([RecorderAction.OpenCamera], Actions(recorder.Open(0, null, windowsWillAsk: true)));
        Assert.Equal(RecorderStage.Opening, recorder.Stage);
        Assert.True(recorder.Asking);
        Assert.Equal("Video messages need the camera and the microphone.", recorder.Status(0, false, false).Line);

        var again = new RoundRecorder(Limits, Say);
        again.Open(0, null, windowsWillAsk: false);
        Assert.Equal("Starting camera…", again.Status(0, false, false).Line);
    }

    /// <summary>Something already refused: straight to it, and NOTHING is opened — no prompt for the other device.</summary>
    [Fact]
    public void AnEarlierRefusalOpensStraightToItWithTheCameraOff()
    {
        var recorder = new RoundRecorder(Limits, Say);
        Assert.Empty(recorder.Open(0, RecorderRefusal.Camera, windowsWillAsk: true));
        Assert.Equal(RecorderStage.Refused, recorder.Stage);
        Assert.Equal("Family needs permission to use your camera. Turn it on in Settings.", recorder.Status(0, false, false).Line);
        // Record is not live; a voice message is offered instead.
        Assert.Empty(recorder.Slot(5_000));
        Assert.Equal([RecorderAction.StartVoice, RecorderAction.CloseCamera, RecorderAction.Closed], Actions(recorder.VoiceInstead(notSentWaits: false)));
    }

    /// <summary>The microphone refused: nothing else is offered — voice needs it too.</summary>
    [Fact]
    public void AMicrophoneRefusalOffersNoVoiceMessage()
    {
        var recorder = new RoundRecorder(Limits, Say);
        recorder.Open(0, RecorderRefusal.Microphone, windowsWillAsk: false);
        Assert.Equal("Family needs permission to use your microphone. Turn it on in Settings.", recorder.Status(0, false, false).Line);
        Assert.Empty(recorder.VoiceInstead(notSentWaits: false));
        Assert.True(recorder.IsOpen);
    }

    /// <summary>Refused while Windows asked: the camera goes off at once.</summary>
    [Fact]
    public void ARefusalWhileOpeningTurnsTheCameraOff()
    {
        var recorder = new RoundRecorder(Limits, Say);
        recorder.Open(0, null, windowsWillAsk: true);
        Assert.Equal([RecorderAction.CloseCamera], Actions(recorder.Refuse(RecorderRefusal.Camera)));
        Assert.Equal(RecorderStage.Refused, recorder.Stage);
        Assert.False(recorder.Asking);
    }

    /// <summary>A camera that opened after the recorder closed is let go of at once.</summary>
    [Fact]
    public void ACameraThatOpensTooLateIsLetGo()
    {
        var recorder = new RoundRecorder(Limits, Say);
        recorder.Open(0, null, windowsWillAsk: false);
        recorder.CloseButton();
        Assert.Equal([RecorderAction.CloseCamera], Actions(recorder.CameraOn(100)));
    }

    // ---- PREVIEW (S3.4) -----------------------------------------------------------------------------------------------

    /// <summary>"Starting camera…" until the first frame, Record dimmed; then "Not recording", the first-time line under it, "Camera ready".</summary>
    [Fact]
    public void RecordWaitsForTheFirstFrame()
    {
        var recorder = new RoundRecorder(Limits, Say);
        recorder.Open(0, null, windowsWillAsk: false);
        recorder.CameraOn(10);
        Assert.Equal(RecorderStage.Preview, recorder.Stage);
        Assert.Equal("Starting camera…", recorder.Status(10, true, false).Line);
        Assert.Empty(recorder.Slot(1_000));
        Assert.Equal(RecorderStage.Preview, recorder.Stage);
        Assert.Equal(["Camera ready"], Said(recorder.FirstFrame()));
        Assert.Empty(recorder.FirstFrame());
        Assert.Equal(new RecorderStatus("Not recording", "Only you can see this until you start recording.", false, false), recorder.Status(20, true, false));
        Assert.Equal(new RecorderStatus("Not recording", null, false, false), recorder.Status(20, false, false));
    }

    /// <summary>Where trial T1 finds the microphone held, PREVIEW does not claim otherwise.</summary>
    [Fact]
    public void AHeldMicrophoneIsSaid() =>
        Assert.Equal("Camera and microphone on · Not recording", Previewing().Status(0, false, microphoneOnInPreview: true).Line);

    /// <summary>A picture that stayed black adds the question; Record stays usable — a dark room is not an error.</summary>
    [Fact]
    public void ACoveredCameraIsAskedAboutAndRecordStaysLive()
    {
        var recorder = Previewing();
        recorder.Covered();
        Assert.Equal("We can't see anything. Is the camera turned off or covered?", recorder.Status(0, true, false).Below);
        Assert.Contains(RecorderAction.StartRecording, Actions(recorder.Slot(3_000)));
    }

    /// <summary>Record: the microphone opens now, "Recording video" is said, the red clock runs.</summary>
    [Fact]
    public void RecordOpensTheMicrophoneOnlyNow()
    {
        var recorder = Previewing();
        var effects = recorder.Slot(1_000);
        Assert.Equal([RecorderAction.StartRecording], Actions(effects));
        Assert.Equal(["Recording video"], Said(effects));
        Assert.Equal(RecorderStage.Recording, recorder.Stage);
        Assert.True(recorder.HoldsClip);
        recorder.TakeStarted(1_000);
        Assert.Equal(new RecorderStatus("0:12", null, true, false), recorder.Status(13_000, false, false));
    }

    /// <summary>
    /// The clock starts with the take, not the press (S6: with a screen reader the microphone opens a second after Record,
    /// once "Recording video" is said, "and the timer starts with it"): until the file begins it shows 0:00 and the ring is
    /// empty, and a Stop 1.3 s after the press but 0.3 s into the take is too short — nothing under 1.0 s is sent (S1.1).
    /// </summary>
    [Fact]
    public void TheClockStartsWithTheTakeNotThePress()
    {
        var recorder = Previewing();
        recorder.Slot(1_000);
        Assert.Equal("0:00", recorder.Status(1_900, false, false).Line);
        Assert.Equal(0, recorder.RingFraction(1_900));
        Assert.Equal(0, recorder.ElapsedMs(1_900));
        recorder.TakeStarted(2_000);
        // A second report of the same start changes nothing.
        recorder.TakeStarted(2_200);
        Assert.Equal(300, recorder.ElapsedMs(2_300));
        var effects = recorder.Slot(2_300);
        Assert.Equal([RecorderAction.StopAndDiscard], Actions(effects));
        Assert.Equal(["That video was too short."], Said(effects));
        Assert.Equal(RecorderStage.Preview, recorder.Stage);

        // And the limit counts from the take too: 59.5 s of FILE, whatever the lead-in took.
        var long_ = Previewing();
        long_.Slot(1_000);
        long_.TakeStarted(2_000);
        Assert.Empty(Actions(long_.Tick(60_499)));
        Assert.Contains(RecorderAction.StopAndKeep, Actions(long_.Tick(61_500)));
        Assert.Equal(59_500, long_.ClipMs);
    }

    /// <summary>A Stop, or an interruption, before the take began at all: nothing was recorded, so nothing is kept.</summary>
    [Fact]
    public void NothingIsKeptBeforeTheTakeBegins()
    {
        var stopped = Previewing();
        stopped.Slot(1_000);
        Assert.Equal([RecorderAction.StopAndDiscard], Actions(stopped.Slot(5_000)));
        Assert.Equal(RecorderStage.Preview, stopped.Stage);

        var called = Previewing();
        called.Slot(1_000);
        Assert.Equal([RecorderAction.StopAndDiscard, RecorderAction.CloseCamera, RecorderAction.Closed],
            Actions(called.Interrupt(RecorderInterruption.Call, 5_000)));

        // A take that would not start, then a new one: the new one waits for its own start.
        var failed = Previewing();
        failed.Slot(1_000);
        failed.TakeStarted(1_000);
        failed.RecordFailed();
        failed.Slot(3_000);
        Assert.Equal(0, failed.ElapsedMs(8_000));
    }

    /// <summary>
    /// The clip READ BACK under 1.0 s is never sent (S1.1), whatever the clock said: after the person's Stop, PREVIEW again
    /// with "That video was too short."; after an interruption, deleted and closed (S4); with a closing window's question
    /// up, the close goes ahead — there is nothing to keep.
    /// </summary>
    [Fact]
    public void AClipThatReadsBackUnderASecondIsNeverSent()
    {
        var recorder = Recording(1_000);
        recorder.Slot(2_100);
        var effects = recorder.ClipFinished(3_000, Round, 950);
        Assert.Equal([RecorderAction.DiscardClip, RecorderAction.OpenCamera], Actions(effects));
        Assert.Equal(["That video was too short."], Said(effects));
        Assert.Equal(RecorderStage.Opening, recorder.Stage);
        Assert.False(recorder.ClipReady);
        Assert.Null(recorder.Plan);
        Assert.Empty(recorder.Slot(10_000));
        Assert.Equal("That video was too short.", recorder.Notice);

        // Exactly a second is enough.
        var second = Recording(1_000);
        second.Slot(2_100);
        second.ClipFinished(3_000, Round, 1_000);
        Assert.True(second.ClipReady);

        var called = Recording(1_000);
        called.Interrupt(RecorderInterruption.Call, 2_100);
        Assert.Equal([RecorderAction.DiscardClip, RecorderAction.Closed], Actions(called.ClipFinished(3_000, Round, 900)));
        Assert.False(called.IsOpen);

        var closing = Recording(1_000);
        closing.Interrupt(RecorderInterruption.WindowClosing, 2_100);
        Assert.Equal(RecorderQuestion.CloseWindow, closing.Question);
        Assert.Equal(
            [RecorderAction.Dismiss, RecorderAction.DiscardClip, RecorderAction.CloseWindow, RecorderAction.Closed],
            Actions(closing.ClipFinished(3_000, Round, 900)));
        Assert.False(closing.IsOpen);

        // A person's Stop after an earlier interrupted take is the person's again.
        var later = Recording(1_000);
        later.Interrupt(RecorderInterruption.Call, 5_000);
        later.ClipFinished(5_100, Round, 4_000);
        later.Retake(6_000);
        later.CameraOn(6_100);
        later.FirstFrame();
        later.Slot(7_000);
        // The new take waits for its own start, whatever the last one did.
        Assert.Equal(0, later.ElapsedMs(7_500));
        later.TakeStarted(7_000);
        later.Slot(8_100);
        Assert.Equal([RecorderAction.DiscardClip, RecorderAction.OpenCamera], Actions(later.ClipFinished(8_200, Round, 900)));
    }

    /// <summary>A minute with nothing used turns the camera off — and says so — and loses nothing.</summary>
    [Fact]
    public void PreviewClosesAfterAMinuteUntouched()
    {
        var recorder = Previewing();
        Assert.Empty(recorder.Tick(59_999));
        recorder.Used(30_000);
        Assert.Empty(recorder.Tick(89_999));
        var effects = recorder.Tick(90_000);
        Assert.Equal([RecorderAction.CloseCamera, RecorderAction.Closed], Actions(effects));
        Assert.Equal(["Camera turned off"], Said(effects));
        Assert.False(recorder.IsOpen);
    }

    /// <summary>Not while Windows asks: a person reading a permission prompt is not idle.</summary>
    [Fact]
    public void ThePromptDoesNotTimeOut()
    {
        var recorder = new RoundRecorder(Limits, Say);
        recorder.Open(0, null, windowsWillAsk: true);
        Assert.Empty(recorder.Tick(120_000));
        Assert.True(recorder.IsOpen);
    }

    /// <summary>Close and Esc close PREVIEW, the camera off.</summary>
    [Fact]
    public void CloseAndEscapeClosePreview()
    {
        Assert.Equal([RecorderAction.CloseCamera, RecorderAction.Closed], Actions(Previewing().CloseButton()));
        Assert.Equal([RecorderAction.CloseCamera, RecorderAction.Closed], Actions(Previewing().Escape(5)));
        Assert.Empty(Recording().CloseButton());
    }

    /// <summary>"Record a voice message instead": the camera closes and a voice recording starts — unless one not sent waits.</summary>
    [Fact]
    public void VoiceInsteadClosesTheCameraUnlessANoteWaits()
    {
        var recorder = Previewing();
        var waits = recorder.VoiceInstead(notSentWaits: true);
        Assert.Empty(Actions(waits));
        Assert.Equal(["Send or delete the voice message that wasn't sent first."], Said(waits));
        Assert.True(recorder.IsOpen);
        Assert.Equal([RecorderAction.StartVoice, RecorderAction.CloseCamera, RecorderAction.Closed], Actions(recorder.VoiceInstead(notSentWaits: false)));
        Assert.Empty(Recording().VoiceInstead(notSentWaits: false));
    }

    /// <summary>Another camera chosen: this one off, that one on, and the picture awaited again.</summary>
    [Fact]
    public void ChoosingACameraReopensIt()
    {
        var recorder = Previewing();
        Assert.Equal([RecorderAction.CloseCamera, RecorderAction.OpenCamera], Actions(recorder.SwitchCamera(10)));
        Assert.Equal(RecorderStage.Opening, recorder.Stage);
        Assert.False(recorder.HasPicture);
        Assert.Empty(Recording().SwitchCamera(10));
    }

    // ---- RECORDING (S3.4) ------------------------------------------------------------------------------------------

    /// <summary>The slot ignores clicks for 600 ms after its own Record: a double click cannot stop the take.</summary>
    [Fact]
    public void ADoubleClickOnRecordDoesNotStop()
    {
        var recorder = Recording(1_000);
        Assert.Empty(recorder.Slot(1_599));
        Assert.Equal(RecorderStage.Recording, recorder.Stage);
        Assert.Contains(RecorderAction.StopAndKeep, Actions(recorder.Slot(5_000)));
    }

    /// <summary>Stop under a second: back to PREVIEW, "That video was too short.", nothing kept.</summary>
    [Fact]
    public void AStopUnderASecondIsTooShort()
    {
        var recorder = Recording(1_000);
        recorder.Slot(1_000);
        // Esc is Stop too, and is not the slot's guard.
        var effects = recorder.Escape(1_999);
        Assert.Equal([RecorderAction.StopAndDiscard], Actions(effects));
        Assert.Equal(["That video was too short."], Said(effects));
        Assert.Equal(RecorderStage.Preview, recorder.Stage);
        Assert.Equal("That video was too short.", recorder.Status(2_000, false, false).Below);
    }

    /// <summary>Stop: the take is kept, the camera goes off, and REVIEW waits for the square.</summary>
    [Fact]
    public void StopGoesToReviewWithTheCameraOff()
    {
        var recorder = Recording(1_000);
        Assert.Equal([RecorderAction.StopAndKeep, RecorderAction.CloseCamera], Actions(recorder.Slot(24_400)));
        Assert.Equal(RecorderStage.Review, recorder.Stage);
        Assert.False(recorder.ClipReady);
        Assert.Equal(23_400, recorder.ClipMs);
        Assert.Equal("Video message · 0:23", recorder.Status(30_000, false, false).Line);
        // Send waits for the square.
        Assert.Empty(recorder.Slot(30_000));
        Assert.Equal(RecorderStage.Review, recorder.Stage);
    }

    /// <summary>"10 seconds left" at 50 s, said once, with the ring orange; at 59.5 s the take stops itself — into REVIEW, never sent.</summary>
    [Fact]
    public void TheLimitWarnsThenStopsIntoReview()
    {
        var recorder = Recording(0);
        Assert.Empty(recorder.Tick(49_999));
        Assert.False(recorder.Warns(49_999));
        Assert.Equal(["10 seconds left"], Said(recorder.Tick(50_000)));
        Assert.Empty(recorder.Tick(50_250));
        Assert.True(recorder.Warns(50_250));
        Assert.Equal(new RecorderStatus("0:50", "10 seconds left", true, true), recorder.Status(50_250, false, false));
        Assert.Equal(50_000.0 / 59_500, recorder.RingFraction(50_000), 6);
        var stopped = recorder.Tick(59_500);
        Assert.Equal([RecorderAction.StopAndKeep, RecorderAction.CloseCamera], Actions(stopped));
        Assert.Equal(["Recording stopped at one minute."], Said(stopped));
        Assert.DoesNotContain(RecorderAction.Send, Actions(stopped));
        Assert.Equal(RecorderStage.Review, recorder.Stage);
        Assert.Equal(0, recorder.RingFraction(60_000));
    }

    /// <summary>Delete under ten seconds: gone at once, back to PREVIEW with the camera still on.</summary>
    [Fact]
    public void DeletingAShortTakeIsImmediate()
    {
        var recorder = Recording(1_000);
        var effects = recorder.Delete(10_999);
        Assert.Equal([RecorderAction.StopAndDiscard], Actions(effects));
        Assert.Equal(["Recording deleted"], Said(effects));
        Assert.Equal(RecorderStage.Preview, recorder.Stage);
    }

    /// <summary>Delete from ten seconds: it stops first and asks; Delete goes back to PREVIEW, Keep to REVIEW with the light out.</summary>
    [Fact]
    public void DeletingALongTakeStopsAndAsks()
    {
        var recorder = Recording(1_000);
        Assert.Equal([RecorderAction.StopAndKeep, RecorderAction.Ask], Actions(recorder.Delete(11_000)));
        Assert.Equal(RecorderQuestion.DeleteWhileRecording, recorder.Question);
        Assert.Equal(RecorderStage.Review, recorder.Stage);
        Assert.Equal([RecorderAction.DiscardClip], Actions(recorder.Answer(delete: true, 12_000)));
        Assert.Equal(RecorderStage.Preview, recorder.Stage);
        Assert.True(recorder.HasPicture);

        var kept = Recording(1_000);
        kept.Delete(11_000);
        Assert.Equal([RecorderAction.CloseCamera], Actions(kept.Answer(delete: false, 12_000)));
        Assert.Equal(RecorderStage.Review, kept.Stage);
        Assert.Equal(RecorderQuestion.None, kept.Question);
    }

    /// <summary>The take would not start: back to PREVIEW, saying so.</summary>
    [Fact]
    public void ATakeThatWouldNotStartSaysSo()
    {
        var recorder = Recording(1_000);
        Assert.Equal(["Couldn't start recording."], Said(recorder.RecordFailed()));
        Assert.Equal(RecorderStage.Preview, recorder.Stage);
        Assert.Empty(Previewing().RecordFailed());
    }

    // ---- the camera's trouble (S3.6) ---------------------------------------------------------------------------------

    /// <summary>In PREVIEW, the sentence in place of the picture and the camera off; mid-take, REVIEW with what was recorded.</summary>
    [Fact]
    public void ACameraTakenAwayStopsIntoReviewOrSaysWhy()
    {
        var preview = Previewing();
        Assert.Equal([RecorderAction.CloseCamera], Actions(preview.CameraTroubled(CameraTrouble.InUse, 100)));
        Assert.Equal(RecorderStage.Unavailable, preview.Stage);
        Assert.Equal("The camera is being used by another app.", preview.Status(100, false, false).Line);
        Assert.Empty(preview.Slot(5_000));

        var unsupported = new RoundRecorder(Limits, Say);
        unsupported.Open(0, null, false);
        unsupported.CameraTroubled(CameraTrouble.Unsupported, 10);
        Assert.Equal("Video messages can't be recorded with this camera.", unsupported.Status(10, false, false).Line);

        var recording = Recording(1_000);
        var effects = recording.CameraTroubled(CameraTrouble.InUse, 8_000);
        Assert.Equal([RecorderAction.StopAndKeep, RecorderAction.CloseCamera], Actions(effects));
        Assert.Equal(["The camera is being used by another app."], Said(effects));
        Assert.Equal(RecorderStage.Review, recording.Stage);
        Assert.Equal("The camera is being used by another app.", recording.Status(8_000, false, false).Below);

        // Gone or failed mid-take: what was recorded, and what S4 says of a recorder that fails.
        var pulled = Recording(1_000);
        Assert.Equal(["The recording stopped unexpectedly."], Said(pulled.CameraTroubled(CameraTrouble.Missing, 8_000)));
        Assert.Equal(RecorderStage.Review, pulled.Stage);
    }

    // ---- REVIEW (S3.4, S3.6) -----------------------------------------------------------------------------------------

    /// <summary>
    /// Space is the recorder's in REVIEW — from the moment the take stops, while the square is still being made too — and
    /// never reaches a focused Delete or Retake (S3.4). Elsewhere it is the focused control's, and under a question the
    /// question's.
    /// </summary>
    [Fact]
    public void SpaceIsTheRecordersThroughoutReview()
    {
        Assert.False(Previewing().CatchesSpace);
        Assert.False(Recording().CatchesSpace);
        var making = Recording(1_000);
        making.Slot(6_000);
        Assert.False(making.ClipReady);
        Assert.True(making.CatchesSpace);
        Assert.True(Reviewing().CatchesSpace);
        var asking = Reviewing(12_000);
        asking.Escape(20_000);
        Assert.False(asking.CatchesSpace);
    }

    /// <summary>
    /// The clip REVIEW plays back is a circle playing (S4's last column): a call, a lock, a hidden window and the output
    /// device going pause it; losing focus does not; and there is nothing to pause before the clip is ready or outside REVIEW.
    /// </summary>
    [Theory]
    [InlineData(PlaybackEvent.Call, true)]
    [InlineData(PlaybackEvent.SessionLocked, true)]
    [InlineData(PlaybackEvent.WindowHidden, true)]
    [InlineData(PlaybackEvent.OutputChanged, true)]
    [InlineData(PlaybackEvent.FocusLost, false)]
    public void ReviewPlaybackPausesAsACircleDoes(PlaybackEvent happened, bool pauses)
    {
        var recorder = Reviewing();
        Assert.Equal(pauses ? [RecorderAction.PauseClip] : [], Actions(recorder.PlaybackInterrupted(happened)));
        // Paused, REVIEW is kept.
        Assert.Equal(RecorderStage.Review, recorder.Stage);
        Assert.True(recorder.ClipReady);

        var making = Recording(1_000);
        making.Slot(6_000);
        Assert.Empty(making.PlaybackInterrupted(happened));
        Assert.Empty(Previewing().PlaybackInterrupted(happened));
        Assert.Empty(Recording().PlaybackInterrupted(happened));
    }

    /// <summary>The square made: REVIEW's line said, Send live — and Send sends, says so, and closes.</summary>
    [Fact]
    public void SendSendsOnceTheClipIsReady()
    {
        var recorder = Recording(1_000);
        recorder.Slot(24_400);
        Assert.Equal(["Video message · 0:23"], Said(recorder.ClipFinished(25_000, Round, 23_450)));
        Assert.True(recorder.ClipReady);
        Assert.Equal(23_450, recorder.ClipMs);
        // The guard after Stop still holds.
        Assert.Empty(recorder.Slot(24_999));
        var sent = recorder.Slot(25_000);
        Assert.Equal([RecorderAction.Send, RecorderAction.Closed], Actions(sent));
        Assert.Equal(["Video message sent"], Said(sent));
        Assert.False(recorder.IsOpen);
    }

    /// <summary>A clip that cannot go round says so in REVIEW, and Send still sends it — as what the plan says.</summary>
    [Fact]
    public void AClipThatCannotGoRoundSaysSo()
    {
        var recorder = Recording(1_000);
        recorder.Slot(6_000);
        var plan = new RoundSendPlan(false, false, "Couldn't make it round. It will be sent as a regular video.");
        Assert.Equal([plan.Notice!], Said(recorder.ClipFinished(7_000, plan, 5_000)));
        Assert.Equal(plan, recorder.Plan);
        Assert.Equal(plan.Notice, recorder.Status(7_000, false, false).Below);
        Assert.Contains(RecorderAction.Send, Actions(recorder.Slot(8_000)));
    }

    /// <summary>A take nothing could be read back from: PREVIEW again, "The recording stopped unexpectedly."</summary>
    [Fact]
    public void ATakeThatCouldNotBeReadGoesBackToPreview()
    {
        var recorder = Recording(1_000);
        recorder.Slot(6_000);
        var effects = recorder.ClipFinished(7_000, plan: null, durationMs: null);
        Assert.Equal([RecorderAction.Dismiss, RecorderAction.DiscardClip, RecorderAction.OpenCamera], Actions(effects));
        Assert.Equal(["The recording stopped unexpectedly."], Said(effects));
        Assert.Equal(RecorderStage.Opening, recorder.Stage);
        Assert.Equal("The recording stopped unexpectedly.", recorder.Notice);
    }

    /// <summary>Only once, and only in REVIEW.</summary>
    [Fact]
    public void AClipLandsOnce()
    {
        var recorder = Reviewing();
        Assert.Empty(recorder.ClipFinished(9_000, Round, 1));
        Assert.Equal(5_000, recorder.ClipMs);
        Assert.Empty(Previewing().ClipFinished(9_000, Round, 1));
    }

    /// <summary>Delete under ten seconds closes at once; from ten it asks, and Delete closes.</summary>
    [Fact]
    public void DeleteInReviewAsksFromTenSeconds()
    {
        Assert.Equal([RecorderAction.DiscardClip, RecorderAction.Closed], Actions(Reviewing(9_999).Delete(20_000)));
        var recorder = Reviewing(10_000);
        Assert.Equal([RecorderAction.Ask], Actions(recorder.Delete(20_000)));
        Assert.Equal(RecorderQuestion.DeleteReview, recorder.Question);
        Assert.Empty(recorder.Answer(delete: false, 21_000));
        Assert.Equal(RecorderStage.Review, recorder.Stage);
        recorder.Delete(22_000);
        Assert.Equal([RecorderAction.DiscardClip, RecorderAction.Closed], Actions(recorder.Answer(delete: true, 23_000)));
        Assert.False(recorder.IsOpen);
    }

    /// <summary>Retake: the camera back on — after asking, from ten seconds.</summary>
    [Fact]
    public void RetakeTurnsTheCameraBackOn()
    {
        var recorder = Reviewing(5_000);
        Assert.Equal([RecorderAction.DiscardClip, RecorderAction.OpenCamera], Actions(recorder.Retake(9_000)));
        Assert.Equal(RecorderStage.Opening, recorder.Stage);

        var asked = Reviewing(12_000);
        Assert.Equal([RecorderAction.Ask], Actions(asked.Retake(20_000)));
        Assert.Equal(RecorderQuestion.Retake, asked.Question);
        Assert.Equal([RecorderAction.DiscardClip, RecorderAction.OpenCamera], Actions(asked.Answer(delete: true, 21_000)));
        Assert.Empty(Previewing().Retake(10));
    }

    /// <summary>Esc in REVIEW always asks, however short the clip; and does nothing while a question is up.</summary>
    [Fact]
    public void EscapeInReviewAlwaysAsks()
    {
        var recorder = Reviewing(2_000);
        Assert.Equal([RecorderAction.Ask], Actions(recorder.Escape(9_000)));
        Assert.Equal(RecorderQuestion.Escape, recorder.Question);
        Assert.Empty(recorder.Escape(9_100));
        Assert.Equal([RecorderAction.DiscardClip, RecorderAction.Closed], Actions(recorder.Answer(delete: true, 9_200)));
    }

    // ---- interruptions (S4) -------------------------------------------------------------------------------------------

    /// <summary>A call, a hidden window or a lock: PREVIEW closes; a take stops into REVIEW; REVIEW is kept.</summary>
    [Theory]
    [InlineData(RecorderInterruption.Call)]
    [InlineData(RecorderInterruption.WindowHidden)]
    [InlineData(RecorderInterruption.SessionLocked)]
    public void AnInterruptionClosesPreviewStopsATakeAndKeepsReview(RecorderInterruption why)
    {
        var preview = Previewing();
        Assert.Equal([RecorderAction.CloseCamera, RecorderAction.Closed], Actions(preview.Interrupt(why, 100)));

        var recording = Recording(1_000);
        var stopped = recording.Interrupt(why, 9_000);
        Assert.Equal([RecorderAction.StopAndKeep, RecorderAction.CloseCamera], Actions(stopped));
        Assert.DoesNotContain(RecorderAction.Send, Actions(stopped));
        Assert.Equal(RecorderStage.Review, recording.Stage);
        Assert.Equal(8_000, recording.ClipMs);

        var review = Reviewing();
        Assert.Empty(review.Interrupt(why, 20_000));
        Assert.Equal(RecorderStage.Review, review.Stage);
    }

    /// <summary>Under a second, an interrupted take is deleted instead: there is nothing worth keeping.</summary>
    [Fact]
    public void AnInterruptedTakeUnderASecondIsDeleted()
    {
        var recorder = Recording(1_000);
        Assert.Equal([RecorderAction.StopAndDiscard, RecorderAction.CloseCamera, RecorderAction.Closed],
            Actions(recorder.Interrupt(RecorderInterruption.Call, 1_500)));
        Assert.False(recorder.IsOpen);
    }

    /// <summary>Focus lost to another window: PREVIEW closes — not to a prompt it raised — and a take records on.</summary>
    [Fact]
    public void LosingFocusClosesPreviewButNotAPromptOrATake()
    {
        Assert.Equal([RecorderAction.CloseCamera, RecorderAction.Closed], Actions(Previewing().Interrupt(RecorderInterruption.FocusLost, 10)));

        var asking = new RoundRecorder(Limits, Say);
        asking.Open(0, null, windowsWillAsk: true);
        Assert.Empty(asking.Interrupt(RecorderInterruption.FocusLost, 10));
        Assert.True(asking.IsOpen);

        var recording = Recording(1_000);
        Assert.Empty(recording.Interrupt(RecorderInterruption.FocusLost, 5_000));
        Assert.Equal(RecorderStage.Recording, recording.Stage);
    }

    /// <summary>A real close over a take stops it into REVIEW and asks; Delete closes the window, Keep leaves everything.</summary>
    [Fact]
    public void AReallyClosingWindowAsksOverAClip()
    {
        var recording = Recording(1_000);
        Assert.Equal([RecorderAction.StopAndKeep, RecorderAction.CloseCamera, RecorderAction.Ask],
            Actions(recording.Interrupt(RecorderInterruption.WindowClosing, 9_000)));
        Assert.Equal(RecorderQuestion.CloseWindow, recording.Question);
        Assert.Equal([RecorderAction.DiscardClip, RecorderAction.CloseWindow, RecorderAction.Closed],
            Actions(recording.Answer(delete: true, 10_000)));

        var review = Reviewing();
        Assert.Equal([RecorderAction.Ask], Actions(review.Interrupt(RecorderInterruption.WindowClosing, 20_000)));
        // A second close while asked: the one question.
        Assert.Empty(review.Interrupt(RecorderInterruption.WindowClosing, 20_100));
        Assert.Empty(review.Answer(delete: false, 21_000));
        Assert.Equal(RecorderStage.Review, review.Stage);

        var otherQuestion = Reviewing(12_000);
        otherQuestion.Delete(20_000);
        Assert.Equal([RecorderAction.Dismiss, RecorderAction.Ask], Actions(otherQuestion.Interrupt(RecorderInterruption.WindowClosing, 20_100)));
        Assert.Equal(RecorderQuestion.CloseWindow, otherQuestion.Question);

        Assert.Equal([RecorderAction.CloseCamera, RecorderAction.Closed], Actions(Previewing().Interrupt(RecorderInterruption.WindowClosing, 5)));
    }

    /// <summary>A take stopped to ask about deleting it, then interrupted: kept, the question taken away, the light out.</summary>
    [Fact]
    public void AnInterruptionDuringTheDeleteQuestionKeepsTheTake()
    {
        var recorder = Recording(1_000);
        recorder.Delete(12_000);
        Assert.Equal([RecorderAction.Dismiss, RecorderAction.CloseCamera], Actions(recorder.Interrupt(RecorderInterruption.Call, 12_500)));
        Assert.Equal(RecorderQuestion.None, recorder.Question);
        Assert.Equal(RecorderStage.Review, recorder.Stage);

        var closing = Recording(1_000);
        closing.Delete(12_000);
        Assert.Equal([RecorderAction.Dismiss, RecorderAction.CloseCamera, RecorderAction.Ask],
            Actions(closing.Interrupt(RecorderInterruption.WindowClosing, 12_500)));
        Assert.Equal(RecorderQuestion.CloseWindow, closing.Question);

        var unfocused = Recording(1_000);
        unfocused.Delete(12_000);
        Assert.Empty(unfocused.Interrupt(RecorderInterruption.FocusLost, 12_500));
        Assert.Equal(RecorderQuestion.DeleteWhileRecording, unfocused.Question);
    }

    /// <summary>A sign-out deletes everything recorded and not sent, whatever state it is in.</summary>
    [Fact]
    public void ASignOutDeletesEverything()
    {
        Assert.Equal([RecorderAction.StopAndDiscard, RecorderAction.CloseCamera, RecorderAction.Closed],
            Actions(Recording(1_000).Interrupt(RecorderInterruption.SignedOut, 30_000)));
        Assert.Equal([RecorderAction.DiscardClip, RecorderAction.Closed],
            Actions(Reviewing().Interrupt(RecorderInterruption.SignedOut, 30_000)));
        var asked = Reviewing(12_000);
        asked.Escape(20_000);
        Assert.Equal([RecorderAction.Dismiss, RecorderAction.DiscardClip, RecorderAction.Closed],
            Actions(asked.Interrupt(RecorderInterruption.SignedOut, 30_000)));
        Assert.Empty(new RoundRecorder(Limits, Say).Interrupt(RecorderInterruption.SignedOut, 0));
    }

    /// <summary>Opened again after closing: everything from before is forgotten.</summary>
    [Fact]
    public void ReopeningStartsAfresh()
    {
        var recorder = Reviewing();
        recorder.Slot(50_000);
        Assert.False(recorder.IsOpen);
        recorder.Open(60_000, null, windowsWillAsk: false);
        Assert.Equal(RecorderStage.Opening, recorder.Stage);
        Assert.False(recorder.ClipReady);
        Assert.Null(recorder.Plan);
        Assert.Equal(0, recorder.ClipMs);
        Assert.False(recorder.HasPicture);
    }
}
