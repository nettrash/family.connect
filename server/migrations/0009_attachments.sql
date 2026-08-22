-- 0009_attachments — photos and videos on messages.
--
-- BYTES LIVE ON THE FILESYSTEM, not in this database, and that is a
-- deliberate reversal of the choice 0003 made for avatars. A 256 KiB avatar
-- in BYTEA costs nothing; a 100 MB video means buffering the whole thing in
-- memory on every read and write, TOAST-compressing already-compressed
-- H.264 for no gain, and a pg_dump that grows without bound. What this row
-- holds is the metadata and the id; `storage_key` names the file.
--
-- The operational consequence is real and belongs in the runbook: a
-- database dump is NO LONGER a complete backup. The attachments directory
-- has to be backed up alongside it.
--
-- message_id is NULL until a message claims the attachment, which is what
-- makes the upload a separate step from the send. The partial index below
-- is the sweeper's access path: unclaimed rows past their grace period are
-- deleted, because a send the user abandoned must not leave 100 MB behind
-- forever.

CREATE TABLE attachments (
    id           BIGSERIAL   PRIMARY KEY,
    uploader_id  BIGINT      NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    message_id   BIGINT      REFERENCES messages(id) ON DELETE CASCADE,
    kind         TEXT        NOT NULL CHECK (kind IN ('photo', 'video')),
    mime         TEXT        NOT NULL,
    size_bytes   BIGINT      NOT NULL,
    width        INTEGER,
    height       INTEGER,
    duration_ms  INTEGER,
    storage_key  TEXT        NOT NULL UNIQUE,
    has_preview  BOOLEAN     NOT NULL DEFAULT false,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- One attachment per message, enforced rather than trusted.
CREATE UNIQUE INDEX attachments_message_id_uq ON attachments (message_id)
    WHERE message_id IS NOT NULL;

-- The sweeper's path: unclaimed uploads, oldest first.
CREATE INDEX attachments_unclaimed_idx ON attachments (created_at)
    WHERE message_id IS NULL;
