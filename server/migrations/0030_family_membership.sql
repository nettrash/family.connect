-- 0030_family_membership — two questions about the same door, on the
-- same row, written through the same PATCH under the same lock: how
-- many people may be inside, and whether the door opens at all
-- (docs/protocol.md, "Families").
--
-- The DROP names a constraint PostgreSQL generated from the inline column
-- CHECK in 0001_init.sql:25. The name is VERIFIED, not guessed:
--   SELECT conname, pg_get_constraintdef(oid) FROM pg_constraint
--    WHERE conrelid = 'families'::regclass;
-- answers `families_join_policy_check | CHECK ((join_policy = ANY
-- (ARRAY['open'::text, 'approval'::text])))`. It is deliberately NOT
-- `DROP CONSTRAINT IF EXISTS`: on a database where the auto-name differs
-- this must fail loudly at boot rather than leave behind a constraint that
-- silently refuses every 'closed'. The replacement is named EXPLICITLY, the
-- way `families_language_check` (0017) is, so the next widening drops a
-- name somebody chose.
ALTER TABLE families DROP CONSTRAINT families_join_policy_check;
ALTER TABLE families ADD CONSTRAINT families_join_policy_check
    CHECK (join_policy IN ('open', 'approval', 'closed'));

-- NULLABLE, and the contrast with `ai_history` (0019, NOT NULL DEFAULT
-- TRUE) is the one 0017 drew for the language: a switch has no third state,
-- a cap does. "We never set one" is not "we set one equal to whatever the
-- operator's ceiling happens to be today", and the ceiling moves between
-- boots. A NOT NULL DEFAULT would also answer the question on behalf of
-- every family that already exists, including one already larger than the
-- number chosen — which would put it in violation of a rule nobody in it
-- ever set.
ALTER TABLE families ADD COLUMN max_members INTEGER;

-- One member is the fewest a family can have and still have an owner:
-- `create_family` makes its creator member #1, so a cap of 0 would be a
-- family nobody could be in, including the person who made it.
--
-- The operator's ceiling is deliberately NOT in this CHECK: it lives in
-- config, it moves between boots, and a CHECK against a moving number turns
-- a config edit into rows that can no longer be UPDATEd. Nor does the CHECK
-- compare the cap to the family's current size — an owner who wants no more
-- members must be able to say so without removing anybody first. The cap is
-- read AT THE DOOR (join and approve), never enforced over the room.
ALTER TABLE families ADD CONSTRAINT families_max_members_positive
    CHECK (max_members IS NULL OR max_members >= 1);
