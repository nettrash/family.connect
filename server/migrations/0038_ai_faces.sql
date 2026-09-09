-- Profile pictures for an @ai mention (docs/protocol.md, "Profile pictures of
-- members"): whether a mention in the family chat may ALSO be shown the
-- profile pictures of the members whose lines are in the transcript it is
-- sent — faces nobody attached to anything, on a message nobody sent.
--
-- A FIFTH switch, and the fourth about disclosure. It stands beside
-- `ai_history_photos` (0033) rather than inside it, for 0033's own reason
-- turned on a different object: every owner who turned Recent photos on did
-- so under a sentence about PHOTOGRAPHS in the chat — pictures a member sent
-- — and a profile picture is not one. It is the first image in this protocol
-- that is attached to no message at all. Widening a switch whose sentence
-- names "the most recent photos in the family chat" to cover a picture that
-- is not in the chat would make that sentence false for every family that
-- said yes to it. So this is a new one.

-- NOT NULL DEFAULT FALSE, like 0032 and 0033 and for their reason: off for
-- families created after it and for every family that existed before. Nobody
-- is opted in by a migration. A face is the most identifying thing a
-- photograph can carry, and a member uploaded theirs to be recognised by
-- their FAMILY, not by a model; whether it may go further is the owner's to
-- say, under a sentence that names the cost, and never assumed.
ALTER TABLE families ADD COLUMN ai_faces BOOLEAN NOT NULL DEFAULT FALSE;

-- It can only be true while `ai_vision` is, and the database says so in its
-- own words rather than trusting the one handler that writes both — the same
-- CHECK 0033 carries, for the same reason: a switch that quietly stayed on
-- underneath the one that was turned off would spring back the day
-- `ai_vision` was turned on again, and a member's face would reach a model
-- without anybody choosing that a second time. PATCH /families/mine refuses
-- `true` here while `ai_vision` is off, and turns this off in the same write
-- whenever `ai_vision` goes off, whether or not the request mentioned it.
--
-- Every existing row is FALSE in both columns, so the constraint holds the
-- moment it is added and validates nothing.
ALTER TABLE families ADD CONSTRAINT families_ai_faces_needs_vision
    CHECK (ai_vision OR NOT ai_faces);

-- What this column can never do: send a face for a name the model has not
-- been told. A profile picture travels only for a member whose line is in
-- the transcript — with `ai_history` off there is no transcript, no names,
-- and therefore no faces, at any setting of this. And it never widens the
-- private `ai` thread, which carries no member's name at all: a face with no
-- name to attach to is a second disclosure this migration does not make.
