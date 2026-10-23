using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.Core.Tests.Store;

/// <summary>
/// The cache, one account in several families (docs/protocol.md, "Several families"; schema step 11): which family each held
/// thing belongs to, a roster, a wall and a pack per family with cursors of their own, leaving ONE family dropping only its
/// rows, and a cache an older build filled made the home family's.
/// </summary>
public class SeveralFamiliesStoreTests : IDisposable
{
    private readonly string directory = Path.Combine(Path.GetTempPath(), "fc-families-" + Guid.NewGuid().ToString("N"));
    private readonly Database database = Database.OpenInMemory();

    public SeveralFamiliesStoreTests() => Directory.CreateDirectory(directory);

    public void Dispose()
    {
        database.Dispose();
        try
        {
            Directory.Delete(directory, recursive: true);
        }
        catch (IOException)
        {
            // A file the OS is still holding is not a test failure.
        }
    }

    private const string Sent = "2026-10-22T10:00:00Z";

    private static PackItemDto Item(long id, long seq, long? family = null) =>
        new(id, 7, new AttachmentDto(70 + id, "photo", "image/png", 4096, 512, 512), Sent, seq, FamilyId: family);

    // ---- the migration ------------------------------------------------------------------------------------------------------

    /// <summary>
    /// Version 10 → 11 adds which family each thing belongs to and a roster per family — and READS NOTHING AGAIN: the chats,
    /// their messages, the wall, the pack and the outbox stay, their families unknown (NULL) until the home family is named.
    /// </summary>
    [Fact]
    public void AVersionTenCacheGainsFamiliesAndLosesNothing()
    {
        var path = Path.Combine(directory, "ten.db");
        using (var ten = new Microsoft.Data.Sqlite.SqliteConnection($"Data Source={path}"))
        {
            ten.Open();
            foreach (var statement in Migrations.All.Take(10).SelectMany(step => step)
                         .Append("INSERT INTO chats (chat_id, kind, title) VALUES (42, 'family', 'The Smiths')")
                         .Append("INSERT INTO messages (message_id, chat_id, sender_id, body, created_at, sequenced) VALUES (20, 42, 9, 'hi', 0, 1)")
                         .Append("INSERT INTO notes (note_id, author_id, text, board_seq) VALUES (1, 9, 'Milk', 4)")
                         .Append("INSERT INTO pack_items (item_id, added_by, pack_seq, attachment_json) VALUES (5, 9, 2, '{\"id\":75,\"kind\":\"photo\"}')")
                         .Append("INSERT INTO members (user_id, username, display_name, role) VALUES (9, 'bob', 'Bob', 'owner')")
                         .Append("INSERT INTO meta (key, value) VALUES ('board.cursor.board_seq', '4')")
                         .Append("INSERT INTO outbox (client_msg_id, chat_id, body, queued_at) VALUES ('k', 42, 'Dinner at 7?', 0)")
                         .Append("PRAGMA user_version = 10"))
            {
                using var command = ten.CreateCommand();
                command.CommandText = statement;
                command.ExecuteNonQuery();
            }
        }
        Microsoft.Data.Sqlite.SqliteConnection.ClearAllPools();

        using var migrated = Database.Open(path);
        Assert.Equal(Database.SchemaVersion, migrated.UserVersion);
        var chats = new ChatStore(migrated);
        Assert.Equal("hi", Assert.Single(chats.Messages(42)).Body);
        Assert.Null(chats.Chat(42)!.Chat.FamilyIds);
        Assert.Equal("Dinner at 7?", Assert.Single(new OutboxStore(migrated).All()).Body);
        var board = new BoardStore(migrated);
        Assert.Null(Assert.Single(board.Notes()).FamilyId);
        Assert.Equal(4, board.Cursor);
        Assert.Null(Assert.Single(new PackStore(migrated).Items()).FamilyId);
        Assert.Contains("family_members", migrated.Tables());
        Assert.Contains(migrated.Columns("chats"), column => column.StartsWith("family_ids_json", StringComparison.Ordinal));
        Assert.Contains(migrated.Columns("notes"), column => column.StartsWith("family_id", StringComparison.Ordinal));
        Assert.Contains(migrated.Columns("pack_items"), column => column.StartsWith("family_id", StringComparison.Ordinal));

        // Named, the home family adopts it all: the wall, the pack, the roster and the cursors are family 3's.
        migrated.AdoptLegacy(3);
        var home = new BoardStore(migrated, () => 3);
        Assert.Equal(3, Assert.Single(home.Notes()).FamilyId);
        Assert.Equal(4, home.Cursor);
        Assert.Equal(0, board.Cursor);
        Assert.Equal(3, Assert.Single(new PackStore(migrated, () => 3).Items()).FamilyId);
        Assert.True(Assert.Single(new ChatStore(migrated, () => 7, () => 3).Members()).Owner);
        Assert.Equal([3L], migrated.HeldFamilies());
        // And twice is once.
        migrated.AdoptLegacy(3);
        Assert.Equal(4, home.Cursor);
        Assert.Single(home.Notes());
    }

    // ---- chats --------------------------------------------------------------------------------------------------------------

    /// <summary>A chat keeps the families it is listed in, from the list and from a `chat_updated` alike.</summary>
    [Fact]
    public void AChatKeepsTheFamiliesItIsListedIn()
    {
        var chats = new ChatStore(database);
        chats.Replace([new ChatRowDto(new ChatDto(44, "direct", "Bob", 9, FamilyIds: [3, 5]))]);
        Assert.Equal([3L, 5L], chats.Chat(44)!.Chat.FamilyIds!);
        chats.Upsert(new ChatDto(44, "direct", "Bob", 9, FamilyIds: [5]));
        Assert.Equal([5L], chats.Chat(44)!.Chat.FamilyIds!);
        Assert.Equal([5L], Assert.Single(chats.Chats()).Chat.FamilyIds!);
    }

    // ---- rosters --------------------------------------------------------------------------------------------------------------

    /// <summary>
    /// A ROSTER PER FAMILY. The same person is one row — their name and picture are the account's — and a member of each family
    /// with that family's role; the roster read is the family SHOWN, and a name resolves for anybody this device has met.
    /// </summary>
    [Fact]
    public void EachFamilyHasItsOwnRosterOverOnePerson()
    {
        long? shown = 3;
        var chats = new ChatStore(database, () => 7, () => shown);
        chats.Replace(3, [new MemberDto(7, "anna", "Anna", Role: "member"), new MemberDto(9, "bob", "Bob", Role: "owner")]);
        chats.Replace(5, [new MemberDto(7, "anna", "Anna", Role: "owner"), new MemberDto(11, "carl", "Carl", Role: "member", PasswordReset: true)],
            [new MemberDto(12, "dana", "Dana", HasLeft: true)]);

        Assert.Equal([7L, 9L], chats.Members().Select(member => member.Id));
        Assert.False(chats.Members().Single(member => member.Id == 7).Owner);
        // Somebody from the other family still has a name.
        Assert.Equal("Carl", chats.Member(11)!.DisplayName);
        Assert.Equal([3L, 5L], chats.RosterFamilies());

        shown = 5;
        Assert.Equal([12L, 7L, 11L], chats.Members().Select(member => member.Id).OrderBy(id => id == 12 ? 0 : 1).ThenBy(id => id));
        Assert.True(chats.Members().Single(member => member.Id == 7).Owner);
        Assert.True(chats.Members().Single(member => member.Id == 11).PasswordReset);
        Assert.True(chats.Members().Single(member => member.Id == 12).IsFormer);
        Assert.Null(chats.MembersOf(3).Single(member => member.Id == 9).PasswordReset);

        // A family's own read replaces that family's rows and nobody else's.
        chats.Replace(5, [new MemberDto(7, "anna", "Anna", Role: "owner")]);
        Assert.Equal([7L], chats.Members().Select(member => member.Id));
        Assert.Equal([7L, 9L], chats.MembersOf(3).Select(member => member.Id));
    }

    /// <summary>The narrow writers go to the family named — and, naming none, the family shown.</summary>
    [Fact]
    public void JoinsLeavesAndOwnersAreAFamilys()
    {
        long? shown = 3;
        var chats = new ChatStore(database, () => 7, () => shown);
        chats.Replace(3, [new MemberDto(7, "anna", "Anna", Role: "owner"), new MemberDto(9, "bob", "Bob", Role: "member")]);
        chats.Replace(5, [new MemberDto(9, "bob", "Bob", Role: "owner"), new MemberDto(7, "anna", "Anna", Role: "member")]);

        chats.Joined(new UserDto(12, "dana", "Dana"), 5);
        chats.Left(9, 5);
        chats.SetOwner(7, 5);
        Assert.DoesNotContain(chats.Members(), member => member.Id == 12);
        Assert.False(chats.Members().Single(member => member.Id == 9).IsFormer);
        Assert.True(chats.MembersOf(5).Single(member => member.Id == 9).HasLeft);
        Assert.True(chats.MembersOf(5).Single(member => member.Id == 7).Owner);
        Assert.True(chats.Members().Single(member => member.Id == 7).Owner);

        // Naming none: the family shown.
        chats.Left(9);
        Assert.True(chats.Members().Single(member => member.Id == 9).HasLeft);
        // An account deleted is out of every family.
        chats.Deleted(new MemberDto(12, "deleted-12", "Deleted account", Deleted: true));
        Assert.True(chats.MembersOf(5).Single(member => member.Id == 12).Deleted);
        Assert.True(chats.MembersOf(5).Single(member => member.Id == 12).HasLeft);
    }

    // ---- the wall and the pack --------------------------------------------------------------------------------------------------

    /// <summary>
    /// ONE WALL PER FAMILY, each with its own seqs, cursor, full-read mark and badge: a full read of one family's wall drops
    /// nothing of another's, and seeing one wall marks nothing on the other.
    /// </summary>
    [Fact]
    public void EachFamilyHasItsOwnWallAndCursor()
    {
        long? shown = 3;
        var board = new BoardStore(database, () => shown);
        var other = board.In(5);
        Assert.Equal(3, board.Family);
        Assert.Equal(5, other.Family);

        board.Replace([new NoteDto(1, 9, "text", "Milk", BoardSeq: 4, ContentSeq: 4)], 4);
        other.Replace([new NoteDto(2, 11, "text", "Bread", BoardSeq: 9, ContentSeq: 9)], 9);
        // A full read of family 3 at a LOWER seq is not older than family 5's wall: each wall has its own numbers.
        board.Replace([new NoteDto(1, 9, "text", "Milk", BoardSeq: 4, ContentSeq: 4), new NoteDto(3, 9, "text", "Tea", BoardSeq: 5, ContentSeq: 5)], 5);

        Assert.Equal([3L, 1L], board.Notes().Select(note => note.Id));
        Assert.All(board.Notes(), note => Assert.Equal(3, note.FamilyId));
        Assert.Equal(2, Assert.Single(other.Notes()).Id);
        Assert.Equal(5, board.Cursor);
        Assert.Equal(9, other.Cursor);
        Assert.Equal(2, board.Unread());
        board.MarkShown();
        Assert.Equal(0, board.Unread());
        Assert.Equal(1, other.Unread());

        // A frame names its family; the store held to that family writes it there.
        other.Apply(new NoteDto(4, 11, "text", "Eggs", BoardSeq: 10, ContentSeq: 10, FamilyId: 5), SeqRoute.LiveFrame);
        Assert.Equal(10, other.Cursor);
        Assert.Equal(5, board.Cursor);
        shown = 5;
        Assert.Equal([4L, 2L], board.Notes().Select(note => note.Id));
    }

    /// <summary>
    /// One pack per family, with its own ceilings and catch-up — and ONE connection: a socket reopening un-catches every family,
    /// and catching one up does not catch the others.
    /// </summary>
    [Fact]
    public void EachFamilyHasItsOwnPackOnOneConnection()
    {
        long? shown = 3;
        var pack = new PackStore(database, () => shown);
        var other = pack.In(5);
        pack.Replace([Item(1, 2)], 2);
        other.Replace([Item(2, 7)], 7);
        pack.SetLimits(new PackLimits(200, 524_288));
        Assert.Equal([1L], pack.Items().Select(item => item.Id));
        Assert.Equal(3, Assert.Single(pack.Items()).FamilyId);
        Assert.Equal(1, other.Count());
        Assert.Equal(2, pack.Cursor);
        Assert.Equal(7, other.Cursor);
        Assert.NotNull(pack.Limits);
        Assert.Null(other.Limits);

        pack.Reconnected();
        var began = other.Connection;
        Assert.True(pack.CaughtUp(began));
        Assert.True(pack.IsCaughtUp);
        Assert.False(other.IsCaughtUp);
        // A frame on a family not yet caught up does not move its cursor.
        other.Apply(Item(3, 8, 5), SeqRoute.LiveFrame);
        Assert.Equal(7, other.Cursor);
        Assert.True(other.CaughtUp(began));
        other.Apply(Item(4, 9, 5), SeqRoute.LiveFrame);
        Assert.Equal(9, other.Cursor);
        pack.Used(1, DateTimeOffset.UnixEpoch);
        other.Used(4, DateTimeOffset.UnixEpoch.AddSeconds(1));
        Assert.Equal([1L], pack.Recents());
        Assert.Equal([4L], other.Recents());
        pack.Reconnected();
        Assert.False(pack.IsCaughtUp);
        Assert.False(other.IsCaughtUp);
    }

    // ---- leaving one family -------------------------------------------------------------------------------------------------

    /// <summary>
    /// LEAVING ONE FAMILY DROPS ONLY ITS ROWS (docs/protocol.md, "Frames": "drops the family's chats, board, pack, roster and
    /// cursors — and only that family's"): its family chat with its messages and queued sends, its wall, its pack and their
    /// cursors and marks, its roster — while a direct chat still listed in another family stays, naming that one only.
    /// </summary>
    [Fact]
    public void DroppingAFamilyTakesItsRowsAndNoOthers()
    {
        var chats = new ChatStore(database, () => 7, () => 3);
        chats.Replace([
            new ChatRowDto(new ChatDto(42, "family", "The Smiths", FamilyIds: [3])),
            new ChatRowDto(new ChatDto(43, "family", "Ivanovs", FamilyIds: [5])),
            new ChatRowDto(new ChatDto(44, "direct", "Bob", 9, FamilyIds: [3, 5])),
            new ChatRowDto(new ChatDto(45, "direct", "Carl", 11, FamilyIds: [5])),
        ]);
        chats.Apply(new MessageDto(1, 43, 11, "c-1", "hi", Sent));
        chats.Apply(new MessageDto(2, 44, 9, "c-2", "hello", Sent));
        new OutboxStore(database).Queue(new OutboxRow("q-1", 43, "queued"));
        new OutboxStore(database).Queue(new OutboxRow("q-2", 42, "stays"));
        chats.Replace(3, [new MemberDto(7, "anna", "Anna", Role: "owner"), new MemberDto(9, "bob", "Bob")]);
        chats.Replace(5, [new MemberDto(7, "anna", "Anna"), new MemberDto(11, "carl", "Carl", Role: "owner")]);
        var board = new BoardStore(database);
        board.In(3).Replace([new NoteDto(1, 9, "text", "Milk", BoardSeq: 4)], 4);
        board.In(5).Replace([new NoteDto(2, 11, "text", "Bread", BoardSeq: 9)], 9);
        var pack = new PackStore(database);
        pack.In(3).Replace([Item(1, 2)], 2);
        pack.In(5).Replace([Item(2, 7)], 7);
        pack.In(5).Used(2, DateTimeOffset.UnixEpoch);
        Assert.Equal([3L, 5L], database.HeldFamilies());

        var gone = database.DropFamily(5);

        Assert.Equal([43L, 45L], gone.Order());
        Assert.Null(chats.Chat(43));
        Assert.Null(chats.Chat(45));
        Assert.Empty(chats.Messages(43));
        Assert.Equal([3L], chats.Chat(44)!.Chat.FamilyIds!);
        Assert.Equal("hello", Assert.Single(chats.Messages(44)).Body);
        Assert.Equal("q-2", Assert.Single(new OutboxStore(database).All()).ClientMsgId);
        Assert.Empty(board.In(5).Notes());
        Assert.Equal(0, board.In(5).Cursor);
        Assert.Single(board.In(3).Notes());
        Assert.Equal(4, board.In(3).Cursor);
        Assert.Empty(pack.In(5).Items());
        Assert.Equal(0, pack.In(5).Cursor);
        Assert.Empty(pack.In(5).Recents());
        Assert.Single(pack.In(3).Items());
        Assert.Empty(chats.MembersOf(5));
        Assert.Equal(2, chats.MembersOf(3).Count);
        // The person stays a name: their old messages still need one.
        Assert.Equal("Carl", chats.Member(11)!.DisplayName);
        Assert.Equal([3L], database.HeldFamilies());
        // A family's keys are its own: dropping 5 leaves a family 55 alone.
        board.In(55).Replace([new NoteDto(3, 9, "text", "Tea", BoardSeq: 2)], 2);
        database.DropFamily(5);
        Assert.Equal(2, board.In(55).Cursor);
    }
}
