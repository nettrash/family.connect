# Family Connect for Windows

The fourth client of the protocol in `docs/protocol.md`, alongside `ios/` (iOS + macOS),
`android/` and `web/`. Issue #64; the assessment that scoped it is
`docs/windows-client-2026-09-11.md`.

**Status: phase 1, the portable core.** There is no window yet. What exists is the part of the
client that has nothing to do with Windows — the wire, the local cache, the send queue, the board's
arithmetic — and it is deliberately the part that can be verified on the Mac this is developed on.

```
win/
  FamilyConnect.slnx
  src/FamilyConnect.Core/
    Protocol/   ApiError, ServerUrl, Dtos, Frames, ApiClient, ChatSocket, SendPipeline,
                SendRules, ReconnectBackoff, ApiResult/ITokenStore
    Store/      Database + Migrations (numbered), ChatStore, BoardStore, OutboxStore, Times
    Board/      NoteText, NoteLook, BoardWall, BoardTasks, BoardPicture, NoteFitting, BoardBadge
    Text/       StringCatalog (the apps' English string IS the key)
  tests/FamilyConnect.Core.Tests/ xUnit, runs anywhere `dotnet` runs
  tools/board-oracle/             Rust: regenerates the shared-arithmetic fixture
```

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
dotnet test tests/FamilyConnect.Core.Tests/FamilyConnect.Core.Tests.csproj
```

.NET 10 (`global.json` pins the SDK band). Nothing here references a Windows API, on purpose: a
WinUI **app** cannot be built on macOS at all — its XAML compiler is .NET Framework — so a port
whose logic could only be tested on Windows would be a port nobody here could verify. When the app
project arrives it is the only one that needs Windows, and md.win's `tools/xamlcheck` trick
type-checks its code-behind here even then.

## The oracle

`tests/.../Fixtures/board-vectors.json` is not hand-written: every vector in it was produced by
`fc_text::board` — the Rust the web client runs — through `tools/board-oracle`, which depends on
`web/text` by path so it cannot drift. `OracleTests` holds this port to it, number for number.

```bash
cd win/tools/board-oracle
cargo run --quiet > ../../tests/FamilyConnect.Core.Tests/Fixtures/board-vectors.json
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
