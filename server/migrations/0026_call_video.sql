-- 0026_call_video — the record learns which KIND of call it was
-- (docs/protocol.md, "Video").
--
-- A video call is a voice call with a flag: decided when the call is placed,
-- fixed for its life, and the media difference lives entirely in the SDP the
-- server never reads. While the call is in flight the flag lives in the
-- in-memory registry with the rest of the signalling; the only DURABLE trace
-- of a call is its record row (0024), so the flag lands there — a client
-- drawing "Video call" a year later reads it from the same row everything
-- else about the call comes from.
--
-- NOT NULL DEFAULT false, because every call recorded before this column
-- existed WAS a voice call: the default is not a guess, it is the truth
-- about the past. On the wire the flag stays optional — `"video": true`
-- present when and only when it was a video call, absent on a voice call
-- like every optional field — so `false` here serializes to no key at all.

ALTER TABLE calls ADD COLUMN video BOOLEAN NOT NULL DEFAULT false;
