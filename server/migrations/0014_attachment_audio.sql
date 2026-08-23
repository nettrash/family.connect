-- Audio attachments: a voice note recorded on the spot, or a sound file
-- picked off a disk (docs/protocol.md, "Audio").
--
-- One kind for both, because a bubble plays them identically and the server
-- cannot tell them apart anyway. Audio carries a duration like a video and
-- never a preview — there is nothing to look at, so a client draws a play
-- control, the duration and a scrubber instead.
--
-- `name` stays optional here, unlike a file: a voice note's identity is its
-- length, but a track picked off a disk has a name worth showing, and the
-- 0010 constraint already only demands one for `file`.

ALTER TABLE attachments DROP CONSTRAINT attachments_kind_check;
ALTER TABLE attachments ADD CONSTRAINT attachments_kind_check
    CHECK (kind IN ('photo', 'video', 'audio', 'file'));
