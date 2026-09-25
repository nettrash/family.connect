using FamilyConnect.Core.Protocol;

namespace FamilyConnect.Core.Tests.Text;

/// <summary>A poll's arithmetic: who holds what, how many voted, how full a bar is, and which polls still want an answer.</summary>
public class PollsTests
{
    private const long Me = 7;

    private static PollDto Poll(params long[][] votes) =>
        new(1, false, [.. votes.Select((voters, index) => new PollOptionDto(index + 1, $"Option {index}", voters))]);

    private static MessageDto Message(long id, PollDto poll, long sender = 9) =>
        new(id, 42, sender, null, "Dinner?", "2026-09-14T10:00:00Z", Poll: poll);

    [Fact]
    public void TheArithmeticCountsEveryoneAndTheNamesSkipTheBlocked()
    {
        var poll = Poll([7, 9], [11], []);
        Assert.Equal(1, Polls.MyOption(poll, Me));
        Assert.Null(Polls.MyOption(poll, 4));
        Assert.Equal(3, Polls.VoterCount(poll));
        Assert.Equal(2.0 / 3.0, Polls.Fraction(poll, 2), 9);
        Assert.Equal(0, Polls.Fraction(Poll([], []), 0));
        // A share of the votes CAST: two votes over four options is a half each, not a quarter.
        Assert.Equal(0.5, Polls.Fraction(Poll([7], [9], [], []), 1), 9);

        // Identity only, in the order they voted: the count above goes on counting 9.
        Assert.Equal([7L], Polls.DrawableVoters([7, 9], voter => voter == 9));
        Assert.Equal([9L, 7L], Polls.DrawableVoters([9, 7], _ => false));
    }

    /// <summary>A member holds one option; a state that somehow said otherwise still counts them once.</summary>
    [Fact]
    public void AVoterIsCountedOnce() => Assert.Equal(1, Polls.VoterCount(Poll([7], [7])));

    [Fact]
    public void TappingTheOptionYouHoldRetractsAndAnyOtherCasts()
    {
        var poll = Poll([7], [9]);
        Assert.True(Polls.TapRetracts(poll, 1, Me));
        Assert.False(Polls.TapRetracts(poll, 2, Me));
        Assert.False(Polls.TapRetracts(poll, 1, 4));
    }

    [Fact]
    public void OnlyANumberedOpenPollTakesVotes()
    {
        Assert.True(Polls.Votable(Message(5, Poll([], []))));
        Assert.False(Polls.Votable(Message(0, Poll([], []))));
        Assert.False(Polls.Votable(Message(5, Poll([], []) with { Closed = true })));
        Assert.False(Polls.Votable(new MessageDto(5, 42, 9, null, "no poll", "2026-09-14T10:00:00Z")));
    }

    [Fact]
    public void AQuestionIsMoreThanWhiteSpace()
    {
        Assert.False(Polls.HasQuestion(" \n\t"));
        Assert.False(Polls.HasQuestion(string.Empty));
        Assert.True(Polls.HasQuestion(" ?"));
    }

    /// <summary>
    /// The badge: open, unanswered, not a blocked member's and not unnumbered — each message once, by its
    /// NEWEST copy, so a vote the list has seen and the cache has not is not asked for again.
    /// </summary>
    [Fact]
    public void AnUnansweredPollIsOpenNotMineToChaseTwiceAndCountedByItsNewestCopy()
    {
        var open = Poll([], []);
        var held = new[]
        {
            Message(100, open),
            Message(90, Poll([Me], [])),
            Message(80, open with { Closed = true }),
            Message(70, open, sender: 13),
            Message(0, open),
            Message(50, open with { PollSeq = 3, Closed = true }),
            Message(40, open, sender: Me),
            new MessageDto(30, 42, 9, null, "not a poll", "2026-09-14T10:00:00Z"),
        };
        var listed = new[]
        {
            Message(100, Poll([Me], []) with { PollSeq = 2 }),
            Message(60, open),
            Message(50, open with { PollSeq = 2 }),
        };
        bool Blocked(long user) => user == 13;

        // 60, and the reader's own 40: 100 was answered in the newer copy, 50 closed in the newer copy.
        Assert.Equal(2, Polls.Unanswered(held, listed, Me, Blocked));
        // Without the list, 100 is still waiting.
        Assert.Equal(2, Polls.Unanswered(held, [], Me, Blocked));
        Assert.Equal(3, Polls.Unanswered(held, [], Me, _ => false));
    }

    [Fact]
    public void TheOptionsSentAreTheOnesChecked()
    {
        Assert.Equal(["Pizza", "Pasta"], Polls.Sanitized([" Pizza", "", "Pasta "])!);
        Assert.Null(Polls.Sanitized(["Pizza"]));
    }
}
