-- 0024_calls — the record a voice call leaves behind, and the token a phone
-- is rung on (docs/protocol.md, "Voice calls" and "Incoming calls").
--
-- A call is not stored while it happens. Its signalling lives in the
-- server's memory for the seconds it rings and the minutes it lasts, and its
-- audio never touches the server at all — so the only durable trace is ONE
-- message in the direct chat, written when the call ends. That message is an
-- ordinary `messages` row with a row here beside it, exactly as a poll is
-- (0022): `calls.message_id` is both the primary key and the foreign key, so
-- there is one record per message at most and the identity of the record IS
-- the identity of the message. `ON DELETE CASCADE` means retention, a direct
-- chat going and a member deleting their account all take the record with
-- the message without a line of code remembering to.
--
-- The message's `client_msg_id` is the call's id, which is what makes the
-- record exactly-once for free: the `(chat_id, sender_id, client_msg_id)`
-- uniqueness that dedups retried sends dedups a call that ends twice — once
-- by a client's `call_end` and once by the sweeper's timeout, say — and no
-- handler has to remember to check.
--
-- `outcome` is CHECK-constrained to the four words the protocol defines, and
-- `duration_secs` is nullable because it means something when it is absent:
-- the call was never answered. A `failed` call that was answered carries one;
-- a `missed` call never does.
--
-- `devices.voip_token` is the iOS PushKit token, and it is a SECOND token on
-- the same row rather than a second row: the alert token and the VoIP token
-- name the same phone, arrive from the OS at different moments, and are
-- rotated independently, so a launch that has only one of them must be able
-- to write it without touching the other. The partial unique index mirrors
-- devices_push_token_uq for the same reason — a token that moves to another
-- account (the same phone, a new login) re-homes its row rather than
-- duplicating it. A Mac never has one: PushKit's VoIP type does not exist
-- on macOS, and a Mac that is not running is not a phone in a pocket.

CREATE TABLE calls (
    message_id    BIGINT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    outcome       TEXT NOT NULL CHECK (outcome IN ('completed', 'missed', 'declined', 'failed')),
    duration_secs BIGINT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE devices ADD COLUMN voip_token TEXT;
CREATE UNIQUE INDEX devices_voip_token_uq ON devices (voip_token) WHERE voip_token IS NOT NULL;
