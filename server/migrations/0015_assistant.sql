-- A private chat with the assistant, one per member (docs/protocol.md,
-- "The assistant").
--
-- Modelled as an ordinary chat with a third kind rather than a separate
-- table, and that is the whole design: reactions, replies, editing,
-- retention, unread counts, catch-up and paging then work on an assistant
-- reply without a line of new code. The reply IS a message.
--
-- `user_a_id` is its owner and `user_b_id` stays NULL — the assistant has no
-- account. The partial unique index is what makes "one per member" a
-- database fact rather than a rule the handler has to remember.

ALTER TABLE chats DROP CONSTRAINT chats_kind_shape;
ALTER TABLE chats DROP CONSTRAINT chats_kind_check;

ALTER TABLE chats ADD CONSTRAINT chats_kind_check
    CHECK (kind IN ('family', 'direct', 'ai'));

ALTER TABLE chats ADD CONSTRAINT chats_kind_shape CHECK (
    (kind = 'family' AND user_a_id IS NULL AND user_b_id IS NULL)
    OR (kind = 'direct' AND user_a_id IS NOT NULL AND user_b_id IS NOT NULL
        AND user_a_id < user_b_id)
    OR (kind = 'ai' AND user_a_id IS NOT NULL AND user_b_id IS NULL)
);

CREATE UNIQUE INDEX chats_ai_owner_uq ON chats (user_a_id) WHERE kind = 'ai';

-- The account the assistant sends under.
--
-- A real users row rather than a magic id, so `messages.sender_id` keeps its
-- NOT NULL foreign key and every join, index and cascade over messages goes
-- on working untouched. It belongs to NO family, which is what keeps it out
-- of every roster query (they all select by family_id) without a single
-- special case.
--
-- The password hash is deliberately unusable — this account is never logged
-- into, and `assistant` is refused at registration so nobody can take the
-- name.
-- No ON CONFLICT target: uniqueness is on `lower(username)`, an EXPRESSION
-- index, which a column-name conflict target does not match.
INSERT INTO users (username, display_name, password_hash, family_id)
SELECT 'assistant', 'Assistant', '!', NULL
WHERE NOT EXISTS (SELECT 1 FROM users WHERE lower(username) = 'assistant');

-- What the assistant cost, for Family Statistics.
--
-- One row per completed reply rather than a running total: a total cannot
-- answer "this month" or "who", and rows aggregate into a total whenever
-- somebody asks. Deleting the user takes their usage with them, the same
-- rule their messages follow.
CREATE TABLE ai_usage (
    id                BIGSERIAL   PRIMARY KEY,
    user_id           BIGINT      NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    family_id         BIGINT      NOT NULL REFERENCES families(id) ON DELETE CASCADE,
    message_id        BIGINT      REFERENCES messages(id) ON DELETE SET NULL,
    prompt_tokens     INTEGER     NOT NULL DEFAULT 0,
    completion_tokens INTEGER     NOT NULL DEFAULT 0,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ai_usage_family_idx ON ai_usage (family_id, created_at);
CREATE INDEX ai_usage_user_idx ON ai_usage (user_id);
