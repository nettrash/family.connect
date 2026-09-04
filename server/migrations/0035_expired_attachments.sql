-- 0035_expired_attachments — an upload the sweep took leaves a marker
-- (docs/protocol.md, "Attachments").
--
-- `sweep_unclaimed` deletes an upload no message ever claimed once it is
-- past `limits.attachment_grace_hours`. Afterwards the id is simply gone,
-- and the claim path cannot tell three different things apart: an id that
-- never existed, one belonging to somebody else, and one this very client
-- uploaded and had swept out from under it. They need different answers —
-- the first two are a bug or an attack, the third is ordinary on a client
-- whose outbox has been holding a message through a long offline stretch,
-- and the right response to it is to upload the bytes again rather than to
-- fail the message forever.
--
-- A MARKER TABLE rather than a column on `attachments`, and the reason is
-- refcounting: since 0011 several rows may name one storage key, and
-- `remove_if_unreferenced` decides whether to delete a FILE by asking
-- whether any `attachments` row still names its key. A row kept behind for
-- its id would answer "yes" forever and the bytes would never be freed.
-- The tombstone names no key, so it cannot hold a file alive.
--
-- Three small columns and no foreign key to `attachments` (the row it
-- describes is gone by definition). `uploader_id` DOES cascade: an account
-- that is deleted takes its markers with it, exactly as it takes its
-- unclaimed uploads.
CREATE TABLE expired_attachments (
    id          BIGINT      PRIMARY KEY,
    uploader_id BIGINT      NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expired_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The sweep's own path: markers old enough that no client could still be
-- holding the id.
CREATE INDEX expired_attachments_expired_at_idx ON expired_attachments (expired_at);
