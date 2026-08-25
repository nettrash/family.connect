-- The one language a family actually speaks to each other in
-- (docs/protocol.md, "The family's language").
--
-- NULLABLE, and that is the decision this migration is really about. A
-- `NOT NULL DEFAULT 'en'` would have been one line shorter and would have
-- declared, on behalf of every family that already exists, that they speak
-- English — and afterwards there is no way left to tell "we never chose"
-- from "we chose English", because both look the same in the column.
--
-- The value is checked HERE as well as in the handler, and deliberately so:
-- the handler's list is what gives a client a readable refusal, but this is
-- what stops a hand-edited row from putting a tag nobody ships into a
-- prompt. Nine values, the same nine the apps are translated into, spelled
-- the way iOS and Android spell them. Both script variants are in the list
-- on purpose: `sr` and `sr-Latn` are one language in two alphabets, and a
-- family that reads one cannot read the other.
ALTER TABLE families ADD COLUMN language TEXT;

ALTER TABLE families ADD CONSTRAINT families_language_check
    CHECK (language IS NULL OR language IN
        ('en', 'de', 'es', 'fr', 'ja', 'ru', 'sr', 'sr-Latn', 'zh-Hans'));
