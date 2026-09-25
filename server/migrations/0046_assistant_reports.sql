-- 0046_assistant_reports — what a member says the ASSISTANT got wrong
-- (docs/protocol.md, "Reporting the assistant").
--
-- A separate table from `member_reports` (0029) rather than a nullable
-- `reported_user_id` on it, because the two rows have different readers and
-- that difference is the whole design. A member report is the family OWNER's,
-- who can remove the person it names; an assistant report is the OPERATOR's,
-- because the reply came from a model nobody in the family controls and
-- because a private `ai` thread "belongs to that member alone" — routing it
-- to the owner would break the one guarantee that thread has. Sharing a table
-- would make it one `WHERE` clause away from appearing in the owner's inbox,
-- and the owner's inbox query is not the place to put that guarantee.
--
-- message_excerpt is NOT NULL here, where 0029 leaves it nullable: that table
-- also carries reports naming a PERSON and no message, and this one cannot —
-- an assistant report is always about one reply. Frozen for 0029's reason:
-- `message_id` is ON DELETE SET NULL so the retention sweep goes on working,
-- and a report whose evidence has been swept is a reason word and a date.
--
-- chat_kind is frozen beside it, and is the one piece of context the operator
-- cannot reconstruct afterwards: it says whether the reply was private to the
-- reporter ('ai') or something the whole family could already read ('family').
-- Reading it through the message's chat would stop working the moment
-- retention took the message, exactly as 0029 found for `family_id`.
--
-- No `status` column and no resolve path. The protocol gives this row no
-- client read at all, so "dealt with" is the operator's business in their own
-- database; a status nothing can set is a column that lies.
CREATE TABLE assistant_reports (
    id               BIGSERIAL PRIMARY KEY,
    reporter_user_id BIGINT      NOT NULL REFERENCES users(id)    ON DELETE CASCADE,
    message_id       BIGINT               REFERENCES messages(id) ON DELETE SET NULL,
    message_excerpt  TEXT        NOT NULL,
    chat_kind        TEXT        NOT NULL CHECK (chat_kind IN ('ai', 'family')),
    reason           TEXT        NOT NULL
                     CHECK (reason IN ('spam', 'harassment', 'inappropriate', 'other')),
    -- The reporter's own words, up to `limits` in the handler. Free text here
    -- where a member report has none, because the reader is one operator
    -- rather than a nine-language owner (protocol.md).
    note             TEXT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- "Already reported" is a database fact, as it is for 0029. Partial, because
-- retention nulls `message_id` and two swept reports of different replies must
-- not then collide — there is no person-shaped report here for such a row to
-- be confused with, so dropping out of the index is the whole story.
CREATE UNIQUE INDEX assistant_reports_reporter_message_uq
    ON assistant_reports (reporter_user_id, message_id)
    WHERE message_id IS NOT NULL;

-- The reporter side nothing else covers, and the partial message index that
-- keeps the retention sweep from scanning this table once per deleted
-- message — measured for `ai_usage` in 0023 and for `member_reports` in 0029.
CREATE INDEX assistant_reports_reporter_idx ON assistant_reports (reporter_user_id);
CREATE INDEX assistant_reports_message_idx  ON assistant_reports (message_id)
    WHERE message_id IS NOT NULL;
-- What the operator actually types: the newest first.
CREATE INDEX assistant_reports_created_idx  ON assistant_reports (created_at DESC);
