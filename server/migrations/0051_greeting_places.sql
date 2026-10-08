-- 0051_greeting_places — today's weather in the daily greeting, for places
-- the family's owner chose (docs/protocol.md, "Today's weather, for places
-- the owner chose").
--
-- At most three place names, as the owner typed them and the server kept
-- them, whose forecast for the day the greeting mentions. NOT a switch: an
-- empty list is "no weather", and it is EMPTY by default for every family
-- before this and after it, on the rule every widening has followed —
-- nothing leaves for somebody new until somebody chose it. These names are
-- the one thing the greeting sends to a party that is not `processor` (the
-- weather provider), and only on a server with `[ai.lookups] weather` on.
--
-- NOT NULL with `{}`, never NULL: the protocol has no "unset" for the list,
-- and a client reads `[]` exactly as it reads an absent key.
--
-- The CHECK holds the SHAPE — one dimension, at most three, no NULL element
-- — because that is what a stray write could get wrong in a way every
-- reader would have to defend against. The rules about each NAME (trimmed,
-- whitespace folded, not empty, no control characters, at most 80
-- characters, no two equal once lower-cased) are `PATCH /families/mine`'s,
-- in code: a CHECK cannot compare names case-insensitively the way the
-- server does, and the handler must answer which rule was broken rather
-- than a constraint name.
ALTER TABLE families
    ADD COLUMN greeting_places TEXT[] NOT NULL DEFAULT '{}'
    CONSTRAINT families_greeting_places_check CHECK (
        cardinality(greeting_places) <= 3
        AND COALESCE(array_ndims(greeting_places), 1) = 1
        AND array_position(greeting_places, NULL) IS NULL
    );
