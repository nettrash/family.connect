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
}
