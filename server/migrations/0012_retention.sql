-- 0012_retention — let old messages actually be deletable.
--
-- The retention sweep deletes messages past the configured age. Without
-- this migration it would fail on exactly the case that matters most: a
-- long-lived family chat where somebody replied to something. 0005 added
--
--     reply_to_message_id BIGINT REFERENCES messages(id)
--
-- with no ON DELETE clause, which is NO ACTION — so deleting a message
-- that any newer message quotes raises a foreign-key violation and the
-- whole sweep rolls back.
--
-- SET NULL, not CASCADE, and the difference is the whole point: a reply is
-- a message in its own right. Deleting a two-day-old reply because it
-- happens to quote something from four months ago would be destroying
-- current conversation to enforce a policy about old conversation. The
-- reply survives and loses its quote — which the read path already handles,
-- since `reply_to` is recomputed from a LEFT JOIN and simply comes back
-- absent (docs/protocol.md, "Replies").

ALTER TABLE messages DROP CONSTRAINT messages_reply_to_message_id_fkey;
ALTER TABLE messages
    ADD CONSTRAINT messages_reply_to_message_id_fkey
    FOREIGN KEY (reply_to_message_id) REFERENCES messages(id) ON DELETE SET NULL;

-- The sweep's access path: "every message older than the cutoff", across
-- all chats. The existing indexes are per-chat and ordered by id, which
-- answers a different question.
CREATE INDEX messages_created_at_idx ON messages (created_at);
