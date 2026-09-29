using System.Text.Json;
using FamilyConnect.Core;

namespace FamilyConnect.Core.Tests.Text;

/// <summary>
/// THE THIRD DIFFERENTIAL ORACLE: what a picked video or sound file becomes before upload. Every case in
/// <c>Fixtures/media-plan-vectors.json</c> was produced by <c>fc_text::media_plan</c> — the Rust the web client runs —
/// and this suite holds <see cref="MediaPlan"/> to it, number for number, every intermediate included.
/// </summary>
/// <remarks>
/// <para>
/// Unlike the other two fixtures this one is held by THREE ports: the same bytes sit in the iOS test bundle and on
/// Android's JVM test classpath, and CI fails when any copy differs from a fresh print. Regenerate with
/// <c>cd win/tools/board-oracle &amp;&amp; cargo run --quiet -- media-plan &gt;
/// ../../tests/FamilyConnect.Core.Tests/Fixtures/media-plan-vectors.json</c> and copy it to the other two (win/README.md).
/// </para>
/// <para>
/// The file is one case per line with its keys sorted, so it is PARSED, never compared as text. Each case names the
/// function it is for; a case this suite does not know fails, so a function added to the original cannot be skipped
/// here in silence. The half-thousand ties (404×288 at 30 is 252 500 exactly) are in it on purpose: they are what
/// <see cref="Math.Round(double)"/>'s default half-to-even would get wrong.
/// </para>
/// </remarks>
public class MediaPlanOracleTests
{
    private static readonly JsonElement[] Cases = Load();

    private static JsonElement[] Load()
    {
        var path = Path.Combine(AppContext.BaseDirectory, "Fixtures", "media-plan-vectors.json");
        using var document = JsonDocument.Parse(File.ReadAllText(path));
        return [.. document.RootElement.EnumerateArray().Select(element => element.Clone())];
    }

    private static long? OptionalLong(JsonElement row, string name) =>
        row.GetProperty(name).ValueKind == JsonValueKind.Null ? null : row.GetProperty(name).GetInt64();

    private static double? OptionalDouble(JsonElement row, string name) =>
        row.GetProperty(name).ValueKind == JsonValueKind.Null ? null : row.GetProperty(name).GetDouble();

    private static string? OptionalString(JsonElement row, string name) =>
        row.GetProperty(name).ValueKind == JsonValueKind.Null ? null : row.GetProperty(name).GetString();

    private static VideoSource Video(JsonElement input) => new(
        input.GetProperty("width").GetInt64(),
        input.GetProperty("height").GetInt64(),
        OptionalDouble(input, "frame_rate"),
        input.GetProperty("container").GetString()!,
        input.GetProperty("video_codec").GetString()!,
        OptionalString(input, "audio_codec"),
        OptionalLong(input, "audio_channels"),
        OptionalLong(input, "video_bitrate"),
        OptionalLong(input, "audio_bitrate"),
        input.GetProperty("size_bytes").GetInt64(),
        OptionalLong(input, "duration_ms"));

    private static AudioSource Audio(JsonElement input) => new(
        input.GetProperty("container").GetString()!,
        input.GetProperty("codec").GetString()!,
        OptionalLong(input, "channels"),
        OptionalLong(input, "bitrate"),
        input.GetProperty("size_bytes").GetInt64(),
        OptionalLong(input, "duration_ms"));

    private static string Named(VideoPlanKind kind) => kind switch
    {
        VideoPlanKind.Keep => "keep",
        VideoPlanKind.Transcode => "transcode",
        _ => "fallback",
    };

    /// <summary>A frame rate is compared bit for bit: the file writes the shortest decimal that round-trips.</summary>
    private static void SameRate(double expected, double actual, string name) =>
        Assert.True(expected.Equals(actual), $"{name}: frame rate {actual:R}, the original says {expected:R}");

    [Fact]
    public void TheFileIsTheOneTheOriginalPrinted()
    {
        // 464 when this was written; fewer would mean a truncated copy, and every function has its cases.
        Assert.True(Cases.Length >= 464, $"only {Cases.Length} cases");
        string[] functions =
        [
            "target_size", "target_frame_rate", "profile_video_bitrate", "target_video_bitrate", "target_audio_bitrate",
            "estimated_bitrate", "plan_video", "plan_audio", "sendable", "on_failure", "keep_smaller",
        ];
        var named = Cases.Select(row => row.GetProperty("function").GetString()).ToHashSet();
        Assert.Equal(functions.Order(), named.Order());
    }

    [Fact]
    public void TheConstantsAreTheOriginals()
    {
        // The protocol's table, by value — a changed number here is a changed protocol, not a refactor.
        Assert.Equal(720, MediaPlan.MaxShortSide);
        Assert.Equal(30.0, MediaPlan.MaxFrameRate);
        Assert.Equal(30.5, MediaPlan.FrameRateTolerance);
        Assert.Equal(250_000, MediaPlan.MinVideoBitrate);
        Assert.Equal(2_000_000, MediaPlan.MaxVideoBitrate);
        Assert.Equal(128_000, MediaPlan.StereoAudioBitrate);
        Assert.Equal(64_000, MediaPlan.MonoAudioBitrate);
        Assert.Equal(64_000, MediaPlan.VoiceNoteBitrate);
        Assert.Equal(192_000, MediaPlan.MaxKeptLossyAudioBitrate);
    }

    /// <summary>
    /// Every case, whatever function it names. One test rather than one per function so that the count below is the
    /// whole file: the assertion names the case that disagreed, which is what finding a divergence needs.
    /// </summary>
    [Fact]
    public void EveryCaseIsDecidedAsTheOriginalDecidesIt()
    {
        var checkedCases = 0;
        foreach (var row in Cases)
        {
            var name = row.GetProperty("name").GetString()!;
            var input = row.GetProperty("input");
            var expected = row.GetProperty("expected");
            switch (row.GetProperty("function").GetString())
            {
                case "target_size":
                {
                    var (width, height) = MediaPlan.TargetSize(input.GetProperty("width").GetInt64(), input.GetProperty("height").GetInt64());
                    Assert.True(
                        (expected.GetProperty("width").GetInt64(), expected.GetProperty("height").GetInt64()) == (width, height),
                        $"{name}: {width}x{height}");
                    break;
                }
                case "target_frame_rate":
                    SameRate(
                        expected.GetProperty("frame_rate").GetDouble(),
                        MediaPlan.TargetFrameRate(OptionalDouble(input, "frame_rate")),
                        name);
                    break;
                case "profile_video_bitrate":
                    Assert.True(
                        expected.GetProperty("bitrate").GetInt64() == MediaPlan.ProfileVideoBitrate(
                            input.GetProperty("width").GetInt64(), input.GetProperty("height").GetInt64(), input.GetProperty("frame_rate").GetDouble()),
                        name);
                    break;
                case "target_video_bitrate":
                    Assert.True(
                        expected.GetProperty("bitrate").GetInt64() == MediaPlan.TargetVideoBitrate(
                            input.GetProperty("width").GetInt64(), input.GetProperty("height").GetInt64(),
                            input.GetProperty("frame_rate").GetDouble(), OptionalLong(input, "source_bitrate")),
                        name);
                    break;
                case "target_audio_bitrate":
                    Assert.True(
                        expected.GetProperty("bitrate").GetInt64() == MediaPlan.TargetAudioBitrate(
                            OptionalLong(input, "channels"), OptionalLong(input, "source_bitrate")),
                        name);
                    break;
                case "estimated_bitrate":
                    Assert.True(
                        OptionalLong(expected, "bitrate") == MediaPlan.EstimatedBitrate(
                            input.GetProperty("size_bytes").GetInt64(), OptionalLong(input, "duration_ms"), input.GetProperty("audio_bitrate").GetInt64()),
                        name);
                    break;
                case "plan_video":
                    CheckVideo(name, Video(input), expected);
                    break;
                case "plan_audio":
                    CheckAudio(name, Audio(input), expected);
                    break;
                case "sendable":
                    Assert.True(
                        expected.GetProperty("sendable").GetBoolean() == MediaPlan.Sendable(
                            input.GetProperty("kind").GetString()!, input.GetProperty("container").GetString()!,
                            input.GetProperty("honest").GetBoolean(), input.GetProperty("size_bytes").GetInt64(),
                            input.GetProperty("ceiling_bytes").GetInt64()),
                        name);
                    break;
                case "on_failure":
                {
                    var send = MediaPlan.AfterFailure(input.GetProperty("source_sendable").GetBoolean());
                    Assert.True(expected.GetProperty("send").GetString() == (send == OnFailure.Original ? "original" : "todays_path"), name);
                    break;
                }
                case "keep_smaller":
                {
                    var upload = MediaPlan.KeepSmaller(
                        input.GetProperty("source_bytes").GetInt64(), input.GetProperty("source_sendable").GetBoolean(), input.GetProperty("result_bytes").GetInt64());
                    Assert.True(expected.GetProperty("upload").GetString() == (upload == Upload.Source ? "source" : "result"), name);
                    break;
                }
                default:
                    Assert.Fail($"{name}: a function this port does not know — {row.GetProperty("function").GetString()}");
                    break;
            }
            checkedCases++;
        }
        Assert.Equal(Cases.Length, checkedCases);
    }

    private static void CheckVideo(string name, VideoSource source, JsonElement expected)
    {
        var plan = MediaPlan.PlanVideo(source);
        Assert.True(expected.GetProperty("plan").GetString() == Named(plan.Kind), $"{name}: plan {Named(plan.Kind)}");
        Assert.True(expected.GetProperty("within_profile").GetBoolean() == MediaPlan.WithinProfile(source), $"{name}: rule A");
        Assert.True(OptionalLong(expected, "source_video_bitrate") == MediaPlan.SourceVideoBitrate(source), $"{name}: V");
        var target = MediaPlan.VideoTargetFor(source);
        var wanted = expected.GetProperty("target");
        if (wanted.ValueKind == JsonValueKind.Null)
        {
            Assert.True(target is null, $"{name}: a target where the original has none");
            return;
        }
        Assert.True(target is not null, $"{name}: no target where the original has one");
        var actual = target.Value;
        Assert.True(wanted.GetProperty("width").GetInt64() == actual.Width, $"{name}: target width {actual.Width}");
        Assert.True(wanted.GetProperty("height").GetInt64() == actual.Height, $"{name}: target height {actual.Height}");
        SameRate(wanted.GetProperty("frame_rate").GetDouble(), actual.FrameRate, name);
        Assert.True(wanted.GetProperty("video_bitrate").GetInt64() == actual.VideoBitrate, $"{name}: target video bitrate {actual.VideoBitrate}");
        Assert.True(OptionalLong(wanted, "audio_bitrate") == actual.AudioBitrate, $"{name}: target audio bitrate {actual.AudioBitrate}");
        if (plan.Kind == VideoPlanKind.Transcode)
        {
            // A transcode is to the target, and to nothing else.
            Assert.Equal(target, plan.Target);
        }
        else
        {
            Assert.Null(plan.Target);
        }
    }

    private static void CheckAudio(string name, AudioSource source, JsonElement expected)
    {
        var bitrate = MediaPlan.SourceAudioBitrate(source);
        Assert.True(OptionalLong(expected, "source_bitrate") == bitrate, $"{name}: source bitrate {bitrate}");
        var target = MediaPlan.TargetAudioBitrate(source.Channels, bitrate);
        Assert.True(expected.GetProperty("target_bitrate").GetInt64() == target, $"{name}: target bitrate {target}");
        var plan = MediaPlan.PlanAudio(source);
        var said = plan.Kind == AudioPlanKind.Transcode ? "transcode" : "keep";
        Assert.True(expected.GetProperty("plan").GetString() == said, $"{name}: plan {said}");
        if (plan.Kind == AudioPlanKind.Transcode)
        {
            Assert.Equal(target, plan.Bitrate);
        }
    }

    /// <summary>
    /// What the file cannot carry: JSON has no NaN or infinity, so the reader's other "unknown" frame rates are pinned
    /// here, against the rule the original states for them (<c>known_frame_rate</c>).
    /// </summary>
    [Fact]
    public void AFrameRateThatIsNotANumberIsUnknown()
    {
        foreach (var nonsense in new[] { double.NaN, double.PositiveInfinity, double.NegativeInfinity, 0.0, -1.0 })
        {
            Assert.Null(MediaPlan.KnownFrameRate(nonsense));
            Assert.Equal(30.0, MediaPlan.TargetFrameRate(nonsense));
        }
        var clip = new VideoSource(1280, 720, double.NaN, "video/mp4", "h264", "aac", 2, 1_500_000, 128_000, 20_000_000, 19_000);
        Assert.False(MediaPlan.WithinProfile(clip));
        Assert.Equal(VideoPlan.Transcode(new VideoTarget(1280, 720, 30.0, 1_500_000, 128_000)), MediaPlan.PlanVideo(clip));
    }

    /// <summary>Half UP, where this platform's default is half to EVEN — the one mistake a C# port is built to make.</summary>
    [Fact]
    public void AHalfThousandRoundsUpAndNotToEven()
    {
        Assert.Equal(253_000, MediaPlan.ProfileVideoBitrate(404, 288, 30.0));
        Assert.Equal(303_000, MediaPlan.ProfileVideoBitrate(484, 360, 24.0));
        Assert.Equal(938_000, MediaPlan.ProfileVideoBitrate(960, 540, 25.0));
        Assert.Equal(888_000, MediaPlan.ProfileVideoBitrate(852, 480, 30.0));
        // Left to right as the protocol writes it, this one is 837.4999… and would round down.
        Assert.Equal(838_000, MediaPlan.ProfileVideoBitrate(3618, 128, 25.0));
    }
}
