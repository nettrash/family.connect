using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>The composer's assistant doors, and what an owner who left is told.</summary>
public sealed class AssistantButtonsTests
{
    // Every door needs a NAMED processor now: a client that cannot say who receives the words
    // offers no assistant (docs/protocol.md, "Consenting to the assistant").
    private static readonly AssistantDto Talks =
        new(99, "Assistant", "@ai", Processor: "Microsoft — Azure OpenAI");

    private static readonly AssistantDto Draws =
        new(99, "Assistant", "@ai", Images: true, Processor: "Microsoft — Azure OpenAI");

    /// <summary>The same server before it said who answers — an assistant this client will not use.</summary>
    private static readonly AssistantDto Unnamed = new(99, "Assistant", "@ai", Images: true);

    /// <summary>"Ask the assistant" belongs to the family chat of a server that has one — nowhere else.</summary>
    [Fact]
    public void TheMentionDoorIsTheFamilyChatsWhereThereIsAnAssistant()
    {
        Assert.True(AssistantButtons.Offered("family", Talks, editing: false).AskAssistant);
        Assert.False(AssistantButtons.Offered("family", null, editing: false).AskAssistant);
        Assert.False(AssistantButtons.Offered("direct", Talks, editing: false).AskAssistant);
        Assert.False(AssistantButtons.Offered("ai", Draws, editing: false).AskAssistant);
        Assert.False(AssistantButtons.Offered(null, Talks, editing: false).AskAssistant);
    }

    /// <summary>"Ask for a picture" belongs to the assistant's own chat, and only where it can draw.</summary>
    [Fact]
    public void ThePictureDoorIsTheAssistantsChatWhereItDraws()
    {
        Assert.True(AssistantButtons.Offered("ai", Draws, editing: false).AskPicture);
        Assert.False(AssistantButtons.Offered("ai", Talks, editing: false).AskPicture);
        Assert.False(AssistantButtons.Offered("ai", null, editing: false).AskPicture);
        Assert.False(AssistantButtons.Offered("family", Draws, editing: false).AskPicture);
    }

    /// <summary>
    /// And neither door on a server that named nobody: there is no honest way to ask, so the
    /// affordances that would send something are not offered (docs/protocol.md, "Consenting to
    /// the assistant").
    /// </summary>
    [Fact]
    public void NeitherDoorWhereTheServerNamedNoProcessor()
    {
        Assert.Equal((false, false), AssistantButtons.Offered("family", Unnamed, editing: false));
        Assert.Equal((false, false), AssistantButtons.Offered("ai", Unnamed, editing: false));
    }

    [Fact]
    public void NeitherDoorWhileAMessageIsEdited()
    {
        Assert.Equal((false, false), AssistantButtons.Offered("family", Draws, editing: true));
        Assert.Equal((false, false), AssistantButtons.Offered("ai", Draws, editing: true));
    }

    [Fact]
    public void AnOwnerWhoLeftIsToldWhoHasTheFamilyNow() =>
        Assert.Equal(("Ownership passed on", "Anna is now the owner of the family."), SettingsText.OwnershipPassed("Anna", EnglishCatalog.Instance));
}
