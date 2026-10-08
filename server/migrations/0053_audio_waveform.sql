-- 0053_audio_waveform — a voice note's shape, sent by its uploader
-- (docs/protocol.md, "A voice note's waveform").
--
-- 48 levels of 0..15, written as 48 lowercase hex digits: the sender meters
-- the recording anyway and reduces its peaks to these; the server stores the
-- string and echoes it, and computes nothing. Nullable, with no default: a
-- picked sound file, an old message and an old client's upload have none, and
-- NULL is the truth about every row that existed before this.
--
-- The CHECK says what the upload handler says, so no later write path can
-- put a waveform on a photo or a malformed one on a voice note. NO REQUEST
-- CAN TRIP IT: POST /attachments answers `validation` (400) for both before
-- the row is inserted, and a constraint that fired there would be a 500
-- that an outbox retries for ever (protocol.md, "Sending on an unreliable
-- network").
ALTER TABLE attachments ADD COLUMN waveform TEXT;

ALTER TABLE attachments ADD CONSTRAINT attachments_waveform_is_audio
    CHECK (waveform IS NULL OR (kind = 'audio' AND waveform ~ '^[0-9a-f]{48}$'));
