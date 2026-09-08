-- 0036_assistant_is_not_removable — the assistant's account survives every
-- sweep, as a database fact rather than a rule each query has to remember.
--
-- The account 0015 seeds is an ordinary `users` row, which is what lets
-- `messages.sender_id` keep its NOT NULL foreign key — and also what makes
-- it reachable by anything that removes users. Two rules already protect it
-- from the familyless sweep: 0034 leaves its `familyless_since` NULL, and
-- the sweep's own scan excludes it by name. Both live in Rust, in ONE query
-- each, and neither is inherited by a cleanup written later against the
-- same table.
--
-- So the invariant moves to where every writer meets it. It is not "the
-- sweep spares the assistant" but the stronger, simpler statement the rest
-- of the server assumes everywhere: THERE IS ALWAYS A LIVE ROW NAMED
-- `assistant`. Three ways to break it are refused —
--
--   * tombstoning it     (`deleted_at` set, which is what `scrub_account` does),
--   * renaming it        (`assistant_user_id` finds the row BY NAME, so a
--                         rename is as fatal as a delete and much quieter),
--   * deleting the row   (no server path does this, but a hand-run ops
--                         statement would, and it would cascade into every
--                         message the assistant ever sent).
--
-- Changing its `display_name` or `avatar_version` stays allowed: those are
-- what the assistant looks like, not whether it exists.
--
-- To remove it DELIBERATELY (re-seeding it under a new id, say):
--     ALTER TABLE users DISABLE TRIGGER users_assistant_not_removable;
--     ...
--     ALTER TABLE users ENABLE TRIGGER users_assistant_not_removable;
-- which is the point — it takes a sentence that says what it is doing.
--
-- The first plpgsql in this schema. `sqlx::raw_sql` speaks the simple query
-- protocol, so the dollar-quoted body reaches PostgreSQL intact and the
-- runner's one-transaction-per-migration still applies.

CREATE FUNCTION assistant_is_not_removable() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION
        'the assistant account (users.id=%) cannot be removed or renamed', OLD.id
        USING ERRCODE = 'restrict_violation';
END;
$$;

CREATE TRIGGER users_assistant_not_removable
    BEFORE UPDATE ON users
    FOR EACH ROW
    WHEN (lower(OLD.username) = 'assistant'
          AND (NEW.deleted_at IS NOT NULL
               OR lower(NEW.username) IS DISTINCT FROM 'assistant'))
    EXECUTE FUNCTION assistant_is_not_removable();

CREATE TRIGGER users_assistant_not_deleted
    BEFORE DELETE ON users
    FOR EACH ROW
    WHEN (lower(OLD.username) = 'assistant')
    EXECUTE FUNCTION assistant_is_not_removable();
