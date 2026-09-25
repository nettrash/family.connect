using FamilyConnect.App.Logic;
using FamilyConnect.Core.Protocol;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>The web client's timeline rules for a bubble: the assistant, an awaited answer, the tick, the Safety items.</summary>
public sealed class BubbleRulesTests
{
    private const long Me = 7;
    private const long Anna = 11;
    private const long Assistant = 99;
    private const string At = "2026-09-16T10:00:00Z";

    private static MessageDto From(long sender, string body = "", long id = 5) => new(id, 42, sender, null, body, At);

    [Fact]
    public void TheAssistantIsItsAccountInTheFamilyAndAnybodyButYouInItsOwnChat()
    {
        Assert.True(BubbleRules.IsAssistant(From(Assistant), Me, assistantChat: false, Assistant));
        Assert.False(BubbleRules.IsAssistant(From(Anna), Me, assistantChat: false, Assistant));
        Assert.False(BubbleRules.IsAssistant(From(Assistant), Me, assistantChat: false, assistantUserId: null));
        Assert.True(BubbleRules.IsAssistant(From(Anna), Me, assistantChat: true, assistantUserId: null));
        Assert.False(BubbleRules.IsAssistant(From(Me), Me, assistantChat: true, Assistant));

        Assert.True(BubbleRules.IsOtherMember(From(Anna), Me, false, Assistant));
        Assert.False(BubbleRules.IsOtherMember(From(Me), Me, false, Assistant));
        Assert.False(BubbleRules.IsOtherMember(From(Assistant), Me, false, Assistant));
        Assert.False(BubbleRules.IsOtherMember(From(Anna), Me, true, Assistant));
    }

    /// <summary>A numbered row carrying nothing, not mine, from somebody who can be the assistant — and nothing else.</summary>
    [Fact]
    public void AnAnswerIsAwaitedWhileItCarriesNothing()
    {
        Assert.True(BubbleRules.Awaited(From(Assistant), "", Me, false, Assistant));
        Assert.True(BubbleRules.Awaited(From(Anna), "", Me, assistantChat: true, null));

        Assert.False(BubbleRules.Awaited(From(Assistant), "Sure — ", Me, false, Assistant));
        Assert.False(BubbleRules.Awaited(From(Assistant) with { Attachments = [new AttachmentDto(3, "photo")] }, "", Me, false, Assistant));
        Assert.False(BubbleRules.Awaited(From(Assistant) with { Poll = new PollDto(1, false, []) }, "", Me, false, Assistant));
        Assert.False(BubbleRules.Awaited(From(Assistant) with { Call = new CallRecordDto("missed") }, "", Me, false, Assistant));
        Assert.False(BubbleRules.Awaited(From(Me), "", Me, true, Assistant));
        Assert.False(BubbleRules.Awaited(From(Assistant, id: 0), "", Me, false, Assistant));
        Assert.False(BubbleRules.Awaited(From(Anna), "", Me, false, Assistant));
    }

    [Fact]
    public void TheTickIsADirectChatFactAboutMyNumberedMessages()
    {
        Assert.True(BubbleRules.ShowsTick(From(Me, "hi"), Me, familyChat: false));
        Assert.False(BubbleRules.ShowsTick(From(Me, "hi"), Me, familyChat: true));
        Assert.False(BubbleRules.ShowsTick(From(Anna, "hi"), Me, familyChat: false));
        Assert.False(BubbleRules.ShowsTick(From(Me, "hi", id: 0), Me, familyChat: false));

        Assert.True(BubbleRules.Seen(From(Me, "hi", id: 5), Me, false, peerReadUpTo: 5));
        Assert.False(BubbleRules.Seen(From(Me, "hi", id: 6), Me, false, peerReadUpTo: 5));
        Assert.False(BubbleRules.Seen(From(Me, "hi", id: 5), Me, familyChat: true, peerReadUpTo: 9));
        Assert.False(BubbleRules.Seen(From(Anna, "hi", id: 5), Me, false, peerReadUpTo: 9));
        Assert.False(BubbleRules.Seen(From(Me, "hi", id: 0), Me, false, peerReadUpTo: 9));
    }

    [Fact]
    public void OnlyAnotherMembersNumberedMessageMayBeReported()
    {
        Assert.True(BubbleRules.MayReport(From(Anna), Me, false, Assistant));
        Assert.False(BubbleRules.MayReport(From(Anna, id: 0), Me, false, Assistant));
        Assert.False(BubbleRules.MayReport(From(Me), Me, false, Assistant));
        Assert.False(BubbleRules.MayReport(From(Assistant), Me, false, Assistant));
    }

    /// <summary>
    /// The assistant's own path, which the member one refuses: a reply may be reported, and the two
    /// rules never both answer true for the same bubble — the member endpoint would answer
    /// `not_same_family` for the assistant, and the assistant endpoint answers `message_not_found`
    /// for a member (docs/protocol.md, "Reporting the assistant").
    /// </summary>
    [Fact]
    public void AnAssistantReplyMayBeReportedDownItsOwnPath()
    {
        Assert.True(BubbleRules.MayReportAssistant(From(Assistant), Me, false, Assistant));
        // Its own chat, where anybody who is not the reader is the assistant.
        Assert.True(BubbleRules.MayReportAssistant(From(Assistant), Me, assistantChat: true, assistantUserId: null));
        // Not until the server has numbered it.
        Assert.False(BubbleRules.MayReportAssistant(From(Assistant, id: 0), Me, false, Assistant));
        // A member's message is not this path's business, and neither is your own.
        Assert.False(BubbleRules.MayReportAssistant(From(Anna), Me, false, Assistant));
        Assert.False(BubbleRules.MayReportAssistant(From(Me), Me, assistantChat: true, Assistant));
        // Exclusive, both ways, for every bubble a chat can hold.
        foreach (var (message, chat) in new[]
        {
            (From(Assistant), false), (From(Anna), false), (From(Me), false),
            (From(Assistant), true), (From(Anna), true), (From(Me), true),
        })
        {
            Assert.False(
                BubbleRules.MayReport(message, Me, chat, Assistant)
                    && BubbleRules.MayReportAssistant(message, Me, chat, Assistant),
                "one bubble is never both a member report and an assistant report");
        }
    }
}
