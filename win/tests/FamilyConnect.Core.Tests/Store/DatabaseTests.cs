using FamilyConnect.Core.Store;

namespace FamilyConnect.Core.Tests.Store;

/// <summary>
/// The cache's schema and how it moves forward.
/// </summary>
/// <remarks>
/// The test that matters here is <see cref="AMigratedSchemaIsTheSameAsAFreshOne"/>. Android's
/// board shipped a column with no migration once, and every upgraded install would have crashed
/// on the next launch (issue #70): the only thing that catches it is comparing a database that
/// walked every step against one created from scratch. This suite does that per table and per
/// column, so a step added without a column — or a column added without a step — fails here.
/// </remarks>
public class DatabaseTests : IDisposable
{
    private readonly string directory =
        Path.Combine(Path.GetTempPath(), "fc-cache-" + Guid.NewGuid().ToString("N"));

    private string Path_(string name) => Path.Combine(directory, name);

    public DatabaseTests() => Directory.CreateDirectory(directory);

    public void Dispose()
    {
        try
        {
            Directory.Delete(directory, recursive: true);
        }
        catch (IOException)
        {
            // A file the OS is still holding is not a test failure.
        }
    }

    [Fact]
    public void AFreshCacheCarriesThisBuildsSchemaNumber()
    {
        using var database = Database.Open(Path_("fresh.db"));
        Assert.Equal(Database.SchemaVersion, database.UserVersion);
        Assert.Equal(1, Database.SchemaVersion);
        Assert.Equal(
            ["blocked", "chats", "members", "messages", "meta", "notes", "outbox"],
            database.Tables());
    }

    [Fact]
    public void OpeningTwiceMigratesNothingAndKeepsWhatIsThere()
    {
        var path = Path_("twice.db");
        using (var first = Database.Open(path))
        {
            new OutboxStore(first).Queue(new OutboxRow("k", 42, "Dinner at 7?"));
        }
        using var again = Database.Open(path);
        Assert.Equal(Database.SchemaVersion, again.UserVersion);
        // The outbox is the one table holding something the server has never seen: it has to
        // survive the app closing, or an interrupted send is lost rather than finished.
        Assert.Equal("Dinner at 7?", Assert.Single(new OutboxStore(again).All()).Body);
    }

    /// <summary>
    /// THE ONE THAT CATCHES A FORGOTTEN MIGRATION. A cache that walked every step must have the
    /// same tables and the same columns as one created from scratch.
    /// </summary>
    [Fact]
    public void AMigratedSchemaIsTheSameAsAFreshOne()
    {
        // Stepped: start at version 0 with no tables, then let Migrate() walk.
        var steppedPath = Path_("stepped.db");
        using (var blank = new Microsoft.Data.Sqlite.SqliteConnection($"Data Source={steppedPath}"))
        {
            blank.Open();
            using var stamp = blank.CreateCommand();
            stamp.CommandText = "PRAGMA user_version = 0";
            stamp.ExecuteNonQuery();
        }
        Microsoft.Data.Sqlite.SqliteConnection.ClearAllPools();
        using var stepped = Database.Open(steppedPath);
        using var fresh = Database.Open(Path_("compare.db"));

        Assert.Equal(fresh.UserVersion, stepped.UserVersion);
        Assert.Equal(fresh.Tables(), stepped.Tables());
        foreach (var table in fresh.Tables())
        {
            Assert.Equal(fresh.Columns(table), stepped.Columns(table));
        }
    }

    /// <summary>
    /// A file from a NEWER build is refused rather than used. The alternative is this build
    /// writing rows a newer schema will misread — and nothing here may delete a family's outbox to
    /// make itself comfortable.
    /// </summary>
    [Fact]
    public void ACacheFromANewerBuildIsRefused()
    {
        var path = Path_("newer.db");
        using (var ahead = new Microsoft.Data.Sqlite.SqliteConnection($"Data Source={path}"))
        {
            ahead.Open();
            using var stamp = ahead.CreateCommand();
            stamp.CommandText = $"PRAGMA user_version = {Database.SchemaVersion + 7}";
            stamp.ExecuteNonQuery();
        }
        Microsoft.Data.Sqlite.SqliteConnection.ClearAllPools();
        var refused = Assert.Throws<InvalidOperationException>(() => Database.Open(path));
        Assert.Contains("newer version", refused.Message, StringComparison.Ordinal);
    }

    [Fact]
    public void ForeignKeysAreOnBecauseTheSchemaLeansOnThem()
    {
        // Off by default in SQLite, per connection — a cascade that silently did nothing would
        // leave a chat's messages behind when the chat went.
        using var database = Database.OpenInMemory();
        using var command = database.Connection.CreateCommand();
        command.CommandText = "PRAGMA foreign_keys";
        Assert.Equal(1L, Convert.ToInt64(command.ExecuteScalar()));
    }

    [Fact]
    public void EveryMigrationIsAppendOnlyAndNumbered()
    {
        // The index is the version it upgrades FROM, so the count IS the schema number. A step
        // that has shipped is never edited: an installed file has already run it.
        Assert.Equal(Database.SchemaVersion, Migrations.All.Count);
        Assert.All(Migrations.All, step => Assert.NotEmpty(step));
    }
}
