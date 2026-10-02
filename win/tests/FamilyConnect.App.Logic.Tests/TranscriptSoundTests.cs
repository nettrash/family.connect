using FamilyConnect.App.Logic;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// The sound a device sends with a transcript request when the server cannot send its own copy (docs/protocol.md,
/// "Transcripts on request", the multipart form): which way it is made — the AAC track passed through, or encoded at
/// 64 kbit/s mono — what bounds it, and what is offered at all.
/// </summary>
public sealed class TranscriptSoundTests
{
    private const long MiB25 = 26_214_400;

    /// <summary>
    /// PASS THROUGH WHAT IS ALREADY AAC: a phone's video carries AAC, and the first way of asking is to copy that track
    /// untouched — then the voice-note encodes, should the copy be refused or come out wrong.
    /// </summary>
    [Fact]
    public void AnAacTrackIsPassedThroughFirst()
    {
        var attempts = TranscriptSound.Attempts("aac", 128_000, 48_000, 60_000, MiB25);
        Assert.Equal(SoundAttempt.Passthrough, attempts[0]);
        Assert.Null(attempts[0].Aac);
        Assert.All(attempts.Skip(1), attempt => Assert.Equal(SoundWay.Reencode, attempt.Way));
        // The encode keeps 48 kHz as 48, and is MONO at 64 kbit/s first.
        Assert.Equal(new AacEncoding(48_000, 1, 64_000), attempts[1].Aac);
    }

    /// <summary>
    /// RE-ENCODE EVERYTHING ELSE — Opus or Vorbis in an Ogg, MP3, PCM, and a codec nobody can name — at a voice note's
    /// numbers: AAC-LC, mono, 64 kbit/s, then the encoder's nearest documented rate. Never a passthrough.
    /// </summary>
    [Theory]
    [InlineData("unknown")]
    [InlineData("mp3")]
    [InlineData("pcm")]
    [InlineData("flac")]
    [InlineData("alac")]
    public void AnythingButAacIsReencoded(string codec)
    {
        var attempts = TranscriptSound.Attempts(codec, 320_000, 44_100, 60_000, MiB25);
        Assert.Equal(
            [new SoundAttempt(SoundWay.Reencode, new AacEncoding(44_100, 1, 64_000)),
             new SoundAttempt(SoundWay.Reencode, new AacEncoding(44_100, 1, 96_000))],
            attempts);
    }

    /// <summary>NO SOUND TRACK, NOTHING TO SEND: no attempt at all, which the device says as "not available".</summary>
    [Fact]
    public void NoTrackIsNoAttempt()
    {
        Assert.Empty(TranscriptSound.Attempts(null, null, null, 60_000, MiB25));
    }

    /// <summary>
    /// THE SIZE BOUND decides what is tried: a copy whose own rate would be over the ceiling for this length is skipped
    /// for the encode; an encode that would be over too is skipped; and a length nothing can fit tries nothing.
    /// </summary>
    [Fact]
    public void WhatCannotFitIsNotTried()
    {
        // 20 minutes of 256 kbit/s AAC is ~38 MB: too big to copy, and 64 kbit/s (~9.8 MB) fits.
        var twenty = 20 * 60_000L;
        var attempts = TranscriptSound.Attempts("aac", 256_000, 44_100, twenty, MiB25);
        Assert.DoesNotContain(SoundAttempt.Passthrough, attempts);
        Assert.Equal(64_000u, attempts[0].Aac!.Value.Bitrate);

        // 40 minutes: 64 kbit/s fits (~19.6 MB), 96 kbit/s (~29 MB) does not.
        var forty = 40 * 60_000L;
        Assert.Equal(
            [new SoundAttempt(SoundWay.Reencode, new AacEncoding(44_100, 1, 64_000))],
            TranscriptSound.Attempts("mp3", 128_000, 44_100, forty, MiB25));

        // An hour does not fit at all, and a lower ceiling the server names is held to as well.
        Assert.Empty(TranscriptSound.Attempts("aac", 64_000, 44_100, 60 * 60_000L, MiB25));
        Assert.Empty(TranscriptSound.Attempts("mp3", 128_000, 44_100, 60_000, 100_000));
    }

    /// <summary>A rate or a length nobody states is TRIED, never assumed too big — the result is measured before it goes.</summary>
    [Fact]
    public void UnknownNumbersAreTriedAndMeasured()
    {
        Assert.Equal(SoundAttempt.Passthrough, TranscriptSound.Attempts("aac", null, null, 60 * 60_000L, MiB25)[0]);
        Assert.Equal(3, TranscriptSound.Attempts("aac", 512_000, null, null, MiB25).Count);
    }

    /// <summary>
    /// THE ESTIMATE: the sound, a fiftieth for the sample tables and a fixed head — 64 kbit/s is 8 000 bytes a second,
    /// so fifty minutes is about 24.5 MB and still fits 25 MiB; nonsense in is nothing, not an overflow.
    /// </summary>
    [Fact]
    public void TheEstimateIsTheSoundAndItsOverhead()
    {
        Assert.Equal(8_000 + 160 + 16_384, TranscriptSound.EstimatedBytes(64_000, 1_000));
        Assert.True(TranscriptSound.EstimatedBytes(64_000, 50 * 60_000L) <= MiB25);
        Assert.True(TranscriptSound.EstimatedBytes(64_000, 55 * 60_000L) > MiB25);
        Assert.Equal(0, TranscriptSound.EstimatedBytes(0, 1_000));
        Assert.Equal(0, TranscriptSound.EstimatedBytes(64_000, -1));
        Assert.Equal(long.MaxValue, TranscriptSound.EstimatedBytes(long.MaxValue, long.MaxValue));
    }

    /// <summary>
    /// HIDDEN ONLY WHEN THE ATTACHMENT ALREADY SAYS NO: a stated length whose 64 kbit/s sound is over the ceiling. A length
    /// nobody stated is offered.
    /// </summary>
    [Fact]
    public void OnlyAStatedLengthThatCannotFitHidesTheAction()
    {
        Assert.True(TranscriptSound.CanFit(50 * 60_000L, MiB25));
        Assert.False(TranscriptSound.CanFit(55 * 60_000L, MiB25));
        Assert.False(TranscriptSound.CanFit(60_000, 100_000));
        Assert.True(TranscriptSound.CanFit(null, MiB25));
        Assert.True(TranscriptSound.CanFit(0, MiB25));
    }

    /// <summary>What may go: something, and never a byte over the ceiling.</summary>
    [Fact]
    public void TheMadeSoundMustFitTheCeiling()
    {
        Assert.True(TranscriptSound.Fits(MiB25, MiB25));
        Assert.False(TranscriptSound.Fits(MiB25 + 1, MiB25));
        Assert.False(TranscriptSound.Fits(0, MiB25));
    }

    /// <summary>
    /// WHAT CAME OUT, JUDGED: AAC within the ceiling is taken; a copy that came out otherwise, or too big, leaves the encode
    /// to try; an encode at the wrong rate leaves the next rate; an encode that is not AAC, or anything carrying a picture,
    /// ends the asking — no other way will put that right.
    /// </summary>
    [Fact]
    public void WhatCameOutIsJudged()
    {
        var encode = new SoundAttempt(SoundWay.Reencode, new AacEncoding(44_100, 1, 64_000));
        var copy = SoundAttempt.Passthrough;

        Assert.Equal(AttemptEnd.Taken, TranscriptSound.Judge(copy, "aac", 256_000, 1_000_000, false, MiB25));
        Assert.Equal(AttemptEnd.Taken, TranscriptSound.Judge(encode, "aac", 64_000, 1_000_000, false, MiB25));
        Assert.Equal(AttemptEnd.Taken, TranscriptSound.Judge(encode, "aac", null, 1_000_000, false, MiB25));

        Assert.Equal(AttemptEnd.Refused, TranscriptSound.Judge(copy, "mp3", null, 1_000_000, false, MiB25));
        Assert.Equal(AttemptEnd.Refused, TranscriptSound.Judge(copy, null, null, 1_000_000, false, MiB25));
        Assert.Equal(AttemptEnd.Refused, TranscriptSound.Judge(copy, "aac", 256_000, MiB25 + 1, false, MiB25));
        Assert.Equal(AttemptEnd.Refused, TranscriptSound.Judge(encode, "aac", 96_000, 1_000_000, false, MiB25));
        Assert.Equal(AttemptEnd.Refused, TranscriptSound.Judge(encode, "aac", 64_000, MiB25 + 1, false, MiB25));
        Assert.Equal(AttemptEnd.Refused, TranscriptSound.Judge(encode, "aac", 64_000, 0, false, MiB25));

        Assert.Equal(AttemptEnd.Wrong, TranscriptSound.Judge(encode, "pcm", 64_000, 1_000_000, false, MiB25));
        Assert.Equal(AttemptEnd.Wrong, TranscriptSound.Judge(copy, "aac", 64_000, 1_000_000, true, MiB25));
        Assert.Equal(AttemptEnd.Wrong, TranscriptSound.Judge(encode, "aac", 64_000, 1_000_000, true, MiB25));
    }

    /// <summary>
    /// The held file's extension for Media Foundation, which reads by it: by the stored type, by the name's own, and
    /// <c>bin</c> otherwise — never anything from a name but letters and digits, since it becomes part of a path.
    /// </summary>
    [Theory]
    [InlineData("video/mp4", null, "mp4")]
    [InlineData("video/quicktime", "clip.mp4", "mov")]
    [InlineData("audio/ogg", null, "ogg")]
    [InlineData("audio/ogg; codecs=opus", null, "ogg")]
    [InlineData("AUDIO/MPEG", null, "mp3")]
    [InlineData("audio/mp4", null, "m4a")]
    [InlineData("audio/m4a", null, "m4a")]
    [InlineData("audio/wav", null, "wav")]
    [InlineData("application/octet-stream", "talk.OPUS", "opus")]
    [InlineData(null, "talk.flac", "flac")]
    [InlineData(null, "talk.../../x", "bin")]
    [InlineData(null, "talk.mp4 x", "bin")]
    [InlineData(null, "talk.verylong", "bin")]
    [InlineData(null, null, "bin")]
    public void TheHeldFileIsNamedForMediaFoundation(string? mime, string? name, string extension) =>
        Assert.Equal(extension, TranscriptSound.TempExtension(mime, name));

    /// <summary>
    /// The device's own "cannot" is final, and says which it was: too long even re-encoded, or no sound it could read.
    /// "Not available for this message." stays for a recording there is nothing to ask with at all.
    /// </summary>
    [Fact]
    public void TheDevicesCannotIsFinal()
    {
        foreach (var (error, said) in new[]
                 {
                     (TranscriptSound.TooLong, "This recording is too long to turn into text."),
                     (TranscriptSound.Unreadable, "Couldn't read the sound in this file."),
                     (TranscriptSound.Unavailable, "Not available for this message."),
                 })
        {
            Assert.False(error.Transient);
            Assert.False(TranscriptRules.MayRetry(error));
            Assert.Equal(said, TranscriptRules.FailureSentence(error, FamilyConnect.Core.EnglishCatalog.Instance));
        }
        Assert.Equal(ErrorCodes.NotTranscribable, TranscriptSound.Unavailable.Code);
    }

    /// <summary>The part, as the server reads it: one part named <c>audio</c>, AAC in MPEG-4.</summary>
    [Fact]
    public void ThePartIsAnM4aNamedAudio()
    {
        Assert.Equal("audio", TranscriptSound.PartName);
        Assert.Equal("audio/mp4", TranscriptSound.Mime);
        Assert.Equal("audio.m4a", TranscriptSound.FileName);
    }
}
