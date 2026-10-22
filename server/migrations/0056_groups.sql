-- Groups inside a family (docs/protocol.md, "Groups"; issue #107).
--
-- A fourth chat kind rather than a separate table, for the reason the `ai`
-- kind gave: replies, threads, reactions, polls, editing, retention, unread
-- counts and catch-up then work in a group without a line of new code.
-- Membership is `chat_members`, which already holds explicit rows for every
-- kind.
--
-- A group's pair columns stay NULL. Its admin is a column of its own, and
-- deliberately NOT `user_a_id`: deleting an account deletes every chat whose
-- `user_a_id` is that account (the direct and `ai` chats it owned), and a
-- group must outlive its admin — the handler hands it on first, and
-- `ON DELETE SET NULL` is the backstop that can never take the group with it.

ALTER TABLE chats DROP CONSTRAINT chats_kind_shape;
ALTER TABLE chats DROP CONSTRAINT chats_kind_check;

ALTER TABLE chats ADD CONSTRAINT chats_kind_check
    CHECK (kind IN ('family', 'direct', 'ai', 'group'));

ALTER TABLE chats ADD COLUMN title TEXT;
ALTER TABLE chats ADD COLUMN admin_user_id BIGINT REFERENCES users(id) ON DELETE SET NULL;

ALTER TABLE chats ADD CONSTRAINT chats_kind_shape CHECK (
    (kind = 'family' AND user_a_id IS NULL AND user_b_id IS NULL)
    OR (kind = 'direct' AND user_a_id IS NOT NULL AND user_b_id IS NOT NULL
        AND user_a_id < user_b_id)
    OR (kind = 'ai' AND user_a_id IS NOT NULL AND user_b_id IS NULL)
    OR (kind = 'group' AND user_a_id IS NULL AND user_b_id IS NULL AND title IS NOT NULL)
);

-- Only a group has a name of its own or an admin; every other kind is named
-- by the server from the family, the peer or the assistant.
ALTER TABLE chats ADD CONSTRAINT chats_group_fields CHECK (
    kind = 'group' OR (title IS NULL AND admin_user_id IS NULL)
);

-- "At most 50 groups in a family" is counted per family.
CREATE INDEX chats_family_groups_idx ON chats (family_id) WHERE kind = 'group';

-- An `@ai` answer in a group may be reported, like one in the family chat.
ALTER TABLE assistant_reports DROP CONSTRAINT assistant_reports_chat_kind_check;
ALTER TABLE assistant_reports ADD CONSTRAINT assistant_reports_chat_kind_check
    CHECK (chat_kind IN ('ai', 'family', 'group'));
