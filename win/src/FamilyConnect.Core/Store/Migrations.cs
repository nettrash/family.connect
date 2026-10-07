namespace FamilyConnect.Core.Store;

/// <summary>
/// The cache's schema, one numbered step at a time.
/// </summary>
/// <remarks>
/// <para>
/// APPEND ONLY. A step that has shipped is never edited — an installed file has already run it,
/// and editing it means two machines with the same <c>user_version</c> and different schemas.
/// A change is a NEW step, and <see cref="Database.SchemaVersion"/> follows the count.
/// </para>
/// <para>
/// Nothing here mirrors the wire's shape for its own sake: the columns are what a window needs to
/// draw without asking the server first, plus the outbox, which is the one table that holds
/// something the server has never seen.
/// </para>
/// </remarks>
public static class Migrations
{
    /// <summary>Step 1: the first schema — chats, messages, the board, the outbox, cursors.</summary>
    private static readonly string[] One =
    [
        // ---- who -----------------------------------------------------
        """
        CREATE TABLE members (
            user_id        INTEGER PRIMARY KEY,
            username       TEXT    NOT NULL,
            display_name   TEXT    NOT NULL,
            avatar_version INTEGER NOT NULL DEFAULT 0,
            -- The wire's own word: "owner" or "member", and NULL for a former member, who has
            -- no role at all (docs/protocol.md, "Objects"). Kept as the string rather than
            -- flattened to a flag so that "not in the family any more" stays distinguishable
            -- from "in it, not the owner".
            role           TEXT,
            -- A member who LEFT and an account that was DELETED both keep their row: their
            -- messages and their notes keep their authors, and a name has to resolve for an old
            -- message (docs/protocol.md, "Objects").
            has_left       INTEGER NOT NULL DEFAULT 0,
            deleted        INTEGER NOT NULL DEFAULT 0,
            birthday_month INTEGER,
            birthday_day   INTEGER
        )
        """,
        // ---- where -----------------------------------------------------
        """
        CREATE TABLE chats (
            chat_id              INTEGER PRIMARY KEY,
            kind                 TEXT    NOT NULL,
            title                TEXT    NOT NULL,
            peer_user_id         INTEGER,
            -- Whether the last list read carried this chat. A direct chat with somebody the
            -- reader has BLOCKED is not listed, for the blocker alone, and **comes back whole on
            -- unblock — nothing about it is deleted** (docs/protocol.md, "Blocking a member"). So
            -- an absent chat is hidden here rather than removed: deleting the row would take its
            -- messages with it (ON DELETE CASCADE) and an unblock would show an empty chat.
            listed               INTEGER NOT NULL DEFAULT 1,
            unread_count         INTEGER NOT NULL DEFAULT 0,
            -- This caller's own marker, applied monotonically: a response still in flight while
            -- the reader is reading must never walk it backwards.
            last_read_message_id INTEGER NOT NULL DEFAULT 0,
            mentioned            INTEGER NOT NULL DEFAULT 0,
            -- High-water marks that never go back down; absent until something of that kind has
            -- happened in the chat, which is 0 here.
            max_reaction_seq     INTEGER NOT NULL DEFAULT 0,
            max_edit_seq         INTEGER NOT NULL DEFAULT 0,
            max_poll_seq         INTEGER NOT NULL DEFAULT 0,
            last_message_at      INTEGER NOT NULL DEFAULT 0
        )
        """,
        // ---- what was said ---------------------------------------------
        """
        CREATE TABLE messages (
            message_id     INTEGER PRIMARY KEY,
            chat_id        INTEGER NOT NULL REFERENCES chats(chat_id) ON DELETE CASCADE,
            sender_id      INTEGER NOT NULL,
            client_msg_id  TEXT,
            body           TEXT    NOT NULL,
            created_at     INTEGER NOT NULL,
            edited_at      INTEGER,
            edit_seq       INTEGER,
            reply_to_id    INTEGER,
            thread_root_id INTEGER,
            reply_count    INTEGER,
            reaction_seq   INTEGER,
            -- The wire's own JSON, stored verbatim: these are lists the server owns whole and
            -- replaces whole, so parsing them into tables would buy nothing and lose the
            -- difference between an absent field and an empty one.
            reactions_json   TEXT,
            attachments_json TEXT,
            mentions_json    TEXT,
            poll_json        TEXT,
            call_json        TEXT
        )
        """,
        "CREATE INDEX messages_by_chat ON messages(chat_id, message_id DESC)",
        "CREATE INDEX messages_by_thread ON messages(thread_root_id, message_id)",
        // ---- the wall ---------------------------------------------------
        """
        CREATE TABLE notes (
            note_id     INTEGER PRIMARY KEY,
            author_id   INTEGER NOT NULL,
            kind        TEXT    NOT NULL DEFAULT 'text',
            text        TEXT    NOT NULL DEFAULT '',
            color       TEXT    NOT NULL DEFAULT 'yellow',
            size        TEXT    NOT NULL DEFAULT 'medium',
            font        TEXT    NOT NULL DEFAULT 'plain',
            x           REAL    NOT NULL DEFAULT 0,
            y           REAL    NOT NULL DEFAULT 0,
            created_at  INTEGER NOT NULL DEFAULT 0,
            updated_at  INTEGER NOT NULL DEFAULT 0,
            board_seq   INTEGER NOT NULL DEFAULT 0,
            -- 0 spells "the server never said", and then the badge judges the note by its id.
            content_seq INTEGER NOT NULL DEFAULT 0,
            starts_at   INTEGER,
            ends_at     INTEGER,
            place       TEXT,
            -- The picture: a photo note's content, an event's BACKDROP. Verbatim, including
            -- `has_preview`, which decides which bytes a card asks for.
            attachment_json TEXT,
            rsvps_json      TEXT,
            mentions_json   TEXT,
            items_json      TEXT
        )
        """,
        // ---- what has not been said yet ---------------------------------
        """
        CREATE TABLE outbox (
            -- The DEDUP KEY, and the row's identity: the same id sent twice is one message, which
            -- is what makes a repeat safe (docs/protocol.md, "Sending on an unreliable network").
            client_msg_id   TEXT    PRIMARY KEY,
            chat_id         INTEGER NOT NULL,
            body            TEXT    NOT NULL,
            reply_to_id     INTEGER,
            -- The ids already uploaded, kept and REUSED within the grace so a retry pushes only
            -- the remainder; and the local files, so the bytes are still there to push.
            attachment_ids  TEXT,
            pending_files   TEXT,
            -- EVERY file this send's media came from, and this list never shrinks. `pending_files`
            -- is what is still owed and empties as the uploads land, which is exactly the wrong
            -- thing to have when the server answers `attachment_expired`: that means UPLOAD IT
            -- AGAIN, and a row that had forgotten where the bytes came from could only give up.
            staged_files    TEXT,
            poll_json       TEXT,
            mentions_json   TEXT,
            queued_at       INTEGER NOT NULL,
            attempts        INTEGER NOT NULL DEFAULT 0,
            -- NULL means due now: that is a row somebody has just asked to retry.
            next_attempt_at INTEGER,
            -- Set only on a TERMINAL refusal, which is the one case the user is told about.
            failed_code     TEXT
        )
        """,
        "CREATE INDEX outbox_by_chat ON outbox(chat_id, queued_at)",
        """
        -- THE NOTES A TOMBSTONE HAS TAKEN, and they never come back. A delete is the last thing
        -- that happens to a note, but seqs commit out of order and history pages carry the
        -- pre-delete copy — so without this table an older answer crossing the tombstone on the
        -- wire RESURRECTS a note the family took down (the web client keeps the same set; Apple
        -- and Android do not, and that is a known gap there).
        CREATE TABLE gone (
            note_id INTEGER PRIMARY KEY
        )
        """,
        // ---- who this reader will not see ------------------------------
        """
        -- The CALLER'S OWN block list, which is complete state: `GET /me` and
        -- `GET /families/mine` both carry it in full, and the `member_blocked` frame is a
        -- state-set rather than an event (docs/protocol.md, "Blocking a member"). A row here
        -- may name somebody the roster cannot: a blocked member who has since left, or whose
        -- account is gone, and resolving the id to a name is the client's job and may be
        -- impossible. So it is its own table and not a column on `members`.
        CREATE TABLE blocked (
            user_id INTEGER PRIMARY KEY
        )
        """,
        // ---- cursors and marks -----------------------------------------
        """
        CREATE TABLE meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        )
        """,
    ];

    /// <summary>
    /// Step 2: WHICH ROWS ARRIVED IN SEQUENCE. `GET /chats` delivers each chat's newest message trimmed, and step 1 stored
    /// it beside the messages pages deliver — so the catch-up cursor, `max(id)`, became the server's newest id, `after_id`
    /// came back empty, and everything between what the device held and that preview was never asked for
    /// (docs/protocol.md, "Best-effort delivery", step 3). `sequenced` is 1 only for a row a page or a live frame
    /// delivered, and the cursor reads nothing else. The messages older builds cached may already have such holes, and
    /// nothing can find them from here, so they are read again: the cache is derived data, and the outbox — the one table
    /// holding what the server has never seen — is untouched.
    /// </summary>
    private static readonly string[] Two =
    [
        "ALTER TABLE messages ADD COLUMN sequenced INTEGER NOT NULL DEFAULT 0",
        "DELETE FROM messages",
    ];

    /// <summary>
    /// Step 3: THE QUOTE OVER A REPLY, KEPT. Step 1 stored a reply's target id and nothing else,
    /// so every reply read back from here was rebuilt with sender 0 and an empty excerpt — and
    /// since the window draws what the cache holds and never the answer straight off the wire,
    /// EVERY reply on this client was drawn as "Someone" with no words under it. The snapshot is
    /// the server's own JSON, both levels of it, stored the way the reactions and the attachments
    /// are; `reply_to_id` stays, for the rows written before this step and for the thread queries.
    /// The cached messages are read again rather than left half-drawn: the cache is derived data,
    /// and the outbox — the one table holding what the server has never seen — is untouched, which
    /// is the same trade step 2 made.
    /// </summary>
    private static readonly string[] Three =
    [
        "ALTER TABLE messages ADD COLUMN reply_to_json TEXT",
        "DELETE FROM messages",
    ];

    /// <summary>
    /// Step 4: THE FAMILY'S STICKER PACK, kept as the board is kept (docs/protocol.md, "Sticker
    /// pack") — and the one bit a queued send needs to stay a sticker across a relaunch. Nothing
    /// held is read again: every table here is new, and the outbox only gains a column whose
    /// default is what every row already in it means.
    /// </summary>
    private static readonly string[] Four =
    [
        """
        -- A pack ITEM: a picture the family keeps. Not a message and not a board note — "sticker"
        -- elsewhere in this cache's code means the card a note is drawn as, which is why these
        -- tables say `pack`, as the wire does.
        CREATE TABLE pack_items (
            item_id     INTEGER PRIMARY KEY,
            added_by    INTEGER NOT NULL DEFAULT 0,
            -- A few words for a screen reader, when whoever added it gave any; NULL is none.
            label       TEXT,
            created_at  INTEGER NOT NULL DEFAULT 0,
            pack_seq    INTEGER NOT NULL DEFAULT 0,
            -- The picture, verbatim. Its ORIGINAL bytes are what is drawn, whatever
            -- `has_preview` in here says.
            attachment_json TEXT NOT NULL
        )
        """,
        """
        -- THE ITEMS A REMOVAL HAS TAKEN, and they never come back: ids are never reused, so an
        -- older copy arriving late — a frame that crossed the tombstone, a slower full read —
        -- must not put a sticker the family took out back in the panel. The board's `gone`, one
        -- table over.
        CREATE TABLE pack_gone (
            item_id INTEGER PRIMARY KEY
        )
        """,
        """
        -- Which stickers THIS DEVICE sent most recently, so the panel can put them first. Never
        -- on the wire: it says something about a person's habits and nothing about the pack.
        CREATE TABLE pack_recents (
            item_id INTEGER PRIMARY KEY,
            used_at INTEGER NOT NULL
        )
        """,
        // A queued send that is a STICKER: the flag has to survive the app closing, or a sticker
        // tapped offline would land the next morning as a photo in a bubble.
        "ALTER TABLE outbox ADD COLUMN sticker INTEGER NOT NULL DEFAULT 0",
    ];

    /// <summary>
    /// Step 5: THE TEXT OF A RECORDING, KEPT ON THIS DEVICE (docs/protocol.md, "Transcripts on request"). A transcript
    /// is the answer to one member's request and is in no message, page or frame, so nothing else on this device could
    /// draw it again — and asking again would be a second provider call for an answer made from sound this device
    /// supplied. Keyed by the attachment, which never names other bytes. Nothing held is read again.
    /// </summary>
    private static readonly string[] Five =
    [
        """
        CREATE TABLE transcripts (
            attachment_id INTEGER PRIMARY KEY,
            -- "" is SILENCE, an answer drawn as "No speech": never NULL, which would read as "not asked".
            text          TEXT    NOT NULL,
            -- Only when the provider named the language it heard, spelled as it spelled it.
            language      TEXT,
            -- 1 when the answer was made from sound THIS DEVICE sent: the server never kept it, so this row is the only
            -- copy. 0 for one made from the server's stored bytes, which any member the rule allows gets back too.
            supplied      INTEGER NOT NULL DEFAULT 0,
            kept_at       INTEGER NOT NULL DEFAULT 0
        )
        """,
    ];

    /// <summary>
    /// Step 6: VIDEO MESSAGES (docs/protocol.md, "Video messages"; docs/audio-video-messages-2026-10-04.md, S5). The one
    /// bit a queued send needs to stay a circle across a relaunch — step 4's sticker column, one over — and which circles
    /// THIS DEVICE has played, for the dot beside an unplayed one. Nothing held is read again: the outbox gains a column
    /// whose default is what every row already in it means, and the other table is new.
    /// </summary>
    private static readonly string[] Six =
    [
        // A queued send that is a VIDEO MESSAGE: without it a circle recorded offline would land as a square video.
        "ALTER TABLE outbox ADD COLUMN round INTEGER NOT NULL DEFAULT 0",
        """
        -- The video messages THIS DEVICE has played, by attachment: the dot's own knowledge (S5.2). Never on the wire —
        -- whether somebody watched something is theirs — and wiped with the rest of the cache at sign-out, so it is
        -- this account's. Only the newest few thousand are kept (PlayedRoundStore).
        CREATE TABLE played_rounds (
            attachment_id INTEGER PRIMARY KEY
        )
        """,
    ];

    /// <summary>
    /// Step 7: which VOICE MESSAGES this device has played, for the dot beside an unplayed voice bubble (the approved design
    /// of 2026-10-05) — step 6's table for circles, one over, so neither kind pushes the other out. New, so nothing held is
    /// read again.
    /// </summary>
    private static readonly string[] Seven =
    [
        """
        -- The voice messages THIS DEVICE has played, by attachment. Never on the wire, wiped at sign-out with the rest of
        -- the cache, and only the newest few thousand are kept (PlayedVoiceStore).
        CREATE TABLE played_voice (
            attachment_id INTEGER PRIMARY KEY
        )
        """,
    ];

    /// <summary>
    /// Step 8: WHICH CACHED ATTACHMENT SETS KNOW THE STICKER AND VIDEO-MESSAGE FLAGS (docs/audio-video-messages-2026-10-04.md,
    /// S5.8; iOS <c>MessageEntity.attachmentsKnowRound</c>, Android MIGRATION_30_31). A build before #58 decoded every
    /// attachment without <c>sticker</c>, and one before #79 without <c>round</c>, and wrote the set back into
    /// <c>attachments_json</c> with only the fields it knew — so a sticker it cached reads as a photo and a circle as a
    /// square video, FOR GOOD: a chat held in sequence is never paged again, and the catch-up only ever adds. 0 is the truth
    /// for every row already here, because nothing says which build wrote it; every set this build writes is 1, and
    /// <c>Resync</c> reads the 0s that could be either once more (<see cref="ChatStore.FlagRepairCandidates"/>).
    /// </summary>
    private static readonly string[] Eight =
    [
        "ALTER TABLE messages ADD COLUMN attachments_know_flags INTEGER NOT NULL DEFAULT 0",
    ];

    /// <summary>
    /// Every step, in order. The index is the version it upgrades FROM, so
    /// <c>All.Count</c> is the schema this build expects.
    /// </summary>
    public static readonly IReadOnlyList<string[]> All = [One, Two, Three, Four, Five, Six, Seven, Eight];
}
