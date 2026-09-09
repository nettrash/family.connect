-- 0042_photo_notes — a note can be a picture (docs/protocol.md, "Board").
--
-- A KIND on the note, and a second way for an attachment to be claimed.
--
-- `kind` is TEXT with the vocabulary in the server (`Note::KINDS`), like
-- `color`, `size` and `font` before it. NOT NULL DEFAULT 'text', because
-- every note ever written WAS a text note — the default is the truth about
-- the past, and nothing already on a wall changes.
--
-- The picture is claimed the way a message claims one: a nullable
-- `note_id` on the attachment, plus a partial unique index so one picture
-- can be pinned by at most one note. NOT a column on `notes`, so the two
-- claims are the same shape and the sweeper below has one rule to learn.
--
-- ON DELETE CASCADE from the note: a board note's row survives its deletion
-- as a tombstone, so this cascade fires only when the whole FAMILY goes —
-- the handler removes a deleted note's picture explicitly, which is what
-- the protocol promises.
--
-- THE SWEEPER IS THE TRAP. `sweep_unclaimed` deletes every attachment with
-- no `message_id` after the grace period; a picture pinned to a board has
-- none and would be eaten hours after it was pinned, leaving a photo note
-- with nothing to show. Its predicate learns about `note_id` in the same
-- change as this column, or the feature ships broken with a delay fuse.

ALTER TABLE notes ADD COLUMN kind TEXT NOT NULL DEFAULT 'text';

ALTER TABLE attachments ADD COLUMN note_id BIGINT REFERENCES notes(id) ON DELETE CASCADE;

-- One picture, one note — the twin of attachments_message_id_uq, which is
-- now a partial index for the same reason: most attachments have neither.
CREATE UNIQUE INDEX attachments_note_id_uq ON attachments (note_id)
    WHERE note_id IS NOT NULL;

-- The hydration path: every live note's picture in one join.
CREATE INDEX attachments_note_idx ON attachments (note_id) WHERE note_id IS NOT NULL;
