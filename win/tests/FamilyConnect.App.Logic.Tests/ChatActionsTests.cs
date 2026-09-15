using System.Net;
using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// What a conversation does besides reading: what is still on its way, reactions, edits, who is
/// typing, and what a reply quotes.
/// </summary>
public class ChatActionsTests : IDisposable
{
    private const long Me = 7;
    private const long Chat = 42;
    private const string Sent = "2026-09-13T10:00:00Z";

    private static readonly IStringCatalog Say = EnglishCatalog.Instance;
    private static readonly string Heart = Reactions.Quick[0];
    private static readonly string Up = Reactions.Quick[1];

    private readonly Database cache = Database.OpenInMemory();
    private readonly ChatStore chats;
    private readonly OutboxStore outbox;

    public ChatActionsTests()
    {
        chats = new ChatStore(cache, () => Me);
        outbox = new OutboxStore(cache);
        chats.Replace([new ChatRowDto(new ChatDto(Chat, "family", "The Smiths"))]);
        chats.Replace(
            [new MemberDto(7, "anna", "Anna"), new MemberDto(11, "bob", "Bob"), new MemberDto(12, "carl", "Carl"), new MemberDto(13, "dora", "Dora")],
            [new MemberDto(14, "gone", "Eve", HasLeft: true, Deleted: true)]);
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

    private ConversationModel Open(Server server)
    {
        var api = new ApiClient(new HttpClient(server), ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken"));
        var socket = new Wire();
        var sending = new SendPipeline(socket, outbox, chats, api, wait: (_, _) => Task.CompletedTask);
        return new ConversationModel(Chat, chats, api, sending, socket, outbox: outbox);
    }

    [Fact]
    public void WhatIsStillOnItsWayIsThisChatsOwnAndCanBeRetriedOrGivenUp()
    {
        var chat = Open(new Server());
        var first = chat.Send("Dinner at 7?");
        var second = chat.Send("…or 8?");
        outbox.Queue(new OutboxRow("elsewhere", 99, "not here", QueuedAt: DateTimeOffset.UnixEpoch));

        Assert.Equal([first.ClientMsgId, second.ClientMsgId], chat.Pending().Select(row => row.ClientMsgId));

        outbox.Refuse(second.ClientMsgId, ErrorCodes.Blocked);
        Assert.True(chat.Pending()[1].Failed);
        chat.Retry(second.ClientMsgId);
        Assert.False(chat.Pending()[1].Failed);

        Assert.True(chat.Discard(first.ClientMsgId));
        Assert.Equal([second.ClientMsgId], chat.Pending().Select(row => row.ClientMsgId));
    }

    [Fact]
    public void AConversationWithoutAnOutboxHasNothingPending()
    {
        var api = new ApiClient(new HttpClient(new Server()), ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken"));
        var socket = new Wire();
        var sending = new SendPipeline(socket, outbox, chats, api);
        var chat = new ConversationModel(Chat, chats, api, sending, socket);
        chat.Send("queued all the same");
        Assert.Empty(chat.Pending());
    }

    /// <summary>
    /// THE TAP IS DECIDED HERE: an emoji the reader does not hold is a PUT with it, the one they
    /// hold is a DELETE — and the answer, the whole list, is what the cache takes.
    /// </summary>
    [Fact]
    public async Task ReactingSetsWhatIsNotMineAndRemovesWhatIs()
    {
        chats.Apply(new MessageDto(1, Chat, 9, null, "Dinner", Sent, Reactions: [new ReactionDto(9, Up)], ReactionSeq: 3));
        var server = new Server().Then(
            "/chats/42/messages/1/reaction",
            (HttpStatusCode.OK, $$"""{"message_id": 1, "reaction_seq": 4, "reactions": [{"user_id": 9, "emoji": "{{Up}}"}, {"user_id": 7, "emoji": "{{Heart}}"}]}"""),
            (HttpStatusCode.OK, $$"""{"message_id": 1, "reaction_seq": 5, "reactions": [{"user_id": 9, "emoji": "{{Up}}"}]}"""));
        var chat = Open(server);

        Assert.Null(await chat.ReactAsync(1, Heart));
        // The body is JSON, and the encoder may escape an emoji as surrogate escapes: what the
        // server reads is the decoded string, so that is what is compared.
        Assert.Equal(Heart, System.Text.Json.JsonDocument.Parse(Assert.Single(server.Bodies))
            .RootElement.GetProperty("emoji").GetString());
        Assert.Equal(Heart, Reactions.Mine(chats.Message(1)!.Reactions!, Me));
        Assert.Equal(4, chats.Message(1)!.ReactionSeq);

        Assert.Null(await chat.ReactAsync(1, Heart));
        // A DELETE carries no body, so there is still exactly one.
        Assert.Single(server.Bodies);
        Assert.Equal(2, server.Asked.Count);
        Assert.Null(Reactions.Mine(chats.Message(1)!.Reactions!, Me));
        Assert.Equal(5, chats.Message(1)!.ReactionSeq);
        // The answer to this client's own write is EVIDENCE: the chat's catch-up cursor stays where the
        // frames and pages left it, or the next catch-up would skip states this device never saw.
        Assert.True((chats.Chat(Chat)!.MaxReactionSeq ?? 0) < 4);
    }

    [Fact]
    public async Task ARefusedReactionChangesNothing()
    {
        chats.Apply(new MessageDto(1, Chat, 9, null, "Dinner", Sent, Reactions: [new ReactionDto(9, Up)], ReactionSeq: 3));
        var chat = Open(new Server().On(
            "/chats/42/messages/1/reaction", """{"error": {"code": "blocked", "message": "no"}}""", HttpStatusCode.Forbidden));

        var error = await chat.ReactAsync(1, Heart);

        Assert.Equal(ErrorCodes.Blocked, error?.Code);
        Assert.Single(chats.Message(1)!.Reactions!);
        Assert.Equal(3, chats.Message(1)!.ReactionSeq);
    }

    [Fact]
    public async Task EditingSendsTrimmedWordsAndTheWordsItAlreadyHasSendNothing()
    {
        chats.Apply(new MessageDto(1, Chat, Me, "c-1", "Dinner at 7?", Sent));
        var server = new Server().On(
            "/chats/42/messages/1",
            """{"message": {"id": 1, "chat_id": 42, "sender_id": 7, "client_msg_id": "c-1", "body": "Dinner at 8?", "created_at": "2026-09-13T10:00:00Z", "edited_at": "2026-09-13T10:05:00Z", "edit_seq": 9}}""");
        var chat = Open(server);

        Assert.Null(await chat.EditAsync(1, "  Dinner at 8?  "));
        Assert.Equal("Dinner at 8?", System.Text.Json.JsonDocument.Parse(Assert.Single(server.Bodies))
            .RootElement.GetProperty("body").GetString());
        Assert.Equal("Dinner at 8?", chats.Message(1)!.Body);
        Assert.Equal(9, chats.Message(1)!.EditSeq);
        Assert.True((chats.Chat(Chat)!.MaxEditSeq ?? 0) < 9);

        Assert.Null(await chat.EditAsync(1, "Dinner at 8?"));
        Assert.Equal(ErrorCodes.MessageEmpty, (await chat.EditAsync(1, "   "))?.Code);
        Assert.Single(server.Asked);
    }

    /// <summary>
    /// The answer to an edit is one message with nothing known under it: held and drawn, and not where the next catch-up
    /// starts (docs/protocol.md, "Best-effort delivery", step 3).
    /// </summary>
    [Fact]
    public async Task AnEditsAnswerIsNotWhereTheCatchUpStarts()
    {
        chats.Apply(new MessageDto(1, Chat, Me, "c-1", "Dinner at 7?", Sent), inSequence: false);
        var chat = Open(new Server().On(
            "/chats/42/messages/1",
            """{"message": {"id": 1, "chat_id": 42, "sender_id": 7, "client_msg_id": "c-1", "body": "Dinner at 8?", "created_at": "2026-09-13T10:00:00Z", "edited_at": "2026-09-13T10:05:00Z", "edit_seq": 9}}"""));

        Assert.Null(await chat.EditAsync(1, "Dinner at 8?"));

        Assert.Equal("Dinner at 8?", chats.Message(1)!.Body);
        Assert.Null(chats.CatchUpCursor(Chat));
    }

    [Fact]
    public void OnlyTheReadersOwnWordsMayBeEdited()
    {
        Bubble Of(long sender, string body = "words", CallRecordDto? call = null, PollDto? poll = null) =>
            new(new MessageDto(1, Chat, sender, null, body, Sent, Call: call, Poll: poll), false, false, sender == Me);

        Assert.True(ConversationModel.MayEdit(Of(Me)));
        Assert.False(ConversationModel.MayEdit(Of(11)));
        Assert.False(ConversationModel.MayEdit(Of(Me, body: "")));
        Assert.False(ConversationModel.MayEdit(Of(Me, call: new CallRecordDto("missed"))));
        Assert.False(ConversationModel.MayEdit(Of(Me, poll: new PollDto(1, false, []))));
    }

    [Fact]
    public void TypingIsMomentaryInMemberOrderAndNeverMineOrABlockedMembers()
    {
        var now = new DateTimeOffset(2026, 9, 13, 10, 0, 0, TimeSpan.Zero);
        var roster = new TypingRoster(chats, () => now);

        roster.Heard(Chat, 12);
        roster.Heard(Chat, 11);
        roster.Heard(Chat, Me);
        roster.Heard(Chat, 13);
        Assert.Equal("Bob, Carl are typing…", roster.Line(Chat));
        Assert.Equal(string.Empty, roster.Line(99));

        roster.Spoke(new MessageDto(5, Chat, 12, null, "here", Sent));
        Assert.Equal("Bob is typing…", roster.Line(Chat));

        now += TypingRoster.Lasts - TimeSpan.FromMilliseconds(1);
        Assert.Equal("Bob is typing…", roster.Line(Chat));
        now += TimeSpan.FromMilliseconds(1);
        Assert.Equal(string.Empty, roster.Line(Chat));

        roster.Heard(Chat, 40);
        Assert.Equal("Someone is typing…", roster.Line(Chat));
    }

    [Fact]
    public void AQuoteNamesWhoItAnswersUnlessTheyAreBlocked()
    {
        MessageDto ReplyTo(long sender) =>
            new(2, Chat, 11, null, "yes", Sent, ReplyTo: new ReplyToDto(1, sender, "Dinner at 7?"));

        Assert.Null(Quotes.Of(new MessageDto(3, Chat, 11, null, "plain", Sent), chats, Say));
        Assert.Equal(new Quote("You", "Dinner at 7?", false), Quotes.Of(ReplyTo(Me), chats, Say));
        Assert.Equal(new Quote("Carl", "Dinner at 7?", false), Quotes.Of(ReplyTo(12), chats, Say));
        Assert.Equal(new Quote("Deleted account", "Dinner at 7?", false), Quotes.Of(ReplyTo(14), chats, Say));
        Assert.Equal(new Quote("Someone", "Dinner at 7?", false), Quotes.Of(ReplyTo(40), chats, Say));
        Assert.Equal(new Quote(string.Empty, "Replying to a hidden message", true), Quotes.Of(ReplyTo(13), chats, Say));

        Assert.Equal("Replying to Carl", Quotes.Banner(new MessageDto(4, Chat, 12, null, "hi", Sent), chats, Say));
        Assert.Equal("Replying to a hidden message", Quotes.Banner(new MessageDto(5, Chat, 13, null, "hi", Sent), chats, Say));
    }
}
