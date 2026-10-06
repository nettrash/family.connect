namespace FamilyConnect.Core;

/// <summary>
/// A voice note's waveform (docs/protocol.md, "A voice note's waveform"): 48 levels of 0–15, made by the SENDER from the
/// peaks its recorder measured and sent with the upload as 48 lowercase hex digits, so a bubble draws its shape before a
/// byte of the recording is downloaded. A port of <c>fc_text::waveform</c>, held to it case for case by
/// <c>WaveformOracleTests</c> over the vectors the Rust prints — four clients drawing the same note must draw the same bars.
/// </summary>
/// <remarks>
/// <para>
/// <b>NO LOGARITHM AND NO ROUNDING MODE.</b> A level is <c>(clamp(p, −60, 0) + 60) ÷ 4</c> rounded half UP, by floor and a
/// comparison — never <see cref="Math.Round(double)"/>, whose default is half to even and would put every tie (−58, −54, …)
/// one level lower than the other three clients.
/// </para>
/// <para>
/// <b>SLICES ARE INTEGER ARITHMETIC.</b> Slice <c>i</c> of <c>n</c> peaks is <c>⌊i·n/count⌋</c> up to
/// <c>max(start + 1, ⌊(i+1)·n/count⌋)</c>, worked in 128 bits so a long recording cannot overflow it; fewer peaks than
/// slices stretch, and a slice takes its LOUDEST.
/// </para>
/// </remarks>
public static class Waveform
{
    /// <summary>How many levels the wire carries.</summary>
    public const int Levels = 48;

    /// <summary>The loudest level: one hex digit.</summary>
    public const byte MaxLevel = 15;

    /// <summary>Level 0: the silence check's own −60 dBFS.</summary>
    public const double FloorDbfs = -60.0;

    /// <summary>One level is 4 dB, so 15 of them reach full scale.</summary>
    public const double DbPerLevel = 4.0;

    /// <summary>What every bar is drawn at when there is no waveform, or it cannot be read.</summary>
    public const byte PlaceholderLevel = 4;

    /// <summary>The neutral shape: 48 bars at <see cref="PlaceholderLevel"/>.</summary>
    public static IReadOnlyList<byte> Placeholder { get; } = Enumerable.Repeat(PlaceholderLevel, Levels).ToArray();

    /// <summary>One peak, in dBFS, as a level. NaN is silence; above full scale is full scale.</summary>
    public static byte Level(double dbfs)
    {
        if (double.IsNaN(dbfs))
        {
            return 0;
        }
        var clamped = Math.Clamp(dbfs, FloorDbfs, 0.0);
        var x = (clamped - FloorDbfs) / DbPerLevel;
        var whole = Math.Floor(x);
        var rounded = x - whole >= 0.5 ? whole + 1.0 : whole;
        return (byte)Math.Min(rounded, MaxLevel);
    }

    private static (int Start, int End) Slice(int i, int n, int count)
    {
        int At(int k) => (int)((UInt128)(ulong)k * (ulong)n / (ulong)count);
        var start = At(i);
        var end = Math.Max(At(i + 1), start + 1);
        return (start, end);
    }

    /// <summary>Levels reduced (or stretched) to <paramref name="count"/>: each slice its loudest, nothing above 15.</summary>
    public static byte[] Reduce(IReadOnlyList<byte> levels, int count)
    {
        var reduced = new byte[Math.Max(count, 0)];
        if (levels.Count == 0)
        {
            return reduced;
        }
        for (var i = 0; i < reduced.Length; i++)
        {
            var (start, end) = Slice(i, levels.Count, reduced.Length);
            byte loudest = 0;
            for (var k = start; k < end; k++)
            {
                loudest = Math.Max(loudest, levels[k]);
            }
            reduced[i] = Math.Min(loudest, MaxLevel);
        }
        return reduced;
    }

    /// <summary>Levels as the wire spells them: one lowercase hex digit each, anything above 15 as <c>f</c>.</summary>
    public static string Encode(IReadOnlyList<byte> levels) =>
        string.Concat(levels.Select(level => "0123456789abcdef"[Math.Min(level, MaxLevel)]));

    /// <summary>A recorder's peaks, in dBFS and in time order, as the wire's waveform of <paramref name="levels"/> digits.</summary>
    public static string FromPeaks(IReadOnlyList<double> samplesDbfs, int levels = Levels) =>
        Encode(Reduce(samplesDbfs.Select(Level).ToArray(), levels));

    /// <summary>Exactly the wire — 48 characters, each <c>0-9</c> or <c>a-f</c> — or null.</summary>
    public static byte[]? Parse(string? waveform)
    {
        if (waveform is null || waveform.Length != Levels)
        {
            return null;
        }
        var levels = new byte[Levels];
        for (var i = 0; i < Levels; i++)
        {
            var c = waveform[i];
            if (c is >= '0' and <= '9')
            {
                levels[i] = (byte)(c - '0');
            }
            else if (c is >= 'a' and <= 'f')
            {
                levels[i] = (byte)(c - 'a' + 10);
            }
            else
            {
                return null;
            }
        }
        return levels;
    }

    /// <summary>The attachment's waveform, or the neutral placeholder when it has none or it cannot be read.</summary>
    public static IReadOnlyList<byte> LevelsOrPlaceholder(string? waveform) => Parse(waveform) ?? Placeholder;

    /// <summary>The bars a bubble draws: the levels reduced to the count that fits.</summary>
    public static byte[] Bars(IReadOnlyList<byte> levels, int count) => Reduce(levels, count);

    /// <summary>A bar's height as a share of the waveform's: <c>(2 + level) / 17</c>, so silence is still a bar.</summary>
    public static double BarFraction(byte level) => (2 + Math.Min(level, MaxLevel)) / 17.0;

    /// <summary>How many bars are played at this position: <c>⌊position · bars / duration⌋</c>, never more than all.</summary>
    public static int PlayedBars(ulong positionMs, ulong durationMs, int bars)
    {
        if (durationMs == 0 || bars <= 0)
        {
            return 0;
        }
        var played = (UInt128)positionMs * (ulong)bars / durationMs;
        return played >= (ulong)bars ? bars : (int)played;
    }
}
