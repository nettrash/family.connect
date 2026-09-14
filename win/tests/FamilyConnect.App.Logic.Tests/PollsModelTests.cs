using System.Net;
using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// Polls: what a tap sends, who may close, the open-polls list, the composer's draft and the words a
/// poll is drawn with (docs/protocol.md, "Polls" and "Finding the open ones").
/// </summary>
public class PollsModelTests : IDisposable
{
    private const long Me = 7;
    private const long Chat = 42;
    private const string Sent = "2026-09-14T10:00:00Z";
    private const string NotMember = """{"error": {"code": "not_chat_member", "message": "no"}}""";

    private static readonly IStringCatalog Say = EnglishCatalog.Instance;

    private readonly Database cache = Database.OpenInMemory();
    private readonly ChatStore chats;
    private readonly OutboxStore outbox;

    public PollsModelTests()
    {
        chats = new ChatStore(cache, () => Me);
        outbox = new OutboxStore(cache);
        chats.Replace([new ChatRowDto(new ChatDto(Chat, "family", "The Smiths"))]);
        chats.Replace(
            [new MemberDto(7, "anna", "Anna"), new MemberDto(11, "bob", "Bob"), new MemberDto(13, "dora", "Dora"), new MemberDto(16, "nameless", ""), new MemberDto(17, "ghost", "Gus", Deleted: true)],
            [new MemberDto(14, "gone", "Eve", HasLeft: true, Deleted: true), new MemberDto(15, "left", "Fay", HasLeft: true)]);
        chats.SetBlocked(13, true);
    }

    public void Dispose() => cache.Dispose();

    private sealed class Wire : IFrameSender
    {
        public bool IsConnected { get; set; }

        public Task<bool> TrySend(string frame, CancellationToken ct = default) => Task.FromResult(IsConnected);

        public event Action<ServerFrame>? Frame;

        public void Unused() => Frame?.Invoke(new ServerFrame.Pong());
    }

    private static ApiClient Api(Server server) =>
        new(new HttpClient(server), ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken"));

    private ConversationModel Open(Server server)
    {
        var api = Api(server);
        var socket = new Wire();
        var sending = new SendPipeline(socket, outbox, chats, api, wait: (_, _) => Task.CompletedTask);
        return new ConversationModel(Chat, chats, api, sending, socket, outbox: outbox);
    }

    private static PollDto Poll(long seq, bool closed, long[] pizza, long[] pasta) =>
        new(seq, closed, [new PollOptionDto(1, "Pizza", pizza), new PollOptionDto(2, "Pasta", pasta)]);

    private static string Votes(long[] voters) => string.Join(", ", voters);

    private static string State(long messageId, long seq, bool closed, long[] pizza, long[] pasta) =>
        $$$"""{"message_id": {{{messageId}}}, "poll": {"poll_seq": {{{seq}}}, "closed": {{{(closed ? "true" : "false")}}}, "options": [{"id": 1, "text": "Pizza", "votes": [{{{Votes(pizza)}}}]}, {"id": 2, "text": "Pasta", "votes": [{{{Votes(pasta)}}}]}]}}""";

    private static string Row(long id, long sender, long seq, long[] pizza) =>
        $$$"""{"id": {{{id}}}, "chat_id": 42, "sender_id": {{{sender}}}, "body": "Q{{{id}}}", "created_at": "{{{Sent}}}", "poll": {"poll_seq": {{{seq}}}, "closed": false, "options": [{"id": 1, "text": "Pizza", "votes": [{{{Votes(pizza)}}}]}, {"id": 2, "text": "Pasta", "votes": []}]}}""";

    private static string List(params string[] rows) => $$"""{"messages": [{{string.Join(", ", rows)}}]}""";

    private void Hold(long id, PollDto poll, long sender = 9) =>
        chats.Apply(new MessageDto(id, Chat, sender, null, "Dinner?", Sent, Poll: poll));

    // ---- a tap --------------------------------------------------------------------------------------

    /// <summary>
    /// THE TAP IS DECIDED HERE: an option not held is a PUT naming it, the one held is a DELETE — and the
    /// answer, the whole poll, is what the cache takes, as evidence that moves no cursor.
    /// </summary>
    [Fact]
    public async Task ATapCastsWhatIsNotHeldAndRetractsWhatIs()
    {
        Hold(1, Poll(3, false, [], [9]));
        var server = new Server().Then(
            "/chats/42/messages/1/vote",
            (HttpStatusCode.OK, State(1, 4, false, [Me], [9])),
            (HttpStatusCode.OK, State(1, 5, false, [], [9])));
        var chat = Open(server);

        Assert.Null(await chat.VoteAsync(1, 1));
        Assert.Equal("{\"option_id\":1}", Assert.Single(server.Bodies));
        Assert.Equal(1, Polls.MyOption(chats.Message(1)!.Poll!, Me));
        Assert.Equal(4, chats.Message(1)!.Poll!.PollSeq);

        Assert.Null(await chat.VoteAsync(1, 1));
        // A DELETE carries no body, so there is still exactly one.
        Assert.Single(server.Bodies);
        Assert.Equal(2, server.Asked.Count);
        Assert.Null(Polls.MyOption(chats.Message(1)!.Poll!, Me));
        Assert.Equal(5, chats.Message(1)!.Poll!.PollSeq);
        Assert.True((chats.Chat(Chat)!.MaxPollSeq ?? 0) < 4);
    }

    [Fact]
    public async Task ChangingYourMindPutsTheOtherOption()
    {
        Hold(1, Poll(3, false, [Me], []));
        var server = new Server().On("/chats/42/messages/1/vote", State(1, 4, false, [], [Me]));

        Assert.Null(await Open(server).VoteAsync(1, 2));

        Assert.Equal("{\"option_id\":2}", Assert.Single(server.Bodies));
        Assert.Equal(2, Polls.MyOption(chats.Message(1)!.Poll!, Me));
    }

    /// <summary>A closed poll is a result, an option the poll never had is nothing to send, and a message not held is not a poll.</summary>
    [Fact]
    public async Task NothingIsSentForAClosedPollAStrangeOptionOrAMessageThatIsNotAPoll()
    {
        Hold(1, Poll(3, true, [], []));
        Hold(2, Poll(3, false, [], []));
        chats.Apply(new MessageDto(3, Chat, 9, null, "just words", Sent));
        var server = new Server();
        var chat = Open(server);

        Assert.Null(await chat.VoteAsync(1, 1));
        Assert.Null(await chat.VoteAsync(2, 9));
        Assert.Null(await chat.VoteAsync(3, 1));
        Assert.Null(await chat.VoteAsync(77, 1));
        Assert.Null(await chat.ClosePollAsync(3));
        Assert.Null(await chat.ClosePollAsync(77));

        Assert.Empty(server.Asked);
    }

    [Fact]
    public async Task ARefusedVoteChangesNothing()
    {
        Hold(1, Poll(3, false, [], []));
        var server = new Server().On(
            "/chats/42/messages/1/vote", """{"error": {"code": "poll_closed", "message": "closed"}}""", HttpStatusCode.Conflict);

        var error = await Open(server).VoteAsync(1, 1);

        Assert.Equal(ErrorCodes.PollClosed, error?.Code);
        Assert.Equal(3, chats.Message(1)!.Poll!.PollSeq);
        Assert.Null(Polls.MyOption(chats.Message(1)!.Poll!, Me));
    }

    /// <summary>Closing is the AUTHOR's — the family owner does not outrank authorship — and only while the poll is open.</summary>
    [Fact]
    public async Task OnlyTheAuthorClosesAnOpenPoll()
    {
        Hold(1, Poll(3, false, [], []), sender: Me);
        Hold(2, Poll(3, false, [], []));
        var server = new Server().On("/chats/42/messages/1/poll/close", State(1, 6, true, [], []));
        var chat = Open(server);

        Assert.Null(await chat.ClosePollAsync(2));
        Assert.Empty(server.Asked);

        Assert.Null(await chat.ClosePollAsync(1));
        Assert.Single(server.Asked);
        Assert.True(chats.Message(1)!.Poll!.Closed);
        Assert.Equal(6, chats.Message(1)!.Poll!.PollSeq);

        // Closed now: there is nothing more to close.
        Assert.Null(await chat.ClosePollAsync(1));
        Assert.Single(server.Asked);

        var mine = new MessageDto(9, Chat, Me, null, "Q", Sent, Poll: Poll(1, false, [], []));
        Assert.True(PollVoting.MayClose(mine, Me));
        Assert.False(PollVoting.MayClose(mine, 11));
        Assert.False(PollVoting.MayClose(mine with { Id = 0 }, Me));
        Assert.False(PollVoting.MayClose(mine with { Poll = mine.Poll! with { Closed = true } }, Me));
    }

    // ---- the open polls -------------------------------------------------------------------------------

    /// <summary>
    /// Oldest first by id, whatever order the answer came in; each row drawn with the newest state this
    /// device knows; a blocked member's poll hidden until revealed.
    /// </summary>
    [Fact]
    public async Task TheListIsOldestFirstNewestStateAndHidesTheBlocked()
    {
        var server = new Server().On("/chats/42/polls/open", List(Row(30, 9, 2, []), Row(10, 13, 1, []), Row(20, Me, 3, [])));
        var model = new OpenPollsModel(Chat, chats, Api(server));
        Assert.False(model.Loaded);

        Assert.Null(await model.LoadAsync());

        Assert.True(model.Loaded);
        Assert.Equal([10L, 20L, 30L], model.Messages().Select(message => message.Id));
        // A frame reached the cache after the list was read: the newer state is the one drawn...
        Hold(30, Poll(4, false, [Me], []));
        Assert.Equal(1, Polls.MyOption(model.Messages()[2].Poll!, Me));
        // ...and an older one is not.
        Hold(20, Poll(2, false, [11], []));
        Assert.Null(Polls.MyOption(model.Messages()[1].Poll!, Me));
        Assert.Equal(3, model.Messages()[1].Poll!.PollSeq);

        Assert.True(model.IsHidden(model.Messages()[0]));
        Assert.False(model.IsHidden(model.Messages()[1]));
        model.Reveal(10);
        Assert.False(model.IsHidden(model.Messages()[0]));
    }

    /// <summary>The badge counts the cache's polls and the list's, once each: here only the reader's own, still unanswered.</summary>
    [Fact]
    public async Task TheBadgeCountsWhatIsStillUnanswered()
    {
        var server = new Server().On("/chats/42/polls/open", List(Row(30, 9, 2, []), Row(10, 13, 1, []), Row(20, Me, 3, [])));
        var model = new OpenPollsModel(Chat, chats, Api(server));
        Hold(30, Poll(4, false, [Me], []));
        Hold(40, Poll(1, false, [], []));
        Assert.Equal(1, model.Unanswered());

        await model.LoadAsync();

        // 20 (the reader's own) and 40; 30 was answered in the newer copy, 10 is a blocked member's.
        Assert.Equal(2, model.Unanswered());
    }

    /// <summary>
    /// A vote from the list is re-read after — and a re-read that fails keeps the vote it answered, says
    /// why, and leaves the message out of the cache it never belonged to.
    /// </summary>
    [Fact]
    public async Task AVoteOnTheListIsReReadAndSurvivesAReReadThatFails()
    {
        var server = new Server()
            .Then("/chats/42/polls/open", (HttpStatusCode.OK, List(Row(10, 9, 1, []))), (HttpStatusCode.Forbidden, NotMember))
            .On("/chats/42/messages/10/vote", State(10, 2, false, [Me], []));
        var model = new OpenPollsModel(Chat, chats, Api(server));
        await model.LoadAsync();

        var error = await model.VoteAsync(10, 1);

        Assert.Equal(ErrorCodes.NotChatMember, error?.Code);
        Assert.Equal(ErrorCodes.NotChatMember, model.Failure?.Code);
        Assert.Equal(2, server.Asked.Count(path => path.StartsWith("/api/v1/chats/42/polls/open", StringComparison.Ordinal)));
        var row = Assert.Single(model.Messages());
        Assert.Equal(2, row.Poll!.PollSeq);
        Assert.Equal(1, Polls.MyOption(row.Poll, Me));
        Assert.Null(chats.Message(10));
    }

    /// <summary>An answer OLDER than the row the list holds does not wind it back, even when the re-read after it fails.</summary>
    [Fact]
    public async Task AnOlderAnswerDoesNotWindTheListBack()
    {
        var server = new Server()
            .Then("/chats/42/polls/open", (HttpStatusCode.OK, List(Row(10, 9, 5, [11]))), (HttpStatusCode.Forbidden, NotMember))
            .On("/chats/42/messages/10/vote", State(10, 3, false, [Me], []));
        var model = new OpenPollsModel(Chat, chats, Api(server));
        await model.LoadAsync();

        await model.VoteAsync(10, 1);

        var row = Assert.Single(model.Messages());
        Assert.Equal(5, row.Poll!.PollSeq);
        Assert.Equal([11L], row.Poll.Options[0].Votes);
    }

    [Fact]
    public async Task ClosingFromTheListTakesThePollOffIt()
    {
        var server = new Server()
            .Then("/chats/42/polls/open", (HttpStatusCode.OK, List(Row(20, Me, 1, []), Row(30, 9, 1, []))), (HttpStatusCode.OK, List(Row(30, 9, 1, []))))
            .On("/chats/42/messages/20/poll/close", State(20, 3, true, [], []));
        var model = new OpenPollsModel(Chat, chats, Api(server));
        await model.LoadAsync();

        Assert.Null(await model.CloseAsync(20));
        Assert.Null(model.Failure);
        Assert.Equal([30L], model.Messages().Select(message => message.Id));

        // Somebody else's, and one no longer listed: nothing is sent.
        var asked = server.Asked.Count;
        Assert.Null(await model.CloseAsync(30));
        Assert.Null(await model.VoteAsync(20, 1));
        Assert.Equal(asked, server.Asked.Count);
    }

    [Fact]
    public async Task AListThatCannotBeReadSaysSoAndKeepsWhatItHad()
    {
        var server = new Server().Then(
            "/chats/42/polls/open", (HttpStatusCode.OK, List(Row(10, 9, 1, []))), (HttpStatusCode.Forbidden, NotMember), (HttpStatusCode.OK, List()));
        var model = new OpenPollsModel(Chat, chats, Api(server));
        await model.LoadAsync();

        Assert.Equal(ErrorCodes.NotChatMember, (await model.LoadAsync())?.Code);
        Assert.True(model.Loaded);
        Assert.Single(model.Messages());

        Assert.Null(await model.LoadAsync());
        Assert.Null(model.Failure);
        Assert.Empty(model.Messages());
    }

    // ---- the composer ---------------------------------------------------------------------------------

    [Fact]
    public void ADraftHoldsTwoToTenOptionsAndAnswersOnlyAPoll()
    {
        var draft = new PollDraft();
        Assert.Equal(2, draft.Options.Count);
        Assert.False(draft.MayRemove);
        Assert.False(draft.Remove(0));
        Assert.Null(draft.Checked());

        while (draft.MayAdd)
        {
            Assert.True(draft.Add());
        }
        Assert.Equal(Polls.MaxOptions, draft.Options.Count);
        Assert.False(draft.Add());

        draft.Set(3, "three");
        draft.Set(10, "nowhere");
        draft.Set(-1, "nowhere");
        Assert.True(draft.Remove(2));
        Assert.Equal("three", draft.Options[2]);
        Assert.False(draft.Remove(9));
        Assert.False(draft.Remove(-1));

        draft.Set(0, " Pizza ");
        draft.Set(1, "Pasta");
        Assert.Null(draft.Checked());
        draft.Question = "  ";
        Assert.Null(draft.Checked());
        draft.Question = " Dinner? ";
        var (question, options) = draft.Checked()!.Value;
        Assert.Equal("Dinner?", question);
        Assert.Equal(["Pizza", "Pasta", "three"], options);

        draft.Set(1, "PIZZA");
        Assert.Null(draft.Checked());
    }

    // ---- the words ------------------------------------------------------------------------------------

    [Fact]
    public void APollSaysHowManyVotedAndWhoChoseWhat()
    {
        var poll = Poll(1, false, [Me, 11], [13]);
        Assert.Equal("3 of 4 voted", PollText.Footer(poll, 4, Say));
        Assert.Equal("3 voted", PollText.Footer(poll, 0, Say));
        Assert.Equal("1 of 4 voted", PollText.Footer(Poll(1, false, [Me], []), 4, Say));

        Assert.Equal("Pasta. 1 vote", PollText.OptionLabel(poll.Options[1], chosen: false, Say));
        Assert.Equal("Pizza. 2 votes. Your choice", PollText.OptionLabel(poll.Options[0], chosen: true, Say));

        Assert.Equal("You", PollText.Name(Me, Me, chats.Member, Say));
        Assert.Equal("Bob", PollText.Name(11, Me, chats.Member, Say));
        Assert.Equal("Deleted account", PollText.Name(14, Me, chats.Member, Say));
        Assert.Equal("Deleted account", PollText.Name(17, Me, chats.Member, Say));
        Assert.Equal("Someone", PollText.Name(16, Me, chats.Member, Say));
        Assert.Equal("Someone", PollText.Name(99, Me, chats.Member, Say));

        string Letter(long id) => ((char)('a' + id)).ToString();
        Assert.Equal("a, b, c, d, e +2", PollText.Voters([0, 1, 2, 3, 4, 5, 6], Letter, Say));
        Assert.Equal("a, b, c, d, e", PollText.Voters([0, 1, 2, 3, 4], Letter, Say));
        Assert.Equal("a, b, c, d, e +1", PollText.Voters([0, 1, 2, 3, 4, 5], Letter, Say));
        Assert.Equal(string.Empty, PollText.Voters([], Letter, Say));

        // Neither the people who left nor the deleted account: Anna, Bob, Dora and the nameless member.
        Assert.Equal(4, PollText.MemberCount(chats.Members()));
    }
}
