using FamilyConnect.Core;

namespace FamilyConnect.Core.Tests.Text;

/// <summary>
/// Apple's placeholder grammar, which the shipped catalogue's values are written in — and the
/// trap the web port hit: a TRANSLATION may need the arguments in another order, and then it
/// writes them positionally.
/// </summary>
public class AppleFormatTests
{
    [Fact]
    public void PlaceholdersFillInOrder()
    {
        Assert.Equal("2 of 5 done", AppleFormat.Apply("%lld of %lld done", 2, 5));
        Assert.Equal("Anna mentioned you", AppleFormat.Apply("%@ mentioned you", "Anna"));
        Assert.Equal("2 going, 1 maybe", AppleFormat.Apply("%lld going, %lld maybe", 2, 1));
    }

    /// <summary>
    /// The German and Serbian sentences in the shipped catalogue are written this way.
    /// </summary>
    [Fact]
    public void ATranslationMayPutThemInAnyOrder()
    {
        Assert.Equal("5 von 2 erledigt", AppleFormat.Apply("%2$lld von %1$lld erledigt", 2, 5));
        Assert.Equal("Anna: hello", AppleFormat.Apply("%1$@: %2$@", "Anna", "hello"));
        // Mixed forms in one sentence: the unpositioned ones keep their own running order.
        Assert.Equal("b a b", AppleFormat.Apply("%2$@ %@ %2$@", "a", "b"));
    }

    [Fact]
    public void AStrayPerCentIsShownRatherThanThrown()
    {
        Assert.Equal("100% full", AppleFormat.Apply("100%% full"));
        // Not a placeholder this grammar knows: copied through, because a sentence with an odd
        // per cent in it is a sentence to show, not an exception to throw at somebody reading
        // their family's chat.
        Assert.Equal("50% off", AppleFormat.Apply("50% off"));
        Assert.Equal("%", AppleFormat.Apply("%"));
        Assert.Equal("% ", AppleFormat.Apply("% "));
    }

    [Fact]
    public void AMissingArgumentLeavesAGapAndNeverThrows()
    {
        Assert.Equal(" of  done", AppleFormat.Apply("%lld of %lld done"));
        Assert.Equal("2 of  done", AppleFormat.Apply("%lld of %lld done", 2));
        Assert.Equal("", AppleFormat.Apply("%99$@", "a"));
    }

    [Fact]
    public void TheEnglishCatalogueAnswersItsOwnKeys()
    {
        Assert.Equal("Add to Calendar", EnglishCatalog.Instance.Get("Add to Calendar"));
        Assert.Equal("2 of 5 done", EnglishCatalog.Instance.Format("%lld of %lld done", 2, 5));
    }
}
