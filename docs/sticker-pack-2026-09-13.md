# A family sticker pack — what it would take (issue #58)

**Assessed 2026-09-13** against `v1.1`, in answer to issue #58: *"I think we can start from webp
stickers. One Family - one sticker pack. It can be bigger than usual."* This document says what
the request implies, which facts were checked rather than assumed, where the hard parts are, what
design is recommended and why, and what has to be decided before any code is written. **No code has
been written for it.**

---

## What is asked, and what it implies

- **WebP.** Transparency and animation in one small file, which is why messengers use it for
  stickers. Today the server REFUSES it as a photo, Apple cannot write it, and neither phone app
  animates any picture.
- **One family, one pack.** The pack is family PROPERTY, like the board — not message history.
  That decides where it lives on the server and what the retention sweep may do to it.
- **Bigger than usual.** The picker has to scroll, and the ceiling should be an operator setting.

What is NOT in the issue, and has to be decided: who may add and remove stickers, whether animated
stickers are in the first version, what a sent sticker IS on the wire, what the wire CALLS it, and
how big one may be.

## Checked facts

**The server**

- **A photo may be `image/jpeg`, `image/png`, `image/heic` or `image/heif`** (protocol.md, under
  the upload rules), and the magic-number table (`handlers_attachment.rs::matches_magic`) has no
  WebP arm. A WebP photo is `invalid_attachment` today. The check itself would be one arm: `RIFF` at
  0 and `WEBP` at 8.
- **The server never decodes an image** — it checks the magic number and stores the bytes. It cannot
  turn a PNG into a WebP for a device that cannot make one, and should not start.
- **An attachment is claimed by `message_id` or `note_id`** (0042). The unclaimed-upload sweeper
  deletes anything with neither after `attachment_grace_hours`. A third claim the sweeper does not
  learn in the SAME migration is a delay fuse — every pack item would vanish hours after it was
  added. 0042's header records exactly this trap for board pictures.
- **Identical bytes are one file per family**, and a file is removed only when NO attachment row
  names it: `remove_if_unreferenced` asks `EXISTS(SELECT 1 FROM attachments WHERE storage_key = $1)`
  — any row, whatever claims it. So a sent sticker aging out under retention cannot take the pack's
  copy with it, and removing a sticker from the pack cannot break a message that used it.
- **Retention leaves the board alone** — "a note is a live thing on a wall, not history." A pack is
  the same kind of thing.
- **A departed member's notes stay**, attributed to "Deleted account", and **deleting a note is
  "Author only"**. The pack's removal rule has to account for that (decision 1).
- Profile pictures are capped at 256 KiB; attachments at 100 MB. The next migration is 0046.

**"Sticker" is already a word in this codebase — for a board NOTE.** protocol.md's drawing rules
say "on the sticker" for a note on the wall, and the code uses the word a few hundred times in that
sense (`StickerProps`, `a_sticker_draws_the_first_lines_of_a_list…`, `server/src/config.rs`). It is
in **no user-visible string**: no key in the Apple catalogue, nothing in Android's or Windows'
strings, only comments in `web/text`. So the apps may say "sticker" to people freely, but a wire
field or table named `sticker` would mean two different things in one document.

**Apple (iOS 26, macOS 26)**

- **Decodes WebP, including animated** — `CGImageSourceCopyTypeIdentifiers()` contains
  `org.webmproject.webp` (run on this Mac).
- **Cannot encode WebP** — `CGImageDestinationCopyTypeIdentifiers()` does not contain it. An Apple
  device can add an existing `.webp` to the pack; it cannot make one from a photo.
- **Draws frame zero only.** Every attachment goes through `PlatformImage.decode`, which is
  `CGImageSourceCreateThumbnailAtIndex(source, 0, …)`. The SDK has `CGAnimateImageDataWithBlock` for
  animation.
- `MediaPrep.sendsAsFile` sends a GIF/WebP/BMP from the chat composer as `kind=file`, because the
  photo path keeps only the first frame. A pack is a different path and would not change that.

**Android (minSdk 26)**

- **Decodes static WebP on every supported version** (`BitmapFactory`).
- **Animation needs API 28** — `ImageDecoder` and `AnimatedImageDrawable` are both `since="28"` in
  the platform's `api-versions.xml`. On 26–27 a sticker would be drawn still.
- **Can encode WebP on every supported version** — `Bitmap.CompressFormat.WEBP` is `since="14"`,
  deprecated at 30 in favour of `WEBP_LOSSY` / `WEBP_LOSSLESS`.
- **Draws frame zero only today** — attachments decode through `AvatarImage.decode` →
  `BitmapFactory`, and the app has no image library.

**Web**

- A `kind == "photo"` attachment is drawn as `<img src={objectUrl}>`, which decodes and animates WebP
  in every current browser. The file-name extension map (`attachments.rs`) lacks `webp`, which only
  matters when saving.

**Windows**

- **Not checked — it cannot be from this Mac.** WinUI draws through WIC, whose WebP support comes
  from a codec package; whether it is present on the minimum Windows build (still an open #64
  decision), and whether it animates, needs a real Windows machine.

## The hard parts

1. **A sent sticker has to outlive the pack.** Remove a sticker and every message that used it must
   still draw something. That is the choice between the two wire shapes below.
2. **Old clients.** A shipped app must show something sensible for a sticker message under the
   rule that a client ignores what it does not know.
3. **Animation is uneven.** Free on the web; an ImageIO animation loop on Apple; `AnimatedImageDrawable`
   on Android 28+ and still below; unknown on Windows. A sticker must never be REFUSED for being
   animated — only drawn still where the device cannot do better.
4. **Authoring is asymmetric.** Android can write WebP, Apple cannot, and the server will not
   convert.
5. **The sweeper** has to learn the pack's claim in the same migration.
6. **The name.** See above.

## Recommended design

### Vocabulary

The apps say "sticker"; **the wire and the database say `pack`** — `pack_items`, `pack_item_id`,
`GET /families/mine/pack`, a `pack_item` frame, `max_pack_seq`. protocol.md's section is titled
"Sticker pack" and says once, at the top, that "sticker" elsewhere in the document means a board
note. Renaming the board's existing internal "sticker" would touch hundreds of lines on four
clients for no user-visible gain, and is not recommended.

### The pack

A table `pack_items` (`id`, `family_id`, `attachment_id`, `added_by`, `created_at`, `pack_seq`, and
an optional short `label` for accessibility). A pack item's picture is uploaded as an ordinary
`kind=photo` — WebP added to the photo types and the magic table — and then claimed with
`POST /families/mine/pack` `{attachment_id}`. The claim is a nullable `pack_item_id` on
`attachments` with a partial unique index, the shape of 0042's `note_id`, and **`sweep_unclaimed`
learns `AND pack_item_id IS NULL` in the same migration**. The per-item byte ceiling is checked at
the claim, because the upload does not know it will become a pack item. `ON DELETE CASCADE` from the
family; retention does not touch it.

Removal is `DELETE /families/mine/pack/{id}`. A `pack_item` frame carries an added or removed item to
every member, and `max_pack_seq` on `GET /families/mine` gives it a catch-up cursor, with a
tombstone for a removal — the board's exact shape, including the gone set every client now keeps,
so no client learns a new sync idea.

### What a sent sticker is — recommended: a COPY

**Option A (recommended): the message carries its own attachment.** The sender uploads the sticker's
bytes as an ordinary `kind=photo` and sends the message as it would a photo, with one new field on
the attachment, `sticker: true` (absent otherwise, by the usual rule), set by the send.

- **Nothing new on the send path.** The outbox, background uploads (#68) and retries are unchanged
  on every client. A device already holds the bytes, because the picker shows them. Dedup makes the
  upload one file on disk; it still costs the upload itself, at most one pack item's ceiling.
- **The message is ordinary history.** Retention sweeps it like any other, and — checked above —
  that never touches the pack's file, nor does removing the item touch the message's.
- **Old clients draw a photo.** Apple's `CGImageSource`, Android's `BitmapFactory` and the browser
  all decode a still WebP today, so an old app shows the first frame in a photo bubble. New clients
  read the flag and draw a sticker: no bubble, larger, animated where they can.

A later optimisation, if the upload ever matters, is a server call that returns a fresh unclaimed
attachment row naming the pack item's file — the row shape the dedup path already writes. It would
need its own offline story, which is why it is not in the first version.

**Option B: the message names the pack item** (`pack_item_id` on the message). No upload per send,
and a pack edit could re-skin old messages — but a removed item leaves every message that used it
pointing at nothing, which needs a tombstone and a drawing rule of its own, and an old client shows
only a placeholder body. Not recommended.

### Animation

Store what was uploaded. Draw animated where the platform can (web always; Apple through
`CGAnimateImageDataWithBlock`; Android through `AnimatedImageDrawable` on 28+), frame zero elsewhere.
Nothing is refused, re-encoded or stripped for being animated.

### Limits (proposed defaults, operator-configurable)

| Limit | Proposed | Why |
| --- | --- | --- |
| Items in one family's pack | 200 | "bigger than usual" |
| One item | 512 KiB | room for a few seconds of animation; twice the profile-picture ceiling |
| Pixel size | 512 × 512, a CLIENT rule | the server never decodes; a client downscales what it writes and takes a WebP as given |

## Phases

1. **Protocol and server.** protocol.md first: the "Sticker pack" section, WebP as a photo type, the
   pack's claim and the sweeper, the frame and its cursor, and the `sticker` flag. Migration 0046,
   handlers, integration tests against a scratch Postgres.
2. **Web** — it animates for free, so the whole loop is visible soonest: the pack in the family pane,
   a picker in the composer, drawing.
3. **Apple** — picker, pack management, animated drawing.
4. **Android** — the same, animated on 28+.
5. **Windows**, once WebP drawing is checked on its minimum build.

Every phase ships on its own: an app that has not reached its phase draws a sent sticker as a photo.

## What has to be decided

1. **Who may remove a pack item?** The board's rule is author only — but a departed member's notes
   stay, so under that rule a departed member's stickers could never be removed by anybody.
   Recommended: **the author or the family owner**, which is a permission shape the board does not
   have. Adding stays open to every member.
2. **Animated stickers in the first version?** Recommended: yes. Static-only still needs every
   client's frame-zero path, so it saves little.
3. **Accept PNG as well as WebP?** Recommended: yes. Without it an Apple device can only add
   stickers that already exist as `.webp` files.
4. **Option A (a copy on the message) or Option B (a name for the pack item)?** Recommended: A.
5. **Wire vocabulary**: `pack` on the wire, "sticker" in the apps? Recommended: yes.
6. **The limits** — the three rows above.

Nothing here changes existing behaviour until phase 1 is written, and every phase keeps the shipped
apps working.
