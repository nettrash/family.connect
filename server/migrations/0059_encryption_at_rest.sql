-- 0059_encryption_at_rest — what people wrote, said, showed or are is stored
-- sealed (docs/protocol.md, "Stored encrypted"; issue #59; the design and its
-- decisions are docs/encryption-at-rest-2026-10-24.md, the formats are
-- src/vault.rs).
--
-- THIS MIGRATION IS HALF OF A CONVERSION. It runs, then the server seals what
-- it left behind (src/seal_existing.rs), and only then does 0060 run — the
-- migrator holds 0060 back until the Rust step has finished, under the same
-- advisory lock, before anything is served. So between this file and 0060 the
-- schema is in a state no request ever sees: every content column is `bytea`,
-- and a value in it is either sealed (first byte 0x01) or the old text behind
-- a 0x00 byte, "not sealed yet". The Rust step turns every 0x00 into a 0x01,
-- 500 rows to a transaction, so a crash or a restart continues where it
-- stopped; 0060 then makes PostgreSQL refuse anything that is not sealed.
--
-- THE KEYS. Every family, every conversation that is not a family chat (a
-- group, a direct chat, an assistant chat) and every account has a data key of
-- its own, kept here WRAPPED by the server key — which lives in a file, never
-- in the database and never in a backup. The owner row names its key in
-- `key_id`; a family chat names its family's. Naming it on the owner rather
-- than the owner on the key is what lets a restore be checked with a join: an
-- owner whose `key_id` has no row here has lost its key, and a key no owner
-- names is one whose owner was deleted.
--
-- NO FOREIGN KEYS, deliberately. The nightly backup leaves this table's rows
-- out of the main dump and copies them separately, kept 7 days
-- (docs/operations.md, "Backups"), so that a family deleted today is
-- unreadable in every backup a week from now. A restore therefore loads a key
-- copy that may be NEWER than the dump — holding keys for families the dump
-- has never heard of — and one such row under a foreign key would fail the
-- whole COPY. Deleting a key with its owner is done by the triggers below
-- instead, in the same transaction as the delete and on every cascade, and the
-- server drops keys nobody names at start.
--
-- Key ids are random 63-bit numbers, not a sequence: a sequence's position
-- travels with the main dump while the keys travel with the key copy, and the
-- two would disagree after the first restore.
CREATE TABLE storage_keys (
    id         BIGINT      PRIMARY KEY CHECK (id > 0),
    -- nonce | the key sealed under the server key | tag (src/vault.rs).
    wrapped    BYTEA       NOT NULL CHECK (length(wrapped) = 72),
    -- The fingerprint of the server key that wrapped it. Not needed to open
    -- anything — the AEAD already refuses the wrong key — but it is what lets
    -- the refusal SAY which key file it wanted.
    wrapped_by TEXT        NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- NULL only until the conversion has given every existing owner a key; 0060
-- makes all three NOT NULL. Unique for a family and an account; a family chat
-- shares its family's key, so the chats index is not.
ALTER TABLE families ADD COLUMN key_id BIGINT;
ALTER TABLE chats    ADD COLUMN key_id BIGINT;
ALTER TABLE users    ADD COLUMN key_id BIGINT;
CREATE UNIQUE INDEX families_key_id_uq ON families (key_id);
CREATE INDEX chats_key_id_idx ON chats (key_id);
CREATE UNIQUE INDEX users_key_id_uq ON users (key_id);

-- A key goes with its owner. AFTER DELETE, per row, so a family deleted by
-- `delete_family_in_tx` takes its own key and — through the cascade to
-- `chats` — the keys of its groups and assistant chats, in that transaction.
-- A family chat names its family's key and must not take it; the family's
-- own trigger does. An account is never deleted (0023) but its key is
-- REPLACED when it is scrubbed, and the replaced key goes the same way.
CREATE FUNCTION storage_key_goes_with_its_owner() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    DELETE FROM storage_keys WHERE id = OLD.key_id;
    RETURN NULL;
END;
$$;

CREATE TRIGGER families_storage_key_goes
    AFTER DELETE ON families
    FOR EACH ROW
    WHEN (OLD.key_id IS NOT NULL)
    EXECUTE FUNCTION storage_key_goes_with_its_owner();

CREATE TRIGGER chats_storage_key_goes
    AFTER DELETE ON chats
    FOR EACH ROW
    WHEN (OLD.key_id IS NOT NULL AND OLD.kind <> 'family')
    EXECUTE FUNCTION storage_key_goes_with_its_owner();

CREATE TRIGGER users_storage_key_goes
    AFTER DELETE ON users
    FOR EACH ROW
    WHEN (OLD.key_id IS NOT NULL)
    EXECUTE FUNCTION storage_key_goes_with_its_owner();

CREATE TRIGGER users_storage_key_replaced
    AFTER UPDATE OF key_id ON users
    FOR EACH ROW
    WHEN (OLD.key_id IS NOT NULL AND OLD.key_id IS DISTINCT FROM NEW.key_id)
    EXECUTE FUNCTION storage_key_goes_with_its_owner();

-- THE CONTENT COLUMNS become `bytea`, each value kept as the old text behind a
-- 0x00 byte until the server seals it. A column stays NULL where it was NULL.
-- What PostgreSQL used to compute on these columns moves out first: an index
-- or a trigger over a column cannot survive its type changing under it, and
-- none of them would mean anything over a sealed value.

-- USERS. Who somebody is: username, display name, password hash, birthday,
-- all under the account's key.
--
-- Signing in found an account by `lower(username)`. It finds it by
-- `username_key` now: an HMAC of the lower-cased name under the server key
-- (src/vault.rs, `Lookup::Username`), unique as the old index was. Filled in
-- by the conversion; NOT NULL from 0060.
--
-- The assistant's account was found BY NAME (`lower(username) =
-- 'assistant'`), in queries and in 0036's two triggers. A sealed name cannot be
-- compared, so the account is marked instead, and 0036's invariant is restated
-- on the mark: there is always exactly one live row with `is_assistant`, it
-- cannot be tombstoned, unmarked or deleted. Renaming it is no longer the
-- danger it was, because nothing finds it by its name any more.
--
-- The birthday was two SMALLINTs that a family's greeting read with SELECT
-- DISTINCT. It is ONE sealed value now, two bytes — month, day — and the
-- greeting reads them in Rust. 0018's two CHECKs go with the columns; the
-- server has always validated a birthday before writing it.
DROP TRIGGER users_assistant_not_removable ON users;
DROP TRIGGER users_assistant_not_deleted ON users;
DROP INDEX users_username_lower_uq;

ALTER TABLE users ADD COLUMN is_assistant BOOLEAN NOT NULL DEFAULT false;
UPDATE users SET is_assistant = true WHERE lower(username) = 'assistant';
CREATE UNIQUE INDEX users_one_assistant_uq ON users (is_assistant) WHERE is_assistant;

ALTER TABLE users
    ALTER COLUMN username      TYPE BYTEA USING '\x00'::bytea || convert_to(username, 'UTF8'),
    ALTER COLUMN display_name  TYPE BYTEA USING '\x00'::bytea || convert_to(display_name, 'UTF8'),
    ALTER COLUMN password_hash TYPE BYTEA USING '\x00'::bytea || convert_to(password_hash, 'UTF8'),
    ADD COLUMN username_key BYTEA,
    ADD COLUMN birthday     BYTEA;
CREATE UNIQUE INDEX users_username_key_uq ON users (username_key);
UPDATE users
   SET birthday = '\x00'::bytea
                  || set_byte(set_byte('\x0000'::bytea, 0, birthday_month), 1, birthday_day)
 WHERE birthday_month IS NOT NULL AND birthday_day IS NOT NULL;
ALTER TABLE users DROP COLUMN birthday_month, DROP COLUMN birthday_day;

CREATE TRIGGER users_assistant_not_removable
    BEFORE UPDATE ON users
    FOR EACH ROW
    WHEN (OLD.is_assistant AND (NEW.deleted_at IS NOT NULL OR NOT NEW.is_assistant))
    EXECUTE FUNCTION assistant_is_not_removable();

CREATE TRIGGER users_assistant_not_deleted
    BEFORE DELETE ON users
    FOR EACH ROW
    WHEN (OLD.is_assistant)
    EXECUTE FUNCTION assistant_is_not_removable();

-- PROFILE PICTURES, under the account's key. Already bytes: only the 0x00
-- goes in front.
UPDATE user_avatars SET bytes = '\x00'::bytea || bytes;

-- DEVICES. The push and VoIP tokens, under the account's key — the server
-- hands them to APNs and FCM, so they are kept — each found again by a lookup
-- column, unique as the tokens were, which is what a registration's upsert and
-- a token moving to a new login go by now.
DROP INDEX devices_push_token_uq;
DROP INDEX devices_voip_token_uq;
ALTER TABLE devices
    ALTER COLUMN push_token TYPE BYTEA USING '\x00'::bytea || convert_to(push_token, 'UTF8'),
    ALTER COLUMN voip_token TYPE BYTEA USING '\x00'::bytea || convert_to(voip_token, 'UTF8'),
    ADD COLUMN push_token_key BYTEA,
    ADD COLUMN voip_token_key BYTEA;
CREATE UNIQUE INDEX devices_push_token_key_uq ON devices (push_token_key)
    WHERE push_token_key IS NOT NULL;
CREATE UNIQUE INDEX devices_voip_token_key_uq ON devices (voip_token_key)
    WHERE voip_token_key IS NOT NULL;

-- FAMILIES. The name, the invite code and the greeting places, under the
-- family's key. Joining found a family by `invite_code`; it finds it by
-- `invite_code_key` now, the HMAC of the code (upper case, as the join has
-- always written it) under the server key, unique as the code was. The places
-- were a TEXT[] with a CHECK on its shape; they are one sealed JSON array now,
-- and the server has always validated them before writing (at most three,
-- none empty).
DROP INDEX families_invite_code_uq;
ALTER TABLE families DROP CONSTRAINT families_greeting_places_check;
ALTER TABLE families
    ALTER COLUMN name            TYPE BYTEA USING '\x00'::bytea || convert_to(name, 'UTF8'),
    ALTER COLUMN invite_code     TYPE BYTEA USING '\x00'::bytea || convert_to(invite_code, 'UTF8'),
    ALTER COLUMN greeting_places DROP DEFAULT,
    ALTER COLUMN greeting_places TYPE BYTEA
        USING '\x00'::bytea || convert_to(array_to_json(greeting_places)::text, 'UTF8'),
    ADD COLUMN invite_code_key BYTEA;
CREATE UNIQUE INDEX families_invite_code_key_uq ON families (invite_code_key);

-- The code a join request was made with, under the family's key.
ALTER TABLE join_requests
    ALTER COLUMN invite_code_used TYPE BYTEA
        USING '\x00'::bytea || convert_to(invite_code_used, 'UTF8');

-- A GROUP's name, under the group's own key: deleting the group takes it
-- from every backup with everything said in it. Only groups have one (0056's
-- CHECKs, which test only whether it is NULL, stay as they are).
ALTER TABLE chats
    ALTER COLUMN title TYPE BYTEA USING '\x00'::bytea || convert_to(title, 'UTF8');

-- WHAT IS SAID, under the key of the conversation it was said in — the chat's
-- own for a group, a direct chat or an assistant chat, the family's for the
-- family chat: the messages, the reactions to them, the names a mention
-- shows, a poll's options and a recording's transcript. The reaction upsert
-- compared emoji in SQL (`IS DISTINCT FROM`); the server compares them opened
-- now, under the message lock it already takes.
ALTER TABLE messages
    ALTER COLUMN body TYPE BYTEA USING '\x00'::bytea || convert_to(body, 'UTF8');
ALTER TABLE message_reactions
    ALTER COLUMN emoji TYPE BYTEA USING '\x00'::bytea || convert_to(emoji, 'UTF8');
ALTER TABLE message_mentions
    ALTER COLUMN name TYPE BYTEA USING '\x00'::bytea || convert_to(name, 'UTF8');
ALTER TABLE poll_options
    ALTER COLUMN text TYPE BYTEA USING '\x00'::bytea || convert_to(text, 'UTF8');
ALTER TABLE transcripts
    ALTER COLUMN text TYPE BYTEA USING '\x00'::bytea || convert_to(text, 'UTF8');

-- THE BOARD AND THE STICKER PACK, under the family's key: a note's words and
-- its event's place, a task list's lines, the names a note mentions, and a
-- sticker's label. 0048's CHECK on the label's length goes; the server has
-- always validated it before writing.
ALTER TABLE notes
    ALTER COLUMN text  TYPE BYTEA USING '\x00'::bytea || convert_to(text, 'UTF8'),
    ALTER COLUMN place TYPE BYTEA USING '\x00'::bytea || convert_to(place, 'UTF8');
ALTER TABLE note_task_items
    ALTER COLUMN text TYPE BYTEA USING '\x00'::bytea || convert_to(text, 'UTF8');
ALTER TABLE note_mentions
    ALTER COLUMN name TYPE BYTEA USING '\x00'::bytea || convert_to(name, 'UTF8');
ALTER TABLE pack_items DROP CONSTRAINT pack_items_label_check;
ALTER TABLE pack_items
    ALTER COLUMN label TYPE BYTEA USING '\x00'::bytea || convert_to(label, 'UTF8');

-- REPORTS. A member report's frozen excerpt is the family's, under its key —
-- the owner reads it, and it goes with the family. An assistant report holds
-- a reply from one conversation and the reporter's own note about it, under
-- that conversation's key, so a deleted conversation takes its reports'
-- words with it. The operator reads them with `family-connect reports`, since
-- psql shows only sealed bytes now (docs/operations.md).
ALTER TABLE member_reports
    ALTER COLUMN message_excerpt TYPE BYTEA
        USING '\x00'::bytea || convert_to(message_excerpt, 'UTF8');
ALTER TABLE assistant_reports
    ALTER COLUMN message_excerpt TYPE BYTEA
        USING '\x00'::bytea || convert_to(message_excerpt, 'UTF8'),
    ALTER COLUMN note TYPE BYTEA USING '\x00'::bytea || convert_to(note, 'UTF8');

-- ATTACHMENTS. A file's name, a location's coordinates and a voice note's
-- waveform, under the key of whatever holds the attachment: the
-- conversation's once a message claims it, the family's for a board note, a
-- sticker or an upload nothing has claimed yet. The coordinates are sealed as
-- eight big-endian bytes (`float8send`). Three CHECKs read these columns'
-- content — a file name's length, the coordinates' range, the waveform's
-- form — and the server has always validated all three before writing; each
-- is replaced by the part that tests only what is NULL, which still holds.
ALTER TABLE attachments
    DROP CONSTRAINT attachments_file_has_name,
    DROP CONSTRAINT attachments_coordinates_in_range,
    DROP CONSTRAINT attachments_waveform_is_audio;
ALTER TABLE attachments
    ALTER COLUMN name      TYPE BYTEA USING '\x00'::bytea || convert_to(name, 'UTF8'),
    ALTER COLUMN latitude  TYPE BYTEA USING '\x00'::bytea || float8send(latitude),
    ALTER COLUMN longitude TYPE BYTEA USING '\x00'::bytea || float8send(longitude),
    ALTER COLUMN waveform  TYPE BYTEA USING '\x00'::bytea || convert_to(waveform, 'UTF8');
ALTER TABLE attachments
    ADD CONSTRAINT attachments_file_has_name CHECK (kind <> 'file' OR name IS NOT NULL),
    ADD CONSTRAINT attachments_accuracy_not_negative CHECK (accuracy_m IS NULL OR accuracy_m >= 0),
    ADD CONSTRAINT attachments_waveform_is_audio CHECK (waveform IS NULL OR kind = 'audio');

-- THE FILE ITSELF has a key of its own, kept here sealed under the holder's
-- key — so a photo shared by dedup between rows is one file under one key,
-- wrapped once per row. NULL means a file written before #59, still
-- plaintext: the server seals those in the background once it is serving,
-- and a file is never guessed at by sniffing.
ALTER TABLE attachments ADD COLUMN file_key BYTEA;

-- DEDUP was a plaintext SHA-256 of the bytes, which an old backup could be
-- asked about ("did this family ever have this picture?"). It becomes an
-- HMAC of that digest under the family's key, and goes with the family. The
-- old digest is kept aside until the conversion has keyed it; 0060 drops it.
ALTER TABLE attachments ADD COLUMN content_sha256 TEXT;
UPDATE attachments SET content_sha256 = content_hash, content_hash = NULL
 WHERE content_hash IS NOT NULL;

-- Files whose rows have moved to a sealed copy, waiting to be removed. A
-- crash between the move and the removal leaves the name here, and the next
-- pass removes it, so no plaintext file outlives its conversion.
CREATE TABLE storage_cleanup (
    storage_key TEXT        PRIMARY KEY,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
