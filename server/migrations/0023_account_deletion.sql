-- 0023_account_deletion — a deleted account is SCRUBBED, not deleted
-- (docs/protocol.md, "Deleting an account").
--
-- `DELETE FROM users` is not available here and never was. `messages.sender_id`
-- is `ON DELETE RESTRICT` (0001) and would refuse outright; `notes.author_id`
-- and `message_reactions.user_id` would cascade and take the family's board
-- and its reactions with them. That refusal is not an obstacle to route
-- around — it is the design. A family chat is a shared record, and destroying
-- one member's half of it punches holes in everybody else's: replies answer
-- messages that are no longer there, a photo everybody remembers is gone, and
-- a conversation reads as a monologue. Nobody else consented to losing that.
-- So the ROW SURVIVES, emptied of everything that identifies the person —
-- username, password, picture, birthday, sessions, devices — and every
-- foreign key that names it goes on resolving. `deleted_at` is what marks it,
-- and its presence is the whole flag: the wire's `"deleted": true` is
-- `deleted_at IS NOT NULL` and nothing else.
--
-- FAMILY_ID GOES NULL, and that single write is what makes a tombstone
-- invisible to code that has not heard of tombstones. Every "is this user in
-- my family" question in this server is already `family_id = $1` — the
-- roster, the direct-chat check, `require_same_family`, the statistics, the
-- member count — so a scrubbed account drops out of all of them with no
-- change to any of them, rather than each growing an `AND deleted_at IS NULL`
-- that one of them would eventually forget.
--
-- `deleted_family_id` remembers where they were, and it exists for exactly
-- one reader: `GET /families/mine`, which returns them in `former_members` so
-- a client can still put a name to the messages, notes and reactions they
-- left behind. It is not a membership — nothing else may read it as one — and
-- `ON DELETE SET NULL` because a family that is itself deleted has no
-- former members left to name. The index is partial because the column is
-- NULL for every live account, which is very nearly every row in the table.
--
-- THE TWO INDEXES BELOW ARE THE REFERENCING SIDES POSTGRESQL NEVER INDEXES,
-- and they are here because scrubbing an account deletes rows in bulk:
--
--   * `messages.sender_id` has no leading index at all — `messages_dedup_uq`
--     starts with `chat_id` and `messages_chat_id_id_idx` likewise, so
--     nothing in this schema can look a sender up. Deleting a direct chat
--     (both halves go, per the protocol) cascades to its messages one row at
--     a time, and the assistant thread goes the same way;
--   * `ai_usage.message_id` is `ON DELETE SET NULL`, which fires once per
--     deleted message rather than once per delete — so every message removed
--     with a direct chat, and every message removed by the retention sweep,
--     currently makes PostgreSQL scan `ai_usage` end to end to find the rows
--     to blank.
--
-- That is the same quadratic pathology 0006_reply_index.sql was written to
-- remove, measured there at ~485x on 60k messages, and it is worth stating
-- plainly that both of these were already costing the retention sweep before
-- account deletion existed — this migration is where they were noticed, not
-- where they were introduced. `ai_usage_message_id_idx` is partial because
-- the column is NULL on every row whose message has already gone.

ALTER TABLE users ADD COLUMN deleted_at        TIMESTAMPTZ;
ALTER TABLE users ADD COLUMN deleted_family_id BIGINT REFERENCES families(id) ON DELETE SET NULL;
CREATE INDEX users_deleted_family_idx ON users (deleted_family_id)
    WHERE deleted_family_id IS NOT NULL;
CREATE INDEX messages_sender_id_idx ON messages (sender_id);
CREATE INDEX ai_usage_message_id_idx ON ai_usage (message_id) WHERE message_id IS NOT NULL;
