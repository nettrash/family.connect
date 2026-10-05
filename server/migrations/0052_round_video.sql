-- 0052_round_video — a video sent as a VIDEO MESSAGE, drawn as a circle
-- (docs/protocol.md, "Video messages").
--
-- The sticker's pattern exactly (0048): one flag on the ATTACHMENT, because
-- that is where the wire carries it and because it is a fact about how one
-- video is drawn. NOT NULL DEFAULT false: no video ever sent before this was
-- a video message, so the default is the truth about the past. The name is
-- `round` and not `video_note`: in this codebase a note is a board note.
--
-- The two CHECKs are the database's own guarantee, and NO REQUEST CAN TRIP
-- EITHER. A constraint that fired inside the claim would be a 500, which no
-- outbox treats as terminal — the send would be retried for ever instead of
-- refused (protocol.md, "Sending on an unreliable network"). So the send
-- refuses `round` beside `sticker` before any id is read, and the claim
-- writes `round = ($6 AND kind = 'video')`, which can never flag anything
-- but a video; a wrong-kind send is answered `invalid_attachment` by the
-- check after the claim, and the transaction it drops takes the flag with
-- it. 0048 put no kind CHECK on `sticker`; this one can afford one because
-- the claim was written around it.
ALTER TABLE attachments ADD COLUMN round BOOLEAN NOT NULL DEFAULT false;

-- A video message is a video: the flag on a photo, an audio, a file or a
-- location would be drawn as the ordinary thing it is by every client, and
-- a row that said otherwise would be a lie only the database could tell.
ALTER TABLE attachments ADD CONSTRAINT attachments_round_is_video
    CHECK (NOT round OR kind = 'video');

-- And it is never a sticker as well — which a kind check alone already
-- implies, since a sticker is a photo; said separately so that a later
-- widening of either flag cannot quietly make the two meet.
ALTER TABLE attachments ADD CONSTRAINT attachments_round_not_sticker
    CHECK (NOT (round AND sticker));
