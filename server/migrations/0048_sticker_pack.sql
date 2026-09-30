-- 0048_sticker_pack — one pack of chat stickers per family
-- (docs/protocol.md, "Sticker pack").
--
-- THE NAME FIRST. "Sticker" already means a board NOTE in this codebase, so
-- the collection is `pack` everywhere it is a table, a column or a cursor.
-- The one column spelled `sticker` is the flag on an attachment, below,
-- which is the word the wire uses for it.
--
-- A pack item is the BOARD'S SHAPE, deliberately: its own server-wide
-- sequence, a per-family high-water mark, and a row that survives its
-- removal as a tombstone so a client that was away learns it is gone. No
-- client learns a second sync idea.
--
-- ON DELETE CASCADE from the family: the pack is the family's property and
-- goes when the family does. From the user too, for the constraint's sake
-- only — an account is scrubbed and never deleted, which is exactly why a
-- departed member's items stay and the OWNER may remove them.

CREATE SEQUENCE family_pack_seq;

CREATE TABLE pack_items (
    id         BIGSERIAL   PRIMARY KEY,
    family_id  BIGINT      NOT NULL REFERENCES families(id) ON DELETE CASCADE,
    added_by   BIGINT      NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    -- A few words for a screen reader; never drawn over the picture.
    label      TEXT        CHECK (label IS NULL OR length(label) BETWEEN 1 AND 64),
    pack_seq   BIGINT      NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at TIMESTAMPTZ
);

-- The catch-up feed, and the live pack (the ceiling count and the full read).
CREATE INDEX pack_items_family_pack_seq_idx ON pack_items (family_id, pack_seq);
CREATE INDEX pack_items_family_live_idx ON pack_items (family_id) WHERE deleted_at IS NULL;

ALTER TABLE families ADD COLUMN last_pack_seq BIGINT NOT NULL DEFAULT 0;

-- The picture is claimed the way a message and a board note claim one
-- (0042): a nullable `pack_item_id` on the attachment, plus a partial
-- unique index so one upload is at most one item. NOT a column on
-- `pack_items`, so the three claims are the same shape.
--
-- ON DELETE CASCADE from the item: an item's row survives its removal as a
-- tombstone, so this fires only when the whole FAMILY goes — the handler
-- removes a removed item's picture explicitly.
--
-- THE SWEEPER IS THE TRAP, AGAIN. `sweep_unclaimed` deletes every
-- attachment with neither a `message_id` nor a `note_id` after the grace
-- period; a pack picture has neither and would be eaten hours after it was
-- added. Its predicate learns `pack_item_id` in the same change as this
-- column — and so do the two claims and the account scrub, which ask the
-- same question.
ALTER TABLE attachments ADD COLUMN pack_item_id BIGINT REFERENCES pack_items(id) ON DELETE CASCADE;

-- One picture, one item — the twin of attachments_note_id_uq, and the index
-- the hydration path reads by.
CREATE UNIQUE INDEX attachments_pack_item_id_uq ON attachments (pack_item_id)
    WHERE pack_item_id IS NOT NULL;

-- A message's picture SENT AS A STICKER. On the attachment rather than the
-- message because that is where the wire carries it, and because it is a
-- fact about how one picture is drawn. NOT NULL DEFAULT false: no picture
-- ever sent before this was a sticker, so the default is the truth about
-- the past.
ALTER TABLE attachments ADD COLUMN sticker BOOLEAN NOT NULL DEFAULT false;
