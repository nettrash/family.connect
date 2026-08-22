-- 0010_attachment_files — the third attachment kind.
--
-- A file is not a photo without pixels: its NAME is its whole identity.
-- Nobody recognises "attachment 34", and a document with no name is a
-- document nobody will open, so `name` is required for kind='file' and
-- meaningless for the other two (a photo already renders itself).
--
-- The CHECK constraint carries that rule rather than the handler alone,
-- because it is the kind of invariant that quietly rots when a second
-- write path appears — a future import tool, a migration, a fix applied
-- by hand at 1am.
--
-- Files are the one kind whose bytes the server does NOT check against a
-- magic number: a family sending each other documents should never be
-- told their file is not allowed, and a fixed list would refuse exactly
-- the things a particular family lives on (docs/protocol.md, "Files").
-- What makes that safe is the DOWNLOAD, not the upload: a file is served
-- Content-Disposition: attachment with X-Content-Type-Options: nosniff,
-- so nothing uploaded here can render or execute from the family
-- server's own origin.

ALTER TABLE attachments ADD COLUMN name TEXT;

ALTER TABLE attachments DROP CONSTRAINT attachments_kind_check;
ALTER TABLE attachments ADD CONSTRAINT attachments_kind_check
    CHECK (kind IN ('photo', 'video', 'file'));

ALTER TABLE attachments ADD CONSTRAINT attachments_file_has_name
    CHECK (kind <> 'file' OR (name IS NOT NULL AND length(name) BETWEEN 1 AND 255));
