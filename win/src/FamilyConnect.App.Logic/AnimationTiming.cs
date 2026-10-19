using FamilyConnect.Core;

namespace FamilyConnect.App.Logic;

/// <summary>One frame of an animation's video: which source frame (0-based, composited) and for how many ticks of 1/30 s.</summary>
public readonly record struct AnimationSample(int Frame, int Ticks);

/// <summary>
/// What an animated picture becomes (docs/protocol.md, "Animations", "The file"): the video's size, its frames in order, how
/// long it is and its bitrate.
/// </summary>
/// <param name="Width">Even, the source's orientation, never larger than the source — the video profile's size rule.</param>
/// <param name="Samples">In order; together they last <paramref name="TotalTicks"/>.</param>
/// <param name="DurationMs"><paramref name="TotalTicks"/> in milliseconds, rounded half up — the upload's <c>duration_ms</c>.</param>
/// <param name="VideoBitrate">The profile's bitrate for the target size at 30 frames a second.</param>
public sealed record AnimationPlan(
    int Width,
    int Height,
    IReadOnlyList<AnimationSample> Samples,
    int TotalTicks,
    long DurationMs,
    long VideoBitrate)
{
    /// <summary>The tick each sample starts on, in order — the video's presentation times on the 30-a-second grid.</summary>
    public IReadOnlyList<long> StartTicks
    {
        get
        {
            var starts = new long[Samples.Count];
            long at = 0;
            for (var index = 0; index < Samples.Count; index++)
            {
                starts[index] = at;
                at += Samples[index].Ticks;
            }
            return starts;
        }
    }
}

/// <summary>
/// The TIMING half of an animated picture sent as a silent looping video (issue #96) — a port of <c>fc_text::animation</c>,
/// held to it by <c>animation-plan-vectors.json</c>, because four encoders must make the same clip of the same picture.
/// Decoding the frames and encoding the video are each platform's own.
/// </summary>
/// <remarks>
/// <para>
/// <b>THE GRID.</b> The animation is SAMPLED thirty times a second: at each tick the frame that is showing; ticks in a row
/// that show the same frame are one video frame, that many ticks long. A held frame stays held, nothing runs faster than
/// 30 a second, and a slow animation is never given frames it did not have. A flash no tick lands on is not in the video.
/// </para>
/// <para>
/// <b>THE SHORTEST FRAME.</b> A frame stated at 10 ms or less is shown for 100 ms — the browsers' rule, for the many GIFs
/// that say "0" and mean "the default".
/// </para>
/// <para>
/// The reference walks every tick; this walks every FRAME (a frame's ticks are those from the first one at or after its
/// start to the first one at or after the next frame's start), which is the same answer without a loop over hours of
/// ticks for a WebP that states a frame of four hours. <c>AnimationTimingTests</c> holds the two to each other.
/// </para>
/// </remarks>
public static class AnimationTiming
{
    /// <summary>Samples a second: the grid, and the timescale every duration in a plan is counted in.</summary>
    public const int TicksPerSecond = 30;

    /// <summary>A frame stated at this or less is shown for <see cref="DefaultFrameMs"/>.</summary>
    public const uint ShortestFrameMs = 10;

    /// <summary>How long such a frame is shown.</summary>
    public const uint DefaultFrameMs = 100;

    /// <summary>What shows through a transparent pixel: a video has none.</summary>
    public static IReadOnlyList<byte> BackgroundRgb { get; } = [0xFF, 0xFF, 0xFF];

    /// <summary>A GIF frame's delay — hundredths of a second, as its Graphic Control Extension states it — in milliseconds.</summary>
    public static uint GifDelayMs(ushort centiseconds) => centiseconds * 10u;

    /// <summary>
    /// An APNG frame's delay from its <c>fcTL</c>: <c>num ÷ den</c> seconds, a <c>den</c> of 0 meaning hundredths, in whole
    /// milliseconds rounded half up.
    /// </summary>
    public static uint ApngDelayMs(ushort numerator, ushort denominator)
    {
        ulong den = denominator == 0 ? 100UL : denominator;
        return (uint)(((2UL * numerator * 1000) + den) / (2 * den));
    }

    /// <summary>How long a frame stated at <paramref name="statedMs"/> is shown: as stated, unless 10 ms or less.</summary>
    public static uint FrameDurationMs(uint statedMs) => statedMs <= ShortestFrameMs ? DefaultFrameMs : statedMs;

    /// <summary>
    /// The plan for an animation of <paramref name="width"/> × <paramref name="height"/> whose frames are stated to last
    /// <paramref name="statedMs"/> each, in the source's order. Null when there is no size, no frame, or a size that rounds
    /// to nothing.
    /// </summary>
    public static AnimationPlan? Plan(long width, long height, IReadOnlyList<uint> statedMs)
    {
        if (width <= 0 || height <= 0 || statedMs.Count == 0)
        {
            return null;
        }
        var (targetWidth, targetHeight) = MediaPlan.TargetSize(width, height);
        if (targetWidth <= 0 || targetHeight <= 0 || targetWidth > int.MaxValue || targetHeight > int.MaxValue)
        {
            return null;
        }
        // Where each frame starts, in milliseconds; and where the last one ends.
        var starts = new ulong[statedMs.Count];
        ulong total = 0;
        for (var index = 0; index < statedMs.Count; index++)
        {
            starts[index] = total;
            total += FrameDurationMs(statedMs[index]);
        }
        const ulong tps = TicksPerSecond;
        // Enough ticks to cover the whole animation: ceil(total × 30 ÷ 1000), at least one.
        var totalTicks = Math.Max(CeilTicks(total), 1UL);
        if (totalTicks > uint.MaxValue || totalTicks > int.MaxValue)
        {
            return null;
        }
        var samples = new List<AnimationSample>();
        for (var frame = 0; frame < starts.Length; frame++)
        {
            // The first tick at or after this frame's start (tick × 1000 ≥ start × 30), and the first at or after the next's.
            var first = CeilTicks(starts[frame]);
            var end = frame + 1 < starts.Length ? Math.Min(CeilTicks(starts[frame + 1]), totalTicks) : totalTicks;
            if (end > first)
            {
                samples.Add(new AnimationSample(frame, (int)(end - first)));
            }
        }
        return new AnimationPlan(
            (int)targetWidth,
            (int)targetHeight,
            samples,
            (int)totalTicks,
            (long)(((2 * totalTicks * 1000) + tps) / (2 * tps)),
            MediaPlan.ProfileVideoBitrate(targetWidth, targetHeight, MediaPlan.MaxFrameRate));

        static ulong CeilTicks(ulong ms) => ((ms * tps) + 999) / 1000;
    }

    /// <summary>
    /// A tick on the grid as Media Foundation counts time — hundreds of nanoseconds — rounded to the nearest: 1/30 s is not a
    /// whole number of them, so each sample's time is placed and its length is what reaches the next one.
    /// </summary>
    public static long Hundreds(long tick) => ((tick * 10_000_000L) + (TicksPerSecond / 2)) / TicksPerSecond;
}
