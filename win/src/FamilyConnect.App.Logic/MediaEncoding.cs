using FamilyConnect.Core;

namespace FamilyConnect.App.Logic;

/// <summary>Which H.264 profile an encode asks for — High, and Main where an encoder offers nothing else.</summary>
public enum H264Profile
{
    High,
    Main,
}

/// <summary>AAC-LC as Media Foundation's encoder is asked for it.</summary>
public readonly record struct AacEncoding(uint SampleRate, uint Channels, uint Bitrate);

/// <summary>
/// One way of asking Media Foundation for the video <see cref="MediaPlan"/> decided on — the numbers that go into the
/// profile, and nothing that needs Windows to hold them.
/// </summary>
/// <param name="Width">
/// The ENCODED sides: the stored orientation, which for a phone's portrait clip is the landscape frame. The turn rides
/// along as <paramref name="Rotation"/>, as metadata, exactly as it did in the source.
/// </param>
/// <param name="Rotation">Degrees, 0, 90, 180 or 270, in the convention the source's own was read in.</param>
/// <param name="ToSdr">Whether to ask for BT.709 SDR explicitly — set only for a source that is HDR.</param>
/// <param name="KeepAudio">
/// Whether the audio asked for is the SOURCE'S OWN TRACK, handed back exactly as it was read so the transcoder passes it
/// through rather than encoding it again. <paramref name="Audio"/> then holds that track's own numbers.
/// </param>
/// <param name="KeyframeSpacing">
/// The most frames from one keyframe to the next, asked for as <c>MF_MT_MAX_KEYFRAME_SPACING</c> — or null to leave it to
/// the encoder, as every planned video does. Only a video message asks (its profile: "keyframes at most every 2 s").
/// </param>
public sealed record VideoEncoding(
    uint Width,
    uint Height,
    uint Rotation,
    uint Bitrate,
    uint FrameRateNumerator,
    uint FrameRateDenominator,
    H264Profile Profile,
    AacEncoding? Audio,
    bool ToSdr,
    bool KeepAudio = false,
    uint? KeyframeSpacing = null);

/// <summary>What is sent once a transcode has run: rule D, and what faststart could not do.</summary>
public enum Chosen
{
    /// <summary>The transcode's bytes.</summary>
    Result,

    /// <summary>The source, untouched — the result is thrown away.</summary>
    Original,

    /// <summary>Nothing: the source could not have gone, and the result is over the ceiling too.</summary>
    TooLarge,
}

/// <summary>
/// The Windows half of "Preparing media before upload" that needs no Windows: what a Media Foundation profile is
/// given for a plan (docs/protocol.md; the planner is <see cref="MediaPlan"/>, the platform calls are
/// <c>MediaPreparing</c> and <c>VoiceRecorder</c>).
/// </summary>
/// <remarks>
/// <para>
/// <b>MEDIA FOUNDATION'S AAC ENCODER TAKES FOUR BITRATES AND NO OTHERS.</b> Its documented output rates are 12 000,
/// 16 000, 20 000 and 24 000 bytes a second — 96, 128, 160 and 192 kbit/s — at 44.1 or 48 kHz, one, two or six
/// channels ("AAC Encoder", Win32 Media Foundation docs). The protocol's 128 000 stereo is one of them; its 64 000 mono
/// and every source-capped rate in between are not. So every encode asks for the planner's EXACT number first — an
/// encoder that takes it gives exactly the protocol — and only then for the nearest documented rate that still keeps
/// rule B: the largest one at or under the target, or else 96 000 when that is not above the source's own. A mono
/// track therefore comes out at 96 000 on a machine whose encoder refuses 64 000, which is 1.5× the protocol's
/// number, and still a tenth of the WAV it came from.
/// </para>
/// <para>
/// <b>WHAT THAT LEAVES OUT IS A TRACK ALREADY UNDER 96 000</b> — a messenger's re-share, a screen recording, an old
/// phone's mono. Rule B forbids the encoder's lowest rate for it, so where the exact number is refused there is nothing
/// left to ENCODE with. When that track is already AAC at or under the profile's row, the video is asked for with the
/// source's own track handed back as it was read (<see cref="VideoEncoding.KeepAudio"/>): a transcoder that is not told
/// to re-encode everything passes a stream through when it is asked for what it already is, and the track was within
/// the profile to begin with. Anything else under 96 000 — an MP3 track, a mono AAC at 80 000 whose row is 64 000 — has
/// no way through this encoder that keeps rule B, and goes as the original (rule C).
/// </para>
/// <para>
/// <b>A TRANSCODE IS CHECKED, NOT TRUSTED.</b> Nothing here can see Media Foundation run, so the window reads the
/// result back and <see cref="Matches"/>, <see cref="FrameRateCameOut"/> and <see cref="AudioRateCameOut"/> decide
/// whether it is what was asked for — and <see cref="FrameTurn"/> whether it still LOOKS like its source, which the
/// numbers alone cannot say. A result that came out sideways, at the wrong size or faster than asked is a failed
/// transcode, and rule C sends the original — a picture can only be lost by being sent wrong, never by being sent as
/// it was.
/// </para>
/// </remarks>
public static class MediaEncoding
{
    /// <summary>The average rates Media Foundation's AAC encoder documents, lowest first.</summary>
    public static IReadOnlyList<uint> EncoderAacBitrates { get; } = [96_000, 128_000, 160_000, 192_000];

    /// <summary>A voice note's sample rate: 44.1 kHz, as the Apple and Android recorders use (the protocol allows 48).</summary>
    public const uint VoiceNoteSampleRate = 44_100;

    /// <summary>The <c>colr</c>-style numbers Media Foundation carries in a video type (mfobjects.h).</summary>
    public const uint TransferFunctionPq = 15;
    public const uint TransferFunctionHlg = 16;
    public const uint PrimariesBt2020 = 9;

    // --- reading ------------------------------------------------------------------------------------------------------

    /// <summary>A count a reader handed over, or null: the first that is more than 0 (0 is what "not stated" reads as).</summary>
    public static long? FirstKnown(params long?[] values) => values.FirstOrDefault(value => value is > 0);

    /// <summary>
    /// The turn, as degrees: Media Foundation's own when it states one, the shell's otherwise. Media Foundation's is
    /// preferred because it is the transcoder's view of the same stream; either way the same reader reads the result
    /// back, so the two are compared in one convention.
    /// </summary>
    public static uint Rotation(long? mediaFoundation, long? shell) =>
        mediaFoundation is 0 or 90 or 180 or 270 ? (uint)mediaFoundation.Value
        : shell is 0 or 90 or 180 or 270 ? (uint)shell.Value
        : 0;

    /// <summary>The size as it is DISPLAYED: a quarter turn either way swaps the sides.</summary>
    public static (long Width, long Height) Displayed(long width, long height, uint rotation) =>
        rotation is 90 or 270 ? (height, width) : (width, height);

    /// <summary>
    /// The frame rate: the stream's ratio when it has one, else the shell's <c>System.Video.FrameRate</c> — frames per
    /// THOUSAND seconds, so 29 970 is 29.97 — else unknown.
    /// </summary>
    public static double? FrameRate(uint numerator, uint denominator, uint perThousandSeconds) =>
        numerator > 0 && denominator > 0 ? (double)numerator / denominator
        : perThousandSeconds > 0 ? perThousandSeconds / 1000.0
        : null;

    /// <summary>Whether a video is HDR by what its type says — PQ or HLG, or BT.2020's primaries.</summary>
    public static bool IsHdr(long? transferFunction, long? primaries) =>
        transferFunction is TransferFunctionPq or TransferFunctionHlg || primaries is PrimariesBt2020;

    /// <summary>
    /// Whether a video has sound Media Foundation did not show: the shell states a channel count, a sample rate or an
    /// audio bit rate for a file whose streams came back with no audio track. Transcoding that would send a SILENT
    /// video where 1.1 sent the sound, and the read-back could not tell — no track was asked for, none came out — so it
    /// is a source this machine cannot transcode (rule C).
    /// </summary>
    public static bool HidesAudio(bool audioTrackRead, long? shellChannels, long? shellSampleRate, long? shellBitrate) =>
        !audioTrackRead && FirstKnown(shellChannels, shellSampleRate, shellBitrate) is not null;

    /// <summary>
    /// Whether a stream's pixels are square — or say nothing, which every phone's and every screen recording's do. The
    /// protocol's <c>W × H</c> is the DISPLAYED size and only names the rotation; an anamorphic source (HDV's 1440×1080
    /// shown 16:9) is wider than its stored sides, and scaling those sides would squash it for good. It is left alone.
    /// </summary>
    public static bool SquarePixels(uint numerator, uint denominator) =>
        numerator == 0 || denominator == 0 || numerator == denominator;

    /// <summary>
    /// The type a picked sound file is judged as when the router sent it as a FILE: its declared type when that is
    /// audio — AIFF and FLAC are, and the server takes neither — and FLAC by its extension when Windows named no type.
    /// Null for anything else, which goes as the file it always went as.
    /// </summary>
    public static string? PickedAudioType(string declared, string name) =>
        declared.StartsWith("audio/", StringComparison.Ordinal) ? declared
        : MediaPrep.Extension(name) == "flac" ? "audio/flac"
        : null;

    /// <summary>A re-encoded track keeps its name with the extension it now has; a name there never was stays absent.</summary>
    public static string? M4aName(string? name)
    {
        if (name is null)
        {
            return null;
        }
        var extension = MediaPrep.Extension(name);
        var stem = extension.Length > 0 ? name[..(name.Length - extension.Length - 1)] : name;
        // Sanitised again: a name with no extension grows by four, and the limit cuts the stem, never the ".m4a".
        return MediaPrep.SanitizedName($"{stem}.m4a");
    }

    // --- the profile ---------------------------------------------------------------------------------------------------

    /// <summary>The ENCODED sides for a displayed target: a quarter turn is undone, and carried as metadata instead.</summary>
    public static (uint Width, uint Height) Encoded(long width, long height, uint rotation)
    {
        var (encodedWidth, encodedHeight) = Displayed(width, height, rotation);
        return ((uint)Math.Clamp(encodedWidth, 0, uint.MaxValue), (uint)Math.Clamp(encodedHeight, 0, uint.MaxValue));
    }

    /// <summary>
    /// The frame rate as the ratio a profile takes, NEVER above <paramref name="target"/>: the source's own ratio when the
    /// target is the source's rate (30000/1001 stays exactly that), a whole number over one, and otherwise the nearest
    /// thousandth that is not above it.
    /// </summary>
    public static (uint Numerator, uint Denominator) FrameRateRatio(double target, uint sourceNumerator, uint sourceDenominator)
    {
        if (sourceNumerator > 0 && sourceDenominator > 0 && (double)sourceNumerator / sourceDenominator == target)
        {
            return (sourceNumerator, sourceDenominator);
        }
        if (!double.IsFinite(target) || target <= 0)
        {
            return ((uint)MediaPlan.MaxFrameRate, 1);
        }
        if (target == Math.Floor(target) && target <= uint.MaxValue)
        {
            return ((uint)target, 1);
        }
        var thousandths = Math.Round(target * 1000, MidpointRounding.AwayFromZero);
        if (thousandths / 1000 > target)
        {
            thousandths -= 1;
        }
        if (thousandths < 1)
        {
            // Under a thousandth of a frame a second: one frame in however many seconds keeps it at or under.
            return (1, (uint)Math.Min(uint.MaxValue, Math.Ceiling(1 / target)));
        }
        var numerator = (uint)Math.Min(uint.MaxValue, thousandths);
        var divisor = Gcd(numerator, 1000);
        return (numerator / divisor, 1000 / divisor);
    }

    /// <summary>44.1 or 48 kHz — the only two the encoder takes: the source's own when it is one, 48 above 44.1, 44.1 otherwise.</summary>
    public static uint AacSampleRate(long? source) => source is > 44_100 ? 48_000u : 44_100u;

    /// <summary>Mono stays mono; anything else is stereo — the profile has a stereo row and a mono row, and 5.1 is folded down.</summary>
    public static uint AacChannels(long? source) => source == 1 ? 1u : 2u;

    /// <summary>
    /// The nearest rate the encoder documents for a <paramref name="target"/> it may refuse: the largest at or under
    /// it, or else the lowest when that is not above the source's own (rule B). Null when neither exists — the target
    /// then has no second chance.
    /// </summary>
    public static uint? EncoderAacBitrate(long target, long? source)
    {
        var under = EncoderAacBitrates.Where(rate => rate <= target).ToList();
        if (under.Count > 0)
        {
            return under[^1];
        }
        var lowest = EncoderAacBitrates[0];
        return source is > 0 && lowest > source ? null : lowest;
    }

    /// <summary>
    /// A picked sound file's encodes, in the order to ask for them: the planner's exact bitrate, then the encoder's
    /// nearest (<see cref="EncoderAacBitrate"/>) when that differs.
    /// </summary>
    public static IReadOnlyList<AacEncoding> AudioAttempts(long bitrate, long? channels, long? sampleRate, long? sourceBitrate)
    {
        var exact = new AacEncoding(AacSampleRate(sampleRate), AacChannels(channels), Rate(bitrate));
        List<AacEncoding> attempts = [exact];
        if (EncoderAacBitrate(bitrate, sourceBitrate) is { } nearest && nearest != exact.Bitrate)
        {
            attempts.Add(exact with { Bitrate = nearest });
        }
        return attempts;
    }

    /// <summary>
    /// A voice note's encodes: the protocol's AAC-LC mono 64 000 at 44.1 kHz, then the nearest the encoder documents.
    /// The recorder falls back to what it recorded before only when both are refused.
    /// </summary>
    public static IReadOnlyList<AacEncoding> VoiceNoteAttempts() =>
        AudioAttempts(MediaPlan.VoiceNoteBitrate, 1, VoiceNoteSampleRate, null);

    /// <summary>
    /// A video's encodes, in the order to ask for them. High profile with the planner's exact audio rate; then High
    /// with the source's own track passed through, where it is one the profile already takes
    /// (<see cref="KeepsAudio"/>); then High with the encoder's nearest audio rate, when that differs; then Main with the
    /// last of those there is — the protocol's "Main where an encoder offers nothing else". Every one carries the same
    /// size, frame rate, video bitrate and turn: only what an encoder may refuse changes.
    /// </summary>
    public static IReadOnlyList<VideoEncoding> VideoAttempts(
        VideoTarget target,
        uint rotation,
        uint sourceFrameRateNumerator,
        uint sourceFrameRateDenominator,
        long? audioChannels,
        long? audioSampleRate,
        long? sourceAudioBitrate,
        bool hdr,
        bool audioIsAac = false)
    {
        var (width, height) = Encoded(target.Width, target.Height, rotation);
        var (numerator, denominator) = FrameRateRatio(target.FrameRate, sourceFrameRateNumerator, sourceFrameRateDenominator);
        AacEncoding? exact = null;
        AacEncoding? nearest = null;
        AacEncoding? kept = null;
        if (target.AudioBitrate is { } audio)
        {
            exact = new AacEncoding(AacSampleRate(audioSampleRate), AacChannels(audioChannels), Rate(audio));
            if (EncoderAacBitrate(audio, sourceAudioBitrate) is { } rate)
            {
                nearest = exact.Value with { Bitrate = rate };
            }
            if (KeepsAudio(audioIsAac, audio, audioChannels, audioSampleRate, sourceAudioBitrate))
            {
                // The track's OWN numbers, not the encoder's: 22.05 kHz stays 22.05, because nothing is encoded.
                kept = new AacEncoding(Rate(audioSampleRate!.Value), Rate(audioChannels!.Value), Rate(audio));
            }
        }
        var first = new VideoEncoding(width, height, rotation, Rate(target.VideoBitrate), numerator, denominator, H264Profile.High, exact, hdr);
        List<VideoEncoding> attempts = [first];
        if (kept is { } own)
        {
            attempts.Add(first with { Audio = own, KeepAudio = true });
        }
        if (nearest is { } second && second != exact)
        {
            attempts.Add(first with { Audio = second });
        }
        attempts.Add(nearest is null && kept is { } passed
            ? first with { Profile = H264Profile.Main, Audio = passed, KeepAudio = true }
            : first with { Profile = H264Profile.Main, Audio = nearest ?? exact });
        return attempts;
    }

    /// <summary>
    /// Whether a video's own audio track may be passed through instead of encoded: it is AAC, mono or stereo, and its
    /// stated bit rate is what the planner asked for — which it is exactly when that rate was already at or under the
    /// profile's row, so the cap was the source itself (rule B). Encoding it again at its own rate could only cost it a
    /// generation; above the row it has to come down, and is never passed through.
    /// </summary>
    public static bool KeepsAudio(bool audioIsAac, long target, long? channels, long? sampleRate, long? sourceBitrate) =>
        audioIsAac && channels is 1 or 2 && sampleRate is > 0 && sourceBitrate == target;

    /// <summary>
    /// Whether what Media Foundation wrote is what was asked for: the encoded sides exactly, and the SAME turn — read
    /// back by the same reader that read the source's. A result that differs came out stretched, sideways or upside
    /// down, and is a failed transcode (rule C), never an upload.
    /// </summary>
    public static bool Matches(VideoEncoding asked, uint width, uint height, uint rotation) =>
        asked.Width == width && asked.Height == height && asked.Rotation == rotation;

    /// <summary>
    /// Whether a result's frame rate is what was asked for: not above it by more than the planner's own tolerance. A
    /// 60 fps source that came out at 60 was given a bitrate worked out for 30 — half the bits a frame, and outside the
    /// profile's "at most 30" — so it is a failed transcode like a wrong size is. A rate BELOW what was asked is not:
    /// a transcoder cannot invent frames the source never had. One nobody can read is not judged.
    /// </summary>
    public static bool FrameRateCameOut(VideoEncoding asked, double? rate) =>
        rate is not { } known
        || asked.FrameRateDenominator == 0
        || known <= ((double)asked.FrameRateNumerator / asked.FrameRateDenominator) + (MediaPlan.FrameRateTolerance - MediaPlan.MaxFrameRate);

    /// <summary>
    /// Whether a result's audio is at the rate asked for, within a tenth. An encoder that takes 64 000 and writes its
    /// own 96 000 has raised a bitrate over what rule B allowed without refusing anything; that result is thrown away
    /// and the next way of asking — which names the encoder's rate only where rule B permits it — is tried. A rate the
    /// result does not state is not judged.
    /// </summary>
    public static bool AudioRateCameOut(uint asked, long? stated) =>
        stated is not > 0 || stated.Value * 10 <= asked * 11L;

    /// <summary>
    /// Whether a recording or a re-encode came out as the AAC it was asked for, in the channels asked for. A count the
    /// result does not state is not judged.
    /// </summary>
    public static bool AacCameOut(AacEncoding asked, string codec, long? channels) =>
        codec == "aac" && (channels is not > 0 || channels == asked.Channels);

    /// <summary>
    /// What goes once a transcode has come out as asked — rule D, and what faststart could not do. A result bigger than
    /// a sendable source is thrown away; a VIDEO whose index could not be moved to the front does not go where the
    /// source can go instead; and a result over the ceiling is refused, as the source would have been. Otherwise the
    /// result is sent — the only thing that can be.
    /// </summary>
    /// <remarks>
    /// <para>
    /// <b>A SENDABLE SOURCE IS NEVER REFUSED HERE</b>, whatever the result turned out to be: preparing media may not
    /// turn a send that worked in 1.1 into one that does not (rule C).
    /// </para>
    /// <para>
    /// <b>THE INDEX MATTERS FOR A VIDEO ONLY</b> (<paramref name="needsMoovFirst"/>). The protocol asks for
    /// <c>moov</c> before <c>mdat</c> in the video container's row and not in "Audio alone", and an M4A a tenth the size
    /// of its WAV is worth more with its index at the end than the WAV is — or than an Ogg original, which does not
    /// play on iOS or macOS at all.
    /// </para>
    /// </remarks>
    public static Chosen Choose(
        long sourceBytes, bool sourceSendable, long resultBytes, bool moovFirst, bool needsMoovFirst, long ceiling) =>
        MediaPlan.KeepSmaller(sourceBytes, sourceSendable, resultBytes) == Upload.Source ? Chosen.Original
        : needsMoovFirst && !moovFirst && sourceSendable ? Chosen.Original
        : resultBytes > ceiling ? Chosen.TooLarge
        : Chosen.Result;

    /// <summary>
    /// How long a transcode may take before it counts as failed: two minutes plus three times the clip — a hardware
    /// encoder is many times faster than that, and a stuck one must not leave the composer "Preparing…" for good.
    /// Ten minutes when the length is unknown. It is ONE ceiling for every way of asking together
    /// (<see cref="TranscodeAttempts"/>), not one each.
    /// </summary>
    public static TimeSpan TranscodeCeiling(long? durationMs) =>
        durationMs is > 0 ? TimeSpan.FromMinutes(2) + TimeSpan.FromMilliseconds(3.0 * durationMs.Value) : TimeSpan.FromMinutes(10);

    private static uint Rate(long bitrate) => (uint)Math.Clamp(bitrate, 0, uint.MaxValue);

    private static uint Gcd(uint a, uint b)
    {
        while (b != 0)
        {
            (a, b) = (b, a % b);
        }
        return Math.Max(a, 1);
    }
}
