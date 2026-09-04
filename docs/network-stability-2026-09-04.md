# Network stability — why a send fails on a bad link

**Audited 2026-09-04** against `master` @ `ea7eee6`, in answer to the field report *"sometimes we
can't send messages"*. Produced by a 48-agent workflow: six parallel readers (the protocol, the iOS
text pipeline, the iOS media pipeline, Android, the server + nginx deployment, and a cross-port
differential), then an adversarial verifier against every finding above low severity. 42 raw
findings → 31 confirmed, 11 refuted. The refutations mattered: they moved the diagnosis, and what
they killed is written down in "What was checked and dismissed" so nobody re-derives it.

The audit changed no code. What has been implemented since is in the status section directly
below; the rest of the document is the audit as it stood.

---

## Status — tier 1 is implemented

Everything in "Tier 1" below is written, plus one item from tier 2 (the nginx throttle response,
because the clients' new 429 handling is only useful if the proxy sends a `Retry-After`). The
findings text is kept as the record of the audit; this section is what has changed since.

**The protocol** gained a transient-versus-terminal rule in "Error shape", the code
`too_many_requests`, and a new section, "Sending on an unreliable network", which states the ack
deadline, the terminal codes, the backoff obligation and the fact that a returning network is a
trigger. "Best-effort delivery" now says the outbox flush is not a step of the read pipeline.

**Both apps** now keep a message queued instead of turning it red when the outcome is unknown. Each
row carries `sendAttempts` and `nextAttemptAt`, retries with full-jitter backoff up to six times,
honours the server's `Retry-After`, and only shows a red bubble on a terminal refusal or after the
budget is spent. Tapping retry resets the budget. The outbox flush runs FIRST in the resync rather
than last, plus on a wake timer, on the app coming to the foreground, and on the network returning.
iOS gained the connectivity observer it never had, which also cuts the reconnect backoff short.

**Android** lost its three divergences: a stale attempt can no longer paint a delivered message red
(the write is conditional in SQL), a transient error frame now falls to REST instead of cancelling
its own rescue, and a failed multi-photo send restores the whole set rather than the unsent tail.
Its ack deadline is 10 seconds like iOS, the shared client has a 20 second end-to-end ceiling, and
the hard 10 minute upload ceiling is gone so a large video finishes if it is making progress.

**nginx** answers a throttle in the protocol's error shape with `Retry-After: 2`.

## Status — tiers 2 and 3 are implemented too

**A media send is now durable.** The message row, the rows for the attachments it still owes, and
the bytes themselves are written before the first upload starts, so an interrupted send is a bubble
that can be finished rather than nothing at all. The bytes move out of the directories the system
may reclaim, into Application Support on iOS and `filesDir` on Android, and they are deleted only
when the message is acked or the person deletes the bubble. Because a queued media send is now an
ordinary pending message row, it inherits every outbox trigger tier 1 added: the socket connecting,
the app coming forward, the network returning, the wake timer.

**A retry pushes only what is still owed.** Every attachment id that landed is remembered, so a
five-photo set interrupted on item four uploads one photo on the retry rather than all five. On a
link that only stays up for half a minute at a time, that is the difference between a set that
eventually sends and one that never can.

**An expired upload is uploaded again rather than failed.** The server keeps a marker for 30 days
after its unclaimed sweep removes an upload, so a client coming back with that id is told
`attachment_expired` rather than `attachment_not_found` — and, still holding the bytes, sends them
again. Before this the two were indistinguishable and a send that sat out a long offline stretch
failed permanently.

**A row that still owes uploads can never be posted.** That guard is in the delivery path itself on
both platforms, not at its call sites, because a message claiming no attachments is a text message,
and for a photo with a caption the server would accept it happily — a green bubble with the
pictures gone and nothing saying so.

Two things this change also fixed, both found by review rather than by the plan: staging must COPY
a file the sender owns rather than move it, because a video that already fits the ceiling is handed
over as the person's own file, and the orphan sweep needs an age floor, because a send stages its
bytes before it writes its rows and a sweep landing between the two would delete the message
somebody had just sent.

Still open: finding 10, the missing background lifeline on a text send, and the composer's own
`tmp` litter, which is still ephemeral by design.

---

## The short answer

**A send that fails once is failed forever.** Both apps write a permanent `failed` / `FAILED` row
the first time anything goes wrong — a timeout, a lost connection, an nginx 429, a transient 500 —
and then never retry it. The outbox sweeps on both ports look only at *pending* rows
(`ChatSyncCoordinator.swift:2319-2323`, `MessageDao.kt:293-295`), so a failed message is delivered
only if the person scrolls back into that exact conversation and taps the red bubble. Composing in
a dead spot does not park a message for later either: the send is attempted at once, fails on both
legs, and goes red immediately. That is the whole of the user's complaint. The network came back
and the app did not care.

**A media send that is interrupted is gone, and says nothing.** On both ports the message row is
created only after every attachment has finished uploading, so until then the send exists solely in
a running task (`ChatSyncCoordinator.swift:1636-1703`, `MessageRepository.kt:349-434`). Suspend the
app, lose the process, or let the upload allowance expire and there is no bubble, no error and
nothing to retry. For an in-app camera capture or a voice note the content itself is destroyed —
the staging file in `tmp` was the only copy, and the next launch sweeps it.

**Nothing re-drives the outbox on the two events that should.** iOS has no connectivity observer at
all (no `NWPathMonitor`, no `waitsForConnectivity`), so "the signal came back" triggers nothing and
the reconnect loop keeps sleeping up to its 30 s ceiling. Android re-sends only from
`SyncEngine.resync()`, which runs only when a WebSocket upgrade succeeds
(`ChatSocketManager.kt:134`), so on a network that allows HTTP but blocks the upgrade nothing is
ever flushed.

---

## What is already right

Worth stating, because the fixes must not break it.

- **Re-sending is safe for every kind of message.** The `(chat_id, sender_id, client_msg_id)`
  conflict is resolved before attachments are claimed and before the assistant is triggered, and a
  repeat returns the original message with `200` instead of `201`. Text, replies, polls,
  attachments, `@ai` and calls all dedup on the same key. There is no need for a new "did this
  land?" endpoint: re-sending *is* that query.
- **The socket-versus-REST race is sound.** An unanswered socket send falls back to REST with the
  same id, and iOS guards the row state on both legs (`ChatSyncCoordinator.swift:2029`, `:2047`)
  so a late ack cannot flip a delivered message to failed.
- **The reconnect backoff is full-jitter**, which is the right shape for a household whose devices
  all lose the same server at the same instant.
- **WebSocket frames cost nothing against the nginx rate limit** — one request per connection, not
  per frame — so an open socket is not a throttling risk.

---

## Findings, ranked by what they cost a user

### 1. Every transient failure is recorded as permanent, and permanent rows are never retried

Confirmed by four independent readers. On iOS a catch-all `catch` writes `.failed` for every
`URLError` (`ChatSyncCoordinator.swift:2044-2047`); on Android the `else` branch of `restFallback`
covers `NetworkError` and *every* `HttpError`, 429 and 502 included
(`MessageRepository.kt:601-607`). Both sweeps then exclude those rows by predicate. The protocol
gives no help: `docs/protocol.md:38-56` is a flat list of codes with no transient-versus-terminal
classification and no client retry obligation.

*The user sees:* a red bubble produced by a network hiccup, still red an hour later on perfect
Wi-Fi, needing a tap the chat list never advertises. People who assume it failed and retype end up
sending twice.

*Fix:* classify the outcome before deciding the row's fate. Transport errors, 408, 429 and 5xx keep
the row pending and belong to the outbox; only a 4xx the server explained (`validation`, `blocked`,
`not_chat_member`, `message_too_long`, `invalid_poll`) is a red bubble. Add `attempts` and
`nextAttemptAt` to the row, retry with the backoff shape that already exists, and give up to a red
bubble after a bounded number of tries. One paragraph of protocol, one migration per port.

### 2. A media send is not durable anywhere

iOS documents the gap in its own header (`MediaOutbox.swift:35-40`: *"WHAT THIS DELIBERATELY DOES
NOT DO: survive a process kill"*), and Android has no `WorkManager`, no `JobScheduler` and stages
files in `cacheDir`, which the OS may evict. Three consequences, all confirmed:

- **Silent total loss** on suspension, jetsam or process death. No row, no bubble, no notice.
- **No partial progress.** One error anywhere in a five-photo set discards every upload already
  made; each retry re-uploads the whole payload, so a link that stays up for 40 s at a time can
  never finish a large set. Android is worse than iOS here: it deletes each source file as that
  item uploads (`MessageRepository.kt:394`), so the composer gets back only the unsent tail and the
  user re-sends **fewer photos than they picked**, with nothing naming the ones that vanished.
- **Uploads that outlive their message.** If the attachments land and the claim POST fails, the
  failed row is never auto-retried, and the server's 24-hour unclaimed sweep eventually deletes the
  bytes — leaving a bubble that still draws its photos and can never be sent.

*Fix, in cost order:* (a) let the sweep pick up failed rows that carry attachment ids, which alone
shrinks the exposure from "until somebody notices" to "the next reconnect"; (b) remember which
items already uploaded so a retry pushes only the remainder — the ids stay valid for 24 hours,
which is exactly the window needed; (c) the real fix, persist the send before the first byte —
a SwiftData `PendingMediaEntity` with staging moved out of `tmp` into Application Support, and a
Room outbox row plus a `CoroutineWorker` with `filesDir` staging on Android. That is the only thing
that closes silent loss.

### 3. The outbox flush is buried at the end of a read pipeline, with too few triggers

`sweepOutbox()` has exactly one call site (`ChatSyncCoordinator.swift:2249`) and `flushPending()`
exactly one (`SyncEngine.kt:120`), both as the last step of the resync — behind `GET /me` on both
ports and behind `GET /chats` on iOS. A verifier correctly narrowed this: iOS resyncs from five
places, and Android's `/chats` failure degrades rather than returning, so this is not the routine
killer the first reading claimed. It is still the wrong shape, and it leaves two real holes: iOS
never reacts to the network returning, and Android never flushes unless a WebSocket upgrade
succeeds.

*Fix:* make the flush independent of the reads — run it first, not last, and drive it from
`NWPathMonitor` / `ConnectivityObserver.onAvailable`, from the foreground edge, and from a timer
while anything is outstanding. Add one protocol sentence: the outbox flush is not a step of the
read pipeline, and a client that could not complete the reads must still flush. The flush is
already idempotent, so running it earlier and more often costs nothing but requests.

### 4. Android turns delivered messages red, and its Retry button then does nothing

`restFallback` re-reads the row *before* the POST but never after, so a stale attempt whose ack
arrived meanwhile writes `FAILED` over a delivered message; `retry()` returns early once `serverId`
is set, so the button is a no-op. Separately, `onSendError` cancels the pending-ack job
(`MessageRepository.kt:884-888`), which is the coroutine that would have run the REST rescue — so a
transient error frame kills its own fallback. iOS gets both of these right; this is a pure
divergence.

*Fix:* re-read the row after the POST and write `FAILED` only when `serverId` is still null; in
`onSendError`, mark failed only for terminal codes and run the fallback immediately for transient
ones. Roughly ten lines.

### 5. The two ports disagree about every timeout that decides how long "Sending…" lasts

| | iOS/macOS | Android |
|---|---|---|
| Ack deadline | 10 s | 15 s |
| REST send timeout | 15 s | none (`connect 10 s` + `read 30 s`, no `callTimeout`) |
| Worst-case time to a verdict | ~25 s | ~85 s |
| Socket write deadline | none (75 s pong horizon) | — |
| Upload ceiling | none (per-operation) | hard 10-minute `callTimeout` |

The protocol names none of these numbers. The Android upload ceiling is a hard wall: a 100 MB video
on a slow uplink cannot finish, however patient the user is, and there is no resumption anywhere.

*Fix:* pick one pair and write it on both ports — 10 s ack, ~20 s end-to-end send budget; drop
`callTimeout` from the upload client and keep the per-operation timeouts; give the iOS socket write
a deadline so a stalled write falls to REST instead of waiting for the pong watchdog. State the ack
deadline in the protocol as a recommendation so the ports cannot drift again.

### 6. Protocol gaps worth closing while the clients are being changed

- **No ack deadline and no meaning for an unanswered `send` frame.** Both ports invented their own.
- **No transient/terminal classification of error codes** (the root of finding 1).
- **429 is not in the protocol at all**, yet nginx returns it for both of its limits, in an HTML
  body the error shape does not cover and with no `Retry-After`. The specific fear — that a
  household resync storm throttles someone's send — was *refuted*, because sends go over the open
  socket during a resync and frames do not touch the bucket. But the handling is still wrong
  wherever a 429 does arrive. Document `too_many_requests`, say it may arrive without the JSON
  shape, emit `Retry-After` from nginx, and honour it on POSTs too.
- **No resumable or idempotent upload.** `POST /attachments` already creates the row before the
  bytes arrive, so an id could be returned first and the bytes sent to a `PATCH` with
  `Content-Range` appending to the existing part file. This is what makes large media possible on
  mobile data.
- **The 24-hour unclaimed grace is a hard expiry** with no way to renew and no distinct code, so a
  client cannot tell "your upload expired" from "that id was never yours". Add
  `attachment_expired`, or measure the grace from last touch.

---

## What was checked and dismissed

Eleven claims were refuted on the evidence. The ones worth remembering:

- **A household resync storm does not throttle sends.** During a resync the socket is open, sends go
  over it, and WebSocket frames cost the rate-limit bucket nothing. A quiet iOS resync is about four
  HTTP calls, requests are strictly sequential, and the avatar fetches a reader called "unbounded"
  are in fact the dedup guards.
- **`limit_conn 24` is not reached** by long-lived sockets in the described scenarios; the
  arithmetic that got there was misread twice.
- **A half-open socket does not block `task.send`** on iOS, so the heartbeat is not deadlocked
  behind the connection it is testing.
- **The push fan-out is not awaited on the message write path**, so a slow APNs/FCM cannot stall a
  send.
- **Deleting a red bubble that actually landed does not hide it forever** — the catch-up cursor is
  the max over rows that have a server id.
- **The 64-frame `DROP_OLDEST` flow on Android is per-collector**, so a slow collector cannot make
  another one lose an ack.

---

## Suggested order of work

**Tier 1 — small, no wire change, fixes the reported complaint.**
1. Transient-versus-terminal classification on both ports, with bounded automatic retry (finding 1).
2. Android: re-read before writing `FAILED`; stop cancelling the REST fallback (finding 4).
3. Flush the outbox first rather than last, and on network-restored and foreground; add
   `NWPathMonitor` to iOS; give Android a resync trigger that does not require the socket
   (finding 3).
4. Align the timeouts; drop Android's upload `callTimeout` (finding 5).

**Tier 2 — protocol and deployment, still small.**
5. Amend `docs/protocol.md`: ack deadline, transient codes, the outbox obligation, `429` +
   `Retry-After`, `attachment_expired`.
6. nginx: return the protocol's error shape and a `Retry-After` on 429.
7. Let the outbox sweep adopt failed rows that carry attachment ids, and remember uploaded ids
   across retries (finding 2a, 2b).

**Tier 3 — the real project, one release of its own.**
8. Persist media sends before the first byte on both ports, with staging outside eviction-prone
   directories and a background transfer (finding 2c).
9. Resumable uploads, which is what finally makes a 100 MB video sendable from a phone on mobile
   data.

Tier 1 alone should end the "sometimes I can't send" reports. Tier 3 is what ends silent loss.
