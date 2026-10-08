using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>Which of the endpoint's two forms a recording's "Show text" uses (docs/protocol.md, "Transcripts on request").</summary>
public enum TranscriptForm
{
    /// <summary>Neither: nothing is offered.</summary>
    None,

    /// <summary>No body — the server sends its own stored copy, and keeps the answer for whoever asks next.</summary>
    Stored,

    /// <summary>A multipart <c>audio</c> part this device made from the file it holds; the answer is never kept there.</summary>
    Supplied,
}

/// <summary>How one way of making the sound track goes about it.</summary>
public enum SoundWay
{
    /// <summary>The file's own AAC track, copied into an M4A untouched: nothing is decoded.</summary>
    Passthrough,

    /// <summary>The sound decoded and encoded again as mono AAC, at <see cref="SoundAttempt.Aac"/>'s numbers.</summary>
    Reencode,
}

/// <summary>One way of asking Media Foundation for the sound track.</summary>
/// <param name="Aac">The encode's numbers; null for <see cref="SoundWay.Passthrough"/>, which keeps the track's own.</param>
public sealed record SoundAttempt(SoundWay Way, AacEncoding? Aac = null)
{
    public static readonly SoundAttempt Passthrough = new(SoundWay.Passthrough);
}

/// <summary>The sound the device made for a transcript, or why it has none.</summary>
/// <param name="Bytes">An M4A of AAC, within the server's ceiling — exactly what the multipart part carries.</param>
/// <param name="Error">
/// Why there is none: <see cref="TranscriptSound.TooLong"/> when what it made, or would make, is over the ceiling;
/// <see cref="TranscriptSound.Unreadable"/> when this device could not take the sound out at all; or whatever the
/// download of the file failed with (a lost connection is still "try again").
/// </param>
public sealed record SuppliedSound(byte[]? Bytes, ApiError? Error)
{
    public static SuppliedSound Made(byte[] bytes) => new(bytes, null);

    public static SuppliedSound Failed(ApiError error) => new(null, error);
}

/// <summary>Where a recording's supplied sound comes from: the ceiling it must fit, and the device's way of making it.</summary>
/// <param name="DurationMs">
/// The attachment's stated length, when it states one: a length that cannot fit <paramref name="MaxBytes"/> even at
/// 64 kbit/s is <see cref="TranscriptSound.TooLong"/> before <paramref name="Make"/> downloads anything.
/// </param>
public sealed record SoundSource(long MaxBytes, Func<CancellationToken, Task<SuppliedSound>> Make, long? DurationMs = null);

/// <summary>
/// The sound track a device sends with a transcript request when the server cannot send its own copy — a video, an Ogg
/// file, an audio file over <c>transcribe_max_bytes</c> (docs/protocol.md, "Transcripts on request", the multipart
/// form). The decisions only; <c>MediaPreparing.SoundTrackAsync</c> runs them through the #74 transcoder.
/// </summary>
/// <remarks>
/// <para>
/// <b>PASS THROUGH WHAT IS ALREADY AAC; ENCODE THE REST AT 64 kbit/s MONO.</b> A phone's video carries AAC, and copying
/// that track into an M4A costs nothing and loses nothing. Anything else — Opus or Vorbis in an Ogg, MP3, PCM, a track
/// whose copy would be over the ceiling — is encoded as a voice note is (<see cref="MediaEncoding.VoiceNoteAttempts"/>'s
/// numbers, mono), which holds about fifty minutes in 25 MiB. No picture is ever asked for, so none is decoded.
/// </para>
/// <para>
/// <b>NEVER HIDDEN FOR A REASON OF THE SOUND.</b> Whether a file can be made into sound is known only once it is here
/// and read, and downloading a video to decide whether to draw a button is the wrong trade; and a hidden action cannot
/// say why it is missing. So every voice note, audio file and video the RULES allow is offered, and what the device then
/// cannot do is said under it, final, in the catalogue's own sentences — the ones iOS, Android and the web draw: a
/// stated length whose 64 kbit/s sound cannot fit (<see cref="CanFit"/>, told at the press with nothing downloaded) or
/// a result still over the ceiling is "This recording is too long to turn into text." (<see cref="TooLong"/>); no track
/// or a codec this machine lacks is "Couldn't read the sound in this file." (<see cref="Unreadable"/>). Neither is "Not
/// available for this message.", which reads as a refusal.
/// </para>
/// </remarks>
public static class TranscriptSound
{
    /// <summary>The encode: AAC-LC, mono, 64 kbit/s — a voice note's numbers.</summary>
    public const long Bitrate = MediaPlan.VoiceNoteBitrate;

    /// <summary>The part's type: AAC in MPEG-4, which the server checks by its <c>ftyp</c> box.</summary>
    public const string Mime = "audio/mp4";

    /// <summary>The part's name, as the server reads it.</summary>
    public const string PartName = "audio";

    /// <summary>The part's file name — what an M4A is called; the server reads only the bytes.</summary>
    public const string FileName = "audio.m4a";

    /// <summary>What an MPEG-4 file costs beyond its sound, as a share of it (its sample tables) and a fixed head.</summary>
    private const long OverheadBytes = 16 * 1024;

    /// <summary>
    /// Nothing to ask with at all — no form for this recording: said as the server's <c>not_transcribable</c> is, final.
    /// </summary>
    public static ApiError Unavailable { get; } =
        new(ErrorCodes.NotTranscribable, "this device could not make the recording's sound");

    /// <summary>The code of <see cref="TooLong"/>: this client's own, never the server's, and never on the wire.</summary>
    public const string TooLongCode = "device_sound_too_long";

    /// <summary>The code of <see cref="Unreadable"/>: this client's own, never the server's, and never on the wire.</summary>
    public const string UnreadableCode = "device_sound_unreadable";

    /// <summary>
    /// The sound is longer than the ceiling holds even at 64 kbit/s — known from the stated length, or measured once made.
    /// Final: the same file makes the same sound. A 4xx status, so it never reads as a transport failure.
    /// </summary>
    public static ApiError TooLong { get; } = new(TooLongCode, "the recording's sound is over the ceiling", 400);

    /// <summary>
    /// This device could not take the sound out of the file: no sound track, a codec this machine lacks, or what came out
    /// was not AAC in MPEG-4. Final on this device.
    /// </summary>
    public static ApiError Unreadable { get; } = new(UnreadableCode, "this device could not read the recording's sound", 400);

    /// <summary>
    /// About how big a track of this rate and length comes out as an M4A: the sound, plus a fiftieth for the sample
    /// tables, plus a fixed head. An over-estimate on purpose — it only ever decides what is NOT tried.
    /// </summary>
    public static long EstimatedBytes(long bitrate, long durationMs)
    {
        if (bitrate <= 0 || durationMs <= 0)
        {
            return 0;
        }
        // In floating point, so nonsense in saturates instead of overflowing; an estimate needs no more.
        var sound = (double)bitrate * durationMs / 8000.0;
        var total = Math.Ceiling(sound + (sound / 50.0) + OverheadBytes);
        return total >= long.MaxValue ? long.MaxValue : (long)total;
    }

    /// <summary>
    /// Whether a recording of this length could fit the ceiling at all once encoded at <see cref="Bitrate"/>. When it
    /// cannot, the press says <see cref="TooLong"/> without downloading anything. A length nobody stated may fit: the
    /// result is measured before it is sent.
    /// </summary>
    public static bool CanFit(long? durationMs, long maxBytes) =>
        durationMs is not > 0 || EstimatedBytes(Bitrate, durationMs.Value) <= maxBytes;

    /// <summary>Whether made sound may go: something, and no more than the ceiling.</summary>
    public static bool Fits(long bytes, long maxBytes) => bytes > 0 && bytes <= maxBytes;

    /// <summary>
    /// The ways to make the sound, in order: the track itself when it is AAC and its copy can fit, then the
    /// voice-note encodes that can fit. EMPTY when there is no sound track — nothing to send, and "not available".
    /// </summary>
    /// <param name="codec">The track's codec as <c>MediaPreparing.AudioCodec</c> names it; null for no track at all.</param>
    /// <param name="trackBitrate">The track's own rate, when anything states it.</param>
    /// <param name="sampleRate">The track's sample rate, when anything states it — the encode keeps 48 kHz as 48.</param>
    /// <param name="durationMs">How long it runs, when known.</param>
    public static IReadOnlyList<SoundAttempt> Attempts(
        string? codec, long? trackBitrate, long? sampleRate, long? durationMs, long maxBytes)
    {
        if (codec is null)
        {
            return [];
        }
        List<SoundAttempt> attempts = [];
        if (codec == "aac" && Possible(trackBitrate, durationMs, maxBytes))
        {
            attempts.Add(SoundAttempt.Passthrough);
        }
        foreach (var aac in MediaEncoding.AudioAttempts(Bitrate, 1, sampleRate, null))
        {
            if (Possible(aac.Bitrate, durationMs, maxBytes))
            {
                attempts.Add(new SoundAttempt(SoundWay.Reencode, aac));
            }
        }
        return attempts;
    }

    /// <summary>
    /// Whether what came out may go: AAC, no pictures, a re-encode at the rate it was asked for, and within the ceiling.
    /// A copy that came out some other way, or too big, leaves the encode to try; an encode that is not AAC, or that
    /// carries a picture, will not be put right by asking again.
    /// </summary>
    /// <param name="codec">What the result's track reads as; null when it has none.</param>
    /// <param name="bitrate">The result's stated rate, when it states one.</param>
    /// <param name="bytes">How big the result is.</param>
    /// <param name="hasVideo">Whether the result carries a picture track, which it never may.</param>
    public static AttemptEnd Judge(SoundAttempt attempt, string? codec, long? bitrate, long bytes, bool hasVideo, long maxBytes)
    {
        ArgumentNullException.ThrowIfNull(attempt);
        if (hasVideo)
        {
            return AttemptEnd.Wrong;
        }
        if (codec != "aac")
        {
            return attempt.Way == SoundWay.Passthrough ? AttemptEnd.Refused : AttemptEnd.Wrong;
        }
        if (attempt.Aac is { } asked && !MediaEncoding.AudioRateCameOut(asked.Bitrate, bitrate))
        {
            return AttemptEnd.Refused;
        }
        return Fits(bytes, maxBytes) ? AttemptEnd.Taken : AttemptEnd.Refused;
    }

    /// <summary>
    /// The extension the held file is written out with for Media Foundation, which picks its reader by it: by the stored
    /// type first, the name's own when the type says nothing, and <c>bin</c> — which it will try to sniff — otherwise.
    /// Never anything from the name but letters and digits: it becomes part of a path.
    /// </summary>
    public static string TempExtension(string? mime, string? name)
    {
        var byType = MediaPrep.Essence(mime ?? string.Empty) switch
        {
            "video/mp4" or "video/x-m4v" => "mp4",
            "video/quicktime" => "mov",
            "audio/mp4" or "audio/m4a" or "audio/x-m4a" => "m4a",
            "audio/aac" or "audio/x-aac" => "aac",
            "audio/mpeg" or "audio/mp3" => "mp3",
            "audio/wav" or "audio/x-wav" or "audio/wave" => "wav",
            "audio/ogg" or "application/ogg" => "ogg",
            "audio/webm" or "video/webm" => "webm",
            "audio/flac" => "flac",
            _ => null,
        };
        if (byType is not null)
        {
            return byType;
        }
        var extension = name is null ? string.Empty : MediaPrep.Extension(name);
        return extension.Length is > 0 and <= 5 && extension.All(char.IsAsciiLetterOrDigit) ? extension : "bin";
    }

    private static bool Possible(long? bitrate, long? durationMs, long maxBytes) =>
        bitrate is not > 0 || durationMs is not > 0 || EstimatedBytes(bitrate.Value, durationMs.Value) <= maxBytes;
}
