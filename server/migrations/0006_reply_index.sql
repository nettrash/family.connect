-- 0006_reply_index — index the reply self-FK.
--
-- 0005 added `messages.reply_to_message_id REFERENCES messages(id)` with no
-- index. Nothing READS by that column (the read path joins on p.id, the
-- primary key), which is why it looked unnecessary — but a foreign key is
-- also checked on DELETE of the referenced row, and there IS a path that
-- deletes messages in bulk: an owner leaving as the sole member deletes the
-- family, which cascades families -> chats -> messages.
--
-- Without this index Postgres runs one sequential scan of `messages` per
-- deleted row to check for referencing replies, turning a linear cascade
-- quadratic. Measured against this schema: 60k messages took 36.8 s with the
-- FK and no index, versus 0.076 s before 0005 existed and 0.27 s (at 20k)
-- with the index — a ~485x regression that this one line removes.
--
-- Partial, because most messages are not replies: the index stays a small
-- fraction of the table while still covering every row the RI check can find.

CREATE INDEX messages_reply_to_message_id_idx ON messages (reply_to_message_id)
    WHERE reply_to_message_id IS NOT NULL;
