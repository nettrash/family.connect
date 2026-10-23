-- 0057_several_families — one account in several families (docs/protocol.md,
-- "Several families"; issue #88; decisions in docs/families-2026-10-22.md).
--
-- Until now `users.family_id` was the whole answer to "which family is this
-- account in", and it could only hold one. Membership becomes a table of its
-- own, `family_members`, and the column goes — DROPPED rather than left to
-- drift, because a column nobody writes any more is a column somebody will
-- read one day and get the answer from before. Every query that read it moves
-- to the table in the same change, and the compiler cannot help with SQL, so
-- the column's absence is what makes a missed one fail loudly at its first
-- execution instead of answering about the wrong family for ever.
--
-- JOINED_AT IS THE HOME FAMILY. The family an account joined first (earliest
-- `joined_at`, ties to the lower family id) is what every endpoint that names
-- no family means, and the whole world of an app that predates this change.
-- The backfill therefore dates each existing membership by the moment the
-- member joined that family's CHAT — the same timestamp `pass_ownership_on`
-- has always ordered successors by — and falls back to the account's own
-- registration only for a row that somehow has no family-chat membership
-- (the divergence `leave_family` already guards against loudly).
--
-- Scrubbed accounts are not members (0023 nulled their `family_id`), so they
-- carry nothing into the table; the assistant has no family by design (0015)
-- and carries nothing either.
CREATE TABLE family_members (
    family_id BIGINT      NOT NULL REFERENCES families(id) ON DELETE CASCADE,
    user_id   BIGINT      NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    joined_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (family_id, user_id)
);
-- The other direction — "which families is this account in, home first" —
-- is asked on every authenticated request, so it gets its own index in the
-- exact order the question sorts by.
CREATE INDEX family_members_user_idx ON family_members (user_id, joined_at, family_id);

INSERT INTO family_members (family_id, user_id, joined_at)
SELECT u.family_id,
       u.id,
       COALESCE(
           (SELECT cm.joined_at
              FROM chat_members cm
              JOIN chats c ON c.id = cm.chat_id
             WHERE c.family_id = u.family_id AND c.kind = 'family' AND cm.user_id = u.id),
           u.created_at)
  FROM users u
 WHERE u.family_id IS NOT NULL
   AND u.deleted_at IS NULL;

-- FORMER MEMBERS, per family. `users.deleted_family_id` remembered the ONE
-- family a scrubbed account had been in, for `former_members` alone; an
-- account deleted while in three families is a former member of all three.
-- ON DELETE CASCADE from the family for the reason 0023's SET NULL had: a
-- family that is itself deleted has no former members left to name.
CREATE TABLE former_family_members (
    family_id BIGINT NOT NULL REFERENCES families(id) ON DELETE CASCADE,
    user_id   BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    PRIMARY KEY (family_id, user_id)
);
INSERT INTO former_family_members (family_id, user_id)
SELECT deleted_family_id, id
  FROM users
 WHERE deleted_family_id IS NOT NULL
   AND deleted_at IS NOT NULL;

-- THE ASSISTANT'S CHAT IS PER FAMILY (decision D6): one `ai` chat per member
-- per family, under that family's owner's switches. Each existing thread is
-- first pointed at the family its member is in NOW — `ensure_ai_chat` used to
-- re-point a thread lazily on the next `GET /chats`, so a member who moved
-- family may still hold one naming the old family — and then the uniqueness
-- widens from "one per account" to "one per account and family".
UPDATE chats c
   SET family_id = u.family_id
  FROM users u
 WHERE c.kind = 'ai'
   AND c.user_a_id = u.id
   AND u.family_id IS NOT NULL
   AND c.family_id <> u.family_id;
DROP INDEX chats_ai_owner_uq;
CREATE UNIQUE INDEX chats_ai_owner_uq ON chats (user_a_id, family_id) WHERE kind = 'ai';

-- The two columns go last, once nothing above needs them. Dropping a column
-- drops its indexes and its foreign key with it (`users_family_id_idx`,
-- `users_family_id_fkey`, `users_deleted_family_idx`).
ALTER TABLE users DROP COLUMN deleted_family_id;
ALTER TABLE users DROP COLUMN family_id;

-- INVITATIONS (decision D4): an owner's invitation into their family, which
-- the invited person accepts or declines. PENDING ONLY — an invitation that
-- is accepted, declined, withdrawn or lapses is simply deleted, so there is
-- no status column to forget to filter on, and the unique pair is the
-- "already invited" answer `POST /families/{family}/invitations` gives back.
--
-- `invited_by` is SET NULL rather than CASCADE: accounts are scrubbed, never
-- deleted, and the scrub removes the invitations an account made itself; the
-- FK is only the backstop that could never take a family's invitation with
-- some future hard delete.
CREATE TABLE invitations (
    id         BIGSERIAL   PRIMARY KEY,
    family_id  BIGINT      NOT NULL REFERENCES families(id) ON DELETE CASCADE,
    user_id    BIGINT      NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    invited_by BIGINT      REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT invitations_pair_uq UNIQUE (family_id, user_id)
);
-- "My invitations" is read on every `GET /me`.
CREATE INDEX invitations_user_idx ON invitations (user_id);

-- REPORTS GO TO THE OWNER OF THE FAMILY THEY ARE FILED IN (docs/protocol.md,
-- "Several families"). "Already reported" was keyed on the reporter alone,
-- which was the family when an account had one; with several, a member who
-- reports the same person to two families' owners has filed two reports, one
-- in each owner's list — and folding the second into the first would answer
-- it with a report from ANOTHER family's inbox. So both partial unique
-- indexes gain the family, keeping every other column and predicate exactly
-- as 0029 wrote them (its comment explains why the person index keys on the
-- excerpt).
DROP INDEX member_reports_open_message_uq;
DROP INDEX member_reports_open_person_uq;
CREATE UNIQUE INDEX member_reports_open_message_uq
    ON member_reports (family_id, reporter_user_id, message_id)
    WHERE status = 'open' AND message_id IS NOT NULL;
CREATE UNIQUE INDEX member_reports_open_person_uq
    ON member_reports (family_id, reporter_user_id, reported_user_id)
    WHERE status = 'open' AND message_id IS NULL AND message_excerpt IS NULL;
