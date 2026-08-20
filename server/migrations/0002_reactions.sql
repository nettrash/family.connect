-- 0002_reactions — emoji reactions on messages.
--
-- One reaction per (message, user): picking a different emoji replaces the
-- row, picking the same one deletes it. Every mutation of a message's
-- reactions takes the next value of message_reaction_seq and stamps it on
-- the message (and, GREATEST-guarded, on the chat), giving clients a
-- monotonic catch-up cursor exactly as message ids drive after_id.

CREATE TABLE message_reactions (
    message_id BIGINT      NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    user_id    BIGINT      NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    emoji      TEXT        NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (message_id, user_id)
);

CREATE SEQUENCE message_reaction_seq;

ALTER TABLE messages ADD COLUMN reaction_seq BIGINT NOT NULL DEFAULT 0;
CREATE INDEX messages_chat_reaction_seq_idx ON messages (chat_id, reaction_seq)
    WHERE reaction_seq > 0;

ALTER TABLE chats ADD COLUMN last_reaction_seq BIGINT NOT NULL DEFAULT 0;
