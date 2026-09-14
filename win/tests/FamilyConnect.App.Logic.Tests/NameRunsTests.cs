using FamilyConnect.App.Logic;
using FamilyConnect.Core.Protocol;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// The names a text says, cut out of it, and which of them is a door (docs/protocol.md, "Board" and "Mentioning a
/// member"): the same answer for a bubble and for an opened note.
/// </summary>
public sealed class NameRunsTests
{
    private const long Me = 7;

    private static readonly MemberDto[] Roster =
    [
        new(7, "me", "Anja"),
        new(11, "anna", "Anna"),
        new(13, "bob", "Bob"),
        new(14, "gran", "Gran", HasLeft: true),
        new(15, "gone", "Gus", Deleted: true),
    ];

    /// <summary>The reader's own name, a blocked member's, and a name whose member left or was deleted are names and not doors.</summary>
    [Fact]
    public void ANameIsADoorOnlyWhereThereIsSomebodyToOpenItWith()
    {
        MentionDto[] named = [new(11, "Anna"), new(13, "Bob"), new(14, "Gran"), new(15, "Gus"), new(7, "Anja")];

        var runs = ComposerMentions.Runs("@Anna and @Bob asked @Gran and @Gus, not @Anja", named, Roster, Me, id => id == 13);

        (string, long?, bool)[] expected =
        [
            ("@Anna", 11, true), (" and ", null, false), ("@Bob", 13, false), (" asked ", null, false),
            ("@Gran", 14, false), (" and ", null, false), ("@Gus", 15, false), (", not ", null, false), ("@Anja", 7, false),
        ];
        Assert.Equal(expected, runs.Select(run => (run.Text, run.UserId, run.Opens)));
    }

    [Fact]
    public void ATextThatNamesNobodyIsOnePlainStretch()
    {
        (string, long?, bool)[] plain = [("@Anna the kit", null, false)];
        Assert.Equal(plain, ComposerMentions.Runs("@Anna the kit", null, Roster, Me, _ => false).Select(run => (run.Text, run.UserId, run.Opens)));
        Assert.Equal(plain, ComposerMentions.Runs("@Anna the kit", [], Roster, Me, _ => false).Select(run => (run.Text, run.UserId, run.Opens)));

        // Named, but not said: the words stay words.
        (string, long?, bool)[] unsaid = [("the kit", null, false)];
        Assert.Equal(unsaid, ComposerMentions.Runs("the kit", [new(11, "Anna")], Roster, Me, _ => false).Select(run => (run.Text, run.UserId, run.Opens)));
    }
}
