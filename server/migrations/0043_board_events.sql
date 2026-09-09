-- 0043_board_events — the third kind of note: something the family is doing
-- (docs/protocol.md, "Board").
--
-- An event is a NOTE. It takes a slot anyone may move, counts against the
-- same ceiling, rides the same feed and the same `board_seq`, and a block
-- hides it the same way — so its fields live on `notes` rather than in a
-- table of their own, and `kind = 'event'` is what makes them meaningful.
-- All three are NULL on every other kind, which is also what every note
-- already pinned gets: they had no times and no place, and still have none.
--
-- starts_at is TIMESTAMPTZ, like every other instant on this wire: a family
-- spread across two countries has to agree on WHEN, and a local time would
-- make "seven" mean two different moments. The handler enforces that
-- `ends_at` is not earlier; a CHECK would be true here too, but the
-- protocol's answer for a bad one is `validation` rather than a 500, and
-- the two must not disagree about which.
--
-- RSVPs are their own table because they are per MEMBER, one answer each,
-- replaced rather than appended: the primary key IS the rule. Anyone in the
-- family may answer — answering is the shared act, like moving — so nothing
-- here is keyed on authorship.
--
-- ON DELETE CASCADE from the note: a tombstoned note keeps its row, so this
-- fires only when the family goes, and the answers go with it. From the
-- user, likewise — a member who leaves takes their answer with them, which
-- is the truth about who is coming.

ALTER TABLE notes ADD COLUMN starts_at TIMESTAMPTZ;
ALTER TABLE notes ADD COLUMN ends_at   TIMESTAMPTZ;
ALTER TABLE notes ADD COLUMN place     TEXT;

CREATE TABLE note_rsvps (
    note_id    BIGINT      NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    user_id    BIGINT      NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    answer     TEXT        NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (note_id, user_id)
);

-- The hydration path: every answer on one event, in a stable order.
CREATE INDEX note_rsvps_note_idx ON note_rsvps (note_id, user_id);
