using System.Globalization;
using System.Net;
using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// Looking things up, from this client's side (docs/protocol.md, "Looking things up"; issue #72): the providers named, the
/// owner's switch, the member's second consent and the writes it turns into, the statistics' searches, and the answers
/// whose links stay out of preview cards (design decision 7).
/// </summary>
public sealed class LookupsTests : IDisposable
{
    private const string Processor = "Microsoft — Azure OpenAI";
    private const long Me = 7;
    private const long AssistantId = 1;

    private static readonly IStringCatalog Say = EnglishCatalog.Instance;

    private static readonly string[] ThreeProviders = ["Brave Search", "Open-Meteo", "Wikipedia"];

    private readonly Database cache = Database.OpenInMemory();

    public void Dispose() => cache.Dispose();

    private static AssistantDto Assistant(string[]? lookups, string? processor = Processor) =>
        new(AssistantId, "Assistant", "@ai", Processor: processor, Lookups: lookups);

    private static ApiClient Api(Server server) =>
        new(new HttpClient(server), ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken"));

    private static MessageDto Message(long sender, string body, long chatId = 5, long? editSeq = 3) =>
        new(42, chatId, sender, null, body, "2026-10-03T09:30:00Z", EditSeq: editSeq);

    private const string Footer =
        "Snow showers tomorrow, around −2 °C.\n\n"
        + "Sources: [Tromsø – Wikipedia](https://en.wikipedia.org/wiki/Troms%C3%B8) · [Weather in Tromsø](https://example.org/tromso)\n"
        + "[Weather data by Open-Meteo.com](https://open-meteo.com/) · Wikipedia, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/) · Powered by Brave";

    // ---- the providers ------------------------------------------------------------------------

    /// <summary>A client that cannot name the providers does not ask: absent, empty and blank all name nobody.</summary>
    [Fact]
    public void ProvidersAreNamedOrNotOfferedAtAll()
    {
        Assert.Null(Lookups.Providers(null));
        Assert.Null(Lookups.Providers(Assistant(null)));
        Assert.Null(Lookups.Providers(Assistant([])));
        Assert.Null(Lookups.Providers(Assistant(["", "  "])));
        Assert.Equal(["SearXNG"], Lookups.Providers(Assistant([" SearXNG ", ""])));
        // The server's order, kept.
        Assert.Equal(ThreeProviders, Lookups.Providers(Assistant(ThreeProviders)));

        Assert.True(Lookups.Offered(Assistant(ThreeProviders)));
        Assert.False(Lookups.Offered(Assistant(null)));
        // The lookup consent stands on the assistant's, which a server naming no processor cannot ask for.
        Assert.False(Lookups.Offered(Assistant(ThreeProviders, processor: null)));
        Assert.False(Lookups.Offered(Assistant(ThreeProviders, processor: " ")));
    }

    [Fact]
    public void TheNamesAreJoinedTheWayTheLanguageJoinsThem()
    {
        Assert.Equal("", Lookups.Names([], Say));
        Assert.Equal("SearXNG", Lookups.Names(["SearXNG"], Say));
        Assert.Equal("Open-Meteo and Wikipedia", Lookups.Names(["Open-Meteo", "Wikipedia"], Say));
        Assert.Equal("Brave Search, Open-Meteo and Wikipedia", Lookups.Names(ThreeProviders, Say));
        // More than the server ever sends still names everybody.
        Assert.Equal("A, B, C and D", Lookups.Names(["A", "B", "C", "D"], Say));

        var japanese = JsonCatalog.For("ja");
        Assert.Equal(
            japanese.Format("%@, %@ and %@", "Brave Search", "Open-Meteo", "Wikipedia"),
            Lookups.Names(ThreeProviders, japanese));
        Assert.DoesNotContain(" and ", Lookups.Names(ThreeProviders, japanese), StringComparison.Ordinal);
        Assert.Equal("Open-Meteo und Wikipedia", Lookups.Names(["Open-Meteo", "Wikipedia"], JsonCatalog.For("de")));
    }

    // ---- what is said -------------------------------------------------------------------------

    /// <summary>
    /// One more line on the consent screen, naming the providers, picked by the owner's <c>ai_history</c> the way the two
    /// family-chat lines are — and no line at all where the server names none.
    /// </summary>
    [Fact]
    public void TheConsentScreenNamesTheProvidersInOneMoreLine()
    {
        var without = AssistantConsent.Disclosure(Processor, familyHistory: true, familyVision: false, Say);
        var with = AssistantConsent.Disclosure(Processor, familyHistory: true, familyVision: false, Say, lookups: ThreeProviders);
        Assert.Equal(without.Count + 1, with.Count);
        var line = Assert.Single(with, line => line.Contains("lookups", StringComparison.Ordinal));
        Assert.Equal(
            "If your family's owner turns on lookups, the assistant may send a short search query or place name it writes from your question — in the family chat, possibly from recent messages too — to Brave Search, Open-Meteo and Wikipedia, and its answer then lists its sources.",
            line);
        // Before where the answer lands and how to stop.
        Assert.Equal(with.Count - 3, with.ToList().IndexOf(line));

        var noHistory = AssistantConsent.Disclosure(Processor, familyHistory: false, familyVision: false, Say, lookups: ["SearXNG"]);
        Assert.Contains(
            "If your family's owner turns on lookups, the assistant may send a short search query or place name it writes from your question to SearXNG, and its answer then lists its sources.",
            noHistory);
        Assert.DoesNotContain(noHistory, text => text.Contains("recent messages", StringComparison.Ordinal));

        Assert.Equal(without, AssistantConsent.Disclosure(Processor, true, false, Say, lookups: []));
    }

    /// <summary>A member who already agreed to the assistant reads the lookup line and the way out, and nothing else.</summary>
    [Fact]
    public void TheLookupQuestionAloneIsTheLineAndTheWayOut()
    {
        var lines = Lookups.Disclosure(["SearXNG"], familyHistory: false, Say);
        Assert.Equal(2, lines.Count);
        Assert.Contains("to SearXNG,", lines[0], StringComparison.Ordinal);
        Assert.Equal("You can stop this at any time in Settings. What has already been sent cannot be taken back.", lines[1]);
    }

    [Fact]
    public void TheSwitchAndSettingsNameWhoWouldReceiveAQuery()
    {
        Assert.StartsWith(
            "With this on, the assistant can look things up when a question needs it — the weather, the news, a fact it isn't sure of — in Open-Meteo and Wikipedia.",
            Lookups.SwitchFootnote(["Open-Meteo", "Wikipedia"], Say));
        Assert.Equal(
            "Until you allow lookups, the assistant answers you from what it already knows, and nothing from your questions is sent to SearXNG.",
            Lookups.SettingsFootnote(["SearXNG"], allowed: false, Say));
        Assert.StartsWith(
            "The assistant may send a short search query or place name it writes from your questions to SearXNG.",
            Lookups.SettingsFootnote(["SearXNG"], allowed: true, Say));
        // Translated where the catalogue has it.
        Assert.Contains("SearXNG", Lookups.SettingsFootnote(["SearXNG"], allowed: false, JsonCatalog.For("ru")), StringComparison.Ordinal);
        Assert.DoesNotContain("Until", Lookups.SettingsFootnote(["SearXNG"], allowed: false, JsonCatalog.For("ru")), StringComparison.Ordinal);
    }

    // ---- the answer and its writes --------------------------------------------------------------

    [Fact]
    public void APressedButtonIsTheAnswerItSays()
    {
        Assert.Equal(ConsentAnswer.AgreeWithLookups, Lookups.AnswerFor(first: true, second: false, lookupsOffered: true));
        Assert.Equal(ConsentAnswer.Agree, Lookups.AnswerFor(first: false, second: true, lookupsOffered: true));
        Assert.Equal(ConsentAnswer.NotNow, Lookups.AnswerFor(first: false, second: false, lookupsOffered: true));
        // Without lookups the one yes is "I Agree", and there is no second button to have pressed.
        Assert.Equal(ConsentAnswer.Agree, Lookups.AnswerFor(first: true, second: false, lookupsOffered: false));
        Assert.Equal(ConsentAnswer.NotNow, Lookups.AnswerFor(first: false, second: true, lookupsOffered: false));
    }

    [Fact]
    public async Task NotNowWritesNothing()
    {
        var server = new Server();
        Assert.Null(await Lookups.RecordAsync(Api(server), ConsentAnswer.NotNow, assistantAgreed: false));
        Assert.Empty(server.Asked);
    }

    [Fact]
    public async Task AgreeingWithoutLookupsIsTheAssistantsConsentAlone()
    {
        var server = new Server().On("/me/assistant-consent", """{"assistant_consent_at": "2026-10-03T09:30:00Z"}""");
        Assert.Null(await Lookups.RecordAsync(Api(server), ConsentAnswer.Agree, assistantAgreed: false));
        Assert.Equal(["/api/v1/me/assistant-consent"], server.Asked);
        Assert.Equal(["{\"granted\":true}"], server.Bodies);
    }

    /// <summary>The lookup consent may only be granted on top of the assistant's, so it is written second.</summary>
    [Fact]
    public async Task AgreeingWithLookupsWritesBothInOrder()
    {
        var server = new Server()
            .On("/me/assistant-consent", """{"assistant_consent_at": "2026-10-03T09:30:00Z"}""")
            .On("/me/assistant-lookup-consent", """{"assistant_lookup_consent_at": "2026-10-03T09:30:00Z"}""");
        Assert.Null(await Lookups.RecordAsync(Api(server), ConsentAnswer.AgreeWithLookups, assistantAgreed: false));
        Assert.Equal(["/api/v1/me/assistant-consent", "/api/v1/me/assistant-lookup-consent"], server.Asked);
        Assert.Equal(["{\"granted\":true}", "{\"granted\":true}"], server.Bodies);
    }

    /// <summary>From Settings, a member who already agreed to the assistant is asked — and written — the lookups alone.</summary>
    [Fact]
    public async Task AnAssistantConsentAlreadyHeldIsNotWrittenAgain()
    {
        var server = new Server().On("/me/assistant-lookup-consent", """{"assistant_lookup_consent_at": "2026-10-03T09:30:00Z"}""");
        Assert.Null(await Lookups.RecordAsync(Api(server), ConsentAnswer.AgreeWithLookups, assistantAgreed: true));
        Assert.Equal(["/api/v1/me/assistant-lookup-consent"], server.Asked);
    }

    [Fact]
    public async Task AFailedFirstWriteIsNeverFollowedByTheSecond()
    {
        var server = new Server()
            .On("/me/assistant-consent", """{"error": {"code": "internal", "message": "x"}}""", HttpStatusCode.InternalServerError)
            .On("/me/assistant-lookup-consent", """{"assistant_lookup_consent_at": "2026-10-03T09:30:00Z"}""");
        var error = await Lookups.RecordAsync(Api(server), ConsentAnswer.AgreeWithLookups, assistantAgreed: false);
        Assert.NotNull(error);
        Assert.DoesNotContain("/api/v1/me/assistant-lookup-consent", server.Asked);
    }

    [Fact]
    public async Task AFailedSecondWriteIsReported()
    {
        var server = new Server()
            .On("/me/assistant-consent", """{"assistant_consent_at": "2026-10-03T09:30:00Z"}""")
            .On("/me/assistant-lookup-consent", """{"error": {"code": "not_found", "message": "no source"}}""", HttpStatusCode.NotFound);
        var error = await Lookups.RecordAsync(Api(server), ConsentAnswer.AgreeWithLookups, assistantAgreed: false);
        Assert.Equal("not_found", error!.Code);
    }

    /// <summary>The stamp is the server's, read with <c>/me</c>, and an older server's silence is no consent.</summary>
    [Fact]
    public async Task TheSessionCarriesTheServersLookupStamp()
    {
        const string me = """
            {"user": {"id": 7, "username": "anna", "display_name": "Anna"},
             "family": {"id": 3, "name": "The Smiths"}, "role": "member",
             "assistant_consent_at": "2026-09-19T08:12:04Z"
            """;
        var server = new Server().Then(
            "/me",
            (HttpStatusCode.OK, me + """, "assistant_lookup_consent_at": "2026-10-03T09:30:00Z"}"""),
            (HttpStatusCode.OK, me + "}"));
        var tokens = new MemoryTokenStore("t0ken");
        var session = new AppSession(
            new ApiClient(new HttpClient(server), ServerUrl.Normalise("chat.example.com")!, tokens), tokens, cache);

        Assert.Null(await session.RefreshAsync());
        Assert.Equal("2026-10-03T09:30:00Z", session.State.AssistantLookupConsentAt);
        Assert.True(Lookups.Allowed(session.State));

        Assert.Null(await session.RefreshAsync());
        Assert.Null(session.State.AssistantLookupConsentAt);
        Assert.False(Lookups.Allowed(session.State));
    }

    /// <summary>Settings offers the lookup agreement where the server names providers AND the member agreed to the assistant.</summary>
    [Fact]
    public void SettingsOffersLookupsOnTopOfTheAssistant()
    {
        var state = new SessionState(Gate.Member, Assistant: Assistant(ThreeProviders), AssistantConsentAt: "2026-09-19T08:12:04Z");
        Assert.True(Lookups.InSettings(state));
        Assert.False(Lookups.InSettings(state with { AssistantConsentAt = null }));
        Assert.False(Lookups.InSettings(state with { Assistant = Assistant(null) }));
        Assert.False(Lookups.Allowed(state));
        Assert.True(Lookups.Allowed(state with { AssistantLookupConsentAt = "2026-10-03T09:30:00Z" }));
    }

    // ---- the owner's switch -------------------------------------------------------------------

    /// <summary>Tied to no other switch: vision going off leaves it where it was, and it goes out as its own key.</summary>
    [Fact]
    public async Task TheOwnersSwitchIsTiedToNoOther()
    {
        var on = new FamilyDto(3, "The Smiths", AiVision: true, AiFaces: true, AiLookups: true);
        Assert.True(FamilyModel.AsApplied(on, new FamilyPatch { AiVision = false }).AiLookups);
        Assert.False(FamilyModel.AsApplied(on, new FamilyPatch { AiLookups = false }).AiLookups);
        Assert.True(FamilyModel.AsApplied(on with { AiLookups = false }, new FamilyPatch { AiLookups = true }).AiLookups);
        Assert.True(FamilyModel.MayTurnOn("ai_lookups", new FamilyDto(3, "The Smiths")));

        var server = new Server().On("/families/mine", """{"family": {"id": 3, "name": "The Smiths", "ai_lookups": true}}""");
        var family = new FamilyModel(Api(server), new ChatStore(cache, () => Me));
        var (changed, error) = await family.ChangeAsync(new FamilyDto(3, "The Smiths"), new FamilyPatch { AiLookups = true });
        Assert.Null(error);
        Assert.True(changed!.AiLookups);
        Assert.Equal("{\"ai_lookups\":true}", Assert.Single(server.Bodies));
    }

    // ---- statistics ---------------------------------------------------------------------------

    [Fact]
    public void AMembersLineCountsTheirWebSearches()
    {
        var culture = CultureInfo.InvariantCulture;
        var member = new StatsMemberDto(1, "Anna", 3, Ai: new StatsAiDto(2, Searches: 1));
        Assert.Equal("2 questions to the assistant · 1 web search", SettingsText.MemberLine(member, Say, culture));
        member = member with { Ai = new StatsAiDto(0, Searches: 5) };
        Assert.Equal("5 web searches", SettingsText.MemberLine(member, Say, culture));
        // Russian counts in three forms.
        var russian = JsonCatalog.For("ru");
        var one = SettingsText.MemberLine(member with { Ai = new StatsAiDto(0, Searches: 1) }, russian, culture);
        var few = SettingsText.MemberLine(member with { Ai = new StatsAiDto(0, Searches: 2) }, russian, culture);
        var many = SettingsText.MemberLine(member with { Ai = new StatsAiDto(0, Searches: 5) }, russian, culture);
        Assert.Equal(3, new[] { one, few, many }.Distinct().Count());
        Assert.Equal(many, SettingsText.MemberLine(member with { Ai = new StatsAiDto(0, Searches: 11) }, russian, culture).Replace("11", "5"));
        // None is not said.
        Assert.Equal("Words only", SettingsText.MemberLine(member with { Ai = new StatsAiDto(0) }, Say, culture));
    }

    [Fact]
    public void TheAssistantGroupShowsForSearchesAlone()
    {
        Assert.False(SettingsText.AssistantUsed(null));
        Assert.False(SettingsText.AssistantUsed(new StatsAiDto(0)));
        Assert.True(SettingsText.AssistantUsed(new StatsAiDto(0, Searches: 1)));
        Assert.True(SettingsText.AssistantUsed(new StatsAiDto(1)));
        Assert.True(SettingsText.AssistantUsed(new StatsAiDto(0, Images: 1)));
        Assert.True(SettingsText.AssistantUsed(new StatsAiDto(0, Transcripts: 1)));
    }

    // ---- the answer and its links ---------------------------------------------------------------

    /// <summary>
    /// The footer is plain markdown, and every link in it opens: the sources, Open-Meteo's credit and the licence — and
    /// nothing the server defanged becomes a link again.
    /// </summary>
    [Fact]
    public void TheFootersLinksAreTappable()
    {
        var links = BubbleBody.Lay(Footer, null, [], Me, _ => false)
            .OfType<BodyTextBlock>()
            .SelectMany(block => block.Runs)
            .Where(run => run.Link is not null)
            .Select(run => run.Link!)
            .Distinct()
            .ToList();
        Assert.Equal(
            [
                "https://en.wikipedia.org/wiki/Troms%C3%B8",
                "https://example.org/tromso",
                "https://open-meteo.com/",
                "https://creativecommons.org/licenses/by-sa/4.0/",
            ],
            links);
        Assert.All(links, link => Assert.NotNull(BubbleBody.Openable(link)));
        // The two lines stay two lines: the credit does not run on from the last source.
        var plain = string.Concat(BubbleBody.Lay(Footer, null, [], Me, _ => false).OfType<BodyTextBlock>().SelectMany(b => b.Runs).Select(r => r.Text));
        Assert.Contains("Weather in Tromsø\n", plain, StringComparison.Ordinal);

        var defanged = BubbleBody.Lay("Look at example[.]com for more.", null, [], Me, _ => false)
            .OfType<BodyTextBlock>().SelectMany(block => block.Runs);
        Assert.All(defanged, run => Assert.Null(run.Link));
    }

    /// <summary>
    /// A finished lookup answer draws no card: its links are its sources and credits, and a card would have every device
    /// that shows it contact a cited page. Every other message is as it was.
    /// </summary>
    [Fact]
    public void ALookupAnswerDrawsNoPreviewCard()
    {
        var answer = Message(AssistantId, Footer);
        Assert.NotNull(BubbleBody.PreviewLink(Footer, emojiOnly: false));
        Assert.False(Lookups.MayPreview(answer, Footer, Me, assistantChat: false, AssistantId, stillWriting: false));
        // In the member's own assistant chat, where the assistant is whoever is not the reader.
        Assert.False(Lookups.MayPreview(Message(99, Footer), Footer, Me, assistantChat: true, AssistantId, stillWriting: false));

        // An answer that looked nothing up keeps its card, as before.
        const string plain = "The docs are at https://example.com/docs";
        Assert.True(Lookups.MayPreview(Message(AssistantId, plain), plain, Me, false, AssistantId, stillWriting: false));
        // A MEMBER's message that happens to look like a footer is a member's message.
        Assert.True(Lookups.MayPreview(Message(11, Footer), Footer, Me, assistantChat: false, AssistantId, stillWriting: false));
        Assert.True(Lookups.MayPreview(Message(Me, Footer), Footer, Me, assistantChat: true, AssistantId, stillWriting: false));
    }

    /// <summary>
    /// An answer still being written is the model's raw words — the server filters links out of the finished body, not
    /// out of the stream — so it draws no card until the finished row replaces them.
    /// </summary>
    [Fact]
    public void AnAnswerStillBeingWrittenDrawsNoCard()
    {
        const string raw = "Here: https://evil.example/?q=family+secrets";
        var streaming = Message(AssistantId, string.Empty, editSeq: null);
        Assert.False(Lookups.MayPreview(streaming, raw, Me, false, AssistantId, stillWriting: true));
        Assert.True(Lookups.MayPreview(streaming with { Body = raw, EditSeq = 4 }, raw, Me, false, AssistantId, stillWriting: false));
        // A member's message is never "being written" by the assistant.
        Assert.True(Lookups.MayPreview(Message(11, raw), raw, Me, false, AssistantId, stillWriting: true));
    }

    [Fact]
    public void WritingLastsUntilTheFinishedRowLands()
    {
        var answers = new AssistantAnswers();
        var held = Message(AssistantId, string.Empty, editSeq: null);
        Assert.False(answers.IsWriting(held));
        answers.Delta(held.ChatId, held.Id, "Here: https://", held);
        Assert.True(answers.IsWriting(held));
        // Stopped part-way: its words are never replaced, so it stays the raw stream.
        answers.Stopped(held.ChatId, held.Id);
        Assert.True(answers.IsWriting(held));
        var finished = held with { Body = Footer, EditSeq = 9 };
        answers.Finished(finished);
        Assert.False(answers.IsWriting(finished));
        Assert.False(answers.IsWriting(held));
    }
}
