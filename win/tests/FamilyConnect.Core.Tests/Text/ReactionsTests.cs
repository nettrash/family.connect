using FamilyConnect.Core.Protocol;

namespace FamilyConnect.Core.Tests.Text;

/// <summary>
/// What a reaction chip says when the pointer rests on it (issue #134): the names "See who reacted" gives that emoji,
/// and nothing at all when it may name nobody.
/// </summary>
public class ReactionsTests
{
    private const long Me = 7;

    private static readonly string Heart = char.ConvertFromUtf32(0x2764) + char.ConvertFromUtf32(0xFE0F);
    private static readonly string ThumbsUp = char.ConvertFromUtf32(0x1F44D);
    private static readonly string Laughing = char.ConvertFromUtf32(0x1F602);

    private static readonly Dictionary<long, string> Roster = new() { [9] = "Anna", [11] = "Ben", [12] = "Kim" };

    private static IReadOnlyList<ReactionDetail> Details(ReactionDto[] reactions, params long[] blocked) =>
        Reactions.Details(reactions, id => Roster.GetValueOrDefault(id), Me, blocked.ToHashSet());

    [Fact]
    public void AChipsTooltipNamesWhoChoseItAsSeeWhoReactedDoes()
    {
        var details = Details([new(9, Heart), new(Me, Heart), new(11, ThumbsUp)]);

        // The flyout's names, "You" first whenever the reader reacted, joined as the flyout joins them.
        Assert.Equal("You, Anna", Reactions.WhoReacted(details, Heart));
        Assert.Equal("Ben", Reactions.WhoReacted(details, ThumbsUp));
        Assert.Equal(string.Join(", ", details[0].Names), Reactions.WhoReacted(details, Heart));
    }

    [Fact]
    public void SomebodyWhoLeftIsSomeone()
    {
        var details = Details([new(9, Heart), new(40, Heart)]);
        Assert.Equal("Anna, Someone", Reactions.WhoReacted(details, Heart));
    }

    [Fact]
    public void AChipWithNobodyItMayNameHasNoTooltipAtAll()
    {
        // Both hearts are from blocked members: the chip still counts two, and its tooltip names nobody — not an
        // empty tip.
        var details = Details([new(9, Heart), new(11, Heart), new(12, ThumbsUp)], 9, 11);
        Assert.Null(Reactions.WhoReacted(details, Heart));
        Assert.Equal("Kim", Reactions.WhoReacted(details, ThumbsUp));
        Assert.Null(Reactions.WhoReacted(details, Laughing));
    }

    [Fact]
    public void EmojiAreMatchedOrdinallyAsTheChipsAre()
    {
        // The bare heart is a different string from the quick set's, and a different chip.
        var bare = char.ConvertFromUtf32(0x2764);
        var details = Details([new(9, Heart), new(11, bare)]);
        Assert.Equal("Anna", Reactions.WhoReacted(details, Heart));
        Assert.Equal("Ben", Reactions.WhoReacted(details, bare));
    }
}
