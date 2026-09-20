-- 0047_assistant_consent — each member's own permission for their words to
-- go to the model (docs/protocol.md, "Consenting to the assistant").
--
-- ON THE USER AND NOT ON THE FAMILY, because it is the member's to give: the
-- owner's `ai_history` and `ai_vision` decide what the family's chat exposes,
-- and neither is permission from the people whose words that history is made
-- of. A member who leaves one family and joins another has still agreed.
--
-- A TIMESTAMP rather than a boolean, for the reason a report freezes its
-- excerpt: "yes" is a thing that happened at a moment, and an operator asked
-- when somebody agreed — by a regulator, or by the member — cannot answer
-- from a bit. Null is "has not agreed", which is also every row this
-- migration creates: nobody is opted in by a schema change, and the members
-- of a server that has been running for a year are all asked again, once.
--
-- Withdrawal sets it back to NULL and deletes nothing else. The member's `ai`
-- chat and its messages stay exactly where they are: consent going away is
-- not a request to lose a conversation, and re-consenting resumes it.
ALTER TABLE users ADD COLUMN assistant_consent_at TIMESTAMPTZ;

-- The history filter reads this per sender for every `@ai` mention that
-- carries family context (protocol.md: "`ai_history` carries only the words
-- of members who have consented"), so it is read far more often than written.
CREATE INDEX users_assistant_consent_idx ON users (id)
    WHERE assistant_consent_at IS NOT NULL;
