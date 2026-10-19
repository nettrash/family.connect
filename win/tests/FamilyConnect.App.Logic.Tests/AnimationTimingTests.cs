using System.Text.Json;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// AN ANIMATION'S TIMING, held to <c>fc_text::animation</c> case for case (issue #96; docs/protocol.md, "Animations", "The
/// file"). <c>Fixtures/animation-plan-vectors.json</c> was printed by the Rust the web client runs (<c>win/tools/board-oracle</c>,
/// <c>cargo run -- animation-plan</c>) and is LINKED here from beside the Core tests, where CI compares it with a fresh print;
/// the Apple and Android ports hold the same bytes.
/// </summary>
public sealed class AnimationTimingTests
{
    private static readonly JsonElement[] Vectors = Load();

    private static JsonElement[] Load()
    {
        var path = Path.Combine(AppContext.BaseDirectory, "Fixtures", "animation-plan-vectors.json");
        using var document = JsonDocument.Parse(File.ReadAllText(path));
        return [.. document.RootElement.EnumerateArray().Select(vector => vector.Clone())];
    }

    private static IEnumerable<JsonElement> Of(string function) =>
        Vectors.Where(vector => vector.GetProperty("function").GetString() == function);

    [Fact]
    public void TheVectorsAreAllHere()
    {
        // Every function the oracle prints is one this port answers: a vector of a kind nobody reads would pass by being ignored.
        Assert.All(Vectors, vector => Assert.Contains(
            vector.GetProperty("function").GetString(),
            new[] { "constants", "gif_delay_ms", "apng_delay_ms", "frame_duration_ms", "plan" }));
        Assert.Equal(18, Of("plan").Count());
        Assert.Equal(12, Of("apng_delay_ms").Count());
        Assert.Equal(7, Of("gif_delay_ms").Count());
        Assert.Equal(8, Of("frame_duration_ms").Count());
    }

    [Fact]
    public void TheConstantsAreTheReferences()
    {
        var expected = Assert.Single(Of("constants")).GetProperty("expected");
        Assert.Equal(AnimationTiming.TicksPerSecond, expected.GetProperty("ticks_per_second").GetInt32());
        Assert.Equal(AnimationTiming.ShortestFrameMs, expected.GetProperty("shortest_frame_ms").GetUInt32());
        Assert.Equal(AnimationTiming.DefaultFrameMs, expected.GetProperty("default_frame_ms").GetUInt32());
        Assert.Equal(
            expected.GetProperty("background_rgb").EnumerateArray().Select(value => (byte)value.GetInt32()),
            AnimationTiming.BackgroundRgb);
    }

    [Fact]
    public void EveryDelayIsReadAsTheReferenceReadsIt()
    {
        foreach (var vector in Of("gif_delay_ms"))
        {
            var input = vector.GetProperty("input");
            Assert.True(
                vector.GetProperty("expected").GetProperty("ms").GetUInt32()
                    == AnimationTiming.GifDelayMs(input.GetProperty("centiseconds").GetUInt16()),
                vector.GetProperty("name").GetString());
        }
        foreach (var vector in Of("apng_delay_ms"))
        {
            var input = vector.GetProperty("input");
            Assert.True(
                vector.GetProperty("expected").GetProperty("ms").GetUInt32()
                    == AnimationTiming.ApngDelayMs(input.GetProperty("num").GetUInt16(), input.GetProperty("den").GetUInt16()),
                vector.GetProperty("name").GetString());
        }
        foreach (var vector in Of("frame_duration_ms"))
        {
            Assert.True(
                vector.GetProperty("expected").GetProperty("ms").GetUInt32()
                    == AnimationTiming.FrameDurationMs(vector.GetProperty("input").GetProperty("stated_ms").GetUInt32()),
                vector.GetProperty("name").GetString());
        }
    }

    [Fact]
    public void EveryPlanIsTheReferences()
    {
        foreach (var vector in Of("plan"))
        {
            var name = vector.GetProperty("name").GetString();
            var input = vector.GetProperty("input");
            var stated = input.GetProperty("stated_ms").EnumerateArray().Select(value => value.GetUInt32()).ToArray();
            var plan = AnimationTiming.Plan(input.GetProperty("width").GetInt64(), input.GetProperty("height").GetInt64(), stated);
            var expected = vector.GetProperty("expected");
            if (expected.ValueKind == JsonValueKind.Null)
            {
                Assert.True(plan is null, name);
                continue;
            }
            Assert.True(plan is not null, name);
            Assert.True(expected.GetProperty("width").GetInt32() == plan.Width, name);
            Assert.True(expected.GetProperty("height").GetInt32() == plan.Height, name);
            Assert.True(expected.GetProperty("total_ticks").GetInt32() == plan.TotalTicks, name);
            Assert.True(expected.GetProperty("duration_ms").GetInt64() == plan.DurationMs, name);
            Assert.True(expected.GetProperty("video_bitrate").GetInt64() == plan.VideoBitrate, name);
            var samples = expected.GetProperty("samples").EnumerateArray()
                .Select(pair => new AnimationSample(pair[0].GetInt32(), pair[1].GetInt32())).ToArray();
            Assert.True(samples.SequenceEqual(plan.Samples), $"{name}: {string.Join(" ", plan.Samples)}");
        }
    }

    /// <summary>
    /// The reference walks every TICK; this port walks every FRAME. The two are held to each other here over shapes the
    /// vectors do not hold: flashes that land on a tick and between ticks, long holds, the shortest-frame rule.
    /// </summary>
    [Fact]
    public void WalkingTheFramesIsWalkingTheTicks()
    {
        var random = new Random(96);
        for (var round = 0; round < 500; round++)
        {
            var stated = Enumerable.Range(0, random.Next(1, 40))
                .Select(_ => (uint)(random.Next(4) switch
                {
                    0 => random.Next(0, 12),
                    1 => random.Next(12, 40),
                    2 => random.Next(40, 200),
                    _ => random.Next(200, 3000),
                }))
                .ToArray();
            var plan = AnimationTiming.Plan(640, 480, stated)!;
            Assert.Equal(TickByTick(stated), plan.Samples);
            Assert.Equal(plan.TotalTicks, plan.Samples.Sum(sample => sample.Ticks));
            Assert.True(plan.Samples.Count <= stated.Length);
        }
    }

    /// <summary><c>fc_text::animation::plan</c>'s own loop, written out again.</summary>
    private static List<AnimationSample> TickByTick(uint[] stated)
    {
        var starts = new List<ulong>();
        ulong total = 0;
        foreach (var ms in stated)
        {
            starts.Add(total);
            total += AnimationTiming.FrameDurationMs(ms);
        }
        var ticks = Math.Max((total * 30 + 999) / 1000, 1);
        var samples = new List<AnimationSample>();
        var frame = 0;
        for (ulong tick = 0; tick < ticks; tick++)
        {
            while (frame + 1 < starts.Count && starts[frame + 1] * 30 <= tick * 1000)
            {
                frame++;
            }
            if (samples.Count > 0 && samples[^1].Frame == frame)
            {
                samples[^1] = samples[^1] with { Ticks = samples[^1].Ticks + 1 };
            }
            else
            {
                samples.Add(new AnimationSample(frame, 1));
            }
        }
        return samples;
    }

    /// <summary>A WebP may state a frame of four and a half hours: the plan is still a handful of samples, made at once.</summary>
    [Fact]
    public void AnHoursLongHoldIsOneSample()
    {
        var plan = AnimationTiming.Plan(320, 240, [100, 16_777_215, 100])!;
        Assert.Equal([new AnimationSample(0, 3), new AnimationSample(1, 503_317), new AnimationSample(2, 3)], plan.Samples);
    }

    /// <summary>
    /// Where each sample starts on the grid, and that tick in Media Foundation's hundreds of nanoseconds, rounded to the
    /// nearest: a thirtieth of a second is 333 333⅓ of them.
    /// </summary>
    [Fact]
    public void EachSampleStartsOnItsTick()
    {
        var plan = AnimationTiming.Plan(200, 200, [20, 1000, 500])!;
        Assert.Equal([0L, 1L, 31L], plan.StartTicks);
        Assert.Equal(0, AnimationTiming.Hundreds(0));
        Assert.Equal(333_333, AnimationTiming.Hundreds(1));
        Assert.Equal(666_667, AnimationTiming.Hundreds(2));
        Assert.Equal(10_000_000, AnimationTiming.Hundreds(30));
        Assert.Equal(10_333_333, AnimationTiming.Hundreds(31));
    }
}
