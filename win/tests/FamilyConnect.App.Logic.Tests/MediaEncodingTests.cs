using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// What goes into a Media Foundation profile for a plan — the numbers only. The platform calls that take them are
/// <c>MediaPreparing</c> and <c>VoiceRecorder</c>, which only a Windows machine can run; everything they are GIVEN is
/// pinned here.
/// </summary>
public sealed class MediaEncodingTests
{
    // --- reading ---------------------------------------------------------------------------------------------------

    [Fact]
    public void ANumberIsKnownWhenItIsMoreThanZero()
    {
        Assert.Equal(5, MediaEncoding.FirstKnown(0, null, 5));
        Assert.Equal(3, MediaEncoding.FirstKnown(-1, 3, 7));
        Assert.Null(MediaEncoding.FirstKnown(null, 0));
        Assert.Null(MediaEncoding.FirstKnown());
    }

    /// <summary>Media Foundation's turn when it states one — even 0 — and the shell's otherwise; nonsense is no turn.</summary>
    [Fact]
    public void TheTurnIsMediaFoundationsThenTheShells()
    {
        Assert.Equal(90u, MediaEncoding.Rotation(90, 0));
        Assert.Equal(0u, MediaEncoding.Rotation(0, 90));
        Assert.Equal(270u, MediaEncoding.Rotation(null, 270));
        Assert.Equal(180u, MediaEncoding.Rotation(45, 180));
        Assert.Equal(0u, MediaEncoding.Rotation(null, null));
        Assert.Equal(0u, MediaEncoding.Rotation(-90, 360));
    }

    [Fact]
    public void AQuarterTurnSwapsTheDisplayedSides()
    {
        Assert.Equal((1080L, 1920L), MediaEncoding.Displayed(1920, 1080, 90));
        Assert.Equal((1080L, 1920L), MediaEncoding.Displayed(1920, 1080, 270));
        Assert.Equal((1920L, 1080L), MediaEncoding.Displayed(1920, 1080, 180));
        Assert.Equal((1920L, 1080L), MediaEncoding.Displayed(1920, 1080, 0));
    }

    /// <summary>The stream's own ratio first; the shell counts frames per THOUSAND seconds.</summary>
    [Fact]
    public void AFrameRateIsTheStreamsRatioOrTheShellsThousandths()
    {
        Assert.Equal(30_000.0 / 1001, MediaEncoding.FrameRate(30_000, 1001, 29_970));
        Assert.Equal(29.97, MediaEncoding.FrameRate(0, 0, 29_970));
        Assert.Equal(25.0, MediaEncoding.FrameRate(60, 0, 25_000));
        Assert.Null(MediaEncoding.FrameRate(0, 1, 0));
        Assert.Null(MediaEncoding.FrameRate(30, 0, 0));
    }

    [Fact]
    public void HdrIsPqOrHlgOrTheWideGamut()
    {
        Assert.True(MediaEncoding.IsHdr(15, null));
        Assert.True(MediaEncoding.IsHdr(16, 2));
        Assert.True(MediaEncoding.IsHdr(null, 9));
        Assert.False(MediaEncoding.IsHdr(5, 2));
        Assert.False(MediaEncoding.IsHdr(null, null));
    }

    /// <summary>AIFF and FLAC reach the audio rules although the server takes neither; a document never does.</summary>
    [Fact]
    public void ASoundFileTheRouterSentAsAFileIsStillJudgedAsSound()
    {
        Assert.Equal("audio/x-aiff", MediaEncoding.PickedAudioType("audio/x-aiff", "take.aiff"));
        Assert.Equal("audio/flac", MediaEncoding.PickedAudioType("application/octet-stream", "Song.FLAC"));
        Assert.Equal("audio/x-flac", MediaEncoding.PickedAudioType("audio/x-flac", "song.flac"));
        Assert.Null(MediaEncoding.PickedAudioType("application/pdf", "minutes.pdf"));
        Assert.Null(MediaEncoding.PickedAudioType("video/x-matroska", "clip.mkv"));
    }

    [Fact]
    public void AReEncodedTrackIsNamedForWhatItNowIs()
    {
        Assert.Equal("song.m4a", MediaEncoding.M4aName("song.wav"));
        Assert.Equal("Take 3.m4a", MediaEncoding.M4aName("Take 3.FLAC"));
        Assert.Equal("memo.m4a", MediaEncoding.M4aName("memo"));
        Assert.Null(MediaEncoding.M4aName(null));
        // At the limit the stem is cut, never the extension.
        var named = MediaEncoding.M4aName(new string('a', 255))!;
        Assert.Equal(255, named.Length);
        Assert.EndsWith(".m4a", named, StringComparison.Ordinal);
    }

    // --- the profile ------------------------------------------------------------------------------------------------

    /// <summary>A portrait phone clip is a landscape frame with a turn; it is ENCODED landscape and turned by metadata.</summary>
    [Fact]
    public void TheEncodedSidesUndoTheTurn()
    {
        Assert.Equal((1280u, 720u), MediaEncoding.Encoded(720, 1280, 90));
        Assert.Equal((1280u, 720u), MediaEncoding.Encoded(720, 1280, 270));
        Assert.Equal((720u, 1280u), MediaEncoding.Encoded(720, 1280, 180));
        Assert.Equal((720u, 1280u), MediaEncoding.Encoded(720, 1280, 0));
    }

    [Fact]
    public void AFrameRateRatioIsTheSourcesOwnWhenItIsTheSourcesRate()
    {
        Assert.Equal((30_000u, 1001u), MediaEncoding.FrameRateRatio(30_000.0 / 1001, 30_000, 1001));
        Assert.Equal((24_000u, 1001u), MediaEncoding.FrameRateRatio(24_000.0 / 1001, 24_000, 1001));
        Assert.Equal((25u, 1u), MediaEncoding.FrameRateRatio(25.0, 25, 1));
        // Capped from 60: the cap, not the source.
        Assert.Equal((30u, 1u), MediaEncoding.FrameRateRatio(30.0, 60, 1));
        Assert.Equal((30u, 1u), MediaEncoding.FrameRateRatio(30.0, 60_000, 1001));
        // Read from the shell alone, or a ratio that does not match: the nearest thousandth not above it.
        Assert.Equal((2997u, 100u), MediaEncoding.FrameRateRatio(29.97, 0, 0));
        Assert.Equal((2997u, 125u), MediaEncoding.FrameRateRatio(23.976, 0, 0));
        Assert.Equal((2997u, 100u), MediaEncoding.FrameRateRatio(30_000.0 / 1001, 0, 0));
        Assert.Equal((49u, 2u), MediaEncoding.FrameRateRatio(24.5, 0, 0));
        Assert.Equal((61u, 2u), MediaEncoding.FrameRateRatio(30.5, 0, 0));
        Assert.Equal((30u, 1u), MediaEncoding.FrameRateRatio(double.NaN, 0, 0));
    }

    /// <summary>Rule B: whatever the ratio, it is never a hair above the target.</summary>
    [Fact]
    public void AFrameRateRatioIsNeverAboveItsTarget()
    {
        for (var target = 0.0004; target <= 30.5; target += 0.0137)
        {
            var (numerator, denominator) = MediaEncoding.FrameRateRatio(target, 0, 0);
            Assert.True((double)numerator / denominator <= target, $"{numerator}/{denominator} for {target:R}");
            Assert.True(numerator > 0 && denominator > 0);
        }
    }

    /// <summary>The encoder takes 44.1 and 48 kHz and nothing else.</summary>
    [Fact]
    public void TheSampleRateIsOneTheEncoderTakes()
    {
        Assert.Equal(48_000u, MediaEncoding.AacSampleRate(48_000));
        Assert.Equal(48_000u, MediaEncoding.AacSampleRate(96_000));
        Assert.Equal(48_000u, MediaEncoding.AacSampleRate(44_101));
        Assert.Equal(44_100u, MediaEncoding.AacSampleRate(44_100));
        Assert.Equal(44_100u, MediaEncoding.AacSampleRate(22_050));
        Assert.Equal(44_100u, MediaEncoding.AacSampleRate(null));
    }

    [Fact]
    public void MonoStaysMonoAndEverythingElseIsStereo()
    {
        Assert.Equal(1u, MediaEncoding.AacChannels(1));
        Assert.Equal(2u, MediaEncoding.AacChannels(2));
        Assert.Equal(2u, MediaEncoding.AacChannels(6));
        Assert.Equal(2u, MediaEncoding.AacChannels(null));
    }

    /// <summary>
    /// The encoder's documented rates are 96, 128, 160 and 192 kbit/s. The nearest for a target it may refuse is the
    /// largest under it — or 96 000, unless that would be above the source's own (rule B).
    /// </summary>
    [Fact]
    public void TheNearestEncoderRateKeepsRuleB()
    {
        Assert.Equal([96_000u, 128_000u, 160_000u, 192_000u], MediaEncoding.EncoderAacBitrates);
        Assert.Equal(128_000u, MediaEncoding.EncoderAacBitrate(128_000, null));
        Assert.Equal(96_000u, MediaEncoding.EncoderAacBitrate(112_000, 112_000));
        Assert.Equal(96_000u, MediaEncoding.EncoderAacBitrate(64_000, null));
        Assert.Equal(96_000u, MediaEncoding.EncoderAacBitrate(64_000, 705_600));
        Assert.Equal(96_000u, MediaEncoding.EncoderAacBitrate(96_000, 96_000));
        Assert.Null(MediaEncoding.EncoderAacBitrate(64_000, 80_000));
        Assert.Null(MediaEncoding.EncoderAacBitrate(48_000, 48_000));
    }

    [Fact]
    public void APickedTrackAsksForTheExactRateFirst()
    {
        // A stereo WAV: 128 000 is one of the encoder's own, so there is nothing to fall back to.
        Assert.Equal([new AacEncoding(48_000, 2, 128_000)], MediaEncoding.AudioAttempts(128_000, 2, 48_000, 1_411_200));
        // A mono WAV: 64 000, and then the encoder's lowest.
        Assert.Equal(
            [new AacEncoding(44_100, 1, 64_000), new AacEncoding(44_100, 1, 96_000)],
            MediaEncoding.AudioAttempts(64_000, 1, 44_100, 705_600));
        // A thin Ogg at 32 000: 96 000 would raise it, so there is no second chance.
        Assert.Equal([new AacEncoding(48_000, 1, 32_000)], MediaEncoding.AudioAttempts(32_000, 1, 48_000, 32_000));
    }

    /// <summary>The protocol's voice note — AAC-LC, mono, 44.1 kHz, 64 000 — and then what the encoder documents.</summary>
    [Fact]
    public void AVoiceNoteAsksForTheProtocolsNumbersFirst()
    {
        Assert.Equal(
            [new AacEncoding(44_100, 1, 64_000), new AacEncoding(44_100, 1, 96_000)],
            MediaEncoding.VoiceNoteAttempts());
        Assert.Equal(MediaPlan.VoiceNoteBitrate, MediaEncoding.VoiceNoteAttempts()[0].Bitrate);
    }

    /// <summary>
    /// The reasoning doc's first fixture: a 4K60 HDR portrait phone clip comes out 720×1280 displayed — ENCODED 1280×720
    /// with the turn carried — at 30 fps, 2 Mbit/s, H.264 High, AAC-LC 128k, asked for SDR.
    /// </summary>
    [Fact]
    public void A4K60HdrPortraitClipIsAskedFor720p30Sdr()
    {
        var phone = new VideoSource(2160, 3840, 60_000.0 / 1001, "video/quicktime", "hevc", "aac", 2, 40_000_000, 256_000, 150_000_000, 30_000);
        var plan = MediaPlan.PlanVideo(phone);
        Assert.Equal(VideoPlanKind.Transcode, plan.Kind);
        var attempts = MediaEncoding.VideoAttempts(plan.Target!.Value, 90, 60_000, 1001, 2, 48_000, 256_000, hdr: true);
        Assert.Equal(
            [
                new VideoEncoding(1280, 720, 90, 2_000_000, 30, 1, H264Profile.High, new AacEncoding(48_000, 2, 128_000), true),
                new VideoEncoding(1280, 720, 90, 2_000_000, 30, 1, H264Profile.Main, new AacEncoding(48_000, 2, 128_000), true),
            ],
            attempts);
    }

    /// <summary>A kept frame rate keeps its exact ratio; the bitrate is the planner's, to the bit.</summary>
    [Fact]
    public void AnNtscClipKeepsItsRatioAndItsCappedBitrate()
    {
        var clip = new VideoSource(1920, 1080, 30_000.0 / 1001, "video/mp4", "h264", "aac", 2, 1_234_567, 128_000, 20_000_000, 19_000);
        var target = MediaPlan.PlanVideo(clip).Target!.Value;
        var first = MediaEncoding.VideoAttempts(target, 0, 30_000, 1001, 2, 44_100, 128_000, hdr: false)[0];
        Assert.Equal((1280u, 720u, 0u), (first.Width, first.Height, first.Rotation));
        Assert.Equal((30_000u, 1001u), (first.FrameRateNumerator, first.FrameRateDenominator));
        Assert.Equal(1_234_567u, first.Bitrate);
        Assert.Equal(new AacEncoding(44_100, 2, 128_000), first.Audio);
        Assert.False(first.ToSdr);
    }

    /// <summary>A mono clip asks for 64 000, then the encoder's 96 000 — and a thin one never above its own.</summary>
    [Fact]
    public void AMonoClipFallsBackToTheEncodersRateOnlyWhereRuleBAllows()
    {
        var mono = new VideoTarget(1280, 720, 30, 2_000_000, 64_000);
        var attempts = MediaEncoding.VideoAttempts(mono, 0, 30, 1, 1, 48_000, 128_000, hdr: false);
        Assert.Equal(
            [
                (H264Profile.High, (uint?)64_000),
                (H264Profile.High, (uint?)96_000),
                (H264Profile.Main, (uint?)96_000),
            ],
            attempts.Select(attempt => (attempt.Profile, attempt.Audio?.Bitrate)));

        var thin = new VideoTarget(1280, 720, 30, 2_000_000, 48_000);
        Assert.Equal(
            [(H264Profile.High, (uint?)48_000), (H264Profile.Main, (uint?)48_000)],
            MediaEncoding.VideoAttempts(thin, 0, 30, 1, 1, 48_000, 48_000, hdr: false).Select(attempt => (attempt.Profile, attempt.Audio?.Bitrate)));
    }

    /// <summary>
    /// A 4K clip with 64 000 mono AAC — a messenger's re-share, a screen recording. The encoder's lowest documented rate
    /// is above it, so rule B leaves nothing to ENCODE with once 64 000 is refused; the track is already what the
    /// profile asks for, and is asked for as itself — passed through — under High and under Main. Without that these
    /// clips had no second chance and went untranscoded, however large.
    /// </summary>
    [Fact]
    public void AThinAacTrackIsPassedThroughWhereTheEncoderHasNoRateForIt()
    {
        var thin = new VideoTarget(1280, 720, 30, 2_000_000, 64_000);
        var attempts = MediaEncoding.VideoAttempts(thin, 0, 30, 1, 1, 22_050, 64_000, hdr: false, audioIsAac: true);
        Assert.Equal(
            [
                (H264Profile.High, new AacEncoding(44_100, 1, 64_000), false),
                // Its OWN numbers — 22.05 kHz — because nothing is encoded.
                (H264Profile.High, new AacEncoding(22_050, 1, 64_000), true),
                (H264Profile.Main, new AacEncoding(22_050, 1, 64_000), true),
            ],
            attempts.Select(attempt => (attempt.Profile, attempt.Audio!.Value, attempt.KeepAudio)));

        // Stereo at 112 000: passed through before the encoder's 96 000 costs it a generation AND bits.
        var stereo = new VideoTarget(1280, 720, 30, 2_000_000, 112_000);
        Assert.Equal(
            [
                (H264Profile.High, (uint?)112_000, false),
                (H264Profile.High, (uint?)112_000, true),
                (H264Profile.High, (uint?)96_000, false),
                (H264Profile.Main, (uint?)96_000, false),
            ],
            MediaEncoding.VideoAttempts(stereo, 0, 30, 1, 2, 48_000, 112_000, hdr: false, audioIsAac: true)
                .Select(attempt => (attempt.Profile, attempt.Audio?.Bitrate, attempt.KeepAudio)));
    }

    /// <summary>Passed through only when it is AAC, mono or stereo, and already at or under the profile's row.</summary>
    [Fact]
    public void ATrackIsPassedThroughOnlyWhenTheProfileAlreadyTakesIt()
    {
        Assert.True(MediaEncoding.KeepsAudio(true, 64_000, 1, 44_100, 64_000));
        Assert.True(MediaEncoding.KeepsAudio(true, 128_000, 2, 48_000, 128_000));
        Assert.False(MediaEncoding.KeepsAudio(false, 64_000, 1, 44_100, 64_000), "an MP3 track is not AAC");
        // Mono at 80 000 has a row of 64 000: the target is below the source, so it has to come down.
        Assert.False(MediaEncoding.KeepsAudio(true, 64_000, 1, 44_100, 80_000), "above the profile's row");
        Assert.False(MediaEncoding.KeepsAudio(true, 128_000, 6, 48_000, 128_000), "5.1 is folded down, not kept");
        Assert.False(MediaEncoding.KeepsAudio(true, 128_000, 2, 48_000, null), "a rate nobody stated");
        Assert.False(MediaEncoding.KeepsAudio(true, 128_000, null, 48_000, 128_000));
        Assert.False(MediaEncoding.KeepsAudio(true, 128_000, 2, null, 128_000));

        // A track above its row is never among the attempts as itself.
        var loud = new VideoTarget(1280, 720, 30, 2_000_000, 128_000);
        Assert.DoesNotContain(
            MediaEncoding.VideoAttempts(loud, 0, 30, 1, 2, 48_000, 256_000, hdr: false, audioIsAac: true),
            attempt => attempt.KeepAudio);
    }

    /// <summary>No audio track in, none out: a transcode does not invent silence.</summary>
    [Fact]
    public void ASilentClipAsksForNoAudio()
    {
        var silent = new VideoTarget(640, 360, 25, 417_000, null);
        var attempts = MediaEncoding.VideoAttempts(silent, 0, 25, 1, null, null, null, hdr: false);
        Assert.Equal(2, attempts.Count);
        Assert.All(attempts, attempt => Assert.Null(attempt.Audio));
        Assert.Equal((640u, 360u, 25u, 1u, 417_000u), (attempts[0].Width, attempts[0].Height, attempts[0].FrameRateNumerator, attempts[0].FrameRateDenominator, attempts[0].Bitrate));
    }

    /// <summary>
    /// The read-back: the sides exactly and the SAME turn. A frame turned into the pixels, a turn lost, or a turn the
    /// other way are all failures — each is a video that plays sideways, squashed or upside down.
    /// </summary>
    [Fact]
    public void AResultMatchesOnlyWhenItIsWhatWasAskedFor()
    {
        var asked = new VideoEncoding(1280, 720, 90, 2_000_000, 30, 1, H264Profile.High, null, false);
        Assert.True(MediaEncoding.Matches(asked, 1280, 720, 90));
        Assert.False(MediaEncoding.Matches(asked, 720, 1280, 0), "turned into the pixels");
        Assert.False(MediaEncoding.Matches(asked, 1280, 720, 0), "the turn lost");
        Assert.False(MediaEncoding.Matches(asked, 1280, 720, 270), "turned the other way");
        Assert.False(MediaEncoding.Matches(asked, 1282, 720, 90), "not the size asked for");
    }

    /// <summary>
    /// A 1080p60 source asked for at 30 and written at 60 has a bitrate worked out for half its frames, and is outside
    /// the profile's "at most 30": a failed transcode, where it was once logged and uploaded. A rate BELOW the one asked
    /// for is fine — no transcoder invents frames — and so is NTSC's hair under 30.
    /// </summary>
    [Fact]
    public void AResultFasterThanAskedIsNotWhatWasAskedFor()
    {
        var thirty = new VideoEncoding(1280, 720, 0, 2_000_000, 30, 1, H264Profile.High, null, false);
        Assert.False(MediaEncoding.FrameRateCameOut(thirty, 60));
        Assert.False(MediaEncoding.FrameRateCameOut(thirty, 60_000.0 / 1001));
        Assert.False(MediaEncoding.FrameRateCameOut(thirty, 30.6));
        Assert.True(MediaEncoding.FrameRateCameOut(thirty, 30));
        Assert.True(MediaEncoding.FrameRateCameOut(thirty, 30.5));
        Assert.True(MediaEncoding.FrameRateCameOut(thirty, 30_000.0 / 1001));
        Assert.True(MediaEncoding.FrameRateCameOut(thirty, 12), "a slow source stays slow");
        Assert.True(MediaEncoding.FrameRateCameOut(thirty, null), "a rate nobody can read is not judged");

        var film = thirty with { FrameRateNumerator = 24_000, FrameRateDenominator = 1001 };
        Assert.True(MediaEncoding.FrameRateCameOut(film, 24));
        Assert.False(MediaEncoding.FrameRateCameOut(film, 25));
    }

    /// <summary>
    /// An encoder that takes 64 000 and writes its own 96 000 raised a bitrate without refusing anything (rule B). A
    /// tenth either way is an encoder's arithmetic, not a different rate.
    /// </summary>
    [Fact]
    public void AnAudioRateTheEncoderChoseForItselfIsNotTheRateAskedFor()
    {
        Assert.False(MediaEncoding.AudioRateCameOut(64_000, 96_000));
        Assert.False(MediaEncoding.AudioRateCameOut(112_000, 128_000));
        Assert.True(MediaEncoding.AudioRateCameOut(64_000, 64_000));
        Assert.True(MediaEncoding.AudioRateCameOut(128_000, 131_072));
        Assert.True(MediaEncoding.AudioRateCameOut(128_000, 140_800));
        Assert.False(MediaEncoding.AudioRateCameOut(128_000, 140_801));
        Assert.True(MediaEncoding.AudioRateCameOut(128_000, 96_000), "under is never a raise");
        Assert.True(MediaEncoding.AudioRateCameOut(64_000, 0), "not stated");
        Assert.True(MediaEncoding.AudioRateCameOut(64_000, null));
    }

    [Fact]
    public void ARecordingOrAReEncodeIsAacInTheChannelsAskedFor()
    {
        var note = MediaEncoding.VoiceNoteAttempts()[0];
        Assert.True(MediaEncoding.AacCameOut(note, "aac", 1));
        Assert.True(MediaEncoding.AacCameOut(note, "aac", 0), "a count not stated is not judged");
        Assert.True(MediaEncoding.AacCameOut(note, "aac", null));
        Assert.False(MediaEncoding.AacCameOut(note, "aac", 2), "stereo where mono was asked for");
        Assert.False(MediaEncoding.AacCameOut(note, "mp3", 1));
        Assert.False(MediaEncoding.AacCameOut(note, "unknown", 1));
    }

    /// <summary>
    /// A video whose audio sample entry Media Foundation does not surface: no track is read, none is asked for, none comes
    /// out — and the read-back agrees with itself over a silent upload. The shell's numbers are the only witness.
    /// </summary>
    [Fact]
    public void SoundTheShellReadsAndMediaFoundationDoesNotIsHiddenSound()
    {
        Assert.True(MediaEncoding.HidesAudio(audioTrackRead: false, 2, null, null));
        Assert.True(MediaEncoding.HidesAudio(audioTrackRead: false, null, 48_000, null));
        Assert.True(MediaEncoding.HidesAudio(audioTrackRead: false, 0, 0, 96_000));
        Assert.False(MediaEncoding.HidesAudio(audioTrackRead: false, null, null, null), "a silent clip");
        Assert.False(MediaEncoding.HidesAudio(audioTrackRead: false, 0, 0, 0), "a silent clip whose shell says nought");
        Assert.False(MediaEncoding.HidesAudio(audioTrackRead: true, 2, 48_000, 128_000));
    }

    /// <summary>HDV's 1440×1080 is shown 16:9 through 4:3 pixels; scaling its stored sides would squash it.</summary>
    [Fact]
    public void OnlySquareOrUnstatedPixelsAreScaled()
    {
        Assert.True(MediaEncoding.SquarePixels(1, 1));
        Assert.True(MediaEncoding.SquarePixels(8, 8));
        Assert.True(MediaEncoding.SquarePixels(0, 0));
        Assert.True(MediaEncoding.SquarePixels(0, 1));
        Assert.True(MediaEncoding.SquarePixels(1, 0));
        Assert.False(MediaEncoding.SquarePixels(4, 3));
        Assert.False(MediaEncoding.SquarePixels(10, 11));
    }

    // --- what goes -------------------------------------------------------------------------------------------------

    private const long Ceiling = MediaPrep.SizeLimit;

    /// <summary>Rule D, with the numbers of the reasoning doc's clips.</summary>
    [Fact]
    public void ASmallerResultGoesAndABiggerOneIsThrownAway()
    {
        // A 60 MB clip brought to 8 MB.
        Assert.Equal(Chosen.Result, MediaEncoding.Choose(60_000_000, true, 8_000_000, moovFirst: true, needsMoovFirst: true, Ceiling));
        // A thin clip that grew: the original, which could go.
        Assert.Equal(Chosen.Original, MediaEncoding.Choose(3_000_000, true, 3_000_001, moovFirst: true, needsMoovFirst: true, Ceiling));
        // Exactly the source's size is not "bigger".
        Assert.Equal(Chosen.Result, MediaEncoding.Choose(3_000_000, true, 3_000_000, moovFirst: true, needsMoovFirst: true, Ceiling));
        // A QuickTime container lying about its bytes cannot go as a video: the result is the only thing that can.
        Assert.Equal(Chosen.Result, MediaEncoding.Choose(3_000_000, false, 4_000_000, moovFirst: true, needsMoovFirst: true, Ceiling));
    }

    /// <summary>A 400 MB 4K clip is what this is for; one still over the ceiling afterwards is refused as it always was.</summary>
    [Fact]
    public void AResultOverTheCeilingIsRefusedOnlyWhenTheSourceCouldNotGoEither()
    {
        Assert.Equal(Chosen.Result, MediaEncoding.Choose(400_000_000, false, 40_000_000, moovFirst: true, needsMoovFirst: true, Ceiling));
        Assert.Equal(Chosen.Result, MediaEncoding.Choose(400_000_000, false, Ceiling, moovFirst: true, needsMoovFirst: true, Ceiling));
        Assert.Equal(Chosen.TooLarge, MediaEncoding.Choose(400_000_000, false, Ceiling + 1, moovFirst: true, needsMoovFirst: true, Ceiling));
        // A sendable source under a result over the ceiling is a bigger result: the source goes.
        Assert.Equal(Chosen.Original, MediaEncoding.Choose(90_000_000, true, Ceiling + 1, moovFirst: false, needsMoovFirst: true, Ceiling));
    }

    /// <summary>
    /// The index in front is asked of a VIDEO. A 40 MB WAV re-encoded to a 4 MB M4A whose index would not move once went
    /// as the 40 MB WAV — and an Ogg as the Ogg, which does not play on iOS or macOS.
    /// </summary>
    [Fact]
    public void AnIndexThatCouldNotBeMovedCostsAVideoItsTranscodeAndASoundFileNothing()
    {
        Assert.Equal(Chosen.Original, MediaEncoding.Choose(60_000_000, true, 8_000_000, moovFirst: false, needsMoovFirst: true, Ceiling));
        // The source could not have gone at all: index at the end beats nothing.
        Assert.Equal(Chosen.Result, MediaEncoding.Choose(400_000_000, false, 40_000_000, moovFirst: false, needsMoovFirst: true, Ceiling));
        Assert.Equal(Chosen.Result, MediaEncoding.Choose(40_000_000, true, 4_000_000, moovFirst: false, needsMoovFirst: false, Ceiling));
        Assert.Equal(Chosen.Result, MediaEncoding.Choose(40_000_000, true, 4_000_000, moovFirst: true, needsMoovFirst: false, Ceiling));
    }

    /// <summary>
    /// Rule C's promise, over every combination: a source that could go is NEVER refused, whatever its transcode turned
    /// out to be; nothing over the ceiling is ever the thing sent; and the original is only ever named when it can go.
    /// </summary>
    [Fact]
    public void NoSendThatWorkedBeforeIsLostToItsTranscode()
    {
        long[] sizes = [1, 1_000, 3_000_000, Ceiling - 1, Ceiling, Ceiling + 1, 4 * Ceiling];
        foreach (var source in sizes)
        {
            foreach (var result in sizes)
            {
                foreach (var sendable in new[] { true, false })
                {
                    // Sendable is within the ceiling by definition (MediaPlan.Sendable).
                    if (sendable && source > Ceiling)
                    {
                        continue;
                    }
                    foreach (var moovFirst in new[] { true, false })
                    {
                        foreach (var video in new[] { true, false })
                        {
                            var chosen = MediaEncoding.Choose(source, sendable, result, moovFirst, video, Ceiling);
                            var said = $"{source} → {result}, sendable {sendable}, moov first {moovFirst}, video {video}: {chosen}";
                            Assert.True(!sendable || chosen != Chosen.TooLarge, said);
                            Assert.True(chosen != Chosen.Result || result <= Ceiling, said);
                            Assert.True(chosen != Chosen.Original || sendable, said);
                            Assert.True(chosen != Chosen.Result || !sendable || result <= source, said);
                        }
                    }
                }
            }
        }
    }

    [Fact]
    public void ATranscodeHasACeiling()
    {
        Assert.Equal(TimeSpan.FromMinutes(5), MediaEncoding.TranscodeCeiling(60_000));
        Assert.Equal(TimeSpan.FromMinutes(10), MediaEncoding.TranscodeCeiling(null));
        Assert.Equal(TimeSpan.FromMinutes(10), MediaEncoding.TranscodeCeiling(0));
    }

    /// <summary>The reasoning doc's fourth fixture: a WAV becomes AAC 128k; a 128 kbit/s MP3 is untouched.</summary>
    [Fact]
    public void AWavIsReEncodedAndA128kMp3IsNot()
    {
        var wav = new AudioSource("audio/wav", "pcm", 2, 1_411_200, 31_752_000, 180_000);
        var plan = MediaPlan.PlanAudio(wav);
        Assert.Equal(AudioPlan.Transcode(128_000), plan);
        Assert.Equal([new AacEncoding(44_100, 2, 128_000)], MediaEncoding.AudioAttempts(plan.Bitrate, wav.Channels, 44_100, MediaPlan.SourceAudioBitrate(wav)));
        Assert.Equal(AudioPlan.Keep, MediaPlan.PlanAudio(new AudioSource("audio/mpeg", "mp3", 2, 128_000, 2_880_000, 180_000)));
    }
}
