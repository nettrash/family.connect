using FamilyConnect.Core;

namespace FamilyConnect.App.Logic.Tests;

public sealed class ReactionSearchTests
{
    [Theory]
    [InlineData("en-US", "litaf", "fire")]
    [InlineData("de-AT", "brennen", "Feuer")]
    [InlineData("es", "llama", "fuego")]
    [InlineData("fr", "fournaise", "feu")]
    [InlineData("ja", "燃える", "火")]
    [InlineData("ru", "костер", "огонь")]
    [InlineData("sr-Cyrl-RS", "плaмeн", "ватра")]
    [InlineData("sr-Latn-RS", "plamen", "vatra")]
    [InlineData("zh-CN", "燃烧", "火焰")]
    public void AllNineLanguagesFindKeywordsAndNameCatalogueCells(string locale, string keyword, string name)
    {
        var search = new ReactionSearch(locale);

        Assert.Contains(new ReactionSearchResult("🔥", name), search.Search(keyword));
        Assert.Equal(name, search.Describe("🔥").Name);
        foreach (var emoji in EmojiCatalog.Categories.SelectMany(category => category.Emoji))
        {
            Assert.NotEqual(emoji, search.Describe(emoji).Name);
        }
    }

    [Fact]
    public void EnglishExactNamesShortcodesAndPrefixesRankWithLocalLabels()
    {
        var russian = new ReactionSearch("ru");

        Assert.Equal(new ReactionSearchResult("🧊", "кубик льда"), russian.Search("  :ICE:  ")[0]);
        Assert.Equal("👍", russian.Search("LIKE")[0].Emoji);
        Assert.Equal("❤️", russian.Search("красное HEART")[0].Emoji);
        var prefix = russian.Search("ic").Select(result => result.Emoji).ToList();
        Assert.Contains("😀", prefix); // Matches the English keyword "nice".
        Assert.Contains("🧊", prefix);
        Assert.True(prefix.IndexOf("🧊") < prefix.IndexOf("😀"));
        Assert.Contains("⚽", new ReactionSearch("de").Search("  FUSSBALL ").Select(result => result.Emoji));
    }

    [Theory]
    [InlineData("❤", "❤️", "red heart")]
    [InlineData("✌", "✌️", "victory hand")]
    [InlineData("♥️", "♥️", "heart suit")]
    public void ExactEmojiReturnsCanonicalBytesWithoutChangingTheBaseCharacter(string query, string emoji, string name)
    {
        var search = new ReactionSearch("unsupported");

        Assert.Equal(new ReactionSearchResult(emoji, name), Assert.Single(search.Search($" {query} ")));
    }

    [Theory]
    [InlineData("")]
    [InlineData(" \u3000 ")]
    [InlineData("__::")]
    [InlineData("no-such-emoji-8372")]
    [InlineData("fire nonexistentkeyword")]
    [InlineData("👍🏽")]
    public void EmptyAndUnmatchedQueriesProduceNoResults(string query) =>
        Assert.Empty(new ReactionSearch("en").Search(query));

    [Fact]
    public void EqualRanksKeepCatalogueOrderAndResultsAreUnique()
    {
        var english = new ReactionSearch("en");

        var matches = english.Search("heart").Select(result => result.Emoji).ToList();
        Assert.Equal("❤️", matches[0]);
        Assert.Contains("🧡", matches);
        Assert.Contains("💛", matches);
        Assert.True(matches.IndexOf("🧡") < matches.IndexOf("💛"));
        Assert.Equal(matches.Count, matches.Distinct(StringComparer.Ordinal).Count());
        Assert.Equal(matches, english.Search(" HEART ").Select(result => result.Emoji));
    }
}
