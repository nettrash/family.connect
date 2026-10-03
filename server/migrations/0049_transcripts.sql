-- 0049_transcripts — the text of a recording, on request
-- (docs/protocol.md, "Transcripts on request").
--
-- THE STORED ANSWER. One row per attachment, written the first time a member
-- asks for the text of a voice note or audio file and the server sends its
-- OWN stored bytes to the transcription deployment. Every later member who
-- may ask is handed this row instead of a second provider call — one bill
-- per recording, not one per reader.
--
-- Only answers made from the server's own bytes are written here. An answer
-- made from sound a client SUPPLIED (a video's sound track, an Ogg file the
-- provider cannot read) is returned to the asker and never stored: the
-- server cannot check that uploaded sound is really this recording's, and a
-- stored answer is handed to other members — so one member could otherwise
-- put words into another's video.
--
-- Keyed by ATTACHMENT, not by `storage_key`, although identical bytes share
-- one file per family (0011): who may read a transcript is decided by the
-- message the attachment is on, and a row keyed by the bytes would answer
-- for a copy in a chat the first asker never saw.
--
-- ON DELETE CASCADE with the attachment, and that is the whole of retention
-- and account deletion for it: the row goes when the recording's row goes —
-- with the message past `retention_days`, with a departing member's direct
-- chats, with a deleted family. Nothing new to sweep.
--
-- `text` may be EMPTY: a recording with no speech in it is answered `""`,
-- and that is an answer worth keeping ("No speech"), not a failure to retry.
-- `language` is what the provider said it heard, when it said anything.
-- Neither ever reaches the log.
CREATE TABLE transcripts (
    attachment_id BIGINT      PRIMARY KEY REFERENCES attachments(id) ON DELETE CASCADE,
    text          TEXT        NOT NULL,
    language      TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- THE OWNER'S SWITCH, the sixth: whether a member may ask for the text of
-- ANOTHER member's recording in the family chat. A member's OWN recordings
-- need no switch, only their own consent.
--
-- FALSE by default, for every family before this and after it, on the rule
-- every widening has followed (0019's argument, 0032's default): nothing
-- anybody else said leaves the server unasked, and a recorded voice is
-- something somebody said. It is bound to no other switch — it widens
-- nothing the assistant is SHOWN, and the assistant never sees a transcript.
ALTER TABLE families ADD COLUMN ai_transcripts BOOLEAN NOT NULL DEFAULT FALSE;

-- WHAT IT COST. Transcription is billed by AUDIO LENGTH, not tokens, so a
-- family reading only `prompt_tokens` would see it as free — the argument
-- 0032 made for `images`. One `ai_usage` row per provider call that
-- produced an answer, with `transcripts = 1` and the recording's
-- `duration_ms`; such a row is not a `question`, and the statistics count
-- questions as the rows with `transcripts = 0`, which is every row written
-- before this migration. A stored answer handed out again writes nothing.
--
-- BIGINT for the duration: a family's total is a sum of these, and
-- `attachments.duration_ms` is an INTEGER per recording, not per family.
ALTER TABLE ai_usage ADD COLUMN transcripts INTEGER NOT NULL DEFAULT 0;
ALTER TABLE ai_usage ADD COLUMN audio_ms BIGINT NOT NULL DEFAULT 0;
