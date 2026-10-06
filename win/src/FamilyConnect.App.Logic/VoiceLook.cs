using System.Buffers.Binary;
using System.Globalization;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>
/// How a VOICE MESSAGE is drawn on this client — the approved design of 2026-10-05 (the "Voice and Video Messages" mockup):
/// a round accent play button, the waveform the sender measured (docs/protocol.md, "A voice note's waveform"), the played
/// bars in the accent as it plays, the time in tabular digits, a speed chip while it plays, and a dot until this device has
/// played someone else's. A drawing rule, not a wire one: the sizes, which bars, what is written and what a screen reader
/// hears.
/// </summary>
/// <remarks>
/// <para>
/// <b>THE WAVEFORM IS THE SENDER'S, OR A NEUTRAL ONE.</b> 48 levels from the attachment, reduced to the bars that fit by the
/// shared rule (<see cref="Waveform.Bars"/>); with none, or one that does not parse, every bar is the placeholder's level —
/// never a shape invented here, which would pass for the recording's own.
/// </para>
/// <para>
/// <b>PLAYED BARS ARE ⌊position · bars / duration⌋</b>, the shared rule, so a note half played shows half its bars lit on
/// every client.
/// </para>
/// </remarks>
public static class VoiceLook
{
    /// <summary>
    /// The bubble's width, in effective pixels: the mockup's <c>min(300, 100%)</c> at the width that fits every surface this
    /// client draws one on — the thread panel leaves a reply 295 beside its 72-wide indent, so 280, never clipped.
    /// </summary>
    public const double Width = 280;

    /// <summary>The round play button's disc (the mockup's 40), in a target of <see cref="ComposerButton.MinTargetWindowsEpx"/>.</summary>
    public const double PlayDisc = 40;

    /// <summary>The waveform's height in the bubble.</summary>
    public const double WaveHeight = 28;

    /// <summary>The mini waveform's height in a review or not-sent chip.</summary>
    public const double ChipWaveHeight = 22;

    /// <summary>A bar's width and the gap after it.</summary>
    public const double BarWidth = 3;

    /// <summary>The gap between two bars.</summary>
    public const double BarGap = 2;

    /// <summary>The unplayed dot.</summary>
    public const double Dot = 7;

    /// <summary>
    /// The waveform's width in the bubble: what is left of <see cref="Width"/> after its padding (8 and 12), the play disc and
    /// the gap beside it (10).
    /// </summary>
    public const double WaveWidth = Width - 8 - 12 - PlayDisc - 10;

    /// <summary>How many bars of <see cref="BarWidth"/> with <see cref="BarGap"/> between fit in <paramref name="width"/>.</summary>
    public static int BarCount(double width) =>
        double.IsFinite(width) && width >= BarWidth ? (int)Math.Floor((width + BarGap) / (BarWidth + BarGap)) : 0;

    /// <summary>The bars to draw: the attachment's waveform — or the placeholder — reduced to <paramref name="count"/>.</summary>
    public static byte[] Bars(string? waveform, int count) =>
        Waveform.Bars(Waveform.LevelsOrPlaceholder(waveform), Math.Max(0, count));

    /// <summary>One bar's height in a waveform <paramref name="height"/> tall: <c>(2 + level) / 17</c> of it.</summary>
    public static double BarHeight(byte level, double height) => Waveform.BarFraction(level) * Math.Max(0, height);

    /// <summary>
    /// How many bars are lit at <paramref name="positionSeconds"/> of <paramref name="totalSeconds"/> — the shared rule on whole
    /// milliseconds; none for a length nobody knows.
    /// </summary>
    public static int PlayedBars(double positionSeconds, double totalSeconds, int bars)
    {
        if (!double.IsFinite(positionSeconds) || !double.IsFinite(totalSeconds) || totalSeconds <= 0)
        {
            return 0;
        }
        var at = (ulong)Math.Round(Math.Max(0, positionSeconds) * 1000, MidpointRounding.AwayFromZero);
        var total = (ulong)Math.Round(totalSeconds * 1000, MidpointRounding.AwayFromZero);
        return Waveform.PlayedBars(at, total, bars);
    }

    /// <summary>
    /// The time under the waveform: where it is while it is the one playing (or paused part way), its whole length at rest —
    /// the mockup's "0:42" that counts up as it plays.
    /// </summary>
    public static string Time(bool active, double positionSeconds, double totalSeconds) =>
        MediaText.TimeLabel(active ? Math.Clamp(positionSeconds, 0, Math.Max(0, totalSeconds)) : totalSeconds);

    /// <summary>The dot: only on SOMEONE ELSE'S voice message that this device has not played — never on the reader's own.</summary>
    public static bool ShowsDot(bool mine, bool played) => !mine && !played;

    /// <summary>What a screen reader hears for the bubble: "Voice message, 0:42".</summary>
    public static string Name(AttachmentDto audio, IStringCatalog say) =>
        say.Format("Voice message, %@", MediaText.TimeLabel(VoiceNotes.TotalSeconds(audio.DurationMs)));

    /// <summary>Its status: "Not played" or "Played" on someone else's, and nothing on the reader's own.</summary>
    public static string? Status(bool mine, bool played, IStringCatalog say) =>
        mine ? null : played ? say.Get("Played") : say.Get("Not played");

    /// <summary>The speed chip shows while this bubble is the one playing — or paused part way through.</summary>
    public static bool ShowsSpeed(bool active) => active;

    /// <summary>
    /// How far a control's target reaches past what is drawn of it, on each side: the negative margin that makes a small
    /// visual — the speed chip, a chip's ✕, its Send — a <see cref="ComposerButton.MinTargetWindowsEpx"/> target (S1.1)
    /// without making the row it sits in any taller. Nothing for a visual that is already that large.
    /// </summary>
    public static double Reach(double visual) =>
        double.IsFinite(visual) ? -Math.Max(0, ComposerButton.MinTargetWindowsEpx - Math.Max(0, visual)) / 2 : 0;

    /// <summary>
    /// Where Play starts a bubble that is not playing: where its waveform was SEEKED to while it was idle, or the start —
    /// and the start, too, for a seek to the very end, where there would be nothing left to hear.
    /// </summary>
    public static double StartAt(double? seekedSeconds, double totalSeconds) =>
        seekedSeconds is { } at && double.IsFinite(at) && at > 0 && double.IsFinite(totalSeconds) && totalSeconds > 0
            && !VoiceNotes.ReplaysFromStart(at, totalSeconds)
            ? at
            : 0;

    /// <summary>
    /// Whether a send still on its way is ONE VOICE NOTE: no words, and exactly one staged item, a sound — drawn as the bubble it
    /// will be (dimmed, with "Sending…") rather than as a count of files.
    /// </summary>
    public static bool IsPendingVoice(string? body, IReadOnlyList<string> stagedKinds) =>
        string.IsNullOrEmpty(body) && stagedKinds is ["audio"];

    /// <summary>The not-sent chip at its widest: the mockup's <c>min(360px, 100%)</c>.</summary>
    public const double ChipWidth = 360;

    /// <summary>The fewest bars the not-sent chip's mini waveform keeps on its line before "Not sent" moves above it.</summary>
    public const int MinChipBars = 8;

    /// <summary>
    /// The not-sent chip fitted to its width (the mockup's "Not sent", ▶, mini waveform, length, Send and ✕ on one line):
    /// its waveform gets the room the other columns leave and is REDUCED to the bars that fit — never more than
    /// <paramref name="designBars"/>, and never cut off part way through the shape — and when a language's "Not sent" or a
    /// large text size leaves fewer than <see cref="MinChipBars"/>, "Not sent" moves onto its own line above the rest.
    /// </summary>
    /// <param name="rowWidth">The chip's inner width.</param>
    /// <param name="othersWidth">What ▶, the length, Send and ✕ take together.</param>
    /// <param name="labelWidth">What "Not sent" takes.</param>
    /// <param name="spacing">The gap between two columns.</param>
    /// <param name="designBars">The mockup's bars, the most it ever draws.</param>
    public static ChipFit FitNotSent(double rowWidth, double othersWidth, double labelWidth, double spacing, int designBars)
    {
        var most = Math.Max(0, designBars);
        // On one line: six columns, five gaps.
        var inline = Math.Min(most, BarCount(rowWidth - othersWidth - labelWidth - 5 * spacing));
        if (inline >= Math.Min(MinChipBars, most))
        {
            return new ChipFit(LabelAbove: false, inline);
        }
        // "Not sent" above: five columns on the line, four gaps.
        return new ChipFit(LabelAbove: true, Math.Min(most, BarCount(rowWidth - othersWidth - 4 * spacing)));
    }
}

/// <summary>How the not-sent chip is laid out at its width: whether "Not sent" is on a line of its own, and how many bars it draws.</summary>
public readonly record struct ChipFit(bool LabelAbove, int Bars);

/// <summary>
/// A voice message's playback speed: 1×, 1.5× and 2×, a press on the chip going round them, REMEMBERED ON THIS DEVICE and
/// applied through the player's own rate — the pitch is the player's business.
/// </summary>
public static class VoiceSpeed
{
    /// <summary>The speeds, in the order a press goes round them.</summary>
    public static IReadOnlyList<double> Rates { get; } = [1.0, 1.5, 2.0];

    /// <summary>The speed after <paramref name="rate"/>: 1× → 1.5× → 2× → 1×; anything else starts again at 1.5×'s turn.</summary>
    public static double Next(double rate) => rate switch
    {
        1.0 => 1.5,
        1.5 => 2.0,
        _ => 1.0,
    };

    /// <summary>"1×", "1.5×" or "2×" — catalogue keys, since a decimal separator is the language's.</summary>
    public static string Label(double rate, IStringCatalog say) => Normal(rate) switch
    {
        1.5 => say.Get("1.5×"),
        2.0 => say.Get("2×"),
        _ => say.Get("1×"),
    };

    /// <summary>What a screen reader hears for the chip: "Playback speed, 1.5×".</summary>
    public static string Name(double rate, IStringCatalog say) => say.Format("Playback speed, %@", Label(rate, say));

    /// <summary>One of <see cref="Rates"/>: anything else — an unreadable file, a hand-edited one — is 1×.</summary>
    public static double Normal(double rate) => Rates.Contains(rate) ? rate : 1.0;

    /// <summary>The device's stored choice, read: <c>"1.5"</c>, <c>"2"</c>, and 1× for anything else or nothing.</summary>
    public static double Parse(string? stored) =>
        double.TryParse(stored?.Trim(), NumberStyles.Float, CultureInfo.InvariantCulture, out var rate) ? Normal(rate) : 1.0;

    /// <summary>The choice as it is stored: the invariant number, never the language's.</summary>
    public static string Store(double rate) => Normal(rate).ToString(CultureInfo.InvariantCulture);
}

/// <summary>
/// The recording row's LIVE waveform (the approved design: "the live waveform scrolls in from the right"): the newest peaks,
/// one bar per meter tick, the oldest falling off the left.
/// </summary>
public sealed class LiveLevels(int capacity)
{
    private readonly Queue<byte> levels = new();

    /// <summary>How many bars it keeps.</summary>
    public int Capacity { get; } = Math.Max(1, capacity);

    /// <summary>How many ticks have been heard since the recording began.</summary>
    public int Heard { get; private set; }

    /// <summary>One tick's peak, in dBFS — as a level by the shared rule (<see cref="Waveform.Level"/>).</summary>
    public void Push(double peakDbfs)
    {
        levels.Enqueue(Waveform.Level(peakDbfs));
        Heard++;
        while (levels.Count > Capacity)
        {
            levels.Dequeue();
        }
    }

    /// <summary>A new recording: nothing heard yet.</summary>
    public void Clear()
    {
        levels.Clear();
        Heard = 0;
    }

    /// <summary>
    /// The last <paramref name="count"/> levels, oldest first and RIGHT-aligned — the newest is the last bar — with silence
    /// (level 0) before the first one heard, so the line grows in from the right.
    /// </summary>
    public byte[] Bars(int count)
    {
        var bars = new byte[Math.Max(0, count)];
        var held = levels.ToArray();
        var take = Math.Min(held.Length, bars.Length);
        Array.Copy(held, held.Length - take, bars, bars.Length - take, take);
        return bars;
    }
}

/// <summary>
/// A voice note's waveform measured from the RECORDING ITSELF on Windows, whose <c>MediaCapture</c> exposes no level while it
/// records: the note is decoded to PCM (a WAV, by Media Foundation, on the window's side) and its peak per
/// <see cref="BlockMs"/> is what another client's meter would have read every tick. Then the shared rule
/// (<see cref="Waveform.FromPeaks"/>) — the same 48 digits whatever measured them.
/// </summary>
/// <remarks>
/// A WAV this does not understand — not RIFF/WAVE, not 16-bit integer or 32-bit float PCM, no samples — is no waveform at
/// all: the note goes without one and every reader draws the placeholder, which the protocol allows.
/// </remarks>
public static class VoiceWaveform
{
    /// <summary>One peak per tenth of a second: twice the other clients' 200 ms meter tick, never coarser.</summary>
    public const int BlockMs = 100;

    /// <summary>A sample's magnitude, 0 to 1, in dBFS; silence is −∞, which the shared rule reads as level 0.</summary>
    public static double Dbfs(double magnitude) =>
        double.IsFinite(magnitude) && magnitude > 0 ? 20 * Math.Log10(Math.Min(magnitude, 1.0)) : double.NegativeInfinity;

    /// <summary>The wire's 48 digits for a WAV, or null when it holds nothing this can read.</summary>
    public static string? FromWav(ReadOnlySpan<byte> wav) =>
        Peaks(wav) is { Count: > 0 } peaks ? Waveform.FromPeaks(peaks) : null;

    /// <summary>The peak of every <see cref="BlockMs"/> of a WAV, in dBFS and in time order; null when it cannot be read.</summary>
    public static IReadOnlyList<double>? Peaks(ReadOnlySpan<byte> wav)
    {
        if (wav.Length < 12 || !wav[..4].SequenceEqual("RIFF"u8) || !wav.Slice(8, 4).SequenceEqual("WAVE"u8))
        {
            return null;
        }
        int? format = null, channels = null, rate = null, bits = null;
        ReadOnlySpan<byte> data = default;
        var found = false;
        for (var at = 12; at + 8 <= wav.Length;)
        {
            var id = wav.Slice(at, 4);
            var size = BinaryPrimitives.ReadUInt32LittleEndian(wav.Slice(at + 4, 4));
            var body = at + 8;
            // A chunk that claims more than is there is read to the end — Media Foundation leaves a streamed data chunk's
            // size as it found it.
            var length = (int)Math.Min(size, (uint)(wav.Length - body));
            if (id.SequenceEqual("fmt "u8) && length >= 16)
            {
                format = BinaryPrimitives.ReadUInt16LittleEndian(wav.Slice(body, 2));
                channels = BinaryPrimitives.ReadUInt16LittleEndian(wav.Slice(body + 2, 2));
                rate = (int)BinaryPrimitives.ReadUInt32LittleEndian(wav.Slice(body + 4, 4));
                bits = BinaryPrimitives.ReadUInt16LittleEndian(wav.Slice(body + 14, 2));
                if (format == 0xFFFE && length >= 26)
                {
                    // WAVE_FORMAT_EXTENSIBLE: the real format is the sub-format GUID's first two bytes.
                    format = BinaryPrimitives.ReadUInt16LittleEndian(wav.Slice(body + 24, 2));
                }
            }
            else if (id.SequenceEqual("data"u8))
            {
                data = wav.Slice(body, length);
                found = true;
                break;
            }
            at = body + length + (length & 1);
        }
        if (!found || format is not { } kind || channels is not > 0 || rate is not > 0
            || !((kind == 1 && bits == 16) || (kind == 3 && bits == 32)))
        {
            return null;
        }
        var width = bits!.Value / 8 * channels!.Value;
        var frames = data.Length / width;
        if (frames == 0)
        {
            return null;
        }
        var perBlock = Math.Max(1, (int)((long)rate!.Value * BlockMs / 1000));
        var peaks = new List<double>((frames + perBlock - 1) / perBlock);
        var loudest = 0.0;
        var inBlock = 0;
        for (var frame = 0; frame < frames; frame++)
        {
            var offset = frame * width;
            for (var channel = 0; channel < channels.Value; channel++)
            {
                var sample = kind == 1
                    ? Math.Abs((double)BinaryPrimitives.ReadInt16LittleEndian(data.Slice(offset + channel * 2, 2))) / 32768.0
                    : Math.Abs((double)BinaryPrimitives.ReadSingleLittleEndian(data.Slice(offset + channel * 4, 4)));
                if (sample > loudest)
                {
                    loudest = sample;
                }
            }
            if (++inBlock == perBlock)
            {
                peaks.Add(Dbfs(loudest));
                loudest = 0;
                inBlock = 0;
            }
        }
        if (inBlock > 0)
        {
            peaks.Add(Dbfs(loudest));
        }
        return peaks;
    }
}
