using System.Text.Json;
using FamilyConnect.Core;
using FamilyConnect.Core.Board;

namespace FamilyConnect.Core.Tests.Board;

/// <summary>
/// THE DIFFERENTIAL ORACLE: every vector in <c>Fixtures/board-vectors.json</c> was produced by
/// <c>fc_text::board</c> itself — the Rust the web client runs — and this suite holds the C# port
/// to it, number for number and string for string.
/// </summary>
/// <remarks>
/// <para>
/// This is the check that a hand-written port cannot give itself. The portfolio has been bitten
/// twice by exactly what it catches: a byte-versus-character split that panicked on Cyrillic, and
/// four ports that agreed with each other and were all wrong about <c>pow(10, n)</c>. Four
/// implementations of one rule need an oracle, not four readings.
/// </para>
/// <para>
/// Regenerate with the little Rust program that produced it (docs/windows-client: the oracle is
/// built from the repo's own <c>web/text</c> crate by path, so it cannot drift from the source it
/// claims to speak for).
/// </para>
/// </remarks>
public class OracleTests
{
    private static readonly JsonDocument Vectors = Load();

    private static JsonDocument Load()
    {
        var path = Path.Combine(AppContext.BaseDirectory, "Fixtures", "board-vectors.json");
        return JsonDocument.Parse(File.ReadAllText(path));
    }

    private static JsonElement Section(string name) => Vectors.RootElement.GetProperty(name);

    /// <remarks>
    /// The port's constant is passed as the "expected" argument throughout, because xUnit's
    /// analyzer insists a constant goes first. The ORACLE is still the authority: what is being
    /// asserted is that the two agree.
    /// </remarks>
    [Fact]
    public void TheConstantsAreTheOriginals()
    {
        var constants = Section("constants");
        Assert.Equal(NoteText.MaxTextChars, constants.GetProperty("max_text").GetInt32());
        Assert.Equal(NoteText.MaxPlaceChars, constants.GetProperty("max_place").GetInt32());
        Assert.Equal(NoteText.MaxTaskItemChars, constants.GetProperty("max_task_item").GetInt32());
        Assert.Equal(NoteText.MaxTaskItems, constants.GetProperty("max_task_items").GetInt32());
        Assert.Equal(NoteText.CounterFrom, constants.GetProperty("counter_from").GetInt32());
        Assert.Equal(BoardWall.Screens, constants.GetProperty("wall_screens").GetDouble());
        Assert.Equal(BoardTasks.OnWall, constants.GetProperty("wall_task_lines").GetInt32());
        Assert.Equal(BoardWall.CompactBelow, constants.GetProperty("compact_below").GetDouble());
        Assert.Equal(NoteFitting.MinTextScale, constants.GetProperty("min_text_scale").GetDouble());
        Assert.Equal(NoteFitting.FitSteps, constants.GetProperty("fit_steps").GetInt32());
    }

    [Fact]
    public void TheCapCutsWhereTheOriginalCuts()
    {
        foreach (var vector in Section("capped").EnumerateArray())
        {
            var text = vector.GetProperty("text").GetString()!;
            var max = vector.GetProperty("max").GetInt32();
            Assert.Equal(vector.GetProperty("scalars").GetInt32(), NoteText.Scalars(text));
            Assert.Equal(vector.GetProperty("kept").GetString(), NoteText.Capped(text, max));
        }
    }

    [Fact]
    public void TheCaretMovesWhereTheOriginalMovesIt()
    {
        foreach (var vector in Section("cap_at_caret").EnumerateArray())
        {
            var value = vector.GetProperty("value").GetString()!;
            var caret = vector.GetProperty("caret").GetInt32();
            var max = vector.GetProperty("max").GetInt32();
            var (kept, moved) = NoteText.CapAtCaret(value, caret, max);
            Assert.Equal(vector.GetProperty("kept").GetString(), kept);
            Assert.Equal(vector.GetProperty("moved").GetInt32(), moved);
        }
    }

    [Fact]
    public void TheCounterCountsWhatTheOriginalCounts()
    {
        foreach (var vector in Section("remaining").EnumerateArray())
        {
            var text = vector.GetProperty("text").GetString()!;
            Assert.Equal(vector.GetProperty("remaining").GetInt32(), NoteText.Remaining(text));
            Assert.Equal(vector.GetProperty("counter").GetBoolean(), NoteText.ShowsCounter(text));
        }
    }

    [Fact]
    public void APictureIsFittedExactlyAsTheOriginalFitsIt()
    {
        foreach (var vector in Section("fitted_picture").EnumerateArray())
        {
            var space = vector.GetProperty("space");
            var picture = vector.GetProperty("picture");
            var fitted = vector.GetProperty("fitted");
            var (width, height) = BoardPicture.Fitted(
                space[0].GetDouble(), space[1].GetDouble(),
                (int)picture[0].GetDouble(), (int)picture[1].GetDouble());
            Assert.Equal(fitted[0].GetDouble(), width, 9);
            Assert.Equal(fitted[1].GetDouble(), height, 9);
        }
    }

    [Fact]
    public void TheWallAndTheCardsMeasureAsTheOriginalDoes()
    {
        foreach (var vector in Section("wall_height").EnumerateArray())
        {
            Assert.Equal(
                vector.GetProperty("height").GetDouble(),
                BoardWall.Height(vector.GetProperty("visible").GetDouble()), 9);
        }
        foreach (var vector in Section("cards").EnumerateArray())
        {
            var size = Notes.SizeFrom(vector.GetProperty("size").GetString());
            var compact = vector.GetProperty("compact").GetBoolean();
            var card = vector.GetProperty("card");
            Assert.Equal((card[0].GetDouble(), card[1].GetDouble()), BoardWall.Card(size, compact));
            Assert.Equal(vector.GetProperty("type_px").GetDouble(), BoardWall.TypePx(size), 9);
        }
        foreach (var vector in Section("lines_that_fit").EnumerateArray())
        {
            Assert.Equal(
                vector.GetProperty("lines").GetInt32(),
                NoteFitting.LinesThatFit(
                    vector.GetProperty("height").GetDouble(),
                    vector.GetProperty("line_height").GetDouble()));
        }
    }

    [Fact]
    public void ANoteSitsWhereTheOriginalPutsIt()
    {
        foreach (var vector in Section("geometry").EnumerateArray())
        {
            var fraction = Pair(vector.GetProperty("fraction"));
            var card = Pair(vector.GetProperty("card"));
            var board = Pair(vector.GetProperty("board"));
            var offset = Pair(vector.GetProperty("offset"));
            var origin = Pair(vector.GetProperty("origin"));
            var dragged = Pair(vector.GetProperty("dragged"));
            var back = Pair(vector.GetProperty("fraction_of_origin"));
            Assert.Equal(origin, BoardWall.Origin(fraction, card, board));
            Assert.Equal(dragged, BoardWall.Dragged(fraction, offset, card, board));
            var (x, y) = BoardWall.FractionOf(origin, board);
            Assert.Equal(back.Item1, x, 9);
            Assert.Equal(back.Item2, y, 9);
        }
        foreach (var vector in Section("tilt").EnumerateArray())
        {
            Assert.Equal(
                vector.GetProperty("degrees").GetInt32(),
                BoardWall.TiltDegrees(vector.GetProperty("id").GetInt64()));
        }
        foreach (var vector in Section("task_lines").EnumerateArray())
        {
            var (shown, left) = BoardTasks.Drawn(vector.GetProperty("total").GetInt32());
            Assert.Equal(vector.GetProperty("shown").GetInt32(), shown);
            Assert.Equal(vector.GetProperty("left").GetInt32(), left);
        }
    }

    [Fact]
    public void EveryNameFallsBackAsTheOriginalFallsBack()
    {
        foreach (var vector in Section("colors").EnumerateArray())
        {
            Assert.Equal(
                vector.GetProperty("hex").GetString(),
                Notes.ColorHex(vector.GetProperty("name").GetString()));
        }
        foreach (var vector in Section("fallbacks").EnumerateArray())
        {
            var given = Given(vector);
            Assert.Equal(vector.GetProperty("size").GetString(), Notes.NameOf(Notes.SizeFrom(given)));
        }
        foreach (var vector in Section("kinds").EnumerateArray())
        {
            var given = Given(vector);
            Assert.Equal(vector.GetProperty("kind").GetString(), Notes.NameOf(Notes.KindFrom(given)));
        }
        foreach (var vector in Section("answers").EnumerateArray())
        {
            var given = Given(vector);
            var answer = Notes.AnswerFrom(given);
            var expected = vector.GetProperty("answer");
            if (expected.ValueKind == JsonValueKind.Null)
            {
                // The one field where an unknown value lights NOTHING rather than falling back.
                Assert.Null(answer);
            }
            else
            {
                Assert.NotNull(answer);
                Assert.Equal(expected.GetString(), Notes.NameOf(answer.Value));
            }
        }
    }

    [Fact]
    public void TheStickerSaysWhatTheOriginalSays()
    {
        foreach (var vector in Section("going_line").EnumerateArray())
        {
            var line = Notes.GoingLine(
                vector.GetProperty("going").GetInt32(),
                vector.GetProperty("maybe").GetInt32(),
                EnglishCatalog.Instance);
            var expected = vector.GetProperty("line");
            if (expected.ValueKind == JsonValueKind.Null)
            {
                Assert.Null(line);
            }
            else
            {
                Assert.Equal(expected.GetString(), line);
            }
        }
    }

    [Fact]
    public void TheBadgeJudgesAsTheOriginalJudges()
    {
        var marks = new BoardMarks(10, 100);
        foreach (var vector in Section("unread").EnumerateArray())
        {
            var seq = vector.GetProperty("content_seq");
            Assert.Equal(
                vector.GetProperty("unread").GetBoolean(),
                BoardBadge.IsUnread(
                    vector.GetProperty("note_id").GetInt64(),
                    seq.ValueKind == JsonValueKind.Null ? null : seq.GetInt64(),
                    marks));
        }
    }

    private static (double, double) Pair(JsonElement element) =>
        (element[0].GetDouble(), element[1].GetDouble());

    private static string? Given(JsonElement vector)
    {
        var given = vector.GetProperty("given");
        return given.ValueKind == JsonValueKind.Null ? null : given.GetString();
    }
}
