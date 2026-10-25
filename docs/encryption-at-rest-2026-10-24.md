# Encryption at rest, in the server (#59)

*Plan, 2026-10-24. **DECIDED 2026-10-24/25 — E1 (a) files too; E2 (a) everything personal; E3 (b) a key per family;
E4 (a) converted at start-up; E5 (a) keys backed up separately, kept 7 days.** Built on the server 2026-10-25: §3's
"In the code" says where; what changed from the plan while building it is marked *(built)*.*

The owner's words (2026-10-24): *"Store to database only encrypted data. The server app should use decrypted in memory,
but in PostgreSQL data should be encrypted."* That is design 2 of the 2026-09-13 assessment
(`docs/family-encryption-2026-09-13.md`): encryption at rest done by the application, with keys the server keeps.
Nothing changes on the wire, so **no client changes**.

## 1. What it protects, and what it does not

It protects against somebody who has the database but **not the key file**:

- a `pg_dump`, a backup (restic or rsync), a hosting snapshot, a stolen or recycled disk;
- a database-level read: a replica, a leaked PostgreSQL password, statement logs (what the server binds is already
  ciphertext when it reaches PostgreSQL).

Because every family, conversation and account has its own key (E3), it also makes **deletion reach the backups**: once
a family, a group, a direct or assistant chat, or an account is deleted, and the key copies older than the deletion have
expired, no backup can open it — not even with the key file (§5).

It does **not** protect against:

- **root on the running server, or whoever runs it.** The server reads the key file and holds plaintext in memory while
  it works. Keeping the operator out is end-to-end encryption, a different project (the assessment's design 3).
- **somebody who can write to the database.** They can already sign in as anyone by adding a session row, so a
  ciphertext is not tied to its row (§3).
- what already leaves the server: push text through APNs/FCM, the assistant's provider, TLS ending at nginx.
- **the shape of the data**: who is in which family, who talks to whom and when, how many messages and how long each is
  (a sealed value is the plaintext's length plus 49 bytes), file sizes and kinds.
- **copies made before the upgrade**: old backups, a WAL archive and free disk blocks keep plaintext until deleted or
  overwritten (§6).
- **a single message, note or file deleted on its own** — by its author, or by retention. Its conversation's key lives
  on, so older backups still open it until they expire (a year under today's restic policy).

And it adds a failure that does not exist today: **losing the key file, or every copy of the keys, loses every family
on the server** — messages, names, photos, all of it (§5).

## 2. What is stored today

Checked 2026-10-24 against all 58 migrations, applied to a scratch database.

- **33 tables, 237 columns.** Content lives in about 40 of them (§3).
- **Already one-way**: session tokens are stored as SHA-256 (`sessions.token_hash`), passwords as argon2id.
- **Profile pictures are in the database** (`user_avatars.bytes`). Every other picture is a file.
- **Files**: attachments and their previews under `attachments_dir`, deduplicated per family by a SHA-256 of the bytes.
  `GET /attachments/{id}` answers byte ranges. The server never transcodes. It touches stored bytes in 8 places: the
  upload and preview writers, the magic-number check, serving, the transcript request, the assistant reading a photo,
  and the assistant saving a picture it drew. A sticker message carries its own copy of the picture.
- **What deletes what**: a family is deleted when its last member leaves or deletes their account — and a direct chat
  that named it is first moved to another family the pair shares. A group is deleted by its admin or when its last
  member leaves. Deleting an account deletes its direct chats and its assistant chats, and scrubs the account row,
  which stays as "Deleted account".
- **Where PostgreSQL computes on content** — the only SQL that ciphertext breaks:
  - signing in: `lower(username) = lower($1)`, and a unique index on `lower(username)`;
  - the assistant's account found by `lower(username) = 'assistant'` in 3 queries and in the 2 triggers that protect
    it;
  - a scrubbed account renamed `'deleted-' || id` and recognised by `LIKE 'deleted-%'`;
  - joining: `WHERE invite_code = $1`, with a unique index;
  - registering a device: `ON CONFLICT (push_token)`, with unique indexes on the push and VoIP tokens;
  - attachment dedup: `content_hash = $2`;
  - the birthday greeting: `SELECT DISTINCT birthday_month, birthday_day`;
  - statistics: `ORDER BY … display_name`;
  - reactions: `ON CONFLICT … WHERE emoji IS DISTINCT FROM EXCLUDED.emoji`;
  - 9 CHECK constraints read content. 6 move to Rust validation: file-name length, the coordinates' range, the
    waveform's form, sticker-label length, the number of greeting places, a valid birthday. The other 3 test only
    NULL and stay.
- **Nothing else.** No text search, no `ILIKE`, no excerpt cut in SQL, no stored frames (resync reads the current
  rows). A `CASE` or `COALESCE` that picks between two content columns keeps working on ciphertext.
- **416 `sqlx::query` sites**, every row read by hand with `row.get` — there is no shared row mapping to change once.
  The integration tests read content columns directly in about 25 places.

## 3. The design

### Keys

Since #88 an account can be in several families and a direct chat belongs to two people rather than to a family, so
"a key per family" becomes **a key per thing the server deletes**:

| Key | Seals | Gone when |
| --- | --- | --- |
| **family** | the family's name, invite code and greeting places; join requests; member reports; the family chat; the board; the sticker pack | the family is deleted |
| **conversation** — one per group, direct chat and assistant chat | that conversation's messages, reactions, mentions, poll options, transcripts, and the assistant reports about it | the group is deleted; a direct or assistant chat is deleted with its account, or an assistant chat with its family |
| **account** | username, display name, password hash, birthday, profile picture, push and VoIP tokens | the account is deleted; its tombstone is re-sealed under a new key |
| **file** — one per stored file | the file and its preview | never on its own: it is wrapped by the key of whatever holds the file |

- Each is 32 random bytes, **wrapped by the server key** and kept in a `storage_keys` table under a random 63-bit id.
  *(built)* The OWNER names its key — `families.key_id`, `chats.key_id`, `users.key_id`, a family chat naming its
  family's — rather than the key naming its owner: a restore is then checked with a join, and an account whose key
  was REPLACED after the dump (it was deleted) shows up as missing, which a key-names-owner table could not see. A
  key is made in the same transaction as its owner, never lazily. **No foreign keys**: a restore puts a newer key copy
  onto an older dump (§5), which may hold keys for families that dump does not have yet, and one such row would fail
  the whole `COPY`. Instead a trigger on `families`, `chats` and `users` deletes the owner's key — so every cascade is
  caught, in the same transaction as the delete, and so is an account's key being replaced — and the server drops
  keys nobody names at start. Each row also records the fingerprint of the server key that wrapped it, so a refusal
  can say which key file it wanted.
- **The server key**: 32 random bytes, base64, in `/etc/family-connect/storage.key` (mode 0400, owned by the service
  user), named by `[storage] key_file`. Never in the database, never in `attachments_dir`, never in a backup.
  systemd's encrypted credentials (`LoadCredentialEncrypted=`, bound to the TPM where there is one) are documented as
  the stronger home for it.
- **Rotating the server key** re-wraps the rows of `storage_keys` and *(built)* recomputes the lookup columns, which
  are HMACs under the server key too (usernames, invite codes, device tokens — one row each, so it is still quick);
  no data is re-encrypted. `family-connect storage-key rotate` is part of this issue, beside `storage-key new` and
  `storage-key show` (which prints the key for a password manager or paper). The old key file is kept 7 days, because
  the key copies from before the rotation are wrapped by it.
- The server's command line gains those subcommands, `storage forget-missing` (§5) and *(built)* `reports`, which
  prints the assistant reports opened — they were read with `psql`, which now shows them sealed. With no subcommand it
  runs the server, as today, so the systemd unit does not change.
- **The server refuses to start** without the key file, or with the wrong one: *(built)* each `storage_keys` row
  records the fingerprint of the key that wrapped it, and the refusal names that and the key file's. It never falls
  back to writing plaintext.

### A sealed value

`0x01 | key id (8 bytes) | 24-byte nonce | ciphertext | 16-byte tag`, in a `bytea` column. **XChaCha20-Poly1305**
(RustCrypto's `chacha20poly1305`, pure Rust), a fresh random nonce on every write, 49 bytes over the plaintext.

- The value **names its key**, so reading needs no context: the key is unwrapped once and cached. Writing names the
  family, conversation or account it belongs to.
- A tampered value, a wrong key or a deleted key is an error, never garbage: the request answers `internal`, and the
  log names the table and the key id, never the content.
- No associated data ties a ciphertext to its row. The threat is a read, and the writer such a binding would stop can
  already add a session (§1). Leaving it out also keeps a `CASE` between two columns working.

### Lookups

What the server must **find** by value gets a second column holding an HMAC-SHA256. Equal inputs give equal results;
without the key nothing else can be learnt from them.

| Value | Lookup column | Keyed by | Normalised as |
| --- | --- | --- | --- |
| `users.username` | `username_key`, unique | the server key | ASCII lower case (usernames are ASCII by rule) |
| `families.invite_code` | `invite_code_key`, unique | the server key | upper case, as the join already does |
| `devices.push_token`, `voip_token` | `push_token_key`, `voip_token_key`, unique | the server key | as is |
| attachment dedup | `content_hash`, now the HMAC of the SHA-256 | the family key | — |

Signing in, joining and registering a device happen before any family is known, so those three are keyed server-wide.
In an old backup they let somebody holding the key file confirm a guess ("was there an account called anna?"), nothing
more. Dedup is per family already, so it uses the family key and goes with it.

The value itself is sealed beside its lookup, because the server hands it back: a member's username, the owner's
invite code, a token sent to APNs.

The `'assistant'` username test becomes a `users.is_assistant` flag (a one-row unique index, the triggers rewritten on
it), and the `deleted-` tests become `deleted_at IS NOT NULL`.

### What is sealed and what stays plain

The line (E2): **everything a person wrote, said, showed or is** is sealed. **What the server routes, orders, counts
and enforces** stays plain.

| Sealed | Plain |
| --- | --- |
| `messages.body` | every id and foreign key; memberships; chat members |
| `notes.text`, `notes.place`, `note_task_items.text`, `poll_options.text` | every timestamp and `*_seq`, event start and end included |
| `transcripts.text` | kinds, statuses, report reasons, platforms, languages, join policy |
| `chats.title`; `families.name`, `invite_code`, `greeting_places` | a note's colour, size, font and position |
| `users.username`, `display_name`, `password_hash`; the birthday, as one value | an attachment's kind, MIME type, size, dimensions, duration, storage key |
| `user_avatars.bytes` | RSVP answers, read positions, call records, usage counters |
| `attachments.name`, `latitude`, `longitude`, `waveform`, and the file key | `sessions.token_hash` (already a hash) |
| `message_reactions.emoji`; `message_mentions.name`, `note_mentions.name` | flags: `ai_*`, `sticker`, `round`, `animation`, … |
| `member_reports.message_excerpt`; `assistant_reports.message_excerpt`, `note` | |
| `pack_items.label`; `join_requests.invite_code_used` | |
| `devices.push_token`, `voip_token` | |

A sealed password hash also works as a pepper: a leaked dump cannot be put to offline guessing.

### Files (E1)

Every stored file has **its own file key**. It is sealed as it streams, in **64 KiB chunks**, each its own AEAD with the
nonce `file prefix | chunk number | last-chunk flag` — the STREAM construction age and Tink use — so a truncated or
reordered file fails to open. A file is `FCS1 | 19-byte prefix` followed by the chunks, 16 bytes per chunk over the
plaintext.

- **Whose key wraps it.** An upload is wrapped by the family it was uploaded for ("Uploads name their family"). When a
  message claims it, the file key and the row's few sealed fields are re-sealed under that conversation's key: 32 bytes,
  with the file untouched. Notes and stickers stay with their family.
- **Dedup** shares one file among rows that each wrap the same file key. A photo in a direct chat therefore stays
  readable when the family it was first uploaded for is deleted, and goes when the last row that wraps it goes.
- **Ranges**: a request maps to the chunks it covers, and only those are read and opened. `size_bytes` stays the
  plaintext size, so `Content-Length` and `Content-Range` do not change.
- **Previews** the same way. The magic-number check reads the plaintext as it streams in. The SHA-256 for dedup is taken
  over the plaintext as it streams, and stored as its HMAC.
- *(built)* `attachments.file_key` — the file's key, sealed under the holder's — is NULL exactly for a file the
  conversion has not reached, and says which files are sealed. A file is never guessed at by sniffing. New uploads are
  sealed from the first byte; a file from before is sealed into a copy whose name is the old one with an `s` in front
  (same shard directory), its rows move to the copy in one transaction that notes the old name in `storage_cleanup`,
  and the old file is removed after the commit — so a crash at any point leaves no plaintext behind for long, and the
  next pass finishes the job. Its rows are locked while it moves, as a preview upload locks its row, so no preview
  lands beside a file that just moved. Dedup never matches a file that is not sealed yet.

### In the code

*(built)*

- `vault.rs`: the cryptography over bytes — the key file, wrapping, sealing and opening, lookups, and the chunked file
  sealer and opener — pinned by fixed vectors an independent implementation produced.
- `keyring.rs`: where it meets the database. Every key is unwrapped at start and cached, and every key made since is
  added as it is made, so OPENING a value needs no query; SEALING asks the owner's key once. `SealedRow` reads sealed
  columns off a row (`row.text(&keys, "body")`), and a value that will not open is a `KeyError` that names the column
  and the key id, never the content.
- **Every content column becomes `bytea`**, and the values are plain `Vec<u8>` — no separate `Sealed` type was
  needed: a query that still binds or reads a `String` there fails in the test suite with a type error, and 0060's
  CHECKs make PostgreSQL itself refuse a value whose first byte is not 0x01. It cannot quietly write plaintext.
- Every write names its owner — `Owner::Family`, `Owner::Chat` (a family chat answers with its family's key) or
  `Owner::Account` — and the owner's key was made with it.
- The SQL of §2 that computes on content moves to Rust: statistics sort by name in Rust; the greeting reads birthdays
  and takes the distinct signs in Rust; the reaction upsert compares in Rust; the 6 CHECKs become Rust validation,
  where all of them already were.
- Deleting an account replaces its account key (a trigger deletes the old one) and re-seals the tombstone under the
  new one.
- `storage.rs` seals as it writes and opens by chunk as it reads; the upload, the preview, serving, the transcript,
  the photo the model is shown and the two pictures the assistant draws go through it. A message claiming an upload
  re-seals the row's file key and words under the conversation's key (`handlers_attachment::reseal_for`).
- `seal_existing.rs` is the start-up conversion, `seal_files.rs` the background one for files, `admin.rs` the
  operator's commands.

## 4. Upgrading a live server (E4)

Migration 0059, plus a Rust step that runs under the migration lock before the server answers anything:

1. **SQL**: `storage_keys` is created; content columns become `bytea`, existing text kept behind a `0x00` "not sealed
   yet" byte. Lookup columns, `is_assistant` and `attachments.sealed` are added; the triggers and indexes move to them.
2. **Rust**: a key is made for every family, group, direct chat, assistant chat and account, and every `0x00` value is
   sealed under its owner's key, 500 rows per transaction — **resumable**, so a crash or a restart continues where it
   stopped. Lookups are filled in.
3. **A CHECK on every sealed column** that its first byte is `0x01`. From then on PostgreSQL itself refuses anything
   that is not a sealed value.
4. **`VACUUM FULL`** on the converted tables, so the old row versions that held plaintext leave the table files.
5. The server starts answering. Files are sealed in the background, oldest first, each written beside the original and
   renamed over it, with progress in the log (counts, never names).

For one family this takes seconds for the database and minutes for the files. **There is no way back to v1.3 except
the pre-upgrade backup.**

## 5. Backups and restores

**A deleted family is unreadable in an old backup only if that backup does not hold its key.** If the keys travel with
the data, the backup kept for a year opens every family deleted during that year. So (E5):

- the nightly backup leaves the keys out of the main dump (`pg_dump --exclude-table-data=storage_keys`);
- the same run writes the keys to a **separate** place, kept **7 days**;
- seven days after a family, conversation or account is deleted, no backup can open it, even with the key file.

A restore needs the data **and** a key copy: the one with the dump's stamp, or, when that has expired, the newest.
Restoring data older than the oldest key copy means what was deleted since stays deleted: at start the server finds
rows whose key no longer exists, and refuses to start until the operator confirms that those rows may be deleted again
(`family-connect storage forget-missing`). A key copy OLDER than the dump leaves newer families without keys; the same
refusal names them, and the fix is a newer key copy. The restore steps are in `docs/operations.md`, "Restoring".

**What must never be lost**: the key file (copied off the server once, by hand) and the key copies (written nightly by
the backup). Without the key file nothing opens; without any key copy, a restore opens nothing.

A WAL archive or a replica, if the operator runs one, holds keys until it is pruned.

## 6. After the upgrade (operator)

- **Take a fresh backup, then delete the old ones.** They hold plaintext and always will.
- Free disk blocks keep old bytes until they are overwritten. That is why disk encryption underneath (the assessment's
  design 1) is still worth having.

## 7. Docs

- `docs/protocol.md`: the wire does not change. One paragraph saying content is sealed at rest, what that does not
  mean, and that deleting a family, a conversation or an account reaches the backups after the key copies expire.
- `docs/operations.md`: a new section, "Encryption at rest": making the key, keeping the off-server copy, systemd
  credentials, rotation, the upgrade steps above, checking the result (`psql` shows `\x01…`), what to do afterwards, and
  **what losing the key means**. "Backups" gains the separate key copy and its 7 days; "Restoring" gains the key copy,
  its age, and `forget-missing`.
- `server/ops/family-connect-backup.sh`: the main dump without keys, and the key copy to its own restic tag (or rsync
  target) with its own retention.

## 8. Tests

- The existing server tests keep their behaviour. The direct reads of content columns go through the vault.
- **Nothing readable at rest.** After a scenario that touches every feature, scan every column of every table and every
  file in `attachments_dir` for the scenario's marker text. None may contain it. The tables come from
  `information_schema`, so a future table is covered without editing the test.
- **Deletion reaches a backup.** Take a copy of the data, delete a family (or a group, an account), take a copy of the
  keys: the deleted family's values and files in the old data copy do not open; everyone else's do. A direct chat moved
  to another family, and a photo in it uploaded for the deleted family, still open.
- **Upgrade**: a database built at v1.3, with plaintext rows and files, converts and reads back the same through the
  API; an interrupted conversion resumes.
- **Refusals**: no key file, the wrong one, a tampered value, a truncated or reordered file, a row whose key is gone.
- **Rotation**: after `rotate`, everything opens with the new key file and nothing with the old.
- **Ranges**: across a chunk boundary, the first and last byte, inside the last chunk. Dedup still finds duplicates;
  signing in is still case-insensitive.
- `vault.rs` unit tests with fixed vectors, so the format cannot drift.

## 9. Order of work

1. `protocol.md` and `operations.md`. *(done)*
2. `vault.rs` and its unit tests. *(done)*
3. Migration 0059, the conversion step, the upgrade test. *(done — with 0060, the second half, held back until the
   conversion has finished; `tests/upgrade_flow.rs`)*
4. The query sites, table by table, with the suite green after each. *(done — in four groups: accounts; families and
   devices; what is said and the board; reports and attachments)*
5. Files, and the claim that moves a file to its conversation's key. *(done)*
6. Deletion: account keys, `forget-missing`, the backup script. *(done, with `storage-key new|show|rotate` and
   `reports`; `tests/storage_key_flow.rs`)*
7. The at-rest scan and the deletion test, clippy, fmt, and the full suite on PostgreSQL. *(done: the scan is in
   `tests/upgrade_flow.rs`, the deletion test in `tests/storage_key_flow.rs`)*

Only the server and its backup script change. The clients, the wire and the string tables do not.

## 10. Decisions

- **E1 — Files too?** DECIDED (a): files and previews are sealed as well.
- **E2 — Where the line goes.** DECIDED (a): everything personal, as in §3.
- **E3 — Keys.** DECIDED (b): a key per family — which after #88 means a key per family, per conversation and per
  account, wrapped by one server key (§3).
- **E4 — Converting the live server.** DECIDED (a): at start-up (§4).
- **E5 — How the keys are backed up.** DECIDED (a): left out of the main dump and copied separately, kept 7 days, so
  a deletion reaches every backup after a week (§5). Rejected: keys in the main dump, which would leave a deleted
  family readable for as long as backups are kept; and going back to one server key.
