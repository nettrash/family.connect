using FamilyConnect.App.Logic;
using FamilyConnect.Core.Protocol;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>What the live frames leave on screen and nowhere else: the peer's read marker and an answer mid-stream.</summary>
public sealed class LiveMarksTests
{
    private const string At = "2026-09-16T10:00:00Z";

    [Fact]
    public void APeersMarkerOnlyGoesForwardAndIsPerChat()
    {
        var reads = new PeerReads();
        var changed = new List<long>();
        reads.Changed += changed.Add;

        Assert.Equal(0, reads.UpTo(42));
        Assert.True(reads.Apply(42, 10));
        Assert.False(reads.Apply(42, 9));
        Assert.False(reads.Apply(42, 10));
        Assert.True(reads.Apply(43, 3));
        Assert.Equal((10L, 3L), (reads.UpTo(42), reads.UpTo(43)));
        Assert.Equal([42L, 43L], changed);

        reads.Clear();
        Assert.Equal(0, reads.UpTo(42));
    }

    /// <summary>Streamed text is drawn after the held body until the answer is finished; late and unknown deltas change nothing.</summary>
    [Fact]
    public void StreamedTextFollowsTheBodyUntilTheAnswerIsFinished()
    {
        var answers = new AssistantAnswers();
        var changed = new List<long>();
        answers.Changed += changed.Add;
        var writing = new MessageDto(5, 42, 99, null, "", At);

        answers.Delta(42, 5, "Sure — ", writing);
        answers.Delta(42, 5, "the park.", writing);
        Assert.Equal("Sure — the park.", answers.BodyOf(writing));
        Assert.Equal([42L, 42L], changed);

        var before = answers.Version;
        answers.Delta(42, 5, "late", writing with { EditSeq = 3 });
        answers.Delta(42, 6, "stranger", null);
        answers.Delta(42, 5, "", writing);
        Assert.Equal(before, answers.Version);
        Assert.Equal("Sure — the park.", answers.BodyOf(writing));

        // The finished row is drawn as itself, and forgets what streamed.
        var finished = writing with { Body = "Sure — the park at noon.", EditSeq = 3 };
        Assert.Equal("Sure — the park at noon.", answers.BodyOf(finished));
        answers.Finished(finished);
        Assert.Equal("", answers.BodyOf(writing));
        Assert.True(answers.Version > before);
    }

    [Fact]
    public void AnAnswerThatStoppedSaysSoUntilItIsFinished()
    {
        var answers = new AssistantAnswers();
        var writing = new MessageDto(5, 42, 99, null, "Half", At);

        Assert.False(answers.Failed(writing));
        answers.Stopped(42, 5);
        Assert.True(answers.Failed(writing));
        Assert.False(answers.Failed(writing with { Id = 6 }));
        // However the finished row reached the cache — a frame, or the edits catch-up — it is not a failure.
        Assert.False(answers.Failed(writing with { EditSeq = 4 }));

        answers.Finished(writing with { EditSeq = 4 });
        Assert.False(answers.Failed(writing));

        // An answer that streamed AND stopped, once finished, forgets both — not only the first it looked at.
        answers.Delta(42, 5, " more", writing);
        // Streamed text follows the words the row already holds.
        Assert.Equal("Half more", answers.BodyOf(writing));
        answers.Stopped(42, 5);
        answers.Finished(writing with { EditSeq = 5 });
        Assert.Equal("Half", answers.BodyOf(writing));
        Assert.False(answers.Failed(writing));

        answers.Stopped(42, 5);
        answers.Delta(42, 5, " again", writing);
        var version = answers.Version;
        answers.Clear();
        Assert.False(answers.Failed(writing));
        Assert.Equal("Half", answers.BodyOf(writing));
        Assert.True(answers.Version > version);
        // Finishing something never streamed changes nothing.
        version = answers.Version;
        answers.Finished(writing with { Id = 77 });
        Assert.Equal(version, answers.Version);
    }
}
