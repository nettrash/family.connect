using System.Net;
using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// One account in several families, from the app's side (docs/protocol.md, "Several families"; issue #88): the list of
/// families and the one shown, the switcher, invitations, the doors to another family, leaving one, and everything
/// family-level following the family shown.
/// </summary>
public class SeveralFamiliesTests : IDisposable
{
    private static readonly IStringCatalog Say = EnglishCatalog.Instance;

    private static readonly TimeSpan Patience = TimeSpan.FromSeconds(5);

    private readonly Database cache = Database.OpenInMemory();

    public void Dispose() => cache.Dispose();

    private const string Token = """{"token": "t0ken", "user": {"id": 7, "username": "anna", "display_name": "Anna"}}""";

    private const string Smiths = """{"id": 3, "name": "The Smiths", "role": "member"}""";
    private const string Ivanovs = """{"id": 5, "name": "Ivanovs", "role": "owner"}""";

    private const string Invitation =
        """{"id": 4, "family_id": 9, "family_name": "Sidorovs", "invited_by": {"id": 11, "username": "carl", "display_name": "Carl"}, "user": {"id": 7, "username": "anna", "display_name": "Anna"}}""";

    /// <summary>`GET /me` of a server that knows several families.</summary>
    private static string Me(string families, string pending = "", string invitations = "", bool enabled = true, int cap = 5) =>
        $$"""
        {"user": {"id": 7, "username": "anna", "display_name": "Anna"}, "blocked_user_ids": [],
         "families": [{{families}}], "pending_join_requests": [{{pending}}], "invitations": [{{invitations}}],
         "member_families_enabled": {{(enabled ? "true" : "false")}}, "max_families_per_account": {{cap}}}
        """;

    private (AppSession Session, ApiClient Api, Server Server, MemoryCurrentFamily Remembered) Build(
        Server server, MemoryCurrentFamily? remembered = null)
    {
        var tokens = new MemoryTokenStore("t0ken");
        var api = new ApiClient(new HttpClient(server), ServerUrl.Normalise("chat.example.com")!, tokens);
        remembered ??= new MemoryCurrentFamily();
        return (new AppSession(api, tokens, cache, currentFamily: remembered), api, server, remembered);
    }

    // ---- the list, and the family shown -------------------------------------------------------------------------------------

    /// <summary>
    /// The model is a LIST: every family from `GET /me`, the home family shown first by default, the gate decided by the role
    /// in the family SHOWN — and that family named on every family endpoint from here on.
    /// </summary>
    [Fact]
    public async Task TheFirstFamilyIsShownAndItsRoleDecidesTheGate()
    {
        var (session, api, _, remembered) = Build(new Server().On("/me", Me($"{Smiths}, {Ivanovs}")));
        Assert.Null(await session.RefreshAsync());

        Assert.Equal([3L, 5L], session.State.AllFamilies.Select(family => family.Id));
        Assert.Equal(3, session.State.CurrentFamilyId);
        Assert.Equal("The Smiths", session.State.Family!.Name);
        Assert.Equal(Gate.Member, session.State.Gate);
        Assert.True(session.State.NamesFamilies);
        Assert.Equal(5, session.State.MaxFamiliesPerAccount);
        Assert.Equal(3, api.FamilyScope);
        Assert.Equal(3, remembered.For(7));
    }

    /// <summary>The family last shown is remembered per account, so a relaunch opens where the reader left off.</summary>
    [Fact]
    public async Task TheFamilyLastShownIsShownAgain()
    {
        var remembered = new MemoryCurrentFamily();
        remembered.Remember(7, 5);
        var (session, _, _, _) = Build(new Server().On("/me", Me($"{Smiths}, {Ivanovs}")), remembered);
        await session.RefreshAsync();
        Assert.Equal(5, session.State.CurrentFamilyId);
        Assert.Equal(Gate.Owner, session.State.Gate);
        // A remembered family the account is no longer in is no answer: the first is.
        remembered.Remember(7, 99);
        var (other, _, _, _) = Build(new Server().On("/me", Me(Smiths)), remembered);
        await other.RefreshAsync();
        Assert.Equal(3, other.State.CurrentFamilyId);
    }

    /// <summary>
    /// Switching shows the other family: its role decides the gate, it is remembered, it is named on the wire, and its own
    /// document is read for what only it carries.
    /// </summary>
    [Fact]
    public async Task SwitchingShowsTheOtherFamily()
    {
        var server = new Server()
            .On("/me", Me($"{Smiths}, {Ivanovs}"))
            .On("/families/5", """{"family": {"id": 5, "name": "Ivanovs", "invite_code": "ABCD2345"}, "assistant": {"user_id": 99, "display_name": "Assistant", "mention": "ai"}}""");
        var (session, api, _, remembered) = Build(server);
        await session.RefreshAsync();
        var gates = new List<Gate>();
        session.Changed += state => gates.Add(state.Gate);

        Assert.Null(await session.SwitchFamilyAsync(5));

        Assert.Equal(5, session.State.CurrentFamilyId);
        Assert.Equal(Gate.Owner, session.State.Gate);
        Assert.Equal("ABCD2345", session.State.Family!.InviteCode);
        Assert.True(session.State.Family.IsOwner);
        Assert.Equal(99, session.State.Assistant!.UserId);
        Assert.Equal(5, api.FamilyScope);
        Assert.Equal(5, remembered.For(7));
        Assert.Contains("/api/v1/families/5", server.Asked);
        Assert.All(gates, gate => Assert.Equal(Gate.Owner, gate));
        // A family that is not the account's, or the one already shown, is no switch.
        Assert.False(session.Switch(5));
        Assert.False(session.Switch(42));
    }

    /// <summary>
    /// A family that is no longer in `GET /me` — left on another device, removed — goes with its rows, and only its; the first
    /// that remains is shown, and nothing says the session ended.
    /// </summary>
    [Fact]
    public async Task AFamilyGoneFromMeTakesOnlyItsRows()
    {
        var server = new Server().Then("/me",
            (HttpStatusCode.OK, Me($"{Smiths}, {Ivanovs}")),
            (HttpStatusCode.OK, Me(Smiths)));
        var (session, _, _, remembered) = Build(server);
        remembered.Remember(7, 5);
        await session.RefreshAsync();
        Assert.Equal(5, session.State.CurrentFamilyId);
        var chats = new ChatStore(cache, () => 7, () => session.State.CurrentFamilyId);
        chats.Replace([
            new ChatRowDto(new ChatDto(42, "family", "The Smiths", FamilyIds: [3])),
            new ChatRowDto(new ChatDto(43, "family", "Ivanovs", FamilyIds: [5])),
        ]);
        new BoardStore(cache).In(5).Replace([new NoteDto(2, 11, "text", "Bread", BoardSeq: 9)], 9);
        var ended = new List<SessionEnd>();
        var dropped = new List<long>();
        session.Ended += ended.Add;
        session.FamilyDropped += dropped.Add;

        await session.RefreshAsync();

        Assert.Equal(3, session.State.CurrentFamilyId);
        Assert.Equal(Gate.Member, session.State.Gate);
        Assert.Empty(ended);
        Assert.Equal([5L], dropped);
        Assert.Null(chats.Chat(43));
        Assert.NotNull(chats.Chat(42));
        Assert.Empty(new BoardStore(cache).In(5).Notes());
    }

    /// <summary>A family this cache holds and `GET /me` does not list — left while the app was closed — goes at the first answer.</summary>
    [Fact]
    public async Task AFamilyLeftWhileClosedIsDroppedAtTheFirstAnswer()
    {
        new BoardStore(cache).In(5).Replace([new NoteDto(2, 11, "text", "Bread", BoardSeq: 9)], 9);
        new BoardStore(cache).In(3).Replace([new NoteDto(1, 9, "text", "Milk", BoardSeq: 4)], 4);
        var (session, _, _, _) = Build(new Server().On("/me", Me(Smiths)));
        await session.RefreshAsync();
        Assert.Empty(new BoardStore(cache).In(5).Notes());
        Assert.Single(new BoardStore(cache).In(3).Notes());
    }

    /// <summary>The LAST family gone is today's ending, unchanged: everything wiped, and the reader told.</summary>
    [Fact]
    public async Task TheLastFamilyGoneEndsAsBefore()
    {
        var server = new Server().Then("/me", (HttpStatusCode.OK, Me(Smiths)), (HttpStatusCode.OK, Me(string.Empty)));
        var (session, _, _, _) = Build(server);
        await session.RefreshAsync();
        new ChatStore(cache).Replace([new ChatRowDto(new ChatDto(42, "family", "The Smiths", FamilyIds: [3]))]);
        var ended = new List<SessionEnd>();
        session.Ended += ended.Add;
        await session.RefreshAsync();
        Assert.Equal(Gate.NoFamily, session.State.Gate);
        Assert.Equal([SessionEnd.RemovedFromFamily], ended);
        Assert.Empty(new ChatStore(cache).Chats());
    }

    /// <summary>
    /// An account with NO family gets today's gate; one with a family and a join request elsewhere is in its family, and the
    /// request waits in the switcher.
    /// </summary>
    [Fact]
    public async Task APendingRequestIsAGateOnlyWithoutAFamily()
    {
        const string Petrovs = """{"family_id": 8, "family_name": "Petrovs"}""";
        var (waiting, _, _, _) = Build(new Server().On("/me", Me(string.Empty, Petrovs)));
        await waiting.RefreshAsync();
        Assert.Equal(Gate.Pending, waiting.State.Gate);
        Assert.Equal("Petrovs", waiting.State.PendingFamilyName);

        var (member, _, _, _) = Build(new Server().On("/me", Me(Smiths, Petrovs)));
        await member.RefreshAsync();
        Assert.Equal(Gate.Member, member.State.Gate);
        Assert.Equal(8, Assert.Single(member.State.AllPending).FamilyId);
    }

    /// <summary>
    /// A server from before several families: its one family is the list, but it is never asked for a family by id — the
    /// paths stay `mine` — and the cache an older build filled becomes that family's.
    /// </summary>
    [Fact]
    public async Task AnOlderServerIsAskedForMine()
    {
        new BoardStore(cache).Replace([new NoteDto(1, 9, "text", "Milk", BoardSeq: 4)], 4);
        var (session, api, _, _) = Build(new Server().On("/me",
            """{"user": {"id": 7, "username": "anna", "display_name": "Anna"}, "family": {"id": 3, "name": "The Smiths"}, "role": "owner"}"""));
        await session.RefreshAsync();
        Assert.Equal(Gate.Owner, session.State.Gate);
        Assert.Equal(3, session.State.CurrentFamilyId);
        Assert.False(session.State.NamesFamilies);
        Assert.Null(api.FamilyScope);
        Assert.Equal(1, Assert.Single(new BoardStore(cache, () => 3).Notes()).Id);
        Assert.Equal(4, new BoardStore(cache, () => 3).Cursor);
    }

    // ---- invitations to me --------------------------------------------------------------------------------------------------

    /// <summary>Frames keep the invitations live: one to this account is listed, one to somebody else is not this account's.</summary>
    [Fact]
    public async Task InvitationFramesKeepTheListLive()
    {
        var (session, _, _, _) = Build(new Server().On("/me", Me(Smiths)));
        await session.RefreshAsync();
        var invitation = Wire.Decode<InvitationDto>(Invitation)!;
        session.ApplyInvitation(invitation);
        session.ApplyInvitation(invitation);
        session.ApplyInvitation(invitation with { Id = 6, User = new UserDto(12, "dana", "Dana") });
        Assert.Equal(4, Assert.Single(session.State.AllInvitations).Id);
        session.InvitationGone(4);
        Assert.Empty(session.State.AllInvitations);
    }

    /// <summary>Join: the family is added AND shown, and the invitation goes.</summary>
    [Fact]
    public async Task JoiningAnInvitationShowsTheFamily()
    {
        var server = new Server()
            .Then("/me", (HttpStatusCode.OK, Me(Smiths, invitations: Invitation)),
                (HttpStatusCode.OK, Me($"{Smiths}, {{\"id\": 9, \"name\": \"Sidorovs\", \"role\": \"member\"}}")))
            .On("/invitations/4/accept", """{"family": {"id": 9, "name": "Sidorovs"}}""");
        var (session, _, _, _) = Build(server);
        await session.RefreshAsync();
        Assert.Single(session.State.AllInvitations);

        Assert.Null(await session.AcceptInvitationAsync(4));

        Assert.Equal(9, session.State.CurrentFamilyId);
        Assert.Empty(session.State.AllInvitations);
    }

    /// <summary>
    /// `invitation_not_pending` drops it — it is gone, whatever the reason — and comes back for the window to say; a full
    /// family or the cap leaves it standing.
    /// </summary>
    [Fact]
    public async Task AnInvitationNoLongerPendingIsDropped()
    {
        var server = new Server()
            .On("/me", Me(Smiths, invitations: Invitation))
            .Then("/invitations/4/accept",
                (HttpStatusCode.Conflict, """{"error": {"code": "too_many_families", "message": "cap"}}"""),
                (HttpStatusCode.Conflict, """{"error": {"code": "invitation_not_pending", "message": "gone"}}"""));
        var (session, _, _, _) = Build(server);
        await session.RefreshAsync();

        Assert.Equal(ErrorCodes.TooManyFamilies, (await session.AcceptInvitationAsync(4))!.Code);
        Assert.Single(session.State.AllInvitations);
        var gone = await session.AcceptInvitationAsync(4);
        Assert.Equal(ErrorCodes.InvitationNotPending, gone!.Code);
        Assert.Empty(session.State.AllInvitations);
        Assert.Equal("This invitation is no longer available.", FamilySentences.For(gone, 5, Say));
    }

    [Fact]
    public async Task DecliningAnInvitationDropsIt()
    {
        var server = new Server()
            .On("/me", Me(Smiths, invitations: Invitation))
            .On("/invitations/4/decline", null, HttpStatusCode.NoContent);
        var (session, _, _, _) = Build(server);
        await session.RefreshAsync();
        Assert.Null(await session.DeclineInvitationAsync(4));
        Assert.Empty(session.State.AllInvitations);
        Assert.Contains("/api/v1/invitations/4/decline", server.Asked);
    }

    // ---- the doors to another family ------------------------------------------------------------------------------------------

    /// <summary>"New Family…": the family is created, added and SHOWN — its owner is this account.</summary>
    [Fact]
    public async Task ANewFamilyIsShown()
    {
        var server = new Server()
            .Then("/me", (HttpStatusCode.OK, Me(Smiths)), (HttpStatusCode.OK, Me($"{Smiths}, {Ivanovs}")))
            .On("/families", """{"family": {"id": 5, "name": "Ivanovs"}}""", HttpStatusCode.Created);
        var (session, api, _, _) = Build(server);
        await session.RefreshAsync();
        Assert.Null(await new FamilyDoor(api, session).CreateAsync("Ivanovs"));
        Assert.Equal(5, session.State.CurrentFamilyId);
        Assert.Equal(Gate.Owner, session.State.Gate);
    }

    /// <summary>"Join Another Family…": joined is added and shown; pending leaves the family shown where it is.</summary>
    [Fact]
    public async Task JoiningAnotherFamilyShowsItWhenJoined()
    {
        var joined = new Server()
            .Then("/me", (HttpStatusCode.OK, Me(Smiths)), (HttpStatusCode.OK, Me($"{Smiths}, {Ivanovs}")))
            .On("/families/join", """{"status": "joined"}""")
            .On("/families/5", """{"family": {"id": 5, "name": "Ivanovs"}}""");
        var (session, api, _, _) = Build(joined);
        await session.RefreshAsync();
        Assert.Null(await new FamilyDoor(api, session).JoinAsync("ABCD2345"));
        Assert.Equal(5, session.State.CurrentFamilyId);

        var pending = new Server()
            .Then("/me", (HttpStatusCode.OK, Me(Smiths)), (HttpStatusCode.OK, Me(Smiths, """{"family_id": 8, "family_name": "Petrovs"}""")))
            .On("/families/join", """{"status": "pending"}""");
        var (waiting, waitingApi, _, _) = Build(pending);
        await waiting.RefreshAsync();
        Assert.Null(await new FamilyDoor(waitingApi, waiting).JoinAsync("ABCD2345"));
        Assert.Equal(3, waiting.State.CurrentFamilyId);
        Assert.Equal("Petrovs", Assert.Single(waiting.State.AllPending).FamilyName);
    }

    /// <summary>
    /// The join answer names its family (2026-10-22): joined, THAT family is shown — even when `/me` brought another new one
    /// along — without comparing lists; waiting, the family shown stays.
    /// </summary>
    [Fact]
    public async Task AJoinAnswerThatNamesItsFamilyShowsThatFamily()
    {
        const string Petrovs = """{"id": 8, "name": "Petrovs", "role": "member"}""";
        var server = new Server()
            .Then("/me", (HttpStatusCode.OK, Me(Smiths)), (HttpStatusCode.OK, Me($"{Smiths}, {Ivanovs}, {Petrovs}")))
            .On("/families/join", """{"status": "joined", "family_id": 8}""");
        var (session, api, _, _) = Build(server);
        await session.RefreshAsync();
        Assert.Null(await new FamilyDoor(api, session).JoinAsync("ABCD2345"));
        Assert.Equal(8, session.State.CurrentFamilyId);
        // Nothing but `/me` was asked to find it.
        Assert.DoesNotContain(server.Asked, path => path.StartsWith("/api/v1/families/8", StringComparison.Ordinal));
        Assert.Equal(8, Wire.Decode<JoinAnswer>("""{"status": "pending", "family_id": 8}""")!.FamilyId);
        Assert.Null(Wire.Decode<JoinAnswer>("""{"status": "joined"}""")!.FamilyId);
    }

    /// <summary>The three refusals of a further family, in their own words — the cap with its number.</summary>
    [Fact]
    public void TheDoorsSayWhyTheyAreShut()
    {
        Assert.Equal("You've reached the most families one account can be in (5).",
            DoorSentences.Join(new ApiError(ErrorCodes.TooManyFamilies, "cap", 409), Say, 5));
        Assert.Equal("You've reached the most families one account can be in (5).",
            DoorSentences.Create(new ApiError(ErrorCodes.TooManyFamilies, "cap", 409), Say, 5));
        Assert.Equal("You already own a family, so you can't start another.",
            DoorSentences.Create(new ApiError(ErrorCodes.AlreadyOwnAFamily, "own", 409), Say));
        Assert.Equal("This server doesn't let members start new families.",
            DoorSentences.Create(new ApiError(ErrorCodes.MemberFamiliesDisabled, "off", 403), Say));
        Assert.Equal("This member is also in a family you don't own, so only they can change their password.",
            FamilySentences.For(new ApiError(ErrorCodes.MemberInOtherFamilies, "no", 403), 5, Say));
        Assert.Null(FamilySentences.For(new ApiError(ErrorCodes.Validation, "no", 400), 5, Say));
        Assert.Equal("Start your own family. You'll be its owner and can invite members of your other families.",
            AnotherFamily.Explanation(Say));
    }

    // ---- the rules ------------------------------------------------------------------------------------------------------------

    private static SessionState State(
        IReadOnlyList<FamilyDto> families, long? current = null, bool enabled = true, int cap = 5,
        IReadOnlyList<PendingJoinDto>? pending = null, IReadOnlyList<InvitationDto>? invitations = null)
    {
        var shown = current ?? families.FirstOrDefault()?.Id;
        var family = families.FirstOrDefault(held => held.Id == shown);
        return new SessionState(
            family is null ? Gate.NoFamily : family.IsOwner ? Gate.Owner : Gate.Member,
            Me: new UserDto(7, "anna", "Anna"),
            Family: family,
            Families: families,
            CurrentFamilyId: shown,
            PendingJoins: pending,
            Invitations: invitations,
            MemberFamiliesEnabled: enabled,
            MaxFamiliesPerAccount: cap,
            NamesFamilies: true);
    }

    private static FamilyDto Family(long id, string name, bool owner = false) => new(id, name, Role: owner ? "owner" : "member");

    /// <summary>A family shows the chats that name it — a direct chat in each family it names — and an unnamed chat everywhere.</summary>
    [Fact]
    public void AFamilyListsTheChatsThatNameIt()
    {
        var direct = new ChatDto(44, "direct", "Bob", 9, FamilyIds: [3, 5]);
        Assert.True(FamilyRules.Lists(direct, 3));
        Assert.True(FamilyRules.Lists(direct, 5));
        Assert.False(FamilyRules.Lists(new ChatDto(43, "family", "Ivanovs", FamilyIds: [5]), 3));
        Assert.True(FamilyRules.Lists(new ChatDto(42, "family", "The Smiths"), 3));
        Assert.True(FamilyRules.Lists(new ChatDto(43, "family", "Ivanovs", FamilyIds: [5]), null));
        // An upload is for the family shown when the chat is listed there, else the chat's first.
        Assert.Equal(5, FamilyRules.UploadFamily(direct, 5));
        Assert.Equal(5, FamilyRules.UploadFamily(new ChatDto(43, "family", "Ivanovs", FamilyIds: [5]), 3));
        Assert.Equal(3, FamilyRules.UploadFamily(null, 3));
        // A notification opens its chat where it is listed.
        Assert.Null(FamilyRules.FamilyToOpen(direct, 5));
        Assert.Equal(5, FamilyRules.FamilyToOpen(new ChatDto(43, "family", "Ivanovs", FamilyIds: [5]), 3));
        Assert.Equal("Ivanovs", FamilyRules.NameOf(new ChatDto(43, "family", "Ivanovs", FamilyIds: [5]), [Family(3, "The Smiths"), Family(5, "Ivanovs")]));
    }

    /// <summary>
    /// "New Family…" while the members' door is open, the account owns none of its families and is under the cap; "Join
    /// Another Family…" while under the cap; "Invite…" for an owner with another family to invite from.
    /// </summary>
    [Fact]
    public void TheDoorsAreOfferedOnlyWhileOpen()
    {
        var one = State([Family(3, "The Smiths")]);
        Assert.True(FamilyRules.OffersNewFamily(one));
        Assert.True(FamilyRules.OffersJoinAnother(one));
        Assert.False(FamilyRules.ShowsSwitcher(one));
        Assert.False(FamilyRules.OffersNewFamily(State([Family(3, "The Smiths")], enabled: false)));
        Assert.False(FamilyRules.OffersNewFamily(State([Family(3, "The Smiths"), Family(5, "Ivanovs", owner: true)])));
        Assert.False(FamilyRules.OffersNewFamily(State([Family(3, "The Smiths")], cap: 1)));
        Assert.False(FamilyRules.OffersJoinAnother(State([Family(3, "The Smiths")], cap: 1)));
        Assert.False(FamilyRules.OffersNewFamily(State([])));
        Assert.False(FamilyRules.OffersJoinAnother(State([])));
        var owner = State([Family(3, "The Smiths"), Family(5, "Ivanovs", owner: true)], current: 5);
        Assert.True(FamilyRules.OffersInvite(owner));
        Assert.False(FamilyRules.OffersInvite(State([Family(5, "Ivanovs", owner: true)])));
        Assert.False(FamilyRules.OffersInvite(State([Family(3, "The Smiths"), Family(5, "Ivanovs", owner: true)], current: 3)));
        // One family and something pending: the switcher shows, to show it.
        Assert.True(FamilyRules.ShowsSwitcher(State([Family(3, "The Smiths")], invitations: [Wire.Decode<InvitationDto>(Invitation)!])));
        Assert.True(FamilyRules.ShowsSwitcher(State([Family(3, "The Smiths")], pending: [new PendingJoinDto(8, "Petrovs")])));
        Assert.True(FamilyRules.ShowsSwitcher(owner));
    }

    /// <summary>A password reset is offered only where this owner owns every family the member is in — or the roster does not say.</summary>
    [Fact]
    public void AResetIsOfferedOnlyWhereItWillWork()
    {
        var bob = new MemberDto(9, "bob", "Bob", Role: "member");
        Assert.True(FamilyRules.MayResetPassword(bob, 7, owner: true));
        Assert.True(FamilyRules.MayResetPassword(bob with { PasswordReset = true }, 7, owner: true));
        Assert.False(FamilyRules.MayResetPassword(bob with { PasswordReset = false }, 7, owner: true));
        Assert.False(FamilyRules.MayResetPassword(bob, 7, owner: false));
        Assert.False(FamilyRules.MayResetPassword(bob, 9, owner: true));
        var tools = FamilyText.ToolsFor(bob with { PasswordReset = false }, 7, owner: true, blocked: false);
        Assert.True(tools.Removable);
        Assert.False(tools.ResetPassword);
    }

    /// <summary>
    /// The switcher: every family with the unread of the chats listed in it — a direct chat counting in each family that lists
    /// it — its accessibility label, the requests waiting, the invitations, and the doors.
    /// </summary>
    [Fact]
    public void TheSwitcherListsEveryFamilyWithItsUnread()
    {
        var state = State(
            [Family(3, "The Smiths"), Family(5, "Ivanovs")],
            pending: [new PendingJoinDto(8, "Petrovs")],
            invitations: [Wire.Decode<InvitationDto>(Invitation)!]);
        IReadOnlyList<ChatRowDto> chats =
        [
            new ChatRowDto(new ChatDto(42, "family", "The Smiths", FamilyIds: [3]), UnreadCount: 2),
            new ChatRowDto(new ChatDto(43, "family", "Ivanovs", FamilyIds: [5]), UnreadCount: 1),
            new ChatRowDto(new ChatDto(44, "direct", "Bob", 9, FamilyIds: [3, 5]), UnreadCount: 4),
        ];
        var rows = FamilySwitcher.Rows(state, chats, Say);

        Assert.Equal(
            [SwitcherRowKind.Family, SwitcherRowKind.Family, SwitcherRowKind.Pending, SwitcherRowKind.Invitation,
             SwitcherRowKind.NewFamily, SwitcherRowKind.JoinAnother],
            rows.Select(row => row.Kind));
        Assert.Equal(6, rows[0].Unread);
        Assert.True(rows[0].Current);
        Assert.Equal(5, rows[1].Unread);
        Assert.Equal("Ivanovs, 5 unread", rows[1].AccessibleName);
        Assert.Equal("Waiting for approval", rows[2].Detail);
        Assert.Equal("Carl invited you to “Sidorovs”", rows[3].Detail);
        Assert.Equal("New Family…", rows[4].Title);
        Assert.Equal("Join Another Family…", rows[5].Title);
        Assert.Equal("Switch Family", Say.Get("Switch Family"));
    }

    [Fact]
    public void TheBannerNamesTheInviterAndTheFamily()
    {
        var invitation = Wire.Decode<InvitationDto>(Invitation)!;
        Assert.Equal("Carl invited you to “Sidorovs”", InvitationText.Banner(invitation, Say));
        Assert.Equal("Deleted account invited you to “Sidorovs”",
            InvitationText.Banner(invitation with { InvitedBy = invitation.InvitedBy with { Deleted = true } }, Say));
    }

    // ---- inviting (owner) -------------------------------------------------------------------------------------------------------

    /// <summary>
    /// "Invite…" lists the members of the owner's OTHER families who are not in this one, each once and in name order, marked
    /// "Invited" when an invitation is pending — never the owner, a former member, or somebody already here.
    /// </summary>
    [Fact]
    public void TheInvitePickerListsTheOtherFamiliesMembers()
    {
        var chats = new ChatStore(cache, () => 7, () => 5);
        chats.Replace(5, [new MemberDto(7, "anna", "Anna", Role: "owner"), new MemberDto(9, "bob", "Bob")]);
        chats.Replace(3, [new MemberDto(7, "anna", "Anna"), new MemberDto(9, "bob", "Bob"), new MemberDto(12, "zoe", "Zoe"),
            new MemberDto(11, "carl", "Carl", Role: "owner")], [new MemberDto(13, "dan", "Dan", HasLeft: true)]);
        chats.Replace(8, [new MemberDto(12, "zoe", "Zoe"), new MemberDto(14, "eve", "Eve")]);
        var state = State([Family(3, "The Smiths"), Family(5, "Ivanovs", owner: true), Family(8, "Petrovs")], current: 5);
        var pending = new List<InvitationDto> { Wire.Decode<InvitationDto>(Invitation)! with { User = new UserDto(14, "eve", "Eve") } };

        var candidates = new InviteModel(new ApiClient(new HttpClient(new Server()), ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore()), chats)
            .Candidates(state, pending);

        Assert.Equal(["Carl", "Eve", "Zoe"], candidates.Select(candidate => candidate.Member.DisplayName));
        Assert.Equal([false, true, false], candidates.Select(candidate => candidate.Invited));
    }

    /// <summary>The owner's three invitation calls, for the family shown.</summary>
    [Fact]
    public async Task TheOwnerInvitesListsAndWithdraws()
    {
        var server = new Server()
            .On("/families/5/invitations", $$"""{"invitation": {{Invitation}}, "invitations": [{{Invitation}}]}""")
            .On("/families/5/invitations/4", null, HttpStatusCode.NoContent);
        var invites = new InviteModel(new ApiClient(new HttpClient(server), ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken")),
            new ChatStore(cache));
        var (invitation, error) = await invites.InviteAsync(5, 9);
        Assert.Null(error);
        Assert.Equal(4, invitation!.Id);
        var (pending, failed) = await invites.PendingAsync(5);
        Assert.Null(failed);
        Assert.Single(pending);
        Assert.Null(await invites.WithdrawAsync(5, 4));
        Assert.Equal("""{"user_id":9}""", server.Bodies[0]);
    }

    // ---- the family shown, everywhere ---------------------------------------------------------------------------------------

    /// <summary>
    /// The chat list is the family SHOWN's: its family chat pinned first, its assistant's chat, and the direct chats it lists —
    /// none of another family's.
    /// </summary>
    [Fact]
    public void TheChatListIsTheFamilyShowns()
    {
        long? shown = 3;
        var chats = new ChatStore(cache, () => 7, () => shown);
        chats.Replace([
            new ChatRowDto(new ChatDto(42, "family", "The Smiths", FamilyIds: [3])),
            new ChatRowDto(new ChatDto(43, "family", "Ivanovs", FamilyIds: [5])),
            new ChatRowDto(new ChatDto(44, "direct", "Bob", 9, FamilyIds: [3, 5])),
            new ChatRowDto(new ChatDto(45, "ai", "Assistant", FamilyIds: [3])),
            new ChatRowDto(new ChatDto(46, "ai", "Assistant", FamilyIds: [5])),
        ]);
        var list = new ChatListModel(chats, () => 7, Say);
        var rows = list.Rows(DateTimeOffset.Now);
        Assert.Equal(42, rows[0].Chat.Id);
        Assert.Equal([42L, 44L, 45L], rows.Select(row => row.Chat.Id).Order());
        Assert.Equal(5, list.Held.Count);

        shown = 5;
        rows = list.Rows(DateTimeOffset.Now);
        Assert.Equal(43, rows[0].Chat.Id);
        Assert.Equal([43L, 44L, 46L], rows.Select(row => row.Chat.Id).Order());
    }

    /// <summary>A family chat's notification is titled with THAT chat's family's name, and a note with its wall's — not the shown family's.</summary>
    [Fact]
    public void ANotificationIsTitledWithItsOwnFamily()
    {
        var chats = new ChatStore(cache, () => 7, () => 3);
        chats.Replace([
            new ChatRowDto(new ChatDto(42, "family", "The Smiths", FamilyIds: [3])),
            new ChatRowDto(new ChatDto(43, "family", "Ivanovs", FamilyIds: [5])),
        ]);
        chats.Replace(5, [new MemberDto(11, "carl", "Carl")]);
        var rules = new NotificationRules(chats, Say)
        {
            Wanted = true,
            FamilyName = "The Smiths",
            Families = [Family(3, "The Smiths"), Family(5, "Ivanovs")],
        };
        var toast = rules.ForMessage(new MessageDto(1, 43, 11, "c-1", "hi", "2026-10-22T10:00:00Z"))!;
        Assert.Equal("Ivanovs — Carl", toast.Title);
        var note = rules.ForNote(new NoteDto(2, 11, "text", "Bread", BoardSeq: 9, FamilyId: 5), isNews: true)!;
        Assert.Equal("Ivanovs — Carl", note.Title);
        Assert.Equal(5, note.FamilyId);
        // The wall's notification says whose, and a click reads it back.
        var arguments = ToastArguments.For(note).ToDictionary(pair => pair.Key, pair => pair.Value);
        Assert.Equal(new ToastTarget(null, 5), ToastArguments.Parse(arguments));
        Assert.Equal(new ToastTarget(null), ToastArguments.Parse(new Dictionary<string, string> { ["board"] = "1", ["family"] = "-2" }));
    }

    /// <summary>
    /// Leaving one of several says which family and what it costs; with one family the dialog is today's, word for word; and
    /// leaving as the last member still says it deletes the family.
    /// </summary>
    [Fact]
    public void TheLeaveDialogNamesTheFamilyWhenThereAreSeveral()
    {
        var several = new LeaveContext(LeaveKind.Member, FamilyName: "Ivanovs");
        Assert.Equal("Leave “Ivanovs”?", SettingsText.LeaveTitle(several, Say));
        Assert.Equal("You'll lose access to this family's chats. Direct chats stay with anyone you share another family with.",
            SettingsText.LeaveMessage(several, Say));
        Assert.Equal(SettingsText.LeaveMessage(several, Say),
            SettingsText.LeaveMessage(new LeaveContext(LeaveKind.Successor, "Bob", "Ivanovs"), Say));
        var one = new LeaveContext(LeaveKind.Member);
        Assert.Equal("Leave the family?", SettingsText.LeaveTitle(one, Say));
        Assert.Equal("You'll lose access to the family chat and your direct chats. Your history returns if you rejoin.",
            SettingsText.LeaveMessage(one, Say));
        var last = new LeaveContext(LeaveKind.LastMember, FamilyName: "Ivanovs");
        Assert.Equal("Delete the family?", SettingsText.LeaveTitle(last, Say));
        Assert.Equal("You're the only member left. Leaving deletes the family and everything in it.", SettingsText.LeaveMessage(last, Say));
    }

    /// <summary>Leaving names the family left — the one shown — and only that family goes.</summary>
    [Fact]
    public async Task LeavingNamesTheFamily()
    {
        var server = new Server().On("/families/5/leave", null, HttpStatusCode.NoContent);
        var family = new FamilyModel(new ApiClient(new HttpClient(server), ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken")),
            new ChatStore(cache));
        var (successor, error) = await family.LeaveAsync(5);
        Assert.Null(error);
        Assert.Null(successor);
        Assert.Equal(["/api/v1/families/5/leave"], server.Asked);
    }

    /// <summary>A queued message's uploads are for the family its chat is in, whatever is shown by the time they go.</summary>
    [Fact]
    public async Task QueuedUploadsAreForTheChatsFamily()
    {
        var outbox = new OutboxStore(cache);
        outbox.Queue(new OutboxRow("k", 43, "look", PendingFiles: ["a.jpg"], StagedFiles: ["a.jpg"]));
        var server = new Server().On("/attachments", """{"attachment": {"id": 34, "kind": "photo"}}""");
        var media = new MediaOutbox(
            outbox,
            new ApiClient(new HttpClient(server), ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken")) { FamilyScope = 3 },
            new OneFile(),
            chatId => chatId == 43 ? 5 : null);
        Assert.Equal(1, await media.PushAsync());
        Assert.EndsWith("&family_id=5", Assert.Single(server.Asked), StringComparison.Ordinal);
    }

    private sealed class OneFile : IMediaStore
    {
        public StagedMedia? Read(string handle) => new("photo", "image/jpeg", new byte[] { 1, 2, 3 }, 16, 16);

        public void Sweep(IReadOnlySet<string> keep)
        {
        }
    }

    // ---- frames, through the live connection ------------------------------------------------------------------------------------

    private sealed class Socket : ILiveSocket
    {
        public bool IsConnected => false;

        public event Action<ServerFrame>? Frame;

        // Never raised here: this socket never connects, so no pass runs under the frames.
        public event Action? Connected { add { } remove { } }

        public event Action? Disconnected { add { } remove { } }

        public event Action? SessionExpired { add { } remove { } }

        public async Task RunAsync(CancellationToken ct)
        {
            try
            {
                await Task.Delay(Timeout.Infinite, ct);
            }
            catch (OperationCanceledException)
            {
                // Stopped.
            }
        }

        public Task<bool> TrySend(string frame, CancellationToken ct = default) => Task.FromResult(false);

        public void Hear(ServerFrame frame) => Frame?.Invoke(frame);
    }

    /// <summary>
    /// The live connection hands the session what the frames say about families: an invitation is listed, and a `member_left`
    /// naming the reader drops THAT family's rows and shows what is left.
    /// </summary>
    [Fact]
    public async Task FramesAboutTheReadersFamiliesReachTheSession()
    {
        var server = new Server().Then("/me",
            (HttpStatusCode.OK, Me($"{Smiths}, {Ivanovs}")),
            (HttpStatusCode.OK, Me(Smiths)));
        var (session, api, _, remembered) = Build(server);
        remembered.Remember(7, 5);
        await session.RefreshAsync();
        var chats = new ChatStore(cache, () => 7, () => session.State.CurrentFamilyId);
        var board = new BoardStore(cache, () => session.State.CurrentFamilyId);
        chats.Replace([new ChatRowDto(new ChatDto(43, "family", "Ivanovs", FamilyIds: [5]))]);
        var socket = new Socket();
        var router = new FrameRouter(chats, board);
        var sending = new SendPipeline(socket, new OutboxStore(cache), chats, api);
        await using var live = new LiveConnection(session, socket, new Resync(api, chats, board), sending, router);

        socket.Hear(ServerFrame.Parse($$"""{"type": "invitation", "invitation": {{Invitation}}}""")!);
        Assert.Equal(4, Assert.Single(session.State.AllInvitations).Id);
        socket.Hear(ServerFrame.Parse("""{"type": "invitation_removed", "invitation_id": 4}""")!);
        Assert.Empty(session.State.AllInvitations);

        var moved = new TaskCompletionSource();
        session.Changed += state =>
        {
            if (state.CurrentFamilyId == 3)
            {
                moved.TrySetResult();
            }
        };
        socket.Hear(ServerFrame.Parse("""{"type": "member_left", "family_id": 5, "user_id": 7}""")!);
        await moved.Task.WaitAsync(Patience);
        Assert.Null(chats.Chat(43));
        Assert.Equal([3L], session.State.AllFamilies.Select(family => family.Id));
    }
}
