using System.Net;
using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// Groups (docs/protocol.md, "Groups"; docs/groups-2026-10-21.md): who may do what, what a name may be, whom the lists
/// offer, what a refusal says, and the model that asks the server and keeps what it answered.
/// </summary>
public class GroupsTests : IDisposable
{
    private const long Me = 7;
    private const long Assistant = 1;

    private static readonly IStringCatalog Say = EnglishCatalog.Instance;

    private static readonly MemberDto[] Roster =
    [
        new(Me, "anna", "Anna", Role: "owner"),
        new(9, "bob", "Bob"),
        new(11, "cleo", "Cleo"),
        new(12, "dan", "Dan"),
        new(13, "eve", "Eve", HasLeft: true),
        new(14, "gone", "Deleted account", Deleted: true),
        new(Assistant, "assistant", "Assistant"),
    ];

    /// <summary>Bob made it; Anna (the reader) and Cleo are in it too.</summary>
    private static readonly ChatDto Cousins = new(77, "group", "Cousins", MemberIds: [9, Me, 11], AdminUserId: 9);

    private static readonly ChatDto Mine = Cousins with { AdminUserId = Me };

    private readonly Database cache = Database.OpenInMemory();
    private readonly ChatStore chats;

    public GroupsTests()
    {
        chats = new ChatStore(cache, () => Me);
        chats.Replace([.. Roster]);
        chats.Replace([new ChatRowDto(new ChatDto(42, "family", "The Smiths")), new ChatRowDto(Cousins)]);
    }

    public void Dispose() => cache.Dispose();

    private (GroupModel Groups, Server Handler) Build(Server server) =>
        (new GroupModel(
            new ApiClient(new HttpClient(server), ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken")),
            chats),
         server);

    private static string Answer(ChatDto chat) => Wire.Encode(new ChatResponse(chat));

    // ---- who may do what ------------------------------------------------------------------------

    /// <summary>
    /// THE TABLE (docs/protocol.md, "Who may do what"): the admin renames, adds, removes and deletes; anybody in it leaves;
    /// nobody may do anything to a chat that is not a group, nor while the reader is not known.
    /// </summary>
    [Theory]
    [InlineData(GroupAction.Rename, true, false)]
    [InlineData(GroupAction.AddMembers, true, false)]
    [InlineData(GroupAction.RemoveMember, true, false)]
    [InlineData(GroupAction.Delete, true, false)]
    [InlineData(GroupAction.Leave, true, true)]
    public void TheAdminManagesItAndAnyMemberMayLeave(GroupAction action, bool admin, bool member)
    {
        Assert.Equal(admin, GroupRules.May(action, Mine, Me));
        Assert.Equal(member, GroupRules.May(action, Cousins, Me));
        Assert.False(GroupRules.May(action, new ChatDto(42, "family", "The Smiths"), Me));
        Assert.False(GroupRules.May(action, new ChatDto(43, "direct", "Bob", PeerUserId: 9), Me));
        Assert.False(GroupRules.May(action, new ChatDto(44, "ai", "Assistant"), Me));
        Assert.False(GroupRules.May(action, Mine, 0));
    }

    /// <summary>The admin may take out somebody else who is in it — never themselves (that is leaving), never a stranger.</summary>
    [Fact]
    public void TheAdminRemovesSomebodyElseWhoIsInIt()
    {
        Assert.True(GroupRules.MayRemove(Mine, Me, 9));
        Assert.True(GroupRules.MayRemove(Mine, Me, 11));
        Assert.False(GroupRules.MayRemove(Mine, Me, Me));
        Assert.False(GroupRules.MayRemove(Mine, Me, 12));
        Assert.False(GroupRules.MayRemove(Cousins, Me, 11));
    }

    // ---- the name ----------------------------------------------------------------------------------

    /// <summary>
    /// TRIMMED, THEN 1 TO 64 CHARACTERS, counted as the server counts — scalar values, so 64 emoji are a name and 65 are not.
    /// Two groups may share a name; nothing here asks.
    /// </summary>
    [Fact]
    public void ANameIsTrimmedAndThenOneToSixtyFourCharacters()
    {
        Assert.Equal("Cousins", GroupRules.Name("  Cousins \n"));
        Assert.Null(GroupRules.Name(""));
        Assert.Null(GroupRules.Name("   "));
        Assert.Null(GroupRules.Name(null));
        Assert.Equal(new string('a', 64), GroupRules.Name(new string('a', 64)));
        Assert.Null(GroupRules.Name(new string('a', 65)));
        var emoji = string.Concat(Enumerable.Repeat("😀", 64));
        Assert.Equal(emoji, GroupRules.Name(emoji));
        Assert.Null(GroupRules.Name(emoji + "😀"));
    }

    /// <summary>What stops "Create Group", said before anything is sent: the name first, then nobody chosen.</summary>
    [Fact]
    public void CreatingNeedsANameAndAtLeastOneOtherMember()
    {
        Assert.Null(GroupRules.CreateProblem("Cousins", [9], Me, Say));
        Assert.Equal("A group's name can be 1 to 64 characters.", GroupRules.CreateProblem(" ", [9], Me, Say));
        Assert.Equal("A group's name can be 1 to 64 characters.", GroupRules.CreateProblem(" ", [], Me, Say));
        Assert.Equal("Choose at least one member.", GroupRules.CreateProblem("Cousins", [], Me, Say));
        Assert.Equal("Choose at least one member.", GroupRules.CreateProblem("Cousins", [Me], Me, Say));
    }

    // ---- whom the lists offer ----------------------------------------------------------------------

    /// <summary>
    /// A new group is made from the live family: never the reader (added anyway), never the assistant, never somebody who
    /// left, never a deleted account. "Add Members" offers the same people, less those already in.
    /// </summary>
    [Fact]
    public void TheListsOfferTheLiveFamilyAndNobodyElse()
    {
        Assert.Equal([9L, 11L, 12L], GroupRules.Choosable(Roster, Me, Assistant).Select(member => member.Id));
        Assert.Equal([12L], GroupRules.Addable(Mine, Roster, Me, Assistant).Select(member => member.Id));
        Assert.Empty(GroupRules.Addable(Mine with { MemberIds = [9, Me, 11, 12] }, Roster, Me, Assistant));
        Assert.Equal("Everyone in the family is already in this group.", GroupText.EveryoneIsIn(Say));
    }

    /// <summary>The info's member list: join order, the admin marked, the reader marked, and who the admin may remove.</summary>
    [Fact]
    public void TheMembersAreListedInTheOrderTheyJoined()
    {
        var members = GroupRules.Members(Mine with { MemberIds = [9, Me, 11, 99] }, Roster, Me, Say);

        Assert.Equal([9L, Me, 11L, 99L], members.Select(member => member.UserId));
        Assert.Equal(["Bob", "Anna", "Cleo", "Someone"], members.Select(member => member.Name));
        Assert.Equal([false, true, false, false], members.Select(member => member.Admin));
        Assert.Equal([false, true, false, false], members.Select(member => member.Me));
        Assert.Equal([true, false, true, true], members.Select(member => member.Removable));
        // Somebody else's group: Bob is its admin, and the reader may remove nobody.
        var theirs = GroupRules.Members(Cousins, Roster, Me, Say);
        Assert.True(theirs[0].Admin);
        Assert.All(theirs, member => Assert.False(member.Removable));
        Assert.Equal("Members: 3", GroupText.Count(Cousins, Say));
    }

    // ---- what is said ------------------------------------------------------------------------------

    [Fact]
    public void ARefusalIsSaidInTheReadersWords()
    {
        Assert.Equal("Only the group's admin can do this.",
            GroupText.Failure(new ApiError(ErrorCodes.NotGroupAdmin, "x", 403), creating: false, Say));
        Assert.Equal("Your family already has 50 groups.",
            GroupText.Failure(new ApiError(ErrorCodes.TooManyGroups, "x", 409), creating: true, Say));
        Assert.Equal("Couldn't create the group.",
            GroupText.Failure(new ApiError(ErrorCodes.NotSameFamily, "x", 409), creating: true, Say));
        Assert.Equal("Couldn't change the group.",
            GroupText.Failure(ApiError.Transport("offline"), creating: false, Say));
        Assert.Equal("Delete “Cousins”? Its messages are deleted for everyone.", GroupText.DeleteQuestion("Cousins", Say));
        Assert.Equal("Leave “Cousins”? You can come back only if its admin adds you again.", GroupText.LeaveQuestion("Cousins", Say));
    }

    // ---- what a group offers -----------------------------------------------------------------------

    /// <summary>
    /// A GROUP IS THE FAMILY CHAT WITH FEWER PEOPLE: sender names, mentions, polls, <c>@ai</c>, no "seen" — and the video
    /// button, which a direct chat has too and the assistant's chat does not.
    /// </summary>
    [Fact]
    public void AGroupSharesTheFamilyChatsRoom()
    {
        Assert.True(ChatKinds.IsShared("family"));
        Assert.True(ChatKinds.IsShared("group"));
        Assert.False(ChatKinds.IsShared("direct"));
        Assert.False(ChatKinds.IsShared("ai"));
        Assert.False(ChatKinds.IsShared(null));
        Assert.True(ChatKinds.TakesVideoMessages("group"));
        Assert.True(ChatKinds.TakesVideoMessages("family"));
        Assert.True(ChatKinds.TakesVideoMessages("direct"));
        Assert.False(ChatKinds.TakesVideoMessages("ai"));
        Assert.Contains(AttachItem.Poll, AttachMenu.Groups(false, false, ChatKinds.IsShared("group"), false, true).SelectMany(group => group));
        Assert.Contains(AttachItem.AskAssistant, AttachMenu.Groups(false, false, ChatKinds.IsShared("group"), false, true).SelectMany(group => group));
        var mine = new MessageDto(1500, 77, Me, null, "hi", "2026-09-12T10:00:00Z");
        Assert.False(BubbleRules.ShowsTick(mine, Me, ChatKinds.IsShared("group")));
    }

    // ---- the model ---------------------------------------------------------------------------------

    /// <summary>A group made here is in the cache at once, with the reader as its admin, before any frame lands.</summary>
    [Fact]
    public async Task CreatingKeepsTheGroupTheServerAnswered()
    {
        var made = new ChatDto(78, "group", "Book club", MemberIds: [Me, 12], AdminUserId: Me);
        var (groups, server) = Build(new Server().On("/chats/groups", Answer(made), HttpStatusCode.Created));

        var (chatId, error) = await groups.CreateAsync("  Book club ", [12, 12]);

        Assert.Null(error);
        Assert.Equal(78, chatId);
        Assert.Equal("Book club", chats.Chat(78)!.Chat.Title);
        Assert.Equal(Me, chats.Chat(78)!.Chat.AdminUserId);
        // Trimmed, and every id once.
        Assert.Equal("{\"title\":\"Book club\",\"member_ids\":[12]}", Assert.Single(server.Bodies));
    }

    /// <summary>A name that cannot be one is refused here, and nothing is sent.</summary>
    [Fact]
    public async Task ABadNameIsNeverSent()
    {
        var (groups, server) = Build(new Server());

        var (chatId, error) = await groups.CreateAsync("   ", [12]);

        Assert.Null(chatId);
        Assert.Equal(ErrorCodes.Validation, error!.Code);
        Assert.Equal(ErrorCodes.Validation, (await groups.RenameAsync(77, new string('a', 65)))!.Code);
        Assert.Empty(server.Asked);
    }

    /// <summary>A rename and an add are written as the server answered them; a refusal changes nothing and is handed back.</summary>
    [Fact]
    public async Task ChangesAreKeptAndRefusalsChangeNothing()
    {
        var renamed = Mine with { Title = "The cousins" };
        var (groups, _) = Build(new Server()
            .Then("/chats/77", (HttpStatusCode.OK, Answer(renamed)),
                  (HttpStatusCode.Forbidden, """{"error": {"code": "not_group_admin", "message": "no"}}"""))
            .On("/chats/77/members", Answer(renamed with { MemberIds = [9, Me, 11, 12] }))
            .On("/chats/77/members/11", Answer(renamed with { MemberIds = [9, Me] })));

        Assert.Null(await groups.RenameAsync(77, "The cousins"));
        Assert.Equal("The cousins", chats.Chat(77)!.Chat.Title);
        Assert.Null(await groups.AddAsync(77, [12]));
        Assert.Equal([9L, Me, 11L, 12L], chats.Chat(77)!.Chat.MemberIds!);
        Assert.Null(await groups.RemoveAsync(77, 11));
        Assert.Equal([9L, Me], chats.Chat(77)!.Chat.MemberIds!);

        Assert.Equal(ErrorCodes.NotGroupAdmin, (await groups.RenameAsync(77, "Ours"))!.Code);
        Assert.Equal("The cousins", chats.Chat(77)!.Chat.Title);
    }

    /// <summary>
    /// LEAVING asks with the reader's own id and is a <c>204</c>; the group goes from the cache at once. A group already gone
    /// — deleted by its admin first — is the same outcome, and nothing is said.
    /// </summary>
    [Fact]
    public async Task LeavingDropsTheGroupEvenIfItWasAlreadyGone()
    {
        var (groups, server) = Build(new Server()
            .Then("/chats/77/members/7", (HttpStatusCode.NoContent, null)));

        Assert.Null(await groups.LeaveAsync(77));
        Assert.Null(chats.Chat(77));
        Assert.Equal("/api/v1/chats/77/members/7", Assert.Single(server.Asked));

        chats.Upsert(Cousins);
        var (again, _) = Build(new Server()
            .On("/chats/77/members/7", """{"error": {"code": "chat_not_found", "message": "gone"}}""", HttpStatusCode.NotFound));
        Assert.Null(await again.LeaveAsync(77));
        Assert.Null(chats.Chat(77));
    }

    /// <summary>Deleting drops it at once; a refusal keeps it, and says why; a transport failure keeps it too.</summary>
    [Fact]
    public async Task DeletingDropsItAndARefusalKeepsIt()
    {
        var (refused, _) = Build(new Server()
            .On("/chats/77", """{"error": {"code": "not_group_admin", "message": "no"}}""", HttpStatusCode.Forbidden));
        Assert.Equal(ErrorCodes.NotGroupAdmin, (await refused.DeleteAsync(77))!.Code);
        Assert.NotNull(chats.Chat(77));

        var (groups, _) = Build(new Server().On("/chats/77", null, HttpStatusCode.NoContent));
        Assert.Null(await groups.DeleteAsync(77));
        Assert.Null(chats.Chat(77));
        // The family chat is never taken this way, whatever answered.
        Assert.NotNull(chats.Chat(42));
    }
}
