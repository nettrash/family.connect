-- 0029_member_reports — the owner's moderation inbox (docs/protocol.md,
-- "Reporting a member").
--
-- Shaped after `join_requests` (0001_init.sql:44-56) down to the column
-- names, because it IS the same screen for the owner: a queue of things
-- somebody raised, each decided once and then off the list.
--
-- THE LOAD-BEARING COLUMN IS message_excerpt. Everywhere else in this
-- protocol a quotation is RECOMPUTED on every read and never stored
-- (protocol.md, "Replies"), so an edit changes what later readers see
-- quoted. A report inverts that rule, for two reasons that both end with
-- the owner opening an empty screen: the author may EDIT the body away
-- through the ordinary author-only path, which has no time limit, and the
-- retention sweep DELETES the message outright after `limits.retention_days`.
-- The excerpt is therefore FROZEN at the moment the report is raised, and
-- it holds the WHOLE body rather than a cut — a moderator judging a report
-- needs all of what was said, and `limits.max_message_chars` already bounds
-- it. `message_id` is ON DELETE SET NULL for the same reason 0012 gave the
-- reply pointer: retention must go on sweeping, and a report whose message
-- has been swept still means something.
--
-- `family_id` is denormalised rather than read through the message's chat,
-- for the same reason: the owner's list is `WHERE family_id = $1 AND status
-- = 'open'`, and reading it through the message would stop working the
-- moment retention took it.
CREATE TABLE member_reports (
    id               BIGSERIAL PRIMARY KEY,
    family_id        BIGINT      NOT NULL REFERENCES families(id) ON DELETE CASCADE,
    reporter_user_id BIGINT      NOT NULL REFERENCES users(id)    ON DELETE CASCADE,
    reported_user_id BIGINT      NOT NULL REFERENCES users(id)    ON DELETE CASCADE,
    message_id       BIGINT               REFERENCES messages(id) ON DELETE SET NULL,
    message_excerpt  TEXT,
    reason           TEXT        NOT NULL
                     CHECK (reason IN ('spam', 'harassment', 'inappropriate', 'other')),
    status           TEXT        NOT NULL DEFAULT 'open'
                     CHECK (status IN ('open', 'resolved')),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at      TIMESTAMPTZ,
    resolved_by      BIGINT               REFERENCES users(id) ON DELETE SET NULL,
    CONSTRAINT member_reports_not_self CHECK (reporter_user_id <> reported_user_id),
    -- A message report ALWAYS carries its excerpt, which protocol.md already
    -- promises ("message_excerpt" is present when and only when one message
    -- was named). It is enforced here because the person index below reads
    -- the excerpt to tell a person report from a message report whose
    -- message retention has since taken, and an index that silently admits
    -- a NULL excerpt would put those two back in the same bucket.
    CONSTRAINT member_reports_message_has_excerpt
        CHECK (message_id IS NULL OR message_excerpt IS NOT NULL)
);

-- "Already reported" is a database fact, the way a pending join request is.
-- TWO partial unique indexes rather than one over `coalesce(message_id, 0)`,
-- because a synthetic zero is a message id that could one day exist.
--
-- THE PERSON INDEX KEYS ON THE EXCERPT, NOT ON `message_id IS NULL`, and
-- that is not a nicety: `message_id` is ON DELETE SET NULL, so retention
-- turns an open MESSAGE report into a row with a null `message_id` — which
-- a person index predicated on `message_id IS NULL` would then collide with
-- the reporter's open PERSON report about the same member. The collision
-- surfaces inside the sweep's own `DELETE FROM messages`
-- (handlers_chat.rs:212), as a unique violation raised by the SET NULL, so
-- the failure is not a rejected report — it is the retention sweep dying
-- for the whole server the first time anybody reports both a member and one
-- of their messages. The excerpt is the only column that still remembers
-- which kind of report this was after the message is gone, which is why the
-- CHECK above makes it non-optional.
CREATE UNIQUE INDEX member_reports_open_message_uq
    ON member_reports (reporter_user_id, message_id)
    WHERE status = 'open' AND message_id IS NOT NULL;
CREATE UNIQUE INDEX member_reports_open_person_uq
    ON member_reports (reporter_user_id, reported_user_id)
    WHERE status = 'open' AND message_id IS NULL AND message_excerpt IS NULL;

-- The owner's list, and the four referencing sides nothing else covers.
-- `member_reports_message_idx` is partial exactly as `ai_usage_message_id_idx`
-- (0023) is, and for the same measured reason: it is what stops the
-- retention sweep scanning this table once per deleted message.
CREATE INDEX member_reports_family_open_idx ON member_reports (family_id)
    WHERE status = 'open';
CREATE INDEX member_reports_reporter_idx    ON member_reports (reporter_user_id);
CREATE INDEX member_reports_reported_idx    ON member_reports (reported_user_id);
CREATE INDEX member_reports_resolved_by_idx ON member_reports (resolved_by)
    WHERE resolved_by IS NOT NULL;
CREATE INDEX member_reports_message_idx     ON member_reports (message_id)
    WHERE message_id IS NOT NULL;
