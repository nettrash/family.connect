using FamilyConnect.Core;

namespace FamilyConnect.App.Logic;

/// <summary>
/// A voice note recorded here (docs/protocol.md, "Audio" and "A browser is a client too"): MP4 with AAC in it — never
/// WebM, which the server refuses — five minutes at most, and staged when it stops, so a caption can be added and a
/// recording made by accident can still be thrown away.
/// </summary>
public static class VoiceNotes
{
    /// <summary>The longest a voice note may run; the recording stops itself there.</summary>
    public static readonly TimeSpan Longest = TimeSpan.FromMinutes(5);

    /// <summary>A recording this small is nothing — ios <c>AudioRecorder</c>'s 1024 bytes or less.</summary>
    public const int NothingAtOrBelowBytes = 1024;

    /// <summary>What a voice note is uploaded as: the container the server checks, and every family phone plays.</summary>
    public const string Mime = "audio/mp4";

    /// <summary>Whether a recording has run as long as a voice note may.</summary>
    public static bool IsDone(TimeSpan elapsed) => elapsed >= Longest;

    /// <summary>
    /// The recording as a staged attachment — its duration its identity, and no name — or null when it is too short to be
    /// anything, which the composer says.
    /// </summary>
    public static StagedMedia? Staged(ReadOnlyMemory<byte> bytes, TimeSpan elapsed) =>
        bytes.Length <= NothingAtOrBelowBytes
            ? null
            : new StagedMedia("audio", Mime, bytes, DurationMs: (int)Math.Round(Math.Min(elapsed.TotalMilliseconds, Longest.TotalMilliseconds), MidpointRounding.AwayFromZero));

    /// <summary>"Recording 0:12", under the composer while it runs.</summary>
    public static string RecordingLine(TimeSpan elapsed, IStringCatalog say) =>
        say.Format("Recording %@", MediaText.TimeLabel(elapsed.TotalSeconds));

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
