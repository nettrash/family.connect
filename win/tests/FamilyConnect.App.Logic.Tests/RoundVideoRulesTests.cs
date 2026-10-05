using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// Recording a video message on Windows, the rules that need no camera (docs/audio-video-messages-2026-10-04.md, Phase 3d;
/// S1.2, S3.3, S3.5, S3.6; "The recording profile for a round video"): the switch, the square, the circle's size, the camera
/// and its mode, the encodes and their check, what a refusal or a busy camera says, and what a finished take is sent as.
/// </summary>
public sealed class RoundVideoRulesTests
{
    private static readonly IStringCatalog Say = EnglishCatalog.Instance;
    private static readonly RoundVideoLimits Limits = new(60_000, 12_582_912);

    // ---- the switch (Blocked 1, Decision 40) ----------------------------------------------------------------

    /// <summary>
    /// SWITCHED OFF until the owner's trials T1–T3 pass on a real machine: no way in is drawn, whatever the server and the
    /// machine offer. Flipping it is a deliberate act (win/README.md), and this test is what says so.
    /// </summary>
    [Fact]
    public void RecordingIsSwitchedOffUntilTheTrials()
    {
        Assert.False(RoundVideoRules.RecordingEnabled);
        Assert.False(RoundVideoRules.Available(RoundVideoRules.RecordingEnabled, Limits, hasCamera: true));
    }

    /// <summary>S1.2's round available: this build records, the server named the limits, and there is a camera — all three.</summary>
    [Fact]
    public void AVideoMessageIsAvailableOnlyWithAllThree()
    {
        Assert.True(RoundVideoRules.Available(true, Limits, hasCamera: true));
        Assert.False(RoundVideoRules.Available(false, Limits, hasCamera: true));
        Assert.False(RoundVideoRules.Available(true, null, hasCamera: true));
        Assert.False(RoundVideoRules.Available(true, Limits, hasCamera: false));
    }

    /// <summary>With the switch off, the video button stays hidden however the composer stands (Decision 40).</summary>
    [Fact]
    public void TheVideoButtonFollowsTheSwitch()
    {
        var empty = new SlotInputs(
            RecorderOpen: false, Recording: Recording.None, Editing: false, DraftBlank: true, Staged: false,
            AssistantChat: false, CanRecord: true, Call: false, Busy: false, NotSent: false);
        var off = new DoorInputs(empty, FamilyOrDirectChat: true, UndoWindow: false, ServerOffersRound: true, HasCamera: true,
            EncoderProbePasses: true, RecordsRoundVideo: RoundVideoRules.RecordingEnabled);
        Assert.Equal(DoorKind.Hidden, ComposerButton.VideoDoor(off).Kind);
        Assert.Equal(DoorKind.Shown, ComposerButton.VideoDoor(off with { RecordsRoundVideo = true }).Kind);
    }

    /// <summary>59.5 s and 50 s at the server's 60 000 — the shared module's arithmetic, from the discovery key.</summary>
    [Fact]
    public void TheCapAndTheWarningComeFromTheServersLimit()
    {
        Assert.Equal(59_500, RoundVideoRules.CapMs(Limits));
        Assert.Equal(50_000, RoundVideoRules.WarningMs(Limits));
        Assert.Equal(29_500, RoundVideoRules.CapMs(new RoundVideoLimits(30_000, 1)));
        Assert.Equal(20_000, RoundVideoRules.WarningMs(new RoundVideoLimits(30_000, 1)));
    }

    /// <summary>The profile's numbers (the plan's "recording profile").</summary>
    [Fact]
    public void TheProfileIsFourEightyAtFiveHundredKilobits()
    {
        Assert.Equal(480u, RoundVideoRules.Edge);
        Assert.Equal(500_000u, RoundVideoRules.VideoBitrate);
        Assert.Equal(30, RoundVideoRules.MaxFrameRate);
        Assert.Equal(60_000, RoundVideoRules.PreviewIdleMs);
        Assert.Equal(2_000, RoundVideoRules.BlackCheckMs);
        Assert.Equal(4, RoundVideoRules.RingWidth);
    }

    // ---- the square --------------------------------------------------------------------------------------------

    /// <summary>The largest square in the middle — a crop, never a squash — its corner and side even.</summary>
    [Theory]
    [InlineData(640u, 480u, 80u, 0u, 480u)]
    [InlineData(1280u, 720u, 280u, 0u, 720u)]
    [InlineData(1920u, 1080u, 420u, 0u, 1080u)]
    [InlineData(480u, 640u, 0u, 80u, 480u)]
    [InlineData(480u, 480u, 0u, 0u, 480u)]
    [InlineData(641u, 481u, 80u, 0u, 480u)]
    [InlineData(645u, 480u, 82u, 0u, 480u)]
    [InlineData(642u, 480u, 80u, 0u, 480u)]
    [InlineData(480u, 642u, 0u, 80u, 480u)]
    [InlineData(1u, 1u, 0u, 0u, 1u)]
    public void TheSquareIsCutFromTheMiddle(uint width, uint height, uint x, uint y, uint side)
    {
        Assert.Equal(new SquareCrop(x, y, side), RoundVideoRules.CentreSquare(width, height));
    }

    [Theory]
    [InlineData(0u, 480u)]
    [InlineData(640u, 0u)]
    public void AFrameWithNoPixelsHasNoSquare(uint width, uint height) =>
        Assert.Null(RoundVideoRules.CentreSquare(width, height));

    // ---- the circle (S3.3) --------------------------------------------------------------------------------------

    /// <summary>D = min(320, width − 48, height − 240 − the banner), at least 160, controls along the bottom.</summary>
    [Theory]
    [InlineData(1000, 900, 0, 320)]
    [InlineData(300, 900, 0, 252)]
    [InlineData(1000, 520, 0, 280)]
    [InlineData(1000, 520, 40, 240)]
    [InlineData(150, 900, 0, 160)]
    [InlineData(1000, 480, 100, 160)]
    public void TheCircleFitsThePane(double width, double height, double banner, double diameter)
    {
        Assert.Equal(new RecorderFit(diameter, RecorderLayout.Row), RoundVideoRules.Fit(width, height, banner));
    }

    /// <summary>Shorter than 480: the controls in a column at the trailing edge, D = min(320, height − 96, width − 200), at least 160.</summary>
    [Theory]
    [InlineData(1000, 479, 0, 320)]
    [InlineData(1000, 400, 0, 304)]
    [InlineData(450, 400, 0, 250)]
    [InlineData(300, 300, 0, 160)]
    [InlineData(1000, 400, 300, 304)]
    public void AShortPaneStandsTheControlsInAColumn(double width, double height, double banner, double diameter)
    {
        Assert.Equal(new RecorderFit(diameter, RecorderLayout.Column), RoundVideoRules.Fit(width, height, banner));
    }

    [Fact]
    public void APaneNotYetLaidOutGetsTheSmallestCircle()
    {
        Assert.Equal(new RecorderFit(160, RecorderLayout.Row), RoundVideoRules.Fit(0, 0, 0));
        Assert.Equal(new RecorderFit(160, RecorderLayout.Row), RoundVideoRules.Fit(double.NaN, 900, 0));
    }

    /// <summary>The ring fills clockwise from 12 o'clock.</summary>
    [Fact]
    public void TheRingFillsClockwiseFromTwelve()
    {
        AssertPoint((100, 0), RoundVideoRules.RingPoint(0, 100, 100));
        AssertPoint((200, 100), RoundVideoRules.RingPoint(0.25, 100, 100));
        AssertPoint((100, 200), RoundVideoRules.RingPoint(0.5, 100, 100));
        AssertPoint((0, 100), RoundVideoRules.RingPoint(0.75, 100, 100));
        AssertPoint((100, 0), RoundVideoRules.RingPoint(2, 100, 100));
        AssertPoint((100, 0), RoundVideoRules.RingPoint(double.NaN, 100, 100));
    }

    private static void AssertPoint((double X, double Y) expected, (double X, double Y) actual)
    {
        Assert.Equal(expected.X, actual.X, 6);
        Assert.Equal(expected.Y, actual.Y, 6);
    }

    // ---- the camera and its mode (S3.5, S3.6) -------------------------------------------------------------------

    /// <summary>The one chosen before while it is there, else the front panel, else the first.</summary>
    [Fact]
    public void TheFrontCameraUnlessAnotherWasChosen()
    {
        var back = new CameraChoice("b", "Rear", CameraPanel.Back);
        var usb = new CameraChoice("u", "USB", CameraPanel.Unknown);
        var front = new CameraChoice("f", "Front", CameraPanel.Front);
        Assert.Equal(front, RoundVideoRules.PickCamera([back, usb, front], remembered: null));
        Assert.Equal(usb, RoundVideoRules.PickCamera([back, usb, front], remembered: "u"));
        Assert.Equal(front, RoundVideoRules.PickCamera([back, usb, front], remembered: "gone"));
        Assert.Equal(back, RoundVideoRules.PickCamera([back, usb], remembered: null));
        Assert.Null(RoundVideoRules.PickCamera([], remembered: "u"));
        Assert.False(RoundVideoRules.OffersCameraChoice(1));
        Assert.True(RoundVideoRules.OffersCameraChoice(2));
    }

    /// <summary>The smallest mode with a short side of 480 or more, at 24 fps or more, nearest 30 without going over.</summary>
    [Fact]
    public void TheModeIsTheSmallestThatNeedsNoUpscale()
    {
        CaptureMode[] modes =
        [
            new(320, 240, 30), new(640, 480, 15), new(640, 480, 30), new(640, 480, 60), new(1280, 720, 30), new(1920, 1080, 30),
        ];
        Assert.Equal(new CaptureMode(640, 480, 30), RoundVideoRules.PickMode(modes));
        // Only 60 at that size: still the size, the nearest rate over 30.
        Assert.Equal(new CaptureMode(640, 480, 60), RoundVideoRules.PickMode([new(640, 480, 60), new(1280, 720, 60), new(640, 480, 90)]));
        // Nothing reaches 480: the largest there is.
        Assert.Equal(new CaptureMode(352, 288, 30), RoundVideoRules.PickMode([new(320, 240, 30), new(352, 288, 30), new(160, 120, 30)]));
        // A slow camera is kept slow rather than refused.
        Assert.Equal(new CaptureMode(640, 480, 15), RoundVideoRules.PickMode([new(640, 480, 15), new(320, 240, 30)]));
        // 24 fps or more beats a smaller size at less.
        Assert.Equal(new CaptureMode(1280, 720, 30), RoundVideoRules.PickMode([new(640, 480, 15), new(1280, 720, 30)]));
    }

    /// <summary>The record stream in the preview's shape where it is offered, so the square cut is the square seen.</summary>
    [Fact]
    public void TheRecordModeKeepsThePreviewsShape()
    {
        CaptureMode[] modes = [new(640, 480, 30), new(848, 480, 30), new(1280, 720, 30)];
        Assert.Equal(new CaptureMode(848, 480, 30), RoundVideoRules.PickMode(modes, aspect: 848.0 / 480));
        Assert.Equal(new CaptureMode(640, 480, 30), RoundVideoRules.PickMode(modes, aspect: 4.0 / 3));
        Assert.Equal(new CaptureMode(640, 480, 30), RoundVideoRules.PickMode(modes, aspect: 1));
    }

    /// <summary>A mode whose frame-source preview stays blank (#9756) is never chosen; a camera with nothing else has none.</summary>
    [Fact]
    public void ABlankPreviewFormatIsNeverChosen()
    {
        Assert.False(RoundVideoRules.PreviewDraws("RGB24"));
        Assert.False(RoundVideoRules.PreviewDraws("uyvy"));
        Assert.False(RoundVideoRules.PreviewDraws("I420"));
        Assert.True(RoundVideoRules.PreviewDraws("NV12"));
        Assert.True(RoundVideoRules.PreviewDraws("YUY2"));
        Assert.True(RoundVideoRules.PreviewDraws("MJPG"));
        Assert.Equal(new CaptureMode(1280, 720, 30, "MJPG"),
            RoundVideoRules.PickMode([new(640, 480, 30, "RGB24"), new(1280, 720, 30, "MJPG")]));
        Assert.Null(RoundVideoRules.PickMode([new(640, 480, 30, "UYVY"), new(1280, 720, 30, "I420")]));
        Assert.Null(RoundVideoRules.PickMode([]));
        Assert.Null(RoundVideoRules.PickMode([new(0, 480, 30)]));
    }

    // ---- the encode (the profile; trial T3) ---------------------------------------------------------------------

    /// <summary>
    /// H.264 High at 480 × 480, 500 000 bit/s, the camera's rate up to 30, upright and SDR; AAC-LC mono 64 000 first, then
    /// the encoder's 96 000; then Main with that.
    /// </summary>
    [Fact]
    public void TheEncodesAreTheProfilesInOrder()
    {
        var encodes = RoundVideoRules.Encodes(30, 1, hasAudio: true);
        Assert.Equal(4, encodes.Count);
        Assert.All(encodes, encode =>
        {
            Assert.Equal((480u, 480u, 0u), (encode.Width, encode.Height, encode.Rotation));
            Assert.Equal(500_000u, encode.Bitrate);
            Assert.Equal((30u, 1u), (encode.FrameRateNumerator, encode.FrameRateDenominator));
            Assert.False(encode.ToSdr);
            Assert.False(encode.KeepAudio);
        });
        Assert.Equal([H264Profile.High, H264Profile.High, H264Profile.Main, H264Profile.Main], encodes.Select(encode => encode.Profile));
        Assert.Equal(
            [new AacEncoding(44_100, 1, 64_000), new AacEncoding(44_100, 1, 96_000), new AacEncoding(44_100, 1, 96_000),
             new AacEncoding(44_100, 1, 96_000)],
            encodes.Select(encode => encode.Audio!.Value));
        // A keyframe at least every 2 s — then, once, the last way again with the encoder's own spacing.
        Assert.Equal([60u, 60u, 60u, null], encodes.Select(encode => encode.KeyframeSpacing));
        Assert.Equal(encodes[2] with { KeyframeSpacing = null }, encodes[3]);
    }

    /// <summary>"Keyframes at most every 2 s", in whole frames at the rate encoded — never 0, never more than 2 s.</summary>
    [Theory]
    [InlineData(30u, 1u, 60u)]
    [InlineData(60u, 1u, 60u)]
    [InlineData(15u, 1u, 30u)]
    [InlineData(30000u, 1001u, 59u)]
    [InlineData(0u, 0u, 60u)]
    public void KeyframesComeAtLeastEveryTwoSeconds(uint numerator, uint denominator, uint frames)
    {
        Assert.Equal(frames, RoundVideoRules.Encodes(numerator, denominator, hasAudio: true)[0].KeyframeSpacing);
        Assert.Equal(frames, RoundVideoRules.Encodes(numerator, denominator, hasAudio: false)[0].KeyframeSpacing);
        // The planner's videos are left to the encoder, as before.
        Assert.Null(new VideoEncoding(640, 360, 0, 1_000_000, 30, 1, H264Profile.High, null, false).KeyframeSpacing);
    }

    /// <summary>Never faster than 30; a camera delivering fewer is kept as it is — its own ratio, 30000/1001 included.</summary>
    [Theory]
    [InlineData(60u, 1u, 30u, 1u)]
    [InlineData(15u, 1u, 15u, 1u)]
    [InlineData(30000u, 1001u, 30000u, 1001u)]
    [InlineData(0u, 0u, 30u, 1u)]
    public void TheFrameRateIsTheCamerasUpToThirty(uint numerator, uint denominator, uint wantNumerator, uint wantDenominator)
    {
        var first = RoundVideoRules.Encodes(numerator, denominator, hasAudio: true)[0];
        Assert.Equal((wantNumerator, wantDenominator), (first.FrameRateNumerator, first.FrameRateDenominator));
    }

    /// <summary>A take with no sound is asked for without any: a transcode does not invent silence.</summary>
    [Fact]
    public void ATakeWithNoSoundIsEncodedWithout()
    {
        var encodes = RoundVideoRules.Encodes(30, 1, hasAudio: false);
        Assert.Equal([H264Profile.High, H264Profile.Main, H264Profile.Main], encodes.Select(encode => encode.Profile));
        Assert.Equal([60u, 60u, null], encodes.Select(encode => encode.KeyframeSpacing));
        Assert.All(encodes, encode => Assert.Null(encode.Audio));
    }

    /// <summary>The square read back: H.264, exactly 480 × 480, upright, AAC where asked, none where not, no faster.</summary>
    [Fact]
    public void TheSquareIsCheckedNotTrusted()
    {
        var asked = RoundVideoRules.Encodes(30, 1, hasAudio: true)[0];
        Assert.True(RoundVideoRules.CameOut(asked, "h264", 480, 480, 0, "aac", 30));
        Assert.False(RoundVideoRules.CameOut(asked, "hevc", 480, 480, 0, "aac", 30));
        Assert.False(RoundVideoRules.CameOut(asked, "h264", 640, 480, 0, "aac", 30));
        Assert.False(RoundVideoRules.CameOut(asked, "h264", 480, 480, 90, "aac", 30));
        Assert.False(RoundVideoRules.CameOut(asked, "h264", 480, 480, 0, null, 30));
        Assert.False(RoundVideoRules.CameOut(asked, "h264", 480, 480, 0, "mp3", 30));
        Assert.False(RoundVideoRules.CameOut(asked, "h264", 480, 480, 0, "aac", 60));
        var silent = RoundVideoRules.Encodes(30, 1, hasAudio: false)[0];
        Assert.True(RoundVideoRules.CameOut(silent, "h264", 480, 480, 0, null, 30));
        Assert.False(RoundVideoRules.CameOut(silent, "h264", 480, 480, 0, "aac", 30));
    }

    // ---- refusals and trouble (S3.2, S3.6) ----------------------------------------------------------------------

    /// <summary>Read before anything opens: either refused opens straight to it — the microphone named first — and asks nothing.</summary>
    [Fact]
    public void WhatWasRefusedIsShownBeforeAnythingIsAsked()
    {
        Assert.Null(RoundVideoRules.RefusedBefore(CapabilityAccess.Allowed, CapabilityAccess.Allowed));
        Assert.Null(RoundVideoRules.RefusedBefore(CapabilityAccess.UserPromptRequired, CapabilityAccess.UserPromptRequired));
        Assert.Equal(RecorderRefusal.Camera, RoundVideoRules.RefusedBefore(CapabilityAccess.DeniedByUser, CapabilityAccess.UserPromptRequired));
        Assert.Equal(RecorderRefusal.Camera, RoundVideoRules.RefusedBefore(CapabilityAccess.DeniedBySystem, CapabilityAccess.Allowed));
        Assert.Equal(RecorderRefusal.Microphone, RoundVideoRules.RefusedBefore(CapabilityAccess.Allowed, CapabilityAccess.DeniedByUser));
        Assert.Equal(RecorderRefusal.Microphone, RoundVideoRules.RefusedBefore(CapabilityAccess.DeniedByUser, CapabilityAccess.DeniedBySystem));
        Assert.True(RoundVideoRules.WillAsk(CapabilityAccess.UserPromptRequired, CapabilityAccess.Allowed));
        Assert.True(RoundVideoRules.WillAsk(CapabilityAccess.Allowed, CapabilityAccess.UserPromptRequired));
        Assert.False(RoundVideoRules.WillAsk(CapabilityAccess.Allowed, CapabilityAccess.Allowed));
        // Refused while opening with Windows saying neither: the camera, which is what this recorder is for.
        Assert.Equal(RecorderRefusal.Camera, RoundVideoRules.RefusedAfter(CapabilityAccess.Unknown, CapabilityAccess.Unknown));
        Assert.Equal(RecorderRefusal.Microphone, RoundVideoRules.RefusedAfter(CapabilityAccess.Allowed, CapabilityAccess.DeniedByUser));
    }

    /// <summary>Each refusal's sentence, and Settings' page for it — not where the manifest itself is at fault.</summary>
    [Fact]
    public void ARefusalSaysWhereToMendIt()
    {
        Assert.Equal(
            ("Family needs permission to use your camera. Turn it on in Settings.", "ms-settings:privacy-webcam"),
            RoundVideoRules.Refusal(RecorderRefusal.Camera, CapabilityAccess.DeniedByUser, Say));
        Assert.Equal(
            ("Family needs permission to use your microphone. Turn it on in Settings.", "ms-settings:privacy-microphone"),
            RoundVideoRules.Refusal(RecorderRefusal.Microphone, CapabilityAccess.DeniedBySystem, Say));
        Assert.Null(RoundVideoRules.Refusal(RecorderRefusal.Camera, CapabilityAccess.NotDeclaredByApp, Say).Settings);
    }

    /// <summary>Media Foundation's codes for a camera another app holds, and for one that went (trial T6 confirms).</summary>
    [Theory]
    [InlineData(0xC00D3704u, CameraTrouble.InUse)]
    [InlineData(0xC00DABE4u, CameraTrouble.InUse)]
    [InlineData(0x80070020u, CameraTrouble.InUse)]
    [InlineData(0x800700AAu, CameraTrouble.InUse)]
    [InlineData(0xC00DABE0u, CameraTrouble.Missing)]
    [InlineData(0xC00DABE3u, CameraTrouble.Missing)]
    [InlineData(0x8007048Fu, CameraTrouble.Missing)]
    [InlineData(0x80004005u, CameraTrouble.Failed)]
    public void TheCamerasTroubleIsReadFromItsCode(uint hresult, CameraTrouble trouble) =>
        Assert.Equal(trouble, RoundVideoRules.Trouble(unchecked((int)hresult)));

    [Fact]
    public void EachTroubleHasItsSentence()
    {
        Assert.Equal("The camera is being used by another app.", RoundVideoRules.TroubleSentence(CameraTrouble.InUse, Say));
        Assert.Equal("Video messages can't be recorded with this camera.", RoundVideoRules.TroubleSentence(CameraTrouble.Unsupported, Say));
        Assert.Equal("Something went wrong. Try again.", RoundVideoRules.TroubleSentence(CameraTrouble.Missing, Say));
        Assert.Equal("Something went wrong. Try again.", RoundVideoRules.TroubleSentence(CameraTrouble.Failed, Say));
    }

    /// <summary>Near-black is a shutter, not a dark room.</summary>
    [Fact]
    public void NearBlackIsAlmostNoLight()
    {
        Assert.True(RoundVideoRules.LooksBlack(0));
        Assert.True(RoundVideoRules.LooksBlack(0.06));
        Assert.False(RoundVideoRules.LooksBlack(0.061));
        Assert.False(RoundVideoRules.LooksBlack(0.2));
        Assert.False(RoundVideoRules.LooksBlack(double.NaN));
    }

    /// <summary>Two seconds of near-black frames, decided once; one lit frame settles it the other way.</summary>
    [Fact]
    public void APictureThatStaysBlackForTwoSecondsIsSaidOnce()
    {
        var check = new PictureCheck();
        Assert.False(check.Frame(1_000, 0.01));
        Assert.False(check.Frame(2_000, 0.0));
        Assert.Null(check.Black);
        Assert.True(check.Frame(3_000, 0.02));
        Assert.True(check.Black);
        Assert.False(check.Frame(4_000, 0.0));

        var lit = new PictureCheck();
        Assert.False(lit.Frame(0, 0.01));
        Assert.False(lit.Frame(500, 0.3));
        Assert.False(lit.Black);
        Assert.False(lit.Frame(5_000, 0.0));
        Assert.False(lit.Black);
    }

    // ---- the video button's guard (S1.1) --------------------------------------------------------------------------

    /// <summary>It ignores clicks for 600 ms after it APPEARS — not after every redraw while it is there.</summary>
    [Fact]
    public void TheVideoButtonIgnoresClicksJustAfterItAppears()
    {
        var guard = new DoorGuard();
        Assert.False(guard.Accepts(10_000));
        guard.Showing(true, 10_000);
        Assert.False(guard.Accepts(10_599));
        guard.Showing(true, 10_300);
        Assert.True(guard.Accepts(10_600));
        guard.Showing(false, 11_000);
        Assert.False(guard.Accepts(11_000));
        guard.Showing(true, 12_000);
        Assert.False(guard.Accepts(12_500));
        Assert.True(guard.Accepts(12_600));
    }

    // ---- what is sent (S3.6) -------------------------------------------------------------------------------------

    /// <summary>The square within the limits goes as a video message, and says nothing more.</summary>
    [Fact]
    public void ASquareWithinTheLimitsGoesRound()
    {
        Assert.Equal(new RoundSendPlan(true, true, null), RoundVideoRules.Plan(4_200_000, 59_500, Limits, Say));
        Assert.Equal(new RoundSendPlan(true, true, null), RoundVideoRules.Plan(12_582_912, 60_000, Limits, Say));
        Assert.Equal(new RoundSendPlan(true, true, null), RoundVideoRules.Plan(1, 1, Limits, Say));
    }

    /// <summary>Over the byte ceiling an operator set: the square itself, as a regular video, after saying so.</summary>
    [Fact]
    public void ASquareTooBigGoesAsARegularVideo()
    {
        Assert.Equal(
            new RoundSendPlan(false, true, "Too big for a video message. It will be sent as a regular video."),
            RoundVideoRules.Plan(12_582_913, 59_500, Limits, Say));
        Assert.Equal(
            new RoundSendPlan(false, true, "Too big for a video message. It will be sent as a regular video."),
            RoundVideoRules.Plan(2_000_000, 10_000, new RoundVideoLimits(60_000, 1_000_000), Say));
    }

    /// <summary>No square, or a length the server would refuse round: the take itself, as a regular video.</summary>
    [Theory]
    [InlineData(null, 30_000L)]
    [InlineData(0L, 30_000L)]
    [InlineData(4_000_000L, null)]
    [InlineData(4_000_000L, 0L)]
    [InlineData(4_000_000L, 60_001L)]
    public void NoSquareGoesAsTheTakeItself(long? bytes, long? duration)
    {
        Assert.Equal(
            new RoundSendPlan(false, false, "Couldn't make it round. It will be sent as a regular video."),
            RoundVideoRules.Plan(bytes, duration, Limits, Say));
    }

    /// <summary>Staged as the wire wants it: kind=video, video/mp4, 480 × 480, its length and its poster.</summary>
    [Fact]
    public void TheSquareIsStagedAsTheWireWantsIt()
    {
        byte[] bytes = [0, 0, 0, 24, (byte)'f', (byte)'t', (byte)'y', (byte)'p'];
        byte[] jpeg = [0xFF, 0xD8, 0xFF];
        var staged = RoundVideoRules.Staged(bytes, 23_400, jpeg);
        Assert.Equal(("video", "video/mp4", 480, 480, 23_400), (staged.Kind, staged.Mime, staged.Width, staged.Height, staged.DurationMs));
        Assert.Equal(bytes, staged.Bytes.ToArray());
        Assert.Equal(jpeg, staged.Preview!.Value.ToArray());
    }

    /// <summary>REVIEW's line, "Video message · 0:23".</summary>
    [Fact]
    public void ReviewSaysWhatItIsAndHowLong()
    {
        Assert.Equal("Video message · 0:23", RoundVideoRules.ReviewLine(23_400, Say));
        Assert.Equal("Video message · 0:59", RoundVideoRules.ReviewLine(59_499, Say));
        Assert.Equal("Video message · 0:00", RoundVideoRules.ReviewLine(-5, Say));
    }

    // ---- what the recorder hears (S4) -------------------------------------------------------------------------------

    [Fact]
    public void TheRecorderHearsWhatEndsAVoiceRecordingItsOwnWay()
    {
        Assert.Equal(RecorderInterruption.Call, RoundVideoRules.InterruptionOf(RecordingEnd.Call));
        Assert.Equal(RecorderInterruption.WindowHidden, RoundVideoRules.InterruptionOf(RecordingEnd.WindowHidden));
        Assert.Equal(RecorderInterruption.WindowHidden, RoundVideoRules.InterruptionOf(RecordingEnd.LeftChat));
        Assert.Equal(RecorderInterruption.SessionLocked, RoundVideoRules.InterruptionOf(RecordingEnd.SessionLocked));
        Assert.Equal(RecorderInterruption.SignedOut, RoundVideoRules.InterruptionOf(RecordingEnd.WindowClosed));
        Assert.Equal(RecorderInterruption.SignedOut, RoundVideoRules.InterruptionOf(RecordingEnd.SignedOut));
        foreach (var own in new[] { RecordingEnd.Stopped, RecordingEnd.Sent, RecordingEnd.Capped, RecordingEnd.Deleted, RecordingEnd.RecorderFailed })
        {
            Assert.Null(RoundVideoRules.InterruptionOf(own));
        }
    }
}
