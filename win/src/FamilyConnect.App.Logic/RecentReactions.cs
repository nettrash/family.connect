using System.Globalization;
using System.Text.Json;
using FamilyConnect.Core.Store;
using Microsoft.Data.Sqlite;

namespace FamilyConnect.App.Logic;

/// <summary>Device-local picker choices. The cache wipes them when the session ends.</summary>
public sealed class RecentReactions(Database cache, Uri server, Func<long> signedInAccount)
{
    public const int Limit = 24;

    public IReadOnlyList<string> Read(long accountId)
    {
        using var held = cache.Hold();
        return IsCurrent(accountId) ? Clean(ReadSaved(Key(accountId))) : [];
    }

    public void Remember(long accountId, string emoji)
    {
        var picked = ReactionSearch.CanonicalEmoji(emoji);
        if (picked is null)
        {
            return;
        }

        using var held = cache.Hold();
        if (!IsCurrent(accountId))
        {
            return;
        }
        var key = Key(accountId);
        var choices = Clean(ReadSaved(key).Prepend(picked));
        try
        {
            using var command = cache.Connection.CreateCommand();
            command.CommandText = """
                INSERT INTO meta (key, value) VALUES ($key, $value)
                ON CONFLICT(key) DO UPDATE SET value = excluded.value
                """;
            command.Parameters.AddWithValue("$key", key);
            command.Parameters.AddWithValue("$value", JsonSerializer.Serialize(choices));
            command.ExecuteNonQuery();
        }
        catch (SqliteException)
        {
            // A failed local history write must never prevent the reaction action.
        }
    }

    private bool IsCurrent(long accountId) => accountId > 0 && accountId == signedInAccount();

    private string Key(long accountId) =>
        $"reaction-recents:{server.AbsoluteUri}\n{accountId.ToString(CultureInfo.InvariantCulture)}";

    private IReadOnlyList<string?> ReadSaved(string key)
    {
        try
        {
            using var command = cache.Connection.CreateCommand();
            command.CommandText = "SELECT value FROM meta WHERE key = $key";
            command.Parameters.AddWithValue("$key", key);
            return command.ExecuteScalar() is string json
                ? JsonSerializer.Deserialize<string?[]>(json) ?? []
                : [];
        }
        catch (Exception error) when (error is JsonException or SqliteException)
        {
            return [];
        }
    }

    private static string[] Clean(IEnumerable<string?> choices) => choices.OfType<string>()
        .Select(ReactionSearch.CanonicalEmoji)
        .OfType<string>()
        .Distinct(StringComparer.Ordinal)
        .Take(Limit)
        .ToArray();
}
