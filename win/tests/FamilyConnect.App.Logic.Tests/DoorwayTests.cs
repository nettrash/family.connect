using System.Globalization;
using System.Net;
using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// The door: which server, who you are, which family — and the one sentence each refusal becomes.
/// </summary>
public class DoorwayTests : IDisposable
{
    private static readonly Uri Home = ServerUrl.Normalise("chat.example.com")!;
    private static readonly IStringCatalog Say = EnglishCatalog.Instance;

    private const string Refusal =
        """{"error": {"code": "unauthorized", "message": "a token is required"}}""";

    private const string MePending =
        """
        {"user": {"id": 7, "username": "anna", "display_name": "Anna"},
         "pending_join_request": {"family_id": 3, "family_name": "The Smiths"}}
        """;

    private const string MeOwner =
        """
        {"user": {"id": 7, "username": "anna", "display_name": "Anna"},
         "family": {"id": 3, "name": "The Smiths"}, "role": "owner"}
        """;

    private readonly Database cache = Database.OpenInMemory();

    public void Dispose() => cache.Dispose();

    private static Task<bool> Probe(Server server) =>
        ServerCheck.AnswersAsync(new HttpClient(server), Home);

    /// <summary>
    /// "CONNECT" ASKS THE SERVER: the one refusal only this protocol writes — `GET /me` with no
    /// token, answered 401 with a code the protocol names.
    /// </summary>
    [Fact]
    public async Task AFamilysServerIsTheOneThatRefusesInTheProtocolsOwnWords()
    {
        var server = new Server().On("/me", Refusal, HttpStatusCode.Unauthorized);

        Assert.True(await Probe(server));
        Assert.Equal(["/api/v1/me"], server.Asked);
    }

    /// <summary>
    /// Everything else that answers is not a family's server: a login page's 401 (HTML, so no
    /// code), a 401 with a code no protocol names, a website that says 200 to anything, a 404,
    /// and a host that never answered at all.
    /// </summary>
    [Fact]
    public async Task ALoginPageOrARouterIsNotAFamilysServer()
    {
        Assert.False(await Probe(new Server().On(
            "/me", "<html><body>Sign in</body></html>", HttpStatusCode.Unauthorized)));
        Assert.False(await Probe(new Server().On(
            "/me", """{"error": {"code": "nope", "message": "who are you"}}""",
            HttpStatusCode.Unauthorized)));
        Assert.False(await Probe(new Server().On("/me", "<html>router</html>")));
        Assert.False(await Probe(new Server().On("/me", null, HttpStatusCode.NotFound)));
        Assert.False(await Probe(new Server().OnAsync(
            "/me",
            () => Task.FromException<(HttpStatusCode, string?)>(
                new HttpRequestException("no route to host")))));
    }

    [Fact]
    public void PlainHttpIsAllowedAndSaidAndABareHostIsNeverPlain()
    {
        Assert.True(ServerCheck.IsPlainText(ServerCheck.Parse("http://192.168.1.10:8080")!));
        Assert.False(ServerCheck.IsPlainText(ServerCheck.Parse("192.168.1.10:8080")!));
        Assert.False(ServerCheck.IsPlainText(ServerCheck.Parse("https://chat.example.com/")!));
        Assert.Null(ServerCheck.Parse("   "));
    }

    /// <summary>
    /// A WRONG PASSWORD IS AN ANSWER TO THE FORM, and each other refusal says what it is — with the
    /// server's own words only where they came with a code.
    /// </summary>
    [Fact]
    public void SigningInSaysWhatWentWrong()
    {
        string Said(ApiError error, bool registering = false) =>
            DoorSentences.SignIn(error, registering, Say);

        Assert.Equal("Wrong username or password.",
            Said(new ApiError(ErrorCodes.InvalidCredentials, "wrong", 401)));
        Assert.Equal("The server rejected the request. Try again.",
            Said(new ApiError(ErrorCodes.Unauthorized, "no", 401), registering: true));
        Assert.Equal("That username is taken.",
            Said(new ApiError(ErrorCodes.UsernameTaken, "taken", 409), registering: true));
        Assert.Equal("Can't reach the server. Check your connection.",
            Said(ApiError.Transport("offline")));
        Assert.Equal("Try again in a moment.",
            Said(new ApiError(ErrorCodes.TooManyRequests, "slow down", 429)));
        Assert.Equal("The server had a problem. Try again in a moment.",
            Said(new ApiError(ErrorCodes.Internal, "boom", 500)));
        Assert.Equal("password must be at least 8 characters",
            Said(new ApiError(ErrorCodes.Validation, "password must be at least 8 characters", 400)));
        // A proxy's reason phrase is not the server's words.
        Assert.Equal("The server rejected the request.",
            Said(new ApiError("", "Bad Request", 400)));
        Assert.Equal("Something went wrong. Try again.",
            Said(new ApiError(ErrorCodes.NotInFamily, "no", 403)));
    }

    [Fact]
    public void JoiningSaysWhatWentWrong()
    {
        string Said(ApiError error) => DoorSentences.Join(error, Say);

        Assert.Equal("That code doesn't match any family. Check it and try again.",
            Said(new ApiError(ErrorCodes.InvalidInviteCode, "no such code", 404)));
        Assert.Equal("That code doesn't work anymore.",
            Said(new ApiError(ErrorCodes.UserNotFound, "gone", 404)));
        Assert.Equal("You're already in a family.",
            Said(new ApiError(ErrorCodes.AlreadyInFamily, "", 409)));
        Assert.Equal("You already have a pending request.",
            Said(new ApiError(ErrorCodes.JoinRequestPending, "", 409)));
        Assert.Equal("That family is full right now. Ask them to make room, then try the code again.",
            Said(new ApiError(ErrorCodes.FamilyFull, "", 409)));
        Assert.Equal("invite code must not be empty",
            Said(new ApiError(ErrorCodes.Validation, "invite code must not be empty", 400)));
        Assert.Equal("The server rejected that code.", Said(new ApiError("", "Conflict", 409)));
        Assert.Equal("Can't reach the server. Try again.", Said(ApiError.Transport("offline")));
    }

    [Fact]
    public void StartingAFamilySaysWhatWentWrong()
    {
        string Said(ApiError error) => DoorSentences.Create(error, Say);

        Assert.Equal("This server doesn't take new families.",
            Said(new ApiError(ErrorCodes.FamilyRegistrationDisabled, "closed", 403)));
        Assert.Equal("You're already in a family.",
            Said(new ApiError(ErrorCodes.AlreadyInFamily, "", 409)));
        Assert.Equal("name must be 1-64 characters",
            Said(new ApiError(ErrorCodes.Validation, "name must be 1-64 characters", 400)));
        Assert.Equal("The server rejected that name.", Said(new ApiError("", "Bad Request", 400)));
        Assert.Equal("Can't reach the server. Try again.", Said(ApiError.Transport("offline")));
    }

    [Fact]
    public void TheSentencesReadInTheReadersLanguage()
    {
        var german = JsonCatalog.For("de");
        Assert.Equal(
            german.Get("That code doesn't match any family. Check it and try again."),
            DoorSentences.Join(new ApiError(ErrorCodes.InvalidInviteCode, "", 404), german));
        Assert.NotEqual(
            "That code doesn't match any family. Check it and try again.",
            DoorSentences.Join(new ApiError(ErrorCodes.InvalidInviteCode, "", 404), german));
    }

    private (FamilyDoor Door, AppSession Session) Door(Server server)
    {
        var tokens = new MemoryTokenStore("t0ken");
        var api = new ApiClient(new HttpClient(server), Home, tokens);
        var session = new AppSession(api, tokens, cache);
        return (new FamilyDoor(api, session), session);
    }

    /// <summary>
    /// THE SCREEN IS DECIDED BY `GET /me`, NOT BY THE ANSWER: a join that came back `pending` is
    /// followed by the read that puts the app on the waiting screen, and the code goes up trimmed.
    /// </summary>
    [Fact]
    public async Task AJoinIsFollowedByTheReadThatDecidesTheScreen()
    {
        var server = new Server()
            .On("/families/join", """{"status": "pending"}""")
            .On("/me", MePending);
        var (door, session) = Door(server);

        Assert.Null(await door.JoinAsync("  ABCD2345 "));

        Assert.Equal(Gate.Pending, session.State.Gate);
        Assert.Equal("The Smiths", session.State.PendingFamilyName);
        Assert.Equal(["/api/v1/families/join", "/api/v1/me"], server.Asked);
        Assert.Contains("\"ABCD2345\"", server.Bodies[0]);
    }

    [Fact]
    public async Task ARefusedJoinSaysSoAndMovesNothing()
    {
        var server = new Server().On(
            "/families/join", """{"error": {"code": "family_full", "message": "full"}}""",
            HttpStatusCode.Conflict);
        var (door, session) = Door(server);

        Assert.Equal(
            "That family is full right now. Ask them to make room, then try the code again.",
            await door.JoinAsync("ABCD2345"));

        Assert.Equal(Gate.SignedOut, session.State.Gate);
        Assert.Equal(["/api/v1/families/join"], server.Asked);
    }

    [Fact]
    public async Task StartingAFamilyMakesItsOwnerAnOwner()
    {
        var server = new Server()
            .On("/families", """{"family": {"id": 3, "name": "The Smiths"}}""", HttpStatusCode.Created)
            .On("/me", MeOwner);
        var (door, session) = Door(server);

        Assert.Null(await door.CreateAsync(" The Smiths "));

        Assert.Equal(Gate.Owner, session.State.Gate);
        Assert.Contains("\"The Smiths\"", server.Bodies[0]);
    }

    [Fact]
    public void TheFormsSendNothingTheyCannotSend()
    {
        Assert.False(FamilyDoor.MayJoin("   "));
        Assert.True(FamilyDoor.MayJoin("ABCD2345"));
        Assert.False(FamilyDoor.MayCreate("  "));
        Assert.True(FamilyDoor.MayCreate(new string('a', FamilyDoor.NameLimit)));
        Assert.False(FamilyDoor.MayCreate(new string('a', FamilyDoor.NameLimit + 1)));
        // Surrounding space is not part of the name.
        Assert.True(FamilyDoor.MayCreate("  " + new string('a', FamilyDoor.NameLimit) + "  "));

        Assert.True(SignInForm.MaySubmit("anna", "hunter22", null, registering: false));
        Assert.False(SignInForm.MaySubmit("anna", "", null, registering: false));
        Assert.False(SignInForm.MaySubmit(" ", "hunter22", null, registering: false));
        Assert.False(SignInForm.MaySubmit("anna", "hunter22", " ", registering: true));
        Assert.True(SignInForm.MaySubmit("anna", "hunter22", "Anna", registering: true));
    }

    [Fact]
    public void ARowsTimeIsInTheReadersLanguage()
    {
        var at = new DateTimeOffset(2026, 9, 13, 14, 5, 0, TimeSpan.Zero);
        var german = CultureInfo.GetCultureInfo("de-DE");
        var american = CultureInfo.GetCultureInfo("en-US");

        Assert.Equal(at.ToLocalTime().ToString("t", german),
            RowTimeText.Format(RowTimeKind.Clock, at, german, Say));
        Assert.NotEqual(
            RowTimeText.Format(RowTimeKind.Clock, at, american, Say),
            RowTimeText.Format(RowTimeKind.Clock, at, german, Say));
        Assert.Equal(at.ToLocalTime().ToString("dddd", german),
            RowTimeText.Format(RowTimeKind.Weekday, at, german, Say));
        Assert.Equal(at.ToLocalTime().ToString("d", german),
            RowTimeText.Format(RowTimeKind.Date, at, german, Say));
        Assert.Equal(JsonCatalog.For("ru").Get("Yesterday"),
            RowTimeText.Format(RowTimeKind.Yesterday, at, german, JsonCatalog.For("ru")));
        Assert.Equal(string.Empty, RowTimeText.Format(RowTimeKind.None, null, german, Say));
    }

    [Fact]
    public void ABubbleSaysWhoWhatAndWhen()
    {
        var chats = new ChatStore(cache, () => 7);
        chats.Replace(
            [new MemberDto(7, "anna", "Anna"), new MemberDto(11, "bob", "Bob")],
            [new MemberDto(12, "carl", "Carl", HasLeft: true, Deleted: true)]);
        var list = new ChatListModel(chats, () => 7);
        const string Sent = "2026-09-13T14:05:00Z";
        Bubble Of(long sender, string body = "hi", bool hidden = false, bool revealed = false,
            string? edited = null, AttachmentDto[]? media = null) =>
            new(new MessageDto(1, 42, sender, null, body, Sent, EditedAt: edited, Attachments: media),
                hidden, revealed, sender == 7);

        Assert.Equal("You", BubbleText.Sender(Of(7), chats, Say));
        Assert.Equal("Bob", BubbleText.Sender(Of(11), chats, Say));
        Assert.Equal("Deleted account", BubbleText.Sender(Of(12), chats, Say));

        Assert.Equal("Hidden — blocked member", BubbleText.Words(Of(11, hidden: true), list, Say));
        Assert.Equal("line one\nline two", BubbleText.Words(Of(11, "line one\nline two"), list, Say));
        Assert.Equal("asked for",
            BubbleText.Words(Of(11, "asked for", hidden: true, revealed: true), list, Say));
        Assert.Equal("Photo",
            BubbleText.Words(Of(11, "", media: [new AttachmentDto(5, "photo")]), list, Say));
        // A CALL READS AS A CALL: its body is the English placeholder the server writes for clients
        // that predate calls, and a bubble that drew it would be English on every translated screen.
        var call = new Bubble(
            new MessageDto(2, 42, 11, "call-1", "Missed voice call", Sent,
                Call: new CallRecordDto("missed")),
            false, false, false);
        var russian = JsonCatalog.For("ru");
        var inRussian = new ChatListModel(chats, () => 7, russian);
        Assert.NotEqual("Missed voice call", BubbleText.Words(call, inRussian, russian));
        Assert.Equal(inRussian.Preview(call.Message, hidden: false), BubbleText.Words(call, inRussian, russian));

        var german = CultureInfo.GetCultureInfo("de-DE");
        var clock = DateTimeOffset.Parse(Sent, CultureInfo.InvariantCulture)
            .ToLocalTime().ToString("t", german);
        Assert.Equal(clock, BubbleText.When(Of(11).Message, german, Say));
        Assert.Equal($"{clock} · edited",
            BubbleText.When(Of(11, edited: "2026-09-13T14:06:00Z").Message, german, Say));
    }
}
