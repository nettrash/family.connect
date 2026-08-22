-- 0005_replies — quoting one message from another.
--
-- The reply is a property of the reply itself, decided once at send time and
-- never changed (protocol.md, "Replies"), so it needs no sequence cursor of
-- its own — unlike reactions, whose mutations an `after_id` catch-up could
-- never see.
--
-- What clients render (the quoted author and an excerpt) is deliberately NOT
-- stored here. It is recomputed from the parent row on every read, so a
-- quote cannot go on showing text its author has since edited.
--
-- No ON DELETE clause: nothing in this server deletes a message, and the
-- default RESTRICT is the honest expression of that — a future delete
-- feature has to decide what a dangling quote means rather than silently
-- cascading.

ALTER TABLE messages ADD COLUMN reply_to_message_id BIGINT REFERENCES messages(id);
