using System.Globalization;
using System.Text.Json;
using Microsoft.Data.Sqlite;

namespace FamilyConnect.Core.Store;

/// <summary>Device-local picker choices, scoped to the server and account and wiped at sign-out.</summary>
public sealed class RecentReactionStore(Database cache, Uri server)
{
    public IReadOnlyList<string?> Read(long accountId)
    {
        using var held = cache.Hold();
        return ReadSaved(Key(accountId));
    }

    /// <summary>Apply the picker's ordering and catalogue rules in the same cache operation as the read.</summary>
    public void Update(long accountId, Func<IReadOnlyList<string?>, string[]> update)
    {
        using var held = cache.Hold();
        var key = Key(accountId);
        var choices = update(ReadSaved(key));
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
}
