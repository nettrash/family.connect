-- 0011_attachment_dedup — one copy of a file per family.
--
-- Forwarding the same photo into the family chat and two direct chats used
-- to put three copies of it on disk; at 100 MB a video that adds up fast on
-- a box in somebody's hallway. Uploads are now content-addressed: an upload
-- whose bytes a family already holds points at the file that is already
-- there.
--
-- SCOPED TO A FAMILY, deliberately, not global. Sharing bytes between two
-- families would save a little more disk and cost the property that a
-- family's attachments are a self-contained set of files — which is what
-- makes "back up this family", "export it" and "delete it and reclaim the
-- space" simple operations rather than graph problems. Access was always
-- checked per ROW, so this changes nothing about who can read what.
--
-- content_hash is SHA-256 of the stored bytes, hex. It is NULL for every
-- attachment uploaded before this migration: those keep one file each, and
-- backfilling would mean re-reading every blob on the disk at boot. New
-- uploads only dedup against rows that have a hash, so old and new coexist
-- without a rewrite.
--
-- family_id is denormalised from the uploader ON PURPOSE. The uploader's
-- family can change — they leave, they join another one — and if the scope
-- key moved with them, a file two of their old family's messages point at
-- would stop being findable by that family. What matters is which family
-- the bytes were uploaded INTO.
--
-- THE INVARIANT THIS CREATES, which every delete path must now respect:
-- a storage_key may be referenced by more than one row, so the FILE may
-- only be removed once the LAST row referencing it is gone. Removing it
-- with the first would silently break somebody else's message.

-- storage_key was UNIQUE (0009), which is exactly the assumption that
-- breaks: shared bytes mean several rows naming the same file. The plain
-- index below replaces it for lookup.
ALTER TABLE attachments DROP CONSTRAINT attachments_storage_key_key;

ALTER TABLE attachments ADD COLUMN content_hash TEXT;
ALTER TABLE attachments ADD COLUMN family_id BIGINT REFERENCES families(id) ON DELETE SET NULL;

-- The dedup lookup: "does this family already hold these bytes?"
CREATE INDEX attachments_dedup_idx
    ON attachments (family_id, content_hash, size_bytes)
    WHERE content_hash IS NOT NULL;

-- The refcount lookup: "does any other row still point at this file?"
CREATE INDEX attachments_storage_key_idx ON attachments (storage_key);
