-- A device that was signed OUT must stop being a push target. This is a
-- security fix and it supersedes one paragraph of 0020's reasoning
-- (docs/protocol.md, "Push notifications").
--
-- 0020 chose ON DELETE SET NULL over CASCADE to protect a working push
-- token: sessions are deleted routinely, and losing the device row with the
-- session looked like it would quietly stop a phone being notified. Put
-- next to the other half of the same change — an unattributed device is
-- read as "push it" — that is what it actually built:
--
--   * logout deletes the session, the FK blanks `session_id`, the row keeps
--     `user_id` and `push_token`, and the signed-out phone becomes a
--     GUARANTEED push target rather than an occasional one;
--   * a password change revokes every other session, so every other device
--     of that person becomes one too;
--   * an owner resetting a member's password — the endpoint whose entire
--     purpose is that "a device somebody else is holding stops working the
--     moment the reset lands" — revokes all of them, and every one of those
--     devices goes on receiving family message bodies on its lock screen,
--     with the sender's name, indefinitely, because nothing else in the
--     server ever deletes a device row.
--
-- So CASCADE, and the sentence 0020 called a cost is the point: a device
-- whose session is gone must not be notified until somebody signs in on it
-- again. What makes that safe is what makes the link possible in the first
-- place — a registration BELONGS to the session it was made from, and
-- clients re-POST /devices on every launch (the protocol requires it), so a
-- device that is legitimately in use gets its row back within seconds of
-- the next launch, with a session that is actually live.
--
-- CASCADE rather than clearing `push_token` on each revocation path,
-- although either would silence the device. There are three places a
-- session is deleted today (logout, change_password, reset_member_password)
-- and a fourth is one feature away; a constraint covers all of them and
-- every future one without anybody remembering. And a row kept with its
-- token cleared is not neutral — it is a `user_id` and a platform pointing
-- at a session that no longer exists, which is the same "cannot say"
-- ambiguity moved into a different column.
--
-- What stays exactly as 0020 built it: a NULL `session_id` still means
-- "this row cannot say" and is still PUSHED. That case is now only what it
-- was always meant to be — a row written before the column existed, which
-- genuinely never told us anything and heals on the next launch.
ALTER TABLE devices DROP CONSTRAINT devices_session_id_fkey;
ALTER TABLE devices
    ADD CONSTRAINT devices_session_id_fkey
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE;

-- The window between 0020 and this migration, closed by hand.
--
-- A session revoked while SET NULL was in force left an orphan that looks
-- exactly like a pre-0020 row, and both are about to be read as "push it".
-- One thing tells them apart: after 0020, EVERY registration writes a
-- session id (handlers_device.rs binds it on both branches), so a row that
-- was created or re-registered after 0020 landed and still has no session
-- can only have been orphaned by the FK. Rows older than that are the
-- honest unknowns and are left alone — silencing those would take the alert
-- away from devices that never did anything wrong.
DELETE FROM devices
 WHERE session_id IS NULL
   AND updated_at >= (SELECT applied_at FROM schema_migrations WHERE version = 20);
