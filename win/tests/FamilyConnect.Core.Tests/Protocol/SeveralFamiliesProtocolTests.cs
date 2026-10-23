using System.Net;
using System.Text;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.Core.Tests.Protocol;

/// <summary>
/// One account in several families on the wire (docs/protocol.md, "Several families", issue #88): the capability on every
/// request and on the socket, the family named in the path, the upload's and the new group's family, the invitation endpoints,
/// <c>GET /me</c>'s list, the two new frames, and a resync that reads EVERY family into its own rows.
/// </summary>
public class SeveralFamiliesProtocolTests : IDisposable
{
    private readonly Database database = Database.OpenInMemory();

    public void Dispose() => database.Dispose();

    /// <summary>Answers by path — the whole path, query aside — and keeps every request.</summary>
    private sealed class Server : HttpMessageHandler
    {
        private readonly List<(string Path, HttpStatusCode Status, string? Json)> routes = [];

        public List<HttpRequestMessage> Sent { get; } = [];

        public List<string?> Bodies { get; } = [];

        public List<string> Asked => [.. Sent.Select(request => request.RequestUri!.PathAndQuery)];

        public Server On(string what, string? json, HttpStatusCode status = HttpStatusCode.OK)
        {
            routes.Add(("/api/v1" + what, status, json));
            return this;
        }

        protected override async Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)
        {
            Sent.Add(request);
            Bodies.Add(request.Content is null ? null : await request.Content.ReadAsStringAsync(cancellationToken));
            var path = request.RequestUri!.AbsolutePath;
            foreach (var (what, status, json) in routes)
            {
                if (what == path)
                {
                    var response = new HttpResponseMessage(status);
                    if (json is not null)
                    {
                        response.Content = new StringContent(json, Encoding.UTF8, "application/json");
                    }
                    return response;
                }
            }
            throw new InvalidOperationException($"no route for {path}");
        }
    }

    private static ApiClient Client(Server server) =>
        new(new HttpClient(server), ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken"));

    private const string Empty = "{}";

    // ---- the capability -----------------------------------------------------------------------------------------------------

    /// <summary>EVERY request says it knows about several families: without it the server answers about the home family alone.</summary>
    [Fact]
    public async Task EveryRequestCarriesTheFamiliesHeader()
    {
        var server = new Server().On("/me", """{"user": {"id": 7, "username": "anna", "display_name": "Anna"}}""")
            .On("/chats", """{"chats": []}""");
        var api = Client(server);
        await api.Me();
        await api.Chats();
        Assert.Equal(2, server.Sent.Count);
        Assert.All(server.Sent, request =>
            Assert.Equal("1", Assert.Single(request.Headers.GetValues(ApiClient.FamiliesHeader))));
        Assert.Equal("X-FC-Families", ApiClient.FamiliesHeader);
        // And on the socket, as a query parameter — a browser cannot set headers there.
        Assert.Equal("?families=1", api.SocketUrl.Query);
    }

    // ---- the family in the path -----------------------------------------------------------------------------------------------

    /// <summary>
    /// With no family shown every family endpoint keeps its old spelling — `mine`, and the forms without an id — so an account
    /// in one family on a server from before asks exactly what it always asked.
    /// </summary>
    [Fact]
    public async Task WithNoFamilyShownTheOldPathsAreAskedUnchanged()
    {
        var server = new Server()
            .On("/families/mine", """{"family": {"id": 3, "name": "The Smiths"}}""")
            .On("/families/mine/board", """{"notes": [], "max_board_seq": 0}""")
            .On("/families/join-requests", """{"requests": []}""")
            .On("/families/leave", null, HttpStatusCode.NoContent);
        var api = Client(server);
        await api.Family();
        await api.Board();
        await api.JoinRequests();
        await api.LeaveFamily();
        Assert.Equal(["/api/v1/families/mine", "/api/v1/families/mine/board", "/api/v1/families/join-requests", "/api/v1/families/leave"], server.Asked);
    }

    /// <summary>
    /// The family SHOWN is named on every family endpoint (docs/protocol.md, "Naming a family") — the roster, the board, the
    /// pack, the owner's settings and lists, leaving, a member's removal, birthday and password, reports and statistics. A block
    /// keeps the form without a family: it is a pair, and holds in every family the two share.
    /// </summary>
    [Fact]
    public async Task TheFamilyShownIsNamedOnEveryFamilyEndpoint()
    {
        var server = new Server()
            .On("/families/5", """{"family": {"id": 5, "name": "Ivanovs"}}""")
            .On("/families/5/board", """{"notes": [], "max_board_seq": 0}""")
            .On("/families/5/board/notes", """{"note": {"id": 1, "author_id": 7}}""")
            .On("/families/5/pack", """{"items": [], "max_pack_seq": 0}""")
            .On("/families/5/stats", """{"totals": {"members": 1}}""")
            .On("/families/5/invite-code/rotate", """{"invite_code": "ABCD2345"}""")
            .On("/families/5/join-requests", """{"requests": []}""")
            .On("/families/5/reports", """{"reports": []}""")
            .On("/families/5/members/9", null, HttpStatusCode.NoContent)
            .On("/families/5/members/9/password", null, HttpStatusCode.NoContent)
            .On("/families/5/leave", null, HttpStatusCode.NoContent)
            .On("/families/members/9/block", null, HttpStatusCode.NoContent);
        var api = Client(server);
        api.FamilyScope = 5;
        Assert.Equal(5, api.FamilyScope);
        await api.Family();
        await api.Board();
        await api.CreateNote(new NoteRequest("Milk", "yellow", 0.5, 0.5));
        await api.Pack();
        await api.Stats();
        await api.RotateInviteCode();
        await api.JoinRequests();
        await api.Reports();
        await api.RemoveMember(9);
        await api.ResetMemberPassword(9, "correct horse");
        await api.LeaveFamily();
        await api.Block(9);
        Assert.Equal(
            ["/api/v1/families/5", "/api/v1/families/5/board", "/api/v1/families/5/board/notes", "/api/v1/families/5/pack",
             "/api/v1/families/5/stats", "/api/v1/families/5/invite-code/rotate", "/api/v1/families/5/join-requests",
             "/api/v1/families/5/reports", "/api/v1/families/5/members/9", "/api/v1/families/5/members/9/password",
             "/api/v1/families/5/leave", "/api/v1/families/members/9/block"],
            server.Asked);
        // Cleared, the home family is `mine` again.
        api.FamilyScope = null;
        Assert.Null(api.FamilyScope);
    }

    /// <summary>A resync names each family itself, whichever is shown: one family's catch-up is never another's.</summary>
    [Fact]
    public async Task AFamilyNamedOutrightIsAskedWhateverIsShown()
    {
        var server = new Server()
            .On("/families/3", """{"family": {"id": 3, "name": "The Smiths"}}""")
            .On("/families/3/board", """{"notes": [], "max_board_seq": 0}""")
            .On("/families/3/board/changes", """{"notes": []}""")
            .On("/families/3/pack", """{"items": [], "max_pack_seq": 0}""")
            .On("/families/3/pack/changes", """{"items": []}""")
            .On("/families/3/leave", null, HttpStatusCode.NoContent);
        var api = Client(server);
        api.FamilyScope = 5;
        await api.Family(3);
        await api.Board(3);
        await api.BoardChanges(3, 12, 50);
        await api.Pack(3);
        await api.PackChanges(3, 4, 50);
        await api.LeaveFamily(3);
        Assert.Equal(
            ["/api/v1/families/3", "/api/v1/families/3/board", "/api/v1/families/3/board/changes?after_seq=12&limit=50",
             "/api/v1/families/3/pack", "/api/v1/families/3/pack/changes?after_seq=4&limit=50", "/api/v1/families/3/leave"],
            server.Asked);
    }

    /// <summary>
    /// An upload names the family it is FOR — the one shown, or the one asked — and a new group is made in the family shown;
    /// with none shown neither says anything, which is the home family.
    /// </summary>
    [Fact]
    public async Task UploadsAndNewGroupsNameTheirFamily()
    {
        var server = new Server()
            .On("/attachments", """{"attachment": {"id": 34, "kind": "photo"}}""")
            .On("/chats/groups", """{"chat": {"id": 77, "kind": "group", "title": "Cousins", "family_ids": [5]}}""");
        var api = Client(server);
        await api.Upload("photo", "image/jpeg", new byte[] { 1, 2, 3 });
        await api.CreateGroup("Cousins", [9]);
        api.FamilyScope = 5;
        await api.Upload("photo", "image/jpeg", new byte[] { 1, 2, 3 });
        await api.Upload("photo", "image/jpeg", new byte[] { 1, 2, 3 }, familyId: 3);
        await api.UploadLocation(55.75, 37.61, 12, familyId: 3);
        var group = await api.CreateGroup("Cousins", [9]);

        var asked = server.Asked;
        Assert.DoesNotContain("family_id", asked[0], StringComparison.Ordinal);
        Assert.DoesNotContain("family_id", server.Bodies[1]!, StringComparison.Ordinal);
        Assert.EndsWith("&family_id=5", asked[2], StringComparison.Ordinal);
        Assert.EndsWith("&family_id=3", asked[3], StringComparison.Ordinal);
        Assert.EndsWith("&family_id=3", asked[4], StringComparison.Ordinal);
        Assert.Contains("\"family_id\":5", server.Bodies[5]!, StringComparison.Ordinal);
        Assert.Equal([5L], group.Value!.Chat.FamilyIds!);
    }

    /// <summary>The five invitation endpoints, as the table spells them.</summary>
    [Fact]
    public async Task InvitationsAreAskedWhereTheTableSays()
    {
        const string Invitation =
            """{"id": 4, "family_id": 5, "family_name": "Ivanovs", "invited_by": {"id": 7, "username": "anna", "display_name": "Anna"}, "user": {"id": 9, "username": "bob", "display_name": "Bob"}, "created_at": "2026-10-22T10:00:00Z"}""";
        var server = new Server()
            .On("/families/5/invitations", $$"""{"invitation": {{Invitation}}}""", HttpStatusCode.Created)
            .On("/families/5/invitations/4", null, HttpStatusCode.NoContent)
            .On("/invitations/4/accept", """{"family": {"id": 5, "name": "Ivanovs"}}""")
            .On("/invitations/4/decline", null, HttpStatusCode.NoContent);
        var api = Client(server);

        var invited = await api.Invite(5, 9);
        Assert.True(invited.Ok);
        Assert.Equal("Ivanovs", invited.Value!.Invitation.FamilyName);
        Assert.Equal("Anna", invited.Value.Invitation.InvitedBy.DisplayName);
        Assert.Equal(9, invited.Value.Invitation.User.Id);
        Assert.Equal("""{"user_id":9}""", server.Bodies[0]);
        Assert.True((await api.WithdrawInvitation(5, 4)).Ok);
        var accepted = await api.AcceptInvitation(4);
        Assert.Equal(5, accepted.Value!.Family.Id);
        Assert.True((await api.DeclineInvitation(4)).Ok);
        Assert.Equal(
            [HttpMethod.Post, HttpMethod.Delete, HttpMethod.Post, HttpMethod.Post],
            server.Sent.Select(request => request.Method));
        Assert.Equal(
            ["/api/v1/families/5/invitations", "/api/v1/families/5/invitations/4", "/api/v1/invitations/4/accept", "/api/v1/invitations/4/decline"],
            server.Asked);
    }

    [Fact]
    public async Task AFamilysPendingInvitationsAreReadOldestFirst()
    {
        var server = new Server().On("/families/5/invitations",
            """{"invitations": [{"id": 4, "family_id": 5, "family_name": "Ivanovs", "invited_by": {"id": 7, "username": "anna", "display_name": "Anna"}, "user": {"id": 9, "username": "bob", "display_name": "Bob"}}]}""");
        var answer = await Client(server).Invitations(5);
        Assert.Equal(4, Assert.Single(answer.Value!.Invitations!).Id);
    }

    // ---- GET /me --------------------------------------------------------------------------------------------------------------

    /// <summary>`GET /me`'s list, each family with its own role, home first; and the requests, invitations and the two numbers.</summary>
    [Fact]
    public void MeCarriesEveryFamilyWithItsRole()
    {
        var me = Wire.Decode<MeResponse>(
            """
            {"user": {"id": 7, "username": "anna", "display_name": "Anna"},
             "family": {"id": 3, "name": "The Smiths"}, "role": "member",
             "families": [{"id": 3, "name": "The Smiths", "role": "member"}, {"id": 5, "name": "Ivanovs", "role": "owner"}],
             "pending_join_requests": [{"family_id": 8, "family_name": "Petrovs", "created_at": "2026-10-22T10:00:00Z"}],
             "invitations": [{"id": 4, "family_id": 9, "family_name": "Sidorovs", "invited_by": {"id": 11, "username": "carl", "display_name": "Carl"}, "user": {"id": 7, "username": "anna", "display_name": "Anna"}}],
             "member_families_enabled": true, "max_families_per_account": 5}
            """)!;
        Assert.Equal([3L, 5L], me.AllFamilies.Select(family => family.Id));
        Assert.False(me.AllFamilies[0].IsOwner);
        Assert.True(me.AllFamilies[1].IsOwner);
        Assert.Equal("Petrovs", Assert.Single(me.AllPending).FamilyName);
        Assert.Equal("Carl", Assert.Single(me.Invitations!).InvitedBy.DisplayName);
        Assert.True(me.MemberFamiliesEnabled);
        Assert.Equal(5, me.FamilyCap);
    }

    /// <summary>A server from before: its one family, with the role beside it, is the list; nothing else is open.</summary>
    [Fact]
    public void AnOlderMeIsReadAsItsOneFamily()
    {
        var me = Wire.Decode<MeResponse>(
            """
            {"user": {"id": 7, "username": "anna", "display_name": "Anna"},
             "family": {"id": 3, "name": "The Smiths"}, "role": "owner",
             "pending_join_request": {"family_id": 8, "family_name": "Petrovs"}}
            """)!;
        var only = Assert.Single(me.AllFamilies);
        Assert.Equal(3, only.Id);
        Assert.True(only.IsOwner);
        Assert.Null(me.Families);
        Assert.Equal(8, Assert.Single(me.AllPending).FamilyId);
        Assert.False(me.MemberFamiliesEnabled);
        Assert.Equal(1, me.FamilyCap);
        Assert.Empty(Wire.Decode<MeResponse>("""{"user": {"id": 7, "username": "anna", "display_name": "Anna"}}""")!.AllFamilies);
    }

    /// <summary>The objects' new fields: a chat's families, a note's and an item's family, a member's password_reset.</summary>
    [Fact]
    public void TheObjectsSayWhoseTheyAre()
    {
        Assert.Equal([3L, 5L], Wire.Decode<ChatDto>("""{"id": 42, "kind": "direct", "title": "Bob", "family_ids": [3, 5]}""")!.FamilyIds!);
        Assert.Null(Wire.Decode<ChatDto>("""{"id": 42, "kind": "family", "title": "The Smiths"}""")!.FamilyIds);
        Assert.Equal(5, Wire.Decode<NoteDto>("""{"id": 1, "author_id": 7, "family_id": 5}""")!.FamilyId);
        Assert.Equal(5, Wire.Decode<PackItemDto>("""{"id": 1, "pack_seq": 2, "family_id": 5}""")!.FamilyId);
        Assert.False(Wire.Decode<MemberDto>("""{"id": 9, "username": "bob", "display_name": "Bob", "password_reset": false}""")!.PasswordReset);
        Assert.Null(Wire.Decode<MemberDto>("""{"id": 9, "username": "bob", "display_name": "Bob"}""")!.PasswordReset);
    }

    // ---- frames ---------------------------------------------------------------------------------------------------------------

    [Fact]
    public void TheTwoInvitationFramesParse()
    {
        var invitation = Assert.IsType<ServerFrame.Invitation>(ServerFrame.Parse(
            """{"type": "invitation", "invitation": {"id": 4, "family_id": 5, "family_name": "Ivanovs", "invited_by": {"id": 7, "username": "anna", "display_name": "Anna"}, "user": {"id": 9, "username": "bob", "display_name": "Bob"}}}"""));
        Assert.Equal(4, invitation.Value.Id);
        Assert.Equal(5, invitation.Value.FamilyId);
        Assert.Equal(4, Assert.IsType<ServerFrame.InvitationRemoved>(
            ServerFrame.Parse("""{"type": "invitation_removed", "invitation_id": 4}""")).InvitationId);
        // Nothing to apply is nothing at all.
        Assert.Null(ServerFrame.Parse("""{"type": "invitation_removed"}"""));
        Assert.Null(ServerFrame.Parse("""{"type": "invitation"}"""));
    }

    /// <summary>
    /// The OWNER's copy of <c>member_joined</c> says whether the newcomer is theirs to reset; everybody else's says nothing, and
    /// nothing is not "no" — nor is a value that is not a boolean, which a guess would turn into one answer or the other.
    /// </summary>
    [Fact]
    public void AJoinSaysWhetherTheNewcomerIsTheOwnersToReset()
    {
        const string user = """{"id": 12, "username": "dana", "display_name": "Dana"}""";
        Assert.False(Assert.IsType<ServerFrame.MemberJoined>(ServerFrame.Parse(
            $$"""{"type": "member_joined", "family_id": 5, "user": {{user}}, "password_reset": false}""")).PasswordReset);
        Assert.True(Assert.IsType<ServerFrame.MemberJoined>(ServerFrame.Parse(
            $$"""{"type": "member_joined", "family_id": 5, "user": {{user}}, "password_reset": true}""")).PasswordReset);
        Assert.Null(Assert.IsType<ServerFrame.MemberJoined>(ServerFrame.Parse(
            $$"""{"type": "member_joined", "family_id": 5, "user": {{user}}}""")).PasswordReset);
        Assert.Null(Assert.IsType<ServerFrame.MemberJoined>(ServerFrame.Parse(
            $$"""{"type": "member_joined", "family_id": 5, "user": {{user}}, "password_reset": "no"}""")).PasswordReset);
    }

    /// <summary>
    /// The flag a join carries reaches the roster, and a join WITHOUT one — everybody else's copy, or the owner's racing a
    /// roster read that already said — keeps the flag held: a newcomer also in a family this owner does not own is never
    /// offered a reset the server would refuse with <c>member_in_other_families</c>.
    /// </summary>
    [Fact]
    public void AJoinWithoutTheFlagKeepsTheOneHeld()
    {
        long? shown = 5;
        var chats = new ChatStore(database, () => 7, () => shown);
        chats.Replace(5, [new MemberDto(7, "anna", "Anna", Role: "owner")]);
        var router = new FrameRouter(chats, new BoardStore(database, () => shown));
        bool? Flag() => chats.MembersOf(5).Single(member => member.Id == 12).PasswordReset;

        router.Hear(ServerFrame.Parse("""{"type": "member_joined", "family_id": 5, "user": {"id": 12, "username": "dana", "display_name": "Dana"}, "password_reset": false}""")!);
        Assert.False(Flag());
        Assert.False(chats.Members().Single(member => member.Id == 12).PasswordReset);

        router.Hear(ServerFrame.Parse("""{"type": "member_joined", "family_id": 5, "user": {"id": 12, "username": "dana", "display_name": "Dana"}}""")!);
        Assert.False(Flag());

        router.Hear(ServerFrame.Parse("""{"type": "member_joined", "family_id": 5, "user": {"id": 12, "username": "dana", "display_name": "Dana"}, "password_reset": true}""")!);
        Assert.True(Flag());

        // A newcomer nothing was ever said about: the old rule, where the reset is offered.
        router.Hear(ServerFrame.Parse("""{"type": "member_joined", "family_id": 5, "user": {"id": 13, "username": "eve", "display_name": "Eve"}}""")!);
        Assert.Null(chats.MembersOf(5).Single(member => member.Id == 13).PasswordReset);
    }

    /// <summary>
    /// A JOIN NEVER DOWNGRADES. A family started in the app is announced by a <c>member_joined</c> naming its CREATOR, who is
    /// its owner — and that frame may land after the roster read that said so. The role held stays; a newcomer, and a former
    /// member back again, are members; the reset flag still follows the frame when it carries one.
    /// </summary>
    [Fact]
    public void AJoinNamingTheOwnerLeavesThemTheOwner()
    {
        long? shown = 5;
        var chats = new ChatStore(database, () => 7, () => shown);
        chats.Replace(3, [new MemberDto(7, "anna", "Anna", Role: "member"), new MemberDto(9, "bob", "Bob", Role: "owner")]);
        chats.Replace(5, [new MemberDto(7, "anna", "Anna", Role: "owner"), new MemberDto(11, "carl", "Carl", Role: "member", PasswordReset: true)]);
        var router = new FrameRouter(chats, new BoardStore(database, () => shown));

        router.Hear(ServerFrame.Parse("""{"type": "member_joined", "family_id": 5, "user": {"id": 7, "username": "anna", "display_name": "Anna"}}""")!);
        Assert.True(chats.MembersOf(5).Single(member => member.Id == 7).Owner);
        Assert.True(chats.Members().Single(member => member.Id == 7).Owner);
        // Family 3's role is family 3's.
        Assert.False(chats.MembersOf(3).Single(member => member.Id == 7).Owner);

        // A member named again stays a member, and the flag the owner's copy carries still lands.
        router.Hear(ServerFrame.Parse("""{"type": "member_joined", "family_id": 5, "user": {"id": 11, "username": "carl", "display_name": "Carl"}, "password_reset": false}""")!);
        var carl = chats.MembersOf(5).Single(member => member.Id == 11);
        Assert.Equal("member", carl.Role);
        Assert.False(carl.PasswordReset);

        // A newcomer is a member; so is somebody who left and came back, whatever they were before.
        router.Hear(ServerFrame.Parse("""{"type": "member_joined", "family_id": 3, "user": {"id": 12, "username": "dana", "display_name": "Dana"}}""")!);
        Assert.Equal("member", chats.MembersOf(3).Single(member => member.Id == 12).Role);
        router.Hear(ServerFrame.Parse("""{"type": "member_left", "family_id": 3, "user_id": 9}""")!);
        router.Hear(ServerFrame.Parse("""{"type": "member_joined", "family_id": 3, "user": {"id": 9, "username": "bob", "display_name": "Bob"}}""")!);
        var bob = chats.MembersOf(3).Single(member => member.Id == 9);
        Assert.False(bob.HasLeft);
        Assert.Equal("member", bob.Role);
    }

    /// <summary>
    /// A <c>chat_updated</c> does not un-hide a direct chat with somebody this reader has BLOCKED. The server sends a direct
    /// chat's frame with no regard to blocks — a held chat moving family when the reader leaves one — while <c>GET /chats</c>
    /// and <c>member_blocked</c> keep it off the list, for the blocker alone (docs/protocol.md, "Blocking a member"). The frame
    /// is still written, so the chat comes back whole — in the families it said — on unblock.
    /// </summary>
    [Fact]
    public void AChatUpdatedKeepsABlockedPeersChatHidden()
    {
        long? shown = 3;
        var chats = new ChatStore(database, () => 7, () => shown);
        chats.Replace([
            new ChatRowDto(new ChatDto(42, "family", "The Smiths", FamilyIds: [3])),
            new ChatRowDto(new ChatDto(44, "direct", "Bob", 9, FamilyIds: [3, 5])),
            new ChatRowDto(new ChatDto(45, "direct", "Carl", 11, FamilyIds: [3])),
        ]);
        var router = new FrameRouter(chats, new BoardStore(database, () => shown));
        router.Hear(ServerFrame.Parse("""{"type": "member_blocked", "user_id": 9, "blocked": true}""")!);
        Assert.False(chats.IsListed(44));

        router.Hear(ServerFrame.Parse("""{"type": "chat_updated", "chat": {"id": 44, "kind": "direct", "title": "Bob", "peer_user_id": 9, "family_ids": [5]}}""")!);
        Assert.False(chats.IsListed(44));
        Assert.DoesNotContain(chats.Chats(), row => row.Chat.Id == 44);
        Assert.Equal([5L], chats.Chat(44)!.Chat.FamilyIds!);

        // A direct chat this device never held, with a blocked peer, arrives hidden too.
        router.Hear(ServerFrame.Parse("""{"type": "member_blocked", "user_id": 14, "blocked": true}""")!);
        router.Hear(ServerFrame.Parse("""{"type": "chat_updated", "chat": {"id": 46, "kind": "direct", "title": "Eve", "peer_user_id": 14, "family_ids": [3]}}""")!);
        Assert.NotNull(chats.Chat(46));
        Assert.False(chats.IsListed(46));

        // Anybody else's chat, a group's included, is put on the list as before.
        router.Hear(ServerFrame.Parse("""{"type": "chat_updated", "chat": {"id": 45, "kind": "direct", "title": "Carl", "peer_user_id": 11, "family_ids": [3, 5]}}""")!);
        Assert.True(chats.IsListed(45));
        router.Hear(ServerFrame.Parse("""{"type": "chat_updated", "chat": {"id": 47, "kind": "group", "title": "Cousins", "member_ids": [7, 9], "admin_user_id": 9, "family_ids": [3]}}""")!);
        Assert.True(chats.IsListed(47));

        // Unblocked, it is back — with what the frame said.
        router.Hear(ServerFrame.Parse("""{"type": "member_blocked", "user_id": 9, "blocked": false}""")!);
        Assert.True(chats.IsListed(44));
        Assert.Equal([5L], chats.Chat(44)!.Chat.FamilyIds!);
    }

    /// <summary>
    /// A family frame is applied to the family it NAMES (docs/protocol.md, "Frames"): a join, a leave and an owner in family 5
    /// leave family 3's roster alone, and a note and an item go onto family 5's wall and pack whatever is shown.
    /// </summary>
    [Fact]
    public void FamilyFramesGoToTheFamilyTheyName()
    {
        long? shown = 3;
        var chats = new ChatStore(database, () => 7, () => shown);
        var board = new BoardStore(database, () => shown);
        var pack = new PackStore(database, () => shown);
        chats.Replace(3, [new MemberDto(7, "anna", "Anna", Role: "owner"), new MemberDto(9, "bob", "Bob", Role: "member")]);
        chats.Replace(5, [new MemberDto(7, "anna", "Anna", Role: "member"), new MemberDto(11, "carl", "Carl", Role: "owner")]);
        var router = new FrameRouter(chats, board, pack);

        router.Hear(ServerFrame.Parse("""{"type": "member_joined", "family_id": 5, "user": {"id": 12, "username": "dana", "display_name": "Dana"}}""")!);
        router.Hear(ServerFrame.Parse("""{"type": "member_left", "family_id": 5, "user_id": 11}""")!);
        router.Hear(ServerFrame.Parse("""{"type": "family_owner", "family_id": 5, "user_id": 7}""")!);
        router.Hear(ServerFrame.Parse("""{"type": "board_note", "note": {"id": 40, "author_id": 11, "text": "Milk", "board_seq": 6, "content_seq": 6, "family_id": 5}}""")!);
        router.Hear(ServerFrame.Parse("""{"type": "pack_item", "item": {"id": 50, "added_by": 11, "pack_seq": 3, "family_id": 5, "attachment": {"id": 70, "kind": "photo"}}}""")!);

        Assert.Equal([7L, 9L], chats.Members().Select(member => member.Id));
        Assert.True(chats.Members().Single(member => member.Id == 7).Owner);
        var other = chats.MembersOf(5);
        Assert.Contains(other, member => member.Id == 12 && !member.IsFormer);
        Assert.True(other.Single(member => member.Id == 11).HasLeft);
        Assert.True(other.Single(member => member.Id == 7).Owner);
        Assert.Empty(board.Notes());
        Assert.Equal(40, Assert.Single(board.In(5).Notes()).Id);
        Assert.Empty(pack.Items());
        Assert.Equal(50, Assert.Single(pack.In(5).Items()).Id);
        // Shown, the other family is the one drawn.
        shown = 5;
        Assert.Equal(40, Assert.Single(board.Notes()).Id);
        Assert.Contains(chats.Members(), member => member.Id == 12);
    }

    /// <summary>
    /// A TOMBSTONE names its family too (2026-10-22): a note or item taken down in family 5 leaves family 5's wall and pack, and
    /// moves THAT family's cursor — never the shown family's.
    /// </summary>
    [Fact]
    public void TombstonesGoToTheFamilyTheyName()
    {
        long? shown = 3;
        var board = new BoardStore(database, () => shown);
        var pack = new PackStore(database, () => shown);
        board.Replace([new NoteDto(1, 9, "text", "Milk", BoardSeq: 4)], 4);
        board.In(5).Replace([new NoteDto(2, 11, "text", "Bread", BoardSeq: 9)], 9);
        pack.In(5).Replace([new PackItemDto(20, 11, new AttachmentDto(70, "photo"), PackSeq: 2)], 2);
        pack.Reconnected();
        pack.In(5).CaughtUp(pack.Connection);
        var router = new FrameRouter(new ChatStore(database, () => 7, () => shown), board, pack);

        router.Hear(ServerFrame.Parse("""{"type": "board_note", "note": {"id": 2, "deleted": true, "board_seq": 10, "family_id": 5}}""")!);
        router.Hear(ServerFrame.Parse("""{"type": "pack_item", "item": {"id": 20, "deleted": true, "pack_seq": 3, "family_id": 5}}""")!);

        Assert.Empty(board.In(5).Notes());
        Assert.Equal(10, board.In(5).Cursor);
        Assert.Equal(4, board.Cursor);
        Assert.Single(board.Notes());
        Assert.Empty(pack.In(5).Items());
        Assert.Equal(3, pack.In(5).Cursor);
        Assert.Equal(0, pack.Cursor);
    }

    /// <summary>
    /// One naming the READER is a different fact — in a family, or out of one — and is the session's to act on; a frame about
    /// somebody else raises neither.
    /// </summary>
    [Fact]
    public void FramesNamingTheReaderSayTheyAreInOrOutOfAFamily()
    {
        long? shown = 3;
        var router = new FrameRouter(new ChatStore(database, () => 7, () => shown), new BoardStore(database, () => shown));
        var joined = new List<long>();
        var left = new List<long>();
        var invited = new List<long>();
        var lapsed = new List<long>();
        router.JoinedFamily += joined.Add;
        router.LeftFamily += left.Add;
        router.InvitationChanged += invitation => invited.Add(invitation.Id);
        router.InvitationRemoved += lapsed.Add;

        router.Hear(ServerFrame.Parse("""{"type": "member_joined", "family_id": 5, "user": {"id": 9, "username": "bob", "display_name": "Bob"}}""")!);
        router.Hear(ServerFrame.Parse("""{"type": "member_left", "family_id": 5, "user_id": 9}""")!);
        router.Hear(ServerFrame.Parse("""{"type": "member_joined", "family_id": 8, "user": {"id": 7, "username": "anna", "display_name": "Anna"}}""")!);
        router.Hear(ServerFrame.Parse("""{"type": "member_left", "family_id": 5, "user_id": 7}""")!);
        router.Hear(ServerFrame.Parse("""{"type": "invitation", "invitation": {"id": 4, "family_id": 9, "family_name": "Sidorovs", "invited_by": {"id": 11, "username": "carl", "display_name": "Carl"}, "user": {"id": 7, "username": "anna", "display_name": "Anna"}}}""")!);
        router.Hear(ServerFrame.Parse("""{"type": "invitation_removed", "invitation_id": 4}""")!);

        Assert.Equal([8L], joined);
        Assert.Equal([5L], left);
        Assert.Equal([4L], invited);
        Assert.Equal([4L], lapsed);
    }

    // ---- the resync -----------------------------------------------------------------------------------------------------------

    /// <summary>
    /// Step 1 is `GET /me` and then EACH family's own document, board and pack, by id, each against its own cursor — and then
    /// the chats, which arrive for every family at once (docs/protocol.md, "Best-effort delivery", step 1).
    /// </summary>
    [Fact]
    public async Task AResyncReadsEveryFamilyIntoItsOwnRows()
    {
        var server = new Server()
            .On("/me", """
                {"user": {"id": 7, "username": "anna", "display_name": "Anna"}, "blocked_user_ids": [],
                 "family": {"id": 3, "name": "The Smiths"}, "role": "member",
                 "families": [{"id": 3, "name": "The Smiths", "role": "member"}, {"id": 5, "name": "Ivanovs", "role": "owner"}],
                 "pending_join_requests": [], "invitations": [], "member_families_enabled": true, "max_families_per_account": 5}
                """)
            .On("/families/3", """{"family": {"id": 3, "name": "The Smiths"}, "members": [{"id": 7, "username": "anna", "display_name": "Anna", "role": "member"}, {"id": 9, "username": "bob", "display_name": "Bob", "role": "owner"}], "max_board_seq": 4, "max_pack_items": 200, "max_pack_item_bytes": 524288, "max_pack_seq": 2, "assistant": {"user_id": 99, "display_name": "Assistant", "mention": "ai"}}""")
            .On("/families/5", """{"family": {"id": 5, "name": "Ivanovs"}, "members": [{"id": 7, "username": "anna", "display_name": "Anna", "role": "owner"}, {"id": 11, "username": "carl", "display_name": "Carl", "role": "member", "password_reset": false}], "max_board_seq": 9, "max_pack_items": 200, "max_pack_item_bytes": 524288}""")
            .On("/families/3/board", """{"notes": [{"id": 1, "author_id": 9, "text": "Milk", "board_seq": 4, "family_id": 3}], "max_board_seq": 4}""")
            .On("/families/5/board", """{"notes": [{"id": 2, "author_id": 11, "text": "Bread", "board_seq": 9, "family_id": 5}], "max_board_seq": 9}""")
            .On("/families/3/pack", """{"items": [{"id": 20, "added_by": 9, "pack_seq": 2, "family_id": 3, "attachment": {"id": 70, "kind": "photo"}}], "max_pack_seq": 2}""")
            .On("/chats", """{"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths", "family_ids": [3]}}, {"chat": {"id": 43, "kind": "family", "title": "Ivanovs", "family_ids": [5]}}, {"chat": {"id": 44, "kind": "direct", "title": "Bob", "peer_user_id": 9, "family_ids": [3, 5]}}]}""");
        long? shown = 5;
        var chats = new ChatStore(database, () => 7, () => shown);
        var board = new BoardStore(database, () => shown);
        var pack = new PackStore(database, () => shown);
        var api = Client(server);
        api.FamilyScope = shown;

        var report = await new Resync(api, chats, board, pack: pack).RunAsync();

        Assert.True(report.Complete, report.Stopped?.Code);
        Assert.Equal([3L, 5L], report.FamiliesRead!);
        Assert.Equal(5, report.ShownFamily);
        // The assistant is the SHOWN family's document's; family 5's names none.
        Assert.Null(report.Assistant);
        Assert.Equal(5, report.Me!.AllFamilies[1].Id);
        Assert.Contains("/api/v1/families/3/board", server.Asked);
        Assert.Contains("/api/v1/families/5/board", server.Asked);
        // Family 5's pack is empty and untouched: nothing is asked of it.
        Assert.DoesNotContain("/api/v1/families/5/pack", server.Asked);
        Assert.Equal(2, Assert.Single(board.Notes()).Id);
        Assert.Equal(1, Assert.Single(board.In(3).Notes()).Id);
        Assert.Equal(9, board.Cursor);
        Assert.Equal(4, board.In(3).Cursor);
        Assert.Equal(20, Assert.Single(pack.In(3).Items()).Id);
        Assert.Empty(pack.Items());
        Assert.Equal([7L, 11L], chats.Members().Select(member => member.Id));
        Assert.False(chats.Members().Single(member => member.Id == 11).PasswordReset);
        Assert.Equal([7L, 9L], chats.MembersOf(3).Select(member => member.Id));
        Assert.Equal([3L, 5L], chats.Chat(44)!.Chat.FamilyIds!);
    }

    /// <summary>A family gone between `GET /me` and its own read is skipped, not a reason to stop: `/me` reconciles next time.</summary>
    [Fact]
    public async Task AFamilyGoneMidPassIsSkipped()
    {
        var server = new Server()
            .On("/me", """{"user": {"id": 7, "username": "anna", "display_name": "Anna"}, "families": [{"id": 3, "name": "The Smiths", "role": "member"}, {"id": 5, "name": "Ivanovs", "role": "member"}]}""")
            .On("/families/3", """{"family": {"id": 3, "name": "The Smiths"}, "members": []}""")
            .On("/families/5", """{"error": {"code": "family_not_found", "message": "no such family"}}""", HttpStatusCode.NotFound)
            .On("/chats", """{"chats": []}""");
        long? shown = 3;
        var report = await new Resync(Client(server), new ChatStore(database, () => 7, () => shown), new BoardStore(database, () => shown))
            .RunAsync();
        Assert.True(report.Complete);
        Assert.Equal([3L], report.FamiliesRead!);
    }
}
