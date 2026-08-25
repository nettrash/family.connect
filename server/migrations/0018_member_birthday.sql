-- A member's birthday: a day and a month, and no year at all
-- (docs/protocol.md, "Birthdays").
--
-- No year because nobody should have to publish their age to be wished a
-- happy birthday, and the year is the only part of a date that carries one.
-- That also rules out DATE, which cannot hold a day and a month without
-- inventing a year for them — and any year invented here would be a year
-- somebody eventually reads as real.
--
-- Two SMALLINTs rather than one packed integer so the range checks below
-- can say what they mean.
ALTER TABLE users ADD COLUMN birthday_month SMALLINT;
ALTER TABLE users ADD COLUMN birthday_day   SMALLINT;

-- The two halves of one fact, written as an EQUIVALENCE so it holds in both
-- directions, exactly as the location constraint in 0016 does. A half-set
-- birthday — a month with no day, a day with no month — is not a state any
-- reader should have to handle, and the cheapest way to keep it out of the
-- readers is to make it impossible to write.
ALTER TABLE users ADD CONSTRAINT users_birthday_is_whole
    CHECK ((birthday_month IS NULL) = (birthday_day IS NULL));

-- The day is checked against ITS month: 31 April is not a date, and neither
-- is 30 February. 29 February IS one, because there is no year here for it
-- to fail to exist in — which is the one place where dropping the year
-- makes the rule looser rather than stricter.
ALTER TABLE users ADD CONSTRAINT users_birthday_in_range
    CHECK (
        birthday_month IS NULL
        OR (birthday_month BETWEEN 1 AND 12
            AND birthday_day >= 1
            AND birthday_day <= CASE birthday_month
                                    WHEN 2 THEN 29
                                    WHEN 4 THEN 30
                                    WHEN 6 THEN 30
                                    WHEN 9 THEN 30
                                    WHEN 11 THEN 30
                                    ELSE 31
                                END)
    );
