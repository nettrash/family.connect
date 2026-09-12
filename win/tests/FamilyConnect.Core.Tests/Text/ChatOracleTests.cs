using System.Text.Json;
using FamilyConnect.Core;

namespace FamilyConnect.Core.Tests.Text;

/// <summary>
/// THE SECOND DIFFERENTIAL ORACLE: the words a chat ROW is drawn with. Every vector in
/// <c>Fixtures/chat-vectors.json</c> was produced by <c>fc_text</c> itself — the Rust the web
/// client runs — and this suite holds the C# port to it, string for string.
/// </summary>
/// <remarks>
/// Regenerate with <c>cd win/tools/board-oracle &amp;&amp; cargo run --quiet -- chat &gt;
/// ../../tests/FamilyConnect.Core.Tests/Fixtures/chat-vectors.json</c>. The generator depends on
/// <c>web/text</c> BY PATH, so it cannot drift from the source it claims to speak for.
/// </remarks>
public class ChatOracleTests
{
    private static readonly JsonDocument Vectors = JsonDocument.Parse(
        File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "Fixtures", "chat-vectors.json")));

    private static JsonElement Section(string name) => Vectors.RootElement.GetProperty(name);

    private static long? Optional(JsonElement row, string name) =>
        row.GetProperty(name).ValueKind == JsonValueKind.Null
            ? null
            : row.GetProperty(name).GetInt64();

    /// <summary>
    /// A record's line, for every outcome the wire names and one it does not, at every duration
    /// and on both sides of the call. 120 vectors, because "whose call was it" changes four of
    /// the seven sentences.
    /// </summary>
    [Fact]
    public void ACallRecordSaysWhatTheOriginalSays()
    {
        var vectors = Section("call_record");
        Assert.NotEmpty(vectors.EnumerateArray());
        foreach (var row in vectors.EnumerateArray())
        {
            var outcome = row.GetProperty("outcome").GetString()!;
            var video = row.GetProperty("video").GetBoolean();
            var mine = row.GetProperty("mine").GetBoolean();
            Assert.Equal(
                row.GetProperty("said").GetString(),
                CallRecordText.Label(outcome, Optional(row, "duration_secs"), video, mine));
        }
    }

    [Fact]
    public void ADurationIsClockedTheSameWay()
    {
        foreach (var row in Section("duration").EnumerateArray())
        {
            Assert.Equal(
                row.GetProperty("said").GetString(),
                CallRecordText.Duration(row.GetProperty("seconds").GetInt64()));
        }
    }

    [Fact]
    public void AnAttachmentWithNoNameIsCalledWhatTheOriginalCallsIt()
    {
        foreach (var row in Section("display_name").EnumerateArray())
        {
            var name = row.GetProperty("name").ValueKind == JsonValueKind.Null
                ? null
                : row.GetProperty("name").GetString();
            Assert.Equal(
                row.GetProperty("said").GetString(),
                AttachmentText.DisplayName(row.GetProperty("kind").GetString()!, name));
        }
    }

    [Fact]
    public void ANotificationsTitleReadsTheSame()
    {
        foreach (var row in Section("notify_title").EnumerateArray())
        {
            var family = row.GetProperty("family").ValueKind == JsonValueKind.Null
                ? null
                : row.GetProperty("family").GetString();
            Assert.Equal(
                row.GetProperty("said").GetString(),
                NotifyText.Title(
                    family,
                    row.GetProperty("sender").GetString()!,
                    row.GetProperty("mentioned").GetBoolean()));
        }
    }

    [Fact]
    public void TheWindowTitleCountsTheSameWay()
    {
        foreach (var row in Section("page_title").EnumerateArray())
        {
            Assert.Equal(
                row.GetProperty("said").GetString(),
                NotifyText.WindowTitle("Family Connect", row.GetProperty("unread").GetInt64()));
        }
    }

    /// <summary>
    /// THE `.ics` FILE, BYTE FOR BYTE. Nothing about it is on the wire, which is exactly why four
    /// clients writing it four ways would diverge in silence — and the three rules that are easy
    /// to get wrong (CRLF, escaping, folding at 75 OCTETS without splitting a UTF-8 sequence) are
    /// invisible until a calendar refuses the file.
    /// </summary>
    [Fact]
    public void AnEventsCalendarFileIsTheOriginalsByteForByte()
    {
        var vectors = Section("ics");
        Assert.NotEmpty(vectors.EnumerateArray());
        foreach (var row in vectors.EnumerateArray())
        {
            string? Optional(string name) =>
                row.GetProperty(name).ValueKind == JsonValueKind.Null
                    ? null
                    : row.GetProperty(name).GetString();
            Assert.Equal(
                row.GetProperty("file").GetString(),
                Calendar.OneEvent(
                    row.GetProperty("uid").GetString()!,
                    row.GetProperty("title").GetString()!,
                    row.GetProperty("starts_at").GetString()!,
                    Optional("ends_at"),
                    Optional("place"),
                    "20260912T120000Z"));
        }
    }

    [Fact]
    public void AnInstantIsStampedTheWayICalendarWantsIt()
    {
        // The wire carries an instant; a calendar file wants UTC and says so with the Z.
        Assert.Equal(
            "20261224T160000Z",
            Calendar.Stamp(new DateTimeOffset(2026, 12, 24, 19, 0, 0, TimeSpan.FromHours(3))));
    }

    [Fact]
    public void TheNotificationBodiesAreTheOriginals()
    {
        var bodies = Section("bodies");
        Assert.Equal(bodies.GetProperty("new_message").GetString(), NotifyText.NewMessage());
        Assert.Equal(bodies.GetProperty("new_note").GetString(), NotifyText.NewNote());
    }
}
