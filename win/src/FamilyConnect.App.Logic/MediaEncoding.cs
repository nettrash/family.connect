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
public sealed record VideoEncoding(
    uint Width,
    uint Height,
    uint Rotation,
    uint Bitrate,
    uint FrameRateNumerator,
    uint FrameRateDenominator,
    H264Profile Profile,
    AacEncoding? Audio,
    bool ToSdr);

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
/// <b>A TRANSCODE IS CHECKED, NOT TRUSTED.</b> Nothing here can see Media Foundation run, so the window reads the
/// result back and <see cref="Matches"/> decides whether it is what was asked for. A result that came out sideways or
/// at the wrong size is a failed transcode, and rule C sends the original — a picture can only be lost by being sent
/// wrong, never by being sent as it was.
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
    /// with the encoder's nearest audio rate, when that differs; then Main with the nearest (or the exact, when there
    /// is no nearest) — the protocol's "Main where an encoder offers nothing else". Every one carries the same size,
    /// frame rate, video bitrate and turn: only what an encoder may refuse changes.
    /// </summary>
    public static IReadOnlyList<VideoEncoding> VideoAttempts(
        VideoTarget target,
        uint rotation,
        uint sourceFrameRateNumerator,
        uint sourceFrameRateDenominator,
        long? audioChannels,
        long? audioSampleRate,
        long? sourceAudioBitrate,
        bool hdr)
    {
        var (width, height) = Encoded(target.Width, target.Height, rotation);
        var (numerator, denominator) = FrameRateRatio(target.FrameRate, sourceFrameRateNumerator, sourceFrameRateDenominator);
        AacEncoding? exact = null;
        AacEncoding? nearest = null;
        if (target.AudioBitrate is { } audio)
        {
            exact = new AacEncoding(AacSampleRate(audioSampleRate), AacChannels(audioChannels), Rate(audio));
            if (EncoderAacBitrate(audio, sourceAudioBitrate) is { } rate)
            {
                nearest = exact.Value with { Bitrate = rate };
            }
        }
        var first = new VideoEncoding(width, height, rotation, Rate(target.VideoBitrate), numerator, denominator, H264Profile.High, exact, hdr);
        List<VideoEncoding> attempts = [first];
        if (nearest is { } second && second != exact)
        {
            attempts.Add(first with { Audio = second });
        }
        attempts.Add(first with { Profile = H264Profile.Main, Audio = nearest ?? exact });
        return attempts;
    }

    /// <summary>
    /// Whether what Media Foundation wrote is what was asked for: the encoded sides exactly, and the SAME turn — read
    /// back by the same reader that read the source's. A result that differs came out stretched, sideways or upside
    /// down, and is a failed transcode (rule C), never an upload.
    /// </summary>
    public static bool Matches(VideoEncoding asked, uint width, uint height, uint rotation) =>
        asked.Width == width && asked.Height == height && asked.Rotation == rotation;

    /// <summary>
    /// How long a transcode may take before it counts as failed: two minutes plus three times the clip — a hardware
    /// encoder is many times faster than that, and a stuck one must not leave the composer "Preparing…" for good.
    /// Ten minutes when the length is unknown.
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
