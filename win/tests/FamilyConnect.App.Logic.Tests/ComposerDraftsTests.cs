using FamilyConnect.App.Logic;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>Half a thought kept for the chat it was written in (the web client's drafts).</summary>
public sealed class ComposerDraftsTests
{
    [Fact]
    public void ADraftIsKeptPerChatAndGivenBack()
    {
        var drafts = new ComposerDrafts();
        drafts.Save(42, "half a thought");
        drafts.Save(50, "a question ");
        Assert.Equal("half a thought", drafts.Of(42));
        // Kept as written — the white space inside and after the words is the author's.
        Assert.Equal("a question ", drafts.Of(50));
        Assert.Equal(string.Empty, drafts.Of(77));
    }

    [Fact]
    public void WhiteSpaceIsNoDraftAndForgetsTheLast()
    {
        var drafts = new ComposerDrafts();
        drafts.Save(42, "half a thought");
        drafts.Save(42, " \n\t");
        Assert.Equal(string.Empty, drafts.Of(42));
        Assert.Equal(0, drafts.Count);
        drafts.Save(42, "again");
        drafts.Save(42, string.Empty);
        Assert.Equal(0, drafts.Count);
    }

    [Fact]
    public void SendingTakesTheDraftWithIt()
    {
        var drafts = new ComposerDrafts();
        drafts.Save(42, "on its way");
        drafts.Save(50, "stays");
        drafts.Sent(42);
        Assert.Equal(string.Empty, drafts.Of(42));
        Assert.Equal("stays", drafts.Of(50));
    }

    [Fact]
    public void AChatThatIsGoneTakesItsDraftWithIt()
    {
        var drafts = new ComposerDrafts();
        drafts.Save(42, "half a thought");
        drafts.Save(50, "a question");
        drafts.Retain([50, 60]);
        Assert.Equal(string.Empty, drafts.Of(42));
        Assert.Equal("a question", drafts.Of(50));
        Assert.Equal(1, drafts.Count);
    }
}
