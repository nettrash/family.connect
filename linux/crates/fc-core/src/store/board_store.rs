//! The wall, as this device holds it (docs/protocol.md, "Board") — a port of `BoardStore.cs`.
//!
//! Two rules from the protocol are the whole design here. A FULL READ REPLACES what is held — so
//! [`BoardStore::replace`] deletes what the answer does not mention, because a note somebody took
//! down while this device was away would otherwise stay on the wall for ever. And the CHANGES feed
//! carries TOMBSTONES, which are deletes and not notes: [`BoardStore::apply`] removes them rather
//! than storing a blank sticker.
//!
//! Seqs commit out of order, so every write is guarded by `board_seq`: a slower answer must never
//! overwrite a newer one it crossed on the wire. A TOMBSTONE WINS FOR GOOD — it is remembered, so
//! an older copy arriving afterwards cannot put the note back.
//!
//! And the CURSOR IS NOT MOVED BY EVERY WRITE. A live frame and a page of the changes feed move
//! it; the answer to this client's OWN write is evidence about one note and says nothing about what
//! else has happened, so moving the cursor from it would step the feed past somebody else's change
//! — the same rule, and the same reasoning, as the chat cursors in
//! [`ChatStore::advance`](super::ChatStore::advance).
//!
//! **ONE WALL PER FAMILY** (docs/protocol.md, "Several families"): every family has its own board,
//! its own seqs, its own cursor and its own badge marks. A store reads and writes the wall of the
//! family its `current` says is being shown, and [`BoardStore::in_family`] is the same store held
//! to one family — what a resync uses for each family in turn. `None` is the one wall of a cache
//! from before several families, unscoped.

use std::fmt;
use std::rc::Rc;

use fc_text::board::{Marks, later, marks_after_showing};
use rusqlite::{Connection, OptionalExtension, Row, named_params};

use super::chat_store::SeqRoute;
use super::database::{Database, StoreResult};
use super::{family_keys, times};
use crate::board::board_badge;
use crate::protocol::{AttachmentDto, MentionDto, NoteDto, RsvpDto, TaskItemDto, wire};

const MARK_NOTE_ID: &str = "board.mark.note_id";
const MARK_CONTENT_SEQ: &str = "board.mark.content_seq";
const CURSOR_SEQ: &str = "board.cursor.board_seq";

/// The `max_board_seq` of the newest FULL read applied. Two can be in flight at once and land in
/// either order, and an older one landing second must change nothing.
const FULL_MARK: &str = "board.full.board_seq";

/// Every key this store keeps in `meta`, before a family's suffix — what
/// [`Database::adopt_legacy`] moves.
pub(crate) const META_KEYS: [&str; 4] = [MARK_NOTE_ID, MARK_CONTENT_SEQ, CURSOR_SEQ, FULL_MARK];

/// The wall, as this device holds it.
pub struct BoardStore {
    database: Rc<Database>,
    family: Box<dyn Fn() -> Option<i64>>,
}

impl fmt::Debug for BoardStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoardStore")
            .field("family", &self.family())
            .finish_non_exhaustive()
    }
}

impl BoardStore {
    /// The one wall of a cache from before several families — `new BoardStore(database)`.
    pub fn new(database: &Rc<Database>) -> Self {
        BoardStore {
            database: Rc::clone(database),
            family: Box::new(|| None),
        }
    }

    /// The same store, reading the wall of the family `current` says is shown (the C#'s
    /// `current`).
    pub fn with_current(mut self, current: impl Fn() -> Option<i64> + 'static) -> Self {
        self.family = Box::new(current);
        self
    }

    /// The family whose wall this is — the one shown, or the one [`in_family`](Self::in_family)
    /// held it to.
    pub fn family(&self) -> Option<i64> {
        (self.family)()
    }

    /// This store, held to ONE family's wall whatever is shown (the C#'s `In`).
    pub fn in_family(&self, family_id: Option<i64>) -> BoardStore {
        BoardStore {
            database: Rc::clone(&self.database),
            family: Box::new(move || family_id),
        }
    }

    /// A family's own spelling of a `meta` key: `board.cursor.board_seq@3`.
    fn key(&self, key: &str) -> String {
        family_keys::of(key, self.family())
    }

    fn connection(&self) -> &Connection {
        self.database.connection()
    }

    /// The whole wall as it stands, newest change first — what the window draws.
    pub fn notes(&self) -> StoreResult<Vec<NoteDto>> {
        let _held = self.database.hold();
        let notes = match self.family() {
            Some(scoped) => self
                .connection()
                .prepare_cached(
                    "SELECT * FROM notes WHERE family_id = $family ORDER BY board_seq DESC",
                )?
                .query_map(named_params! { "$family": scoped }, read)?
                .collect::<Result<Vec<_>, _>>()?,
            None => self
                .connection()
                .prepare_cached("SELECT * FROM notes ORDER BY board_seq DESC")?
                .query_map([], read)?
                .collect::<Result<Vec<_>, _>>()?,
        };
        Ok(notes)
    }

    /// One note, as this device holds it.
    pub fn note(&self, note_id: i64) -> StoreResult<Option<NoteDto>> {
        let _held = self.database.hold();
        Ok(self
            .connection()
            .prepare_cached("SELECT * FROM notes WHERE note_id = $id")?
            .query_row(named_params! { "$id": note_id }, read)
            .optional()?)
    }

    /// A whole-board read: what arrived IS the wall. Notes the answer does not carry are gone —
    /// somebody took them down while this device was not looking.
    pub fn replace(&self, notes: &[NoteDto], max_board_seq: i64) -> StoreResult<()> {
        let _held = self.database.hold();
        // TWO FULL READS CAN LAND IN EITHER ORDER, and an older one landing second must change
        // nothing at all: it would drop every note written between the two and set the cursor
        // back to its own mark.
        if max_board_seq < number(meta(&self.database, &self.key(FULL_MARK))?.as_deref()) {
            return Ok(());
        }
        let transaction = self.connection().unchecked_transaction()?;
        // EXCEPT what is NEWER THAN THE READ'S OWN MARK: a frame that landed while the read was in
        // flight is not on that answer, and wiping it would take a note off the wall seconds after
        // somebody pinned it — then put it back at the next catch-up. And only this family's:
        // another family's wall has seqs of its own.
        match self.family() {
            Some(scoped) => {
                transaction
                    .prepare_cached(
                        "DELETE FROM notes WHERE board_seq <= $mark AND family_id = $family",
                    )?
                    .execute(named_params! { "$mark": max_board_seq, "$family": scoped })?;
            }
            None => {
                transaction
                    .prepare_cached("DELETE FROM notes WHERE board_seq <= $mark")?
                    .execute(named_params! { "$mark": max_board_seq })?;
            }
        }
        for note in notes.iter().filter(|note| !note.deleted) {
            if !is_gone(&transaction, note.id)? {
                self.write(&transaction, note)?;
            }
        }
        set_meta(&transaction, &self.key(CURSOR_SEQ), max_board_seq)?;
        set_meta(&transaction, &self.key(FULL_MARK), max_board_seq)?;
        transaction.commit()?;
        Ok(())
    }

    /// One note from the change feed or from an answer to this device's own write. A TOMBSTONE
    /// deletes; anything older than what is held is dropped. Answers whether anything changed.
    /// (The C#'s default route: `CatchUpPage`.)
    pub fn apply(&self, note: &NoteDto, route: SeqRoute) -> StoreResult<bool> {
        let _held = self.database.hold();
        let transaction = self.connection().unchecked_transaction()?;
        let changed = self.apply_one(&transaction, note)?;
        // A FRAME MOVES THE CURSOR ONLY ONCE THIS DEVICE HAS READ THE BOARD. Before that the
        // cursor is 0, which is what tells the resync to read the WHOLE wall — and a frame that
        // jumped that queue would leave the cursor above changes nobody had read, so the wall
        // would be whatever frames happened to arrive (docs/protocol.md, "The board cursor moves
        // in three ways and no others").
        let may_move = match route {
            SeqRoute::Evidence => false,
            SeqRoute::LiveFrame => self.cursor()? != 0,
            SeqRoute::CatchUpPage => true,
        };
        if may_move && note.board_seq > self.cursor()? {
            set_meta(&transaction, &self.key(CURSOR_SEQ), note.board_seq)?;
        }
        transaction.commit()?;
        Ok(changed)
    }

    /// A page of the change feed, applied in order. Answers how many notes changed.
    pub fn apply_page(&self, notes: &[NoteDto], route: SeqRoute) -> StoreResult<usize> {
        let _held = self.database.hold();
        let transaction = self.connection().unchecked_transaction()?;
        let mut changed = 0;
        let mut highest = self.cursor()?;
        for note in notes {
            if self.apply_one(&transaction, note)? {
                changed += 1;
            }
            highest = highest.max(note.board_seq);
        }
        if route != SeqRoute::Evidence {
            set_meta(&transaction, &self.key(CURSOR_SEQ), highest)?;
        }
        transaction.commit()?;
        Ok(changed)
    }

    fn apply_one(&self, connection: &Connection, note: &NoteDto) -> StoreResult<bool> {
        if note.deleted {
            // Remembered as well as removed: a note is taken down once and for all, and an older
            // copy of it is still travelling.
            connection
                .prepare_cached("INSERT OR IGNORE INTO gone (note_id) VALUES ($id)")?
                .execute(named_params! { "$id": note.id })?;
            // Whether the WALL changed, which a tombstone for a note this device never held did
            // not — remembering it is not news.
            let deleted = connection
                .prepare_cached("DELETE FROM notes WHERE note_id = $id")?
                .execute(named_params! { "$id": note.id })?;
            return Ok(deleted > 0);
        }
        if is_gone(connection, note.id)? {
            // A tombstone is the last word. This is an older copy that crossed it on the wire, and
            // writing it would put a note the family took down back on the wall.
            return Ok(false);
        }
        let current: Option<Option<i64>> = connection
            .prepare_cached("SELECT board_seq FROM notes WHERE note_id = $id")?
            .query_row(named_params! { "$id": note.id }, |row| row.get(0))
            .optional()?;
        if current.flatten().is_some_and(|held| held >= note.board_seq) {
            // A slower answer that crossed a newer one on the wire. Seqs commit out of order, so
            // this is ordinary rather than exceptional.
            return Ok(false);
        }
        self.write(connection, note)?;
        Ok(true)
    }

    /// Where the change feed should carry on from.
    pub fn cursor(&self) -> StoreResult<i64> {
        Ok(number(
            meta(&self.database, &self.key(CURSOR_SEQ))?.as_deref(),
        ))
    }

    /// What this DEVICE has shown its reader — not the sync cursor, and moved only when the board
    /// has actually been on screen (docs/protocol.md, "Board").
    pub fn marks(&self) -> StoreResult<Marks> {
        let _held = self.database.hold();
        Ok(Marks {
            note_id: number(meta(&self.database, &self.key(MARK_NOTE_ID))?.as_deref()),
            content_seq: number(meta(&self.database, &self.key(MARK_CONTENT_SEQ))?.as_deref()),
        })
    }

    /// Monotonic in both fields: a cleared badge must not come back.
    pub fn mark(&self, marks: Marks) -> StoreResult<()> {
        let _held = self.database.hold();
        let later = later(self.marks()?, marks);
        let transaction = self.connection().unchecked_transaction()?;
        set_meta(&transaction, &self.key(MARK_NOTE_ID), later.note_id)?;
        set_meta(&transaction, &self.key(MARK_CONTENT_SEQ), later.content_seq)?;
        transaction.commit()?;
        Ok(())
    }

    /// The marks after the board has been on screen, applied.
    pub fn mark_shown(&self) -> StoreResult<()> {
        // Read and write as ONE operation: a note landing between them would be marked unseen.
        let _held = self.database.hold();
        let shown = marks_after_showing(seen(&self.notes()?), self.marks()?);
        self.mark(shown)
    }

    /// How many notes have something new to READ — the badge.
    pub fn unread(&self) -> StoreResult<usize> {
        let _held = self.database.hold();
        Ok(board_badge::count(seen(&self.notes()?), self.marks()?))
    }

    // ---- rows ------------------------------------------------------------------------------

    fn write(&self, connection: &Connection, note: &NoteDto) -> StoreResult<()> {
        connection
            .prepare_cached(
                "INSERT INTO notes (note_id, author_id, kind, text, color, size, font, x, y,
                   created_at, updated_at, board_seq, content_seq,
                   starts_at, ends_at, place,
                   attachment_json, rsvps_json, mentions_json, items_json, family_id)
VALUES ($id, $author, $kind, $text, $color, $size, $font, $x, $y,
        $created, $updated, $seq, $content,
        $starts, $ends, $place,
        $attachment, $rsvps, $mentions, $items, $family)
ON CONFLICT(note_id) DO UPDATE SET
    author_id = excluded.author_id, kind = excluded.kind, text = excluded.text,
    color = excluded.color, size = excluded.size, font = excluded.font,
    x = excluded.x, y = excluded.y, updated_at = excluded.updated_at,
    board_seq = excluded.board_seq, content_seq = excluded.content_seq,
    starts_at = excluded.starts_at, ends_at = excluded.ends_at, place = excluded.place,
    attachment_json = excluded.attachment_json, rsvps_json = excluded.rsvps_json,
    mentions_json = excluded.mentions_json, items_json = excluded.items_json,
    family_id = COALESCE(excluded.family_id, notes.family_id)",
            )?
            .execute(named_params! {
                "$id": note.id,
                "$author": note.author_id,
                // An unknown kind, colour, size or face is kept AS SENT and drawn at the fallback:
                // a fourth size from a newer server must not be written back as medium by an edit
                // (docs/protocol.md, "Board").
                "$kind": note.kind.as_deref().unwrap_or("text"),
                "$text": note.text.as_deref().unwrap_or(""),
                "$color": note.color.as_deref().unwrap_or("yellow"),
                "$size": note.size.as_deref().unwrap_or("medium"),
                "$font": note.font.as_deref().unwrap_or("plain"),
                "$x": note.x,
                "$y": note.y,
                "$created": times::instant(note.created_at.as_deref()).unwrap_or(0),
                "$updated": times::instant(note.updated_at.as_deref()).unwrap_or(0),
                "$seq": note.board_seq,
                "$content": note.content_seq.unwrap_or(0),
                "$starts": times::instant(note.starts_at.as_deref()),
                "$ends": times::instant(note.ends_at.as_deref()),
                "$place": note.place,
                "$attachment": note.attachment.as_ref().map(wire::encode),
                // `[]` and ABSENT are different answers and both are kept: an event nobody has
                // answered sends `[]`, and every other kind sends nothing at all.
                "$rsvps": note.rsvps.as_ref().map(wire::encode),
                "$mentions": note.mentions.as_ref().map(wire::encode),
                "$items": note.items.as_ref().map(wire::encode),
                // Whose wall: the family this store is held to, else what the note says
                // (docs/protocol.md, "Several families").
                "$family": self.family().or(note.family_id),
            })?;
        Ok(())
    }
}

/// Each note as the badge reads it: its id, and its content seq — 0 for "the server never said".
fn seen(notes: &[NoteDto]) -> Vec<(i64, Option<i64>)> {
    notes
        .iter()
        .map(|note| (note.id, Some(note.content_seq.unwrap_or(0))))
        .collect()
}

fn is_gone(connection: &Connection, note_id: i64) -> StoreResult<bool> {
    Ok(connection
        .prepare_cached("SELECT 1 FROM gone WHERE note_id = $id")?
        .query_row(named_params! { "$id": note_id }, |_| Ok(()))
        .optional()?
        .is_some())
}

fn read(row: &Row<'_>) -> rusqlite::Result<NoteDto> {
    let text = |column: &str| row.get::<_, Option<String>>(column);
    let json = |column: &str| -> rusqlite::Result<Option<String>> { text(column) };
    let content_seq: i64 = row.get("content_seq")?;
    Ok(NoteDto {
        id: row.get("note_id")?,
        author_id: row.get("author_id")?,
        kind: text("kind")?,
        text: Some(text("text")?.unwrap_or_default()),
        color: text("color")?,
        size: text("size")?,
        font: text("font")?,
        x: row.get("x")?,
        y: row.get("y")?,
        created_at: times::rfc3339(row.get("created_at")?),
        updated_at: times::rfc3339(row.get("updated_at")?),
        board_seq: row.get("board_seq")?,
        // 0 is this table's spelling of "the server never said", which is an ABSENT content_seq
        // on the wire — and the badge then judges the note by its id.
        content_seq: (content_seq != 0).then_some(content_seq),
        attachment: json("attachment_json")?.and_then(|json| wire::decode::<AttachmentDto>(&json)),
        starts_at: times::rfc3339(row.get("starts_at")?),
        ends_at: times::rfc3339(row.get("ends_at")?),
        place: text("place")?,
        rsvps: json("rsvps_json")?.and_then(|json| wire::decode::<Vec<RsvpDto>>(&json)),
        items: json("items_json")?.and_then(|json| wire::decode::<Vec<TaskItemDto>>(&json)),
        mentions: json("mentions_json")?.and_then(|json| wire::decode::<Vec<MentionDto>>(&json)),
        deleted: false,
        family_id: row.get("family_id")?,
    })
}

// ---- meta: shared with the pack store, which is this one table over ----------------------------

/// A number this cache wrote for ITSELF, read back the same way it was written: .NET's
/// `long.TryParse(text, NumberStyles.Integer, InvariantCulture)` — ASCII digits with an optional
/// sign and white space around them — and 0 for anything else, absence included. A machine's
/// number is invariant in both directions: the reader's culture decides how a number is SHOWN, and
/// has no business in a value nobody reads.
pub(super) fn number(text: Option<&str>) -> i64 {
    text.map(|text| text.trim_matches(|c: char| matches!(c, '\t'..='\r' | ' ')))
        .and_then(|text| text.parse().ok())
        .unwrap_or(0)
}

/// One `meta` value, or `None`.
pub(super) fn meta(database: &Database, key: &str) -> StoreResult<Option<String>> {
    let _held = database.hold();
    let value: Option<Option<String>> = database
        .connection()
        .prepare_cached("SELECT value FROM meta WHERE key = $key")?
        .query_row(named_params! { "$key": key }, |row| row.get(0))
        .optional()?;
    Ok(value.flatten())
}

/// One `meta` value, written — a number, in the invariant spelling Rust writes an `i64` in.
pub(super) fn set_meta(connection: &Connection, key: &str, value: i64) -> StoreResult<()> {
    connection
        .prepare_cached(
            "INSERT INTO meta (key, value) VALUES ($key, $value) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )?
        .execute(named_params! { "$key": key, "$value": value.to_string() })?;
    Ok(())
}
