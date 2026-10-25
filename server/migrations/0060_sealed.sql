-- 0060_sealed — the second half of 0059's conversion, run only once the
-- server has sealed every value 0059 left behind (src/seal_existing.rs). The
-- migrator does not apply this file until that step has finished.
--
-- From here on PostgreSQL itself refuses what is not sealed: every owner has a
-- key, and every content column holds a sealed value (first byte 0x01) or
-- NULL. A query that tried to store plaintext would fail here, in the
-- database, not quietly succeed.
ALTER TABLE families ALTER COLUMN key_id SET NOT NULL;
ALTER TABLE chats    ALTER COLUMN key_id SET NOT NULL;
ALTER TABLE users    ALTER COLUMN key_id SET NOT NULL;

ALTER TABLE users ALTER COLUMN username_key SET NOT NULL;
ALTER TABLE users
    ADD CONSTRAINT users_username_sealed      CHECK (substr(username, 1, 1) = '\x01'::bytea),
    ADD CONSTRAINT users_display_name_sealed  CHECK (substr(display_name, 1, 1) = '\x01'::bytea),
    ADD CONSTRAINT users_password_hash_sealed CHECK (substr(password_hash, 1, 1) = '\x01'::bytea),
    ADD CONSTRAINT users_birthday_sealed      CHECK (substr(birthday, 1, 1) = '\x01'::bytea);

ALTER TABLE user_avatars
    ADD CONSTRAINT user_avatars_bytes_sealed CHECK (substr(bytes, 1, 1) = '\x01'::bytea);

-- A token and its lookup are there together or not at all.
ALTER TABLE devices
    ADD CONSTRAINT devices_push_token_sealed CHECK (substr(push_token, 1, 1) = '\x01'::bytea),
    ADD CONSTRAINT devices_voip_token_sealed CHECK (substr(voip_token, 1, 1) = '\x01'::bytea),
    ADD CONSTRAINT devices_push_token_found CHECK ((push_token IS NULL) = (push_token_key IS NULL)),
    ADD CONSTRAINT devices_voip_token_found CHECK ((voip_token IS NULL) = (voip_token_key IS NULL));

ALTER TABLE families ALTER COLUMN invite_code_key SET NOT NULL;
ALTER TABLE families
    ADD CONSTRAINT families_name_sealed CHECK (substr(name, 1, 1) = '\x01'::bytea),
    ADD CONSTRAINT families_invite_code_sealed CHECK (substr(invite_code, 1, 1) = '\x01'::bytea),
    ADD CONSTRAINT families_greeting_places_sealed
        CHECK (substr(greeting_places, 1, 1) = '\x01'::bytea);
ALTER TABLE join_requests
    ADD CONSTRAINT join_requests_invite_code_used_sealed
        CHECK (substr(invite_code_used, 1, 1) = '\x01'::bytea);
ALTER TABLE chats
    ADD CONSTRAINT chats_title_sealed CHECK (substr(title, 1, 1) = '\x01'::bytea);

ALTER TABLE messages
    ADD CONSTRAINT messages_body_sealed CHECK (substr(body, 1, 1) = '\x01'::bytea);
ALTER TABLE message_reactions
    ADD CONSTRAINT message_reactions_emoji_sealed CHECK (substr(emoji, 1, 1) = '\x01'::bytea);
ALTER TABLE message_mentions
    ADD CONSTRAINT message_mentions_name_sealed CHECK (substr(name, 1, 1) = '\x01'::bytea);
ALTER TABLE poll_options
    ADD CONSTRAINT poll_options_text_sealed CHECK (substr(text, 1, 1) = '\x01'::bytea);
ALTER TABLE transcripts
    ADD CONSTRAINT transcripts_text_sealed CHECK (substr(text, 1, 1) = '\x01'::bytea);

ALTER TABLE notes
    ADD CONSTRAINT notes_text_sealed  CHECK (substr(text, 1, 1) = '\x01'::bytea),
    ADD CONSTRAINT notes_place_sealed CHECK (substr(place, 1, 1) = '\x01'::bytea);
ALTER TABLE note_task_items
    ADD CONSTRAINT note_task_items_text_sealed CHECK (substr(text, 1, 1) = '\x01'::bytea);
ALTER TABLE note_mentions
    ADD CONSTRAINT note_mentions_name_sealed CHECK (substr(name, 1, 1) = '\x01'::bytea);
ALTER TABLE pack_items
    ADD CONSTRAINT pack_items_label_sealed CHECK (substr(label, 1, 1) = '\x01'::bytea);

ALTER TABLE member_reports
    ADD CONSTRAINT member_reports_message_excerpt_sealed
        CHECK (substr(message_excerpt, 1, 1) = '\x01'::bytea);
ALTER TABLE assistant_reports
    ADD CONSTRAINT assistant_reports_message_excerpt_sealed
        CHECK (substr(message_excerpt, 1, 1) = '\x01'::bytea),
    ADD CONSTRAINT assistant_reports_note_sealed CHECK (substr(note, 1, 1) = '\x01'::bytea);

ALTER TABLE attachments
    ADD CONSTRAINT attachments_name_sealed      CHECK (substr(name, 1, 1) = '\x01'::bytea),
    ADD CONSTRAINT attachments_latitude_sealed  CHECK (substr(latitude, 1, 1) = '\x01'::bytea),
    ADD CONSTRAINT attachments_longitude_sealed CHECK (substr(longitude, 1, 1) = '\x01'::bytea),
    ADD CONSTRAINT attachments_waveform_sealed  CHECK (substr(waveform, 1, 1) = '\x01'::bytea),
    ADD CONSTRAINT attachments_file_key_sealed  CHECK (substr(file_key, 1, 1) = '\x01'::bytea);
ALTER TABLE attachments DROP COLUMN content_sha256;
