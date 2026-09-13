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

    private static List<FamilyConnect.Core.Protocol.ReactionDto> ReactionList(JsonElement array) =>
        array.EnumerateArray()
            .Select(item => new FamilyConnect.Core.Protocol.ReactionDto(
                item.GetProperty("user_id").GetInt64(), item.GetProperty("emoji").GetString()!))
            .ToList();

    /// <summary>
    /// The chips under a bubble: first-seen order, counting everybody, marking the reader — for
    /// six lists including two spellings of one letter that are equal only canonically (they are
    /// two chips here, as on the server and Android).
    /// </summary>
    [Fact]
    public void ReactionChipsAreGroupedAsTheOriginalGroupsThem()
    {
        var vectors = Section("reaction_chips");
        Assert.NotEmpty(vectors.EnumerateArray());
        foreach (var row in vectors.EnumerateArray())
        {
            var chips = Reactions.Chips(ReactionList(row.GetProperty("reactions")), row.GetProperty("me").GetInt64());
            var expected = row.GetProperty("chips").EnumerateArray().ToList();
            Assert.Equal(expected.Count, chips.Count);
            for (var at = 0; at < chips.Count; at++)
            {
                Assert.Equal(expected[at].GetProperty("emoji").GetString(), chips[at].Emoji);
                Assert.Equal(expected[at].GetProperty("count").GetInt32(), chips[at].Count);
                Assert.Equal(expected[at].GetProperty("includes_me").GetBoolean(), chips[at].IncludesMe);
            }
        }
    }

    /// <summary>
    /// "See who reacted": "You" first, names or "Someone", and a BLOCKED reactor left out of the
    /// names while the chip still counts them.
    /// </summary>
    [Fact]
    public void WhoReactedReadsAsTheOriginalReads()
    {
        var vectors = Section("reaction_details");
        Assert.NotEmpty(vectors.EnumerateArray());
        string? NameOf(long id) => id switch { 9 => "Anna", 11 => "Bob", _ => null };
        foreach (var row in vectors.EnumerateArray())
        {
            var blocked = row.GetProperty("blocked").EnumerateArray().Select(id => id.GetInt64()).ToHashSet();
            var details = Reactions.Details(
                ReactionList(row.GetProperty("reactions")), NameOf, row.GetProperty("me").GetInt64(), blocked);
            var expected = row.GetProperty("details").EnumerateArray().ToList();
            Assert.Equal(expected.Count, details.Count);
            for (var at = 0; at < details.Count; at++)
            {
                Assert.Equal(expected[at].GetProperty("emoji").GetString(), details[at].Emoji);
                Assert.Equal(
                    expected[at].GetProperty("names").EnumerateArray().Select(name => name.GetString()!),
                    details[at].Names);
                Assert.Equal(Optional(expected[at], "lead_user_id"), details[at].LeadUserId);
            }
        }
    }

    /// <summary>A tap on the emoji already held removes it; any other appends it as the newest.</summary>
    [Fact]
    public void AToggleRewritesTheListAsTheOriginalDoes()
    {
        var vectors = Section("reaction_toggle");
        Assert.NotEmpty(vectors.EnumerateArray());
        foreach (var row in vectors.EnumerateArray())
        {
            var toggled = Reactions.Toggle(
                ReactionList(row.GetProperty("reactions")), row.GetProperty("me").GetInt64(),
                row.GetProperty("emoji").GetString()!);
            Assert.Equal(row.GetProperty("removing").GetBoolean(), toggled.Removing);
            Assert.Equal(ReactionList(row.GetProperty("after")), toggled.Reactions);
        }
    }

    [Fact]
    public void TheCapsuleOffersWhatTheOriginalOffers()
    {
        foreach (var row in Section("capsule").EnumerateArray())
        {
            var mine = row.GetProperty("mine").ValueKind == JsonValueKind.Null ? null : row.GetProperty("mine").GetString();
            Assert.Equal(
                row.GetProperty("emojis").EnumerateArray().Select(emoji => emoji.GetString()!),
                Reactions.Capsule(mine));
        }
    }

    /// <summary>
    /// The reply quote is cut per SCALAR, as the server cuts it — so a cut may land inside a family
    /// emoji, and a client cutting per grapheme would quote a different length than the server.
    /// </summary>
    [Fact]
    public void AQuoteIsCutWhereTheServerCutsIt()
    {
        var vectors = Section("excerpt");
        Assert.NotEmpty(vectors.EnumerateArray());
        foreach (var row in vectors.EnumerateArray())
        {
            Assert.Equal(row.GetProperty("excerpt").GetString(), Excerpt.Cut(row.GetProperty("body").GetString()!));
        }
    }
}
