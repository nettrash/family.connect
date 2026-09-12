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
            owner          INTEGER NOT NULL DEFAULT 0,
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
        // ---- cursors and marks -----------------------------------------
        """
        CREATE TABLE meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        )
        """,
    ];

    /// <summary>
    /// Every step, in order. The index is the version it upgrades FROM, so
    /// <c>All.Count</c> is the schema this build expects.
    /// </summary>
    public static readonly IReadOnlyList<string[]> All = [One];
}
