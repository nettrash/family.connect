-- 0061_paid_families — paid family hosting (docs/protocol.md, "Paid
-- families"; issue #89).
--
-- Every family that exists now is FREE: the operator's own, and any other
-- already here, keep working with nothing to do when the operator first
-- turns `[subscriptions] enabled` on. A family created while subscriptions
-- are on is made 'paid' by the server; the operator moves one either way
-- with `family-connect families plan`.
--
-- `unpaid_since` is when the server noticed a paid family's owner no longer
-- covers it; it is deleted `unpaid_days` later. `unpaid_warned` records the
-- "tomorrow" push, so the hourly sweep sends it once.
ALTER TABLE families
    ADD COLUMN plan TEXT NOT NULL DEFAULT 'free'
        CONSTRAINT families_plan_known CHECK (plan IN ('free', 'paid')),
    ADD COLUMN unpaid_since TIMESTAMPTZ,
    ADD COLUMN unpaid_warned BOOLEAN NOT NULL DEFAULT false;

CREATE INDEX families_unpaid_idx ON families (unpaid_since) WHERE unpaid_since IS NOT NULL;

-- The UUID a purchase carries as Apple's `appAccountToken`, so the server can
-- tell whose purchase it is and Apple learns nothing about the account. Made
-- the first time `GET /me` needs it, and sealed under the account's key like
-- everything else that identifies somebody (#59).
ALTER TABLE users
    ADD COLUMN app_account_token BYTEA
        CONSTRAINT users_app_account_token_sealed
            CHECK (substr(app_account_token, 1, 1) = '\x01'::bytea);

-- One row per App Store subscription an account presented, found by Apple's
-- `originalTransactionId` — which stays the same across renewals, upgrades
-- and restores — through its HMAC (#59's lookups). The id itself is kept
-- beside it sealed under the account's key, as a username is beside its
-- lookup: `storage-key rotate` recomputes every lookup from the value next
-- to it, and an HMAC with nothing beside it would be lost to a rotation.
--
-- It holds STATE, not events: the transaction with the latest expiry seen
-- (its own HMAC in `latest_transaction_key`, so a refund of it can be told
-- from a refund of an older period), whether it was revoked, and the newest
-- renewal info. `signed_at` and `renewal_signed_at` are Apple's own
-- `signedDate`s, so a notification that arrives out of order changes
-- nothing it should not.
CREATE TABLE subscriptions (
    id                       BIGSERIAL   PRIMARY KEY,
    user_id                  BIGINT      NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    original_transaction_id  BYTEA       NOT NULL
        CONSTRAINT subscriptions_original_transaction_sealed
            CHECK (substr(original_transaction_id, 1, 1) = '\x01'::bytea),
    original_transaction_key BYTEA       NOT NULL
        CONSTRAINT subscriptions_original_transaction_uq UNIQUE,
    latest_transaction_id    BYTEA       NOT NULL
        CONSTRAINT subscriptions_latest_transaction_sealed
            CHECK (substr(latest_transaction_id, 1, 1) = '\x01'::bytea),
    latest_transaction_key   BYTEA       NOT NULL,
    product_id               TEXT        NOT NULL,
    environment              TEXT        NOT NULL,
    expires_at               TIMESTAMPTZ NOT NULL,
    revoked_at               TIMESTAMPTZ,
    signed_at                TIMESTAMPTZ NOT NULL,
    renews                   BOOLEAN,
    grace_until              TIMESTAMPTZ,
    renewal_signed_at        TIMESTAMPTZ,
    created_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at               TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX subscriptions_user_idx ON subscriptions (user_id);
