-- 0039_threads — a reply knows the top of its chain (docs/protocol.md, "Threads").
--
-- A chain of replies can be read on its own, rooted at the TOP: the first
-- message in a reply's ancestry that is not itself a reply. Stored rather
-- than recomputed, unlike the quote, because a chain is read far more often
-- than it is written and walking `reply_to_message_id` on every read would
-- need the whole ancestry in hand — the unbounded thing the two-level quote
-- exists to avoid.
--
-- ON DELETE SET NULL, exactly as 0012 made the reply's own link, and for the
-- same reason: a reply is a message in its own right. When the root is swept
-- the replies stay and simply stop belonging to a chain.
ALTER TABLE messages
    ADD COLUMN thread_root_id BIGINT REFERENCES messages(id) ON DELETE SET NULL;

-- The chain's access path — "every reply under this root" — and the count
-- every read of a root performs. Partial: most messages are not replies.
CREATE INDEX messages_thread_root_idx ON messages (thread_root_id)
    WHERE thread_root_id IS NOT NULL;

-- Backfill: every existing reply's root, by walking its surviving ancestry
-- to the first message that is not a reply. A reply whose parent was swept
-- before this migration is not a reply any more (0012 set its link NULL), so
-- it becomes the root of whatever still quotes it — the most the surviving
-- links can say.
WITH RECURSIVE chain AS (
    SELECT m.id, m.reply_to_message_id AS up, 1 AS depth
      FROM messages m
     WHERE m.reply_to_message_id IS NOT NULL
    UNION ALL
    SELECT c.id, p.reply_to_message_id, c.depth + 1
      FROM chain c
      JOIN messages p ON p.id = c.up
     WHERE p.reply_to_message_id IS NOT NULL
),
tops AS (
    SELECT DISTINCT ON (id) id, up AS root
      FROM chain
     ORDER BY id, depth DESC
)
UPDATE messages m
   SET thread_root_id = tops.root
  FROM tops
 WHERE m.id = tops.id;
