# Family Connect for Windows

The fourth client of the protocol in `docs/protocol.md`, alongside `ios/` (iOS + macOS),
`android/` and `web/`. Issue #64; the assessment that scoped it is
`docs/windows-client-2026-09-11.md`.

**Status: the whole client, run on Windows.** The core and the logic are the part of the client that
has nothing to do with Windows — the wire, the local cache, the send queue, the board's arithmetic,
the reconnect resync, the live frame router and the session gate — and they are tested wherever
`dotnet` runs (419 + 366 tests). `FamilyConnect.App` is the WinUI 3 window over them, and it carries
what the Mac and the web carry: the chats with threads, polls, reactions, edits, mentions, the
assistant, link previews and every attachment kind; the board; the family and its owner's console;
settings; one-to-one voice and video calls; notifications; the notification area; and files shared in
from other apps. It is run and checked on an ARM64 Windows 11 machine, type-checked on the Mac by
`tools/xamlcheck`, and built into an MSIX by CI. What it still does not do is listed, with the reason,
under "What is NOT here".

```
win/
  FamilyConnect.slnx                   everything, app included — open it in Visual Studio on Windows;
                                       off Windows build the projects, not the solution (see below)
  src/FamilyConnect.Core/
    Protocol/   ApiError, ServerUrl, Dtos, Frames, ApiClient, ChatSocket, SendPipeline,
                SendRules, ReconnectBackoff, Resync, FrameRouter, ApiResult/ITokenStore
    Store/      Database + Migrations (numbered), ChatStore, BoardStore, OutboxStore, Times
    Board/      NoteText, NoteLook, BoardWall, BoardTasks, BoardPicture, NoteFitting, BoardBadge
    Text/       StringCatalog (the apps' English string IS the key), CallRecordText,
                AttachmentText, NotifyText, Calendar (the .ics a client writes itself)
  src/FamilyConnect.App.Logic/
                AppSession — which screen the app is on, and the three ways a session ends
                LiveConnection — the socket, the resync, the outbox and the router under one policy
                ChatList — the rows, their order, and the one line under each name
                Conversation — one open chat: the window, paging back, the read marker, typing
                Board — the wall: the stickers on it, the badge over it, the writes that change it
                MediaOutbox — the uploads a queued message owes, and the bytes waiting for them
                AttachmentCache — downloaded bytes, kept, with the preview rule in ONE place
                Family — the door, the owner's console, and the numbers everybody may see
                Notifications — when this client speaks up, and what it says when it does
                Avatars — what a picture must be before it is sent, and a cache keyed by VERSION
  src/FamilyConnect.App/               the WinUI 3 window: structure + code-behind, no decisions
                Services/ Connection (one server, wired), LockerTokenStore (the credential
                          locker), AppServices, AppFolders, the settings files, Toasts and
                          Attention, TrayIcon, ShareInbox, WindowPlacement, WebViewCallMedia,
                          VoiceRecorder, MediaPreparing, LocationFinder
                Views/    ServerView, SignInView, DoorView, PendingView, OfflineView, ChatsView,
                          BoardView + NoteSheet, FamilyView, SettingsView, CallCardView, and the
                          sheets and cards they open (polls, emoji, dialogs)
  i18n/                                generate.py + win.json (the port's own strings)
  tests/FamilyConnect.Core.Tests/      xUnit, runs anywhere `dotnet` runs
  tests/FamilyConnect.App.Logic.Tests/ the same, for the app's own behaviour
  tools/board-oracle/                  Rust: regenerates the shared-arithmetic fixture
  tools/xamlcheck/                     md.win's off-Windows check of the app's XAML and code-behind
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
REMEMBERED (the `gone` table), so an older copy of a deleted note cannot put it back — the web,
Apple and Android clients keep the same set. And the
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

**The door takes the LOWER of two numbers.** A family's own `max_members` and the operator's
ceiling are different questions with different answers — a family that set 40 under a ceiling of 50
goes on reporting 40 after the operator drops it to 10, because a stored cap is never re-validated
when the ceiling moves. `Seats` keeps both and answers what is actually left. And the assistant's
two dependent switches (`ai_history_photos`, `ai_faces`) are never offered, never sent and never
drawn as on while `ai_vision` is off — the server refuses that combination and clears them in the
same write.

**The statistics' rows do not add up to the totals, and the gap is the block.** The totals are the
family's numbers; the rows are what this caller may see of them. Nothing here sums the rows.

**NO PUSH, AND THAT IS THE PROTOCOL'S ANSWER RATHER THAN A GAP HERE.** `POST /devices` takes
`ios`, `macos` or `android` — a Windows client has no token any of those name, so it registers
nothing and is in the browser's position: **local toasts from the socket, and nothing on a lock
screen**. Giving Windows real push means adding a platform to `docs/protocol.md` AND a WNS sender
to the server; until that is decided, `NotificationRules` is the whole of it.

**SO CLOSING THE WINDOW DOES NOT QUIT.** With no push, a client that stopped listening when its
window closed would miss the very call it exists to ring for. The window hides instead — as a Mac app
stays in the Dock — and an icon in the notification area opens it again or quits (`TrayIcon`, on a
hidden window of its own rather than WinUI's, re-added when Explorer restarts). "Keep running when the
window is closed" in Settings turns that off, and without an icon to come back from a close is always
a quit.

**A profile picture is cached by (user, VERSION).** No frame carries a picture — only the number —
so a cache keyed on the user alone shows a face the family replaced weeks ago. It is the board
backdrop's mistake in another place. "No picture" is an answer and is kept for that version; a
transient failure is not.

**The `.ics` a client writes itself is pinned byte for byte.** Nothing about it is on the wire,
which is exactly why four clients writing it four ways would diverge in silence — and its three
rules are invisible until a calendar refuses the file: CRLF line endings, escaping the format's own
separators, and folding at 75 OCTETS **per code point** (the oracle caught a first draft that
folded per GRAPHEME and so broke a ZWJ emoji one byte later than the others).

**THE NINE LANGUAGES ARE JSON, NOT `.resw`, AND THAT IS A DECISION.** The key IS the English
sentence — the apps' `Localizable.xcstrings` is the source of truth for all four clients — and a
`.resw` name cannot BE an English sentence: `%@`, `·`, an em dash and a full stop are variously
invalid or meaningful in a resource name, and WinUI mangles what it accepts. A `.resw` would
therefore need a second table mapping slugs to sentences, which is a second source of truth and
the thing this design exists to avoid. So `win/i18n/generate.py` writes eight JSON tables (English
needs none: its keys are its values) which ship embedded in `FamilyConnect.Core`, and the app's own
XAML chrome may still use `.resw` for labels nobody else has to agree with.

A sentence nobody has translated yet reads in ENGLISH rather than as a slug, and the five that are
English for now are NAMED in `win/i18n/win.json` — a test fails on any other untranslated key, so
a new string cannot go quietly missing in nine catalogues that look complete. Completeness is asked
of the TABLE and not of the answer, because "Video" in German and "Photo" in French are real
translations identical to the English.

**A family's language is NOT the display language.** It is what the assistant answers in; the
window draws in whatever the device is set to, because a family setting that silently re-languaged
somebody's computer would be a surprise nobody asked for.

**A notification is never the message.** The body is the one a server with
`include_message_body = false` would send, and the block reaches one step further than the sender:
the assistant's answer to a blocked member's question raises nothing either, because it would light
up a notification for a thread its reader cannot read.

The send path is whole: `SendPipeline` writes the row down, tries the socket, gives the frame the
ack deadline, falls back to `POST /chats/{id}/messages` with the same `client_msg_id`, and marks a
row failed only on a terminal code. Both the deadline and the clock are injectable, because a suite
that sleeps ten seconds per silent socket is a suite nobody runs.

The conversation draws replies, reactions, edits, who is typing, and what is still on its way:

- **A reaction tap is decided on this side.** The server's set is a state-set, so the emoji the
  reader already holds means DELETE and anything else PUT, and a CHIP only ever joins — on the
  reader's own chip it shows who reacted, where their row is the remove control. `Reactions` is a
  port of `fc_text::reactions` and `fc_text::emoji`, held to it by `ChatOracleTests` (chips, who
  reacted with blocked members left out of the names but not the count, the toggle, the capsule).
  The server's answer is applied as EVIDENCE: it may not move the chat's catch-up cursor.
- **A reply quote is cut per scalar** (`Excerpt`, pinned to `fc_text::excerpt`), and a reply to a
  blocked member is "Replying to a hidden message".
- **An edit is trimmed, an unchanged body sends nothing, and an empty one is refused here.** Only the
  reader's own words may be edited — never a call record's placeholder or a poll's question.
- **Typing stands for five seconds**, ignores the reader and blocked members, orders by member so the
  line does not reshuffle, and ends when that person's message arrives.
- **A message not yet landed is drawn under the newest one**: "Sending…", or — refused — Try Again
  and Delete.

**Notifications come from the live socket, and only from live frames.** A message or a note that
says something new raises one while the window is not in front (`NotificationRules` decides; the
body is never the message), a second one about the same chat REPLACES the first (its tag is
`NotificationRules.ChatTag`), and opening the chat takes them away. A resync after a night asleep
raises nothing — it moves the unread count, which is in the window's title and on the taskbar icon.
Clicking one opens its chat, including the click that LAUNCHED the app: the manifest declares the
notification COM activator (its CLSID must never change), `Program.Main` subscribes and registers
before anything reads the activation, and `ToastActivation` holds a click until the window exists.
What a click carries is untrusted input (`ToastArguments.Parse`: anything but a positive id opens
nothing).

**Attachments are drawn and handed over, not yet sent.** A photo or a video is a tile at its own
shape from METADATA (`MediaText.TileSize`, so a row never changes height when its picture lands);
several are a grid of four with the rest counted. A tile draws the preview when there is one, a
photo's own bytes when there is not, and for a video with no poster nothing at all
(`AttachmentFiles.SourceFor` — a tile never downloads a whole video to draw itself). A photo opens
whole with Save…; a video or a recording opens in whatever plays it on this machine; a file row
saves through the save picker; a place opens in Maps. Bytes are fetched once by `AttachmentCache`
into `FileBlobStore` (one file per key, written whole or not at all, wiped with the SQLite cache
when the server changes). The measuring — sizes in the reader's decimal format, shapes, the
location line with a POINT, the Maps link — is `fc_text::media` ported and pinned by the oracle.
File sizes, "Zero KB" and "%lld byte(s)" are English for now: the Apple apps use the system's byte
formatter, so the shared catalogue has no such sentences, and this port's catalogue has no plural
forms (two keys stand in for English's one and other).

**Sharing INTO the app** is a share target in the manifest. What was shared is copied into the app's
inbox by the process Windows launched for it, BEFORE that process hands its activation to the running
window and exits — the share belongs to it — and only a copy whose marker was written last is taken
(`ShareInbox`). The window asks "Send to" (the family first, never the assistant's chat) and stages the
files in that chat's composer; nothing is sent until the reader presses Send.

What is NOT here, and why:

- **Push.** A closed app hears nothing (see "No push" above); the notification area is the answer
  until a `windows` platform and a WNS sender exist.
- **A map in a location message.** The Apple apps draw one with MapKit. Windows has no map of its
  own, and drawing one means sending the coordinate a family member shared to a third party (Azure
  Maps, or OpenStreetMap's tile servers). The web client makes the same choice this port does: the
  pin, the name, and a link that hands the place to a map the reader opens. Nothing leaves the box.
- **A chat or a call in a window of its own.** The Mac can; here a call is the card in the corner.
- **Upload progress as a number.** No client has it; a sending bubble says "Sending…".

**One cache, one connection, one operation at a time.** The socket applies frames on its own
thread, the resync applies pages on the thread pool, and the window reads on the UI thread — all
through the connection `Database` owns, which is not safe to share without a rule. So every store
operation takes `Database.Hold()` for its whole length: per OPERATION and not per command, because
a transaction spans several commands and a read slipped between two of them is exactly the failure
("Execute requires the command to have a transaction object…"). The lock is re-entrant, and no
store method awaits or raises an event while holding it. `CacheConcurrencyTests` holds the cache
and insists every named operation WAITS — deterministically, so a method that forgot the lock
fails every run — and a guard fails when a store grows an operation the list does not name. The
first version of that test was a timed stress run, and CI failed it on both runners before the lock
existed; that is how this was found.

The cache is SQLite with numbered migrations and no destructive fallback. `DatabaseTests` compares
a database that walked every step against one created fresh, table by table and column by column —
which is the only thing that catches a column added without a migration. Android shipped exactly
that once (issue #70) and every upgraded install would have crashed on the next launch.

## Build and test — on any OS

The solution holds the WinUI app, and the app's XAML compiler runs only on Windows — so off Windows
(the Mac, the ubuntu CI runner) name the test projects, which build Core and App.Logic with them:

```bash
cd win
dotnet test tests/FamilyConnect.Core.Tests
dotnet test tests/FamilyConnect.App.Logic.Tests
python3 i18n/generate.py ..          # rewrite the nine catalogues from the apps' own
python3 i18n/generate.py .. --check  # …or just say whether they are out of date
```

**Under somebody else's language.** Everything machine-facing is invariant and everything shown is
the reader's, and the way to know is to run it as they would:

```bash
for loc in de_DE.UTF-8 tr_TR.ISO8859-9 ru_RU.UTF-8 fi_FI.UTF-8 ja_JP.UTF-8; do
  for suite in tests/FamilyConnect.Core.Tests tests/FamilyConnect.App.Logic.Tests; do
    LC_ALL=$loc DOTNET_CLI_UI_LANGUAGE=en dotnet test "$suite" --nologo -v q
  done
done
```

`CultureTests` sets the culture itself as well, so the coverage cannot be lost by launching the
suite a different way — but the sweep is what found the one that mattered: **`fi-FI` spells the
time separator `.`**, so `ToString("yyyy-MM-ddTHH:mm:ss.fffZ")` on a Finnish machine writes
`2026-12-24T16.00.00.000Z` — an instant no server can read, from a client that looked correct in
German, Turkish, Russian, Japanese and Swedish. The invariant culture in `Times.Rfc3339` is
load-bearing, and now a test says so.

CI runs this lane on **ubuntu AND windows** (`.github/workflows/ci.yml`, job `win`): the portable
half is worth nothing unless something actually runs it on the target, and what that catches is not
compile errors but SQLite's locking, path separators, line endings in the fixtures and the reader's
own culture. The same lane re-prints both oracle fixtures from `web/text` and fails on a diff.

.NET 10 (`global.json` pins the SDK band). Nothing here references a Windows API, on purpose: a
WinUI **app** cannot be built on macOS at all — its XAML compiler is .NET Framework — so a port
whose logic could only be tested on Windows would be a port nobody here could verify.

## The window

`FamilyConnect.App` is the only project that needs Windows, and it holds no decisions: which screen
shows is `AppSession`'s gate, what a refusal says is `DoorSentences`, what a row and a bubble say
is `ChatListModel` and `BubbleText` — all in App.Logic, all tested. What is left in the window is
structure and wiring, and **every visible word is set from code through the catalogue** (no
`{Binding}`, no `x:Bind`, no text in XAML), which is what lets `CatalogueTests` scan it and
`xamlcheck` prove it.

```bash
cd win
tools/xamlcheck/run.sh .     # any OS: lint every XAML file, shadow-compile the code-behind
```

It proves the XAML's elements, attributes and handlers against the real WinUI 3 metadata and the
code-behind against the real Windows App SDK — and nothing only the XAML compiler checks, and
nothing at run time. On Windows:

```powershell
cd win
dotnet run --project src/FamilyConnect.App -p:Platform=x64   # a debug package identity, registered for you
```

or open `FamilyConnect.slnx` in Visual Studio. The bare `bin\…\FamilyConnect.exe` does not
start on its own: a packaged app's Deployment Manager needs its identity and fails before `Main`
(build with `-p:WindowsPackageType=None` for a real unpackaged binary).

**What running it on Windows taught, and where it is kept:**

- **A crash in the window is usually native and silent.** WinUI turns a failure inside its own
  controls into a fail-fast (0xc000027b) that no managed handler sees, so the diagnostics log only
  knows how far a launch got. The chat list was emptied and refilled on every change, and a chat was
  opened from inside the list's own SelectionChanged; both now happen in place and a beat later
  (`ChatsView.DrawList`, `OnChatPicked`). The rail's badges are made once, and the board never
  rebuilds inside SizeChanged.
- **`AppWindow` counts physical pixels.** `Resize(1100, 760)` was half a window at 200%;
  `WindowPlacement` sizes in effective pixels and restores the place the window was closed in.
- **An async click handler that throws ends the process** unless the XAML handler marks it handled,
  which `App` does once it has written the exception down.

Things chosen for the window that nettrash has not decided yet, and where they live:

- **Windows 11 (22000) as the minimum** — `TargetPlatformMinVersion` and the manifest, md.win's floor.
- **MSIX, unsigned** — the `win-app` CI job; the Store signs what it publishes.
- **A placeholder package identity** — `Package.appxmanifest`; Partner Center supplies the real one.
- **Calls through WebView2** — the browser engine's own WebRTC in a page of the app's
  (`Assets/Call/call.html`), driven by `CallEngine`; the engine starts on the first call and is kept.
- **Closing to the notification area**, on by default.
- **No map in a location message** (see "What is NOT here").

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

Still nettrash's to answer (see the issue): push (WNS, or the socket and the notification area as
now), a map in location messages (a third party would see the coordinate), the minimum Windows
build, and whether it ships through the Store or as a signed installer. Calls were answered by
building them on WebView2.
