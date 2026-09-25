-- 0044_note_mentions — a board note names members (docs/protocol.md,
-- "Board": "A note may NAME MEMBERS").
--
-- The same shape as `message_mentions`, and deliberately so: one row per
-- member named, `name` the display name AS TYPED after the "@" so a client
-- can find the token in the text without knowing what the member is called
-- today, `position` the author's order.
--
-- The one difference is in the writing, not the schema: a note's list is
-- RE-DECIDED on every edit (a message's is fixed at send), because an edit
-- to a note notifies nobody and so cannot wake anyone twice. The rows for
-- a note are therefore deleted and rewritten by a PATCH that changes the
-- text.
--
-- CASCADE both ways: the list dies with the note (a delete, or retention),
-- and with the account it names.
CREATE TABLE note_mentions (
    note_id  BIGINT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    user_id  BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name     TEXT   NOT NULL,
    position INT    NOT NULL,
    PRIMARY KEY (note_id, user_id)
);

-- One board read returns every live note with its names, so the lookup is
-- by note.
CREATE INDEX note_mentions_note_idx ON note_mentions (note_id);
