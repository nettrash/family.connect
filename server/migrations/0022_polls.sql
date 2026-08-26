-- 0022_polls — a message the family can vote on (docs/protocol.md, "Polls").
--
-- A poll is not a new kind of thing. It is an ordinary `messages` row with a
-- row here beside it, which is why `polls.message_id` is both the primary key
-- and the foreign key: one poll per message, at most, and the identity of the
-- poll IS the identity of the message. There is no surrogate id to keep in
-- step, no way to write two polls for one message, and `ON DELETE CASCADE`
-- means retention, a direct chat going and a member deleting their account
-- all take the poll with the message without a line of code remembering to.
-- The QUESTION is the message body and is deliberately not a column: the
-- chat-list preview, the push alert, the reply excerpt and the assistant's
-- transcript already read it.
--
-- A FOURTH SEQUENCE, after message_reaction_seq (0002), message_edit_seq
-- (0007) and family_board_seq (0008), and for the fourth time the same
-- reason: `after_id` is `WHERE id > cursor` and can never see a change to an
-- OLDER row, so a client that was offline while a poll three pages back was
-- voted on would never learn of it. Every change to a poll — a vote, a
-- retraction, a close — takes the next value of message_poll_seq and stamps
-- it here (and, GREATEST-guarded, on the chat as last_poll_seq), giving
-- `GET /chats/{id}/polls?after_seq=` a monotonic cursor. Separate from the
-- other three rather than folded in, because consolidating would change the
-- shape of endpoints deployed clients already speak.
--
-- polls_chat_poll_seq_idx is NOT PARTIAL, and that is the one place this
-- differs from messages_chat_reaction_seq_idx and messages_chat_edit_seq_idx.
-- Those two are `WHERE seq > 0` because most messages were never reacted to
-- or edited, so the predicate keeps the index to the small minority that
-- were. A poll has a seq from the moment it exists — it is stamped at
-- creation, never zero — so a partial index would cover every row anyway
-- while costing the planner the guarantee that it does. The catch-up feed
-- reads `WHERE chat_id = $1 AND poll_seq > $2 ORDER BY poll_seq`, and this
-- index answers it outright.
--
-- ONE CHOICE PER MEMBER IS A DATABASE FACT. `poll_votes` is keyed by
-- (poll_id, user_id), not by (poll_id, option_id, user_id): changing your
-- vote is an UPSERT on the primary key that moves the row from one option to
-- another, and there is no state in which a second row for the same member
-- can exist to be cleaned up later. Multiple choice is not a feature this
-- schema is one migration away from — it is a feature this key forbids, which
-- is what the protocol says a v1 poll is.
--
-- poll_options carries its own BIGSERIAL because a vote points at an option
-- and an option's `text` may repeat across polls; `position` is what fixes
-- creation order, and the unique constraint on (poll_id, position) is what
-- stops two options claiming the same slot. Options never change after
-- creation — the votes already cast were cast against the list as it was
-- read, and there is no honest way to re-point them.
--
-- THE THREE INDEXES ON THE REFERENCING SIDES exist for the reason
-- 0006_reply_index.sql was written: PostgreSQL indexes the referenced side of
-- a foreign key and never the referencing side, so without them every DELETE
-- of a referenced row has to prove nothing names it by scanning the child
-- table end to end. That is not a hypothetical here — deleting a family
-- cascades to chats, to messages, to polls, to options and to votes, one row
-- at a time, and 0006 measured exactly this pathology at a ~485x regression
-- on 60k messages. polls_chat_poll_seq_idx covers the chats FK as its leading
-- column, poll_votes_option_idx covers poll_options and poll_votes_user_idx
-- covers users; poll_options and poll_votes both lead on poll_id, which
-- covers their FK to polls.

CREATE SEQUENCE message_poll_seq;

CREATE TABLE polls (
    message_id BIGINT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    chat_id    BIGINT NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    closed_at  TIMESTAMPTZ,
    poll_seq   BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX polls_chat_poll_seq_idx ON polls (chat_id, poll_seq);

CREATE TABLE poll_options (
    id       BIGSERIAL PRIMARY KEY,
    poll_id  BIGINT   NOT NULL REFERENCES polls(message_id) ON DELETE CASCADE,
    position SMALLINT NOT NULL,
    text     TEXT     NOT NULL,
    CONSTRAINT poll_options_position_uq UNIQUE (poll_id, position)
);

CREATE TABLE poll_votes (
    poll_id    BIGINT NOT NULL REFERENCES polls(message_id) ON DELETE CASCADE,
    option_id  BIGINT NOT NULL REFERENCES poll_options(id) ON DELETE CASCADE,
    user_id    BIGINT NOT NULL REFERENCES users(id)        ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (poll_id, user_id)
);
CREATE INDEX poll_votes_option_idx ON poll_votes (option_id);
CREATE INDEX poll_votes_user_idx   ON poll_votes (user_id);

ALTER TABLE chats ADD COLUMN last_poll_seq BIGINT NOT NULL DEFAULT 0;
