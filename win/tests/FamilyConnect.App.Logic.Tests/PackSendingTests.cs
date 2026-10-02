using FamilyConnect.App.Logic;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// Where the sticker button is offered, and what stands between one click in the panel and the
/// send (docs/protocol.md, "Sending one" and "Consenting to the assistant").
/// </summary>
/// <remarks>
/// The window only draws what these answer: the button's visibility is <see cref="PackSending.Offered"/>
/// and <see cref="PackSending.OfferedInThread"/>, and the click goes through <see cref="PackSending.For"/>
/// before anything is staged — so the rule is pinned here, where no window is needed to run it.
/// </remarks>
public class PackSendingTests
{
    private const string Processor = "Microsoft — Azure OpenAI";
    private const string Agreed = "2026-09-13T10:00:00Z";

    /// <summary>
    /// THE BUTTON IS OFFERED IN EVERY CHAT A MESSAGE CAN BE SENT IN: the family chat, a one-to-one
    /// chat, and the assistant's own chat — none of them is left out.
    /// </summary>
    [Theory]
    [InlineData("family")]
    [InlineData("direct")]
    [InlineData("ai")]
    public void TheButtonIsOfferedInEveryKindOfChat(string kind)
    {
        Assert.True(PackSending.Offered(kind, editing: false, packOffered: true));
    }

    [Fact]
    public void TheButtonIsNotOfferedWithoutAPackAChatOrWhileEditing()
    {
        // A server that predates packs names no ceilings: no button, rather than a 404 on a click.
        Assert.False(PackSending.Offered("family", editing: false, packOffered: false));
        Assert.False(PackSending.Offered("ai", editing: false, packOffered: false));
        // No conversation open.
        Assert.False(PackSending.Offered(null, editing: false, packOffered: true));
        // A sticker is its own message, and an edit is somebody else's.
        Assert.False(PackSending.Offered("family", editing: true, packOffered: true));
        Assert.False(PackSending.Offered("ai", editing: true, packOffered: true));
    }

    /// <summary>The THREAD's composer has the button wherever it can send — which needs a root to answer.</summary>
    [Fact]
    public void TheThreadComposerOffersItWhereverItCanSend()
    {
        Assert.True(PackSending.OfferedInThread(hasRoot: true, packOffered: true));
        Assert.False(PackSending.OfferedInThread(hasRoot: false, packOffered: true));
        Assert.False(PackSending.OfferedInThread(hasRoot: true, packOffered: false));
    }

    /// <summary>
    /// IN THE ASSISTANT'S CHAT A STICKER GOES THROUGH THE SAME QUESTION ANY MESSAGE THERE DOES —
    /// never around it. Until this member has agreed the click ASKS; it sends only once the
    /// server's <c>/me</c> carries the answer.
    /// </summary>
    [Fact]
    public void InTheAssistantsChatAStickerIsAskedAboutLikeAnyMessage()
    {
        Assert.Equal(PackSending.Gate.Ask, PackSending.For("ai", hasAssistant: true, Processor, agreedAt: null));
        Assert.Equal(PackSending.Gate.Ask, PackSending.For("ai", hasAssistant: true, Processor, agreedAt: "  "));
        Assert.Equal(PackSending.Gate.Send, PackSending.For("ai", hasAssistant: true, Processor, Agreed));
    }

    /// <summary>The gate is the composer's own, word for word: what a message with no body is asked, a sticker is.</summary>
    [Theory]
    [InlineData("ai", true, Processor, null)]
    [InlineData("ai", true, Processor, Agreed)]
    [InlineData("ai", true, null, null)]
    [InlineData("ai", true, "  ", Agreed)]
    [InlineData("ai", false, null, null)]
    [InlineData("family", true, Processor, null)]
    [InlineData("family", true, null, null)]
    [InlineData("direct", true, Processor, null)]
    [InlineData(null, true, Processor, null)]
    public void TheGateIsTheOneAnEmptyMessageGoesThrough(string? kind, bool hasAssistant, string? processor, string? agreedAt)
    {
        var expected =
            AssistantConsent.IsRequired(kind, string.Empty, processor, agreedAt) ? PackSending.Gate.Ask
            : AssistantConsent.IsWithheldFromAnUnnamedAssistant(kind, string.Empty, hasAssistant, processor) ? PackSending.Gate.Withheld
            : PackSending.Gate.Send;

        Assert.Equal(expected, PackSending.For(kind, hasAssistant, processor, agreedAt));
    }

    /// <summary>
    /// An assistant whose owner this server will not name gets NOTHING, a sticker included: there
    /// is no honest way to ask, so there is nothing to send.
    /// </summary>
    [Fact]
    public void AStickerIsWithheldFromAnAssistantNobodyNamed()
    {
        Assert.Equal(PackSending.Gate.Withheld, PackSending.For("ai", hasAssistant: true, processor: null, agreedAt: null));
        Assert.Equal(PackSending.Gate.Withheld, PackSending.For("ai", hasAssistant: true, processor: " ", agreedAt: Agreed));
    }

    /// <summary>
    /// Between people nothing is asked: a sticker has no words, so in the family chat it cannot
    /// mention the assistant, and a one-to-one chat never reaches the model at all. That holds in
    /// a thread too — a thread's sticker is a reply in the same chat.
    /// </summary>
    [Theory]
    [InlineData("family")]
    [InlineData("direct")]
    public void BetweenPeopleAStickerIsSimplySent(string kind)
    {
        Assert.Equal(PackSending.Gate.Send, PackSending.For(kind, hasAssistant: true, Processor, agreedAt: null));
        Assert.Equal(PackSending.Gate.Send, PackSending.For(kind, hasAssistant: true, processor: null, agreedAt: null));
        Assert.Equal(PackSending.Gate.Send, PackSending.For(kind, hasAssistant: false, processor: null, agreedAt: null));
    }
}
