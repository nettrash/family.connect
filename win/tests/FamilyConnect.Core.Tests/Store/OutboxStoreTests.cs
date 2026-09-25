using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.Core.Tests.Store;

/// <summary>
/// The send queue: the one table holding something the server has never seen
/// (docs/protocol.md, "Sending on an unreliable network").
/// </summary>
public class OutboxStoreTests : IDisposable
{
    private static readonly DateTimeOffset Now = new(2026, 9, 12, 10, 0, 0, TimeSpan.Zero);

    private readonly Database database = Database.OpenInMemory();

    public void Dispose() => database.Dispose();

    private OutboxStore Store() => new(database);

    /// <summary>The ceiling, not a sample, so the delays are exact.</summary>
    private static ReconnectBackoff Ceilings() => new(random: ceiling => ceiling);

    [Fact]
    public void ASendIsWrittenDownBeforeAnyOfItMoves()
    {
        var store = Store();
        store.Queue(new OutboxRow(
            "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01", 42, "Dinner at 7?",
            ReplyToMessageId: 1337,
            Mentions: [new MentionDto(9, "Anna")],
            QueuedAt: Now));
        var row = Assert.Single(store.All());
        Assert.Equal(42, row.ChatId);
        Assert.Equal("Dinner at 7?", row.Body);
        Assert.Equal(1337, row.ReplyToMessageId);
        Assert.Equal("Anna", Assert.Single(row.Mentions!).Name);
        Assert.Equal(Now, row.QueuedAt);
        Assert.Equal(0, row.Attempts);
        // Due now: nothing has failed yet, and a queue that waited for a first delay would be a
        // message that sits still under a full signal bar.
        Assert.Null(row.NextAttemptAt);
        Assert.False(row.Failed);
        Assert.Single(store.Due(Now));
    }

    [Fact]
    public void ADeliveredRowGoesAndTheSecondAnswerIsHarmless()
    {
        var store = Store();
        store.Queue(new OutboxRow("k", 42, "hi", QueuedAt: Now));
        Assert.True(store.Delivered("k"));
        Assert.Empty(store.All());
        // An ack that crossed a REST answer for the same id is ordinary, not an error.
        Assert.False(store.Delivered("k"));
    }

    [Fact]
    public void ATransientFailureKeepsTheRowAndSetsWhenToTryAgain()
    {
        var store = Store();
        store.Queue(new OutboxRow("k", 42, "hi", QueuedAt: Now));
        var what = store.Failed("k", ApiError.Transport("connection reset"), Now, Ceilings());
        Assert.Equal(SendRules.Outcome.Retry, what);
        var row = Assert.Single(store.All());
        Assert.Equal(1, row.Attempts);
        Assert.Equal(Now.AddSeconds(1), row.NextAttemptAt);
        Assert.False(row.Failed);
        // Not due until then — and due the moment it is.
        Assert.Empty(store.Due(Now));
        Assert.Single(store.Due(Now.AddSeconds(1)));
    }

    [Fact]
    public void ATerminalRefusalIsShownFailedAndNothingElseIsComing()
    {
        var store = Store();
        store.Queue(new OutboxRow("k", 42, "hi", QueuedAt: Now));
        var what = store.Failed(
            "k", new ApiError(ErrorCodes.MessageTooLong, "too long", 400), Now, Ceilings());
        Assert.Equal(SendRules.Outcome.Failed, what);
        var row = Assert.Single(store.All());
        Assert.True(row.Failed);
        Assert.Equal(ErrorCodes.MessageTooLong, row.FailedCode);
        // A failed row is not retried behind the user's back.
        Assert.Empty(store.Due(Now.AddHours(1)));
    }

    [Fact]
    public void TheBudgetIsSpentAfterSixTriesAndThenItSaysSo()
    {
        var store = Store();
        store.Queue(new OutboxRow("k", 42, "hi", QueuedAt: Now));
        var offline = ApiError.Transport("offline");
        for (var attempt = 1; attempt < SendRules.MaxAttempts; attempt++)
        {
            Assert.Equal(SendRules.Outcome.Retry, store.Failed("k", offline, Now, Ceilings()));
        }
        Assert.Equal(SendRules.Outcome.Failed, store.Failed("k", offline, Now, Ceilings()));
        var row = Assert.Single(store.All());
        Assert.Equal(SendRules.MaxAttempts, row.Attempts);
        Assert.True(row.Failed);
    }

    [Fact]
    public void PressingRetryStartsTheBudgetAgainAndDoesNotWait()
    {
        var store = Store();
        store.Queue(new OutboxRow("k", 42, "hi", QueuedAt: Now));
        store.Failed("k", new ApiError(ErrorCodes.Validation, "no", 400), Now, Ceilings());
        Assert.True(Assert.Single(store.All()).Failed);

        store.Retry("k");
        var row = Assert.Single(store.All());
        Assert.False(row.Failed);
        Assert.Equal(0, row.Attempts);
        Assert.Null(row.NextAttemptAt);
        Assert.Single(store.Due(Now));
    }

    /// <summary>
    /// A row with uploads still owed must never be posted: a message claiming no attachments is a
    /// text message, and the server would take it happily — a delivered bubble with the pictures
    /// gone.
    /// </summary>
    [Fact]
    public void ARowThatOwesUploadsSaysSoAndKeepsTheIdsThatLanded()
    {
        var store = Store();
        store.Queue(new OutboxRow(
            "k", 42, "", PendingFiles: ["/tmp/a.jpg", "/tmp/b.jpg"], QueuedAt: Now));
        Assert.True(Assert.Single(store.All()).OwesUploads);

        store.Uploaded("k", 34, "/tmp/a.jpg");
        var half = Assert.Single(store.All());
        Assert.Equal([34L], half.AttachmentIds!);
        Assert.Equal(["/tmp/b.jpg"], half.PendingFiles!);
        Assert.True(half.OwesUploads);

        store.Uploaded("k", 35, "/tmp/b.jpg");
        var whole = Assert.Single(store.All());
        // The ids that DID land are kept and reused, so a retry pushes only the remainder.
        Assert.Equal([34L, 35L], whole.AttachmentIds!);
        Assert.False(whole.OwesUploads);
        Assert.Empty(whole.PendingFiles!);
    }

    [Fact]
    public void AFailureForARowThatIsAlreadyGoneIsNotAFailureToShow()
    {
        var store = Store();
        // Delivered by the socket while a REST attempt for the same id was in flight.
        var what = store.Failed("gone", ApiError.Transport("late"), Now, Ceilings());
        Assert.Equal(SendRules.Outcome.Retry, what);
        Assert.Empty(store.All());
    }

    [Fact]
    public void OneChatsQueueIsItsOwn()
    {
        var store = Store();
        store.Queue(new OutboxRow("a", 42, "one", QueuedAt: Now));
        store.Queue(new OutboxRow("b", 43, "two", QueuedAt: Now.AddSeconds(1)));
        store.Queue(new OutboxRow("c", 42, "three", QueuedAt: Now.AddSeconds(2)));
        Assert.Equal(["a", "c"], store.ForChat(42).Select(row => row.ClientMsgId));
        Assert.Equal(["b"], store.ForChat(43).Select(row => row.ClientMsgId));
        // Oldest first, which is the order they were written in.
        Assert.Equal(["a", "b", "c"], store.All().Select(row => row.ClientMsgId));
    }

    [Fact]
    public void AnExpiredUploadIsQueuedAgainRatherThanGivenUpOn()
    {
        var store = Store();
        store.Queue(new OutboxRow("k", 42, "", AttachmentIds: [34], QueuedAt: Now));
        var expired = new ApiError(ErrorCodes.AttachmentExpired, "swept", 404);
        Assert.Equal(
            SendRules.Outcome.ReuploadAndRetry, store.Failed("k", expired, Now, Ceilings()));
        var row = Assert.Single(store.All());
        Assert.False(row.Failed);
        Assert.Single(store.Due(Now));
        // Without the bytes there is no way back, and it says so instead of retrying for ever.
        Assert.Equal(
            SendRules.Outcome.Failed,
            store.Failed("k", expired, Now, Ceilings(), holdsBytes: false));
        Assert.True(Assert.Single(store.All()).Failed);
    }
}
