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
}
