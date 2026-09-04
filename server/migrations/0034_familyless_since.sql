-- 0034_familyless_since — since when an account has had no family
-- (docs/protocol.md, "Accounts without a family").
--
-- The familyless sweep scrubs an account that has been without a family
-- for longer than `[families] familyless_account_ttl_days`, exactly as
-- `POST /me/delete` would. It needs to know SINCE WHEN, and `created_at`
-- is the wrong answer for anybody who was ever a member: a member removed
-- today from a year-old account would be swept tonight, with none of the
-- grace the rule promises. So the moment is its own column, and it is
-- maintained at the three places membership changes:
--
--   * registration leaves the DEFAULT, now() — the clock starts;
--   * `grant_membership` (create, join, approve) sets it NULL — it stops;
--   * `remove_membership` (leave, remove) sets it to now() — it restarts.
--
-- NULL therefore means "not a candidate", and it is deliberately the same
-- NULL for the three rows that are never candidates: a member, a tombstone
-- (the scrub blanks it too), and the assistant's account, which has no
-- family by design and is the one row 0015 inserted with `family_id`
-- NULL on purpose.
--
-- The backfill starts every existing familyless account's clock NOW, at
-- the upgrade, rather than at its registration: the grace is a promise the
-- server was not making before, and an account that has been sitting here
-- a year is still owed the full period from the day the rule appeared.
-- `ADD COLUMN ... DEFAULT now()` is exactly that — PostgreSQL evaluates a
-- volatile default once and writes it into every existing row — and the
-- UPDATE then blanks the rows that are not candidates.
--
-- The index is partial because the column is NULL for every member, which
-- on a family server is very nearly every row; the sweep's scan is over
-- the handful that are not.
ALTER TABLE users ADD COLUMN familyless_since TIMESTAMPTZ DEFAULT now();
UPDATE users
   SET familyless_since = NULL
 WHERE family_id IS NOT NULL
    OR deleted_at IS NOT NULL
    OR lower(username) = 'assistant';
CREATE INDEX users_familyless_since_idx ON users (familyless_since)
    WHERE familyless_since IS NOT NULL;
