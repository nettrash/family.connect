using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>A body as a bubble lays it out: blocks, styled runs, links, and which names open a chat.</summary>
public sealed class BubbleBodyTests
{
    private static readonly MemberDto[] Roster = [new(7, "anna", "Anna"), new(11, "bob", "Bob"), new(12, "carl", "Carl", HasLeft: true)];

    private static IReadOnlyList<BodyBlock> Lay(string body, MentionDto[]? named = null, long me = 1, Func<long, bool>? blocked = null) =>
        BubbleBody.Lay(body, named, Roster, me, blocked ?? (_ => false));

    private static IReadOnlyList<BodyRun> Runs(BodyBlock block) => Assert.IsType<BodyTextBlock>(block).Runs;

    private static BodyRun Find(IReadOnlyList<BodyRun> runs, string text) => Assert.Single(runs, run => run.Text == text);

    [Fact]
    public void MarkdownAndLinksReachTheRunsWithOrWithoutNames()
    {
        var runs = Runs(Assert.Single(Lay("**hi** see https://example.com and *so*")));
        Assert.True(Find(runs, "hi").Style.Strong);
        Assert.Equal("https://example.com", Find(runs, "https://example.com").Link);
        Assert.True(Find(runs, "so").Style.Emphasis);
        Assert.All(runs, run => Assert.False(run.Marked));
    }

    /// <summary>A name is a door only onto somebody this reader can message: not themselves, not somebody gone, not somebody blocked.</summary>
    [Fact]
    public void ANameOpensOnlyWhereTheReaderCanMessageThem()
    {
        MentionDto[] named = [new(7, "Anna"), new(11, "Bob"), new(12, "Carl")];
        var runs = Runs(Assert.Single(Lay("**@Anna** @Bob @Carl", named, me: 11)));
        Assert.Equal((7L, true, true), (Find(runs, "@Anna").MemberId, Find(runs, "@Anna").Opens, Find(runs, "@Anna").Marked));
        Assert.Equal((11L, false, true), (Find(runs, "@Bob").MemberId, Find(runs, "@Bob").Opens, Find(runs, "@Bob").Marked));
        Assert.Equal((12L, false, true), (Find(runs, "@Carl").MemberId, Find(runs, "@Carl").Opens, Find(runs, "@Carl").Marked));

        var blocked = Runs(Assert.Single(Lay("@Anna", named, me: 11, blocked: id => id == 7)));
        Assert.False(Find(blocked, "@Anna").Opens);
    }

    /// <summary>A table is a block of its own, and only the first block may carry the picture token.</summary>
    [Fact]
    public void ATableSplitsTheBodyAndOnlyTheFirstBlockAsksForAPicture()
    {
        var blocks = Lay("/draw a cat\n| a | b |\n| --- | :-: |\n| 1 | 2 |\n/draw a dog");
        Assert.Equal(3, blocks.Count);
        Assert.True(Find(Runs(blocks[0]), "/draw").Marked);
        var table = Assert.IsType<BodyTableBlock>(blocks[1]).Table;
        Assert.Equal([MarkdownAlignment.Leading, MarkdownAlignment.Center], table.Alignments);
        Assert.Equal("2", table.Rows[0][1].Plain);
        Assert.All(Runs(blocks[2]), run => Assert.False(run.Marked));
    }

    /// <summary>Only an absolute http(s), mailto or tel target is handed to the system.</summary>
    [Fact]
    public void OnlyWebMailAndPhoneTargetsOpen()
    {
        Assert.NotNull(BubbleBody.Openable("https://example.com/a?b#c"));
        Assert.NotNull(BubbleBody.Openable("mailto:me@example.com"));
        Assert.NotNull(BubbleBody.Openable("tel:5551234567"));
        Assert.Null(BubbleBody.Openable("javascript:alert(1)"));
        Assert.Null(BubbleBody.Openable("file:///etc/passwd"));
        Assert.Null(BubbleBody.Openable(null));
    }

    [Fact]
    public void ALinkWaitsTheMacsBeatForADoubleClick() => Assert.Equal(TimeSpan.FromMilliseconds(350), BubbleBody.LinkDelay);
}
