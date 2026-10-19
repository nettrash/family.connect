-- 0055_animation — an animated picture (a GIF, an animated WebP or PNG) that
-- the SENDER turned into a silent looping video, drawn as a picture that
-- moves (docs/protocol.md, "Animations"; issue #96).
--
-- The video message's pattern exactly (0052), and the sticker's before it
-- (0048): one flag on the ATTACHMENT, because that is where the wire carries
-- it and because it is a fact about how one video is drawn. NOT NULL DEFAULT
-- false: no video ever sent before this was an animation — an animated GIF
-- went as a file — so the default is the truth about the past.
--
-- The three CHECKs are the database's own guarantee, and NO REQUEST CAN TRIP
-- ANY OF THEM. A constraint that fired inside the claim would be a 500, which
-- no outbox treats as terminal — the send would be retried for ever instead
-- of refused (protocol.md, "Sending on an unreliable network"). So the send
-- refuses `animation` beside `sticker` or `round` before any id is read, and
-- the claim writes `animation = ($7 AND kind = 'video')`, which can never
-- flag anything but a video; a wrong-kind send is answered
-- `invalid_attachment` by the check after the claim, and the transaction it
-- drops takes the flag with it.
ALTER TABLE attachments ADD COLUMN animation BOOLEAN NOT NULL DEFAULT false;

-- An animation is a video: the flag on a photo, an audio, a file or a
-- location would be drawn as the ordinary thing it is by every client, and a
-- row that said otherwise would be a lie only the database could tell.
ALTER TABLE attachments ADD CONSTRAINT attachments_animation_is_video
    CHECK (NOT animation OR kind = 'video');

-- Never a sticker as well — which the kind check alone already implies,
-- since a sticker is a photo; said separately, as 0052 says it of `round`,
-- so that a later widening of either flag cannot quietly make the two meet.
ALTER TABLE attachments ADD CONSTRAINT attachments_animation_not_sticker
    CHECK (NOT (animation AND sticker));

-- And never a video message as well: both are videos, so no kind check can
-- keep them apart. A message is one of the three presentations, not two —
-- a circle that loops silently for ever is neither a video message (which
-- talks) nor an animation (which has a balloon for its caption).
ALTER TABLE attachments ADD CONSTRAINT attachments_animation_not_round
    CHECK (NOT (animation AND round));
