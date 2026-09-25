using FamilyConnect.Core.Protocol;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>The member limit's wait and write — the web client's MemberLimit tests, ported.</summary>
public sealed class CapDraftTests
{
    private readonly List<TaskCompletionSource> pauses = [];
    private readonly List<(int? To, TaskCompletionSource<ApiError?> Answer)> sent = [];
    private int changes;

    private CapDraft Draft()
    {
        var draft = new CapDraft(
            to =>
            {
                var answer = new TaskCompletionSource<ApiError?>();
                sent.Add((to, answer));
                return answer.Task;
            },
            (_, token) =>
            {
                var pause = new TaskCompletionSource();
                token.Register(() => pause.TrySetCanceled(token));
                pauses.Add(pause);
                return pause.Task;
            });
        draft.Changed += () => changes++;
        return draft;
    }

    /// <summary>Two clicks before the next redraw are two steps — and one write, of where they ended.</summary>
    [Fact]
    public void TwoQuickStepsAreOneWriteOfWhereTheyEnded()
    {
        var draft = Draft();

        draft.Step(1, held: 3, ceiling: 10);
        draft.Step(1, held: 3, ceiling: 10);

        Assert.Equal(5, draft.Drawn(3));
        Assert.Equal(2, pauses.Count);
        Assert.True(pauses[0].Task.IsCanceled, "the first wait gave way to the second");
        Assert.Empty(sent);
        pauses[^1].SetResult();
        Assert.Equal(5, Assert.Single(sent).To);
    }

    /// <summary>An answer to an OLDER write must not wipe what was changed since.</summary>
    [Fact]
    public void AnOlderAnswerDoesNotUndoANewerChange()
    {
        var draft = Draft();
        draft.Toggle(true, members: 3, ceiling: 10);
        pauses[0].SetResult();
        Assert.Equal(3, Assert.Single(sent).To);

        draft.Step(1, held: null, ceiling: 10);
        sent[0].Answer.SetResult(null);

        Assert.Equal(4, draft.Drawn(null));
        pauses[1].SetResult();
        Assert.Equal(4, sent[1].To);
        sent[1].Answer.SetResult(null);
        // The last answer is the truth now: what is drawn is the family's own again.
        Assert.Equal(7, draft.Drawn(7));
    }

    /// <summary>Turned off, it stays off for the wait, and what goes is a null — which clears.</summary>
    [Fact]
    public void TurnedOffStaysOffWhileItsWriteWaits()
    {
        var draft = Draft();

        draft.Toggle(false, members: 3, ceiling: 10);

        Assert.Null(draft.Drawn(4));
        Assert.Empty(sent);
        pauses[0].SetResult();
        Assert.Null(Assert.Single(sent).To);
    }

    [Fact]
    public void TurningItOnFreezesTheFamilyWhereItStandsInsideTheCeiling()
    {
        var draft = Draft();
        draft.Toggle(true, members: 12, ceiling: 10);
        Assert.Equal(10, draft.Drawn(null));
        draft.Toggle(true, members: 3, ceiling: 10);
        Assert.Equal(3, draft.Drawn(null));
        draft.Toggle(true, members: 0, ceiling: 10);
        Assert.Equal(1, draft.Drawn(null));
    }

    [Fact]
    public void ATypedValueIsHeldInsideOneToTheCeiling()
    {
        var draft = Draft();
        draft.Typed(0, ceiling: 10);
        Assert.Equal(1, draft.Drawn(null));
        draft.Typed(99, ceiling: 10);
        Assert.Equal(10, draft.Drawn(null));
    }

    [Fact]
    public void SteppingALimitThatIsOffDoesNothing()
    {
        var draft = Draft();
        draft.Step(1, held: null, ceiling: 10);
        Assert.Empty(pauses);
        Assert.Null(draft.Drawn(null));
        Assert.Equal(0, changes);
    }

    /// <summary>Only the LAST write's failure is said: an older one was overtaken by what came after it.</summary>
    [Fact]
    public void OnlyTheLastWritesFailureIsSaid()
    {
        var draft = Draft();
        draft.Typed(4, ceiling: 10);
        pauses[0].SetResult();
        draft.Typed(5, ceiling: 10);
        sent[0].Answer.SetResult(ApiError.Transport("reset"));
        Assert.False(draft.Failed);

        pauses[1].SetResult();
        var before = changes;
        sent[1].Answer.SetResult(new ApiError(ErrorCodes.Validation, "x", 400));
        Assert.True(draft.Failed);
        Assert.Equal(before + 1, changes);
        Assert.Equal(6, draft.Drawn(6));

        draft.Typed(6, ceiling: 10);
        Assert.False(draft.Failed, "a new change clears the old failure");
    }
}
