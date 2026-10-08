using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// The line said while a picture is being asked for — real names and brands are often refused (docs/protocol.md, "A
/// refused description is reworded once") — and only there.
/// </summary>
public sealed class PictureHintTests
{
    private static readonly AssistantDto Draws =
        new(99, "Assistant", "@ai", Images: true, Processor: "Microsoft — Azure OpenAI");

    private static readonly AssistantDto Talks =
        new(99, "Assistant", "@ai", Processor: "Microsoft — Azure OpenAI");

    /// <summary>A server that draws but named nobody: this client offers it nothing, so it is told nothing about drawing.</summary>
    private static readonly AssistantDto Unnamed = new(99, "Assistant", "@ai", Images: true);

    /// <summary>
    /// From the moment the token is typed — "Ask for a picture" leaves <c>/draw </c> with nothing after it, and that is
    /// exactly when the description is about to be written — and through the description.
    /// </summary>
    [Fact]
    public void TheAssistantsChatSaysItOnceADrawIsBegun()
    {
        foreach (var draft in new[] { "/draw ", "/draw a cat in a hat", "  /draw a cat", "\n/Draw a cat", "/DRAW\ta cat", "@ai /draw a cat" })
        {
            Assert.True(PictureHint.ForComposer("ai", draft, Draws, editing: false), draft);
        }
    }

    [Fact]
    public void NotForADraftThatIsNoPictureRequest()
    {
        foreach (var draft in new[] { "", "/draw", "/drawer a cat", "draw a cat", "what does /draw do?", "a cat /draw ", "@ai @ai /draw a cat" })
        {
            Assert.False(PictureHint.ForComposer("ai", draft, Draws, editing: false), draft);
        }
        Assert.False(PictureHint.ForComposer("ai", null, Draws, editing: false));
    }

    /// <summary>In the family chat a picture is <c>@ai /draw</c>: a bare <c>/draw</c> reaches nobody and nothing refuses it.</summary>
    [Fact]
    public void TheFamilyChatSaysItOnlyForAMentionThatDraws()
    {
        Assert.True(PictureHint.ForComposer("family", "@ai /draw ", Draws, editing: false));
        Assert.True(PictureHint.ForComposer("family", " @AI /draw a cat", Draws, editing: false));
        Assert.False(PictureHint.ForComposer("family", "/draw a cat", Draws, editing: false));
        Assert.False(PictureHint.ForComposer("family", "hey @ai /draw a cat", Draws, editing: false));
        Assert.False(PictureHint.ForComposer("family", "@ai what is a cat?", Draws, editing: false));
    }

    [Fact]
    public void NotWhereNothingDraws()
    {
        Assert.False(PictureHint.ForComposer("direct", "/draw a cat", Draws, editing: false));
        Assert.False(PictureHint.ForComposer(null, "/draw a cat", Draws, editing: false));
        Assert.False(PictureHint.ForComposer("ai", "/draw a cat", Talks, editing: false));
        Assert.False(PictureHint.ForComposer("ai", "/draw a cat", null, editing: false));
        Assert.False(PictureHint.ForComposer("ai", "/draw a cat", Unnamed, editing: false));
        Assert.False(PictureHint.ForComposer("family", "@ai /draw a cat", Talks, editing: false));
    }

    /// <summary>An edit rewrites what was said; it cannot become a request, so nothing is about to be refused.</summary>
    [Fact]
    public void NotWhileAMessageIsEdited()
    {
        Assert.False(PictureHint.ForComposer("ai", "/draw a cat", Draws, editing: true));
        Assert.False(PictureHint.ForComposer("family", "@ai /draw a cat", Draws, editing: true));
    }

    /// <summary>
    /// The backdrop's line goes with its button: the author's event, on a server that draws — and names who does, since
    /// the title needs the author's consent and nobody can agree to an unnamed recipient.
    /// </summary>
    [Fact]
    public void TheBackdropSaysItWhereverItsButtonIs()
    {
        Assert.True(PictureHint.OffersBackdrop(editable: true, Draws));
        Assert.False(PictureHint.OffersBackdrop(editable: false, Draws));
        Assert.False(PictureHint.OffersBackdrop(editable: true, Talks));
        Assert.False(PictureHint.OffersBackdrop(editable: true, null));
        Assert.False(PictureHint.OffersBackdrop(editable: true, Unnamed));
    }

    [Fact]
    public void TheLineIsTheAppsSentence() =>
        Assert.Equal(
            "Describe people and things in general words — real names and brands are often refused.",
            PictureHint.Sentence(EnglishCatalog.Instance));
}
