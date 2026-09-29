namespace FamilyConnect.Core;

/// <summary>
/// A video the client is about to send as <c>kind=video</c>, as its reader saw it (<c>fc_text::media_plan::VideoSource</c>).
/// </summary>
/// <remarks>
/// Codecs are lowercase names, not a platform's subtype strings, so four readers can agree on them: <c>"h264"</c>,
/// <c>"hevc"</c>, <c>"av1"</c>, <c>"vp9"</c>, and <c>"unknown"</c> for anything a reader cannot name. Only <c>"h264"</c>
/// and, for the audio, <c>"aac"</c> (any AAC profile) are ever asked about. Every integer is a <see cref="long"/> where
/// the original has a <c>u32</c> or a <c>u64</c>; a count of 0 or less is "unknown", as the original's 0 is.
/// </remarks>
/// <param name="Width">The size as it is DISPLAYED — after its rotation. 0 when the reader could not tell.</param>
/// <param name="FrameRate">Frames per second; anything but a finite number above zero is unknown.</param>
/// <param name="Container">The type it would be uploaded as — <c>"video/mp4"</c> or <c>"video/quicktime"</c>.</param>
/// <param name="AudioCodec">Null when the file has NO audio track; a track nobody can name is <c>"unknown"</c>.</param>
/// <param name="VideoBitrate">The rate the container STATES, in bit/s.</param>
public sealed record VideoSource(
    long Width,
    long Height,
    double? FrameRate,
    string Container,
    string VideoCodec,
    string? AudioCodec,
    long? AudioChannels,
    long? VideoBitrate,
    long? AudioBitrate,
    long SizeBytes,
    long? DurationMs);

/// <summary>What a video is transcoded TO: even sides in the source's orientation, never larger than it.</summary>
/// <param name="AudioBitrate">Null when the source has no audio track — a transcode does not invent silence.</param>
public readonly record struct VideoTarget(long Width, long Height, double FrameRate, long VideoBitrate, long? AudioBitrate);

public enum VideoPlanKind
{
    /// <summary>Rule A: already within the profile, and the ORIGINAL is uploaded.</summary>
    Keep,

    /// <summary>Rule 5: transcode to <see cref="VideoPlan.Target"/>.</summary>
    Transcode,

    /// <summary>
    /// Rule C: there is no size to scale to — none was read, or the short side is one pixel and an even target would
    /// have none — so this platform "cannot transcode this source at all" (<see cref="MediaPlan.AfterFailure"/>).
    /// </summary>
    Fallback,
}

/// <summary>What happens to a video; <see cref="Target"/> is set for a transcode only.</summary>
public readonly record struct VideoPlan(VideoPlanKind Kind, VideoTarget? Target = null)
{
    public static VideoPlan Keep => new(VideoPlanKind.Keep);

    public static VideoPlan Fallback => new(VideoPlanKind.Fallback);

    public static VideoPlan Transcode(VideoTarget target) => new(VideoPlanKind.Transcode, target);
}

/// <summary>
/// A sound file the member PICKED — never a voice note, which is recorded to the profile directly
/// (<c>fc_text::media_plan::AudioSource</c>).
/// </summary>
/// <param name="Container">
/// The type the file is, lowercase, no parameters: one the server accepts as audio, or the platform's own name for one
/// it does not (<c>"audio/aiff"</c>, <c>"audio/flac"</c>).
/// </param>
/// <param name="Codec">
/// <c>"pcm"</c> (WAV and AIFF are both PCM), <c>"flac"</c>, <c>"alac"</c>, <c>"aac"</c>, <c>"mp3"</c>, <c>"vorbis"</c>,
/// <c>"opus"</c>, or <c>"unknown"</c>.
/// </param>
/// <param name="Bitrate">The rate the file STATES, in bit/s.</param>
public sealed record AudioSource(
    string Container,
    string Codec,
    long? Channels,
    long? Bitrate,
    long SizeBytes,
    long? DurationMs);

public enum AudioPlanKind
{
    /// <summary>Uploaded untouched.</summary>
    Keep,

    /// <summary>Re-encoded as M4A (<c>audio/mp4</c>), AAC-LC, at <see cref="AudioPlan.Bitrate"/>.</summary>
    Transcode,
}

/// <summary>What happens to a picked sound file; <see cref="Bitrate"/> is set for a transcode only.</summary>
public readonly record struct AudioPlan(AudioPlanKind Kind, long Bitrate = 0)
{
    public static AudioPlan Keep => new(AudioPlanKind.Keep);

    public static AudioPlan Transcode(long bitrate) => new(AudioPlanKind.Transcode, bitrate);
}

/// <summary>What rule C sends when a transcode fails, or the platform cannot do one.</summary>
public enum OnFailure
{
    /// <summary>The original, untouched, as its kind.</summary>
    Original,

    /// <summary>Whatever the client did before the section existed — as a <c>file</c>, or refused. The old rule, unchanged.</summary>
    TodaysPath,
}

/// <summary>Which bytes rule D uploads.</summary>
public enum Upload
{
    Source,
    Result,
}

/// <summary>
/// What a picked video or sound file becomes before it is uploaded (docs/protocol.md, "Preparing media before
/// upload"; the reasoning is docs/media-upload-2026-09-28.md, issue #74). Ported from <c>fc_text::media_plan</c> and
/// held to it, case for case, by <c>MediaPlanOracleTests</c>.
/// </summary>
/// <remarks>
/// <para>
/// <b>THIS IS ONLY THE DECISION, AS ARITHMETIC.</b> The server never transcodes, so the size of a family's history is
/// decided on the sending device — by four codebases that have to reach the same answer for the same file. iOS
/// compressing to 1080p while Android compressed to 720p is what happens when nothing writes the target down. What the
/// source is, as Windows read it, goes in; what to do with it comes out. Reading and encoding are
/// <c>MediaPreparing</c>'s business, and the numbers that go into a Media Foundation profile are
/// <c>MediaEncoding</c>'s.
/// </para>
/// <para>
/// <b>INTEGERS EVERYWHERE THE PROTOCOL SAYS SO</b>, a <see cref="double"/> only where a frame rate enters, and every
/// function total: a number a reader could not fill in is "unknown", never an exception. One rule for every optional
/// number — a count of 0 (or, here, less) is unknown, and so is a frame rate that is not a finite number above zero —
/// so no port has to guess which of its platform's "unknown"s is which.
/// </para>
/// </remarks>
public static class MediaPlan
{
    /// <summary>The target's SHORT side at most — a 720p clip is indistinguishable from 1080p in a chat bubble.</summary>
    public const long MaxShortSide = 720;

    /// <summary>The frame rate a transcode is capped at.</summary>
    public const double MaxFrameRate = 30.0;

    /// <summary>
    /// The frame rate a source may have and still count as "at most 30": readers report a nominal 30 as 30.0003, and
    /// re-encoding a 30 fps clip for that would only cost quality. Above it becomes <see cref="MaxFrameRate"/>; at or
    /// below it is kept exactly, 29.97, 25 and 24 included.
    /// </summary>
    public const double FrameRateTolerance = 30.5;

    /// <summary>The video bitrate's floor and ceiling; 2 000 000 is the profile's rate at 1280×720, 30 fps.</summary>
    public const long MinVideoBitrate = 250_000;
    public const long MaxVideoBitrate = 2_000_000;

    /// <summary>AAC-LC for the audio in a video and for a re-encoded sound file. Mono gets half.</summary>
    public const long StereoAudioBitrate = 128_000;
    public const long MonoAudioBitrate = 64_000;

    /// <summary>A voice note — recorded to the profile directly, never planned: AAC-LC, mono, 44.1 or 48 kHz.</summary>
    public const long VoiceNoteBitrate = 64_000;

    /// <summary>MP3 or AAC at or below this is uploaded untouched: a second lossy generation costs more than it saves.</summary>
    public const long MaxKeptLossyAudioBitrate = 192_000;

    /// <summary>The types the server takes as each kind (server <c>Attachment::ACCEPTED</c>; docs/protocol.md, "Audio").</summary>
    private static readonly string[] AcceptedVideo = ["video/mp4", "video/quicktime"];
    private static readonly string[] AcceptedAudio = ["audio/mp4", "audio/m4a", "audio/mpeg", "audio/wav", "audio/ogg"];

    // --- unknowns --------------------------------------------------------------------------------------------------

    /// <summary>A count a reader handed over, or null: 0 is what a container writes for a rate it does not state.</summary>
    private static long? Known(long? value) => value is > 0 ? value : null;

    /// <summary>A frame rate worth believing, or null: absent, zero, negative, infinite and NaN are all "unknown".</summary>
    public static double? KnownFrameRate(double? frameRate) =>
        frameRate is { } rate && double.IsFinite(rate) && rate > 0.0 ? rate : null;

    // --- the rules -------------------------------------------------------------------------------------------------

    /// <summary>
    /// A bitrate the container does not state: <c>size × 8 × 1000 ÷ duration_ms − audio_bitrate</c>, in integers (the
    /// division truncates). Null when it "cannot be estimated": no duration, or nothing left once the audio is taken
    /// off. It counts every byte of the file, so container overhead and an MP3's cover art read as bitrate — the
    /// protocol's formula as written; a stated rate always wins over it.
    /// </summary>
    public static long? EstimatedBitrate(long sizeBytes, long? durationMs, long audioBitrate)
    {
        if (Known(durationMs) is not { } duration)
        {
            return null;
        }
        // Saturating, as the original does, so that nonsense in cannot throw.
        var whole = SaturatingMultiply(sizeBytes, 8 * 1000) / duration;
        var video = whole - audioBitrate;
        return audioBitrate <= whole && video > 0 ? video : null;
    }

    /// <summary>
    /// <c>V</c>: the stated video bitrate, or else the estimate. The estimate takes the AUDIO's rate off the whole: 0
    /// with no audio track, but a track whose rate is not stated leaves an unknown in the formula — and then <c>V</c>
    /// "cannot be estimated either". (Which sends the file past rule A to a transcode; rule D keeps the original if
    /// that came out bigger.)
    /// </summary>
    public static long? SourceVideoBitrate(VideoSource source)
    {
        if (Known(source.VideoBitrate) is { } stated)
        {
            return stated;
        }
        long audio;
        if (source.AudioCodec is null)
        {
            audio = 0;
        }
        else if (Known(source.AudioBitrate) is { } known)
        {
            audio = known;
        }
        else
        {
            return null;
        }
        return EstimatedBitrate(source.SizeBytes, source.DurationMs, audio);
    }

    /// <summary>
    /// Rule 1: the target size for a DISPLAYED <paramref name="width"/> × <paramref name="height"/>. The short side
    /// becomes <c>min(720, short)</c> — never upscaled — and the long side follows, rounded half up in integers:
    /// <c>(2 × long × ts + short) ÷ (2 × short)</c>. Each side then DROPS to even (never rounds up, so it cannot exceed
    /// the source), and the target keeps the source's orientation. (0, 0) for a source with no size.
    /// </summary>
    public static (long Width, long Height) TargetSize(long width, long height)
    {
        var shortSide = Math.Min(width, height);
        var longSide = Math.Max(width, height);
        if (shortSide <= 0)
        {
            return (0, 0);
        }
        var targetShort = Math.Min(shortSide, MaxShortSide);
        var targetLong = ((2 * longSide * targetShort) + shortSide) / (2 * shortSide);
        targetShort -= targetShort % 2;
        targetLong -= targetLong % 2;
        return width >= height ? (targetLong, targetShort) : (targetShort, targetLong);
    }

    /// <summary>
    /// Rule 2: 30 when the source's rate is above <see cref="FrameRateTolerance"/> or unknown, and the source's own rate
    /// otherwise — never raised, and never "tidied": 29.97 stays 29.97.
    /// </summary>
    public static double TargetFrameRate(double? frameRate) =>
        KnownFrameRate(frameRate) is { } rate && rate <= FrameRateTolerance ? rate : MaxFrameRate;

    /// <summary>
    /// Rule 3 before the source cap: <c>2 000 000 × (w × h ÷ 921 600) × (f ÷ 30)</c>, clamped to [250 000, 2 000 000],
    /// rounded to the nearest 1 000, HALF UP.
    /// </summary>
    /// <remarks>
    /// <para>
    /// <b>THE EVALUATION ORDER IS PART OF THE RULE.</b> "The rounding absorbs the last bit" is true everywhere except
    /// on an exact half-thousand, and those exist at real sizes: 960×540 at 25 fps is 937 500, 404×288 at 30 is
    /// 252 500. So the rate in THOUSANDS is <c>w × h × f ÷ 13 824</c> (<c>2 000 000 ÷ (921 600 × 30)</c> is exactly
    /// <c>1 ÷ 13 824</c>): one <see cref="double"/> product of the integer pixel count and the rate, then one division.
    /// For a whole frame rate the product is exact and the division correctly rounded, so a true half comes out .5.
    /// </para>
    /// <para>
    /// <b>HALF UP, NOT THIS PLATFORM'S DEFAULT.</b> <see cref="Math.Round(double)"/> rounds half to EVEN and would make
    /// 404×288 252 000; <see cref="MidpointRounding.AwayFromZero"/> is the original's <c>f64::round</c>.
    /// </para>
    /// </remarks>
    public static long ProfileVideoBitrate(long width, long height, double frameRate)
    {
        var pixels = width * height;
        var thousands = (double)pixels * frameRate / 13_824.0;
        // Clamping before rounding, as written, is the same as after: both bounds are whole thousands.
        var clamped = Math.Clamp(thousands, (double)(MinVideoBitrate / 1000), (double)(MaxVideoBitrate / 1000));
        var rounded = Math.Round(clamped, MidpointRounding.AwayFromZero);
        // The original's `as u64` turns a NaN into 0; say so rather than leave it to the conversion.
        return double.IsNaN(rounded) ? 0 : (long)rounded * 1000;
    }

    /// <summary>
    /// Rule 3 whole: the profile's bitrate, and then — if <c>V</c> is known — no higher than <c>V</c> (rule B). The cap
    /// comes AFTER the rounding, so a capped target is the source's exact rate, not a whole thousand.
    /// </summary>
    public static long TargetVideoBitrate(long width, long height, double frameRate, long? sourceBitrate)
    {
        var profile = ProfileVideoBitrate(width, height, frameRate);
        return Known(sourceBitrate) is { } source ? Math.Min(profile, source) : profile;
    }

    /// <summary>
    /// 128 000 for stereo, 64 000 for mono — never above the source's, when that is known. Only exactly one channel is
    /// mono: 5.1 takes the stereo rate, and so does a count nobody could read (guessing mono would halve a stereo track).
    /// </summary>
    public static long TargetAudioBitrate(long? channels, long? sourceBitrate)
    {
        var profile = channels == 1 ? MonoAudioBitrate : StereoAudioBitrate;
        return Known(sourceBitrate) is { } source ? Math.Min(profile, source) : profile;
    }

    /// <summary>
    /// Rules 1–3 for a source: the size, frame rate and two bitrates it would be transcoded to; null when it has no size.
    /// The audio's cap is the rate the container STATES for it — the protocol gives no way to estimate an audio track's
    /// rate inside a video.
    /// </summary>
    public static VideoTarget? VideoTargetFor(VideoSource source)
    {
        if (source.Width <= 0 || source.Height <= 0)
        {
            return null;
        }
        var (width, height) = TargetSize(source.Width, source.Height);
        var frameRate = TargetFrameRate(source.FrameRate);
        var videoBitrate = TargetVideoBitrate(width, height, frameRate, SourceVideoBitrate(source));
        long? audioBitrate = source.AudioCodec is null ? null : TargetAudioBitrate(source.AudioChannels, source.AudioBitrate);
        return new VideoTarget(width, height, frameRate, videoBitrate, audioBitrate);
    }

    /// <summary>
    /// Rule A — leave it alone. Every condition exactly as the protocol lists it: <c>video/mp4</c>; H.264; AAC or no
    /// audio; the short side at most 720; <c>F</c> known and at most 30.5; <c>V</c> known and at most 1.25 × step 3's
    /// bitrate for it — <c>4 × V ≤ 5 × target</c>, in integers. (<c>video/quicktime</c> is never within: Firefox will
    /// not play the container.) Not asked, because the list does not: the audio's bitrate, even sides, or where the
    /// <c>moov</c> box is — a kept file is the original, whatever those are.
    /// </summary>
    public static bool WithinProfile(VideoSource source)
    {
        if (VideoTargetFor(source) is not { } target
            || KnownFrameRate(source.FrameRate) is not { } rate
            || SourceVideoBitrate(source) is not { } bitrate)
        {
            return false;
        }
        return source.Container == "video/mp4"
            && source.VideoCodec == "h264"
            && source.AudioCodec is null or "aac"
            && Math.Min(source.Width, source.Height) <= MaxShortSide
            && rate <= FrameRateTolerance
            && SaturatingMultiply(bitrate, 4) <= SaturatingMultiply(target.VideoBitrate, 5);
    }

    /// <summary>
    /// Rules A, 5 and C for a video, in that order: kept when within the profile (a one-pixel-wide clip can be — it is
    /// still the original that goes); otherwise transcoded; and with no size to transcode to, the fallback.
    /// </summary>
    public static VideoPlan PlanVideo(VideoSource source)
    {
        if (VideoTargetFor(source) is not { } target)
        {
            return VideoPlan.Fallback;
        }
        if (WithinProfile(source))
        {
            return VideoPlan.Keep;
        }
        if (target.Width == 0 || target.Height == 0)
        {
            return VideoPlan.Fallback;
        }
        return VideoPlan.Transcode(target);
    }

    /// <summary>A sound file's bitrate: as stated, or estimated from size and duration with nothing taken off.</summary>
    public static long? SourceAudioBitrate(AudioSource source) =>
        Known(source.Bitrate) ?? EstimatedBitrate(source.SizeBytes, source.DurationMs, 0);

    /// <summary>
    /// The audio rules, in order: lossless (<c>pcm</c>, <c>flac</c>, <c>alac</c>) is re-encoded; anything in an Ogg
    /// container is re-encoded (a platform that cannot decode it fails the transcode, and rule C sends it as before);
    /// MP3 or AAC above 192 000 bit/s is re-encoded, and at or below it — or at a rate nobody can read or estimate —
    /// is untouched. Anything else is untouched, because no rule says to re-encode it.
    /// </summary>
    public static AudioPlan PlanAudio(AudioSource source)
    {
        var bitrate = SourceAudioBitrate(source);
        var transcode = source.Codec switch
        {
            "pcm" or "flac" or "alac" => true,
            _ when source.Container == "audio/ogg" => true,
            "mp3" or "aac" => bitrate is > MaxKeptLossyAudioBitrate,
            _ => false,
        };
        return transcode ? AudioPlan.Transcode(TargetAudioBitrate(source.Channels, bitrate)) : AudioPlan.Keep;
    }

    // --- when it goes wrong, or does not help ------------------------------------------------------------------------

    /// <summary>
    /// Whether a file could be uploaded as <paramref name="kind"/> (<c>"video"</c> or <c>"audio"</c>) just as it is: an
    /// accepted type, honest bytes (<see cref="MediaPrep.MatchesMagic"/>), and within the ceiling — at most it, because
    /// the server refuses only a body LARGER than <c>max_attachment_bytes</c>. Any other kind is not these rules' business.
    /// </summary>
    public static bool Sendable(string kind, string container, bool honest, long sizeBytes, long ceilingBytes)
    {
        var accepted = kind switch
        {
            "video" => AcceptedVideo.Contains(container),
            "audio" => AcceptedAudio.Contains(container),
            _ => false,
        };
        return accepted && honest && sizeBytes <= ceilingBytes;
    }

    /// <summary>
    /// Rule C — a failure sends what would have been sent without the section: a sendable source goes untouched, and
    /// anything else takes the path it always took. Preparing media may never turn a send that worked into one that
    /// does not.
    /// </summary>
    public static OnFailure AfterFailure(bool sourceSendable) =>
        sourceSendable ? OnFailure.Original : OnFailure.TodaysPath;

    /// <summary>
    /// Rule D — a result BIGGER than its source is thrown away and the source sent, provided the source is itself
    /// sendable; otherwise the result is the only thing that can be sent. Equal is not bigger: the result is in the
    /// profile and the source may not be.
    /// </summary>
    public static Upload KeepSmaller(long sourceBytes, bool sourceSendable, long resultBytes) =>
        resultBytes > sourceBytes && sourceSendable ? Upload.Source : Upload.Result;

    /// <summary>The original's <c>saturating_mul</c>, for the non-negative numbers it is used on.</summary>
    private static long SaturatingMultiply(long value, long by)
    {
        if (value <= 0 || by <= 0)
        {
            return value * by;
        }
        return value > long.MaxValue / by ? long.MaxValue : value * by;
    }
}
