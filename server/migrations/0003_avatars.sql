-- 0003_avatars — profile pictures.
--
-- Bytes live in their own table, never on `users`: every SELECT in this
-- server reads whole user rows, and a BYTEA column there would drag a
-- quarter-megabyte through each of them. `users.avatar_version` is the
-- cheap part everybody reads — 0 means "no picture, draw initials", and
-- it doubles as the client cache key, so it is bumped (never reused) on
-- every upload and reset to 0 on delete.
--
-- Postgres rather than the filesystem, chosen deliberately: nothing in
-- this server ever deletes a `users` row, so on-disk blobs would have no
-- natural GC trigger, and the family's backup stays a single pg_dump.

ALTER TABLE users ADD COLUMN avatar_version BIGINT NOT NULL DEFAULT 0;

CREATE TABLE user_avatars (
    user_id      BIGINT      PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    content_type TEXT        NOT NULL CHECK (content_type IN ('image/jpeg', 'image/png')),
    bytes        BYTEA       NOT NULL,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
