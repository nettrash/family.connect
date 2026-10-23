using System.Globalization;
using FamilyConnect.Core.Protocol;
using Microsoft.Data.Sqlite;

namespace FamilyConnect.Core.Store;

/// <summary>
/// The pack's two ceilings, as <c>GET /families/mine</c> last gave them. Both or neither: a server
/// that has packs always sends both, and one that predates them sends neither.
/// </summary>
public readonly record struct PackLimits(int MaxItems, long MaxItemBytes);

/// <summary>
/// The family's sticker pack, as this device holds it (docs/protocol.md, "Sticker pack").
/// </summary>
/// <remarks>
/// <para>
/// <b>THE BOARD'S MACHINERY, UNCHANGED</b> — the protocol says so in as many words, and this is
/// <see cref="BoardStore"/> one table over. A FULL READ REPLACES what is held, except an item held
/// above the read's own mark, which arrived after the read was taken; an older full read landing
/// second is ignored outright. A REMOVAL IS REMEMBERED (<c>pack_gone</c>), so an older copy of an
/// item the family took out cannot put it back — whether the removal was seen as a tombstone, as
/// a full read that left the item out, or as this device's own <c>DELETE</c>. And every write is
/// guarded by <c>pack_seq</c>: an item, live or a tombstone, is applied only when the incoming
/// seq is greater than the one held.
/// </para>
/// <para>
/// <b>THE CURSOR MOVES IN THREE WAYS AND NO OTHERS</b>: a full read sets it to its mark, a
/// catch-up page to the highest seq on the page, and a <c>pack_item</c> frame to its own — the
/// frame ONLY ONCE THIS CONNECTION HAS CAUGHT UP. A frame that moved it sooner would step the
/// feed past every change made while the socket was down: the device would hold the newest
/// sticker and never learn of the three added, or the one removed, before it. The answer to this
/// device's own <c>POST</c> is evidence about one item and moves nothing.
/// </para>
/// <para>
/// <b>"STICKER" HERE IS THE CHAT ONE.</b> The board's cards are called stickers in this code
/// (<c>App.Logic.Sticker</c>); these tables and this class say <c>pack</c>, as the wire does, so
/// that nobody has to read a name twice to know which is meant.
/// </para>
/// <para>
/// <b>ONE PACK PER FAMILY</b> (docs/protocol.md, "Several families"): its own items, seqs, cursor, ceilings and catch-up. A
/// store reads and writes the pack of the family shown, and <see cref="In"/> is the same store held to one family; the
/// connection's count and which families have caught up on it are SHARED between them, because they are one socket's.
/// </para>
/// </remarks>
public sealed class PackStore
{
    private const string CursorSeq = "pack.cursor.pack_seq";

    /// <summary>The <c>max_pack_seq</c> of the newest FULL read applied; an older one landing second changes nothing.</summary>
    private const string FullMark = "pack.full.pack_seq";

    private const string LimitItems = "pack.limit.items";
    private const string LimitBytes = "pack.limit.item_bytes";

    /// <summary>Every key this store keeps in <c>meta</c>, before a family's suffix — what <see cref="Database.AdoptLegacy"/> moves.</summary>
    internal static readonly string[] MetaKeys = [CursorSeq, FullMark, LimitItems, LimitBytes];

    private readonly Database database;
    private readonly Func<long?> family;
    private readonly Link link;

    /// <summary>A pack store over <paramref name="database"/>, for the family <paramref name="current"/> says is shown.</summary>
    public PackStore(Database database, Func<long?>? current = null)
        : this(database, current ?? (() => null), new Link())
    {
    }

    private PackStore(Database database, Func<long?> family, Link link)
    {
        this.database = database;
        this.family = family;
        this.link = link;
    }

    /// <summary>The family whose pack this is — the one shown, or the one <see cref="In"/> held it to.</summary>
    public long? Family => family();

    /// <summary>This store, held to ONE family's pack whatever is shown — sharing this connection's catch-up.</summary>
    public PackStore In(long? familyId) => new(database, () => familyId, link);

    private string Key(string key) => FamilyKeys.Of(key, family());

    /// <summary>
    /// One socket's catch-up, shared by every family's store: which connection it is — counted up each time a socket opens,
    /// so a pass can say which one it caught up WITH — and which families have caught up on it. Read and written under the
    /// cache's own lock, and deliberately not kept in the file: a relaunch is a new connection, and it has not.
    /// </summary>
    private sealed class Link
    {
        public long Connection;

        /// <summary>The families caught up on this connection; 0 stands for the unscoped pack of an older cache.</summary>
        public readonly HashSet<long> CaughtUp = [];
    }

    private long Caught => family() ?? 0;

    /// <summary>The pack as it stands, in the order the items were added — the order a panel shows them in.</summary>
    public IReadOnlyList<PackItemDto> Items()
    {
        using var serialised = database.Hold();
        var items = new List<PackItemDto>();
        using var command = database.Connection.CreateCommand();
        command.CommandText = "SELECT * FROM pack_items" + Only() + " ORDER BY item_id ASC";
        Scope(command);
        using var reader = command.ExecuteReader();
        while (reader.Read())
        {
            items.Add(Read(reader));
        }
        return items;
    }

    public PackItemDto? Item(long itemId)
    {
        using var serialised = database.Hold();
        using var command = database.Connection.CreateCommand();
        command.CommandText = "SELECT * FROM pack_items WHERE item_id = $id";
        command.Parameters.AddWithValue("$id", itemId);
        using var reader = command.ExecuteReader();
        return reader.Read() ? Read(reader) : null;
    }

    /// <summary>How many items are held — what the pack's ceiling is checked against at the picker.</summary>
    public int Count()
    {
        using var serialised = database.Hold();
        using var command = database.Connection.CreateCommand();
        command.CommandText = "SELECT COUNT(*) FROM pack_items" + Only();
        Scope(command);
        return Convert.ToInt32(command.ExecuteScalar() ?? 0, CultureInfo.InvariantCulture);
    }

    /// <summary>
    /// A whole-pack read: what arrived IS the pack. Items the answer does not carry are gone —
    /// somebody removed them while this device was not looking — and are remembered as gone.
    /// </summary>
    /// <returns>Whether the read was applied; an older one landing second is not.</returns>
    public bool Replace(IReadOnlyList<PackItemDto> items, long maxPackSeq)
    {
        using var serialised = database.Hold();
        // TWO FULL READS CAN LAND IN EITHER ORDER, and an older one landing second must change
        // nothing at all: it would drop every item added between the two and set the cursor back.
        // Measured against the CURSOR ALREADY APPLIED, whichever of the three ways moved it — a
        // full read, a catch-up page, or a frame on a caught-up connection: a read taken before
        // any of those knows less than this device does, and would set the cursor back behind
        // changes it has already applied.
        if (maxPackSeq < Math.Max(Number(Meta(Key(FullMark))), Cursor))
        {
            return false;
        }
        using var transaction = database.Connection.BeginTransaction();
        var arrived = items.Where(item => !item.Deleted).Select(item => item.Id).ToHashSet();
        // A FULL READ THAT LEFT AN ITEM OUT HAS SEEN IT REMOVED, and that is remembered like any
        // tombstone — but only for what the read could have known about. An item held ABOVE the
        // read's mark arrived after the read was taken: it is not on that answer, and it stays.
        var left = new List<long>();
        using (var held = database.Connection.CreateCommand())
        {
            held.Transaction = transaction;
            held.CommandText = "SELECT item_id FROM pack_items WHERE pack_seq <= $mark" + Only("AND");
            held.Parameters.AddWithValue("$mark", maxPackSeq);
            Scope(held);
            using var reader = held.ExecuteReader();
            while (reader.Read())
            {
                var id = reader.GetInt64(0);
                if (!arrived.Contains(id))
                {
                    left.Add(id);
                }
            }
        }
        foreach (var id in left)
        {
            Remember(id, transaction);
        }
        using (var clear = database.Connection.CreateCommand())
        {
            clear.Transaction = transaction;
            clear.CommandText = "DELETE FROM pack_items WHERE pack_seq <= $mark" + Only("AND");
            clear.Parameters.AddWithValue("$mark", maxPackSeq);
            Scope(clear);
            clear.ExecuteNonQuery();
        }
        foreach (var item in items.Where(item => !item.Deleted))
        {
            // Through the same guard as everything else: never over a newer copy, never a
            // resurrection.
            Apply(item, transaction);
        }
        SetMeta(Key(CursorSeq), Text(maxPackSeq), transaction);
        SetMeta(Key(FullMark), Text(maxPackSeq), transaction);
        transaction.Commit();
        return true;
    }

    /// <summary>
    /// One item: from a frame, from the change feed, or from the answer to this device's own
    /// add or removal. A TOMBSTONE removes; anything not newer than what is held is dropped.
    /// </summary>
    /// <returns>Whether the pack, as drawn, changed.</returns>
    public bool Apply(PackItemDto item, SeqRoute route = SeqRoute.CatchUpPage)
    {
        using var serialised = database.Hold();
        using var transaction = database.Connection.BeginTransaction();
        var changed = Apply(item, transaction);
        var mayMove = route switch
        {
            // The item in a POST's answer, or the tombstone this device makes of its own DELETE:
            // one item's news, which says nothing about what else has changed.
            SeqRoute.Evidence => false,
            // ONLY ONCE THIS CONNECTION HAS CAUGHT UP. Before that the cursor is where the
            // catch-up starts from, and a frame that jumped it would leave everything between the
            // two unread for good (docs/protocol.md, "The cursor moves in three ways and no others").
            SeqRoute.LiveFrame => link.CaughtUp.Contains(Caught),
            _ => true,
        };
        if (mayMove && item.PackSeq > Cursor)
        {
            SetMeta(Key(CursorSeq), Text(item.PackSeq), transaction);
        }
        transaction.Commit();
        return changed;
    }

    /// <summary>A page of the change feed, applied in order; the cursor follows the page's highest seq.</summary>
    /// <returns>How many items changed what is drawn.</returns>
    public int Apply(IReadOnlyList<PackItemDto> items, SeqRoute route = SeqRoute.CatchUpPage)
    {
        using var serialised = database.Hold();
        using var transaction = database.Connection.BeginTransaction();
        var changed = 0;
        var highest = Cursor;
        foreach (var item in items)
        {
            if (Apply(item, transaction))
            {
                changed++;
            }
            highest = Math.Max(highest, item.PackSeq);
        }
        if (route != SeqRoute.Evidence)
        {
            SetMeta(Key(CursorSeq), Text(highest), transaction);
        }
        transaction.Commit();
        return changed;
    }

    private bool Apply(PackItemDto item, SqliteTransaction transaction)
    {
        // ONE GUARD FOR BOTH STATES: an item — live or a tombstone — is applied only when its
        // `pack_seq` is ABOVE the one held. A removal always takes a newer seq than the add it
        // removes, so this never holds a real tombstone back; what it stops is a copy that is not
        // newer than what this device already has.
        long? heldSeq;
        using (var held = database.Connection.CreateCommand())
        {
            held.Transaction = transaction;
            held.CommandText = "SELECT pack_seq FROM pack_items WHERE item_id = $id";
            held.Parameters.AddWithValue("$id", item.Id);
            heldSeq = held.ExecuteScalar() is { } current and not DBNull
                ? Convert.ToInt64(current, CultureInfo.InvariantCulture)
                : null;
        }
        if (heldSeq is { } mine && mine >= item.PackSeq)
        {
            return false;
        }
        if (item.Deleted)
        {
            // Whether the PANEL changed, which a tombstone for an item this device never held did
            // not — remembering it is not news.
            return Remove(item.Id, transaction);
        }
        if (item.Attachment is null || item.AddedBy <= 0)
        {
            // A live item IS its picture, and it always says who added it — that is what decides
            // who may remove it. One missing either is not something this client can draw, send
            // or offer a removal for: it is dropped, not stored as a blank square in the panel.
            return false;
        }
        if (IsGone(item.Id, transaction))
        {
            // A removal is the last word: this is an older copy that crossed it on the wire.
            return false;
        }
        Write(item, item.Attachment, transaction);
        return true;
    }

    /// <summary>
    /// This device's own <c>DELETE</c> was answered — <c>204</c>, or <c>pack_item_not_found</c>,
    /// which says the same thing: the item is not in the pack. It leaves the panel as the click
    /// lands and is remembered as gone, like a tombstone.
    /// </summary>
    /// <remarks>
    /// NOT THROUGH <see cref="Apply(PackItemDto, SeqRoute)"/>, and NO CURSOR: the seq the removal
    /// was given is the server's to announce, in the frame and on the change feed, and this device
    /// does not know it. A tombstone invented here would either fail the seq guard or move the
    /// cursor to a number nobody issued.
    /// </remarks>
    /// <returns>Whether the pack, as drawn, changed.</returns>
    public bool Removed(long itemId)
    {
        using var serialised = database.Hold();
        using var transaction = database.Connection.BeginTransaction();
        var changed = Remove(itemId, transaction);
        transaction.Commit();
        return changed;
    }

    private bool Remove(long itemId, SqliteTransaction transaction)
    {
        Remember(itemId, transaction);
        using (var forget = database.Connection.CreateCommand())
        {
            // A sticker that is gone cannot be "recently used" either.
            forget.Transaction = transaction;
            forget.CommandText = "DELETE FROM pack_recents WHERE item_id = $id";
            forget.Parameters.AddWithValue("$id", itemId);
            forget.ExecuteNonQuery();
        }
        using var delete = database.Connection.CreateCommand();
        delete.Transaction = transaction;
        delete.CommandText = "DELETE FROM pack_items WHERE item_id = $id";
        delete.Parameters.AddWithValue("$id", itemId);
        return delete.ExecuteNonQuery() > 0;
    }

    /// <summary>Where the change feed should carry on from; 0 until this device has read the pack at all.</summary>
    public long Cursor => Number(Meta(Key(CursorSeq)));

    // ---- one connection's catch-up ---------------------------------------

    /// <summary>
    /// A connection has OPENED: frames on it may not move the cursor until a pass that BEGAN on it
    /// has read what was missed while the socket was down.
    /// </summary>
    public void Reconnected()
    {
        using var serialised = database.Hold();
        link.Connection++;
        link.CaughtUp.Clear();
    }

    /// <summary>
    /// The connection a pass is beginning on — taken before its first request and handed back to
    /// <see cref="CaughtUp"/>, which is how a pass that a reconnect overtook is told apart from
    /// one that was not.
    /// </summary>
    public long Connection
    {
        get
        {
            using var serialised = database.Hold();
            return link.Connection;
        }
    }

    /// <summary>
    /// The pass reached the pack and is level with the server: frames may move the cursor from
    /// here — IF the connection the pass began on is still the one that is open.
    /// </summary>
    /// <remarks>
    /// <b>A PASS CATCHES UP THE CONNECTION IT BEGAN ON, AND NO OTHER.</b> A socket that drops and
    /// reopens while a pass is waiting on a request leaves that pass holding an answer taken
    /// before the new socket was listening: a change committed in between is on neither the
    /// answer nor the wire. Were that pass allowed to say "caught up", the next frame would step
    /// the cursor over the change, the pass the new connection started would find the cursor
    /// level with the server's mark and ask for nothing, and the change would be lost for good —
    /// a full read only ever happens at a cursor of 0. So an overtaken pass's word is dropped, and
    /// the pass the new connection started (every connection starts one) is the one that says it.
    /// </remarks>
    /// <param name="began">What <see cref="Connection"/> answered when the pass began.</param>
    /// <returns>Whether the connection is now caught up; false when a newer one has opened since.</returns>
    public bool CaughtUp(long began)
    {
        using var serialised = database.Hold();
        if (began != link.Connection)
        {
            return false;
        }
        link.CaughtUp.Add(Caught);
        return true;
    }

    /// <summary>Whether this connection has caught up.</summary>
    public bool IsCaughtUp
    {
        get
        {
            using var serialised = database.Hold();
            return link.CaughtUp.Contains(Caught);
        }
    }

    // ---- the ceilings ----------------------------------------------------

    /// <summary>
    /// The pack's ceilings as <c>GET /families/mine</c> last gave them, or null on a server that
    /// predates packs — which is how the window knows to offer no sticker button and no pack
    /// management there (docs/protocol.md, "What old clients and old servers do"). Kept in the
    /// cache so the answer is there before the first read of a launch, and offline.
    /// </summary>
    public PackLimits? Limits
    {
        get
        {
            using var serialised = database.Hold();
            var items = Meta(Key(LimitItems));
            var bytes = Meta(Key(LimitBytes));
            return items is null || bytes is null
                ? null
                : new PackLimits((int)Math.Min(Number(items), int.MaxValue), Number(bytes));
        }
    }

    /// <summary>What the family's own document said this time — null when it said nothing, which is an answer too.</summary>
    public void SetLimits(PackLimits? limits)
    {
        using var serialised = database.Hold();
        using var transaction = database.Connection.BeginTransaction();
        if (limits is { } known)
        {
            SetMeta(Key(LimitItems), Text(known.MaxItems), transaction);
            SetMeta(Key(LimitBytes), Text(known.MaxItemBytes), transaction);
        }
        else
        {
            using var clear = database.Connection.CreateCommand();
            clear.Transaction = transaction;
            clear.CommandText = "DELETE FROM meta WHERE key IN ($items, $bytes)";
            clear.Parameters.AddWithValue("$items", Key(LimitItems));
            clear.Parameters.AddWithValue("$bytes", Key(LimitBytes));
            clear.ExecuteNonQuery();
        }
        transaction.Commit();
    }

    // ---- what this device used last --------------------------------------

    /// <summary>This device just sent that item. Never on the wire.</summary>
    public void Used(long itemId, DateTimeOffset at)
    {
        using var serialised = database.Hold();
        using var command = database.Connection.CreateCommand();
        command.CommandText =
            "INSERT INTO pack_recents (item_id, used_at) VALUES ($id, $at) " +
            "ON CONFLICT(item_id) DO UPDATE SET used_at = excluded.used_at";
        command.Parameters.AddWithValue("$id", itemId);
        command.Parameters.AddWithValue("$at", at.ToUnixTimeMilliseconds());
        command.ExecuteNonQuery();
    }

    /// <summary>
    /// The items this device sent most recently, newest first — only ones the pack still holds.
    /// Kept in the cache file, so they are THIS DEVICE's and survive a restart; and gone with
    /// everything else at sign-out (<see cref="Database.WipeAll"/>).
    /// </summary>
    public IReadOnlyList<long> Recents(int limit = 16)
    {
        using var serialised = database.Hold();
        var ids = new List<long>();
        using var command = database.Connection.CreateCommand();
        command.CommandText =
            "SELECT r.item_id FROM pack_recents r JOIN pack_items i ON i.item_id = r.item_id " +
            (family() is null ? string.Empty : "WHERE i.family_id = $family ") +
            "ORDER BY r.used_at DESC, r.item_id DESC LIMIT $limit";
        command.Parameters.AddWithValue("$limit", Math.Max(0, limit));
        Scope(command);
        using var reader = command.ExecuteReader();
        while (reader.Read())
        {
            ids.Add(reader.GetInt64(0));
        }
        return ids;
    }

    // ---- rows ------------------------------------------------------------

    private void Remember(long itemId, SqliteTransaction transaction)
    {
        using var remember = database.Connection.CreateCommand();
        remember.Transaction = transaction;
        remember.CommandText = "INSERT OR IGNORE INTO pack_gone (item_id) VALUES ($id)";
        remember.Parameters.AddWithValue("$id", itemId);
        remember.ExecuteNonQuery();
    }

    private bool IsGone(long itemId, SqliteTransaction transaction)
    {
        using var command = database.Connection.CreateCommand();
        command.Transaction = transaction;
        command.CommandText = "SELECT 1 FROM pack_gone WHERE item_id = $id";
        command.Parameters.AddWithValue("$id", itemId);
        return command.ExecuteScalar() is not null;
    }

    private void Write(PackItemDto item, AttachmentDto picture, SqliteTransaction transaction)
    {
        using var command = database.Connection.CreateCommand();
        command.Transaction = transaction;
        command.CommandText =
            """
            INSERT INTO pack_items (item_id, added_by, label, created_at, pack_seq, attachment_json, family_id)
            VALUES ($id, $by, $label, $created, $seq, $attachment, $family)
            ON CONFLICT(item_id) DO UPDATE SET
                added_by = excluded.added_by, label = excluded.label,
                created_at = excluded.created_at, pack_seq = excluded.pack_seq,
                attachment_json = excluded.attachment_json,
                family_id = COALESCE(excluded.family_id, pack_items.family_id)
            """;
        // Whose pack: the family this store is held to, else what the item says.
        command.Parameters.AddWithValue("$family", family() ?? item.FamilyId ?? (object)DBNull.Value);
        command.Parameters.AddWithValue("$id", item.Id);
        command.Parameters.AddWithValue("$by", item.AddedBy);
        // Absent and empty are the same answer here: the server never sends an empty label.
        command.Parameters.AddWithValue("$label",
            string.IsNullOrEmpty(item.Label) ? DBNull.Value : item.Label);
        command.Parameters.AddWithValue("$created", Times.Instant(item.CreatedAt) ?? 0);
        command.Parameters.AddWithValue("$seq", item.PackSeq);
        command.Parameters.AddWithValue("$attachment", Wire.Encode(picture));
        command.ExecuteNonQuery();
    }

    private static PackItemDto Read(SqliteDataReader reader)
    {
        var created = Convert.ToInt64(reader["created_at"], CultureInfo.InvariantCulture);
        return new PackItemDto(
            Id: Convert.ToInt64(reader["item_id"], CultureInfo.InvariantCulture),
            AddedBy: Convert.ToInt64(reader["added_by"], CultureInfo.InvariantCulture),
            Attachment: Wire.Decode<AttachmentDto>(Convert.ToString(reader["attachment_json"]) ?? "null"),
            CreatedAt: created == 0 ? null : Times.Rfc3339(created),
            PackSeq: Convert.ToInt64(reader["pack_seq"], CultureInfo.InvariantCulture),
            Label: reader["label"] is DBNull ? null : Convert.ToString(reader["label"]),
            FamilyId: reader["family_id"] is DBNull ? null : Convert.ToInt64(reader["family_id"], CultureInfo.InvariantCulture));
    }

    /// <summary>
    /// The clause that keeps a read to this store's family — <c> WHERE family_id = $family</c>, or with <c>AND</c> after a
    /// clause of its own — and nothing for the unscoped pack of an older cache.
    /// </summary>
    private string Only(string join = "WHERE") => family() is null ? string.Empty : $" {join} family_id = $family";

    /// <summary>The <c>$family</c> that <see cref="Only"/> names, when it names one.</summary>
    private void Scope(SqliteCommand command)
    {
        if (family() is { } scoped)
        {
            command.Parameters.AddWithValue("$family", scoped);
        }
    }

    // ---- meta ------------------------------------------------------------

    /// <summary>A number this cache wrote for itself, read back as it was written: invariant both ways.</summary>
    private static long Number(string? text) =>
        long.TryParse(text, NumberStyles.Integer, CultureInfo.InvariantCulture, out var value)
            ? value
            : 0;

    private static string Text(long value) => value.ToString(CultureInfo.InvariantCulture);

    private string? Meta(string key)
    {
        using var serialised = database.Hold();
        using var command = database.Connection.CreateCommand();
        command.CommandText = "SELECT value FROM meta WHERE key = $key";
        command.Parameters.AddWithValue("$key", key);
        return command.ExecuteScalar() as string;
    }

    private void SetMeta(string key, string value, SqliteTransaction transaction)
    {
        using var command = database.Connection.CreateCommand();
        command.Transaction = transaction;
        command.CommandText =
            "INSERT INTO meta (key, value) VALUES ($key, $value) " +
            "ON CONFLICT(key) DO UPDATE SET value = excluded.value";
        command.Parameters.AddWithValue("$key", key);
        command.Parameters.AddWithValue("$value", value);
        command.ExecuteNonQuery();
    }
}
