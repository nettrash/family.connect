using FamilyConnect.Core.Board;
using FamilyConnect.Core.Protocol;
using Microsoft.Data.Sqlite;

namespace FamilyConnect.Core.Store;

/// <summary>
/// The wall, as this device holds it (docs/protocol.md, "Board").
/// </summary>
/// <remarks>
/// <para>
/// Two rules from the protocol are the whole design here. A FULL READ REPLACES what is held —
/// so <see cref="Replace"/> deletes what the answer does not mention, because a note somebody
/// took down while this device was away would otherwise stay on the wall for ever. And the
/// CHANGES feed carries TOMBSTONES, which are deletes and not notes: <see cref="Apply"/> removes
/// them rather than storing a blank sticker.
/// </para>
/// <para>
/// Seqs commit out of order, so every write is guarded by <c>board_seq</c>: a slower answer must
/// never overwrite a newer one it crossed on the wire. A TOMBSTONE WINS FOR GOOD — it is
/// remembered, so an older copy arriving afterwards cannot put the note back.
/// </para>
/// <para>
/// And the CURSOR IS NOT MOVED BY EVERY WRITE. A live frame and a page of the changes feed move
/// it; the answer to this client's OWN write is evidence about one note and says nothing about
/// what else has happened, so moving the cursor from it would step the feed past somebody else's
/// change — the same rule, and the same reasoning, as the chat cursors in
/// <see cref="ChatStore.Advance"/>.
/// </para>
/// </remarks>
public sealed class BoardStore(Database database)
{
    private const string MarkNoteId = "board.mark.note_id";
    private const string MarkContentSeq = "board.mark.content_seq";
    private const string CursorSeq = "board.cursor.board_seq";

    /// <summary>
    /// The <c>max_board_seq</c> of the newest FULL read applied. Two can be in flight at once and
    /// land in either order, and an older one landing second must change nothing.
    /// </summary>
    private const string FullMark = "board.full.board_seq";

    /// <summary>The whole wall as it stands, newest change first — what the window draws.</summary>
    public IReadOnlyList<NoteDto> Notes()
    {
        var notes = new List<NoteDto>();
        using var command = database.Connection.CreateCommand();
        command.CommandText = "SELECT * FROM notes ORDER BY board_seq DESC";
        using var reader = command.ExecuteReader();
        while (reader.Read())
        {
            notes.Add(Read(reader));
        }
        return notes;
    }

    public NoteDto? Note(long noteId)
    {
        using var command = database.Connection.CreateCommand();
        command.CommandText = "SELECT * FROM notes WHERE note_id = $id";
        command.Parameters.AddWithValue("$id", noteId);
        using var reader = command.ExecuteReader();
        return reader.Read() ? Read(reader) : null;
    }

    /// <summary>
    /// A whole-board read: what arrived IS the wall. Notes the answer does not carry are gone —
    /// somebody took them down while this device was not looking.
    /// </summary>
    public void Replace(IReadOnlyList<NoteDto> notes, long maxBoardSeq)
    {
        // TWO FULL READS CAN LAND IN EITHER ORDER, and an older one landing second must change
        // nothing at all: it would drop every note written between the two and set the cursor
        // back to its own mark.
        if (maxBoardSeq < long.Parse(Meta(FullMark) ?? "0"))
        {
            return;
        }
        using var transaction = database.Connection.BeginTransaction();
        using (var clear = database.Connection.CreateCommand())
        {
            clear.Transaction = transaction;
            // EXCEPT what is NEWER THAN THE READ'S OWN MARK: a frame that landed while the read
            // was in flight is not on that answer, and wiping it would take a note off the wall
            // seconds after somebody pinned it — then put it back at the next catch-up.
            clear.CommandText = "DELETE FROM notes WHERE board_seq <= $mark";
            clear.Parameters.AddWithValue("$mark", maxBoardSeq);
            clear.ExecuteNonQuery();
        }
        foreach (var note in notes.Where(note => !note.Deleted))
        {
            if (!IsGone(note.Id, transaction))
            {
                Write(note, transaction);
            }
        }
        SetMeta(CursorSeq, maxBoardSeq.ToString(), transaction);
        SetMeta(FullMark, maxBoardSeq.ToString(), transaction);
        transaction.Commit();
    }

    private bool IsGone(long noteId, SqliteTransaction transaction)
    {
        using var command = database.Connection.CreateCommand();
        command.Transaction = transaction;
        command.CommandText = "SELECT 1 FROM gone WHERE note_id = $id";
        command.Parameters.AddWithValue("$id", noteId);
        return command.ExecuteScalar() is not null;
    }

    /// <summary>
    /// One note from the change feed or from an answer to this device's own write. A TOMBSTONE
    /// deletes; anything older than what is held is dropped.
    /// </summary>
    /// <returns>Whether anything changed.</returns>
    public bool Apply(NoteDto note, SeqRoute route = SeqRoute.CatchUpPage)
    {
        using var transaction = database.Connection.BeginTransaction();
        var changed = Apply(note, transaction);
        // A FRAME MOVES THE CURSOR ONLY ONCE THIS DEVICE HAS READ THE BOARD. Before that the
        // cursor is 0, which is what tells the resync to read the WHOLE wall — and a frame that
        // jumped that queue would leave the cursor above changes nobody had read, so the wall
        // would be whatever frames happened to arrive (docs/protocol.md, "The board cursor moves
        // in three ways and no others").
        var mayMove = route switch
        {
            SeqRoute.Evidence => false,
            SeqRoute.LiveFrame => Cursor != 0,
            _ => true,
        };
        if (mayMove && note.BoardSeq > Cursor)
        {
            SetMeta(CursorSeq, note.BoardSeq.ToString(), transaction);
        }
        transaction.Commit();
        return changed;
    }

    /// <summary>A page of the change feed, applied in order.</summary>
    public int Apply(IReadOnlyList<NoteDto> notes, SeqRoute route = SeqRoute.CatchUpPage)
    {
        using var transaction = database.Connection.BeginTransaction();
        var changed = 0;
        var highest = Cursor;
        foreach (var note in notes)
        {
            if (Apply(note, transaction))
            {
                changed++;
            }
            highest = Math.Max(highest, note.BoardSeq);
        }
        if (route != SeqRoute.Evidence)
        {
            SetMeta(CursorSeq, highest.ToString(), transaction);
        }
        transaction.Commit();
        return changed;
    }

    private bool Apply(NoteDto note, SqliteTransaction transaction)
    {
        if (note.Deleted)
        {
            // Remembered as well as removed: a note is taken down once and for all, and an
            // older copy of it is still travelling.
            using (var remember = database.Connection.CreateCommand())
            {
                remember.Transaction = transaction;
                remember.CommandText = "INSERT OR IGNORE INTO gone (note_id) VALUES ($id)";
                remember.Parameters.AddWithValue("$id", note.Id);
                remember.ExecuteNonQuery();
            }
            using var delete = database.Connection.CreateCommand();
            delete.Transaction = transaction;
            delete.CommandText = "DELETE FROM notes WHERE note_id = $id";
            delete.Parameters.AddWithValue("$id", note.Id);
            // Whether the WALL changed, which a tombstone for a note this device never held did
            // not — remembering it is not news.
            return delete.ExecuteNonQuery() > 0;
        }
        if (IsGone(note.Id, transaction))
        {
            // A tombstone is the last word. This is an older copy that crossed it on the wire,
            // and writing it would put a note the family took down back on the wall.
            return false;
        }
        using var held = database.Connection.CreateCommand();
        held.Transaction = transaction;
        held.CommandText = "SELECT board_seq FROM notes WHERE note_id = $id";
        held.Parameters.AddWithValue("$id", note.Id);
        var current = held.ExecuteScalar();
        if (current is not null and not DBNull && Convert.ToInt64(current) >= note.BoardSeq)
        {
            // A slower answer that crossed a newer one on the wire. Seqs commit out of order, so
            // this is ordinary rather than exceptional.
            return false;
        }
        Write(note, transaction);
        return true;
    }

    /// <summary>Where the change feed should carry on from.</summary>
    public long Cursor => long.TryParse(Meta(CursorSeq), out var seq) ? seq : 0;

    /// <summary>
    /// What this DEVICE has shown its reader — not the sync cursor, and moved only when the board
    /// has actually been on screen (docs/protocol.md, "Board").
    /// </summary>
    public BoardMarks Marks =>
        new(long.TryParse(Meta(MarkNoteId), out var id) ? id : 0,
            long.TryParse(Meta(MarkContentSeq), out var seq) ? seq : 0);

    /// <summary>Monotonic in both fields: a cleared badge must not come back.</summary>
    public void Mark(BoardMarks marks)
    {
        var later = BoardMarks.Later(Marks, marks);
        using var transaction = database.Connection.BeginTransaction();
        SetMeta(MarkNoteId, later.NoteId.ToString(), transaction);
        SetMeta(MarkContentSeq, later.ContentSeq.ToString(), transaction);
        transaction.Commit();
    }

    /// <summary>The marks after the board has been on screen, applied.</summary>
    public void MarkShown() =>
        Mark(BoardBadge.AfterShowing(
            Notes().Select(note => (note.Id, (long?)(note.ContentSeq ?? 0))), Marks));

    /// <summary>How many notes have something new to READ — the badge.</summary>
    public int Unread() =>
        BoardBadge.Count(Notes().Select(note => (note.Id, (long?)(note.ContentSeq ?? 0))), Marks);

    // ---- rows ------------------------------------------------------------

    private void Write(NoteDto note, SqliteTransaction transaction)
    {
        using var command = database.Connection.CreateCommand();
        command.Transaction = transaction;
        command.CommandText =
            """
            INSERT INTO notes (note_id, author_id, kind, text, color, size, font, x, y,
                               created_at, updated_at, board_seq, content_seq,
                               starts_at, ends_at, place,
                               attachment_json, rsvps_json, mentions_json, items_json)
            VALUES ($id, $author, $kind, $text, $color, $size, $font, $x, $y,
                    $created, $updated, $seq, $content,
                    $starts, $ends, $place,
                    $attachment, $rsvps, $mentions, $items)
            ON CONFLICT(note_id) DO UPDATE SET
                author_id = excluded.author_id, kind = excluded.kind, text = excluded.text,
                color = excluded.color, size = excluded.size, font = excluded.font,
                x = excluded.x, y = excluded.y, updated_at = excluded.updated_at,
                board_seq = excluded.board_seq, content_seq = excluded.content_seq,
                starts_at = excluded.starts_at, ends_at = excluded.ends_at, place = excluded.place,
                attachment_json = excluded.attachment_json, rsvps_json = excluded.rsvps_json,
                mentions_json = excluded.mentions_json, items_json = excluded.items_json
            """;
        command.Parameters.AddWithValue("$id", note.Id);
        command.Parameters.AddWithValue("$author", note.AuthorId);
        // An unknown kind, colour, size or face is kept AS SENT and drawn at the fallback: a
        // fourth size from a newer server must not be written back as medium by an edit
        // (docs/protocol.md, "Board").
        command.Parameters.AddWithValue("$kind", note.Kind ?? "text");
        command.Parameters.AddWithValue("$text", note.Text ?? string.Empty);
        command.Parameters.AddWithValue("$color", note.Color ?? "yellow");
        command.Parameters.AddWithValue("$size", note.Size ?? "medium");
        command.Parameters.AddWithValue("$font", note.Font ?? "plain");
        command.Parameters.AddWithValue("$x", note.X);
        command.Parameters.AddWithValue("$y", note.Y);
        command.Parameters.AddWithValue("$created", Times.Instant(note.CreatedAt) ?? 0);
        command.Parameters.AddWithValue("$updated", Times.Instant(note.UpdatedAt) ?? 0);
        command.Parameters.AddWithValue("$seq", note.BoardSeq);
        command.Parameters.AddWithValue("$content", note.ContentSeq ?? 0);
        command.Parameters.AddWithValue("$starts", Times.Instant(note.StartsAt) ?? (object)DBNull.Value);
        command.Parameters.AddWithValue("$ends", Times.Instant(note.EndsAt) ?? (object)DBNull.Value);
        command.Parameters.AddWithValue("$place", note.Place ?? (object)DBNull.Value);
        command.Parameters.AddWithValue("$attachment",
            note.Attachment is null ? DBNull.Value : Wire.Encode(note.Attachment));
        // `[]` and ABSENT are different answers and both are kept: an event nobody has answered
        // sends `[]`, and every other kind sends nothing at all.
        command.Parameters.AddWithValue("$rsvps",
            note.Rsvps is null ? DBNull.Value : Wire.Encode(note.Rsvps));
        command.Parameters.AddWithValue("$mentions",
            note.Mentions is null ? DBNull.Value : Wire.Encode(note.Mentions));
        command.Parameters.AddWithValue("$items",
            note.Items is null ? DBNull.Value : Wire.Encode(note.Items));
        command.ExecuteNonQuery();
    }

    private static NoteDto Read(SqliteDataReader reader)
    {
        string? Text(string column) =>
            reader[column] is DBNull ? null : Convert.ToString(reader[column]);
        long? Number(string column) =>
            reader[column] is DBNull ? null : Convert.ToInt64(reader[column]);
        var contentSeq = Convert.ToInt64(reader["content_seq"]);
        return new NoteDto(
            Id: Convert.ToInt64(reader["note_id"]),
            AuthorId: Convert.ToInt64(reader["author_id"]),
            Kind: Text("kind"),
            Text: Text("text") ?? string.Empty,
            Color: Text("color"),
            Size: Text("size"),
            Font: Text("font"),
            X: Convert.ToDouble(reader["x"]),
            Y: Convert.ToDouble(reader["y"]),
            CreatedAt: Times.Rfc3339(Number("created_at")),
            UpdatedAt: Times.Rfc3339(Number("updated_at")),
            BoardSeq: Convert.ToInt64(reader["board_seq"]),
            // 0 is this table's spelling of "the server never said", which is an ABSENT
            // content_seq on the wire — and the badge then judges the note by its id.
            ContentSeq: contentSeq == 0 ? null : contentSeq,
            Attachment: Wire.Decode<AttachmentDto>(Text("attachment_json") ?? "null"),
            StartsAt: Times.Rfc3339(Number("starts_at")),
            EndsAt: Times.Rfc3339(Number("ends_at")),
            Place: Text("place"),
            Rsvps: Text("rsvps_json") is { } rsvps ? Wire.Decode<RsvpDto[]>(rsvps) : null,
            Mentions: Text("mentions_json") is { } mentions ? Wire.Decode<MentionDto[]>(mentions) : null,
            Items: Text("items_json") is { } items ? Wire.Decode<TaskItemDto[]>(items) : null);
    }

    // ---- meta ------------------------------------------------------------

    private string? Meta(string key)
    {
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
