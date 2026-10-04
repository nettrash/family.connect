-- 0050_lookups — the assistant looking things up (docs/protocol.md,
-- "Looking things up").
--
-- THE OWNER'S SWITCH, the seventh: whether the assistant may send a query it
-- wrote to the web search, weather and Wikipedia providers the operator
-- configured in [ai.lookups]. Those are NOT `processor` — the first parties
-- the assistant has ever reached that are not — so this is FALSE by default,
-- for every family before this and after it, on the rule every widening has
-- followed: nothing leaves for somebody new until somebody chose it. Bound to
-- no other switch, and inert on a server with no lookup source.
ALTER TABLE families ADD COLUMN ai_lookups BOOLEAN NOT NULL DEFAULT FALSE;

-- THE MEMBER'S OWN CONSENT to those recipients, the second timestamp beside
-- 0047's `assistant_consent_at` and of its exact shape: a moment rather than
-- a bit, null for every existing row — nobody is opted in by a schema change
-- — and withdrawable. It stands only on top of the first: withdrawing the
-- assistant consent clears this one too. Without it the member's questions
-- declare no lookup tool, and a mention that declares them leaves this
-- member's words out of its transcript.
ALTER TABLE users ADD COLUMN assistant_lookup_consent_at TIMESTAMPTZ;

-- The transcript filter reads it per sender for every lookup-enabled mention,
-- as it reads the first (0047's index, for the same reason).
CREATE INDEX users_assistant_lookup_consent_idx ON users (id)
    WHERE assistant_lookup_consent_at IS NOT NULL;

-- WHAT IT COST. A web search is billed per call, so a family reading only
-- tokens would see it as free — the argument 0032 made for `images`. The
-- calls to the Brave or SearXNG provider that came back with an answer, per
-- completed reply; weather and Wikipedia are free and are not counted.
ALTER TABLE ai_usage ADD COLUMN searches INTEGER NOT NULL DEFAULT 0;

-- THE DAILY CAP, which is the operator's bill rather than a statistic: every
-- web search the server SENDS for a family, by UTC day — including those a
-- reply that later failed made, which `ai_usage` never records. Reserved
-- atomically before each call (an upsert that refuses past the cap), so two
-- replies at once cannot both slip past it. One row per family per day that
-- searched, pruned by the reservation itself; gone with the family.
CREATE TABLE lookup_search_days (
    family_id BIGINT  NOT NULL REFERENCES families(id) ON DELETE CASCADE,
    day       DATE    NOT NULL,
    searches  INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (family_id, day)
);
