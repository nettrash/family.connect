# A Windows client — what it would take (issue #64)

**Assessed 2026-09-11** against `v1.1`, in answer to issue #64: *"Copy of macOS client for
Windows 10/11 — better for 11 than 10."* This document says what the macOS client actually
contains, which Windows facts are verified rather than assumed, where the four hard parts are, and
what has to be decided.

**Phase 1 has since been started** — `win/`, the portable core: the wire (REST and the socket),
the local cache, the send queue, the board's shared arithmetic, the reconnect resync, the live
frame router, the app's session gate, the live connection's policy, the chat list, the
conversation, the board, the media outbox, the attachment cache, the family console and the
notification rules, the account's own screens, the calendar hand-off and the nine languages, with **338 tests** that run on macOS and TWO differential oracles generated from the web client's own
Rust (the wall's arithmetic, and the words a chat row is drawn with). The second assembly now
exists too — `FamilyConnect.App.Logic`, everything the window DOES with none of the window, on
md.win's split.
See `win/README.md` for what exists and "Phases" below for what does not. The stack question was
answered the way this document recommends (WinUI 3 + C#/.NET 10, md.win's layout); calls, push,
the minimum Windows build and where it ships are still open.

Writing the board model turned up THREE more, all of them rules `protocol.md` already states and
this port had not kept: a full read wiped notes that arrived while it was in flight (it must keep
anything held above the read's own mark); a deleted note could be RESURRECTED by an older copy
arriving late (the `gone` set the web client keeps — still a gap on Apple and Android); and the
board cursor was moved by the answer to the client's own write and by a frame arriving before the
wall had ever been read, either of which leaves the cursor above changes nobody has read. An older
full read landing second is now ignored too.

A FIFTH defect came out of writing the chat list: the unread count was RECOMPUTED from local
history on every applied message, so a device told "12 unread" by `GET /chats` drew 1 the moment
anything arrived — local history is not the whole of it. The count is now incremented by a live
frame only (never by a page, an edit, or the reader's own send), a list read takes the server's
number plus whatever raced it, and reading subtracts rather than recounts.

Four defects in the wire layer were found by writing the resync against the document rather than
against the other ports, and each is written up where it was fixed: a `GET /chats` row's
`max_*_seq` is the SERVER's mark and had been stored as this device's catch-up cursor (which
makes every catch-up gate false for ever); the edit feed was missing from the resync — and from
step 3 of `protocol.md`'s own list, which has been amended; `Member.role` is a STRING and had been
declared a boolean, so every owner read as a member; and a birthday is an OBJECT, which had been
declared a string — that one made `GET /families/mine` unreadable for any family where somebody
had set one, and took the whole resync down with it.

The precedent is `md.win`: the Windows port of `md`, written here in C#/WinUI 3, with all of its
logic in platform-independent libraries so that **1271 + 1215 tests run on this Mac** while the
WinUI app itself only builds on Windows. That split is the single most important lesson to carry
over, and it shapes the plan below.

---

## What "a copy of the macOS client" is

The Mac is not a small client. It shares its model, storage and networking with iOS and has its own
eleven-file view layer:

| | lines |
| --- | --- |
| Apple client (iOS + macOS, shared `Core`/`Models`/`Storage` + two view layers) | ~45,300 |
| — of which macOS-only views (`MacViews/`) | ~6,900 |
| Android client | ~44,500 |
| Web client (Rust + Yew, full macOS parity) | ~32,900 |
| `docs/protocol.md` | 4,247 |

What it does, surface by surface:

- **Getting in** — server URL, sign in, sign up, the family gate, joining by code, pending approval,
  delete account.
- **Chat** — the chat list with badges and mention marks, the conversation with day pills, unread
  divider and seen state, threads, polls, reactions, member mentions, drafts, the assistant (`@ai`).
- **Attachments** — photos, videos, voice notes, files and locations; previews, an album stack and a
  viewer; paste, drag-and-drop and share import; background uploads with an outbox.
- **The board** — notes, bare photos, events (RSVPs, calendar hand-off, assistant backdrops) and
  task lists, on a scrolling wall with drag, sizes, colours, fonts and per-note badges.
- **Calls** — one-to-one audio over WebRTC, with the system call UI (CallKit) and VoIP push.
- **The family** — the owner's console, members, invites, reports and blocks, avatars, statistics,
  the assistant's switches, birthdays.
- **Everything around it** — nine languages, a live WebSocket with reconnection and an outbox,
  APNs push, the privacy switches, and the in-app notifications.

A Windows client at parity is therefore a **30–45k line** project, not a week's work. It is the
fourth full client of a protocol that is already written down, which is the good news: the wire is
not being designed, only spoken.

## Verified Windows facts

- **Windows App SDK runs on Windows 10 1809 and later.** learn.microsoft.com's Windows App SDK hub
  (page dated 2026-09-10): *"Windows App SDK APIs run on Windows 11 and earlier versions starting
  from Windows 10, version 1809."* So "Windows 10/11, better on 11" is achievable in one binary: the
  manifest's minimum can be 10.0.17763 while Mica, the rounded corners and the newer Fluent surfaces
  are applied only where the OS has them.
- **Stable channel is Windows App SDK 2.4.0** (released 2026-08-13, supported to 2027-04-29); 1.8
  left support 2026-09-09. Current WinUI templates target `net10.0-windows10.0.26100.0` and publish
  for `x64;ARM64`. (Measured and recorded for md.win on 2026-09-05 — `md.win/docs/windows-facts.md`,
  which also holds the packaging, icon and side-loading traps already paid for once.)
- **A `UseWinUI` LIBRARY builds on macOS** with `-p:EnableWindowsTargeting=true`; the APP fails only
  in `XamlCompiler.exe` (net472). md.win's `tools/xamlcheck` shadow project type-checks the
  code-behind here, so everything but XAML markup and the MSIX can be verified on this machine.
- **MSIX packaging locally needs `dotnet build`**, not Visual Studio's MSBuild, unless the C++
  workload is installed, and an unsigned MSIX cannot be side-loaded under a normal `Publisher`.
  Both are in md.win's facts doc with the exact failure text.

## The four hard parts

**1. Calls.** There is no WebRTC in the box for .NET, and the Apple client is a thin layer over
Google's `WebRTC` framework (`Core/Calls/WebRTCClient.swift`, ~31 KB). Three ways out:

- **SIPSorcery** — a pure-C# WebRTC/SIP stack, actively maintained (10.0.16 on NuGet), with
  `SIPSorceryMedia.Windows` for capture and playback. No native build step; the protocol's own
  signalling (`docs/protocol.md`, "Calls") is ours either way. The risk is interop: our peers are
  libwebrtc on iOS/macOS/Android and Chromium on the web, so the DTLS-SRTP/Opus path needs proving
  against all three early, not late.
- **WebView2** — Chromium's own WebRTC, driven by the *web* client's call code, hosted in a view.
  Lowest risk on media and the shortest path to a working call; the cost is a second runtime inside
  a native app and a seam nobody else has.
- **Ship 1.0 without calls.** The Windows client would then do everything except ring, and say so.
  Calls are the one surface where the platform gives us nothing, and the only one whose absence is
  explainable ("calls are on the phones, the Mac and the web").

**2. Push.** The server speaks APNs and FCM. Windows would mean WNS as a third channel: an Azure app
registration, a `docs/protocol.md` amendment for the token kind, server work, and infrastructure
that only nettrash can own. The cheap and honest v1.0 is what the **web** client already does: no
push at all, a live socket while the app runs, and local Windows toasts raised from that socket.
The parity gap against macOS is then one line in the README.

**3. Storage.** SwiftData on Apple, Room on Android — both versioned local databases, and #70 has
just shown what a missing migration costs. On Windows: `Microsoft.Data.Sqlite` with explicit,
numbered migrations (closest to Room, and testable on this Mac), rather than EF Core, whose
model-first machinery is more than four tables of chat cache need.

**4. Nine languages.** `ios/FamilyConnect/Localizable.xcstrings` is the source of truth (617 keys
today) and the apps' English string IS the key. A Windows port needs `.resw` per language generated
from that catalogue, and a check that nothing was invented locally — the same rule the web client
follows.

## Where it would live

`win/`, in this repository, beside `ios/`, `android/`, `web/` and `server/`. The four clients of one
protocol are already versioned together here, and the protocol doc they are all held to is in the
same tree. (md/md.macOS/md.Android/md.win are separate repos because they are separate products;
these are not.)

Inside it, md.win's shape, which is what makes it testable here:

```
win/
  FamilyConnect.slnx
  src/FamilyConnect.Core/        protocol client, models, sync, outbox, board rules  (AnyCPU, tests on macOS)
  src/FamilyConnect.App.Logic/   view-model logic, no XAML                            (AnyCPU, tests on macOS)
  src/FamilyConnect.App/         WinUI 3 app, XAML + code-behind                      (x64;ARM64, Windows only)
  tests/FamilyConnect.Core.Tests/
  tests/FamilyConnect.App.Logic.Tests/
  tools/xamlcheck/               shadow library that type-checks the code-behind here
```

The board's shared arithmetic is the model: `fc_text::board` on the web, `BoardWall`/`BoardTasks`/
`BoardPicture` on Apple and Android. A fourth copy belongs in `FamilyConnect.Core` with the same
numbers and the same tests, so a Windows wall is the same wall.

## Phases

1. **Skeleton and the wire.** Solution, CI-less local build, `Core`: HTTP client, models, error
   shape, auth and the keychain equivalent (DPAPI/`PasswordVault`), SQLite cache with migrations,
   the WebSocket with reconnection and the send outbox. Tests on this Mac. *No UI.*
2. **The chat.** Window shell, chat list, conversation, composer, day/unread rules, mentions,
   threads, polls, reactions.
3. **Attachments.** Upload/download, previews, album, viewer, paste and drag-and-drop, the media
   outbox in the background.
4. **The board.** The wall, all four note kinds, drag, the `.ics` hand-off, backdrops.
5. **Account and family.** Sign-up, the family gate, settings, the owner's console, avatars,
   reports and blocks, statistics, the assistant's switches.
6. **Notifications, languages and the shell.** Local toasts from the socket, the nine language
   catalogues — **JSON rather than `.resw`**, because a `.resw` name cannot be an English sentence
   and the key IS the English sentence for all four clients (see `win/README.md`) — Windows 11
   chrome where present (Mica, corner preference, taskbar badge), Windows 10 fallbacks.
7. **Calls** — only if the decision below says so, and proven against all three existing peers
   before anything else in it is written.

Each phase is one session's work at the pace the web client went at (six phases, one repository,
committed as it went), except the board and the chat, which are two.

## One of the open decisions has an answer in the document

**Push.** `POST /devices` takes `ios`, `macos` or `android` and nothing else, and the protocol says
in as many words that "a browser registers no device and receives no push". A Windows client is in
exactly that position: it has no APNs or FCM token to give. So *socket-only toasts* is not a
compromise this port chose — it is what the protocol currently allows, and it is built and tested
(`NotificationRules`, 15 mutants). Giving Windows real push is a protocol change plus a server
change: a `windows` platform on `POST /devices`, a WNS channel URI where a token goes, and a WNS
sender beside the APNs and FCM ones. Worth deciding on its own merits rather than as part of #64.

## What has to be decided before phase 1

1. **The stack.** WinUI 3 + C#/.NET 10 (md.win's, and the only one where the Fluent chrome the issue
   asks for is native) — or the cheaper alternative, the **web client in a WebView2 shell**, which
   would be a Windows app in a week with no second implementation to keep in step. The issue says
   "copy of the macOS client", which reads as native; it is worth saying so out loud, because the
   second option costs a tenth as much and the difference is chrome, not features.
2. **Calls in 1.0** — SIPSorcery, WebView2-hosted, or not yet.
3. **Push in 1.0** — WNS (server work, Azure, protocol amendment) or socket-only toasts.
4. **Minimum Windows.** 10.0.17763 (Windows 10 1809, what the SDK supports) or 10.0.19041 (Windows
   10 2004, WinUI's practical floor for the newer controls), with Windows 11 getting the better
   chrome either way.
5. **Where it ships** — Microsoft Store (MSIX, Partner Center, a privacy URL is mandatory for a
   packaged desktop app) or a signed installer from nettrash.me, as `md.win` does both.

Until 1 and 2 are answered, phase 1 is the only part that is safe to write: `Core` is the same code
whatever the UI turns out to be.
