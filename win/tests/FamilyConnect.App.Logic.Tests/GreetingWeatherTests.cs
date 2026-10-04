using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// Today's weather in the daily greeting, from this client's side (docs/protocol.md, "Today's weather, for places the
/// owner chose"; issue #72): who sees the places field and when it may be edited, the fields as they are edited and how
/// they follow the server, the request a save makes — and the credit line under a greeting, drawn as a link that opens.
/// </summary>
public sealed class GreetingWeatherTests : IDisposable
{
    private const long Me = 7;
    private const long AssistantId = 1;

    private readonly Database cache = Database.OpenInMemory();

    public void Dispose() => cache.Dispose();

    private static AssistantDto Assistant(bool weather) =>
        new(AssistantId, "Assistant", "@ai", Processor: "Azure OpenAI", GreetingWeather: weather);

    private static FamilyDto Smiths(bool greeting = true, string[]? places = null) =>
        new(3, "The Smiths", "open", AiGreeting: greeting, GreetingPlaces: places);

    private static SessionState State(Gate gate, AssistantDto? assistant, FamilyDto? family = null) =>
        new(gate, new UserDto(Me, "anna", "Anna"), family ?? Smiths(), assistant, GreetingsEnabled: true);

    // ---- who sees it, and when it may be changed -----------------------------------------------

    /// <summary>
    /// The owner's field, on a server that says it can — like every other AI setting of the family, which a member's screen
    /// does not draw at all. An older server sends no <c>greeting_weather</c>, which reads as false.
    /// </summary>
    [Fact]
    public void TheFieldIsTheOwnersAndOnlyWhereTheServerOffersIt()
    {
        Assert.True(GreetingWeather.Shown(State(Gate.Owner, Assistant(weather: true))));
        Assert.False(GreetingWeather.Shown(State(Gate.Member, Assistant(weather: true))));
        Assert.False(GreetingWeather.Shown(State(Gate.Owner, Assistant(weather: false))));
        Assert.False(GreetingWeather.Shown(State(Gate.Owner, assistant: null)));

        var older = Wire.Decode<AssistantDto>("""{"user_id": 1, "display_name": "Assistant", "mention": "@ai"}""");
        Assert.False(GreetingWeather.Offered(older));
        Assert.True(GreetingWeather.Offered(
            Wire.Decode<AssistantDto>("""{"user_id": 1, "display_name": "Assistant", "mention": "@ai", "greeting_weather": true}""")));
        // Tied to no family switch: lookups off, places still offered.
        Assert.True(GreetingWeather.Shown(State(Gate.Owner, Assistant(weather: true), Smiths() with { AiLookups = false })));
    }

    /// <summary>
    /// Editable with the greeting on or off — the owner may choose the places before turning it on, as on iOS, Android
    /// and the web (protocol.md) — and only not while another change of the family's is on its way.
    /// </summary>
    [Fact]
    public void ThePlacesAreEditableWithTheGreetingOnOrOffWhileNothingElseIsSaving()
    {
        Assert.True(GreetingWeather.Editable(idle: true));
        Assert.False(GreetingWeather.Editable(idle: false));
    }

    [Fact]
    public void TheSavedListIsTheServersAndAnAbsentOneIsNone()
    {
        Assert.Empty(GreetingWeather.Saved(null));
        Assert.Empty(GreetingWeather.Saved(Smiths(places: null)));
        Assert.Equal(["Moscow", "Belgrade"], GreetingWeather.Saved(Smiths(places: ["Moscow", "Belgrade"])));
        Assert.Equal(["Moscow"], GreetingWeather.Saved(Wire.Decode<FamilyDto>(
            """{"id": 3, "name": "The Smiths", "greeting_places": ["Moscow", null]}""")));
        Assert.Null(Wire.Decode<FamilyDto>("""{"id": 3, "name": "The Smiths"}""")!.GreetingPlaces);
    }

    // ---- the fields ----------------------------------------------------------------------------

    [Fact]
    public void AtMostThreeFieldsAreListed()
    {
        var draft = new PlacesDraft();
        Assert.True(draft.CanAdd);
        Assert.True(draft.Add());
        Assert.True(draft.Add());
        Assert.True(draft.Add());
        Assert.False(draft.CanAdd);
        Assert.False(draft.Add());
        Assert.Equal(3, draft.Fields.Count);

        Assert.False(draft.Remove(3));
        Assert.False(draft.Remove(-1));
        Assert.True(draft.Remove(0));
        Assert.True(draft.CanAdd);
    }

    /// <summary>Nothing is sent that changes nothing, a blank field is no place, and a field is typed into as the server keeps it.</summary>
    [Fact]
    public void OnlyARealChangeIsSent()
    {
        var draft = new PlacesDraft();
        draft.Sync(["Moscow"]);
        Assert.Equal(["Moscow"], draft.Fields);
        Assert.Null(draft.Pending());

        // An empty field added and left is not a change.
        draft.Add();
        Assert.Null(draft.Pending());
        Assert.Equal("Bel grade", draft.Set(1, "Bel\u0001 grade"));
        draft.Set(1, "  Belgrade  ");
        Assert.Equal(["Moscow", "Belgrade"], draft.Pending());
        // Whitespace only: the same list once folded.
        draft.Set(0, "  Moscow ");
        draft.Set(1, string.Empty);
        Assert.Null(draft.Pending());

        // Clearing every field is a change: [] clears the list.
        draft.Set(0, " ");
        var cleared = draft.Pending();
        Assert.NotNull(cleared);
        Assert.Empty(cleared);

        // A control character typed or pasted is taken out as it is typed; the field never holds one to refuse.
        Assert.Equal("Moscow", draft.Set(0, "Mos\u0000cow"));
        Assert.Null(draft.Pending());

        // A pasted name padded with whitespace the server folds away is not cut for it: the server keeps "Paris".
        var padded = new string(' ', 79) + "Paris";
        Assert.Equal(padded, draft.Set(0, padded));
        Assert.Equal(["Paris"], draft.Pending());
    }

    /// <summary>
    /// A redraw is not a reset: the server's list replaces the fields only when it CHANGED and nothing unsaved is in them,
    /// and an empty field the owner has just added survives the redraws in between.
    /// </summary>
    [Fact]
    public void TheServersListFollowsWithoutThrowingAwayTyping()
    {
        var draft = new PlacesDraft();
        draft.Sync(["Moscow"]);
        draft.Add();
        draft.Sync(["Moscow"]);
        Assert.Equal(["Moscow", ""], draft.Fields);

        // Another device changed it, and nothing here is unsaved: it is taken — the added field still at the end.
        draft.Sync(["Paris"]);
        Assert.Equal(["Paris", ""], draft.Fields);

        // Somebody is typing: a new list from elsewhere does not overwrite them, and becomes what they are measured against.
        draft.Set(1, "Rome");
        draft.Sync(["Paris", "Oslo"]);
        Assert.Equal(["Paris", "Rome"], draft.Fields);
        Assert.Equal(["Paris", "Oslo"], draft.Basis);
        Assert.Equal(["Paris", "Rome"], draft.Pending());
    }

    /// <summary>
    /// After a save the fields are the list the server KEPT — shorter or respelt — unless the owner changed them while the
    /// save was on its way, which is then the next save.
    /// </summary>
    [Fact]
    public void TheFieldsBecomeWhatTheServerKept()
    {
        var draft = new PlacesDraft();
        draft.Add();
        draft.Set(0, "Moscow");
        draft.Add();
        draft.Set(1, "moscow");
        var sent = draft.Pending()!;
        Assert.Equal(["Moscow", "moscow"], sent);
        draft.Adopt(sent, ["Moscow"]);
        Assert.Equal(["Moscow"], draft.Fields);
        Assert.Null(draft.Pending());

        // Typed into while the save was away: kept, and measured against what the server kept.
        draft.Add();
        draft.Set(1, "Oslo");
        var second = draft.Pending()!;
        draft.Set(1, "Oslo, Norway");
        draft.Adopt(second, ["Moscow", "Oslo"]);
        Assert.Equal(["Moscow", "Oslo, Norway"], draft.Fields);
        Assert.Equal(["Moscow", "Oslo, Norway"], draft.Pending());

        // A server that predates the key keeps nothing, and the screen says so rather than showing what was sent.
        var third = draft.Pending()!;
        draft.Adopt(third, []);
        Assert.Empty(draft.Fields);
    }

    // ---- the request ---------------------------------------------------------------------------

    /// <summary>A save is ONE key, the whole list, through the family's own PATCH; the answer's list is what was kept.</summary>
    [Fact]
    public async Task ASaveSendsTheWholeListAndReadsBackWhatWasKept()
    {
        var chats = new ChatStore(cache, () => Me);
        var server = new Server().On("/families/mine", """
            {"family": {"id": 3, "name": "The Smiths", "ai_greeting": true, "greeting_places": ["Moscow", "Belgrade"]}}
            """);
        var family = new FamilyModel(
            new ApiClient(new HttpClient(server), ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken")),
            chats);

        var draft = new PlacesDraft();
        draft.Sync([]);
        draft.Add();
        draft.Set(0, " Moscow ");
        draft.Add();
        draft.Set(1, "Belgrade");
        var request = draft.Pending()!;
        var (answered, error) = await family.ChangeAsync(Smiths(), new FamilyPatch { GreetingPlaces = request });

        Assert.Null(error);
        Assert.Equal("{\"greeting_places\":[\"Moscow\",\"Belgrade\"]}", Assert.Single(server.Bodies));
        draft.Adopt(request, GreetingWeather.Saved(answered));
        Assert.Equal(["Moscow", "Belgrade"], draft.Fields);

        // Drawn as the server will apply it while the save is away: the list replaced, nothing else touched.
        var applied = FamilyModel.AsApplied(Smiths(places: ["Paris"]), new FamilyPatch { GreetingPlaces = ["Oslo"] });
        Assert.Equal(["Oslo"], applied.GreetingPlaces!);
        Assert.True(applied.AiGreeting);
        Assert.Equal(["Paris"], FamilyModel.AsApplied(Smiths(places: ["Paris"]), new FamilyPatch { AiGreeting = false }).GreetingPlaces!);
    }

    // ---- the greeting itself ---------------------------------------------------------------------

    /// <summary>
    /// The credit under a greeting that used the weather is plain markdown the server writes: drawn as a link that opens
    /// (a Hyperlink over <see cref="BubbleBody.Openable"/>, as every bubble link is), in each of the nine languages' words as server/src/lookups.rs writes them —
    /// and, being the lookups' credit line, it keeps the greeting out of a preview card.
    /// </summary>
    [Theory]
    [InlineData("Weather data by Open-Meteo.com")]
    [InlineData("Wetterdaten von Open-Meteo.com")]
    [InlineData("Datos meteorológicos de Open-Meteo.com")]
    [InlineData("Données météo par Open-Meteo.com")]
    [InlineData("気象データ: Open-Meteo.com")]
    [InlineData("Данные о погоде: Open-Meteo.com")]
    [InlineData("Подаци о времену: Open-Meteo.com")]
    [InlineData("Podaci o vremenu: Open-Meteo.com")]
    [InlineData("天气数据：Open-Meteo.com")]
    public void TheGreetingsWeatherCreditIsALinkThatOpens(string words)
    {
        var greeting = "Good morning! Moscow, Russia: sunny, 18 °C at most. ♌\n\n"
            + $"[{words}](https://open-meteo.com/)";
        var runs = BubbleBody.Lay(greeting, null, [], Me, _ => false).OfType<BodyTextBlock>().SelectMany(block => block.Runs).ToList();
        var credit = Assert.Single(runs, run => run.Link is not null);
        Assert.Equal(words, credit.Text);
        Assert.Equal(new Uri("https://open-meteo.com/"), BubbleBody.Openable(credit.Link));

        Assert.True(SourcesFooter.EndsWithFooter(greeting));
        var message = new MessageDto(42, 5, AssistantId, null, greeting, "2026-10-03T07:00:00Z");
        Assert.False(Lookups.MayPreview(message, greeting, Me, assistantChat: false, AssistantId, stillWriting: false));
    }
}
