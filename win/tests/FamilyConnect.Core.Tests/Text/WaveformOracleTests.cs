using System.Text.Json;
using FamilyConnect.Core;

namespace FamilyConnect.Core.Tests.Text;

/// <summary>
/// A voice note's waveform held to <c>fc_text::waveform</c>, case for case: every case in
/// <c>Fixtures/waveform-vectors.json</c> was printed by the Rust (<c>cd win/tools/board-oracle &amp;&amp; cargo run --quiet --
/// waveform</c>), and the same bytes sit in the iOS test bundle and on Android's JVM classpath. A case naming a function
/// this suite does not know FAILS, so a function added to the original cannot be skipped here in silence.
/// </summary>
/// <remarks>
/// JSON holds no NaN or infinity, so those peaks are the strings <c>"NaN"</c>, <c>"Infinity"</c> and <c>"-Infinity"</c>.
/// A level is compared as a number and a fraction bit for bit: the file writes the shortest decimal that round-trips.
/// </remarks>
public class WaveformOracleTests
{
    private static readonly JsonElement[] Cases = Load();

    private static JsonElement[] Load()
    {
        var path = Path.Combine(AppContext.BaseDirectory, "Fixtures", "waveform-vectors.json");
        using var document = JsonDocument.Parse(File.ReadAllText(path));
        return [.. document.RootElement.EnumerateArray().Select(element => element.Clone())];
    }

    private static double Peak(JsonElement value) => value.ValueKind == JsonValueKind.String
        ? value.GetString() switch
        {
            "NaN" => double.NaN,
            "Infinity" => double.PositiveInfinity,
            "-Infinity" => double.NegativeInfinity,
            var other => throw new InvalidDataException($"a peak spelled {other}"),
        }
        : value.GetDouble();

    private static byte[] LevelsOf(JsonElement value) =>
        [.. value.EnumerateArray().Select(level => (byte)Math.Min(level.GetInt32(), 255))];

    [Fact]
    public void TheFileIsTheOneTheOriginalPrinted()
    {
        // 401 when this was written; fewer would mean a truncated copy.
        Assert.True(Cases.Length >= 401, $"{Cases.Length} cases");
        var functions = Cases.Select(c => c.GetProperty("function").GetString()).ToHashSet();
        foreach (var name in new[] { "constants", "level", "from_peaks", "parse", "levels_or_placeholder", "bars", "bar_fraction", "played_bars" })
        {
            Assert.Contains(name, functions);
        }
    }

    [Fact]
    public void EveryCaseAgrees()
    {
        var failures = new List<string>();
        foreach (var row in Cases)
        {
            var name = row.GetProperty("name").GetString()!;
            var input = row.GetProperty("input");
            var expected = row.GetProperty("expected");
            string? Fail(string what) => $"{name}: {what}";
            string? failure = row.GetProperty("function").GetString() switch
            {
                "constants" =>
                    expected.GetProperty("levels").GetInt32() != Waveform.Levels
                    || expected.GetProperty("max_level").GetInt32() != Waveform.MaxLevel
                    || expected.GetProperty("floor_dbfs").GetDouble() != Waveform.FloorDbfs
                    || expected.GetProperty("db_per_level").GetDouble() != Waveform.DbPerLevel
                    || expected.GetProperty("placeholder_level").GetInt32() != Waveform.PlaceholderLevel
                    || expected.GetProperty("placeholder").GetString() != Waveform.Encode(Waveform.Placeholder)
                        ? Fail("the constants differ")
                        : null,
                "level" => Waveform.Level(Peak(input.GetProperty("dbfs"))) is var level
                    && level != expected.GetProperty("level").GetInt32()
                        ? Fail($"level {level}")
                        : null,
                "from_peaks" => Waveform.FromPeaks(
                        [.. input.GetProperty("samples_dbfs").EnumerateArray().Select(Peak)],
                        input.GetProperty("levels").GetInt32()) is var wire
                    && wire != expected.GetProperty("waveform").GetString()
                        ? Fail($"waveform {wire}")
                        : null,
                "parse" => Waveform.Parse(input.GetProperty("waveform").GetString()) is var parsed
                    && !SameLevels(parsed, expected.GetProperty("levels"))
                        ? Fail("parse differs")
                        : null,
                "levels_or_placeholder" => Waveform.LevelsOrPlaceholder(
                        input.TryGetProperty("waveform", out var w) && w.ValueKind == JsonValueKind.String ? w.GetString() : null) is var shown
                    && !SameLevels([.. shown], expected.GetProperty("levels"))
                        ? Fail("levels differ")
                        : null,
                "bars" => Waveform.Bars(LevelsOf(input.GetProperty("levels")), input.GetProperty("count").GetInt32()) is var bars
                    && !SameLevels(bars, expected.GetProperty("bars"))
                        ? Fail($"bars {string.Join(',', bars)}")
                        : null,
                "bar_fraction" => Waveform.BarFraction((byte)Math.Min(input.GetProperty("level").GetInt32(), 255)) is var fraction
                    && !fraction.Equals(expected.GetProperty("fraction").GetDouble())
                        ? Fail($"fraction {fraction:R}")
                        : null,
                "played_bars" => Waveform.PlayedBars(
                        input.GetProperty("position_ms").GetUInt64(),
                        input.GetProperty("duration_ms").GetUInt64(),
                        input.GetProperty("bars").GetInt32()) is var played
                    && played != expected.GetProperty("played").GetInt32()
                        ? Fail($"played {played}")
                        : null,
                var other => Fail($"a function this suite does not know: {other}"),
            };
            if (failure is not null)
            {
                failures.Add(failure);
            }
        }
        Assert.True(failures.Count == 0, string.Join("\n", failures.Take(20)));
    }

    private static bool SameLevels(byte[]? actual, JsonElement expected) =>
        expected.ValueKind == JsonValueKind.Null
            ? actual is null
            : actual is not null && actual.SequenceEqual(LevelsOf(expected));

    [Fact]
    public void ATieRoundsUpWhereHalfToEvenWouldNot()
    {
        // −58 is x = 0.5 and −30 is x = 7.5: Math.Round would say 0 and 8; the rule says 1 and 8.
        Assert.Equal(1, Waveform.Level(-58.0));
        Assert.Equal(8, Waveform.Level(-30.0));
        Assert.Equal(0, Waveform.Level(BitConverter.Int64BitsToDouble(BitConverter.DoubleToInt64Bits(-58.0) + 1)));
    }
}
