# Family encryption on the server — what it would take (issue #59)

**Assessed 2026-09-13** against `v1.1`, in answer to issue #59: *"All information stored in Family
area should be encrypted and accessible only for family members."* This document says what the
request can mean, which facts were checked rather than assumed, what each meaning costs, what is
recommended, and what has to be decided first. **No code has been written for it.**

---

## Two requests in one sentence

- **"Stored … encrypted"** — encryption AT REST. It protects the family from somebody who gets a
  copy of the data without running the server: a stolen or recycled disk, a hosting provider's
  snapshot, a leaked backup or database dump. The server still holds the keys, so it can still do
  everything it does today.
- **"Accessible only for family members"** — taken literally, that excludes the person who RUNS the
  server, and the only design that does is END-TO-END encryption: the family's devices hold the keys
  and the server stores what it cannot read.

They are different projects of very different size. The first adds a layer and changes nothing on
the wire. The second changes what the server is able to do, and a large part of what this server
does is read what it stores.

## Checked facts

**What is stored today, and how**

- **Nothing in the server is encrypted at rest** — no `pgcrypto`, no cipher, nothing matching
  `encrypt` in `server/src`, `server/migrations` or `Cargo.toml`.
- **Attachments are plain files** under `/var/lib/family-connect/attachments`, sharded by key,
  deduplicated per family by a SHA-256 of the bytes. `GET /attachments/{id}` answers **byte
  ranges**, which is how a video seeks (`handlers_attachment.rs`).
- **The database** holds the rest in ordinary columns across 25 tables — messages, notes, polls,
  reactions, mentions, users, birthdays, reports, locations and the rest.
- **In transit**: nginx terminates TLS and the server listens on loopback. **Call media is already
  encrypted between the devices**; a TURN relay "carries the ENCRYPTED media and cannot read it".
- **Backups** (`server/ops/family-connect-backup.sh`) are a `pg_dump -Fc` plus the attachments
  directory, sent off the box through **restic** — whose repositories are always encrypted — or
  **rsync**, which copies them as they are. `docs/operations.md` suggests a separate volume for
  attachments and says nothing about encrypting one.

**What the server reads — all of it would stop working under end-to-end encryption**

- **The assistant**: `mentions.rs` decides from the BODY whether a family-chat message reaches the
  assistant at all, and the assistant's requests carry bodies, the family-chat history under
  `ai_history`, photographs under `ai_vision`, profile pictures under `ai_faces`, and the daily
  greeting.
- **Push**: the APNs alert and the FCM notification carry the message or note TEXT as their body
  unless `[push] include_message_body = false` (default `true`), and the title carries the family's
  and the sender's names either way. The server sends system-displayed notifications on both
  platforms, and the iOS project has **no notification service extension** (its targets: the app,
  two test bundles, the share extension), so no device could decrypt a push before showing it.
- **Excerpts the server cuts**: a reply's quote and a report's frozen excerpt.
- **Upload honesty**: the magic-number check reads the first bytes of what is uploaded.
- **Limits**: the 4000-character body ceiling.

**Membership and accounts — where keys would have to travel**

- Under join policy `open`, an invite code admits a new member **with no existing member's device
  involved at all**. Under `approval`, the owner approves through the API.
- **The owner can reset a member's password without knowing it**
  (`POST /families/members/{id}/password`), because the member has forgotten it.
- A deleted account's words **stay** in the family chat under "Deleted account".
- There is **no client-version gate** in the protocol. Compatibility rests on clients ignoring what
  they do not know — and an installed app handed a ciphertext `body` would show the ciphertext.

**The clients**

- Apple keeps its session in the Keychain (`KeychainStore.swift`); Android has a Keystore-backed
  `TokenStore`. **The web client keeps its token in `sessionStorage` on purpose** — closing the tab
  is a sign-out — and has no Web Crypto bindings enabled. The Windows core has no cryptography yet.
- The portfolio already has a precedent for byte-compatible cryptography across platforms: the
  Exchange apps' `EXC2` envelope, shared by iOS, macOS and Android.

**Who runs the server**

- The config's own words: "Family Connect is one family on a server of its own". But
  `[families] registration = true` is the default, so one server can host several families, and then
  its operator is not a member of most of them.

## What each design protects against

| Somebody who… | At rest, by the operator | At rest, in the application | End-to-end |
| --- | --- | --- | --- |
| steals or recycles the disk | protected | protected | protected |
| gets a backup | protected with restic; rsync only to an encrypted target | protected, if the key is not in the backup | protected |
| gets a database dump or a DB-level read | **not** protected | protected | protected |
| gets root on the running server | not protected | not protected | protected, except for new keys while they are compromised |
| runs the server | not protected | not protected | protected |
| **Costs** | a section in operations.md | every content column, every file, a key the operator must never lose | the assistant, readable pushes, owner resets, open joins, and new crypto in four clients |

## The three designs

### 1. At rest, by the operator — no code

An encrypted volume (LUKS) under PostgreSQL's data directory and under the attachments directory;
backups through restic, or rsync only to a target that is itself encrypted. A new section in
`docs/operations.md`, beside "Disk" and "Backups", with the commands and the one thing it does NOT
do: protect a server that is running.

### 2. At rest, in the application — no wire change

A data key per family, wrapped by a server key that lives in a file under `/etc/family-connect` —
**outside the database and outside the backup**, and backed up separately, because losing it loses
every family on the server.

- Content columns (bodies, note texts, poll and task texts, names, places, coordinates, birthdays)
  become ciphertext; ids, sequences and timestamps stay plain, because the server sorts and syncs on
  them.
- Files are encrypted in chunks (AEAD per chunk) so a byte range can still be served without
  decrypting the whole video.
- Dedup keys on an HMAC under the family's key instead of a bare SHA-256.
- Every existing row and file is converted by a migration that has to be resumable.
- Everything the server does today keeps working, because the server can decrypt.

### 3. End-to-end — the literal reading

Keys live on the family's devices; the server stores ciphertext and routes it. What it costs,
from the facts above:

- **The assistant cannot read an encrypted chat.** Asking it would mean the device sending that
  message in the clear, deliberately — the chat is then no longer end-to-end for that message.
- **Pushes say "New message"** until iOS gets a notification service extension and Android moves to
  data-only messages that the app decrypts itself. The names in the title still leave.
- **An owner's password reset cannot restore access** to history; another member's device has to
  re-share the family key the next time both are online.
- **An `open` family admits members nobody can hand a key to** — they see nothing until a member's
  device comes online.
- **Every departure rotates the key.**
- **The web client must keep a key beyond the tab**, reversing the reason its token lives in
  `sessionStorage`, or re-derive it from the password — which the owner's reset then breaks.
- **Installed apps must be kept out of encrypted chats** by a version gate the protocol does not
  have.
- **Losing every device loses the history**, by design.
- Magic-number checks, server-cut excerpts and the body-length limit move to the clients.

## Recommendation

1. **Now: design 1.** It costs a documentation section, protects against the most likely loss — a
   disk or a backup — and changes nothing for the family.
2. **Design 2 only if a leaked database dump, or a database read without root, is a threat worth a
   large migration.** It is honest encryption at rest, but it does not keep the operator out, and
   it must not be described as if it did.
3. **Design 3 only as a separate product decision, and not for the family chat while the assistant
   lives there.** If it is wanted, start with **direct chats**: the assistant never reads them
   already, they are the conversation the protocol treats as most private, and the design can be
   proven there before anything else pays its costs.

## What has to be decided

1. **Whom must it stop?** (a) somebody with a disk or a backup; (b) somebody with the database;
   (c) whoever runs the server. And does `fc.nettrash.me` host families other than yours?
2. **If (a)**: write the operations section now? Recommended: yes.
3. **If (b)**: accept that a lost server key file loses every family on the server?
4. **If (c)**: accept, for encrypted chats, no assistant, "New message" pushes until the two platform
   changes land, owner resets that cannot restore history, and a key kept in the browser?
5. **If (c)**: start with direct chats only? Recommended: yes.
6. **If (c)**: is losing the history when every device is lost acceptable?
