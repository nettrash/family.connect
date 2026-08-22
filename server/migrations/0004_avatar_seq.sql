-- 0004_avatar_seq — stop profile-picture versions from being reused.
--
-- 0003 bumped `avatar_version` on upload and reset it to 0 on delete, so
-- upload → delete → upload handed out version 1 twice for two different
-- pictures. Clients are told they may cache `(user_id, avatar_version)`
-- forever (protocol.md, "Profile pictures"), so the second upload would
-- never be seen by anyone who had cached the first.
--
-- `avatar_seq` is the counter that only ever goes up; `avatar_version`
-- stays exactly what it was on the wire — the current seq while a picture
-- exists, 0 while none does.

ALTER TABLE users ADD COLUMN avatar_seq BIGINT NOT NULL DEFAULT 0;

-- Anyone who already has a picture keeps its version: their next upload
-- must land above it, not back at 1.
UPDATE users SET avatar_seq = avatar_version WHERE avatar_version > 0;
