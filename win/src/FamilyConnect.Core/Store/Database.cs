using Microsoft.Data.Sqlite;

namespace FamilyConnect.Core.Store;

/// <summary>
/// The local cache: one SQLite file, opened once and migrated forward by number.
/// </summary>
/// <remarks>
/// <para>
/// WHY A CACHE AT ALL. The wire is authoritative and this file is a copy — every read replaces
/// what it holds, and nothing here is a second source of truth. What it buys is a window that
/// opens with the family's chat already in it rather than a spinner, and an outbox that survives
/// the app being closed mid-send.
/// </para>
/// <para>
/// MIGRATIONS ARE NUMBERED AND THERE IS NO DESTRUCTIVE FALLBACK. Android's board shipped a column
/// without a migration once and every upgraded install would have crashed on the next launch
/// (issue #70); the only thing that catches a forgotten column is comparing a migrated schema
/// against a fresh one, which <c>DatabaseTests</c> does. `user_version` is the schema's number,
/// which is what SQLite gives us for free and what Room uses too.
/// </para>
/// </remarks>
public sealed class Database : IDisposable
{
    private readonly SqliteConnection connection;

    private Database(SqliteConnection connection) => this.connection = connection;

    /// <summary>The one connection. SQLite is fine with one writer, and this client has one.</summary>
    public SqliteConnection Connection => connection;

    private readonly object serial = new();

    /// <summary>
    /// Hold the cache for ONE WHOLE OPERATION: every store method takes this first, and nothing else
    /// may use <see cref="Connection"/> while another thread holds it.
    /// </summary>
    /// <remarks>
    /// <para>
    /// <b>ONE CONNECTION, MANY THREADS.</b> In the app the socket applies frames on its own thread,
    /// the resync applies pages on the thread pool, and the window reads on the UI thread — all
    /// through this one connection, which is not safe to share without a rule. Without one, a read
    /// that lands while another thread's transaction is open throws ("Execute requires the command
    /// to have a transaction object…"), and worse can follow; CI caught exactly that on both runners.
    /// </para>
    /// <para>
    /// <b>PER OPERATION, NOT PER COMMAND</b>, because a transaction spans several commands and a
    /// read slipped between two of them is the failure. The lock is re-entrant, so an operation that
    /// calls another (the chat list asking for each chat's newest message) simply holds it deeper.
    /// No store method awaits or raises an event while holding it, which is what keeps one lock from
    /// ever deadlocking.
    /// </para>
    /// </remarks>
    public Held Hold()
    {
        Monitor.Enter(serial);
        return new Held(serial);
    }

    /// <summary>The cache, held. Disposing it lets the next operation in.</summary>
    public readonly struct Held : IDisposable
    {
        private readonly object serial;

        internal Held(object serial) => this.serial = serial;

        public void Dispose() => Monitor.Exit(serial);
    }

    /// <summary>The schema this build expects.</summary>
    public static int SchemaVersion => Migrations.All.Count;

    /// <summary>Open (or create) the cache at <paramref name="path"/> and migrate it forward.</summary>
    public static Database Open(string path)
    {
        var builder = new SqliteConnectionStringBuilder
        {
            DataSource = path,
            // The file is created on first run; a missing directory is the caller's problem.
            Mode = SqliteOpenMode.ReadWriteCreate,
            // One connection, one cache: this client has a single writer and no reason to share.
            Cache = SqliteCacheMode.Private,
            Pooling = false,
        };
        var connection = new SqliteConnection(builder.ToString());
        connection.Open();
        Configure(connection);
        var database = new Database(connection);
        database.Migrate();
        return database;
    }

    /// <summary>A cache that lives only as long as the process — for tests.</summary>
    public static Database OpenInMemory()
    {
        var connection = new SqliteConnection("Data Source=:memory:");
        connection.Open();
        Configure(connection);
        var database = new Database(connection);
        database.Migrate();
        return database;
    }

    private static void Configure(SqliteConnection connection)
    {
        // FOREIGN KEYS ARE OFF BY DEFAULT IN SQLITE, per connection, and this schema leans on
        // them: a chat's messages and a note's rows go with it, and a cascade that silently did
        // nothing would leave rows nobody can reach.
        Execute(connection, "PRAGMA foreign_keys = ON");
        // A crash mid-write must not cost the whole cache. WAL also lets a read proceed while the
        // sync writes, which is most of what this file does.
        Execute(connection, "PRAGMA journal_mode = WAL");
        Execute(connection, "PRAGMA synchronous = NORMAL");
        Execute(connection, "PRAGMA busy_timeout = 5000");
    }

    /// <summary>
    /// Run every migration this build knows and the file has not had, in order, each in its own
    /// transaction with the version bumped inside it — so an interrupted upgrade is either done or
    /// not done, never half.
    /// </summary>
    public void Migrate()
    {
        using var serialised = Hold();
        var from = UserVersion;
        if (from > Migrations.All.Count)
        {
            // A file written by a NEWER build. Refusing is the honest answer: the alternative is
            // an older client writing rows a newer schema will misread, and nothing here may
            // delete a family's outbox to make itself comfortable.
            throw new InvalidOperationException(
                $"this cache was written by a newer version (schema {from}, this build knows {Migrations.All.Count})");
        }
        for (var version = from; version < Migrations.All.Count; version++)
        {
            using var transaction = connection.BeginTransaction();
            foreach (var statement in Migrations.All[version])
            {
                using var command = connection.CreateCommand();
                command.Transaction = transaction;
                command.CommandText = statement;
                command.ExecuteNonQuery();
            }
            using (var stamp = connection.CreateCommand())
            {
                stamp.Transaction = transaction;
                // PRAGMA takes no parameter binding; the value is an int this code computed.
                stamp.CommandText = $"PRAGMA user_version = {version + 1}";
                stamp.ExecuteNonQuery();
            }
            transaction.Commit();
        }
    }

    /// <summary>The schema number the file carries.</summary>
    public int UserVersion
    {
        get
        {
            using var serialised = Hold();
            using var command = connection.CreateCommand();
            command.CommandText = "PRAGMA user_version";
            return Convert.ToInt32(command.ExecuteScalar() ?? 0);
        }
    }

    /// <summary>Every table in the file, in name order — what a schema test compares.</summary>
    public IReadOnlyList<string> Tables()
    {
        using var serialised = Hold();
        var names = new List<string>();
        using var command = connection.CreateCommand();
        command.CommandText =
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name";
        using var reader = command.ExecuteReader();
        while (reader.Read())
        {
            names.Add(reader.GetString(0));
        }
        return names;
    }

    /// <summary>
    /// One table's columns as <c>name TYPE [NOT NULL]</c>, in declaration order — enough to catch
    /// the failure that matters: a column a fresh schema has and a migrated one does not.
    /// </summary>
    /// <summary>
    /// Every row of every table, gone — the schema stays. What a sign-out, a session that
    /// expired and being removed from a family all do.
    /// </summary>
    /// <remarks>
    /// THE OUTBOX GOES WITH IT, and deliberately: a queued message belongs to the account that
    /// wrote it, and a send that landed in the next person's family would be the worst bug this
    /// app could have. The tables are read from the file rather than listed here, so a table
    /// added later is wiped without anybody remembering to come back — which is exactly the
    /// mistake a list would eventually make.
    /// </remarks>
    public void WipeAll()
    {
        using var serialised = Hold();
        using var transaction = connection.BeginTransaction();
        foreach (var table in Tables())
        {
            using var command = connection.CreateCommand();
            command.Transaction = transaction;
            command.CommandText = $"DELETE FROM \"{table}\"";
            command.ExecuteNonQuery();
        }
        transaction.Commit();
    }

    public IReadOnlyList<string> Columns(string table)
    {
        using var serialised = Hold();
        var columns = new List<string>();
        using var command = connection.CreateCommand();
        // `PRAGMA table_info` takes no parameters either; the name is checked against the file's
        // own table list first, so nothing user-supplied reaches the statement.
        if (!Tables().Contains(table))
        {
            return columns;
        }
        command.CommandText = $"PRAGMA table_info({table})";
        using var reader = command.ExecuteReader();
        while (reader.Read())
        {
            var name = reader.GetString(1);
            var type = reader.GetString(2);
            var notNull = reader.GetInt32(3) == 1;
            columns.Add(notNull ? $"{name} {type} NOT NULL" : $"{name} {type}");
        }
        return columns;
    }

    private static void Execute(SqliteConnection connection, string sql)
    {
        using var command = connection.CreateCommand();
        command.CommandText = sql;
        command.ExecuteNonQuery();
    }

    public void Dispose()
    {
        // Waits for an operation in flight rather than pulling the connection out from under it.
        lock (serial)
        {
            connection.Dispose();
        }
        // The pool is off, so the file is actually released — which a test that opens the same
        // path twice depends on.
        SqliteConnection.ClearAllPools();
    }
}
