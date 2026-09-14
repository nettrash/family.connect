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

    /// <summary>A file's size, at every threshold where the unit or the rounding changes.</summary>
    [Fact]
    public void AFileSizeReadsAsTheOriginalReads()
    {
        var vectors = Section("display_size");
        Assert.NotEmpty(vectors.EnumerateArray());
        foreach (var row in vectors.EnumerateArray())
        {
            Assert.Equal(
                row.GetProperty("said").GetString(),
                MediaText.DisplaySize(row.GetProperty("bytes").GetInt64(), culture: System.Globalization.CultureInfo.InvariantCulture));
        }
    }

    /// <summary>A size is for a person: the reader's decimal separator, the same arithmetic.</summary>
    [Fact]
    public void AFileSizeUsesTheReadersDecimalSeparator()
    {
        var german = System.Globalization.CultureInfo.GetCultureInfo("de-DE");
        Assert.Equal("1,2 MB", MediaText.DisplaySize(1_250_000, culture: german));
        Assert.Equal("1,23 GB", MediaText.DisplaySize(1_234_567_890, culture: german));
        Assert.Equal("10 GB", MediaText.DisplaySize(10_000_000_000, culture: german));
    }

    /// <summary>Tiles and album cards, from metadata alone — including sizes the uploader could not give.</summary>
    [Fact]
    public void AShapeIsMeasuredAsTheOriginalMeasuresIt()
    {
        var vectors = Section("shapes");
        Assert.NotEmpty(vectors.EnumerateArray());
        int? Size(JsonElement row, string name) =>
            row.GetProperty(name).ValueKind == JsonValueKind.Null ? null : row.GetProperty(name).GetInt32();
        foreach (var row in vectors.EnumerateArray())
        {
            var (width, height) = (Size(row, "width"), Size(row, "height"));
            Assert.Equal(row.GetProperty("aspect").GetDouble(), MediaText.AspectRatio(width, height));
            var tile = MediaText.TileSize(width, height);
            Assert.Equal(row.GetProperty("tile")[0].GetDouble(), tile.Width);
            Assert.Equal(row.GetProperty("tile")[1].GetDouble(), tile.Height);
            var card = MediaText.CardSize(width, height, 300);
            Assert.Equal(row.GetProperty("card")[0].GetDouble(), card.Width);
            Assert.Equal(row.GetProperty("card")[1].GetDouble(), card.Height);
            var odd = MediaText.CardSize(width, height, 250.625);
            Assert.Equal(row.GetProperty("card_odd")[0].GetDouble(), odd.Width);
            Assert.Equal(row.GetProperty("card_odd")[1].GetDouble(), odd.Height);
        }
    }

    [Fact]
    public void WhatIsLookedAtIsWhatTheOriginalLooksAt()
    {
        foreach (var row in Section("is_media").EnumerateArray())
        {
            Assert.Equal(row.GetProperty("is_media").GetBoolean(), MediaText.IsMedia(row.GetProperty("kind").GetString()!));
        }
    }

    /// <summary>
    /// A place: coordinates always with a POINT, the accuracy only when it is a number, and the Maps
    /// link percent-encoded byte by byte with the label trimmed — or "Location" when there is none.
    /// </summary>
    [Fact]
    public void APlaceReadsAndLinksAsTheOriginalDoes()
    {
        var vectors = Section("locations");
        Assert.NotEmpty(vectors.EnumerateArray());
        foreach (var row in vectors.EnumerateArray())
        {
            var latitude = row.GetProperty("latitude").GetDouble();
            var longitude = row.GetProperty("longitude").GetDouble();
            double? accuracy = row.GetProperty("accuracy_nan").GetBoolean()
                ? double.NaN
                : row.GetProperty("accuracy_m").ValueKind == JsonValueKind.Null ? null : row.GetProperty("accuracy_m").GetDouble();
            var name = row.GetProperty("name").ValueKind == JsonValueKind.Null ? null : row.GetProperty("name").GetString();
            Assert.Equal(row.GetProperty("line").GetString(), MediaText.LocationLine(latitude, longitude, accuracy));
            Assert.Equal(row.GetProperty("maps").GetString(), MediaText.MapsUrl(latitude, longitude, name));
        }
    }

    private static string? Nullable(JsonElement row, string name) =>
        row.GetProperty(name).ValueKind == JsonValueKind.Null ? null : row.GetProperty(name).GetString();

    [Fact]
    public void AFilesTypeIsReadAsTheOriginalReadsIt()
    {
        var vectors = Section("prep_types");
        Assert.NotEmpty(vectors.EnumerateArray());
        foreach (var row in vectors.EnumerateArray())
        {
            var mime = row.GetProperty("mime").GetString()!;
            var name = row.GetProperty("name").GetString()!;
            Assert.Equal(row.GetProperty("essence").GetString(), MediaPrep.Essence(mime));
            Assert.Equal(row.GetProperty("extension").GetString(), MediaPrep.Extension(name));
            Assert.Equal(row.GetProperty("mime_for").GetString(), MediaPrep.MimeFor(name));
            Assert.Equal(row.GetProperty("declared").GetString(), MediaPrep.DeclaredType(mime, name));
            Assert.Equal(Nullable(row, "audio"), MediaPrep.AudioMime(MediaPrep.Essence(mime), name));
        }
    }

    private static Dictionary<string, byte[]> Heads() =>
        Section("prep_heads").EnumerateArray().ToDictionary(
            row => row.GetProperty("label").GetString()!,
            row => Convert.FromHexString(row.GetProperty("hex").GetString()!));

    /// <summary>
    /// Which kind a picked file goes as — a video or a recording only when the server will take its
    /// BYTES as one — for sixteen picks against nine heads.
    /// </summary>
    [Fact]
    public void APickedFileTakesTheRouteTheOriginalGivesIt()
    {
        var heads = Heads();
        var vectors = Section("prep_route");
        Assert.NotEmpty(vectors.EnumerateArray());
        foreach (var row in vectors.EnumerateArray())
        {
            var routed = MediaPrep.Route(
                row.GetProperty("mime").GetString()!, row.GetProperty("name").GetString()!,
                heads[row.GetProperty("head").GetString()!]);
            var said = routed.Route switch
            {
                MediaRoute.Photo => "photo",
                MediaRoute.Video => "video",
                MediaRoute.Audio => $"audio:{routed.AudioMime}",
                _ => "file",
            };
            Assert.Equal(row.GetProperty("route").GetString(), said);
        }
    }

    [Fact]
    public void TheMagicNumbersAreTheServersOwn()
    {
        var heads = Heads();
        foreach (var row in Section("prep_magic").EnumerateArray())
        {
            Assert.Equal(
                row.GetProperty("matches").GetBoolean(),
                MediaPrep.MatchesMagic(row.GetProperty("mime").GetString()!, heads[row.GetProperty("head").GetString()!]));
        }
    }

    /// <summary>
    /// A name for somebody else's disk: separators and control or format characters replaced, trimmed,
    /// and a long one cut in its STEM so the extension survives.
    /// </summary>
    [Fact]
    public void ANameIsCleanedAsTheOriginalCleansIt()
    {
        var vectors = Section("prep_names");
        Assert.NotEmpty(vectors.EnumerateArray());
        foreach (var row in vectors.EnumerateArray())
        {
            Assert.Equal(Nullable(row, "clean"), MediaPrep.SanitizedName(row.GetProperty("raw").GetString()!));
        }
    }

    [Fact]
    public void APictureIsFittedAsTheOriginalFitsIt()
    {
        foreach (var row in Section("prep_fit").EnumerateArray())
        {
            var fit = MediaPrep.FitWithin(
                row.GetProperty("width").GetUInt32(), row.GetProperty("height").GetUInt32(), row.GetProperty("edge").GetUInt32());
            Assert.Equal(row.GetProperty("fit")[0].GetUInt32(), fit.Width);
            Assert.Equal(row.GetProperty("fit")[1].GetUInt32(), fit.Height);
        }
    }

    [Fact]
    public void PluralCategoriesAreTheOriginals()
    {
        var vectors = Section("plural_categories");
        Assert.NotEmpty(vectors.EnumerateArray());
        foreach (var row in vectors.EnumerateArray())
        {
            Assert.Equal(
                row.GetProperty("category").GetString(),
                PluralRules.Category(row.GetProperty("lang").GetString()!, row.GetProperty("count").GetInt64()));
        }
    }

    [Fact]
    public void AProfilePictureIsTheOriginalsSquare()
    {
        var vectors = Section("avatar_square");
        Assert.NotEmpty(vectors.EnumerateArray());
        foreach (var row in vectors.EnumerateArray())
        {
            var square = AvatarPrep.Square(row.GetProperty("width").GetUInt32(), row.GetProperty("height").GetUInt32());
            var said = row.GetProperty("square");
            if (said.ValueKind == JsonValueKind.Null)
            {
                Assert.Null(square);
                continue;
            }
            Assert.Equal(
                new AvatarSquare(said.GetProperty("x").GetUInt32(), said.GetProperty("y").GetUInt32(),
                    said.GetProperty("side").GetUInt32(), said.GetProperty("edge").GetUInt32()),
                square);
        }
        var budget = Section("avatar_budget");
        Assert.Equal(budget.GetProperty("edge").GetUInt32(), AvatarPrep.Edge);
        Assert.Equal(budget.GetProperty("max_bytes").GetInt32(), AvatarPrep.MaxBytes);
        Assert.Equal(budget.GetProperty("qualities").EnumerateArray().Select(q => q.GetDouble()), AvatarPrep.Qualities);
    }
}
