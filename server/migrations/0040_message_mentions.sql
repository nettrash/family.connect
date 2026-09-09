-- 0040_message_mentions — a message names members (docs/protocol.md,
-- "Mentioning a member").
--
-- One row per member named, decided at send time and never changed by an
-- edit. `name` is the display name AS TYPED after the "@", so a client can
-- find the token in the body to highlight it without knowing what the
-- member is called today. `position` is the sender's order.
--
-- CASCADE both ways: the list dies with the message (retention), and with
-- the account it names — a scrubbed account is a different row, so this
-- only fires for the rows deletion does remove.
CREATE TABLE message_mentions (
    message_id BIGINT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    user_id    BIGINT NOT NULL REFERENCES users(id)    ON DELETE CASCADE,
    name       TEXT   NOT NULL,
    position   INT    NOT NULL,
    PRIMARY KEY (message_id, user_id)
);

-- The chat-list question — "does an unread message here name me" — is asked
-- by user, over the newest ids first.
CREATE INDEX message_mentions_user_idx ON message_mentions (user_id, message_id DESC);
