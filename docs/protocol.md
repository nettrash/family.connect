# Family Connect protocol — v1

This document is the single source of truth for the API between the Family Connect server
(`server/`) and the iOS (`ios/`) and Android (`android/`) clients. When code and this document
disagree, this document wins; fix the code.

## Transport

- REST: JSON over HTTP under `{base}/api/v1`. `{base}` is the user-entered server URL
  (e.g. `https://chat.example.com` or `http://192.168.1.10:8080`).
- Realtime: WebSocket at `{base}/api/v1/ws` (scheme `wss://` for https servers, `ws://` for http).
- In production the server binds to loopback and sits behind nginx, which terminates TLS and
  proxies both REST and the WebSocket upgrade.

## Compatibility rules

- Clients MUST ignore unknown JSON fields in any response, and unknown `type` values in any
  WebSocket frame. The server likewise ignores unknown client frame types (logged at debug).
  This is how the voice-call signalling frames (`call_offer`, `call_answer`, …) were added
  without breaking v1 clients — see "Voice calls".
- All field names are `snake_case`. Timestamps are RFC 3339 UTC strings
  (e.g. `"2026-08-19T17:03:12Z"`). All ids are 64-bit integers.

## Authentication

- Opaque per-device session tokens. `POST /auth/register` and `POST /auth/login` each create a
  new session and return its token exactly once. The server stores only a SHA-256 hash.
- Every other endpoint (and the WebSocket upgrade) requires `Authorization: Bearer <token>`.
- Sessions have a sliding expiry (default 180 days, refreshed by use). A `401` means the session
  is gone — the client wipes local state (keeping the server URL) and returns to login.
- A password change revokes sessions, which is what makes it useful for recovery rather than just
  hygiene: changing your own password ends every OTHER session you have, and an owner resetting a
  member's password ends ALL of theirs. Those devices find out the ordinary way — their next call
  answers `401` and they return to login.
- Deleting the account ends every session it has, the calling one included, and takes the device
  rows with them. See "Deleting an account".

## Error shape

Non-2xx responses carry:

```json
{"error": {"code": "username_taken", "message": "username is already in use"}}
```

`code` is a stable machine-readable string; `message` is a human-readable English sentence.
Canonical codes: `unauthorized`, `invalid_credentials`, `username_taken`, `validation`,
`already_in_family`, `family_full`, `not_in_family`, `not_family_owner`, `invalid_invite_code`,
`join_request_pending`, `join_request_not_pending`, `user_already_in_family`, `owner_cannot_leave`,
`cannot_remove_owner`, `cannot_dm_self`, `cannot_block_self`, `blocked`, `cannot_report_self`,
`report_not_pending`, `not_same_family`, `user_not_found`, `chat_not_found`, `not_chat_member`,
`message_empty`, `message_too_long`, `message_not_found`, `not_message_author`, `invalid_emoji`,
`note_not_found`, `not_note_author`, `invalid_note_color`, `invalid_note_size`, `invalid_language`,
`board_full`, `invalid_pagination`, `device_not_found`, `invalid_poll`, `poll_closed`,
`calls_disabled`, `video_calls_disabled`, `invalid_call`, `call_not_found`, `call_busy`,
`peer_busy`, `peer_unreachable`, `avatar_too_large`, `invalid_image`, `attachment_too_large`,
`invalid_attachment`, `attachment_not_found`, `attachment_already_used`, `internal`.

`owner_cannot_leave` is RETIRED: no endpoint raises it any more (see `POST /families/leave`). It
stays listed because clients that predate the hand-off still branch on it, and a code that vanishes
from this document is a code somebody deletes from a client that is still talking to an old server.

## Objects

```json
User      {"id": 7, "username": "anna", "display_name": "Anna", "created_at": "…",
           "avatar_version": 3}
          — plus "birthday": {"month": 3, "day": 14} when (and only when) one is set
          — plus "deleted": true when (and only when) that account has been deleted. Such
            a user has no usable username, no picture and no birthday, and their
            "display_name" is the English placeholder "Deleted account" — a client that
            knows the flag SHOULD draw its own translation instead of that text; see
            "Deleting an account"
Member    {"id": 7, "username": "anna", "display_name": "Anna", "role": "owner|member",
           "avatar_version": 3}
          — plus "birthday": {"month": 3, "day": 14} when (and only when) one is set
          — plus "deleted": true likewise. A deleted account is never in "members"; it
            appears in "former_members" (see GET /families/mine) and exists there for one
            reason — so a client can still put a name to the messages, notes and reactions
            it left behind
          — a Member carries NOTHING about blocking, deliberately. Whether one member has
            blocked another depends on WHO IS READING, and this object also travels inside
            the "member_deleted" frame, which is serialised once and sent to many — the same
            rule the Polls section states for "did I vote". The caller's own list rides on
            GET /me and GET /families/mine as "blocked_user_ids" instead, which is also the
            only shape that can CARRY a blocked former member — resolving that id to a
            name is the client's job, and may not be possible (see "Blocking a member")
Family    {"id": 3, "name": "The Smiths", "join_policy": "open|approval|closed",
           "created_at": "…", "ai_history": true}
          — plus "invite_code": "ABCD2345" when (and only when) the caller is the owner
          — plus "max_members": 8 when (and only when) the owner has set a cap. ABSENT means
            the family has no cap of its own and only the operator's ceiling binds — absent is
            NOT the ceiling, for the same reason absent "language" is not English. A cap has a
            real third state where a switch has none: "we never set one" is not "we set one
            equal to whatever the ceiling happens to be today", and the ceiling moves between
            server restarts. A client therefore holds TWO numbers — this one and
            "max_family_members" from GET /me — and the door takes the LOWER of them. They
            can disagree, and legitimately: a family that set 40 under a ceiling of 50 goes
            on reporting 40 after the operator drops the ceiling to 10, because a stored cap
            is never re-validated when the ceiling moves (that would make one config edit
            lock owners out of their own settings screen). So a client shows this number as
            the family's own setting and the lower of the two as the seats actually left
          — plus "language": "ru" when (and only when) the owner has chosen one; absent
            means unset, and unset is NOT English — see "The family's language"
          — "ai_history" is ALWAYS present, unlike the two above. It is a boolean with a
            real default (true), so there is no "unset" for a missing key to mean and a
            client never has to guess one — see "Mentioning the assistant in the family chat"
JoinRequest {"id": 12, "user": {User}, "created_at": "…"}
Report    {"id": 4, "reporter": {User}, "reported": {User},
           "reason": "spam|harassment|inappropriate|other", "created_at": "…"}
          — plus "message_id": 1338 when (and only when) one message was named AND that
            message still exists
          — plus "message_excerpt": "…" when (and only when) one message was named. This is
            the WHOLE body, FROZEN at the moment the report was raised, named for
            symmetry with "reply_to.excerpt" and, unlike it, never CUT — a moderator
            judging a message has to see all of it — and it is the one quotation in
            this protocol that is STORED rather than recomputed. Two things would
            otherwise leave the owner an empty screen: the author may edit the body afterwards
            through the ordinary author-only path, which has no time limit, and retention
            deletes the message outright. "message_id" is dropped once retention has swept
            that message; the excerpt stays, so a client draws the excerpt always and offers
            to jump to the message only while the id survives
          — plus "message_attachments": [{"kind": "photo", "name": null}] when (and only when) the
            reported message carried any, trimmed exactly as a chat-list preview is: kind and name,
            no dimensions and no coordinates. A photo sent without a caption has an empty body, and
            "inappropriate" is very often exactly that message — the excerpt is then the empty
            string (present, because a message was named) and this is the only thing on the row that
            says what was reported. The bytes are not copied, so a client offers to open the
            attachment only while "message_id" survives, exactly as it offers the jump
Chat      {"id": 42, "kind": "family|direct|ai", "title": "The Smiths", "peer_user_id": 9|null}
          — "title" is the family name for the family chat, the peer's display name for direct,
            and the assistant's name for "ai";
            "peer_user_id" is set for direct chats only
Message   {"id": 1338, "chat_id": 42, "sender_id": 7,
           "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
           "body": "Dinner at 7?", "created_at": "…"}
          — plus "reactions": [Reaction] and "reaction_seq": 123 when (and only when) the
            message has ever been reacted to. After the last reaction is removed the fields
            stay present with "reactions": [] — clients distinguish "cleared" from "no data".
          — plus "reply_to": {ReplyTo} when (and only when) the message is a reply.
          — plus "edited_at": "…" and "edit_seq": 88 when (and only when) the body has been
            edited. Both absent on a message still in its original form.
          — plus "attachments": [Attachment] when the message carries photos, videos,
            audio, files or a location — 1 to 10 of them, in the order the sender chose;
            absent (never an empty array) on a message that carries none.
          — plus "attachment": {Attachment}, the FIRST element of "attachments", kept for
            clients that predate plurality; a client that reads "attachments" ignores it.
            The two are never present without each other.
          — plus "poll": {Poll} when (and only when) the message is a poll. A poll's
            QUESTION is the message body, so a client that knows nothing of polls shows it
            as an ordinary message and loses only the options — see "Polls".
          — plus "call": {Call} when (and only when) the message is the record of a voice
            call. The body is then the English placeholder "Voice call" / "Missed voice
            call", which a client that knows the object never shows — see "Voice calls".
ReplyTo   {"message_id": 41, "sender_id": 9, "excerpt": "See you at six"}
Reaction  {"user_id": 9, "emoji": "❤️"}
Attachment {"id": 34, "kind": "photo|video|audio|file|location", "mime": "image/jpeg",
            "size": 182734, "width": 1600, "height": 1200, "duration_ms": 8400,
            "has_preview": true, "name": "receipts.pdf",
            "latitude": 55.7558, "longitude": 37.6173, "accuracy_m": 12}
           — "duration_ms" on videos and audio; "width"/"height" absent when the
             uploader could not determine them; "name" is REQUIRED on a file (its whole
             identity) and optional on audio and a location, where it is a label;
             a file, a piece of audio and a location never have a preview, and only a
             photo or video has dimensions;
             "latitude"/"longitude" on a location only, and always both — a location
             IS its coordinates; "accuracy_m" additionally when the sending device
             reported one
Poll      {"poll_seq": 88, "closed": false,
           "options": [{"id": 5, "text": "Pizza", "votes": [7, 9]},
                       {"id": 6, "text": "Pasta", "votes": []}]}
          — the poll's QUESTION is the message body and is deliberately not a field here.
            "options" is 2–10 of them in creation order and never changes for the life of
            the poll; "votes" is the full current list of user ids that chose that option,
            so a Poll is complete state and never a delta. "poll_seq" and "closed" are
            ALWAYS present, unlike "reaction_seq": a poll has a sequence from the moment
            it exists, and "closed" is a boolean with a real default, so there is no
            "unset" for a missing key to mean
Call      {"outcome": "completed|missed|declined|failed", "duration_secs": 222,
           "video": true}
          — "video" present when (and only when) it was a VIDEO call; absent on a voice
            call, like every optional field on this wire
          — "duration_secs" when (and only when) the call was ever answered: the seconds
            from the answer to the end on the server's clock, so a "failed" call may carry
            one and a "missed" call never does. The record's sender is the CALLER and its
            "client_msg_id" is the call's id — see "Voice calls"
IceServer {"urls": ["turn:turn.example.com:3478?transport=udp"],
           "username": "1756300000:7", "credential": "…"}
          — "username"/"credential" on a TURN server only, and only when the operator
            configured credentials; a STUN server is "urls" alone — see "Voice calls"
Note      {"id": 12, "author_id": 7, "text": "Milk", "color": "yellow", "size": "medium",
           "x": 0.42, "y": 0.13, "created_at": "…", "updated_at": "…", "board_seq": 88}
          — plus "deleted": true INSTEAD of the content fields on a tombstone; see "Board"
```

**A body is plain text on the wire, and always has been.** No markup is parsed, transformed or
validated by the server: `body` goes out exactly as it came in, the 4000-character limit counts the
characters that were typed, `reply_to.excerpt` is a cut of that same raw text, and a push carries it
verbatim. What a client does when DRAWING it is a client's business, and clients do render a
markdown subset in the bubble — emphasis, inline code, strikethrough, fenced code blocks, headings
(`#`, `##`, `###`), bullet and numbered lists, and tables — so a message written on one platform
reads the same on another. That is a rendering convention rather than a wire format: a client that
renders none of it shows the source, which is still exactly what was written, and nothing about a
message means anything different because of it. Anything that needed the server to agree — a stored
rich body, a sanitiser, a markup flag — would belong here instead, and does not exist.

The subset grew, and the wire did not, which is the whole point of it being a convention. What
pushed it was the assistant: ask for three options and it answers with a list, ask it to compare
two things and it answers with a table, and until the bubbles drew them those arrived as literal
hashes and pipe characters in front of the family.

**Only the three chat bubbles render it** — the family chat, a direct chat, and the assistant's
own. Every other surface shows the same body as SOURCE, and that is deliberate in each case: the
excerpt above a quoted reply and the preview line under a chat in the list are one line with
nowhere to lay a table out; a push body is drawn by the operating system and never by the app; and
copy and share must hand on the characters that were typed rather than one client's idea of what
they looked like. The visible consequence is that `reply_to.excerpt` is a cut of raw text at 120
characters and can land in the middle of a table row or inside a fence, quoting half a `|---|---|`.
That is worth saying rather than hiding, because the only way to avoid it is a server that
understands the markup well enough to cut on a boundary — which is exactly what the paragraph above
says does not exist.

`avatar_version` counts how many times that user has set a profile picture: `0` means they have
none and clients draw initials. It is a cache key, not a URL — the bytes come from
`GET /users/{id}/avatar`, and because the version changes on every upload, clients may cache a
fetched picture forever under `(user_id, avatar_version)` and never revalidate.

That licence rests on one server guarantee: **a version is never reused for a different
picture.** Deleting a picture reports `avatar_version: 0`, but the underlying counter does not go
back — the next upload after a delete reports `2`, not `1` again. Without this a client that had
cached the first picture under version `1` would keep showing it after the owner deleted it and
uploaded a different one.

### Replies

A reply is decided once, at send time, and never changes afterwards. The request names the message
being answered with `reply_to_message_id`; it must be a message in the SAME chat, and anything else
— including a real message in a chat the caller cannot see — is `message_not_found`, so the
endpoint never confirms that an id exists elsewhere.

What comes back is not that id but a `reply_to` snippet, which the server RECOMPUTES on every read
rather than storing:

```json
"reply_to": {"message_id": 41, "sender_id": 9, "excerpt": "See you at six",
             "parent": {"message_id": 38, "sender_id": 4, "excerpt": "What time?"}}
```

Recomputed, because a quoted message can be edited later — a snippet frozen at send time would go
on showing text its author has since changed. `excerpt` is the parent's body cut to at most 120
characters (counted in Unicode scalar values, never cut mid-scalar) and exists only so a client can
draw the quote without having the original in its cache; a client that wants the whole message
reads it from its own history or pages back to it, keyed by `message_id`.

**`parent` carries ONE more level, and exactly one.** When the quoted message is itself a reply, its
own quote comes along, so answering an answer shows both halves of the exchange rather than a
snippet with no idea what it was responding to. It is deliberately a DIFFERENT shape from `reply_to`
— it has no `parent` of its own — so the depth cap is structural: there is no field to recurse into,
and a message four deep still shows exactly two. Unbounded nesting would let one old thread drag its
whole ancestry into every page.

`parent` is absent whenever there is nothing to show: the quoted message was not itself a reply, or
its own parent has since been swept by retention (the reply FK is `ON DELETE SET NULL`, migration
0012). Clients degrade to a single quote silently — a missing second level is normal, not an error.
It is cut to the same 120 scalar values, the same way, and is recomputed on every read exactly as
`reply_to` is.

`last_message` in `GET /chats` and push payloads still carry no quote at all — neither level.

Replies are ordinary messages in every other respect — they take part in `after_id` catch-up,
reactions and unread counts exactly as any other message does, and nothing about them mutates, so
they need no sequence cursor of their own.

Every mutation of a message's reactions (set, replace, remove) takes the next value of one
server-wide sequence and stamps it on the message as `reaction_seq`; each chat exposes the
maximum such value over its messages as `max_reaction_seq` in `GET /chats`. Together they give
clients a monotonic cursor for reaction catch-up, exactly as message ids drive `after_id`.

### Editing

Only the author may edit, there is no time limit, and only the body changes — id, sender,
timestamp, reactions and any reply the message carries all survive untouched. An edit is not a
new message: it never re-notifies, never bumps an unread count, and never moves the chat's
ordering.

Editing has the same catch-up problem reactions have, and takes the same shape. `after_id` is
`WHERE id > cursor`, so it can never see a change to an OLDER row — a client that was offline
while a message three pages back was edited would never learn of it. So every edit takes the next
value of a second server-wide sequence, stamped on the message as `edit_seq`, with each chat
exposing its maximum as `max_edit_seq` in `GET /chats`, and `GET /chats/{id}/edits?after_seq=`
replaying what changed. It is a SEPARATE sequence from `reaction_seq`, deliberately: consolidating
the two would change the shape of an endpoint deployed clients already speak.

The catch-up returns whole `Message` objects rather than a bespoke patch, so a client applies them
through exactly the same path as a page of history.

**Applying an edit is guarded, and this is load-bearing.** A client must overwrite a stored body
only when the incoming `edit_seq` is greater than or equal to the one it holds (absent counts as
`0`). Without that guard, a history page fetched BEFORE an edit — but delivered after it — quietly
restores the old text, and the two devices in a family disagree about what was said.

A quote is a snapshot of the body at read time (see "Replies"), so editing a quoted message changes
what later readers see quoted. Clients that hold the quoting message locally should refresh its
excerpt when they apply the edit, cutting it the same way the server does.

### Polls

A poll is an ordinary message that happens to be votable. Any member may start one **in the family
chat**, and it behaves like every other message everywhere else in this protocol: it counts as
unread, it pushes once when it arrives, it can be replied to, edited and reacted to, and the
retention sweep takes it away with everything else of its age.

**The question is the message body.** The `Poll` object carries the options and the votes and
nothing else. That is the whole reason a poll costs so little: a chat-list preview, a push alert,
a reply excerpt and the assistant's transcript all read the question already, without one new case
between them, and a client that has never heard of polls draws "Pizza or pasta?" as a plain message
and loses only the buttons. The price is that a poll cannot have a caption separate from its
question, and that a poll's body — unlike a message carrying an attachment — may **not** be empty:
`message_empty` applies to a poll with no question.

Polls are refused in a direct chat and in the assistant's. A poll is a family deciding something
together; between two people it is a question, and the answer is the next message. Anywhere but the
family chat is `invalid_poll`.

Options are fixed at creation: 2–10 of them, each trimmed, non-empty, at most 100 characters, and
no two the same ignoring case. A poll can never gain, lose or rename an option — the votes already
cast were cast against the list as it was read, and there is no honest way to re-point them.
Editing the message edits the QUESTION, through the ordinary author-only edit path, and that is as
far as changing a poll goes.

**One choice, and you may change it.** `PUT …/vote` names the option the caller now holds and
`DELETE …/vote` retracts it. Like a reaction it is an idempotent state-set rather than a toggle:
re-PUTting the option already held is a no-op that burns no sequence value and fans nothing out, and
whether tapping your current choice means "keep it" or "clear it" is a decision each client makes
locally. Multiple choice is not part of this; a poll asks one question and takes one answer.

**Votes are attributed.** Every option carries the full list of user ids that chose it, so a client
draws who voted for what and, more usefully in a family, who has not voted at all. This is not only
a product choice: one frame is serialised once and sent to every connection, so a field whose value
depends on who is reading it — "did I vote" — cannot exist. Clients derive that from the list.

Closing a poll is the author's, and one-way. It is an authorship act exactly as editing is, and the
family owner does not outrank an author here for the same reason they cannot edit or delete anybody
else's message anywhere else in this protocol. A closed poll keeps its result and refuses further
votes with `poll_closed`; closing a closed poll is a no-op.

Voting has the same catch-up problem reactions and edits have, and takes the same shape for the same
reason: `after_id` is `WHERE id > cursor` and can never see a change to an older row. Every change
to a poll — a vote, a retraction, a close — takes the next value of a fourth server-wide sequence
and stamps it on the poll as `poll_seq`, with each chat exposing its maximum as `max_poll_seq` in
`GET /chats` and `GET /chats/{id}/polls?after_seq=` replaying what changed. A separate sequence
again, deliberately: folding it into an existing one would change the shape of an endpoint deployed
clients already speak.

The `poll` frame and the catch-up feed both carry a poll's **full current state**, never a delta, so
ordering races resolve locally: a client applies one only when its `poll_seq` is greater than the
value it holds for that message. Sequence values are assigned before commit, so a catch-up read can
skip a not-yet-committed lower value on a *different* poll — the same accepted gap `after_id` has
for message ids, and self-healing here for the same reason it is for reactions, because any later
vote on the same poll re-delivers the whole thing.

A poll dies with its message and nothing has to remember to take it: the retention sweep, a direct
chat going, and a member deleting their account all remove messages, and the poll, its options and
its votes go with them.

### Board

Each family has exactly one board: a wall of sticker notes anyone in the family can add to and
rearrange. Notes are not messages — they carry no unread count and never appear in a chat.

A block hides a note's CONTENT and its author from the blocker, and nothing else about the board
changes. In particular the split rule stands: position is everyone's, so a blocked member may still
move a note the blocker wrote and the blocker's board still rearranges. Refusing the move would be a
tell, and there is no refusal here that a blocked person could not otherwise explain. The note
ceiling (`board_full`) is a fact about the FAMILY and is never projected per reader: a note the
blocker hides still occupies its slot — any quantity a third member can measure must mean the same
thing for everybody.

A hidden note keeps its slot, its size, its colour and its tilt, and draws the same "Hidden —
blocked member" placeholder a hidden row does, with no author line under it. It reveals on ONE TAP,
the same peek a hidden row gets: per note, per device, never on the wire, never stored, and gone on
the next launch. The reveal is not a nicety here but the only safe answer to the tap, because the
tap already does something — it opens the note editor, which would put the text on screen that the
note was hiding, and a client that merely made hidden notes inert would still leak them through
every other way in. Dragging is unchanged whether a note is hidden or revealed: position is
everyone's, and a note that could not be tidied because of who wrote it is a difference the blocker
has to explain to themselves every time they touch the wall.

**A NEW note does notify.** This reverses the original "raise no notification": a board nobody is
told about is a board nobody reads, and a note pinned to the family wall is exactly the kind of
thing meant to be seen. Only CREATION notifies. Edits, moves and deletes do not — tidying the wall
is the shared act (see below), and a push for every drag would make the board unusable. The author
is never notified of their own note. Everything else about push applies unchanged, which means
per device: a member with the board open on a desktop still gets the note on the phone in their
pocket, because the socket that delivered the `board_note` frame delivered it to the desktop and
to nothing else.

Counting what is new is the CLIENT's job, and it needs a marker of its own. `max_board_seq` is a
SYNC cursor — it advances whenever a client applies a change, including a background resync — so
using it to mean "seen" would clear the badge for a user who never opened the board. A client keeps
a separate high-water mark of the note ids it has SHOWN the user, advanced only when the board is
actually opened, and counts live notes above it. Note ids are server-assigned and monotonic, so
"created since you last looked" needs nothing further from the server.

Who may do what: **anyone in the family may MOVE a note; only its author may change its text,
colour or size, or delete it.** Moving is the shared act (tidying the wall together), authorship is
the personal one.

`x` and `y` are fractions of the board, `0.0`–`1.0` from the top-left, so a note sits in the same
relative place on a phone and a tablet. Values outside that range are CLAMPED rather than
rejected — a drag that ends past the edge should stick to the edge, not fail. `color` is one of
`yellow`, `pink`, `blue`, `green`, `orange`, `purple`; anything else is `invalid_note_color`.
`text` is trimmed, non-empty and at most 280 characters.

**A note has a size**, so the thing that matters this week can be made big enough to read from
across the room and a passing remark can stay small. `size` is one of `small`, `medium`, `large`;
anything else is `invalid_note_size`, and a note created without one is `medium` — the size every
note had before the field existed, so nothing already on a wall changed when it arrived. It is a
STEP, not a measurement: a fixed name each client draws at its own idiom (a `large` note is
bigger on a Mac than on a phone, as everything is) with a larger type size and more lines of the
text showing, the same way `color` is a name and not a hex value. A free width and height in board
fractions would have made the same note a third of a phone and a third of a desktop window, and
left the type size undefined. Size belongs to the AUTHOR, with text and colour: how loudly a note
speaks is the writer's call, and a size anyone could change is a size anyone could shrink to
nothing. Resizing takes a `board_seq` like any other mutation and does not notify, like a move.

Every board mutation — create, edit, move, delete — takes the next value of a third server-wide
sequence and stamps it on the note as `board_seq`, with the family exposing its maximum as
`max_board_seq`. It is the same catch-up machinery as reactions and edits, for the same reason:
`after_id` cannot see a change to an older row, and a board is nothing BUT changes to older rows.

The change feed carries each note **once, in the state it is now in** — it is a state feed, not an
event log. A note created and then moved five times appears as a single entry with its latest
`board_seq`, which is what lets a client apply entries idempotently and in any order, exactly as the
reaction feed carries a message's full reaction state rather than a delta.

**Deletes leave tombstones.** A deleted note keeps its row, takes a new `board_seq`, and appears in
the change feed as `{"id": 12, "deleted": true, "board_seq": 91}` with no content. Without that a
client who was offline when a note was removed would go on showing it forever — there is no other
signal that it is gone. The full-board read never returns tombstones; only the change feed does.

### The family's language

A family may declare the ONE language it actually speaks to each other in. It is unset until
somebody sets it, and **unset is not English.** A `NOT NULL DEFAULT 'en'` would have been a shorter
migration and would have declared, on behalf of every family that already exists, that they speak a
language most of them do not — and afterwards there is no way left to tell "we never chose" from
"we chose English".

The value is one of the nine locales the apps themselves ship in, spelled the way the platforms
spell them: `en`, `de`, `es`, `fr`, `ja`, `ru`, `sr`, `sr-Latn`, `zh-Hans`. A fixed list rather
than any well-formed BCP 47 tag, for the same reason the board's colours are a fixed list: the only
thing that reads this is the assistant, and a family that typed something the server merely
accepted would get answers they could not explain and no error to explain them. Anything outside
the list is `invalid_language`. Casing is not significant coming in — `sr-latn` is the same choice
as `sr-Latn` — and what goes back out is always the canonical spelling above, so a client can
compare it against its own list without normalising first.

Two of the nine name a SCRIPT, and that is on purpose. `sr` and `sr-Latn` are one language in two
alphabets — a family that reads Cyrillic cannot read the answer that comes back in the other one —
and `zh-Hans` is there for the same reason. A resolution rule that kept only the language and threw
the script away would quietly give two of the nine choices back.

Only the OWNER may set it, through `PATCH /families/mine`, where `"language": null` clears it back
to unset and omitting the key entirely leaves it alone. Every member SEES it: it rides on the
`Family` object, so it reaches a client from `GET /families/mine` and from `GET /me` alike — which
matters, because `/me` is what a client bootstraps from.

What it is FOR, today, is one thing: the assistant answers in it when it is asked in the family
chat (see "Mentioning the assistant in the family chat"). It is deliberately NOT a display
language — clients go on drawing their interface in whatever the device is set to, and a family
setting that silently re-languaged somebody's phone would be a surprise nobody asked for. It is
also deliberately not applied to a member's private assistant thread, for the reason given there.

### Birthdays

A member may have a birthday, and it is **a day and a month, with no year**. Nobody should have to
publish their age to be wished a happy birthday, and the year is the only part of a date that
carries one. The consequence is deliberate rather than tolerated: 29 February is a perfectly good
birthday here, because there is no year for it to fail to exist in, and a client that wanted to
show an age has nothing to compute one from and should not try.

```json
"birthday": {"month": 3, "day": 14}
```

One object rather than two sibling fields, because the two halves are a single fact: a birthday is
either there or it is not, and "month but no day" is not a state anything should have to handle.
Absent means unset — the key is simply not present, exactly as `invite_code` is not present for a
non-owner — and it rides on both `User` and `Member`, so it arrives with `GET /me`,
`GET /families/mine` and the join-request list without any of them needing a new call.

`month` is 1–12 and `day` must be a day THAT month has: 31 April is `validation` and so is 30
February, while 29 February is accepted. The check is the server's rather than each client's, so
that three apps cannot end up disagreeing about which dates exist.

Two endpoints write it, and which one a client uses is decided by whose birthday it is.
`PUT /me/birthday` sets your own, and its shape is deliberately the avatar's — a `PUT` that
replaces whatever was there and a `DELETE` that clears it — because it is the same kind of thing:
a small optional piece of a profile that is either set or is not.
`PUT /families/members/{user_id}/birthday` is the owner filling one in for somebody else, which is
what makes the family calendar usable at all: a parent knows a child's birthday, and the child is
never going to open a settings screen to type it. The owner's endpoint is scoped to their own
family, and a user outside it answers `not_same_family` whether or not they exist — the same
refusal, and for the same reason, as the password reset: this is personal information, and an
endpoint that answered differently for a real stranger than for an id nobody holds would be a way
to enumerate accounts.

Unlike the password reset, the owner MAY point the roster endpoint at their own row. The reset
refuses that because it would be a way around proving you know the current password; a birthday has
no such proof to skip, the owner can already set their own with `PUT /me/birthday`, and both paths
write the same two numbers. Refusing would buy nothing and would make every roster screen carry a
special case for exactly one row.

**A birthday change raises no WebSocket frame and no push.** There is no `member_updated` in this
protocol, and this is not the feature to invent one for: a birthday is set about once, it is not
time-critical to the second, and both clients already re-read `GET /families/mine` on every
resync — which is where they learn it, along with anything else about the roster that moved while
they were away. The device that made the change has the new value in its own response.

### The assistant

Each member may have one private chat with an assistant, and **private is the whole point**: it
belongs to that member alone, and no other member can read it. The assistant can also be spoken to
in the family chat, but only by being asked for by name — see "Mentioning the assistant in the
family chat" below, which is a separate surface with a rule of its own about what it is shown, and
a family setting that decides which of two that rule is.
`kind` is `"ai"`, `user_a_id` is its owner, and `GET /chats` returns it only to them.

**In an `ai` chat the assistant is only ever shown that member's own AI thread.** Not the family
chat, not another member's AI chat, not anything anyone else wrote. This is the invariant that lets
a self-hosted family server talk to a hosted model at all: a member asking a question sends their
own words, and nothing anybody else said leaves the server unasked.

The family chat is the one other place the assistant can be reached, and it is a **separate surface
with its own rule about what it is shown**. That rule was once "one message and nothing else,
always"; it is now the family's own choice between exactly that and the recent history of the family
chat, and the choice belongs to the owner alone (`ai_history`, described in full below). This section
used to say that such a widening "would need saying so here first" — this is it being said. What a
mention may send is enumerated under "Mentioning the assistant in the family chat", both settings
are enumerated there, and nothing outside those lists is sent at either setting.

Two things did NOT widen with it, and neither of them is a setting anybody can turn:

- **In an `ai` chat the assistant is still only ever shown that member's own thread**, exactly as
  above. The family chat never reaches a private thread and a private thread never reaches the
  family chat — the two surfaces cannot see each other's history in either direction, whatever the
  family has chosen. A member's own words stay theirs.
- **A direct chat is never consulted by the ASSISTANT.** Not by a mention, not by a private thread,
  at any setting, ever. Two people talking one-to-one are the one conversation in this protocol that
  no OTHER conversation reads. The one thing that can ever lift a line out of a direct chat is the
  person who received it, deliberately, by reporting it: that excerpt reaches the family owner — or,
  when the report names the owner, the operator's log instead — and nobody else, and it is frozen
  there (see "Reporting a member").

Talking to it is an ordinary send — `POST /chats/{id}/messages` — and what comes back is ordinary
too. The server:

1. stores the member's message and fans it out, exactly like any other message;
2. immediately stores an EMPTY assistant message and fans that out, so every one of the member's
   devices has a row to fill;
3. streams the text as it is generated, as `ai_delta` frames naming that message id;
4. writes the finished text to the row, which takes an `edit_seq` and fans out `message_edited`.

**The deltas are cosmetic and the row is the truth.** A client that was not listening — asleep, on
another screen, connected halfway through — gets the whole reply from the edits feed it already
speaks (see "Editing"), with no special path. That is why the assistant's reply is a normal message
rather than a bespoke object: reactions, replies, retention, catch-up and search all work on it
without a line of new code.

```json
{"type": "ai_delta", "chat_id": 42, "message_id": 1339, "text": "Sure — the "}
```

A reply that fails midway leaves the row with whatever text arrived and an `ai_error` frame; the
member sees a partial answer and can ask again, which is better than a bubble that never resolves.

The assistant sends under a **reserved account** that belongs to no family, so `sender_id` stays a
real user id and every foreign key, join and index over messages keeps working untouched. It is not
in the `members` roster (`GET /families/mine` selects by family, and the assistant has none) and the
username is refused at registration so nobody can impersonate it. It cannot be messaged directly:
`POST /chats/direct` naming it answers `not_same_family`, because it is in no family. The only two
ways to reach it are its own `ai` chat and a mention in the family chat.

In an `ai` chat clients need no special id: it has exactly two participants, so a message in one
that is not yours is the assistant's. Draw it with the chat's own name and icon rather than looking
the sender up in the roster, where it will not be found.

The family chat has no such shortcut — it has many senders — so `GET /families/mine` names the
assistant outright when the server has one:

```json
"assistant": {"user_id": 1, "display_name": "Assistant", "mention": "@ai"}
```

The field is **absent when the server is not configured for an assistant**, and that absence is the
whole of the capability check: a client with no `assistant` must not offer `@ai` in the composer,
because typing it would produce nothing. It is deliberately not part of `members` — the assistant is
not a member, cannot be removed, made owner, given a password or messaged one-to-one, and every
screen that lists people would need a special case for it.

**The assistant answers in the language the question was asked from.** A client sends
`Accept-Language` with its send (Apple's URLSession does automatically; Android sets it from the
app's own resolved locale), and the server appends one instruction to the configured system prompt
naming that language. Only the FIRST tag is read — the header may be a whole weighted list, and
what is wanted is "the language this device is in", not a negotiation — and of that tag, the
language and the script. A region is dropped, because a region is not a language: `ru-RU` is
Russian and `de-CH` is German. A script is kept, because a script is very nearly one: `sr-Latn` is
Serbian written in Latin letters, and a family that reads Cyrillic cannot read the answer that
comes back if the `Latn` is quietly thrown away.

That instruction goes LAST in the prompt, and there is always exactly ONE of it. Last, because
everything above it is English — the configured prompt usually is and the mention note certainly
is — and a model handed a paragraph of English instructions answers in English unless the final
line says otherwise. Exactly one, because when no language can be resolved at all the instruction
becomes "answer in the language of the message you were sent": what the model would have done
anyway, said out loud so the English above it cannot win by default. Two sentences about language,
or none, is precisely the failure this ordering exists to prevent.

This is deliberately per-DEVICE rather than per-account: the same person may run the app in Russian
on their phone and English on a work Mac, and each question should come back in the language it was
asked from. It is also one system prompt rather than one per language — what the assistant is FOR
does not change with the language, only what it answers in.

All of that is the rule for a member's PRIVATE thread. The family chat resolves the language
differently and says why under "Mentioning the assistant in the family chat".

The feature is OFF unless the server is configured for it (`[ai] enabled`, an endpoint, a deployment
and a key). A server without it simply never creates the chat, and `POST` to one that does not exist
is the usual `chat_not_found`.

#### Mentioning the assistant in the family chat

Writing **`@ai`** in the family chat asks the assistant a question in front of everyone. The answer
arrives as an ordinary message in that chat, quoting the message that asked, and every member sees
both — there is nothing private about this half.

**What leaves the server is one of two lists, and a single family setting decides which.**

`ai_history` is a boolean on the `Family` object. Only the owner may change it, through
`PATCH /families/mine`; it is one family-wide switch with no third state and no per-member
override; and it is **`true` by default**, for families created after it existed and for every
family that existed before. Defaulting the other way would have been the more cautious sentence to
write and the wrong thing to ship: asking about something said last week is the entire reason a
family mentions the assistant in the chat rather than in a private thread, and a feature that stays
off until somebody finds a switch is a feature most families never see. Turning it off restores
the older behaviour exactly — nothing else about a mention changes with it.

With **`ai_history: false`**, what leaves the server is exactly this and nothing else:

- the body of the message that contains the mention;
- the body of the message it is a reply to, when the member deliberately replied to one, with that
  author's display name so the assistant can tell the two apart.

Not the messages before it. Not the messages after it. Not the roster, not the family name, not the
board, not anyone's private assistant thread, and not the previous mention or its answer — **a
mention has no memory**, so a follow-up must repeat what it is about. The quoted message is included
only because the member chose it by replying, which is an explicit act by somebody in the chat.

With **`ai_history: true`** the same two things go, and in addition a **transcript of what was
recently said in this family chat** — that, and nothing more:

- one line per message, `[YYYY-MM-DD HH:MM UTC] Display Name: what was said`, ordered oldest
  first. The timestamps are the point rather than decoration: "when did we say we'd go" is not
  answerable by a model that has been handed a month of undated sentences. The zone is spelled out
  on every line and again in the note that introduces the transcript, which also says that the
  family did not see UTC and that the assistant has not been told what they did see. **No timezone
  is stored for a family and none travels on the wire** — not with a message, not on the `Family`
  object — so nothing here can convert, and the alternative to saying so is a model answering
  "16:03" to a family who all read 19:03 on their own phones;
- display names, the same names the family sees, because an answer about what Anna said needs to
  know which of these lines are Anna's;
- the assistant's own past replies as well — they are part of what was said, and a conversation
  handed back with its own half missing reads as a room of people talking past each other. Those
  lines carry the assistant's CONFIGURED name, the same `assistant.display_name` that
  `GET /families/mine` reports and every client draws it under, and the note names it too so the
  assistant can recognise which lines are its own;
- **not** the mentioning message. That is the question; it already reaches the model as the
  question, and the transcript is strictly what came before it.

The transcript is given as a NOTE, the same mechanism the mention instruction already uses, and not
as conversation turns. What the family said last Tuesday is context, not an instruction addressed to
the assistant, and the two must not be confusable by something whose whole job is following
instructions.

**The window has an arithmetic ceiling: the intersection of three bounds.** The last **30 days**,
the newest **200 messages**, and **40 000 characters**. Filling runs backwards from the mention and
stops at whichever bound is reached first, so what survives is always the newest. Three bounds
rather than one because each catches a different family: one that says ten things a month would
otherwise send a year of them, one that says a thousand things a day would otherwise send a morning,
and one member pasting a novel would otherwise send the novel. There is no per-family tuning and no
rate limiting layered on top — the reason for naming the three numbers here is that the cost of a
mention should be something a reader can point at rather than estimate.

**A message that is not text contributes a placeholder plus whatever words rode with it.** A photo
with a caption is `[photo] look at this`; without one it is `[photo]`. The five kinds are
`[photo]`, `[video]`, `[voice note]`, `[file] receipts.pdf` and `[location] Grandma's house` — a
file by its name, which is its whole identity, and a location by its label. A kind added later that
this list has not been extended for renders as the bare word `[attachment]`, never as anything about
itself.

Several attachments contribute one placeholder each, space-separated and in the sender's order —
`[photo] [photo] [video] beach day` — each under the same rules as if it were alone.

**A location contributes its label or the bare `[location]`, and NEVER its coordinates.** That is
the rule a coordinate already lives under everywhere else here — it may not reach a log, an alert or
a push body — and a third-party model is the strictest case of all of them, because what reaches it
leaves the building for good. `[location] Grandma's house` says a member shared a place and what
they called it. `[location]` says a member shared a place. Neither says where anybody was.

A message with neither a body nor an attachment contributes nothing and is skipped: the empty row an
answer streams into is exactly such a message, and a transcript quoting blank lines back at the
assistant is noise it would try to explain. Bodies go in raw, exactly as stored — the server parses
markup nowhere else and this is not where it starts.

**The assistant is told which of the two it was given**, in its prompt: what it can see, and what it
still cannot — with the transcript, that means nothing older than the window, no member's private
thread, and no direct chat. Both halves matter. Without the first it invents the context it thinks
it is missing; without the second, a model told it can see one message while a month of them sits in
front of it answers "I cannot see the conversation" to a question about the conversation.

**And it is told what the transcript IS**: the conversation itself rather than a month of
instructions addressed to it, with its own earlier replies among the lines, and only the message
that mentioned it a question for it to answer. The obvious phrasing — these were written between
the family, not to you — is false of every line the assistant wrote itself, and a model told
nothing in front of it is its own answers "I can't see our earlier conversation" with its own reply
four lines up.

The mention grammar is part of this contract, because the server decides from it whether anything is
sent at all and each client draws the same token as a highlight — a client that highlighted
something the server ignores is a family watching a question go unanswered with no way to tell why:

- the token is `@ai`, matched case-insensitively over ASCII, so `@AI` and `@Ai` are mentions;
- the `@` must start the body or follow a character that is not an ASCII letter, digit or `_` —
  which is what stops `anna@ai.example` being one;
- the `i` must end the body or be followed by a character that is not an ASCII letter, digit or `_`
  — which is what stops `@aiden` being one.

The boundary test is ASCII-only on purpose. A Unicode one would refuse `@ai` written against
Japanese or Russian text with no space after it, and would make three implementations depend on
three Unicode tables agreeing; the ambiguity it exists to resolve only arises in ASCII anyway.

**Only the family chat.** A direct chat is two people who each already have a private assistant, and
a third party appearing in a conversation that had two is not something either of them asked for. A
mention in an `ai` chat is just text — that chat already answers everything.

**In the family chat the FAMILY's language wins.** When the owner has set one (see "The family's
language") the answer comes back in it, whatever the asking device happens to be set to. When they
have not, it falls back to that device's own `Accept-Language`, resolved exactly as a private
question is. With neither, nothing is named and the assistant answers in the language the mention
itself was written in. That order and no other, because the answer appears in front of everyone:
the language that matters here is the family's, not that of whoever happened to be holding a phone.
A member whose work Mac runs in English, asking a question in a family that speaks Russian, gets an
answer the rest of the family can read.

The private thread keeps the per-device rule unchanged, and the difference between the two is not
an inconsistency: a private thread has exactly one reader, and it is the person holding the device.

The sequence is the one above, with two differences: the reply carries `reply_to` naming the
mentioning message, and the `ai_delta` frames go to **every member of the family chat**, not one
person, so the whole family watches the answer arrive. Fragments are coalesced server-side to a few
frames a second: a per-fragment frame to every connected member is how a slow socket's outbound
queue fills, and a full queue closes that connection.

Exactly one notification is raised per answer, and it carries the finished text. The empty
placeholder is fanned out without a push — an alert saying the assistant sent a blank line is worse
than no alert — and the completing edit does not push either, so the alert is raised explicitly once
the body exists.

A server with no assistant configured ignores `@ai` completely: it is three characters of ordinary
text, stored and delivered like any other, and `GET /families/mine` carries no `assistant` field to
suggest otherwise.


**A block does not reach the assistant.** The family-chat history a mention is shown is built once,
for a mention that is usually not the blocker's, and its answer is posted as a real message the WHOLE
family reads. Filtering it per-asker would therefore change the text everybody sees, and the omission
would be inferable by the very person the block is meant to be silent towards; refusing `@ai`
outright to a member who has blocked somebody would be visible to the whole family. So the transcript
is not filtered. A block hides bubbles; it does not edit the assistant's memory, and a client should
say so where it offers the block.

### Photos, videos, audio, files and locations

A message may carry attachments — up to ten of them: photos, videos, audio, files, in any mix.
(For a long time it was one, and "sending three photos makes three messages" was the rule; a
family sends albums, and ten separate bubbles for one beach afternoon was the wire being honest at
the reader's expense.) Order is the sender's: the `attachment_ids` array is preserved, and every
read returns the attachments in exactly that order. Two rules keep the shape sane: a LOCATION is
always alone — a place is a statement, not a page of an album, and a bubble that is half map and
half photos serves neither — and a poll still carries no attachment at all. Claiming is
all-or-nothing: if ANY id in the array cannot be claimed (already used, swept, not the caller's),
the whole message is refused and nothing is written — the same transaction rule the single
attachment always had, extended to the set.

**Uploading is a separate step from sending.** The bytes go up first, on their own request, and the
message that follows names the attachment by id. A 100 MB video and a 30-byte message have nothing
in common — coupling them would put the whole upload inside the send retry, and there would be
nowhere to show progress.

```
POST /attachments?kind=video&width=1920&height=1080&duration_ms=8400   (raw bytes)
  → 201 {attachment: {id: 34, …}}
PUT  /attachments/34/preview                                          (raw JPEG bytes)
  → 204
POST /chats/42/messages  {client_msg_id, body: "", attachment_id: 34}
  → 201 {message: {…, attachment: {…}}}
```

Metadata rides in the QUERY STRING and the bytes are the whole body: no multipart, so the server
can stream straight to disk without parsing anything.

**The server never decodes an image or a video.** It checks the declared type against the file's
magic number and stores what it is given, exactly as it does for avatars. That means the PREVIEW —
the downscaled photo, or the poster frame of a video — is produced and uploaded by the client. A
message may be sent before its preview arrives; `has_preview` says whether one is there yet.

#### Audio

`kind=audio` covers both halves of the same thing: a sound file picked from disk, and a voice note
recorded on the spot. They are one kind because a bubble plays them identically and the server
cannot tell them apart anyway — what a member cares about is that it is something to listen to.

It carries `duration_ms` like a video, and **no preview**: `has_preview` is always false and
`PUT /attachments/{id}/preview` on one is `invalid_attachment`. There is nothing to look at. A
client draws a play control, the duration, and a scrubber — deliberately not a waveform, which
would be a second artefact to generate, upload and version for something the ear does not need.

The magic-number check applies, as it does to photos and video: the declared type must match what
the bytes are. A recording that a client cannot encode into a checkable container should be sent as
`kind=file` instead, where nothing is verified.

`name` is optional for audio, unlike a file: a voice note has no name worth showing (its duration
is its identity), but a track picked from disk does, and a client that has one may send it.

#### Files

`kind=file` is the fourth kind, and it is the one that accepts ANYTHING: a family sending each other
documents should never be told their file is not allowed, and a fixed list would refuse the very
things a particular family lives on. A file therefore skips the magic-number check entirely — its
declared type is metadata, not a claim the server verifies — and carries a `name`, which for a
document is its whole identity. `name` is required for `kind=file` (1–255 characters), ignored for
a photo or a video, and echoed back on the attachment.

Accepting anything is safe only because the DOWNLOAD is defensive. `GET /attachments/{id}` for a
file answers with `Content-Disposition: attachment` and `X-Content-Type-Options: nosniff`, so
nothing a member uploads can be coaxed into rendering or executing from the family server's own
origin. The filename in that header is sanitised: control characters, quotes and path separators
are stripped (a header is a line — an unescaped newline in a filename is a header injection), and
anything non-ASCII goes in the RFC 5987 `filename*` form.

A file has no preview, no dimensions and no duration; clients draw a row with its name and size.
`PUT /attachments/{id}/preview` on one is `invalid_attachment`.

#### Locations

`kind=location` is the fifth kind and **the only one with no bytes**. A location is three numbers,
and they ride in the query string like every other piece of attachment metadata:

```
POST /attachments?kind=location&latitude=55.7558&longitude=37.6173&accuracy_m=12&name=Home
  (empty body — nothing is uploaded and the body is ignored)
  → 201 {attachment: {id: 61, kind: "location", latitude: 55.7558, longitude: 37.6173,
                      accuracy_m: 12, name: "Home", size: 0, has_preview: false, …}}
POST /chats/42/messages  {client_msg_id, body: "", attachment_id: 61}
```

Metadata rather than an uploaded blob because **a bubble must be able to draw the pin the moment the
message arrives**. Fetching forty bytes over HTTP first would make the one attachment that needs no
download the slowest thing in the thread to appear, and it would put a family member's position
behind a request that can fail.

`latitude` and `longitude` are degrees in WGS 84, required together, and range-checked
(`-90..=90` and `-180..=180`; both ends of the longitude range are the same real meridian and both
are accepted). `accuracy_m` is the radius in metres the sending device believed its fix good to, and
is **absent when it did not know** — which a client draws as a plain pin rather than as perfect
precision. `name` is an optional label somebody typed ("Home", "The restaurant"), 1–255 characters.

Consequences of having no bytes, all of them deliberate:

- `GET /attachments/{id}` on a location is `invalid_attachment` (400). There is nothing to serve;
  everything it is arrived with the message.
- `PUT /attachments/{id}/preview` on one is `invalid_attachment`, and `has_preview` is always false.
- `size` is `0`, so a location costs a family nothing in `stored_bytes` and adds nothing to what a
  backup has to carry.
- It still takes an attachment id, is still claimed exactly once by one message, is still swept
  after 24 hours if no message claims it, and still goes when its message goes. Nothing about the
  lifecycle is special.

**The map is drawn by each viewing device, and the server neither draws nor stores one.** A stored
snapshot would be one sender's idea of zoom, frozen, and it would also mean the sender contacting a
map provider on everyone's behalf. Drawing it per device is the same trade the link previews make
(see the Settings switch clients offer for those): the coordinate reaches whichever map framework
the platform provides, for locations a family member deliberately sent, and a client that would
rather not may draw the pin, the label and a hand-off to the system map app instead — that is a
client's choice and changes nothing on the wire.

Live location — a pin that keeps updating until it expires — is **not** part of this. A location is
decided once, at send time, and never changes, exactly like a reply. Adding it later would need a
mutation path and a sequence cursor of its own, and would be a new section here.

An attachment belongs to whoever uploaded it until a message claims it, and to that message's chat
afterwards. Before it is claimed only the uploader may read it; after, every member of the chat may.
An attachment can be claimed once — by one message, alongside up to nine others: a second message
naming it is `attachment_already_used`, and the same id twice in one `attachment_ids` array is
`invalid_attachment`.
**Unclaimed attachments are deleted after 24 hours** — a send the user abandoned must not leave
100 MB on the server forever.

A message carrying an attachment MAY have an empty body: a photo needs no caption. `message_empty`
applies only to a message with neither.

Size ceiling: **100 MB** by default (`limits.max_attachment_bytes`), and the preview has its own
much smaller ceiling. Over either is `attachment_too_large` (413).

For `kind=photo` and `kind=video` the accepted types are `image/jpeg`, `image/png`, `image/heic`,
`image/heif`, `video/mp4` and `video/quicktime`; a type outside that list, a type that contradicts
the kind, or bytes that do not match the type declared, is `invalid_attachment`. For `kind=file`
any type is accepted and none is verified — an absent or unparseable one is stored as
`application/octet-stream`.

**One copy per family.** An upload whose bytes a family already holds is
stored once: the server hashes what it writes (SHA-256, computed as the bytes
stream past) and points the new attachment at the file that is already there.
Forwarding the same photo into the family chat and two direct chats therefore
costs one file, not three. It is a storage detail with no wire effect — each
upload still gets its own attachment id, its own metadata and its own access
check, and two attachments sharing bytes never share visibility.

Deduplication is scoped to a FAMILY, not global: sharing bytes between
families would save a little more disk and cost the property that one
family's attachments are a self-contained set of files, which is what keeps
backing up, exporting or deleting a family simple. The consequence for the
server is that a stored file may be referenced by several rows, so it is
removed only when the last row referencing it is gone. Attachments uploaded
before this was introduced have no hash and keep a file each.

### Family statistics

`GET /families/mine/stats` — what the family has actually sent, for every member to see. Not
owner-only: it is a shared curiosity, and the same numbers go to everyone.

A member the caller has blocked is omitted from the per-member rows: a leaderboard row naming
somebody whose every bubble is a hidden row is exactly the presence a block is meant to remove, and
this is a per-caller response so the server can leave them out. The TOTALS are not projected —
including `totals.members` — for the same reason the board ceiling is not: they are the family's
numbers, and a number two members can compare must be one number.

One consequence is worth stating, because a client will otherwise draw it wrong: the rows no longer
add up to the totals, and the gap IS the block. A client must never derive a total by summing the
rows, nor a member's share by dividing into that sum — the totals are the family's number, and the
rows are what this caller may see of it.

```json
{"generated_at": "…",
 "totals": {"members": 4, "messages": 1284, "board_notes": 7,
            "attachments": {"count": 96, "photo": 61, "video": 12, "audio": 9, "file": 14,
                            "location": 3,
                            "bytes": 734003200, "stored_bytes": 612368384},
            "ai": {"questions": 43, "prompt_tokens": 12040, "completion_tokens": 30512}},
 "members": [{"user_id": 7, "display_name": "Anna", "messages": 512,
              "attachments": {"count": 31, "photo": 22, "video": 4, "audio": 3, "file": 2,
                              "location": 0, "bytes": 241172480},
              "ai": {"questions": 12, "prompt_tokens": 3400, "completion_tokens": 9120}}]}
```

**`bytes` and `stored_bytes` are different numbers and the gap is the point.** `bytes` adds up what
was sent; `stored_bytes` counts each distinct file once, because identical bytes are stored once per
family (see "One copy per family"). The difference is what dedup saved. `stored_bytes` is a family
total only — a file two members both sent belongs to neither of them alone, so a per-member share
would be a made-up number.

Counts are of what is **still here**. Retention sweeps messages past `retention_days` and takes their
attachments with them, so this describes the family's current history rather than everything it has
ever said. A member who has left keeps their rows and so keeps appearing, which is deliberate: their
messages are still in the thread.

Assistant usage is counted per completed reply, with the token counts the provider reported. A reply
that failed midway records nothing.

### Retention

The server deletes messages older than `limits.retention_days` (**100 days** by default), together
with any photo, video or file they own; the sweep runs at boot and hourly after. Setting it to `0`
keeps everything — "off" is a state rather than a very large number.

What goes with a message: its reactions, and its attachment (whose FILE is removed only once no
other message shares those bytes — see "One copy per family"). What survives: a newer **reply** that
quoted it, which keeps existing and simply loses its quote, because a reply is a message in its own
right and deleting today's conversation to enforce a policy about last spring's would be wrong. The
family **board** is not touched at all — a note is a live thing on a wall, not history.

This is a SERVER-side policy. Clients keep whatever history they have already downloaded, and
nothing in the protocol tells them to forget it: `after_id` catch-up only ever adds. A device that
has been offline past the cutoff simply never receives what was swept.

Bytes are stored on the server's filesystem, not in PostgreSQL — at this size a database row means
buffering 100 MB in memory on every read and write, and a `pg_dump` that grows without bound.
**This means a database dump is no longer a complete backup**; the attachments directory has to be
backed up alongside it.

### Deleting an account

Anyone may delete their own account, from inside the app, without asking anybody — which is what
Apple's App Store guideline 5.1.1(v) requires of an app that lets people sign up, and what a
self-hosted family server should offer regardless. It is `POST /me/delete`, it takes the account
password, and it cannot be undone.

**The person is erased; the words stay.** Everything that identifies the account is destroyed —
the username (which becomes available for somebody else to register), the password, the profile
picture, the birthday, the blocks it MADE, every session and every device row, and with them every
push token. The blocks made AGAINST it survive: somebody else's decision not to read those messages
was never the departing account's to revoke (see "Blocking a member"). What survives is what the
family said to each other: the member's messages in the family chat, the notes they pinned to the
board, and the reactions they left, all of them still attributed to a row that now reads "Deleted
account" and can never be signed into again.

That asymmetry is the whole design, and the alternative is worse. A family chat is a shared record,
and destroying one member's half of it punches holes in everybody else's: replies answer messages
that are no longer there, a photo everybody remembers is gone, and a conversation reads as a
monologue. Nobody else consented to losing that, and the guideline does not ask for it — what has
to go is the *account*.

**Direct chats do go**, both halves. A one-to-one chat has no meaning with one side removed, it was
private to the two of them rather than shared with the family, and it is the only history the
departing member can take with them without taking somebody else's. The other person's messages in
it go too; that is the honest reading of a private conversation ending. The member's private
assistant thread goes the same way.

Attachments the member uploaded are removed from the server's disk, subject to the one rule
attachment deletion always obeys: a file is removed only once no row still names those bytes (see
"One copy per family"). Their votes are retracted from any poll still open, which re-stamps that
poll and fans out its new state — a tally must not go on counting somebody who no longer exists.

**A deleted account is still resolvable, and that is what `former_members` is for.** Their messages
are still in the family chat and a client has to put a name to them, so `GET /families/mine`
returns them in a second array alongside the live roster. They are not members: they hold no role,
they are not offered as somebody to start a chat with, they receive nothing, and the family's
statistics and its member count do not include them. A client stores both arrays in one place and
draws only `members`.

**An owner may delete too, and the family survives.** If they are the last member, the family is
deleted with them — its chat, its board, its attachments and its invite code. If anybody else is
still there, **ownership passes to the longest-standing remaining member** and everything else stays
exactly as it was. These are the two places this protocol changes `families.owner_user_id` without
the owner naming a successor — an owner deleting their account, and an owner LEAVING (`POST
/families/leave`, which reaches this same rule by the other door) — and it is deliberate in both:
the alternative is an endpoint that can refuse, and an account somebody cannot delete because of who
else is in their family is exactly the dead end the guideline exists to forbid. Longest-standing
means the earliest to join the family chat, ties broken by the lower user id, so the answer is the
same on every read. The new owner is told by a `family_owner` frame and finds out the ordinary way
on their next `GET /me` regardless.

Every device the account was signed in on is signed out: the sessions are deleted, their sockets
close with `4401`, and their device rows go with them, so nothing is pushed to a phone whose account
no longer exists. Other members are told by `member_left` and `member_deleted` — and so is anybody
who only ever shared a direct chat, including somebody in another family entirely, because their
chat is about to vanish and nothing else would ever say why. An account with no family at all still
sends `member_deleted` to those peers, without a `family_id`.

Nothing is scheduled and nothing is reversible. There is no grace period, no "deactivated" state and
no way to cancel: a deletion that has not happened yet is a deletion an operator has to be trusted
to carry out, and this server would rather be believed.

### Blocking a member

Any member may block any other member of their own family, **the owner included**. The owner has no
special standing here and is not consulted: a block is a personal rendering-and-notification
preference, not a governance action. The owner's administrative powers are untouched by being
blocked, and a report still reaches them, because a report is a write to a table and not a message
the blocker has to read. A block affordance is weakest exactly where the person you want to stop
reading is the one with power, so this is the case it is built for.

**It is server-side.** A block survives a reinstall and suppresses push. A client-only block is a
block a new phone forgets, and it cannot stop a notification arriving while the app is not running.

**It is silent.** The blocked person is never told. Nothing in their app changes, no frame and no
push reaches them about it, no object they can read carries the fact, and every refusal this feature
introduces is aimed at the BLOCKER and never at the blocked. Where a blocked person's action must be
stopped, the outcome they see is one that already has an innocent explanation — being ignored —
never a new error. That is why the caller's own list rides on their own `GET /me` and never on a
broadcast object.

**What a block cannot hide.** Three things, named here so that no implementation tries to close them
and no client promises otherwise. The first is TIMING: a blocker is no longer woken, so when they
answer a blocked member changes shape — nothing at three in the morning, nothing while the app is
shut — and no amount of protocol removes that without removing the feature. It is accepted, and it
must NOT be masked by pushing to the blocker anyway. The second is the OPERATOR: a block is silent
towards a member using the apps and cannot be silent towards whoever administers the server, who
reads the table directly and, on a self-hosted family server, is very often the family owner — the
person this feature exists to be usable against. The guarantee is that no NEW signal reaches a
member, not that nothing at all can be inferred by anybody over months, and a client should not
imply more than that where it offers the block. It is also the honest reason `support_contact`
exists (see "Reporting a member"). The third is the FROZEN MARKER in a hidden direct chat: the
blocker's read marker there stops where it stood and their typing stops with it (see "Direct chats"
below), which the blocked member can see. That one is deliberate and is the innocent explanation
this whole feature rests on — somebody who has stopped opening your chat — and it is the ONLY
surface where a blocker's own behaviour goes visibly still, because in the family chat their `read`
and `typing` frames go on reaching that member byte-identically.

**The family chat keeps the message and hides it.** A blocked member's message is replaced by a
thin, tappable "Hidden — blocked member" row that reveals ONCE on demand. It is not deleted:
deleting it would break reply chains, because a quote is recomputed on every read, and it reads as
data loss. A revealed row is FULLY interactive — reply, react, vote — because a reveal that was
read-only would make the blocker's behaviour towards that thread deterministic and observable.

**A reveal is a peek, not a setting.** It is per row and per DEVICE, it is never on the wire and
never stored server-side, and it does not survive a restart — nothing about who has looked at what
may become a fact another device or another person can read. A revealed row stays revealed for as
long as it lives in that client's list, including when newer messages arrive after it and when a
`message_edited` frame rewrites its body. A hidden row draws the placeholder and the timestamp and
nothing else: no display name, no avatar, no attachment thumbnail, no reaction chips — all of them
come back with the reveal. And revealing changes nothing on the wire: no frame, no request, and no
movement of the read marker, which advances THROUGH hidden rows as they scroll past whether they
were revealed or not. A client that held its marker at an unrevealed row would rebuild oracle (c)
below on its own side, one layer above the server.

**A hidden row still fetches, and draws none of it.** A client resolves a hidden message's link
preview, its map tiles, its attachment preview and its sender's avatar exactly as it would for a
visible row, on the same schedule, and discards every result until the row is revealed — and it
fetches nothing a visible row would not have fetched yet, so a 90 MB video is no more downloaded
here than it is there. Not fetching is an oracle the server cannot close: a link preview and a map
tile are requests to a THIRD party, and the poster of a link owns that server's log, so a family
that is one distinct fetcher short from the day somebody blocked them has said so out loud,
repeatably and on demand. One wasted request per hidden row is the cheaper half of that trade. A
client's link-preview setting decides the fetch for every row alike and is never evaluated per
sender.

**A quote of a blocked member is hidden too, and only by the client.** A `reply_to` — or the
`parent` one level under it — whose `sender_id` is blocked draws as the same "Hidden — blocked
member" treatment as a row does, on every surface that draws a quote, and reveals under the same
one-tap rule; the quoting message above it is untouched and stays readable. The server sends the
excerpt unchanged, because it recomputes quotes on every read for everybody and a per-reader excerpt
is the projection this section has already refused. Without the rule the block is defeated by the
most ordinary thing in the chat: anybody replying to a blocked member carries 120 characters of them
into a bubble that is not hidden, and the assistant does it on every `@ai` a blocked member sends —
its answer is a real message from an unblocked sender quoting exactly the mention that asked (see
"Mentioning the assistant in the family chat").

**It follows that the server still DELIVERS.** A blocked member's message reaches the blocker over
the socket and over REST, unfiltered, exactly as before. What the server suppresses is the PUSH and
the direct chat. Three separate reasons the history must never be filtered, written down here so a
later optimisation does not undo them: (a) the reveal would have nothing to read; (b) history is a
keyset-paginated feed, and a filtered page is a SHORT page, which every client reads as the end of
the feed; (c) a client's read cursor only ever learns message ids the server actually handed it, so
a filtered page would FREEZE the blocker's read marker at the id before the blocked message — and a
frozen marker that leaps forward the moment a third member posts is a perfect, repeatable oracle for
the blocked person watching the other end.

**Read markers and typing keep flowing OUTWARD.** The blocker goes on marking read THROUGH hidden
rows exactly as if they were visible, and goes on sending typing frames as before. What is
suppressed is only what reaches the blocker INWARD: a typing frame or a read frame FROM a blocked
member is not relayed TO them. Suppressing the outward direction instead — the same one-line filter
with its two arguments transposed — is a flawless leak, so an implementation must be tested
POSITIVELY: in the FAMILY chat, after A blocks B, A's read frame and A's typing frame still reach
B, byte-identical. The hidden direct chat is the one exception, stated under "Direct chats" below:
there the blocker's own marker and typing stop, because the chat is gone from every path they can
reach it by.

**Direct chats.** The blocker may not open a direct chat with somebody they have blocked (`blocked`,
409), and that chat no longer appears in the blocker's `GET /chats`. The refusal is ASYMMETRIC on
purpose: the blocked person may go on creating and sending into the chat exactly as before, and
their messages simply reach nobody. `POST /chats/direct` refuses today for four reasons — yourself,
no such user, not in your family, and not in a family at all — every one of which a member of the
same family can rule out about somebody they can see posting, so a refusal in that direction would
have no innocent reading at all. The chat's history is not destroyed; only the blocker's way back
into it, and unblocking returns it whole. For the blocker that chat is not merely unlisted; it is
gone from every path they can reach it by. No `message` frame from it reaches their connections, it
contributes nothing to any unread count and nothing to the APNs `badge`, and `GET
/chats/{id}/messages`, `POST /chats/{id}/read` and a send by an id their client still holds all
answer `blocked` — 409 on REST, an `error` frame carrying the `client_msg_id` on the socket. There
is no hidden ROW to reveal here and no read cursor to freeze, so the three reasons above, which are
reasons about the FAMILY chat, buy nothing; and a badge the blocker could never clear would itself
be a quantity that moved when they blocked somebody. Their read marker in that chat therefore stops
where it stood and their typing stops with it. That is the one thing the blocked person can see, and
it is the innocent explanation this whole feature is built on — somebody who has stopped opening
your chat. Having the server advance the marker on the blocker's behalf would be worse, not better:
a marker that answers every message within milliseconds, at four in the morning, is not a person
reading. Unblocking puts the chat back in `GET /chats` with its true `unread_count` and
`last_read_message_id`, and a client pages it from `after_id = <the highest id it still holds>`, or
from the newest page if it holds none.

**Calls.** A call the blocked person places never rings the blocker: no `call_offer` frame, no VoIP
push, no ring, no entry in the operating system's call log. It is not refused, and it is not
declined. It RINGS OUT for the full ring timeout (`[calls] ring_timeout_secs`, 45 s by default — the
same one every other unanswered call uses) and ends with `timeout`, leaving the caller the same
`missed` record they would get from a phone nobody picked up: the only outcome indistinguishable
from being ignored. **The block is applied at DELIVERY and never at the decision.** The server works
out what to answer the caller — `call_ringing`, `peer_busy`, `peer_unreachable` — from the callee's
REAL, unblocked state, and only then drops the frames on the floor. So a blocked caller is told
`peer_busy` exactly when the callee is genuinely on a call, and `peer_unreachable` exactly when they
genuinely have neither a socket nor a wakeable device, in the same second an unblocked caller would
have been told it. Either other ordering is a one-frame oracle: removing the blocker's devices from
the candidate list FIRST makes the offer answer `peer_unreachable` instantly, and suppressing those
refusals instead makes a blocked caller the one person whose call rings out for 45 s at three in the
morning while a control call to a third member is refused at once. The suppression is TOTAL and
covers every frame of that call towards the blocker — the offer, the live and the buffered
`call_ice`, the `call_end` the server sends when the call rings out or the caller cancels, and the
registration-time replay to a device that connects while it is still ringing (see "Late arrivals"),
which would otherwise ring the blocker's freshly-woken phone for a call nobody was supposed to hear.
While such a call is pending the CALLEE is not busy: they may place a call, and a third member
calling them is not told `peer_busy`, or a blocked person could keep somebody uncallable by the
whole family with a loop of offers. The CALLER is busy for it exactly as they would be for any call
they placed — a blocked caller who could place two calls at once has been told something. The
reverse direction — the blocker calling somebody they blocked — is refused with `blocked`, which
only ever reaches the blocker.

A client does not need to be told this to know it: it holds its own block list, so it leaves a
blocked member out of every place a call can START — the conversation's call button, and the roster
gate an OS call request resolves through. The OS surfaces are the ones that matter, because they
OUTLIVE the block: a Recents row, a contact card's call button, a Favorites entry and Siri all go on
offering a handle the app will no longer act on, and a client that resolved one anyway would place
the call, take the refusal and then have to explain it. When a request does arrive for a blocked
member the honest answer is that they have been blocked, offered to the blocker alone; it must not
be the answer given for a person the app cannot identify, which sends somebody to re-link a contact
that is already linked. Where a spoken name matches several members and only one of them is
callable, that one is called — the ambiguity Siri would ask about is not ambiguous once the blocked
half of it is out.

What the app itself PUT on those surfaces, it takes back. A client that tells the operating system
about the calls it places — so the system can suggest them later — withdraws that person's entries
when they are blocked, because the suggestion is the app's own statement and goes on being made in
the app's name long after it stops being true. This is device hygiene and not part of the silence:
it happens entirely on the blocker's own device, changes nothing any other member can observe, and
is not the same claim as "no entry in the operating system's call log", which is about a call that
was never delivered. Unblocking does not put the old entries back — they were suggestions, not
history, and the record of the calls themselves is untouched.

**What is NOT hidden, and why.** Reactions, poll votes and message edits keep flowing, and the
client hides the identity. One frame is serialised once and sent to every connection, so
per-recipient content is structurally impossible on the socket; and a REST projection the socket
could not match would make the two transports disagree about what the chat contains. So: a reaction
chip keeps its COUNT and drops the blocked reactor from the who-reacted list; a poll keeps its
tallies and its bars and drops the blocked voter's face and name, and the "N of M voted" line a
client draws from the roster does not move, because the roster does not move (see "Polls": the vote
lists go out whole and every client counts them itself). Integers are not presence, and a count that
changed when you blocked somebody would tell you they had reacted. A board NOTE is the one object
where the client hides the CONTENT as well as the author: a note is a piece of writing pinned to a
shared wall with no bubble to collapse into a hidden row, and leaving the text up while dropping the
name would hide nothing that mattered (see "Board"). Every roster the server sends is COMPLETE — a
blocked member is still in `members`, still in a poll's vote lists, still nameable, still a person
whose old messages need a name — and the server projects per caller in exactly TWO
places — the statistics leaderboard, because a row there is a ranking rather than an identity, and
the blocker's own chat list, where a blocked direct chat is not returned at all (see `GET /chats`
and "Family
statistics"). What a client does with a complete roster is its own business, with one steer: leave a
blocked member out of the picker that starts a new direct chat, rather than letting the tap answer
`blocked`.

**A block is a pair, not a membership.** It survives one of them leaving the family and the two of
them meeting again. It is not scoped to a family and never lapses on a rejoin. A client draws the
list from the names it already holds — `members`, then `former_members` — and falls back to the
English placeholder "Blocked user" for an id it can resolve from neither, which happens for exactly
one reason: the block outlived that person's membership. There is no endpoint that turns an
arbitrary user id into a name, and adding one would be a user directory, which this product
deliberately does not have (see "Reporting a member").

**Account deletion is asymmetric.** Deleting an account destroys the blocks it MADE, along with the
avatar and the birthday, because a preference is part of the account. It does NOT destroy the blocks
made AGAINST it: those messages stay in the family chat, and somebody else's decision not to read
them was never the departing account's to revoke.

### Reporting a member

The owner is the moderator. That follows from the containment this product already has:
invite-code-only membership, approval by default, no cross-family contact, no user directory, and an
owner who can already remove a member, reset their password, rotate the code and close the family.

A report names a PERSON, or ONE MESSAGE of theirs, and carries a `reason` from a fixed list of four
— `spam`, `harassment`, `inappropriate`, `other`. Fixed rather than free text because this is a
nine-language product with a nine-language owner, and a report row a moderator cannot read is not a
report.

The excerpt is FROZEN, and this is the one place the recompute rule inverts. Everywhere else a
quotation is rebuilt on every read; here it is stored, for two reasons that both end with the owner
opening an empty screen: the author may EDIT the body away through the ordinary author-only path,
which has no time limit, and retention DELETES the message outright. `message_id` disappears when
retention takes the message; the excerpt does not, so a client draws the excerpt always and offers
to jump to the message only while the id survives.

**A message in a DIRECT chat may be reported**, and that is deliberate: a private conversation is
exactly where harassment happens, and the excerpt is the whole of the evidence. It is the one way
anything from a direct chat reaches a third person in this protocol, and it takes the deliberate act
of the person who received it — so a client offers the affordance in a direct chat, and SAYS where
it offers it that the message and its text will be shown to the family owner. Somebody who reports a
message without knowing that has been surprised by their own app.

A report is identified by `(reporter, reported, message_id)`, so a double tap is not two rows in the
owner's list while a second message of the same member IS a second report — the rule and the reason
for it are on `POST /families/reports`.

A report raises a PUSH to the owner and NO WebSocket frame — exactly as a join request does. The
owner reads the list on their next visit.

**A report ABOUT the owner never reaches the owner.** It is stored, it is logged at WARN for the
operator, and it is omitted from `GET /families/reports`. Otherwise the single most likely case for
a member who needs a report mechanism at all ends with the owner reading, by name, exactly who
complained about them. The reporter's `201` is byte-identical either way, so the response never says
whether the owner was shielded. For the same reason `Report.id` carries no ordering and is not a
sequence anybody can count in: an owner who sees ids 4, 6 and 7 in their own inbox has been told
that report 5 exists and, by elimination on a family whose other reports they can all see, roughly
what it must be about. A shield a gap in the numbering undoes is not a shield. The operator's
published contact travels on `GET /me` as `support_contact` for the same reason: it is the honest
escalation path when the moderator is the problem.

Resolving is the owner's, and says nothing about what they did. This protocol has removing a member,
resetting a password and closing the family; it does not have deleting somebody else's message.

Blocking and reporting are independent. Reporting does not block, blocking does not report, and the
owner's inbox NEVER shows who blocked whom — a family owner is often a parent and the blocked person
is often in the same house, which is the exact case the silence exists for.

### Voice calls

Two members can talk. A call is **one to one, voice only, and peer to peer**: the audio travels
directly between the two devices over WebRTC (Opus over DTLS-SRTP), and the server never carries,
hears or stores a single frame of it. What the server does is what it already does for everything
else — it passes small JSON frames between two people over the socket they already hold, wakes a
phone that has no socket open, and writes one message into the direct chat afterwards so that a
call is part of the history like anything else that happened between those two people.

That division is the whole design, and it is worth stating what it buys and what it costs. It buys
privacy of the strongest kind available: there is no media server to compromise, no recording to
subpoena, and a family whose server sits on a Raspberry Pi behind a home router can call each
other across the world without that box ever seeing more than a few kilobytes of signalling. It
costs group calls — three people need either a mesh of three connections or a server that mixes,
and neither is part of this — and it costs the ordinary NAT problem that every peer-to-peer
protocol has, which is what the `GET /calls/ice` endpoint is for.

**A call lives in a direct chat.** `call_offer` names a `chat_id`, and that chat must be a direct
one — a family chat has no single person to ring, and the assistant has no ears. The callee is the
chat's other member. Anchoring a call to the chat rather than to a user id is what gives the record
somewhere to live, and it means a client rings somebody from exactly the screen where it would
otherwise write to them; a client that wants to call from the member list does `POST /chats/direct`
first, which it needed anyway to write.

**One call per person.** A person who is ringing, being rung, or talking is busy. A second
`call_offer` from them is `call_busy`; an offer TO them is `peer_busy`. Both are refused before
anything is rung, and neither writes a record — nothing happened. The one exception is a call that
is ringing only nominally because the callee has blocked the caller: it does not make the CALLEE
busy. They may place calls of their own while it rings out, and an offer to them is never
`peer_busy` on account of it — otherwise a blocked person could keep somebody uncallable by the
whole family with a loop of offers (see "Blocking a member").

#### Identity: the client names the call

`call_id` is a UUID v4 **minted by the caller**, in exactly the way `client_msg_id` is minted by a
sender. The reason is the same: the caller must be able to correlate everything that comes back —
`call_ringing`, an `error`, the peer's `call_answer` — with the thing it started, and a server-issued
id would arrive one round trip too late to be useful for the frames already in flight. It is also
what makes the record exactly-once for free: the message the server writes afterwards carries the
`call_id` as its `client_msg_id`, so the `(chat_id, sender, client_msg_id)` uniqueness that dedups
retried sends dedups a call's record as well, and no code path has to remember to.

Every frame about a call — inbound and outbound, and an `error` answering one — carries the
`call_id`. **A client applies a call frame only to a call it holds and ignores every other one in
silence.** That single rule is what makes the multi-device story below work without the server
tracking which of a person's devices is doing what.

#### The sequence

1. The caller fetches `GET /calls/ice`, creates a peer connection with those servers, gathers an
   offer, and sends `{"type": "call_offer", "call_id", "chat_id", "sdp"}`. The offer may — should —
   be sent as soon as the local description exists; candidates trickle after it as `call_ice`.
2. The server checks the chat, the membership, the kind, and that neither party is busy, then
   answers the caller's connection with `call_ringing` and delivers
   `{"type": "call_offer", "call_id", "chat_id", "from_user_id", "sdp"}` to **every connection the
   callee has**. Callee devices that hold no socket are woken with a push — see "Push
   notifications" — and the caller's OTHER devices are told nothing: a call is placed from one
   device, and only that device is on it.
3. The callee's devices ring. The first to send `{"type": "call_answer", "call_id", "sdp"}` takes
   the call: the caller's connections receive `call_answer`, and every OTHER connection of the
   callee receives `{"type": "call_end", "reason": "answered_elsewhere"}` and stops ringing. A
   `call_decline` does not exist — declining is `call_end` with `reason: "decline"` from any one
   device, and it ends the call for all of them: a family member who has said no on their watch
   should not have their phone go on ringing.
4. Candidates are relayed by `call_ice` for as long as the call lasts. The caller's candidates go
   to every connection of the callee; the callee's, once it has answered, to every connection of
   the caller. Devices not on the call ignore them by `call_id`. **While a call is ringing the
   server BUFFERS the caller's candidates** (the most recent 64) and replays them, after the offer,
   to any callee connection that arrives late — a phone woken by a push connects seconds after the
   offer was delivered, and the caller's candidates were gathered in those seconds. A client MUST
   buffer remote candidates it receives before it has set the remote description, because the
   order in which a replay arrives is offer-then-candidates but nothing about a live relay
   promises it.
5. Either side ends the call with `{"type": "call_end", "call_id", "reason"}`, where a client's
   reasons are `hangup` (a call that was answered), `decline` (the callee refusing), `cancel` (the
   caller giving up while it rings) and `failed` (the media never came up, or died). The server
   relays it, with the same reason, to every connection of both parties except the one it came
   from, and writes the record.

The server ends a call on its own in three cases, and each reaches both parties as a `call_end` with
the reason named — both parties meaning both who were told of the call at all, so a callee who never
received the offer because they had blocked the caller receives no `call_end` for it either (see
"Blocking a member"): **`timeout`** when nobody has answered within the ring timeout (45 s by
default); **`cancel`** when the connection that placed the call closes while it is still ringing —
the caller's app was killed, or its socket blipped, and nothing on the far side should ring for
somebody who is no longer there; and **`failed`** when a call that was answered has had NEITHER
party connected for 60 s. The last is a backstop, not the rule: an answered call's audio is peer to
peer and outlives any number of socket reconnects, and the server deliberately does NOT end an
active call when one socket drops, because a momentary network change would otherwise hang up a
perfectly good conversation. Clients report a dead call themselves, from the peer connection's own
failure, with `reason: "failed"`.

**A client keeps its socket open for the life of a call**, ringing phase included. That is a change
from the ordinary iOS and Android behaviour of suspending or closing the socket in the background:
a call's `call_end`, its candidates and an ICE restart all arrive over it, and the platform
permissions a call runs under (an active audio session; a foreground service of the call type) are
exactly what allow a socket to stay up.

#### Late arrivals: the replay on connect

A connection is REGISTERED — authenticated and able to receive frames — some seconds after the push
that woke its device, and the frames that would have told it what to do were sent before it
existed. So the server replays, to a connection at the moment it registers:

- `call_offer` (followed by the buffered `call_ice` frames) when its user is the callee of a call
  that is still ringing;
- `call_end` with the reason, when its user was the callee of a call that ended within the last
  two minutes — answered on another device, cancelled, or rung out — so a phone that was woken for
  a call that has meanwhile been taken on the Mac stops ringing at once instead of at its own
  guard timeout.

A client that receives a `call_offer` for a `call_id` it already holds treats it as the duplicate it
is and does nothing. A device woken by a push that never receives an offer — the server is
unreachable, or the call ended more than two minutes before it managed to connect — gives up on its
own after the ring timeout.

#### The record

Every call the server ACCEPTED writes ONE message into the direct chat: sender = the caller,
`client_msg_id` = the `call_id`, and a `call` object with the outcome. `completed` is a call that
was answered and hung up; `missed` is one that was never answered — rung out, cancelled, or the
callee was nowhere to be reached; `declined` is one the callee refused; `failed` is one whose media
never came up or died. `duration_secs` is present when — and only when — the call was ever
answered, measured on the server's clock from the answer to the end, so `failed` may carry one and
`missed` never does. A refused offer (`call_busy`, `peer_busy`, `blocked`, a chat of the wrong kind) writes
nothing: nothing was accepted. A call the callee never heard because they had blocked the caller WAS
accepted and writes the ordinary `missed` record — that record is the entire point of letting it
ring out, and a call that vanished instead of being missed is the tell (see "Blocking a member").

**The body is the English placeholder** `"Voice call"`, or `"Missed voice call"` for a `missed`
one, in exactly the way a deleted account's `display_name` is the English placeholder "Deleted
account": a client that knows the `call` object draws its OWN wording from the outcome, the
duration and which side of the call it was on, and never shows the body; a client that predates
calls shows a readable line rather than a blank bubble. The chat-list preview carries the `call`
object for the same reason, so no client with the feature ever has to fall back to English there.
The record is a message in every other respect — it counts as unread for the callee, except in a
direct chat the callee has blocked into, which contributes nothing to any unread count (see
"Blocking a member"); it can be
replied to and reacted to, retention takes it — with one exception: **it cannot be edited.**
`PATCH` on it is `validation`. The author-only rule would otherwise let a caller write anything
into a line that other clients render as "Voice call".

A `missed` record pushes as the message it is (title = the caller's name, body = the placeholder,
or "New message" under `include_message_body = false`), which is what a missed-call notification
is. The other three outcomes are things both parties were present for and do not push.

#### Where the servers come from

`GET /calls/ice` answers with the STUN and TURN servers a client hands to its peer connection. The
list is the operator's, from the `[calls]` section of the server config: STUN is what lets two
phones behind ordinary home routers find each other directly, and TURN is the relay of last resort
for the networks where they cannot (carrier-grade NAT, corporate Wi-Fi). A TURN relay carries the
encrypted media and cannot read it, but it does carry it, which is why it is the operator's own
coturn rather than anything this protocol provides. When the operator has set a TURN shared secret,
the response carries time-limited credentials minted for the caller (coturn's `use-auth-secret`
scheme: the username is `<expiry>:<user_id>`, the credential is base64 of HMAC-SHA1 over it), and
`ttl_secs` says how long they are good for. Clients fetch this at the start of every call rather
than caching it — a credential is cheap, and a stale one is a call that silently cannot relay.

The default STUN list names Google's public STUN servers, because a voice feature that only works
inside one Wi-Fi network is not a voice feature. A STUN binding request carries no family data —
it asks "what is my public address" and nothing else — but it does tell that server a device's
address, and an operator who would rather it did not sets `stun_urls` to their own or to nothing.

`GET /me` says whether the server has calls on at all, as `calls_enabled`; a client hides the call
button behind it rather than discovering `calls_disabled` at the moment somebody wants to talk.
`video_calls_enabled` sits beside it and gates the video button alone — see "Video".

#### Video

A call is a VIDEO call when its `call_offer` says `"video": true` — decided when it is placed,
fixed for its life. The flag is what the SERVER needs to know (the ringing push and the record say
what kind of call it was, and a woken phone shows a camera the moment it rings); the media itself
is the SDP's business, which is where video actually lives: a video call's offer carries an audio
and a video m-line, and both peers negotiate the usual WebRTC way.

What the flag deliberately does NOT do is change mid-call. **Cameras toggle; the call's kind does
not.** Both tracks exist from the answer onward, so turning a camera off is disabling a track — no
renegotiation, no frame, the far side simply sees the stream stop — and turning it back on is the
reverse. A VOICE call can never become a video call (and the other way round): that upgrade is a
mid-call renegotiation this protocol does not carry yet, and when it does it will arrive as new
frame types under the unknown-frame rule, exactly as calls themselves arrived.

A callee whose camera permission is denied still answers a video call — with the camera off,
microphone rules unchanged. Refusing the call outright over a camera would turn a privacy setting
into missed calls; the SDP negotiates the video receive-only and the far side's picture still
shows. The same holds for placing one.

`[calls] video_enabled` (default true, meaningful only with calls enabled) lets an operator turn
video off — a relay sized for voice carries ~10 kB/s per call, video two orders of magnitude more,
and a Raspberry Pi behind home broadband is entitled to say no. With it off, a `call_offer` with
`"video": true` is refused with `video_calls_disabled`, voice calls untouched. `GET /me` exposes it
as `video_calls_enabled` (ALWAYS present, like `calls_enabled`).

The record's `call` object carries `"video": true`, its placeholder body becomes `"Video call"` /
`"Missed video call"` (the same old-client courtesy the voice wording is), and the ringing push
carries the flag so the incoming UI is a camera one — see "Incoming calls".

## REST endpoints

### Auth

| Method & path | Body → Response |
|---|---|
| `POST /auth/register` | `{username, display_name, password}` → `201 {token, user: User}`. Username: 3–32 chars `[a-zA-Z0-9_.]`, case-insensitively unique. Password ≥ 8 chars. Display name 1–64 chars. Errors: `username_taken`, `validation`. |
| `POST /auth/login` | `{username, password}` → `200 {token, user: User}`. Error: `invalid_credentials` (401). |
| `POST /auth/logout` | (auth) → `204`. Revokes the calling session and closes its sockets. |
| `POST /me/password` | (auth) `{current_password, new_password}` → `204`. Changing your own password requires proving you know the current one — a live session is not proof, because an unattended unlocked phone is exactly what this protects against. Every OTHER session of yours is revoked and its sockets closed; the calling session survives, so the device making the change stays signed in. Errors: `invalid_credentials` (401, wrong current password), `validation` (new password under 8 characters). |
| `POST /me/delete` | (auth) `{password}` → `204`. **Permanently deletes the calling account** (see "Deleting an account"). The password is required for the same reason `POST /me/password` requires it — a live session is not proof, and an unattended unlocked phone is exactly what this protects against. A `POST` rather than a `DELETE /me`, because the request carries a body and RFC 9110 gives content on a DELETE no defined semantics; this is a self-hosted product behind whatever proxy an operator runs, and `POST /families/leave` already sets the house precedent for a destructive self-service action with a body. Always succeeds for an authenticated caller who knows their password: an owner with other members hands ownership on rather than being refused. The successor is NOT named in the response, unlike `POST /families/leave`: the caller's sessions are about to be destroyed and every socket closed, so there is nobody left to show a name to. The successor learns of it by `family_owner`, and the rest of the family on their next read. Errors: `invalid_credentials` (401, wrong password), `validation` (no password given). |
| `PUT /me/birthday` | (auth) `{month, day}` → `200 {user: User}`. Your own birthday: a day and a month, no year (see "Birthdays"). Replaces whatever was there. Errors: `validation` (a month outside 1–12, or a day that month does not have). |
| `DELETE /me/birthday` | (auth) → `204`. Clears it. Idempotent — clearing a birthday nobody set is still `204`. |
| `POST /families/members/{id}/password` | (owner) `{new_password}` → `204`. The owner resets a member's password WITHOUT knowing the current one — the whole point is that the member has forgotten it. ALL of that member's sessions are revoked and their sockets closed, so every device they are signed in on returns to login; that is what makes a reset a recovery rather than a convenience. The owner cannot target themselves here (`POST /me/password` is for that), and a user outside the family is `not_same_family` whether or not they exist. Errors: `not_family_owner` (403), `not_same_family` (403), `validation`. |
| `GET /me` | (auth) → `200 {user: User, family: Family\|null, role: "owner"\|"member"\|null, pending_join_request: {family_id, family_name, created_at}\|null, calls_enabled: bool, video_calls_enabled: bool, max_family_members: 50}`. `pending_join_request` is the caller's live join request, if any — a client that was waiting and sees neither `family` nor `pending_join_request` knows the request was rejected. `calls_enabled` is ALWAYS present and says whether this server signals calls at all (`[calls] enabled`); a client hides its call button when it is false — see "Voice calls". `video_calls_enabled` is ALWAYS present too and gates the video-call button alone (`[calls] video_enabled`) — see "Video". `max_family_members` is ALWAYS present too and is the operator's ceiling on a family's size, so an owner's cap picker draws its range from it instead of discovering `validation` at the moment somebody tries to set one — the same reason `calls_enabled` is there instead of `calls_disabled` arriving when somebody wants to talk. Plus `blocked_user_ids: [11, 14]`, the caller's own block list, ALWAYS present and `[]` when they have blocked nobody — the one read in this protocol where absence is not allowed to mean "leave what you hold alone", for the same reason reactions stay present as `[]`: a list that vanished when it emptied would never tell a second device about the last unblock, and the standing rule everywhere else is that an absent field clears nothing. It is a complete state-set and never a delta, so a client replaces what it stores with what arrives. It rides here as well as on `GET /families/mine` because a block is a pair and not a membership: a caller with no family at all is answered `not_in_family` by that endpoint and still holds blocks, and `/me` is step 1 of the documented resync — which is what makes the list a step-1 fact and the `member_blocked` frame a latency optimisation rather than the only delivery path. Plus `support_contact: "…"`, the operator's published contact (`[server] support_contact`), absent when unset; clients show it on the report screen — see "Reporting a member". It is free text, at most 256 characters, in whatever form the operator configured; clients draw it VERBATIM, selectable and copyable, and never linkify it — an operator may write an address, a URL or a sentence, and three apps guessing differently about which it is would be worse than three apps showing the same text. |

### Profile pictures

Bytes, not JSON — the only binary surface in the protocol. Clients downscale and re-encode
before upload (a square JPEG, longest edge ≤ 512 px, is what both apps send); the server stores
what it is given after validating the type and size, and never transcodes.

| Method & path | Body → Response |
|---|---|
| `PUT /me/avatar` | Raw image bytes with `Content-Type: image/jpeg` or `image/png` → `200 {user: User}` with the incremented `avatar_version`. Replaces any previous picture. Errors: `avatar_too_large` (413, over the limit below), `invalid_image` (415 for any other content type, 400 when the bytes are not a readable JPEG/PNG). |
| `DELETE /me/avatar` | → `204`. Drops the picture and sets `avatar_version` back to `0`. Idempotent. |
| `GET /users/{id}/avatar` | → `200` with the stored bytes and that user's `Content-Type`. Visible to the caller themselves and to members of the same family; anyone else — and a user with no picture — gets `404 user_not_found` (the same answer, so the endpoint never reveals who exists). Sends `ETag` and `Cache-Control: private, max-age=31536000, immutable`, and honours `If-None-Match` with `304`. Takes an optional `v` query parameter, ignored by the server: it exists so a client can cache-bust by `avatar_version` in the URL. |

The picture is never pushed and never travels in a WebSocket frame — a frame carries at most the
`avatar_version` that tells a client to re-fetch.

### Families

| Method & path | Body → Response |
|---|---|
| `POST /families` | `{name}` (1–64 chars) → `201 {family: Family}`. Caller becomes owner; the family chat is created automatically. Error: `already_in_family`. |
| `POST /families/join` | `{invite_code}` → `200 {status: "joined"}` (policy `open` — membership immediate) or `200 {status: "pending"}` (policy `approval` — join request created). A family whose policy is `closed` admits nobody: the invite code answers `invalid_invite_code` (404), byte-identical to a code that never existed, so a shut door tells a stranger nothing — the same non-enumeration reasoning the avatar and password-reset endpoints follow. A family that is full answers `family_full` (409) — full meaning at its own `max_members`, or at the operator's ceiling when it has set none, because a valve that limited only what an owner may TYPE would hold nothing shut. The checks run in order — closed, then already in a family, then a pending request, then full — so a closed family answers `invalid_invite_code` whatever else is true of it, and under policy `approval` this door is where the REQUEST is created and the cap is read there too, then read again at approval. `family_full` does admit that the code is real, and that is the one thing this endpoint tells a stranger: the alternative is telling an invited member their code is invalid on the day the family filled up, which costs a real person a real join, where a closed family's code may be years old and in anybody's hands. Errors: `invalid_invite_code` (404), `already_in_family`, `join_request_pending`, `family_full` (409). |
| `GET /families/mine` | → `200 {family: Family, members: [Member], former_members: [Member], max_board_seq: 88, assistant: {user_id, display_name, mention}}`. `former_members` carries the accounts that were deleted while in this family, each with `"deleted": true` and no `role`; it is omitted when there are none, and it exists so a client can name the messages, notes and reactions they left behind (see "Deleting an account"). Nothing else counts them as members. `max_board_seq` is omitted while the board is empty and untouched — it is how a client knows whether a board catch-up is worth a request. `assistant` is present only when the server has one configured, and is how a client both NAMES its messages in the family chat and knows whether to offer `@ai` at all (see "Mentioning the assistant in the family chat"); it is not a member and is not in `members`. `family.invite_code` present for the owner only. Plus `blocked_user_ids: [11, 14]` as on `GET /me`, always present and `[]` when empty, and a complete state-set there too. Plus `next_owner_user_id: 11`, present for the OWNER only, naming the member who would inherit the family if the owner left right now — the same rule as "Deleting an account", computed once server-side so the leave dialog can say who it is instead of two clients computing it and eventually disagreeing (the roster does not carry join times, so no client could compute it anyway); omitted when the owner is the sole member. It is a PREDICTION and takes no frame of its own: any `member_joined` or `member_left` can change the answer, so a client re-reads `GET /families/mine` immediately before it shows the leave dialog and never names a successor from a cached value. Absence on that fresh read means the owner is the last member and leaving DELETES the family, which is a different dialog and a different confirmation. Error: `not_in_family`. |
| `POST /families/invite-code/rotate` | (owner) → `200 {invite_code}`. Old code stops working; pending requests survive. |
| `PATCH /families/mine` | (owner) `{join_policy?: "open"\|"approval"\|"closed", max_members?: int\|null, language?: "ru"\|null, ai_history?: true\|false}` → `200 {family: Family}`. Every field is optional and which fields are PRESENT decides what changes, exactly as on a board note — sending none of them is a valid no-op that answers with the family unchanged. `"language": null` CLEARS the family's language and `"max_members": null` CLEARS the cap, while leaving either key out entirely leaves it alone — these are **the two places** in this protocol where sending a `null` means something a missing key does not (see "The family's language"). `ai_history` is NOT such a place: it is a boolean with a real default, absent leaves it alone, and there is nothing for a `null` to mean (see "Mentioning the assistant in the family chat"). A cap must be between 1 and the operator's ceiling (`limits.max_family_members`). A cap BELOW the family's current size is ACCEPTED and acts as a freeze — nobody new until people leave — rather than being refused: an owner who inherits a large family must still be able to shut the door, and the cap is read at the door and never enforced over the room. Errors: `not_family_owner` (403), `validation` (a `join_policy` that is none of the three, or a `max_members` outside 1..ceiling), `invalid_language`. |
| `GET /families/join-requests` | (owner) → `200 {requests: [JoinRequest]}` (pending only). |
| `POST /families/join-requests/{id}/approve` | (owner) → `200 {member: Member}`. The cap is re-checked here, because the roster can fill between a request and the decision: `family_full` (409), which leaves the request PENDING — a full family is a temporary condition and not a decision, and the owner may approve it again once a seat frees. The cap counts the rows in `members`, the owner included; `former_members` do not count, and a pending request reserves nothing — three members, a cap of four and two pending requests means the first approval succeeds and the second is `family_full`. Closing the family does NOT touch requests that were already pending, and the owner may still approve them — closing is about the invite code, and an approval is the deliberate act of the person who closed it. Errors: `join_request_not_pending`, `user_already_in_family`, `family_full` (409). |
| `POST /families/join-requests/{id}/reject` | (owner) → `204`. Error: `join_request_not_pending`. |
| `POST /families/leave` | → `204`, or `200 {new_owner_user_id: 11}` when the caller was the owner and the family passed to somebody. **An owner who leaves hands the family on**; they are never refused. The successor is the longest-standing remaining member — the same rule defined once under "Deleting an account", and reached now from two doors rather than one. If nobody remains, the family is deleted with them exactly as before, and the answer is `204`. Leaving removes the caller from the family chat and their direct chats; history is retained and resurfaces on rejoin. The successor is told by a `family_owner` frame. That frame is sent BEFORE the departing owner's `member_left`, so no client ever momentarily holds a family whose `owner_user_id` names nobody in `members`. The leaving owner resolves `new_owner_user_id` against the roster it still holds, tells the user who inherited, and only then tears its family state down; a member sitting in their own leave dialog when `family_owner` arrives dismisses it and re-presents it from a fresh `GET /families/mine`, because both the successor and the consequences of leaving have just changed under them. `owner_cannot_leave` is no longer raised by this or any other endpoint. |
| `DELETE /families/members/{user_id}` | (owner) → `204`. Error: `cannot_remove_owner`. |
| `PUT /families/members/{user_id}/birthday` | (owner) `{month, day}` → `200 {member: Member}`. The owner filling in a birthday for a member of their own family — a parent for a child, typically. The owner MAY name themselves here, unlike the password reset: there is no proof being skipped (see "Birthdays"). A user outside the family is `not_same_family` whether or not they exist. Errors: `not_family_owner` (403), `not_same_family` (403), `validation`. |
| `DELETE /families/members/{user_id}/birthday` | (owner) → `204`. Clears it. Idempotent. Same errors. |
| `PUT /families/members/{user_id}/block` | → `204`. The caller stops seeing this member. Any member may block any other member of their own family, the OWNER INCLUDED — the owner has no special standing here and is not consulted. Idempotent: blocking somebody already blocked is still `204`. **The blocked member is never told** and nothing in their app changes (see "Blocking a member"). A user outside the family is `not_same_family` whether or not they exist — the same refusal, and for the same reason, as the password reset. Errors: `not_in_family` (409), `cannot_block_self` (400), `not_same_family` (403). |
| `DELETE /families/members/{user_id}/block` | → `204`. Unblocks. Idempotent — clearing a block nobody set is still `204`. Unblocking is scoped to the CALLER'S OWN list and not to the roster: any id in `blocked_user_ids` may be cleared, including one that has since left the family or deleted its account, so `not_same_family` is NOT raised here and neither is `not_in_family` — a block is a pair and not a membership, and a caller with no family at all may still clear one. A block the blocker cannot lift would be a permanent entry on their own screen and a block that silently came back to life on a rejoin (see "Blocking a member"). Error: `cannot_block_self` (400), kept for symmetry with the `PUT`. |
| `POST /families/reports` | `{reported_user_id, reason, message_id?}` → `201 {report: Report}`, or `200 {report: Report}` when an open report by this caller against this member already exists. `reason` is one of `spam`, `harassment`, `inappropriate`, `other`. `message_id` must name a message in a chat the caller is in AND have been sent by `reported_user_id`; anything else — including a real message in a chat the caller cannot see — is `message_not_found`, so the endpoint never confirms an id exists elsewhere, exactly as `reply_to_message_id` does not. A report is identified by `(reporter, reported, message_id)`, with a null `message_id` for one that names a person rather than a message; raising one that matches an OPEN row returns that row and creates nothing, whatever `reason` was sent — the stored reason is not overwritten. So a double tap is not two rows in the owner's list, while reporting a second message of the same member IS a second report, which is what a moderator needs in order to see a pattern. The excerpt is frozen here (see "Reporting a member"). Errors: `not_in_family` (409), `cannot_report_self` (400), `not_same_family` (403), `message_not_found` (404), `validation`. |
| `GET /families/reports` | (owner) → `200 {reports: [Report]}`, open only, oldest first — shaped exactly as `GET /families/join-requests`. Reports naming the owner themselves are NOT listed (see "Reporting a member"). Open reports are capped per family at the page maximum (200); the oldest are what the owner sees, and a family that has hit the ceiling has a moderation problem rather than a pagination problem. Error: `not_family_owner` (403). |
| `POST /families/reports/{id}/resolve` | (owner) → `204`. Takes it off the list; what "dealt with" MEANS is the owner's business. Idempotent for the owner's own inbox: resolving a report they have already resolved is still `204`, because a double tap and a retry after a timeout that actually worked are the same request twice and neither is an error. `report_not_pending` (409) is kept for what the owner may not see at all — a report of another family, or one that names the owner — one answer for both, so the endpoint never confirms an id exists elsewhere. Errors: `not_family_owner` (403), `report_not_pending` (409). |

### Attachments

| Method & path | Body → Response |
|---|---|
| `POST /attachments` | Raw bytes with `Content-Type` set to the media type. Query: `kind` (`photo`\|`video`\|`audio`\|`file`\|`location`), `width`, `height`, `duration_ms`, `name`, `latitude`, `longitude`, `accuracy_m`. `name` is REQUIRED for `kind=file` (1–255 characters) and optional on audio and a location; `latitude` and `longitude` are REQUIRED for `kind=location` and refused on anything else. A location sends **no body** — it is metadata only. → `201 {attachment: Attachment}`. Errors: `attachment_too_large` (413), `invalid_attachment` (415 for a media type not accepted on a photo/video/audio, 400 when the bytes do not match the declared type, a file has no name, or a location has no or out-of-range coordinates), `not_in_family`. |
| `PUT /attachments/{id}/preview` | Raw JPEG bytes of the downscaled photo or poster frame → `204`. Uploader only, and never on a `file`, `audio` or `location` (`invalid_attachment`). Errors: `attachment_not_found`, `attachment_too_large`, `invalid_attachment`. |
| `GET /attachments/{id}` | → `200` with the stored bytes and their `Content-Type`. A location has none and answers `invalid_attachment` (400). A `file` additionally gets `Content-Disposition: attachment; filename=…` (sanitised) and `X-Content-Type-Options: nosniff`, so an uploaded document can never render or execute from the server's own origin. Readable by the uploader always, and by every member of the chat once a message claims it; anyone else gets `404 attachment_not_found`. Sends `ETag` and `Cache-Control: private, max-age=31536000, immutable`, and honours `If-None-Match` with `304`. Honours a single-byte-range `Range` request with `206` + `Content-Range` (`416` for a range past the end) — that is how a video player seeks, and without it scrubbing a 90 MB clip re-downloads it from the start. A multi-range or unrecognised `Range` is ignored and the whole body sent, per RFC 9110. |
| `GET /attachments/{id}/preview` | → `200` with the preview JPEG, same access rules. `404` when there is no preview yet. |

### Board

| Method & path | Body → Response |
|---|---|
| `GET /families/mine/board` | → `200 {notes: [Note], max_board_seq: 88}`. The whole board as it now stands, tombstones excluded, newest `board_seq` first. `max_board_seq` is `0` for a board nothing has ever been written to. Error: `not_in_family`. |
| `GET /families/mine/board/changes` | Query: `after_seq` (default 0), `limit` (default 50, max 200) → `200 {notes: [Note]}` ordered by `board_seq` ascending, INCLUDING tombstones — the board catch-up, looped until a short page. Errors: `not_in_family`, `invalid_pagination`. |
| `POST /families/mine/board/notes` | `{text, color, x, y, size?}` → `201 {note: Note}`. Caller becomes the author. `size` defaults to `medium` when absent. Errors: `validation` (text empty or > 280), `invalid_note_color`, `invalid_note_size`, `board_full` (409, over the note ceiling), `not_in_family`. |
| `PATCH /families/mine/board/notes/{id}` | `{text?, color?, size?, x?, y?}` → `200 {note: Note}`. Any member may send `x`/`y`; only the author may send `text`, `color` or `size` (`not_note_author`, 403). Sending nothing that differs is a no-op: no new seq, no fan-out. Errors: `note_not_found` (404), `not_note_author`, `invalid_note_color`, `invalid_note_size`, `validation`, `not_in_family`. |
| `DELETE /families/mine/board/notes/{id}` | → `204`. Author only. Idempotent: deleting an already-deleted note is still `204` and takes no new seq. Errors: `note_not_found`, `not_note_author`, `not_in_family`. |

### Chats & messages

| Method & path | Body → Response |
|---|---|
| `GET /chats` | → `200 {chats: [{chat: Chat, last_message: Message\|null, unread_count: 3, last_read_message_id: 1337, max_reaction_seq: 123}]}`. Family chat included always; direct chats once they exist. `last_read_message_id` is the CALLER'S OWN read marker for this chat — the value `POST /chats/{id}/read` and the `read` frame maintain, monotonic and shared across all of that user's devices. It is the other half of `unread_count` and comes from the same row, and unlike the three `max_*_seq` cursors it is ALWAYS present: `0` means the caller has never reported reading anything here, which is a real answer rather than an absent one. It is an id THRESHOLD and not a reference — retention may already have swept the message it names, so a client must never assume it can fetch that id, only compare against it. Clients apply it monotonically into whatever they store (`max(stored, received)`), for the same reason the server does: a response still in flight while the reader is reading must never walk a local marker backwards. `max_reaction_seq` is omitted while no message in the chat has ever been reacted to, `max_edit_seq` likewise while nothing in it has ever been edited, and `max_poll_seq` likewise while no poll has ever been created in it — all three are high-water marks that never go back down, so a chat whose polls the retention sweep has since taken still reports one, and a client reading an empty feed is the correct outcome rather than a bug. `last_message` previews never carry `reactions`, the `poll` or the quote, but DO carry `attachments` (and the legacy `attachment`), trimmed exactly as before — kind and name, no dimensions, no coordinates — a photo sent without a caption has an empty body, and a preview with nothing in it is a chat row that looks like nothing happened. A direct chat with somebody the caller has blocked is NOT listed, for the blocker alone, and comes back whole on unblock — nothing about it is deleted (see "Blocking a member"); it contributes nothing to `unread_count` anywhere and nothing to the APNs `badge`, because there is nothing here for a client to count. Nothing else on this row is projected per caller: in the family chat a blocked member's message still moves `unread_count` and may still BE `last_message`, because the count is the other half of the read marker and projecting one without the other desynchronises them — and a count that changed when you blocked somebody is a quantity the blocked person's own behaviour can be tested against. A client draws such a preview as the hidden row rather than as text, with no sender name, and it is not revealable from the list. |
| `POST /chats/direct` | `{user_id}` → `200 {chat: Chat}` — get-or-create, idempotent. Errors: `cannot_dm_self` (400), `not_in_family` (409, the caller belongs to no family), `not_same_family` (409), `user_not_found` (404). Plus `blocked` (409) when the CALLER has blocked this member. Only that direction refuses: somebody who has been blocked may go on opening and sending into the chat exactly as before, and it is the blocker who no longer sees it (see "Blocking a member"). |
| `GET /chats/{id}/messages` | Query: `before_id` XOR `after_id` (optional), `limit` (default 50, max 200) → `200 {messages: [Message]}`. `before_id`: strictly older, **newest-first** (history pages). `after_id`: strictly newer, **oldest-first** (reconnect catch-up). Neither: the newest `limit`, newest-first. Errors: `blocked` (409, a direct chat with somebody the caller has blocked — see "Blocking a member"), `chat_not_found`, `not_chat_member`, `invalid_pagination`. |
| `POST /chats/{id}/messages` | `{client_msg_id: "<uuid>", body, reply_to_message_id?, attachment_id?, poll?}` → `201 {message: Message}`. In the family chat a body containing `@ai` additionally reaches the assistant (see "Mentioning the assistant in the family chat"). Retrying with the same `client_msg_id` returns the existing message as `200` — never a duplicate. Body: trimmed, non-empty, ≤ 4000 chars. `reply_to_message_id` is optional and must name a message in this same chat (see "Replies"). `attachment_ids: [34, 61]` claims 1–10 attachments this caller uploaded, in the order given; `attachment_id` (one id) is the legacy spelling of a one-element array, still accepted — sending BOTH is `validation`. A message carrying any may have an empty body. A location id must be the array's only element, and one id may not appear twice (`invalid_attachment`). `poll: {options: ["Pizza", "Pasta"]}` makes the message a poll (see "Polls"): the body is then the QUESTION and must be non-empty, `poll` and `attachment_id` are mutually exclusive, and only the family chat accepts one. Options: 2–10, each trimmed, non-empty, ≤ 100 characters, no two the same ignoring case. Errors: `blocked` (409, a direct chat with somebody the caller has blocked — see "Blocking a member"), `message_empty` (no body AND no attachment, or a poll with no question), `message_too_long`, `not_chat_member`, `message_not_found` (the reply target is not a message in this chat), `attachment_not_found`, `attachment_already_used`, `invalid_poll` (400 — a poll outside the family chat, alongside an attachment, or with options that break the rules above). |
| `PATCH /chats/{id}/messages/{mid}` | `{body}` → `200 {message: Message}`. Author only. Replaces the body, stamps `edited_at` and the next `edit_seq`, and fans out `message_edited`. Body rules are the send rules: trimmed, non-empty, ≤ 4000 chars. Re-sending the body it already has is a no-op: no new seq, no fan-out. Errors: `message_empty`, `message_too_long`, `not_message_author` (403), `message_not_found` (404 — no such message *in this chat*), `not_chat_member`, `chat_not_found`. |
| `GET /chats/{id}/edits` | Query: `after_seq` (default 0), `limit` (default 50, max 200) → `200 {messages: [Message]}` ordered by `edit_seq` ascending — the edit catch-up, looped until a short page like `after_id`. Errors: `chat_not_found`, `not_chat_member`, `invalid_pagination`. |
| `PUT /chats/{id}/messages/{mid}/vote` | `{option_id: 5}` → `200 {message_id, poll: {Poll}}`. Sets the caller's choice on a poll — an idempotent state-set, not a toggle (clients decide locally whether a tap means set or clear). One choice per member; there is no multiple choice. Re-PUT of the option already held is a no-op: no seq bump, no fan-out. Errors: `invalid_poll` (400 — no such option on this poll), `poll_closed` (409), `message_not_found` (404 — no such poll *in this chat*), `not_chat_member`, `chat_not_found`. |
| `DELETE /chats/{id}/messages/{mid}/vote` | → `200 {message_id, poll: {Poll}}`. Retracts the caller's vote; idempotent (retracting nothing returns the current state unchanged and burns no seq). Errors: `poll_closed` (409), `message_not_found`, `not_chat_member`, `chat_not_found`. |
| `POST /chats/{id}/messages/{mid}/poll/close` | → `200 {message_id, poll: {Poll}}`. Author only, one-way. Closing a closed poll is a no-op with no seq bump. Errors: `not_message_author` (403), `message_not_found`, `not_chat_member`, `chat_not_found`. |
| `GET /chats/{id}/polls` | Query: `after_seq` (default 0), `limit` (default 50, max 200) → `200 {polls: [{message_id, poll: {Poll}}]}` ordered by `poll_seq` ascending — the poll catch-up, looped until a short page like `after_id`. Errors: `chat_not_found`, `not_chat_member`, `invalid_pagination`. |
| `POST /chats/{id}/read` | `{last_read_message_id}` → `204`. Monotonic — the server keeps the max ever reported. Errors: `blocked` (409, a direct chat with somebody the caller has blocked — see "Blocking a member"), `chat_not_found`, `not_chat_member`. |
| `PUT /chats/{id}/messages/{mid}/reaction` | `{emoji}` → `200 {message_id, reaction_seq, reactions: [Reaction]}`. Sets or replaces the caller's reaction on the message — an idempotent state-set, not a toggle (clients decide locally whether a tap means set or remove). One reaction per user per message. Emoji: trimmed, non-empty, ≤ 32 bytes UTF-8. Re-PUT of the current emoji is a no-op: no seq bump, no fan-out. Errors: `invalid_emoji`, `message_not_found` (404 — no such message *in this chat*), `not_chat_member`, `chat_not_found`. |
| `DELETE /chats/{id}/messages/{mid}/reaction` | → `200 {message_id, reaction_seq, reactions: [Reaction]}`. Removes the caller's reaction; idempotent (deleting nothing returns the current state unchanged). Same errors minus `invalid_emoji`. |
| `GET /chats/{id}/reactions` | Query: `after_seq` (default 0), `limit` (default 50, max 200) → `200 {message_reactions: [{message_id, reaction_seq, reactions: [Reaction]}]}` ordered by `reaction_seq` ascending — the reaction catch-up, looped until a short page like `after_id`. Errors: `chat_not_found`, `not_chat_member`, `invalid_pagination`. |

### Devices (push hook — no delivery in v1)

| Method & path | Body → Response |
|---|---|
| `POST /devices` | `{platform: "ios"\|"macos"\|"android", push_token: string\|null, voip_token?: string\|null}` → `201 {device_id}`. Upserts by token when non-null. `voip_token` is the iOS PushKit VoIP token, the one an incoming call is delivered to (see "Push notifications"): ABSENT leaves whatever the row holds untouched, `null` or `""` clears it, a string sets it — the same absent-is-not-null rule every optional field on this wire follows, here because the two tokens arrive from the OS at different moments and a launch that has only one of them must not wipe the other. Only an `ios` device has one; a Mac never registers one and is never rung by push. `macos` is delivered over APNs alongside `ios` — the macOS build shares the iOS bundle id, so it shares the APNs topic and the payload is identical; it is a distinct platform in the DATA because a Mac claiming to be an iPhone makes every future question about delivery harder to answer. The caller's SESSION is recorded on the row as well, which is what makes the push gate per-device — see "Push notifications"; re-POST on every launch so it stays true. |
| `DELETE /devices/{id}` | → `204`. Error: `device_not_found`. |

### Calls

| Method & path | Body → Response |
|---|---|
| `GET /calls/ice` | (auth) → `200 {ice_servers: [IceServer], ttl_secs: 86400}`. The STUN/TURN servers to hand a peer connection, with time-limited TURN credentials minted for the caller when the operator configured a shared secret (see "Voice calls"). Fetched at the start of every call, never cached across calls. Error: `calls_disabled` (403). |

### Ops

| Method & path | Response |
|---|---|
| `GET /healthz` | `200 {"status": "ok"}` (checks the database). No auth. |

## WebSocket protocol

Connect with `Authorization: Bearer <token>` on the upgrade request. A bad token fails the
upgrade with `401`. The server closes with code `4401` when the session expires mid-connection.

Frames are JSON text messages tagged by `"type"`.

### Client → server

```json
{"type": "send",   "chat_id": 42, "client_msg_id": "8f14e45f-…", "body": "Dinner at 7?"}
{"type": "send",   "chat_id": 42, "client_msg_id": "1c4a9b02-…", "body": "Six works",
                   "reply_to_message_id": 1337}
{"type": "send",   "chat_id": 42, "client_msg_id": "9d3f1e77-…", "body": "",
                   "attachment_ids": [34, 35, 36]}
{"type": "send",   "chat_id": 42, "client_msg_id": "5b2e0c14-…", "body": "Pizza or pasta?",
                   "poll": {"options": ["Pizza", "Pasta"]}}
{"type": "read",   "chat_id": 42, "last_read_message_id": 1337}
{"type": "typing", "chat_id": 42}
{"type": "ping"}
{"type": "call_offer",  "call_id": "6a1f0c3e-…", "chat_id": 42, "sdp": "v=0\r\n…"}
{"type": "call_offer",  "call_id": "7b2e1d4f-…", "chat_id": 42, "sdp": "v=0\r\n…",
                        "video": true}
{"type": "call_answer", "call_id": "6a1f0c3e-…", "sdp": "v=0\r\n…"}
{"type": "call_ice",    "call_id": "6a1f0c3e-…",
                        "candidate": {"candidate": "candidate:…", "sdp_mid": "0", "sdp_mline_index": 0}}
{"type": "call_end",    "call_id": "6a1f0c3e-…", "reason": "hangup|decline|cancel|failed"}
```

(`sdp_mid` and `sdp_mline_index` are each optional on a candidate — a WebRTC stack supplies one,
the other, or both, and the receiving stack accepts whichever it was given.)

### Server → client

```json
{"type": "ack",     "client_msg_id": "8f14e45f-…", "message": {Message}}
{"type": "message", "message": {Message}}
{"type": "read",    "chat_id": 42, "user_id": 9, "last_read_message_id": 1338}
{"type": "typing",  "chat_id": 42, "user_id": 9}
{"type": "member_joined", "family_id": 3, "user": {"id": 11, "username": "junior", "display_name": "Junior", "avatar_version": 0}}
{"type": "member_left",   "family_id": 3, "user_id": 11}
{"type": "member_deleted", "family_id": 3, "member": {Member with "deleted": true}}
                          — "family_id" absent when the account belonged to no family
{"type": "family_owner",  "family_id": 3, "user_id": 9}
{"type": "member_blocked", "user_id": 11, "blocked": true}
{"type": "reaction", "chat_id": 42, "message_id": 1338, "reaction_seq": 124,
                     "reactions": [{"user_id": 9, "emoji": "❤️"}]}
{"type": "message_edited", "message": {Message}}
{"type": "poll", "chat_id": 42, "message_id": 1340,
                 "poll": {"poll_seq": 89, "closed": false,
                          "options": [{"id": 5, "text": "Pizza", "votes": [7, 9]},
                                      {"id": 6, "text": "Pasta", "votes": []}]}}
{"type": "board_note", "note": {Note}}
{"type": "ai_delta", "chat_id": 42, "message_id": 1339, "text": "…"}   — assistant, mid-reply
{"type": "ai_error", "chat_id": 42, "message_id": 1339}                — it stopped early
{"type": "call_offer",   "call_id": "6a1f0c3e-…", "chat_id": 42, "from_user_id": 7, "sdp": "v=0\r\n…"}
{"type": "call_offer",   "call_id": "7b2e1d4f-…", "chat_id": 42, "from_user_id": 7, "sdp": "v=0\r\n…",
                         "video": true}
{"type": "call_ringing", "call_id": "6a1f0c3e-…"}
{"type": "call_answer",  "call_id": "6a1f0c3e-…", "sdp": "v=0\r\n…"}
{"type": "call_ice",     "call_id": "6a1f0c3e-…",
                         "candidate": {"candidate": "candidate:…", "sdp_mid": "0", "sdp_mline_index": 0}}
{"type": "call_end",     "call_id": "6a1f0c3e-…",
                         "reason": "hangup|decline|cancel|timeout|failed|answered_elsewhere"}
{"type": "pong"}
{"type": "error",   "code": "not_chat_member", "message": "…", "client_msg_id": "8f14e45f-…"}
{"type": "error",   "code": "peer_busy", "message": "…", "call_id": "6a1f0c3e-…"}
```

(`client_msg_id` on `error` is present when the error answers a `send`; `call_id` when it answers a
call frame. Never both.)

`message_edited` carries the whole message, exactly as `message` does, and is a SEPARATE frame
type on purpose: `message` is what bumps unread counts and raises a notification, and an edit must
do neither. Clients apply it under the `edit_seq` guard described under "Editing".

`member_deleted` reaches every member of the deleted account's family AND every member of any chat
it was part of — a direct chat can outlive the family that created it, and its peer has to be told
too. `family_id` is therefore absent when the account belonged to no family, and a client keys the
frame on the `member`, never on the family: a peer outside the family receives a tombstone tagged
with a family they are not in, and in the sole-owner case with a family that no longer exists at all.

`member_deleted` says a member deleted their account, and it carries the whole tombstone `Member`
— `deleted: true`, the placeholder display name, `avatar_version: 0`, no birthday — because that is
exactly what a client has to overwrite. It is the one frame in this protocol whose job is to WIPE
stored fields, so a client applies it by writing the tombstone deliberately rather than by feeding
it through the ordinary member upsert, which everywhere else must never let an absent field clear a
stored one. A `member_left` frame is sent alongside it, so a client that predates this frame at
least fixes its roster and merely goes on showing the old name against old messages.

`family_owner` names the family's new owner and reaches every member of the family. It is sent when
an owner LEAVES or deletes their account and ownership passes on (see "Deleting an account" and
`POST /families/leave`); a client that receives it for itself gains the owner's screens immediately
rather than at its next `GET /me`.

`member_blocked` is delivered to every connection of the BLOCKER and to NOBODY else. It is the only
frame in this protocol about ANOTHER member that reaches one person's own devices, stops there, and
reports a fact the recipient alone established — `call_ringing` answers a single connection and an
`answered_elsewhere` `call_end` reaches one person's other devices, but both are about a call that
person is already in. That is exactly what makes blocking silent: nothing about a block ever reaches
the person blocked. `blocked` is a boolean carrying full current state rather than an event, so an
unblock is the same frame with `false` and a client applies it as a state-set, the idiom `reaction`
and `poll` use. It carries NO `family_id`: a block is a pair, not a membership. It has NO sequence
value, and deliberately — the four sequences that exist do so because `after_id` is `WHERE id >
cursor` and cannot see a change to an OLDER row, so an offline client would never learn of it; but
the whole block list is re-read in full from a per-caller endpoint on every resync, so there is no
catch-up feed to cursor and nothing to miss. It never notifies and never counts as unread. A REPORT
raises no frame at all: the owner is pushed, exactly as for a join request, and reads the list on
their next visit. It is delivered to the acting device as well, whose `PUT` or `DELETE` is answered
by a bodiless `204`; a client may apply the change optimistically, because the frame carries full
state and cannot conflict with it. A no-op — blocking somebody already blocked — emits nothing,
exactly as a re-PUT reaction does. It is ordered against `message` frames on the same socket, so a
message serialised before the block arrives before it: a client that draws that message and hides it
on the following frame is behaving correctly.

`poll` carries a poll's full current state to every member of the chat, the voter's own connections
included — the voter's own request is answered by its HTTP response. It never notifies and never
counts as unread. Clients apply it under the rule the reaction frame uses: only when the incoming
`poll_seq` is greater than the one held for that message, so an out-of-order frame cannot undo a
newer vote.

`board_note` carries one note in whatever state it now has — created, edited, moved, or a
tombstone — to every member of the family. It never notifies and never counts as unread. Clients
apply it under the same rule the board catch-up uses: a note is written only when the incoming
`board_seq` is greater than the one held, so an out-of-order frame cannot undo a newer move.

### Semantics

- **Ack fan-out**: the connection that sent a `send` frame receives `ack`; every other
  connection of every chat member — including the sender's other devices — receives `message`.
  A message posted over REST is acked by the HTTP response; all connections (including the
  sender's own) receive `message`.
- **Dedup**: `(chat_id, sender, client_msg_id)` is unique server-side. A retried `send` re-emits
  the `ack` with the original message and does not re-fan-out.
- **Read/typing**: relayed to the other members of the chat. `typing` is never persisted and is
  throttled server-side to one per chat per 3 s per connection. A `typing` frame naming a chat the
  sender is not a member of is **dropped in silence** — no relay and no `error` frame. Silence in
  both directions is the point: answering would spam a client whose membership lapsed mid-connection
  with an error per keystroke, and answering only for chats that exist would turn the indicator into
  a way to enumerate chat ids. The membership check runs AFTER the throttle, so a client sending a
  chat id it has no business with cannot turn every keystroke into a database round trip.
  A client folds an incoming `read` into a displayed "seen" marker only in a DIRECT chat, where
  there is exactly one peer and the marker means one person. In the family chat a `read` frame is
  roster data that no bubble draws: a per-member seen state over N members is a row of faces nobody
  asked for, and a per-member ABSENCE is precisely what a blocker's suppressed inward frames would
  put on screen. `typing` is drawn in both kinds. This is written down as a rule rather than left to
  each client because it is a behaviour change for one of them, and two apps that disagree about it
  disagree in public. One recipient is also missing from the relay: a `read` or a `typing` frame from
  a member is not relayed to anybody who has blocked them. The filter is on the RECIPIENT and never
  on the sender — the blocker's own `read` and `typing` frames still reach the blocked member
  byte-identically in the FAMILY chat, which is the positive test "Blocking a member" demands. A
  `read` or a `typing` frame naming a DIRECT chat the sender has blocked into is **dropped in
  silence** — no relay and no `error` frame, the same answer this bullet already gives a `typing`
  frame naming a chat the sender does not belong to. Dropped rather than answered, and specified
  here rather than left to each client, because a blocker whose two phones disagreed about whether
  to keep sending would show the blocked member one marker that moves and one that does not.
- **Reactions**: the `reaction` frame carries the message's **full current reaction state**
  (never a delta), so it is idempotent and ordering races resolve locally: a client applies it
  only when `reaction_seq` is greater than the last value it stored for that message. It goes
  to every connection of every chat member, the actor's own included (the actor's originating
  request is answered by its HTTP response). Reaction mutations are REST-only in v1; a
  client-to-server `reaction` frame may be added later under the unknown-frame rule.
  Sequence values are assigned before commit, so a catch-up read can theoretically skip a
  not-yet-committed lower seq on a *different* message — the same accepted gap `after_id` has
  for message ids, and self-healing here because any later reaction to the same message
  re-delivers its full state.
- **Polls**: the `poll` frame carries the poll's **full current state** (never a delta), so it
  is idempotent and ordering races resolve locally: a client applies it only when `poll_seq` is
  greater than the last value it stored for that message. It goes to every connection of every
  chat member, the voter's own included (the voter's originating request is answered by its HTTP
  response). It never pushes and never touches an unread count — a vote is not a message. Poll
  mutations are REST-only in v1; a client-to-server vote frame may be added later under the
  unknown-frame rule. Sequence values are assigned before commit, with the same accepted gap
  reactions have and the same self-healing.
- **Calls**: the whole signalling exchange is under "Voice calls"; the transport rules are
  these. `call_offer` is answered on the ORIGINATING connection with `call_ringing` or an
  `error` carrying the `call_id` (`calls_disabled`, `invalid_call` for a chat that is not
  direct, `chat_not_found`, `not_chat_member`, `call_busy`, `peer_busy`, `blocked`
  when the CALLER has blocked the callee — the one call error aimed at the person who set the block,
  so it can never reach the person it is about — and `peer_unreachable` when the callee has neither a
  socket nor a device that can be woken). A block never changes WHICH of these answers an offer: the
  callee's busyness and reachability are computed on their real state and the block is applied one
  step later, at delivery, so an offer to somebody who has blocked the caller is answered
  `call_ringing` exactly when it otherwise would have been, and rings out to `timeout` (see
  "Blocking a member"). `call_answer` for a call
  the server does not hold, or from someone who is not its callee, is `call_not_found` /
  `invalid_call`. `call_ice` and `call_end` naming an unknown call are **dropped in silence**:
  the first is noise-level and the second is a client tidying up after a restart, and neither
  deserves an error per frame. Relays never go back to the connection they came from. The
  registry replays `call_offer` (plus buffered candidates) and recent `call_end` frames to a
  connection at registration, as described under "Late arrivals".
- **Keepalive**: the server pings (WS protocol level) every 30 s and drops sockets idle for
  75 s. Clients should send `{"type":"ping"}` every ~25 s if their WS library hides protocol
  pings, and treat a missing `pong` as a dead connection.
- **Best-effort delivery**: the socket is a live wire, not a queue. A slow client's socket is
  dropped; REST is the source of truth. On every (re)connect a client must resync:
  1. `GET /me` — reconcile membership, and REPLACE the stored block list with `blocked_user_ids`.
     Then `GET /families/mine`, unless step 1 says the caller has no family: the roster,
     `former_members`, `next_owner_user_id` and `max_board_seq` live only there, and nothing else
     replays a roster change missed while offline — a join, a leave, a birthday, the join policy,
     the member cap and the family's language either raise no frame at all or raise one a sleeping
     client did not get.
  2. `GET /chats` — chat list, previews, authoritative unread counts, and the caller's own
     read marker per chat.
  3. Per chat: `GET /chats/{id}/messages?after_id=<max known message id>` looped until a short
     page — message ids are globally monotonic, so `max(id)` is the sync cursor. That cursor
     belongs to the LOOP: it is read once, before the first page, and then advanced by the
     largest id each page actually returned. Re-reading `max(id)` from the store between pages
     instead lets a live message arriving mid-loop jump the cursor to its id, and every message
     between the last page and it is skipped — permanently, because `after_id` can never look
     back and history paging only ever goes older than the OLDEST row held.
     Then, when the chat's `max_reaction_seq` from step 2 exceeds the locally stored reaction
     cursor: `GET /chats/{id}/reactions?after_seq=<stored cursor>` looped until a short page,
     applied per message under the `reaction_seq` guard. The stored cursor advances with every
     page **even for messages the client does not hold** (states for unknown messages are
     dropped; history paging re-delivers them embedded on the `Message` objects).
     Then, when the chat's `max_poll_seq` from step 2 exceeds the locally stored poll cursor:
     `GET /chats/{id}/polls?after_seq=<stored cursor>` looped until a short page, applied per
     message under the `poll_seq` guard, with the cursor advancing exactly as the reaction one
     does.
     **Only a live frame and a catch-up page may move either chat cursor.** A reaction or poll
     state that reaches a client by any other route — embedded on a fetched `Message`, or in the
     HTTP response to that client's own reaction, vote, retraction or close — is applied under
     the per-message guard and must NOT advance the chat cursor. Such a state is evidence about
     one message and none at all about another message's lower value, and REST goes on working
     while the socket is down, which is precisely when the frames carrying those lower values
     were missed: a vote answered with `poll_seq` 100 would push the cursor past somebody else's
     99, step 2's `max_poll_seq > cursor` test would then ask for nothing, and that state would
     be lost until the poll holding it next changed. One redundant catch-up page is the cheaper
     mistake.
  4. Re-send any locally pending outbound messages (safe: `client_msg_id` dedups).

## Push notifications

The server pushes **per device, not per user**. A device is pushed unless the session it
registered itself with has a live WebSocket at that moment — an open socket is already carrying
the frames, so waking the device holding it would only say the same thing twice.

That last sentence is a fact about ONE device, and it was for a long time applied to the whole
person, which is a different and wrong rule. The three apps treat their sockets differently on
purpose: a desktop app holds its socket open for as long as it is running, iOS suspends its socket
the moment the app is backgrounded, and Android closes its socket outright. So "does this user have
a socket" was always answered by whichever device needed the alert least. Somebody with a Mac
running at home heard nothing on any phone all day — the machine that was certainly being watched
silenced the two that were not.

The consequence of the per-device rule is deliberate and worth stating: a member reading on their
Mac also gets a banner on the phone in their pocket, for a message they are already looking at.
The phone cannot know what the Mac is showing — nothing in this protocol tells it — and a
redundant banner is a far smaller harm than a message nobody is told about. Every desktop chat
client makes the same trade. What stays whole-user is the SENDER: none of your own devices is ever
told about your own message, note or join, and none ever was.

A device learns its session from the bearer token it registers with, so nothing new travels on the
wire: `POST /devices` records the caller's session on the row. A device whose session is UNKNOWN is
pushed: the server cannot prove anyone is looking at it, and it errs towards the alert. Unknown now
means one thing only — a row written before this rule existed — and it heals on that device's next
launch, because clients re-POST `/devices` every time they start. A STALE link (a session that is
valid but has no socket open on it right now) is the ordinary case and is exactly what gets the
push. Both fall on the side of a notification too many, which is the side this rule exists to fall
on.

**A device that is signed out is not pushed at all**, and that is the one place the doubt runs the
other way. Two things sign a device out and both are handled at the source rather than by this
rule:

- **Revoked.** Deleting a session deletes the devices registered from it. A logout, a password
  change (which revokes every other session) and an owner resetting a member's password (which
  revokes all of them) therefore take those devices' push tokens away with them — which is what
  makes "a device somebody else is holding stops working the moment the reset lands" true of the
  lock screen as well as of the API. The device re-registers on its next launch, if whoever holds
  it can still sign in.
- **Expired.** A session that has passed its expiry is not deleted, only left behind, so a device
  can go on naming one for as long as its row survives. Such a device is skipped: the link has to
  name a session that is still alive, not merely one that still exists.

Neither is a case of "the server cannot tell", which is why neither gets the benefit of the doubt.
A signed-out phone showing family message bodies on its lock screen is not a redundant banner; it
is a leak, and it lasts until somebody notices.

Six events push: a **new message** (to every chat member but the sender), a **new board note** (to
every family member but the author), a **join request created** (to the family owner), a **join
request approved** (to the requester), an **incoming call** (to the callee — see below), and a
**report created** (to the family owner, unless the report names the owner — see "Reporting a
member") — each of them narrowed device by device by the rule above. Typing, reads, reactions, note
moves and edits, and other frames never push.

**A block removes a candidate before the per-device rule runs.** A member is not woken for a message
(a poll included — a poll IS a message and pushes as one), a board note or a call from somebody they
have blocked. A block never narrows the owner's MODERATION pushes: a join request and a report wake
the owner whoever they name, because being blocked by the moderator is not a way to stop being
moderated (see "Blocking a member"). This is a per-RECIPIENT
narrowing one level ABOVE the existing per-device gate: the device rule narrows a candidate list
device by device, and the block removes the candidate first. It applies to the SENDER of the
message, and to one further case: a message from the ASSISTANT whose `reply_to` names a blocked
member, because its answer to a blocked member's `@ai` would otherwise light up the blocker's phone
for a thread they cannot read. It stops there, deliberately. Suppressing every reply that quotes a
blocked member would silence pushes written by people the blocker has NOT blocked, and would hand
the blocked member a way to do it on purpose: post something reply-worthy, wait for a third member
to quote it, and the blocker's phone stays dark for a message they were meant to see. A block is
about one member's own words.

**In the family chat the frame is NOT suppressed, only the push.** The message still arrives over
the socket, because the "Hidden — blocked member" row has to have something to reveal, and because a
client's read cursor only learns ids the server handed it. The one exception is a direct chat the
recipient has blocked into, where the frame is suppressed too — there is no hidden row to reveal
there and no read cursor to freeze (see "Blocking a member").

A poll is a message and pushes exactly once, as one, with its question for a body — there is no
"Anna started a poll" alert, because the question is more useful than the fact. Votes, retractions
and closes never push, for the same reason reactions do not. An account being deleted never pushes
either: `member_deleted` and `family_owner` are corrections to what a client already holds, not
news.

Device lifecycle: `POST /devices {platform, push_token}` upserts by token (re-login moves the
token to the new account); the response `device_id` should be stored so `DELETE /devices/{id}`
can be called on logout. That call is a courtesy rather than the guarantee — `POST /auth/logout`
removes the session's devices anyway — so a client whose best-effort delete fails has not left a
signed-out phone receiving alerts. Clients re-POST on every launch, after every login, and whenever
the OS rotates the token — the launch re-POST is also what keeps the session link fresh, and after
any of the sign-outs above it is what brings the device back at all. A push rejected as
unregistered (APNs `410`/`BadDeviceToken`, FCM `UNREGISTERED`) deletes the device row.

Titles: direct chat → sender's display name; family chat → `"<Family> — <Sender>"`. Body: the
message text, or `"New message"` when the server's `[push] include_message_body = false`.

A new board note pushes with `"kind": "board_note"` and `family_id` + `note_id` instead of chat and
message ids. Title `"<Family> — <Author>"`, body the note's text (or `"New note"` when
`include_message_body = false`, the same switch that governs message bodies). Tapping it opens the
board.

A report pushes to the family owner with `"kind": "report"` and `family_id`, and never carries the
reported text, regardless of `include_message_body`: the excerpt is the very content somebody asked
to have looked at, and a lock screen is where it must not be readable. Title is the family name and
body the fixed English "New report", which `include_message_body` does not vary because there is
nothing there to withhold. Tapping it opens the owner's report inbox. A report naming the owner
pushes to nobody (see "Reporting a member").

A message carrying attachments MAY have an empty body — which is how photos are normally sent —
and an alert showing a name above a blank line says nothing arrived. Such a message pushes what
arrived instead: for ONE attachment, `"Photo"`, `"Video"`, `"Audio"`, the file's name, or a
location's label falling back to `"Location"`; for several of one kind, a count — `"3 Photos"`,
`"2 Videos"`, `"2 Audio"`, `"4 Files"` (names give way to the count); for a mixed set,
`"N attachments"`. A caption, when there is one, still wins. A location's COORDINATES are never
in an alert — a lock screen is the one place a family member's position should not be readable
without unlocking the phone.

APNs (token-based auth, HTTP/2): headers `apns-topic` = bundle id, `apns-push-type: alert`,
`apns-priority: 10`; payload:

```json
{"aps": {"alert": {"title": "Anna", "body": "Dinner at 7?"}, "sound": "default",
         "badge": 3, "thread-id": "chat-42"},
 "chat_id": 42, "message_id": 1338, "kind": "message"}
```

(`badge` = the user's total unread across chats at send time, and it is APNs-only. It is what the
SYSTEM puts on the icon while the app is not running; a running client derives its own icon badge
from its store instead, because the server pushes only to a device with no live socket and can
therefore never correct a foregrounded app's number. Join events use `"kind": "join_request"` /
`"kind": "joined"`, and a report `"kind": "report"`, all three with `family_id` instead of
chat/message ids.)

FCM (HTTP v1): notification + data so the system tray renders when the app is dead, while a
foregrounded app — its own socket live — is never pushed at all:

```json
{"message": {"token": "…",
  "notification": {"title": "Anna", "body": "Dinner at 7?"},
  "data": {"kind": "message", "chat_id": "42", "message_id": "1338"},
  "android": {"priority": "HIGH",
              "notification": {"channel_id": "messages", "tag": "chat-42",
                               "notification_count": 3}}}}
```

`notification_count` is the recipient's unread count **in the chat this push is about**, not the
total `badge` carries. It is a number rather than a string — the everything-is-a-string rule applies
to `data` only — and it rides on `message` pushes alone, omitted from `board_note`, `join_request`,
`joined` and `report`.

Per chat rather than total, because Android has no icon-badge API at all: the only number an app can
offer a launcher is `Notification.number`, and a launcher that draws one SUMS it across the app's
live notifications. The app posts one per chat (`tag: "chat-<id>"`), so a total on each of three
chats would render as three times the total. It follows that the count only means anything while the
notifications it rides on are alive, which is why a client MUST take a chat's notification down when
that chat is read — on any of the user's devices, not merely the one holding it. Whether a number
appears at all is the launcher's decision and not the app's: several draw a plain dot and never a
count, and that is a correct rendering of the same data.

Tapping a notification opens the chat named by `chat_id` (or the board for `board_note`, the
join-requests screen for `join_request`, the chat list for `joined`, the report inbox for
`report`).

### Incoming calls

A call has to RING a phone whose app is not running, and an alert notification cannot do that: it
draws a banner, and a banner is not a ringing phone. So an incoming call wakes a device through the
one channel each platform provides for exactly this, and the payload is not a notification at all —
it is the fact of the call, and the device does the ringing itself.

Who is woken: the callee's devices that have no live socket of their own — the same per-device
rule as everything above — that can present a call: an `ios` device with a `voip_token`, and an
`android` device with a push token. An iOS device that never registered a VoIP token is not woken;
a Mac is never woken, because a Mac that is not running is not a phone in a pocket. What a woken
device does next is connect its socket, and the server's registration-time replay ("Late arrivals")
hands it the offer — or the `call_end`, if the call is already over.

APNs, to the VoIP token: headers `apns-topic` = `<bundle id>.voip`, `apns-push-type: voip`,
`apns-priority: 10`, `apns-expiration` = now + the ring timeout; payload:

```json
{"kind": "call", "call_id": "6a1f0c3e-…", "chat_id": 42, "from_user_id": 7,
 "caller_name": "Anna", "video": true}
```

(`video` present when — and only when — it is a video call: it is what makes the woken device ring
with a camera UI, `hasVideo` on CallKit included. A voice call's payload carries no `video` key at
all:)

```json
{"kind": "call", "call_id": "6a1f0c3e-…", "chat_id": 42, "from_user_id": 7,
 "caller_name": "Anna"}
```

No `aps` dictionary: a VoIP push has nothing for the system to draw, and iOS requires the app to
report the call to CallKit the moment it arrives — which is what the app does with these fields.

FCM, data only and no `notification` block, so the app process is woken to ring rather than the
tray asked to draw:

```json
{"message": {"token": "…",
  "data": {"kind": "call", "call_id": "6a1f0c3e-…", "chat_id": "42", "from_user_id": "7",
           "caller_name": "Anna", "video": "true"},
  "android": {"priority": "HIGH", "ttl": "45s"}}}
```

`ttl` is the ring timeout: a push FCM could not deliver while the call was ringing must not deliver
once it is over and wake a phone into a call that no longer exists.

`caller_name` is the display name a device shows while it rings, and it is on the wire even under
`include_message_body = false` — a sender's name is the TITLE of every message push under that
setting too, and a phone that rings without saying who is calling is worse than one that does not.

A VoIP token APNs reports as unregistered is CLEARED from its device row rather than the row
deleted — the alert token beside it may be perfectly good. An Android call push rejected as
unregistered deletes the row, as an ordinary push would.

## Limits (server defaults, configurable)

| Limit | Default |
|---|---|
| Message body | 4000 chars |
| Members in one family | no per-family cap by default; `limits.max_family_members` (default 50) is the operator's ceiling and binds at the join door for EVERY family, capped or not — an owner's `max_members` may only be lower, and is the control the apps show |
| Reaction emoji | 32 bytes UTF-8 (fixed) |
| Page size | 50 default / 200 max |
| HTTP body | 16 KiB (except `PUT /me/avatar`) |
| Profile picture | 256 KiB |
| Per-socket outbound queue | 64 frames |
| Session TTL | 180 days, sliding |
| Poll options | 2 minimum (fixed), 10 maximum |
| Poll option text | 100 chars |
| Family-chat history sent with a mention | 30 days / 200 messages / 40 000 chars, whichever binds first (fixed) |
| Attachment size | 100 MB (`limits.max_attachment_bytes`; keep nginx in step) |
| Attachments per message | 10 (`limits.max_attachments_per_message`; the fewest is 1, fixed) |
| Call ring timeout | 45 s |
| Buffered caller candidates while ringing | 64 (fixed) |
| Offer / answer SDP | 64 KiB (fixed) |
| One ICE candidate | 2 KiB (fixed); larger or empty ones are dropped in silence |
| Answered call with neither party connected | 60 s, then `failed` (fixed) |
| TURN credential lifetime | 24 h |
