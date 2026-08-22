-- 0008_board — the family board: a wall of sticker notes.
--
-- One board per family, so the board IS the family's note rows; there is no
-- separate board table to own.
--
-- The third catch-up cursor, and the same machinery as reactions (0002) and
-- edits (0007): `after_id` is `WHERE id > cursor` and can never see a change
-- to an older row — and a board is nothing BUT changes to older rows, since
-- moving a note is its whole point.
--
-- DELETES LEAVE TOMBSTONES. `deleted_at` is set and the row keeps its id and
-- takes a new board_seq, so the change feed can tell a client who was
-- offline that a note is gone. Nothing else in this schema would say so —
-- an absent row is indistinguishable from one the client never fetched.
--
-- x/y are fractions of the board (0..1), not pixels, so a note sits in the
-- same relative place whatever the screen. DOUBLE PRECISION rather than a
-- fixed grid: a drag is continuous, and rounding it to a grid server-side
-- would make notes jump under the finger.

CREATE SEQUENCE family_board_seq;

CREATE TABLE notes (
    id         BIGSERIAL   PRIMARY KEY,
    family_id  BIGINT      NOT NULL REFERENCES families(id) ON DELETE CASCADE,
    author_id  BIGINT      NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    text       TEXT        NOT NULL,
    color      TEXT        NOT NULL,
    x          DOUBLE PRECISION NOT NULL,
    y          DOUBLE PRECISION NOT NULL,
    board_seq  BIGINT      NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at TIMESTAMPTZ
);

-- The change feed's access path: everything for one family, in seq order.
CREATE INDEX notes_family_board_seq_idx ON notes (family_id, board_seq);
-- The full-board read skips tombstones, which are a small minority.
CREATE INDEX notes_family_live_idx ON notes (family_id) WHERE deleted_at IS NULL;

ALTER TABLE families ADD COLUMN last_board_seq BIGINT NOT NULL DEFAULT 0;
