using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

public sealed class ComposerMentionsTests
{
    private const long Me = 7;

    private static readonly MemberDto[] Roster =
    [
        new(7, "me", "Anja"),
        new(11, "anna", "Anna"),
        new(12, "annalee", "Anna Lee"),
        new(13, "bob", "Bob"),
        new(14, "ann", "Annika", HasLeft: true),
        new(15, "gone", "Anneliese", Deleted: true),
    ];

    private static bool BobBlocked(long id) => id == 13;

    /// <summary>Offered in roster order: never the reader, never the blocked, never a former or deleted member.</summary>
    [Fact]
    public void AHalfTypedAtOffersTheNamesItCouldMean()
    {
        Assert.Equal([11L, 12L], ComposerMentions.Offered("dinner @An", Roster, Me, BobBlocked, familyChat: true, editing: false).Select(member => member.Id));
        Assert.Equal([11L, 12L], ComposerMentions.Offered("@", Roster, Me, BobBlocked, true, false).Select(member => member.Id));
        Assert.Equal([13L], ComposerMentions.Offered("@b", Roster, Me, _ => false, true, false).Select(member => member.Id));
    }

    [Fact]
    public void NothingIsOfferedOutsideTheFamilyChatWhileEditingOrMidWord()
    {
        Assert.Empty(ComposerMentions.Offered("@An", Roster, Me, BobBlocked, familyChat: false, editing: false));
        Assert.Empty(ComposerMentions.Offered("@An", Roster, Me, BobBlocked, familyChat: true, editing: true));
        Assert.Empty(ComposerMentions.Offered("mail@An", Roster, Me, BobBlocked, true, false));
        Assert.Empty(ComposerMentions.Offered("no at", Roster, Me, BobBlocked, true, false));
    }

    /// <summary>A send names whom its text names — in the family chat alone, and nothing at all when it names nobody.</summary>
    [Fact]
    public void ASendCarriesTheNamesItsTextSays()
    {
        Assert.Equal([new MentionDto(12, "Anna Lee"), new MentionDto(13, "Bob")],
            ComposerMentions.ForSend("@Anna Lee and @Bob, 7?", Roster, familyChat: true)!);
        Assert.Null(ComposerMentions.ForSend("@Anna Lee", Roster, familyChat: false));
        Assert.Null(ComposerMentions.ForSend("@Annika and @Anneliese", Roster, familyChat: true));
        Assert.Null(ComposerMentions.ForSend("nobody here", Roster, familyChat: true));
    }

    [Fact]
    public void ANameOpensAChatOnlyWithSomebodyWhoCanBeMessaged()
    {
        Assert.True(ComposerMentions.OpensChat(11, Roster, Me, BobBlocked));
        Assert.False(ComposerMentions.OpensChat(Me, Roster, Me, BobBlocked));
        Assert.False(ComposerMentions.OpensChat(13, Roster, Me, BobBlocked));
        Assert.False(ComposerMentions.OpensChat(14, Roster, Me, BobBlocked));
        Assert.False(ComposerMentions.OpensChat(99, Roster, Me, BobBlocked));
    }

    private static readonly AssistantDto Helper = new(99, "Helper", "@ai", Processor: "Microsoft — Azure OpenAI");

    /// <summary>
    /// #106 (docs/protocol.md, "Mentioning a member", amended 2026-10-20): the assistant FIRST, above the members, under its
    /// display name — narrowed by that name and by "ai" — and a pick writes "@ai ", never "@Helper ".
    /// </summary>
    [Fact]
    public void TheAssistantIsOfferedFirstAndWritesTheAiToken()
    {
        var all = ComposerMentions.Choices("@", Roster, Me, BobBlocked, familyChat: true, editing: false, Helper);
        Assert.Equal(["Helper", "Anna", "Anna Lee"], all.Select(choice => choice.Name));
        Assert.Null(all[0].Member);
        Assert.Equal("ai", all[0].Writes);
        Assert.Equal("@ai", all[0].Handle);
        Assert.Equal("dinner @ai ", Mentions.Accept("dinner @He", all[0].Writes));
        Assert.Equal(11L, all[1].Member!.Id);
        Assert.Equal("Anna", all[1].Writes);
        Assert.Equal("@anna", all[1].Handle);

        Assert.Equal(["Helper"], ComposerMentions.Choices("@he", Roster, Me, BobBlocked, true, false, Helper).Select(choice => choice.Name));
        // Narrowed by "ai" as well as by its name: "@ai" finds an assistant called Robin, and "@ro" does too.
        var robin = Helper with { DisplayName = "Robin" };
        Assert.Equal(["Robin"], ComposerMentions.Choices("@ai", Roster, Me, BobBlocked, true, false, robin).Select(choice => choice.Name));
        Assert.Equal(["Robin"], ComposerMentions.Choices("@Ro", Roster, Me, BobBlocked, true, false, robin).Select(choice => choice.Name));
        Assert.Equal(["Robin", "Anna", "Anna Lee"], ComposerMentions.Choices("@a", Roster, Me, BobBlocked, true, false, robin).Select(choice => choice.Name));
        Assert.Equal(["Anna", "Anna Lee"], ComposerMentions.Choices("@An", Roster, Me, BobBlocked, true, false, Helper).Select(choice => choice.Name));
    }

    /// <summary>Only where @ai may be written: the family chat, not editing, on a server whose assistant names who answers.</summary>
    [Fact]
    public void TheAssistantIsOfferedOnlyWhereItCanBeAsked()
    {
        Assert.Empty(ComposerMentions.Choices("@", Roster, Me, BobBlocked, familyChat: false, editing: false, Helper));
        Assert.Empty(ComposerMentions.Choices("@", Roster, Me, BobBlocked, familyChat: true, editing: true, Helper));
        Assert.Empty(ComposerMentions.Choices("no at", Roster, Me, BobBlocked, true, false, Helper));
        Assert.All(ComposerMentions.Choices("@", Roster, Me, BobBlocked, true, false, Helper with { Processor = null }), choice => Assert.NotNull(choice.Member));
        Assert.All(ComposerMentions.Choices("@", Roster, Me, BobBlocked, true, false, null), choice => Assert.NotNull(choice.Member));
    }

    /// <summary>The assistant's own account, should the roster carry it, is never offered as a member.</summary>
    [Fact]
    public void TheAssistantsAccountIsNeverAMemberLine()
    {
        MemberDto[] roster = [.. Roster, new(99, "helper", "Helper")];
        var choices = ComposerMentions.Choices("@H", roster, Me, BobBlocked, true, false, Helper);
        Assert.Single(choices);
        Assert.Null(choices[0].Member);
    }
}
