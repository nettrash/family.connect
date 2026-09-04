-- 0028_member_blocks — one member deciding not to read another
-- (docs/protocol.md, "Blocking a member").
--
-- THE PAIR IS THE WHOLE ROW. No surrogate id, no state column, so PUT and
-- DELETE are idempotent for free and "is this blocked" is a primary-key
-- lookup rather than a scan with a predicate. That is the shape 0022 gave
-- poll votes for the same reason.
--
-- NO family_id, and that is the decision this migration is really about. A
-- block is a statement about a PERSON, not about a membership, so it has to
-- survive one of them leaving and the two of them meeting again — exactly
-- as `chats_direct_pair_uq` keys a direct chat by its pair and merely
-- refreshes `family_id` when a pair reunites somewhere else. A block scoped
-- to a family would quietly lapse on a rejoin, which is the one moment
-- somebody most wants it to hold.
--
-- The self-block is refused HERE rather than only in the handler, the
-- argument 0018 makes about a half-set birthday: a state no reader should
-- have to handle is cheapest to make impossible to write.
--
-- Both CASCADES never fire. 0023 SCRUBS a users row rather than deleting
-- it, so the cleanup is the handler's — and it is deliberately ASYMMETRIC:
-- rows where the deleted account is the BLOCKER go, with their avatar and
-- their birthday, because a preference is part of the account; rows where
-- it is the BLOCKED stay, because their messages stay in the family chat
-- and somebody else's decision not to read them was never theirs to revoke.
CREATE TABLE member_blocks (
    blocker_user_id BIGINT      NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    blocked_user_id BIGINT      NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (blocker_user_id, blocked_user_id),
    CONSTRAINT member_blocks_not_self CHECK (blocker_user_id <> blocked_user_id)
);

-- The referencing side PostgreSQL never indexes for you. The primary key
-- leads on `blocker_user_id` and covers that foreign key and the "who have
-- I blocked" read; nothing would cover the other direction, and the other
-- direction is the hot one — the push gate asks "who has blocked THIS
-- sender" once per message. Same rule 0006, 0022 and 0023 all state.
CREATE INDEX member_blocks_blocked_idx ON member_blocks (blocked_user_id);
