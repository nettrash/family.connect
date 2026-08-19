-- 0001_init — the complete v1 schema in a single migration.
--
-- Executed via sqlx::raw_sql (multi-statement) inside one transaction by the
-- hand-rolled migrator in src/migrate.rs. users <-> families is a circular
-- reference, so users.family_id gains its FK with ALTER TABLE after families
-- exists. chat_members holds explicit rows for ALL chats including the family
-- chat, so authorization checks and message fan-out are uniform across chat
-- kinds.

CREATE TABLE users (
    id            BIGSERIAL PRIMARY KEY,
    username      TEXT        NOT NULL,
    display_name  TEXT        NOT NULL,
    password_hash TEXT        NOT NULL,          -- argon2id PHC string
    family_id     BIGINT,                        -- NULL right after registration; FK added below
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX users_username_lower_uq ON users (lower(username));
CREATE INDEX users_family_id_idx ON users (family_id);

CREATE TABLE families (
    id            BIGSERIAL PRIMARY KEY,
    name          TEXT        NOT NULL,
    invite_code   TEXT        NOT NULL,
    join_policy   TEXT        NOT NULL DEFAULT 'approval' CHECK (join_policy IN ('open', 'approval')),
    owner_user_id BIGINT      NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX families_invite_code_uq ON families (invite_code);

ALTER TABLE users ADD CONSTRAINT users_family_id_fkey FOREIGN KEY (family_id) REFERENCES families(id) ON DELETE SET NULL;

CREATE TABLE sessions (
    id           BIGSERIAL PRIMARY KEY,
    user_id      BIGINT      NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash   TEXT        NOT NULL,           -- hex SHA-256 of the opaque bearer token
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at   TIMESTAMPTZ NOT NULL
);
CREATE UNIQUE INDEX sessions_token_hash_uq ON sessions (token_hash);
CREATE INDEX sessions_user_id_idx ON sessions (user_id);

CREATE TABLE join_requests (
    id               BIGSERIAL PRIMARY KEY,
    family_id        BIGINT      NOT NULL REFERENCES families(id) ON DELETE CASCADE,
    user_id          BIGINT      NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    invite_code_used TEXT        NOT NULL,
    status           TEXT        NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'approved', 'rejected')),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    decided_at       TIMESTAMPTZ,
    decided_by       BIGINT REFERENCES users(id) ON DELETE SET NULL
);
CREATE UNIQUE INDEX join_requests_pending_uq ON join_requests (family_id, user_id) WHERE status = 'pending';
CREATE INDEX join_requests_family_pending_idx ON join_requests (family_id) WHERE status = 'pending';

CREATE TABLE chats (
    id         BIGSERIAL PRIMARY KEY,
    family_id  BIGINT      NOT NULL REFERENCES families(id) ON DELETE CASCADE,
    kind       TEXT        NOT NULL CHECK (kind IN ('family', 'direct')),
    user_a_id  BIGINT REFERENCES users(id) ON DELETE RESTRICT,
    user_b_id  BIGINT REFERENCES users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chats_kind_shape CHECK (
        (kind = 'family' AND user_a_id IS NULL AND user_b_id IS NULL)
        OR (kind = 'direct' AND user_a_id IS NOT NULL AND user_b_id IS NOT NULL AND user_a_id < user_b_id)
    )
);
CREATE UNIQUE INDEX chats_family_singleton_uq ON chats (family_id) WHERE kind = 'family';
CREATE UNIQUE INDEX chats_direct_pair_uq ON chats (user_a_id, user_b_id) WHERE kind = 'direct';

CREATE TABLE chat_members (
    chat_id   BIGINT      NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    user_id   BIGINT      NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    joined_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (chat_id, user_id)
);
CREATE INDEX chat_members_user_idx ON chat_members (user_id);

CREATE TABLE messages (
    id            BIGSERIAL PRIMARY KEY,
    chat_id       BIGINT      NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    sender_id     BIGINT      NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    client_msg_id UUID        NOT NULL,
    body          TEXT        NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX messages_dedup_uq ON messages (chat_id, sender_id, client_msg_id);
CREATE INDEX messages_chat_id_id_idx ON messages (chat_id, id DESC);

CREATE TABLE chat_reads (
    chat_id              BIGINT      NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    user_id              BIGINT      NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    last_read_message_id BIGINT      NOT NULL DEFAULT 0,
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (chat_id, user_id)
);

CREATE TABLE devices (
    id         BIGSERIAL PRIMARY KEY,
    user_id    BIGINT      NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    platform   TEXT        NOT NULL CHECK (platform IN ('ios', 'android')),
    push_token TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX devices_push_token_uq ON devices (push_token) WHERE push_token IS NOT NULL;
CREATE INDEX devices_user_id_idx ON devices (user_id);
