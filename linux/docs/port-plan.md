# Family Connect for Linux — the port plan

The Linux client is Family Connect 1.3 for Ubuntu Desktop 26.04: a Rust + GTK 4 / libadwaita /
WebKitGTK 6.0 / GStreamer app with **the behaviour of the macOS client**, shipped as `.deb`, `.rpm`
and `.tar.gz` for amd64 and arm64. Why this stack and what was measured first:
`docs/linux-client-2026-10-28.md` (issue #112). This file is binding on every port.

## The references

- **`win/`** (C#, .NET 10) is the Mac's behaviour in portable form and the reference this port is
  written against:
  - `win/src/FamilyConnect.Core` — the wire (`Protocol/`), the SQLite cache (`Store/`), the board's
    rules (`Board/`), the text rules (`Text/`); tested by `win/tests/FamilyConnect.Core.Tests`.
  - `win/src/FamilyConnect.App.Logic` — everything the window does with none of the window; tested
    by `win/tests/FamilyConnect.App.Logic.Tests`.
  - `win/README.md` — what each piece is and every behaviour decision, with its reason.
- **`web/text`** (`fc-text`) — the shared Rust text rules. It is a **dependency, not a source to
  copy**: where `Core/Text` (or anything else) has a counterpart in `fc-text`, call `fc-text`.
  `Core/Text/RustChar.cs`, `RustCase.cs` and `UnicodeProperties.g.cs` exist only to make C# behave
  like Rust and are not ported.
- **`web/src`** — the web client, Rust: a second reference for the same behaviour written in Rust
  (`sync`, `outbox`, `timeline`, `live`, `calls`, `store`, …). Read it; do not copy its
  `web-sys`/Yew code.
- **`docs/protocol.md`** — the wire. Where a reference and the protocol disagree, the protocol
  wins, and the disagreement is reported.
- **`ios/FamilyConnect`** (the Mac: `MacViews/`, shared `Core/`) — consult it when `win/`'s
  behaviour or comment is unclear; the Mac wins.

Read all of them; **never modify `win/`, `web/`, `ios/`, `android/` or `server/`** from a port.

## Layout

| Path | What | Port of |
| --- | --- | --- |
| `crates/fc-core` | the wire, the cache, sync, outbox, board rules, text the `fc-text` crate lacks | `FamilyConnect.Core` |
| `crates/fc-logic` | the window's logic, no GTK | `FamilyConnect.App.Logic` |
| `app` (binary `family-connect`) | GTK 4 / libadwaita / WebKitGTK / GStreamer shell | `FamilyConnect.App` / `MacViews` |
| `crates/*/tests/fixtures` | fixtures shared with `win/tests`, byte-identical | |
| `docker/`, `scripts/` | the Ubuntu 26.04 dev image, `scripts/dev.sh`, the package scripts | md.linux's |
| `packaging/` | desktop file, AppStream, icons, AppArmor profile, Debian and rpm metadata (`docs/packaging.md`) | md.linux's |
| `tools/rtc-spike` | the phase-0 call spike (not built by the workspace) | |

Module map of `fc-core` (one Rust file per C# file where that reads naturally):

| Module | `win/src/FamilyConnect.Core` source |
| --- | --- |
| `rt` (seams, done) | the injected `HttpClient`, `IWebSocket`, clocks and `CancellationToken` |
| `protocol` | `Protocol/*` |
| `store` | `Store/*` |
| `board` | `Board/*` (call `fc_text::board` where it has the rule) |
| `text` | the files of `Text/*` with no `fc-text` counterpart |

## Rules for every port

**Names.** `win/`'s names in Rust spelling: `ApiClient.SendAsync` → `ApiClient::send`, a C# record
→ a struct or enum with the same fields, `FooAsync` drops `Async`. Keep the C# comments that say
*why* — they are the family's memory of each defect found and each divergence — adapted to Rust.

**Async.** Everything runs on ONE thread, the GLib main context, as the C# runs on the UI thread.
`fc-core` and `fc-logic` use `async fn` and never name an executor or a runtime: no tokio, no
`std::thread` for work. Futures need not be `Send`; shared state is `Rc<RefCell<…>>`/`Cell`, never a
`Mutex`. The platform is reached only through the seams in `fc_core::rt` (HTTP, WebSocket, clock,
sleep), which return `LocalBoxFuture`. A C# `CancellationToken` becomes **dropping the future**;
where the C# observes the token part-way (`ct.IsCancellationRequested`, a transcode), take an
`rt::Cancel`. Tests drive futures with `futures::executor::LocalPool` and the fake clock in
`rt::testing` — never real time, never a real socket.

**Strings.** Rust strings are UTF-8 and C#'s are UTF-16. APIs take and return byte offsets on char
boundaries unless the name says `utf16`; never slice at a position that was not produced by a char
walk or an ASCII match (slicing off a boundary *panics* — "Привет" crashed the server once). Where
the protocol or `win/` counts UTF-16 units (a length limit, a mention's range), count them
(`str::encode_utf16().count()`) and say so in a comment.

**Unicode.** `fc-text` is Rust's own semantics, and `win/` imitates them (`RustChar`, `RustCase`) —
so in this port `char` and `str` methods are what the C# was imitating, and are right. Where the
C# uses .NET's own rules on purpose (`CompareInfo`, `ToUpperInvariant` without the Rust shims),
port the behaviour and pin it with the C# test.

**Numbers and time.** .NET formats a `double` (`InvariantCulture`) as the shortest round-trip
string with `E` notation; Rust's `{}` never uses an exponent. `Math.Round` is banker's rounding
unless `AwayFromZero`; Rust's `round` is away from zero. Write a helper with tests wherever the C#
formats or rounds. `DateTimeOffset` → `time::OffsetDateTime`, RFC 3339 on the wire, as the server.

**JSON.** `System.Text.Json` → `serde`. A field the protocol may omit is `Option` with
`#[serde(default)]`; unknown fields are ignored (a newer server must not break an older client).
The defects `win/` found by reading the protocol (a role is a STRING, a birthday is an OBJECT, a
`GET /chats` `max_*_seq` is the server's mark, not a cursor) are pinned by tests: port them.

**Dependencies.** `fc-core`: std, `fc-text`, `serde`, `serde_json`, `time`, `rusqlite` (the
system's SQLite, no `bundled`), `futures-util`/`futures-channel` for combinators and channels.
`fc-logic`: `fc-core`, `fc-text` and those. Dev-dependencies: `futures` (`LocalPool`). Anything
else: ask first.

**Toolchain.** Ubuntu 26.04's `rustc` 1.93, edition 2024, the MSRV-aware resolver (the workspace
sets `rust-version` and `resolver = "3"`). Use no std API stabilised after 1.93. `cargo fmt --all`
and `cargo clippy --all-targets -- -D warnings` clean: **zero warnings**, tests included. No
`unsafe` (`forbid`).

**Tests.** Port **every** `win/` test of the module — same name in snake_case, same inputs, same
assertions — as integration tests in `crates/<crate>/tests/<module>_*.rs` (helpers in
`tests/common`). Fixtures are copied byte for byte and **never edited**. A test of something
`fc-text` now does is not ported; the report names it and the `fc-text` test that covers it. A test
that cannot pass on Linux by nature gets `#[ignore = "<reason>"]` and is listed in the report. A
test that fails because the port is wrong is a bug: fix the port.

**Logs.** Never log message text, a name, a token or a server URL with a path; ids and counts only
(the family rule, and `win/`'s).

**Files and git.** A port writes only the files of its own module (and its tests and fixtures); a
change it needs elsewhere — a seam, the workspace manifest, another module — goes in its report as
a request. Nothing is committed: nettrash commits. Never push, merge or tag.

## Waves

1. **`fc-core`:** `protocol` (DTOs and frames first — everything else reads them), then `store`,
   `board`, `text` and the rest of `protocol` (API client, socket, router, send pipeline, resync)
   in parallel.
2. **`fc-logic`**, in packages by the window's surfaces: session and the doorway; chat list,
   conversation and composer; media (outbox, recorder rules, round video, transcode attempts);
   board, family and settings; notifications, calls and the desktop rules.
3. **The shell** (`docs/shell-design.md`, written first: the Mac mapped onto GTK, every deliberate
   deviation listed), GStreamer media, the call engine, the desktop integration, packaging and the
   release workflow — then the real-desktop checklist.
