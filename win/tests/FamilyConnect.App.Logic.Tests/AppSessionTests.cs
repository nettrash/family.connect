using System.Net;
using System.Text;
using FamilyConnect.App.Logic;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// Which screen the app is on, and the three ways a signed-in state ends.
/// </summary>
public class AppSessionTests : IDisposable
{
    private readonly Database cache = Database.OpenInMemory();

    public void Dispose() => cache.Dispose();

    private const string Token = """{"token": "t0ken", "user": {"id": 7, "username": "anna", "display_name": "Anna"}}""";

    private static string Me(string extra = "") =>
        $$"""
        {"user": {"id": 7, "username": "anna", "display_name": "Anna"},
         "blocked_user_ids": [], "calls_enabled": true, "max_family_members": 8,
         "support_contact": "help@example.com"{{extra}}}
        """;

    private const string InFamily =
        """, "family": {"id": 3, "name": "The Smiths", "ai_history": true}, "role": "owner" """;

    private const string AMember =
        """, "family": {"id": 3, "name": "The Smiths", "ai_history": true}, "role": "member" """;

    private const string Waiting =
        """, "pending_join_request": {"family_id": 3, "family_name": "The Smiths"} """;

    private (AppSession Session, MemoryTokenStore Tokens, Server Handler) Build(
        Server server, string? token = null)
    {
        var tokens = new MemoryTokenStore(token);
        var api = new ApiClient(
            new HttpClient(server), ServerUrl.Normalise("chat.example.com")!, tokens);
        return (new AppSession(api, tokens, cache), tokens, server);
    }

    [Fact]
    public async Task SigningInPutsTheAppOnTheScreenMeAnswersFor()
    {
        var server = new Server().On("/auth/login", Token).On("/me", Me(InFamily));
        var (session, tokens, _) = Build(server);
        var screens = new List<Gate>();
        session.Changed += state => screens.Add(state.Gate);

        Assert.Null(await session.SignInAsync("anna", "hunter2"));

        Assert.Equal("t0ken", tokens.Token);
        Assert.Equal(Gate.Owner, session.State.Gate);
        Assert.True(session.State.IsOwner);
        Assert.True(session.State.CanChat);
        Assert.Equal("The Smiths", session.State.Family!.Name);
        // The capabilities arrive with the answer rather than as errors later.
        Assert.True(session.State.CallsEnabled);
        Assert.Equal(8, session.State.MaxFamilyMembers);
        Assert.Equal([Gate.Owner], screens);
    }

    [Fact]
    public async Task AMemberIsNotAnOwnerAndAnAccountWithNoFamilyIsAtTheGate()
    {
        var member = new Server().On("/auth/login", Token).On("/me", Me(AMember));
        var (session, _, _) = Build(member);
        await session.SignInAsync("anna", "hunter2");
        Assert.Equal(Gate.Member, session.State.Gate);
        Assert.False(session.State.IsOwner);

        var alone = new Server().On("/auth/login", Token).On("/me", Me());
        var (gate, _, _) = Build(alone);
        await gate.SignInAsync("anna", "hunter2");
        Assert.Equal(Gate.NoFamily, gate.State.Gate);
        Assert.False(gate.State.CanChat);

        var waiting = new Server().On("/auth/login", Token).On("/me", Me(Waiting));
        var (pending, _, _) = Build(waiting);
        await pending.SignInAsync("anna", "hunter2");
        Assert.Equal(Gate.Pending, pending.State.Gate);
        Assert.Equal("The Smiths", pending.State.PendingFamilyName);
    }

    /// <summary>
    /// A WRONG PASSWORD IS NOT AN EXPIRED SESSION. Both are 401, and one of this app's own
    /// clients read every one of them as "your session expired" — so a mistyped password told
    /// the user they had been signed out of a session they were never in.
    /// </summary>
    [Fact]
    public async Task AWrongPasswordIsAnswerToTheFormAndNotTheEndOfAnything()
    {
        var server = new Server().On(
            "/auth/login",
            """{"error": {"code": "invalid_credentials", "message": "no"}}""",
            HttpStatusCode.Unauthorized);
        var (session, tokens, _) = Build(server);
        var endings = new List<SessionEnd>();
        session.Ended += end => endings.Add(end);

        var refused = await session.SignInAsync("anna", "wrong");

        Assert.Equal(ErrorCodes.InvalidCredentials, refused!.Code);
        Assert.False(refused.SessionGone);
        Assert.Empty(endings);
        Assert.Null(tokens.Token);
        Assert.Equal(Gate.SignedOut, session.State.Gate);
    }

    /// <summary>
    /// And a failed sign-in never ends a session that IS open: the answer belongs to the form,
    /// and the account already signed in on this device is none of its business.
    /// </summary>
    [Fact]
    public async Task AFailedSignInLeavesASessionThatIsAlreadyOpenAlone()
    {
        var server = new Server()
            .On("/me", Me(InFamily))
            .On(
                "/auth/login",
                """{"error": {"code": "invalid_credentials", "message": "no"}}""",
                HttpStatusCode.Unauthorized);
        var (session, tokens, _) = Build(server, token: "t0ken");
        await session.RefreshAsync();
        var endings = new List<SessionEnd>();
        session.Ended += end => endings.Add(end);

        Assert.Equal(
            ErrorCodes.InvalidCredentials,
            (await session.SignInAsync("bob", "wrong"))!.Code);

        Assert.Empty(endings);
        Assert.Equal("t0ken", tokens.Token);
        Assert.Equal(Gate.Owner, session.State.Gate);
    }

    /// <summary>
    /// THE SESSION ENDS ONCE. A dead session is discovered by whatever asks next, and on a
    /// reconnect loop that is every few seconds — the user needs telling once.
    /// </summary>
    [Fact]
    public void ASessionThatIsGoneEndsOnceHoweverOftenItIsDiscovered()
    {
        var (session, tokens, _) = Build(new Server(), token: "t0ken");
        var endings = new List<SessionEnd>();
        session.Ended += end => endings.Add(end);

        session.SessionGone();
        session.SessionGone();
        session.SessionGone();

        Assert.Equal([SessionEnd.Expired], endings);
        Assert.Null(tokens.Token);
        Assert.Equal(Gate.SignedOut, session.State.Gate);
    }

    /// <summary>
    /// A FLAKY NETWORK IS NOT A SIGN-OUT. The alternative is an app that drops to the sign-in
    /// screen in a lift — and asks for a password it has no business asking for.
    /// </summary>
    [Fact]
    public async Task ATransientFailureLeavesTheAppExactlyWhereItIs()
    {
        var server = new Server()
            .On("/auth/login", Token)
            .Then("/me",
                (HttpStatusCode.OK, Me(InFamily)),
                (HttpStatusCode.BadGateway, "<html>502</html>"));
        var (session, tokens, _) = Build(server);
        await session.SignInAsync("anna", "hunter2");

        var failed = await session.RefreshAsync();

        Assert.True(failed!.Transient);
        Assert.Equal(Gate.Owner, session.State.Gate);
        Assert.Equal("t0ken", tokens.Token);
    }

    /// <summary>
    /// A 401 on an authenticated read IS the session going away underneath them, and it lands in
    /// the same place the socket's 4401 does.
    /// </summary>
    [Fact]
    public async Task A401OnAnAuthenticatedReadEndsTheSession()
    {
        var server = new Server()
            .On("/auth/login", Token)
            .Then("/me",
                (HttpStatusCode.OK, Me(InFamily)),
                (HttpStatusCode.Unauthorized,
                 """{"error": {"code": "unauthorized", "message": "gone"}}"""));
        var (session, tokens, _) = Build(server);
        await session.SignInAsync("anna", "hunter2");
        var endings = new List<SessionEnd>();
        session.Ended += end => endings.Add(end);

        await session.RefreshAsync();

        Assert.Equal([SessionEnd.Expired], endings);
        Assert.Null(tokens.Token);
    }

    /// <summary>
    /// Being removed from a family is found out on `GET /me` and nowhere else: a removal this
    /// device slept through raises no frame it will ever see.
    /// </summary>
    [Fact]
    public async Task BeingRemovedFromAFamilyIsFoundOutOnTheNextReadAndTakesItsDataWithIt()
    {
        var server = new Server()
            .On("/auth/login", Token)
            .Then("/me", (HttpStatusCode.OK, Me(AMember)), (HttpStatusCode.OK, Me()));
        var (session, tokens, _) = Build(server);
        await session.SignInAsync("anna", "hunter2");
        var chats = new ChatStore(cache);
        chats.Replace([new ChatRowDto(new ChatDto(42, "family", "The Smiths"))]);
        var endings = new List<SessionEnd>();
        session.Ended += end => endings.Add(end);

        await session.RefreshAsync();

        Assert.Equal([SessionEnd.RemovedFromFamily], endings);
        Assert.Equal(Gate.NoFamily, session.State.Gate);
        // Their family's chats are not theirs to keep — but they are still signed in.
        Assert.Empty(chats.Chats());
        Assert.Equal("t0ken", tokens.Token);
    }

    [Fact]
    public async Task AJoinRequestThatIsNoLongerPendingWasRefused()
    {
        var server = new Server()
            .On("/auth/login", Token)
            .Then("/me", (HttpStatusCode.OK, Me(Waiting)), (HttpStatusCode.OK, Me()));
        var (session, _, _) = Build(server);
        await session.SignInAsync("anna", "hunter2");
        var endings = new List<SessionEnd>();
        session.Ended += end => endings.Add(end);

        await session.RefreshAsync();

        Assert.Equal([SessionEnd.JoinRequestRejected], endings);
        Assert.Equal(Gate.NoFamily, session.State.Gate);
    }

    /// <summary>
    /// Signing out tells the server — which is what stops this device's notifications — and
    /// being unable to tell it changes nothing: the token is this device's to forget.
    /// </summary>
    [Fact]
    public async Task SigningOutTellsTheServerAndForgetsEverythingEitherWay()
    {
        var server = new Server()
            .On("/auth/login", Token)
            .On("/me", Me(InFamily))
            .On("/auth/logout", null, HttpStatusCode.NoContent);
        var (session, tokens, handler) = Build(server);
        await session.SignInAsync("anna", "hunter2");
        var chats = new ChatStore(cache);
        chats.Replace([new ChatRowDto(new ChatDto(42, "family", "The Smiths"))]);
        new OutboxStore(cache).Queue(new OutboxRow("k", 42, "Dinner at 7?"));
        var endings = new List<SessionEnd>();
        session.Ended += end => endings.Add(end);

        await session.SignOutAsync();

        Assert.Contains("/api/v1/auth/logout", handler.Asked);
        Assert.Equal([SessionEnd.SignedOut], endings);
        Assert.Null(tokens.Token);
        Assert.Equal(Gate.SignedOut, session.State.Gate);
        Assert.Empty(chats.Chats());
        // THE OUTBOX GOES TOO: a queued message belongs to the account that wrote it, and a send
        // that landed in the next person's family would be the worst bug this app could have.
        Assert.Empty(new OutboxStore(cache).All());
        // The support address outlives the session: it is the server's, not the account's, and
        // it is what a sign-in screen shows when somebody cannot get in.
        Assert.Equal("help@example.com", session.State.SupportContact);
    }

    /// <summary>
    /// Not only a refusal: a request that cannot even be made — no route, a dead handler, a
    /// cancelled token — must not leave the app signed in to something it has already forgotten
    /// how to talk to.
    /// </summary>
    [Fact]
    public async Task ASignOutWhoseRequestCannotEvenBeMadeIsStillASignOut()
    {
        // No /auth/logout route at all: the handler throws rather than answering.
        var server = new Server().On("/auth/login", Token).On("/me", Me(InFamily));
        var (session, tokens, _) = Build(server);
        await session.SignInAsync("anna", "hunter2");
        var endings = new List<SessionEnd>();
        session.Ended += end => endings.Add(end);

        await session.SignOutAsync();

        Assert.Equal([SessionEnd.SignedOut], endings);
        Assert.Null(tokens.Token);
        Assert.Equal(Gate.SignedOut, session.State.Gate);
    }

    [Fact]
    public async Task ASignOutTheNetworkRefusesIsStillASignOut()
    {
        var server = new Server()
            .On("/auth/login", Token)
            .On("/me", Me(InFamily))
            .On("/auth/logout", "<html>502</html>", HttpStatusCode.BadGateway);
        var (session, tokens, _) = Build(server);
        await session.SignInAsync("anna", "hunter2");

        await session.SignOutAsync();

        Assert.Null(tokens.Token);
        Assert.Equal(Gate.SignedOut, session.State.Gate);
    }

    /// <summary>
    /// Signing in as somebody else on a device that was signed in before must not leave the last
    /// family's messages on screen — nor their outbox anywhere near this account's socket.
    /// </summary>
    [Fact]
    public async Task ANewSignInDoesNotInheritTheLastAccountsCache()
    {
        var chats = new ChatStore(cache);
        chats.Replace([new ChatRowDto(new ChatDto(42, "family", "The Other Family"))]);
        var server = new Server().On("/auth/login", Token).On("/me", Me(InFamily));
        var (session, _, _) = Build(server);

        await session.SignInAsync("bob", "hunter2");

        Assert.Empty(chats.Chats());
    }

    /// <summary>
    /// A WRONG PASSWORD ON A PASSWORD CHANGE IS NOT AN EXPIRED SESSION EITHER. It is the same
    /// 401 and the same distinction: the account is still there, and so is the session.
    /// </summary>
    [Fact]
    public async Task AWrongCurrentPasswordDoesNotSignAnybodyOut()
    {
        var server = new Server()
            .On("/auth/login", Token)
            .On("/me", Me(InFamily))
            .On(
                "/me/password",
                """{"error": {"code": "invalid_credentials", "message": "no"}}""",
                HttpStatusCode.Unauthorized);
        var (session, tokens, _) = Build(server);
        await session.SignInAsync("anna", "hunter2");
        var endings = new List<SessionEnd>();
        session.Ended += end => endings.Add(end);

        var refused = await session.ChangePasswordAsync("wrong", "hunter22");

        Assert.Equal(ErrorCodes.InvalidCredentials, refused!.Code);
        Assert.Empty(endings);
        Assert.Equal("t0ken", tokens.Token);
        Assert.Equal(Gate.Owner, session.State.Gate);
    }

    /// <summary>
    /// Deleting the account ends everything — and it is its OWN ending, because there is nothing
    /// left to sign back into and a screen offering to would be cruel as well as wrong.
    /// </summary>
    [Fact]
    public async Task DeletingTheAccountIsItsOwnEndingAndTakesEverythingWithIt()
    {
        var server = new Server()
            .On("/auth/login", Token)
            .On("/me", Me(InFamily))
            .On("/me/delete", null, HttpStatusCode.NoContent);
        var (session, tokens, _) = Build(server);
        await session.SignInAsync("anna", "hunter2");
        var chats = new ChatStore(cache);
        chats.Replace([new ChatRowDto(new ChatDto(42, "family", "The Smiths"))]);
        var endings = new List<SessionEnd>();
        session.Ended += end => endings.Add(end);

        Assert.Null(await session.DeleteAccountAsync("hunter2"));

        Assert.Equal([SessionEnd.AccountDeleted], endings);
        Assert.Null(tokens.Token);
        Assert.Equal(Gate.SignedOut, session.State.Gate);
        Assert.Empty(chats.Chats());
    }

    [Fact]
    public async Task ARefusedDeletionChangesNothing()
    {
        var server = new Server()
            .On("/auth/login", Token)
            .On("/me", Me(InFamily))
            .On(
                "/me/delete",
                """{"error": {"code": "invalid_credentials", "message": "no"}}""",
                HttpStatusCode.Unauthorized);
        var (session, tokens, _) = Build(server);
        await session.SignInAsync("anna", "hunter2");
        var endings = new List<SessionEnd>();
        session.Ended += end => endings.Add(end);

        var refused = await session.DeleteAccountAsync("wrong");

        Assert.Equal(ErrorCodes.InvalidCredentials, refused!.Code);
        Assert.Empty(endings);
        Assert.Equal("t0ken", tokens.Token);
        Assert.Equal(Gate.Owner, session.State.Gate);
    }

    [Fact]
    public async Task TheFamilysOwnDocumentFillsInTheAssistantAndTheRoster()
    {
        var server = new Server()
            .On("/auth/login", Token)
            .On("/me", Me(InFamily))
            .On("/families/mine", """
                {"family": {"id": 3, "name": "The Smiths", "ai_history": true, "ai_vision": true},
                 "members": [{"id": 7, "username": "anna", "display_name": "Anna", "role": "owner"}],
                 "blocked_user_ids": [], "max_board_seq": 11,
                 "assistant": {"user_id": 1, "display_name": "Assistant", "mention": "@ai",
                               "draw": "/draw", "vision": true, "images": true}}
                """);
        var (session, _, _) = Build(server);
        await session.SignInAsync("anna", "hunter2");

        Assert.Null(await session.RefreshFamilyAsync());

        Assert.Equal("Assistant", session.State.Assistant!.DisplayName);
        Assert.True(session.State.Assistant.Images);
        Assert.True(session.State.Family!.AiVision);
        // And the gate did not move: the family document says nothing about who is signed in.
        Assert.Equal(Gate.Owner, session.State.Gate);
    }
}
