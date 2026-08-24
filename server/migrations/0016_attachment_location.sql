-- Locations: the fifth attachment kind, and the first with no bytes.
--
-- Every other kind is a file the server stores and hands back. A location
-- is three numbers. Putting them in the row rather than in an uploaded blob
-- is what lets a bubble draw the pin the moment the message arrives —
-- fetching 40 bytes over HTTP before anything can be shown would make the
-- one attachment that needs no download the slowest to appear.
--
-- The consequence, carried through in handlers_attachment.rs: a location
-- row names a storage_key like every other row (the column is NOT NULL and
-- shared code reads it) but no file is ever written there, and
-- `GET /attachments/{id}` on one refuses rather than 500s. `Storage::remove`
-- already treats a missing file as normal, so the sweeper and the retention
-- pass need no special case.

ALTER TABLE attachments DROP CONSTRAINT attachments_kind_check;
ALTER TABLE attachments ADD CONSTRAINT attachments_kind_check
    CHECK (kind IN ('photo', 'video', 'audio', 'file', 'location'));

ALTER TABLE attachments ADD COLUMN latitude   DOUBLE PRECISION;
ALTER TABLE attachments ADD COLUMN longitude  DOUBLE PRECISION;
-- Reported accuracy radius in metres, when the sending device knew one. A
-- pin drawn without it claims a precision no phone has.
ALTER TABLE attachments ADD COLUMN accuracy_m INTEGER;

-- DOUBLE PRECISION, not NUMERIC and not a fixed grid: a rounded coordinate
-- moves the pin, and at the sixth decimal place that is about 10 cm — which
-- is finer than any phone, so nothing is lost by keeping what was sent.

-- The two halves of one rule, written as an equivalence so it holds in both
-- directions: a location has coordinates, and nothing else does. Without
-- the second half a photo could carry a latitude that no code reads and no
-- client draws — a claim about where someone was, stored for nothing.
ALTER TABLE attachments ADD CONSTRAINT attachments_location_has_coordinates
    CHECK ((kind = 'location') = (latitude IS NOT NULL AND longitude IS NOT NULL));

-- Range, so a typo in a client cannot store a point that is not on Earth.
-- Longitude is closed at both ends: -180 and +180 are the same meridian and
-- refusing one of them would refuse a real place.
ALTER TABLE attachments ADD CONSTRAINT attachments_coordinates_in_range
    CHECK (
        (latitude IS NULL OR latitude BETWEEN -90 AND 90)
        AND (longitude IS NULL OR longitude BETWEEN -180 AND 180)
        AND (accuracy_m IS NULL OR accuracy_m >= 0)
    );

-- A location never has a preview: there is nothing to downscale, and each
-- device draws its own map from the coordinates.
ALTER TABLE attachments ADD CONSTRAINT attachments_location_has_no_preview
    CHECK (kind <> 'location' OR has_preview = false);
