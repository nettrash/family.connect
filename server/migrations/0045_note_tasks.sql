-- 0045_note_tasks — the fourth kind of note: something the family has to
-- get done (docs/protocol.md, "Board").
--
-- The items are their own table because they are a LIST: ordered, added to,
-- rewritten and ticked one at a time. `id` is what a tick refers to, and it
-- is a bigserial rather than a position for the reason the protocol gives —
-- a position moves under somebody's finger the moment the author inserts a
-- line above it, and a tick that arrived a second later would land on the
-- wrong line.
--
-- `position` carries the AUTHOR's order and is rewritten wholesale with the
-- list; it is not unique, because a replacement writes the new order in one
-- statement per row and two rows may hold the same number for the length of
-- that transaction. The read orders by (position, id), so a list whose
-- positions somehow collided still draws in a stable order rather than a
-- different one per query.
--
-- Done is `done_at IS NOT NULL`, not a boolean: the moment is worth keeping
-- and the flag is derivable from it, where the reverse is not.
--
-- The CHECK is the one-directional half of that: an item nobody has done
-- has nobody who did it, so `done_by` without `done_at` is a state the
-- table refuses. The other direction is allowed on purpose — a deleted
-- account's tick keeps the item done and loses the name (see the SET NULL
-- below), which is exactly `done_at` without `done_by`.
--
-- ON DELETE CASCADE from the note, like the RSVPs: a tombstoned note keeps
-- its row, so this fires only when the family goes. From the user it is SET
-- NULL instead, and that is the one place this table differs from
-- note_rsvps: an answer is a fact ABOUT a member and goes with them, while
-- a tick is a fact about the ITEM — the milk was still bought — so the item
-- stays done and forgets who did it.

CREATE TABLE note_task_items (
    id         BIGSERIAL   PRIMARY KEY,
    note_id    BIGINT      NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    position   INTEGER     NOT NULL,
    text       TEXT        NOT NULL,
    done_by    BIGINT      REFERENCES users(id) ON DELETE SET NULL,
    done_at    TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT note_task_items_done_by_needs_done CHECK (done_at IS NOT NULL OR done_by IS NULL)
);

-- The hydration path: every item on one list, in the author's order.
CREATE INDEX note_task_items_note_idx ON note_task_items (note_id, position, id);
