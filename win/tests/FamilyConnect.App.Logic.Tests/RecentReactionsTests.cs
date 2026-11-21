using FamilyConnect.Core;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic.Tests;

public sealed class RecentReactionsTests
{
    private static readonly Uri Server = new("https://chat.example.test/");

    [Fact]
    public void RecentsAreIsolatedByAccountAndServerAndRejectAStalePicker()
    {
        using var cache = Database.OpenInMemory();
        long current = 7;
        var first = new RecentReactions(cache, Server, () => current);
        var second = new RecentReactions(cache, new Uri("https://other.example.test/"), () => current);
        first.Remember(7, "🔥");
        Assert.Empty(second.Read(7));
        second.Remember(7, "👍");
        Assert.Equal(["🔥"], first.Read(7));
        Assert.Equal(["👍"], second.Read(7));

        current = 9;
        Assert.Empty(first.Read(9));
        Assert.Empty(first.Read(7));
        first.Remember(7, "👍");
        first.Remember(9, "😂");
        Assert.Equal(["😂"], first.Read(9));

        current = 0;
        first.Remember(0, "😂");
        Assert.Empty(first.Read(0));
        current = 7;
        Assert.Equal(["🔥"], first.Read(7));
    }

    [Fact]
    public void RecentsSurviveReopeningTheCacheAndAreWipedAtSessionEnd()
    {
        var path = Path.Combine(Path.GetTempPath(), $"familyconnect-recents-{Guid.NewGuid()}.db");
        try
        {
            using (var cache = Database.Open(path))
            {
                new RecentReactions(cache, Server, () => 7).Remember(7, "🔥");
            }
            using var reopened = Database.Open(path);
            var recent = new RecentReactions(reopened, Server, () => 7);
            Assert.Equal(["🔥"], recent.Read(7));

            reopened.WipeAll();

            Assert.Empty(recent.Read(7));
        }
        finally
        {
            File.Delete(path);
        }
    }

    [Fact]
    public void RecentChoicesMoveToFrontCanonicaliseAndStayBounded()
    {
        using var cache = Database.OpenInMemory();
        var recent = new RecentReactions(cache, Server, () => 7);
        var emojis = EmojiCatalog.Categories[0].Emoji.Take(25).ToArray();
        foreach (var emoji in emojis)
        {
            recent.Remember(7, emoji);
        }

        Assert.Equal(emojis.Reverse().Take(24), recent.Read(7));
        recent.Remember(7, "❤");
        recent.Remember(7, "not an emoji");
        recent.Remember(7, "❤️");

        Assert.Equal(new[] { "❤️" }.Concat(emojis.Reverse().Take(23)), recent.Read(7));
    }

    [Theory]
    [InlineData("{", new string[0])]
    [InlineData("[\"😀\",null,\"❤\",\"😀\",\"invalid\"]", new[] { "😀", "❤️" })]
    public void RecentsHandleInvalidAndLegacyStoredData(string json, string[] expected)
    {
        using var cache = Database.OpenInMemory();
        var recent = new RecentReactions(cache, Server, () => 7);
        recent.Remember(7, "🔥");
        using var command = cache.Connection.CreateCommand();
        command.CommandText = "UPDATE meta SET value = $value";
        command.Parameters.AddWithValue("$value", json);
        Assert.Equal(1, command.ExecuteNonQuery());

        Assert.Equal(expected, recent.Read(7));
        recent.Remember(7, "🔥");
        Assert.Equal(expected.Prepend("🔥"), recent.Read(7));
    }
}
