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
  This is how voice/video signaling frames (`call_offer`, `call_answer`, …) will be added later
  without breaking v1 clients.
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

## Error shape

Non-2xx responses carry:

```json
{"error": {"code": "username_taken", "message": "username is already in use"}}
```

`code` is a stable machine-readable string; `message` is a human-readable English sentence.
Canonical codes: `unauthorized`, `invalid_credentials`, `username_taken`, `validation`,
`already_in_family`, `not_in_family`, `not_family_owner`, `invalid_invite_code`,
`join_request_pending`, `join_request_not_pending`, `user_already_in_family`,
`owner_cannot_leave`, `cannot_remove_owner`, `cannot_dm_self`, `not_same_family`,
`user_not_found`, `chat_not_found`, `not_chat_member`, `message_empty`, `message_too_long`,
`message_not_found`, `not_message_author`, `invalid_emoji`, `note_not_found`,
`not_note_author`, `invalid_note_color`, `board_full`, `invalid_pagination`, `device_not_found`,
`avatar_too_large`, `invalid_image`, `attachment_too_large`, `invalid_attachment`,
`attachment_not_found`, `attachment_already_used`, `internal`.

## Objects

```json
User      {"id": 7, "username": "anna", "display_name": "Anna", "created_at": "…",
           "avatar_version": 3}
Member    {"id": 7, "username": "anna", "display_name": "Anna", "role": "owner|member",
           "avatar_version": 3}
Family    {"id": 3, "name": "The Smiths", "join_policy": "open|approval", "created_at": "…"}
          — plus "invite_code": "ABCD2345" when (and only when) the caller is the owner
JoinRequest {"id": 12, "user": {User}, "created_at": "…"}
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
          — plus "attachment": {Attachment} when the message carries a photo, video or file.
ReplyTo   {"message_id": 41, "sender_id": 9, "excerpt": "See you at six"}
Reaction  {"user_id": 9, "emoji": "❤️"}
Attachment {"id": 34, "kind": "photo|video|file", "mime": "image/jpeg", "size": 182734,
            "width": 1600, "height": 1200, "duration_ms": 8400, "has_preview": true,
            "name": "receipts.pdf"}
           — "duration_ms" on videos only; "width"/"height" absent when the uploader
             could not determine them; "name" on files only (their whole identity),
             and a file never has a preview or dimensions
Note      {"id": 12, "author_id": 7, "text": "Milk", "color": "yellow",
           "x": 0.42, "y": 0.13, "created_at": "…", "updated_at": "…", "board_seq": 88}
          — plus "deleted": true INSTEAD of the content fields on a tombstone; see "Board"
```

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

### Board

Each family has exactly one board: a wall of sticker notes anyone in the family can add to and
rearrange. Notes are not messages — they carry no unread count and never appear in a chat.

**A NEW note does notify.** This reverses the original "raise no notification": a board nobody is
told about is a board nobody reads, and a note pinned to the family wall is exactly the kind of
thing meant to be seen. Only CREATION notifies. Edits, moves and deletes do not — tidying the wall
is the shared act (see below), and a push for every drag would make the board unusable. The author
is never notified of their own note. Everything else about push applies unchanged: only members
with no live socket are pushed, because an open socket already delivers the `board_note` frame.

Counting what is new is the CLIENT's job, and it needs a marker of its own. `max_board_seq` is a
SYNC cursor — it advances whenever a client applies a change, including a background resync — so
using it to mean "seen" would clear the badge for a user who never opened the board. A client keeps
a separate high-water mark of the note ids it has SHOWN the user, advanced only when the board is
actually opened, and counts live notes above it. Note ids are server-assigned and monotonic, so
"created since you last looked" needs nothing further from the server.

Who may do what: **anyone in the family may MOVE a note; only its author may change its text or
colour, or delete it.** Moving is the shared act (tidying the wall together), authorship is the
personal one.

`x` and `y` are fractions of the board, `0.0`–`1.0` from the top-left, so a note sits in the same
relative place on a phone and a tablet. Values outside that range are CLAMPED rather than
rejected — a drag that ends past the edge should stick to the edge, not fail. `color` is one of
`yellow`, `pink`, `blue`, `green`, `orange`, `purple`; anything else is `invalid_note_color`.
`text` is trimmed, non-empty and at most 280 characters.

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

### The assistant

Each member may have one private chat with an assistant, and **private is the whole point**: it
belongs to that member alone, no other member can read it, and it is never part of the family chat.
`kind` is `"ai"`, `user_a_id` is its owner, and `GET /chats` returns it only to them.

**The assistant is only ever shown that member's own AI thread.** Not the family chat, not another
member's AI chat, not anything anyone else wrote. This is the invariant that lets a self-hosted
family server talk to a hosted model at all: a member asking a question sends their own words, and
nothing anybody else said ever leaves the server. A future setting to widen this would be a change
to what leaves the building, not a preference, and would need saying so here first.

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
in any roster (`GET /families/mine` selects by family, and the assistant has none), the username is
refused at registration so nobody can impersonate it, and it can never be messaged directly.

Clients need no special id: an `ai` chat has exactly two participants, so a message in one that is
not yours is the assistant's. Draw it with the chat's own name and icon rather than looking the
sender up in the roster, where it will not be found.

**The assistant answers in the language the question was asked from.** A client sends
`Accept-Language` with its send (Apple's URLSession does automatically; Android sets it from the
app's own resolved locale), and the server appends one instruction to the configured system prompt
naming that language. Only the first tag's primary subtag is used — the header may be a whole
weighted list, and what is wanted is "the language this device is in", not a negotiation. A missing
or unusable header adds nothing, and the model simply answers in the language it was written to.

This is deliberately per-DEVICE rather than per-account: the same person may run the app in Russian
on their phone and English on a work Mac, and each question should come back in the language it was
asked from. It is also one system prompt rather than one per language — what the assistant is FOR
does not change with the language, only what it answers in.

The feature is OFF unless the server is configured for it (`[ai] enabled`, an endpoint, a deployment
and a key). A server without it simply never creates the chat, and `POST` to one that does not exist
is the usual `chat_not_found`.

### Photos, videos, audio and files

A message may carry one attachment: a photo, a video, a piece of audio, or any other file. One, not
many: sending three photos makes three messages, which is what a thread shows anyway, and it keeps
both the wire shape and the bubble layout honest.

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

An attachment belongs to whoever uploaded it until a message claims it, and to that message's chat
afterwards. Before it is claimed only the uploader may read it; after, every member of the chat
may. An attachment can be claimed once: a second message naming it is `attachment_already_used`.
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

```json
{"generated_at": "…",
 "totals": {"members": 4, "messages": 1284, "board_notes": 7,
            "attachments": {"count": 96, "photo": 61, "video": 12, "audio": 9, "file": 14,
                            "bytes": 734003200, "stored_bytes": 612368384},
            "ai": {"questions": 43, "prompt_tokens": 12040, "completion_tokens": 30512}},
 "members": [{"user_id": 7, "display_name": "Anna", "messages": 512,
              "attachments": {"count": 31, "photo": 22, "video": 4, "audio": 3, "file": 2,
                              "bytes": 241172480},
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

## REST endpoints

### Auth

| Method & path | Body → Response |
|---|---|
| `POST /auth/register` | `{username, display_name, password}` → `201 {token, user: User}`. Username: 3–32 chars `[a-zA-Z0-9_.]`, case-insensitively unique. Password ≥ 8 chars. Display name 1–64 chars. Errors: `username_taken`, `validation`. |
| `POST /auth/login` | `{username, password}` → `200 {token, user: User}`. Error: `invalid_credentials` (401). |
| `POST /auth/logout` | (auth) → `204`. Revokes the calling session and closes its sockets. |
| `POST /me/password` | (auth) `{current_password, new_password}` → `204`. Changing your own password requires proving you know the current one — a live session is not proof, because an unattended unlocked phone is exactly what this protects against. Every OTHER session of yours is revoked and its sockets closed; the calling session survives, so the device making the change stays signed in. Errors: `invalid_credentials` (401, wrong current password), `validation` (new password under 8 characters). |
| `POST /families/members/{id}/password` | (owner) `{new_password}` → `204`. The owner resets a member's password WITHOUT knowing the current one — the whole point is that the member has forgotten it. ALL of that member's sessions are revoked and their sockets closed, so every device they are signed in on returns to login; that is what makes a reset a recovery rather than a convenience. The owner cannot target themselves here (`POST /me/password` is for that), and a user outside the family is `not_same_family` whether or not they exist. Errors: `not_family_owner` (403), `not_same_family` (403), `validation`. |
| `GET /me` | (auth) → `200 {user: User, family: Family\|null, role: "owner"\|"member"\|null, pending_join_request: {family_id, family_name, created_at}\|null}`. `pending_join_request` is the caller's live join request, if any — a client that was waiting and sees neither `family` nor `pending_join_request` knows the request was rejected. |

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
| `POST /families/join` | `{invite_code}` → `200 {status: "joined"}` (policy `open` — membership immediate) or `200 {status: "pending"}` (policy `approval` — join request created). Errors: `invalid_invite_code` (404), `already_in_family`, `join_request_pending`. |
| `GET /families/mine` | → `200 {family: Family, members: [Member], max_board_seq: 88}`. `max_board_seq` is omitted while the board is empty and untouched — it is how a client knows whether a board catch-up is worth a request. `family.invite_code` present for the owner only. Error: `not_in_family`. |
| `POST /families/invite-code/rotate` | (owner) → `200 {invite_code}`. Old code stops working; pending requests survive. |
| `PATCH /families/mine` | (owner) `{join_policy: "open"\|"approval"}` → `200 {family: Family}`. |
| `GET /families/join-requests` | (owner) → `200 {requests: [JoinRequest]}` (pending only). |
| `POST /families/join-requests/{id}/approve` | (owner) → `200 {member: Member}`. Errors: `join_request_not_pending`, `user_already_in_family`. |
| `POST /families/join-requests/{id}/reject` | (owner) → `204`. Error: `join_request_not_pending`. |
| `POST /families/leave` | → `204`. The owner may leave only as the sole member (the family is then deleted); otherwise `409 owner_cannot_leave`. Leaving removes the caller from the family chat and their direct chats; history is retained and resurfaces on rejoin. |
| `DELETE /families/members/{user_id}` | (owner) → `204`. Error: `cannot_remove_owner`. |

### Attachments

| Method & path | Body → Response |
|---|---|
| `POST /attachments` | Raw bytes with `Content-Type` set to the media type. Query: `kind` (`photo`\|`video`\|`file`), `width`, `height`, `duration_ms`, `name` (all optional except `name`, which is REQUIRED for `kind=file` and 1–255 characters). → `201 {attachment: Attachment}`. Errors: `attachment_too_large` (413), `invalid_attachment` (415 for a media type not accepted on a photo/video, 400 when the bytes do not match the declared type or a file has no name), `not_in_family`. |
| `PUT /attachments/{id}/preview` | Raw JPEG bytes of the downscaled photo or poster frame → `204`. Uploader only, and never on a `file` (`invalid_attachment`). Errors: `attachment_not_found`, `attachment_too_large`, `invalid_attachment`. |
| `GET /attachments/{id}` | → `200` with the stored bytes and their `Content-Type`. A `file` additionally gets `Content-Disposition: attachment; filename=…` (sanitised) and `X-Content-Type-Options: nosniff`, so an uploaded document can never render or execute from the server's own origin. Readable by the uploader always, and by every member of the chat once a message claims it; anyone else gets `404 attachment_not_found`. Sends `ETag` and `Cache-Control: private, max-age=31536000, immutable`, and honours `If-None-Match` with `304`. Honours a single-byte-range `Range` request with `206` + `Content-Range` (`416` for a range past the end) — that is how a video player seeks, and without it scrubbing a 90 MB clip re-downloads it from the start. A multi-range or unrecognised `Range` is ignored and the whole body sent, per RFC 9110. |
| `GET /attachments/{id}/preview` | → `200` with the preview JPEG, same access rules. `404` when there is no preview yet. |

### Board

| Method & path | Body → Response |
|---|---|
| `GET /families/mine/board` | → `200 {notes: [Note], max_board_seq: 88}`. The whole board as it now stands, tombstones excluded, newest `board_seq` first. `max_board_seq` is `0` for a board nothing has ever been written to. Error: `not_in_family`. |
| `GET /families/mine/board/changes` | Query: `after_seq` (default 0), `limit` (default 50, max 200) → `200 {notes: [Note]}` ordered by `board_seq` ascending, INCLUDING tombstones — the board catch-up, looped until a short page. Errors: `not_in_family`, `invalid_pagination`. |
| `POST /families/mine/board/notes` | `{text, color, x, y}` → `201 {note: Note}`. Caller becomes the author. Errors: `validation` (text empty or > 280), `invalid_note_color`, `board_full` (409, over the note ceiling), `not_in_family`. |
| `PATCH /families/mine/board/notes/{id}` | `{text?, color?, x?, y?}` → `200 {note: Note}`. Any member may send `x`/`y`; only the author may send `text` or `color` (`not_note_author`, 403). Sending nothing that differs is a no-op: no new seq, no fan-out. Errors: `note_not_found` (404), `not_note_author`, `invalid_note_color`, `validation`, `not_in_family`. |
| `DELETE /families/mine/board/notes/{id}` | → `204`. Author only. Idempotent: deleting an already-deleted note is still `204` and takes no new seq. Errors: `note_not_found`, `not_note_author`, `not_in_family`. |

### Chats & messages

| Method & path | Body → Response |
|---|---|
| `GET /chats` | → `200 {chats: [{chat: Chat, last_message: Message\|null, unread_count: 3, max_reaction_seq: 123}]}`. Family chat included always; direct chats once they exist. `max_reaction_seq` is omitted while no message in the chat has ever been reacted to, and `max_edit_seq` likewise while nothing in it has been edited; `last_message` previews never carry `reactions` or the quote, but DO carry `attachment` — a photo sent without a caption has an empty body, and a preview with nothing in it is a chat row that looks like nothing happened. |
| `POST /chats/direct` | `{user_id}` → `200 {chat: Chat}` — get-or-create, idempotent. Errors: `cannot_dm_self`, `not_same_family`, `user_not_found`. |
| `GET /chats/{id}/messages` | Query: `before_id` XOR `after_id` (optional), `limit` (default 50, max 200) → `200 {messages: [Message]}`. `before_id`: strictly older, **newest-first** (history pages). `after_id`: strictly newer, **oldest-first** (reconnect catch-up). Neither: the newest `limit`, newest-first. Errors: `chat_not_found`, `not_chat_member`, `invalid_pagination`. |
| `POST /chats/{id}/messages` | `{client_msg_id: "<uuid>", body, reply_to_message_id?, attachment_id?}` → `201 {message: Message}`. Retrying with the same `client_msg_id` returns the existing message as `200` — never a duplicate. Body: trimmed, non-empty, ≤ 4000 chars. `reply_to_message_id` is optional and must name a message in this same chat (see "Replies"). `attachment_id` claims an attachment this caller uploaded; a message carrying one may have an empty body. Errors: `message_empty` (no body AND no attachment), `message_too_long`, `not_chat_member`, `message_not_found` (the reply target is not a message in this chat), `attachment_not_found`, `attachment_already_used`. |
| `PATCH /chats/{id}/messages/{mid}` | `{body}` → `200 {message: Message}`. Author only. Replaces the body, stamps `edited_at` and the next `edit_seq`, and fans out `message_edited`. Body rules are the send rules: trimmed, non-empty, ≤ 4000 chars. Re-sending the body it already has is a no-op: no new seq, no fan-out. Errors: `message_empty`, `message_too_long`, `not_message_author` (403), `message_not_found` (404 — no such message *in this chat*), `not_chat_member`, `chat_not_found`. |
| `GET /chats/{id}/edits` | Query: `after_seq` (default 0), `limit` (default 50, max 200) → `200 {messages: [Message]}` ordered by `edit_seq` ascending — the edit catch-up, looped until a short page like `after_id`. Errors: `chat_not_found`, `not_chat_member`, `invalid_pagination`. |
| `POST /chats/{id}/read` | `{last_read_message_id}` → `204`. Monotonic — the server keeps the max ever reported. |
| `PUT /chats/{id}/messages/{mid}/reaction` | `{emoji}` → `200 {message_id, reaction_seq, reactions: [Reaction]}`. Sets or replaces the caller's reaction on the message — an idempotent state-set, not a toggle (clients decide locally whether a tap means set or remove). One reaction per user per message. Emoji: trimmed, non-empty, ≤ 32 bytes UTF-8. Re-PUT of the current emoji is a no-op: no seq bump, no fan-out. Errors: `invalid_emoji`, `message_not_found` (404 — no such message *in this chat*), `not_chat_member`, `chat_not_found`. |
| `DELETE /chats/{id}/messages/{mid}/reaction` | → `200 {message_id, reaction_seq, reactions: [Reaction]}`. Removes the caller's reaction; idempotent (deleting nothing returns the current state unchanged). Same errors minus `invalid_emoji`. |
| `GET /chats/{id}/reactions` | Query: `after_seq` (default 0), `limit` (default 50, max 200) → `200 {message_reactions: [{message_id, reaction_seq, reactions: [Reaction]}]}` ordered by `reaction_seq` ascending — the reaction catch-up, looped until a short page like `after_id`. Errors: `chat_not_found`, `not_chat_member`, `invalid_pagination`. |

### Devices (push hook — no delivery in v1)

| Method & path | Body → Response |
|---|---|
| `POST /devices` | `{platform: "ios"\|"macos"\|"android", push_token: string\|null}` → `201 {device_id}`. Upserts by token when non-null. `macos` is delivered over APNs alongside `ios` — the macOS build shares the iOS bundle id, so it shares the APNs topic and the payload is identical; it is a distinct platform in the DATA because a Mac claiming to be an iPhone makes every future question about delivery harder to answer. |
| `DELETE /devices/{id}` | → `204`. Error: `device_not_found`. |

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
                   "attachment_id": 34}
{"type": "read",   "chat_id": 42, "last_read_message_id": 1337}
{"type": "typing", "chat_id": 42}
{"type": "ping"}
```

### Server → client

```json
{"type": "ack",     "client_msg_id": "8f14e45f-…", "message": {Message}}
{"type": "message", "message": {Message}}
{"type": "read",    "chat_id": 42, "user_id": 9, "last_read_message_id": 1338}
{"type": "typing",  "chat_id": 42, "user_id": 9}
{"type": "member_joined", "family_id": 3, "user": {"id": 11, "username": "junior", "display_name": "Junior", "avatar_version": 0}}
{"type": "member_left",   "family_id": 3, "user_id": 11}
{"type": "reaction", "chat_id": 42, "message_id": 1338, "reaction_seq": 124,
                     "reactions": [{"user_id": 9, "emoji": "❤️"}]}
{"type": "message_edited", "message": {Message}}
{"type": "board_note", "note": {Note}}
{"type": "ai_delta", "chat_id": 42, "message_id": 1339, "text": "…"}   — assistant, mid-reply
{"type": "ai_error", "chat_id": 42, "message_id": 1339}                — it stopped early
{"type": "pong"}
{"type": "error",   "code": "not_chat_member", "message": "…", "client_msg_id": "8f14e45f-…"}
```

(`client_msg_id` on `error` is present when the error answers a `send`.)

`message_edited` carries the whole message, exactly as `message` does, and is a SEPARATE frame
type on purpose: `message` is what bumps unread counts and raises a notification, and an edit must
do neither. Clients apply it under the `edit_seq` guard described under "Editing".

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
  throttled server-side to one per chat per 3 s per connection.
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
- **Keepalive**: the server pings (WS protocol level) every 30 s and drops sockets idle for
  75 s. Clients should send `{"type":"ping"}` every ~25 s if their WS library hides protocol
  pings, and treat a missing `pong` as a dead connection.
- **Best-effort delivery**: the socket is a live wire, not a queue. A slow client's socket is
  dropped; REST is the source of truth. On every (re)connect a client must resync:
  1. `GET /me` — reconcile membership.
  2. `GET /chats` — chat list, previews, authoritative unread counts.
  3. Per chat: `GET /chats/{id}/messages?after_id=<max known message id>` looped until a short
     page — message ids are globally monotonic, so `max(id)` is the sync cursor.
     Then, when the chat's `max_reaction_seq` from step 2 exceeds the locally stored reaction
     cursor: `GET /chats/{id}/reactions?after_seq=<stored cursor>` looped until a short page,
     applied per message under the `reaction_seq` guard. The stored cursor advances with every
     page **even for messages the client does not hold** (states for unknown messages are
     dropped; history paging re-delivers them embedded on the `Message` objects).
  4. Re-send any locally pending outbound messages (safe: `client_msg_id` dedups).

## Push notifications

The server pushes only to users with **no live socket** (an open WebSocket means the device is
getting frames already). Four events push: a **new message** (to every offline chat member), a
**new board note** (to every other offline family member), a **join request created** (to the family
owner), and a **join request approved** (to the requester). Typing, reads, reactions, note moves and
edits, and other frames never push.

Device lifecycle: `POST /devices {platform, push_token}` upserts by token (re-login moves the
token to the new account); the response `device_id` should be stored so `DELETE /devices/{id}`
can be called on logout. Clients re-POST whenever the OS rotates the token. A push rejected as
unregistered (APNs `410`/`BadDeviceToken`, FCM `UNREGISTERED`) deletes the device row.

Titles: direct chat → sender's display name; family chat → `"<Family> — <Sender>"`. Body: the
message text, or `"New message"` when the server's `[push] include_message_body = false`.

A new board note pushes with `"kind": "board_note"` and `family_id` + `note_id` instead of chat and
message ids. Title `"<Family> — <Author>"`, body the note's text (or `"New note"` when
`include_message_body = false`, the same switch that governs message bodies). Tapping it opens the
board.

A message carrying an attachment MAY have an empty body — which is how a photo is normally sent —
and an alert showing a name above a blank line says nothing arrived. Such a message pushes what
arrived instead: `"Photo"`, `"Video"`, `"Audio"`, or the file's name. A caption, when there is one,
still wins.

APNs (token-based auth, HTTP/2): headers `apns-topic` = bundle id, `apns-push-type: alert`,
`apns-priority: 10`; payload:

```json
{"aps": {"alert": {"title": "Anna", "body": "Dinner at 7?"}, "sound": "default",
         "badge": 3, "thread-id": "chat-42"},
 "chat_id": 42, "message_id": 1338, "kind": "message"}
```

(`badge` = the user's total unread across chats at send time. Join events use
`"kind": "join_request"` / `"kind": "joined"` with `family_id` instead of chat/message ids.)

FCM (HTTP v1): notification + data so the system tray renders when the app is dead, while a
foregrounded app (socket live) is never pushed at all:

```json
{"message": {"token": "…",
  "notification": {"title": "Anna", "body": "Dinner at 7?"},
  "data": {"kind": "message", "chat_id": "42", "message_id": "1338"},
  "android": {"priority": "HIGH",
              "notification": {"channel_id": "messages", "tag": "chat-42"}}}}
```

Tapping a notification opens the chat named by `chat_id` (or the board for `board_note`, the
join-requests screen for `join_request`, the chat list for `joined`).

## Limits (server defaults, configurable)

| Limit | Default |
|---|---|
| Message body | 4000 chars |
| Reaction emoji | 32 bytes UTF-8 (fixed) |
| Page size | 50 default / 200 max |
| HTTP body | 16 KiB (except `PUT /me/avatar`) |
| Profile picture | 256 KiB |
| Per-socket outbound queue | 64 frames |
| Session TTL | 180 days, sliding |
