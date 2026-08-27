-- 0025_attachment_sets — several attachments on one message (docs/protocol.md,
-- "Photos, videos, audio, files and locations").
--
-- For a long time a message carried at most ONE attachment, and 0009 made
-- that a database fact: a unique partial index on message_id. A family sends
-- albums, and ten separate bubbles for one beach afternoon was the wire being
-- honest at the reader's expense — so the protocol now allows up to ten per
-- message, and the index that forbade the second one has to go. The CEILING
-- is not enforced here: it is `limits.max_attachments_per_message`, an
-- operator setting, and a CHECK constraint cannot read a config file. The
-- write path refuses an eleventh id before anything is claimed.
--
-- ORDER IS THE SENDER'S, and it has to be stored: the protocol promises that
-- the `attachment_ids` array comes back in exactly the order it was sent,
-- and neither `id` nor `created_at` can promise that — a client may upload
-- its photos in parallel and finish them in any order, then send the array
-- in the order the sender arranged them. `position` is the index into that
-- array, stamped at claim time. SMALLINT because the ceiling is ten and an
-- operator raising it a hundredfold still fits; DEFAULT 0 because every
-- existing claimed attachment IS its message's whole set, at position zero,
-- which makes the backfill free.
--
-- attachments_message_idx is the hydration path: every message read now
-- fills its attachment list with `WHERE message_id = ANY($page) ORDER BY
-- message_id, position, id`, the same after-the-fact shape polls and
-- reactions use, and this index answers it in order. `id` rides at the end
-- as a tiebreak so the order is total even if a bug ever stamped two rows
-- with one position — reads must never depend on a uniqueness this index no
-- longer enforces. Partial, like the index it replaces: unclaimed uploads
-- have no message to be listed under.
--
-- Nothing else moves. Claiming is still one UPDATE per row inside the
-- message's transaction (all-or-nothing exactly as before, extended to the
-- set), the sweeper still deletes per row, and the 0011 invariant — a FILE
-- is removed only when the LAST row naming its storage_key is gone — is
-- untouched by any of this.

DROP INDEX attachments_message_id_uq;

ALTER TABLE attachments ADD COLUMN position SMALLINT NOT NULL DEFAULT 0;

CREATE INDEX attachments_message_idx ON attachments (message_id, position, id)
    WHERE message_id IS NOT NULL;
