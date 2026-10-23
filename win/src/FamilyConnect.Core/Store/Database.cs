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

    /// <summary>
    /// How many times the roster or the block list may have changed — the members and blocked tables. Read and bumped under
    /// <see cref="Hold"/> only.
    /// </summary>
    /// <remarks>
    /// <para>
    /// <b>WHAT LETS A STORE KEEP THEM IN MEMORY.</b> A conversation asks "is this sender blocked?" and "who is this?" for
    /// every bubble it draws, and each of those was a query under the one lock this file has — hundreds of them per redraw,
    /// on the window's thread, while the socket's thread waited to write. They are tiny tables that change a few times a
    /// day, so <see cref="ChatStore"/> keeps them in memory and asks this number whether what it keeps is still the table.
    /// </para>
    /// <para>
    /// <b>ON THE DATABASE, NOT THE STORE</b>, because every writer has to bump it and more than one store can sit on one
    /// file (the tests make several): a counter in each store would let one store's write leave another's copy stale. And
    /// <see cref="WipeAll"/> bumps it too, which no store sees happen.
    /// </para>
    /// </remarks>
    internal long RosterGeneration { get; private set; }

    /// <summary>The members or the blocked table is about to change: every copy kept of them is stale. Under <see cref="Hold"/>.</summary>
    internal void RosterChanged() => RosterGeneration++;

    /// <summary>
    /// The same for the chats table's rows — what <see cref="ChatStore.Kind"/> keeps. Bumped by every write of a chat's
    /// list row and by <see cref="WipeAll"/>; under <see cref="Hold"/> only.
    /// </summary>
    internal long ChatsGeneration { get; private set; }

    /// <summary>A chat's list row is about to be written. Under <see cref="Hold"/>.</summary>
    internal void ChatsChanged() => ChatsGeneration++;

    /// <summary>
    /// How many times a chat has been TOUCHED outside a list read — upserted by <c>chat_updated</c> or a group endpoint's
    /// answer, or removed — and when each was last (see <see cref="ChatStore.ListMark"/>). On the database, like the
    /// generations, so a second store on the same file sees them; under <see cref="Hold"/> only.
    /// </summary>
    internal long ChatTouches { get; private set; }

    private readonly Dictionary<long, long> touchedAt = [];

    /// <summary>This chat changed outside a list read, as of now. Under <see cref="Hold"/>.</summary>
    internal void Touched(long chatId) => touchedAt[chatId] = ++ChatTouches;

    /// <summary>The chats touched after <paramref name="mark"/>. Under <see cref="Hold"/>.</summary>
    internal HashSet<long> TouchedSince(long mark) =>
        [.. touchedAt.Where(touch => touch.Value > mark).Select(touch => touch.Key)];

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
        // Whatever a store keeps in memory of these tables goes with them.
        RosterChanged();
        ChatsChanged();
        touchedAt.Clear();
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

    /// <summary>
    /// ONE FAMILY's rows, gone — and nothing else (docs/protocol.md, "Several families"): the account left that family, was
    /// removed from it, or the family went. Its board, its pack, its roster and every cursor and mark kept for it; its chats
    /// with their messages and queued sends — except a direct chat still listed in another family, which only stops naming
    /// this one (a direct chat belongs to the pair, and stays while they share another family).
    /// </summary>
    /// <remarks>
    /// <see cref="WipeAll"/> is still what a sign-out, an expired session and leaving the LAST family do. This is the
    /// narrower thing leaving one of several needs: the people stay on <c>members</c>, because a name still has to resolve
    /// for an old message in a chat that stays.
    /// </remarks>
    /// <returns>The chats that went, so a window showing one can close it.</returns>
    public IReadOnlyList<long> DropFamily(long familyId)
    {
        using var serialised = Hold();
        RosterChanged();
        ChatsChanged();
        var gone = new List<long>();
        using var transaction = connection.BeginTransaction();
        var listed = new List<(long Chat, long[] Families)>();
        using (var read = connection.CreateCommand())
        {
            read.Transaction = transaction;
            read.CommandText = "SELECT chat_id, family_ids_json FROM chats WHERE family_ids_json IS NOT NULL";
            using var reader = read.ExecuteReader();
            while (reader.Read())
            {
                if (Protocol.Wire.Decode<long[]>(reader.GetString(1)) is { } families && families.Contains(familyId))
                {
                    listed.Add((reader.GetInt64(0), families));
                }
            }
        }
        foreach (var (chat, families) in listed)
        {
            Touched(chat);
            var rest = families.Where(family => family != familyId).ToArray();
            if (rest.Length > 0)
            {
                using var shrink = connection.CreateCommand();
                shrink.Transaction = transaction;
                shrink.CommandText = "UPDATE chats SET family_ids_json = $families WHERE chat_id = $id";
                shrink.Parameters.AddWithValue("$families", Protocol.Wire.Encode(rest));
                shrink.Parameters.AddWithValue("$id", chat);
                shrink.ExecuteNonQuery();
                continue;
            }
            gone.Add(chat);
            foreach (var statement in new[]
                     {
                         "DELETE FROM outbox WHERE chat_id = $id",
                         "DELETE FROM messages WHERE chat_id = $id",
                         "DELETE FROM chats WHERE chat_id = $id",
                     })
            {
                using var command = connection.CreateCommand();
                command.Transaction = transaction;
                command.CommandText = statement;
                command.Parameters.AddWithValue("$id", chat);
                command.ExecuteNonQuery();
            }
        }
        foreach (var statement in new[]
                 {
                     "DELETE FROM pack_recents WHERE item_id IN (SELECT item_id FROM pack_items WHERE family_id = $family)",
                     "DELETE FROM pack_items WHERE family_id = $family",
                     "DELETE FROM notes WHERE family_id = $family",
                     "DELETE FROM family_members WHERE family_id = $family",
                     // Its cursors, its marks and its ceilings: every key spelled for it (FamilyKeys).
                     "DELETE FROM meta WHERE key LIKE '%@' || $family",
                 })
        {
            using var command = connection.CreateCommand();
            command.Transaction = transaction;
            command.CommandText = statement;
            command.Parameters.AddWithValue("$family", familyId.ToString(System.Globalization.CultureInfo.InvariantCulture));
            command.ExecuteNonQuery();
        }
        transaction.Commit();
        return gone;
    }

    /// <summary>
    /// Every family this cache holds anything per family for — a roster, a note, a pack item — so a family the account left
    /// while this app was not running is found and dropped once <c>GET /me</c> no longer lists it.
    /// </summary>
    public IReadOnlyList<long> HeldFamilies()
    {
        using var serialised = Hold();
        var ids = new List<long>();
        using var command = connection.CreateCommand();
        command.CommandText =
            """
            SELECT family_id FROM family_members
            UNION SELECT family_id FROM notes WHERE family_id IS NOT NULL
            UNION SELECT family_id FROM pack_items WHERE family_id IS NOT NULL
            ORDER BY 1
            """;
        using var reader = command.ExecuteReader();
        while (reader.Read())
        {
            ids.Add(reader.GetInt64(0));
        }
        return ids;
    }

    /// <summary>
    /// What a cache from before several families held, made the HOME family's — the one family such a build ever saw — once
    /// <c>GET /me</c> names it: the notes and pack items migration 11 left without a family, the roster as that family's, and
    /// the board's and the pack's cursors, marks and ceilings under that family's keys. Idempotent: a second call finds
    /// nothing unadopted.
    /// </summary>
    public void AdoptLegacy(long homeFamilyId)
    {
        using var serialised = Hold();
        RosterChanged();
        using var transaction = connection.BeginTransaction();
        foreach (var statement in new[]
                 {
                     "UPDATE notes SET family_id = $family WHERE family_id IS NULL",
                     "UPDATE pack_items SET family_id = $family WHERE family_id IS NULL",
                     // Only while no roster is kept per family at all: afterwards `members` is everybody, of every family.
                     """
                     INSERT INTO family_members (family_id, user_id, role, has_left)
                     SELECT $family, user_id, role, MAX(has_left, deleted) FROM members
                      WHERE NOT EXISTS (SELECT 1 FROM family_members)
                     """,
                 })
        {
            using var command = connection.CreateCommand();
            command.Transaction = transaction;
            command.CommandText = statement;
            command.Parameters.AddWithValue("$family", homeFamilyId);
            command.ExecuteNonQuery();
        }
        foreach (var key in BoardStore.MetaKeys.Concat(PackStore.MetaKeys))
        {
            using var move = connection.CreateCommand();
            move.Transaction = transaction;
            // The family's own key wins if it is already there; the bare one goes either way.
            move.CommandText =
                """
                INSERT OR IGNORE INTO meta (key, value) SELECT $scoped, value FROM meta WHERE key = $bare;
                DELETE FROM meta WHERE key = $bare;
                """;
            move.Parameters.AddWithValue("$scoped", FamilyKeys.Of(key, homeFamilyId));
            move.Parameters.AddWithValue("$bare", key);
            move.ExecuteNonQuery();
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
