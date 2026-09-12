# Family Connect for Windows

The fourth client of the protocol in `docs/protocol.md`, alongside `ios/` (iOS + macOS),
`android/` and `web/`. Issue #64; the assessment that scoped it is
`docs/windows-client-2026-09-11.md`.

**Status: phase 1, the portable core and the app's logic.** There is no window yet. What exists is
the part of the client that has nothing to do with Windows — the wire, the local cache, the send
queue, the board's arithmetic, the reconnect resync, the live frame router and the session gate —
and it is deliberately the part that can be verified on the Mac this is developed on.

```
win/
  FamilyConnect.slnx
  src/FamilyConnect.Core/
    Protocol/   ApiError, ServerUrl, Dtos, Frames, ApiClient, ChatSocket, SendPipeline,
                SendRules, ReconnectBackoff, Resync, FrameRouter, ApiResult/ITokenStore
    Store/      Database + Migrations (numbered), ChatStore, BoardStore, OutboxStore, Times
    Board/      NoteText, NoteLook, BoardWall, BoardTasks, BoardPicture, NoteFitting, BoardBadge
    Text/       StringCatalog (the apps' English string IS the key)
  src/FamilyConnect.App.Logic/
                AppSession — which screen the app is on, and the three ways a session ends
                LiveConnection — the socket, the resync, the outbox and the router under one policy
                ChatList — the rows, their order, and the one line under each name
                Conversation — one open chat: the window, paging back, the read marker, typing
                Board — the wall: the stickers on it, the badge over it, the writes that change it
                MediaOutbox — the uploads a queued message owes, and the bytes waiting for them
                AttachmentCache — downloaded bytes, kept, with the preview rule in ONE place
  tests/FamilyConnect.Core.Tests/      xUnit, runs anywhere `dotnet` runs
  tests/FamilyConnect.App.Logic.Tests/ the same, for the app's own behaviour
  tools/board-oracle/                  Rust: regenerates the shared-arithmetic fixture
```

`Resync` is what happens on every (re)connect, in the protocol's own order, and two of its rules
only look small. **The flush is not a step**: the outbox is flushed FIRST and unconditionally,
because a client that could not even sign in must still flush what it holds. **The message cursor
belongs to the loop**: read once, then advanced by the largest id each page actually returned — one
live message landing mid-loop would otherwise jump it and skip everything in between, for good.

`LiveConnection` is the policy over all of it. The socket runs only while there is something to
listen to (signed in AND in a family); EVERY connection starts a resync, because what a client
missed while the socket was down is exactly what the pass reads; and passes DO NOT STACK — a
flapping proxy raising three connections in a second coalesces into one more pass, not three.
An expired session lands where a REST `401` lands, once.

`FrameRouter` is the only place a live frame meets the cache. A `read` frame naming YOURSELF is
your own marker from your other device; one naming somebody else is roster data, drawn as "seen" in
a direct chat and nowhere else. An edit is not a message: it raises no notification, bumps no
count, carries the WHOLE message, and moves the edit cursor nothing else may move.

**The unread count is INCREMENTED by a live frame and never recomputed.** Local history is not the
whole of it — retention swept some, paging never fetched the rest — so a recount is a badge that
silently falls to one the moment anything arrives. A page may not raise it either: the list read
that preceded the page already counted every message on it. A list read takes the server's number
PLUS whatever raced it (anything held above the preview that answer carried), and reading
subtracts rather than recounts.

**The board has three cursors' worth of rules and they are all in `BoardStore`.** A full read
REPLACES the wall — except a note held above the read's own `max_board_seq`, which arrived after
the read was taken; an older full read landing second is ignored outright. A tombstone is
REMEMBERED (the `gone` table), so an older copy of a deleted note cannot put it back — the web
client keeps the same set, and Apple and Android do not, which is a known gap there. And the
cursor moves in three ways and no others: a full read sets it to its mark, a catch-up page to the
page's highest seq, and a frame to its own — the frame only once this device has read the board at
all, because a cursor of 0 is what asks for the whole wall and a frame that jumped that queue
would leave the wall to whatever frames happened to arrive.

The three catch-up cursors are **this device's**, and a `GET /chats` row's `max_*_seq` is the
**server's** — same names, different numbers. Storing one as the other makes the gate "the
server's mark exceeds what I have applied" false for ever and silently stops every catch-up; this
port did exactly that until a resync test asked for a page and got none.

**A media send is represented before the first byte moves.** The row, the files it owes and the
files it CAME FROM are written down first — `staged_files` never shrinks, because
`attachment_expired` means *upload it again* and a row that had forgotten where the bytes came
from could only give up. Every landing is remembered as it happens, so a crash halfway through a
four-photo message costs the remaining three; a file this device can no longer find fails the row
outright (an id cannot recover a picture); and a flush PUSHES what is owed before it posts
anything, because the pipeline passes over a row that still owes a byte.

**A preview is only asked for when the attachment says it has one.** The server generates none for
a picture the assistant drew, and none at all for a file, audio or a location — so `has_preview` is
a fact, not a hint. Asking anyway answers 404, and a client that reads that as "no picture" draws
an empty frame for ever (the Android board's backdrops, issue #71's follow-up). The rule lives in
`AttachmentCache` rather than at every call site, where one of them will always forget.

The send path is whole: `SendPipeline` writes the row down, tries the socket, gives the frame the
ack deadline, falls back to `POST /chats/{id}/messages` with the same `client_msg_id`, and marks a
row failed only on a terminal code. Both the deadline and the clock are injectable, because a suite
that sleeps ten seconds per silent socket is a suite nobody runs.

What is NOT here yet: the WinUI app, the credential store (`ITokenStore` is the seam; on Windows it
belongs in the locker), the uploads a media send owes, and calls.

The cache is SQLite with numbered migrations and no destructive fallback. `DatabaseTests` compares
a database that walked every step against one created fresh, table by table and column by column —
which is the only thing that catches a column added without a migration. Android shipped exactly
that once (issue #70) and every upgraded install would have crashed on the next launch.

## Build and test — on any OS

```bash
cd win
dotnet build FamilyConnect.slnx
dotnet test FamilyConnect.slnx
```

.NET 10 (`global.json` pins the SDK band). Nothing here references a Windows API, on purpose: a
WinUI **app** cannot be built on macOS at all — its XAML compiler is .NET Framework — so a port
whose logic could only be tested on Windows would be a port nobody here could verify. When the app
project arrives it is the only one that needs Windows, and md.win's `tools/xamlcheck` trick
type-checks its code-behind here even then.

## The oracle

There are TWO oracle fixtures, both generated: `board-vectors.json` (the wall's arithmetic) and
`chat-vectors.json` (the words a chat row is drawn with — a call record's line, an attachment's
name, the notification sentences). `tests/.../Fixtures/board-vectors.json` is not hand-written: every vector in it was produced by
`fc_text::board` — the Rust the web client runs — through `tools/board-oracle`, which depends on
`web/text` by path so it cannot drift. `OracleTests` holds this port to it, number for number.

```bash
cd win/tools/board-oracle
cargo run --quiet > ../../tests/FamilyConnect.Core.Tests/Fixtures/board-vectors.json
cargo run --quiet -- chat > ../../tests/FamilyConnect.Core.Tests/Fixtures/chat-vectors.json
```

Four implementations of one rule need an oracle, not four readings. This repo has been bitten by a
byte-versus-character split that panicked on Cyrillic, and the portfolio by four ports that agreed
with each other and were all wrong about `pow(10, n)`.

## What is decided, and what is not

Decided here, because the code needed an answer: WinUI 3 + C#/.NET 10 (md.win's stack), the
project layout above, `Microsoft.Data.Sqlite` with numbered migrations when the cache lands, and
one Windows face per note "hand" — Segoe UI, Georgia, Cascadia Mono, Segoe Print — all in-box, so
nothing is bundled and nothing is synthesised.

Still nettrash's to answer (see the issue): calls in 1.0 (SIPSorcery, WebView2-hosted, or not
yet), push in 1.0 (WNS, or socket-only toasts as the web client does), the minimum Windows build,
and whether it ships through the Store or as a signed installer.
