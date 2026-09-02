-- 0031_note_content_seq — the seq a BADGE counts (docs/protocol.md, "Board").
--
-- `board_seq` answers "what has changed?", which is what a SYNC cursor
-- needs and why a move has to take one: the change feed carries moves, or a
-- drag on one device never reaches another. It is the wrong number for a
-- badge, which claims there is something to READ — and a wall somebody
-- tidied has nothing new to read on it.
--
-- So a note carries a second stamp, taken from the SAME sequence so the two
-- are comparable, and reset only when the note's `text` changes. Creation
-- sets it; a move, a resize and a recolour leave it exactly where it was.
-- `content_seq <= board_seq` therefore holds for every row, always.
--
-- It is a column on the note and not a flag on the change entry BECAUSE the
-- feed is a state feed. A note edited and then dragged five times appears
-- once, in the state it is now in: a flag describing the LAST event would
-- say "only a move" and lose the edit for every client that was away across
-- both. A column survives the collapse.
--
-- BACKFILL: `board_seq`, not 0 and not a new value. Nothing here knows
-- which of a note's past mutations were rewrites, and `board_seq` is the
-- only bound that is certainly not too LOW — the last thing that happened
-- to this note happened at that seq, so "its text was last written no later
-- than then" is true of every row. Too high is safe (a client whose mark is
-- above it simply does not badge an old note); too low would badge notes
-- nobody touched. Devices that have shown the board before seed their mark
-- past all of it anyway (protocol.md, "Board").
--
-- NOT NULL DEFAULT 0 only for the instant of the backfill; the default is
-- dropped afterwards so that a row inserted without one is a compile error
-- in the handler rather than a note that can never badge.

ALTER TABLE notes ADD COLUMN content_seq BIGINT NOT NULL DEFAULT 0;
UPDATE notes SET content_seq = board_seq;
ALTER TABLE notes ALTER COLUMN content_seq DROP DEFAULT;
