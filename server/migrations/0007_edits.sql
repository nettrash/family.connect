-- 0006_edits — editing a message's body.
--
-- The same shape as 0002's reaction cursor, and for the same reason:
-- `after_id` is `WHERE id > cursor`, so a catch-up can never see a change to
-- an OLDER row. Every edit therefore takes the next value of message_edit_seq
-- and stamps it on the message (and, GREATEST-guarded, on the chat), giving
-- clients a monotonic cursor to replay what changed while they were away.
--
-- A SEPARATE sequence from message_reaction_seq on purpose: folding the two
-- together would change the shape of an endpoint deployed clients already
-- speak (protocol.md, "Editing").
--
-- edited_at is NULL for a message still in its original form — that absence,
-- not a sentinel timestamp, is what the wire reports as "not edited".

CREATE SEQUENCE message_edit_seq;

ALTER TABLE messages ADD COLUMN edit_seq BIGINT NOT NULL DEFAULT 0;
ALTER TABLE messages ADD COLUMN edited_at TIMESTAMPTZ;
CREATE INDEX messages_chat_edit_seq_idx ON messages (chat_id, edit_seq)
    WHERE edit_seq > 0;

ALTER TABLE chats ADD COLUMN last_edit_seq BIGINT NOT NULL DEFAULT 0;
