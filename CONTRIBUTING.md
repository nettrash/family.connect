# Contributing to Family Connect

Thank you for helping. Family Connect is one product built from seven parts that are kept in step by
hand: a server, five clients and one wire contract. These rules keep the repository readable for the
next person, and **every pull request is checked against them** (the checklist at the end). A pull
request that breaks one is not rejected for it. It is asked to change, so the earlier you read this,
the shorter the review.

## 1. Start from an issue

- **Open or pick an issue first.** For anything bigger than a small fix, agree on the approach in the
  issue before writing code. That matters most for new dependencies, CI changes and anything that
  changes the wire.
- **One issue, one pull request.** A refactor, a reformat or a fix you found along the way gets its own
  issue.
- **Branch name:** `<issue number>-<short-slug>`, for example `116-emoji-reaction-search`.
- **Target the open release branch** (`v1.x`, currently `v1.4`), never `master`. `master` only receives
  a finished release, from the maintainer.

## 2. Keep the pull request to its issue

A feature pull request contains the code, tests, strings and design note the issue needs. It does
**not** contain:

- **A version bump** in any manifest (`Cargo.toml`, `version.properties`, the Xcode project,
  `Directory.Build.props`). Versions are set at release time.
- **A `CHANGELOG` entry.** The release notes are written for the whole release when it is cut.
- **A `README.md` paragraph about the feature.** The README describes the product and how to run it,
  and `CHANGELOG` lists the features.
- **Unrelated changes:** reformatting files you did not otherwise touch, renames, or moving code.

## 3. Repository layout

The layout is fixed:

```
docs/      the protocol, operations and dated design notes
server/    ios/   android/   web/   win/   linux/      one directory per stack
tools/     generators shared by several stacks (strings, emoji tables)
CHANGELOG  CODEOWNERS  CONTRIBUTING.md  LICENSE  README.md
```

- **No new top-level directories or files**, including output folders such as `output/`.
- **Each file lives with its stack.** Code, tests, docs and tools for one client stay in that client's
  directory (`linux/docs/`, `win/README.md`, `linux/tools/` and so on).
- **A generator used by several stacks goes in `tools/`**, next to the existing ones
  (`tools/emoji/make_names.py`).
- **`.gitignore` changes need a reason.** A tool's output goes in a directory that is already ignored,
  or in a temporary directory outside the repository.

## 4. Documentation

- **`docs/protocol.md` is the single source of truth** for the REST and WebSocket contract. If a change
  touches the wire, amend `protocol.md` first, in the same pull request, and make the code follow it.
- **`docs/operations.md`** covers running the server. Update it when a change affects configuration,
  limits, backups or monitoring.
- **A design note** is optional and named `docs/<topic>-YYYY-MM-DD.md`, using the date it was
  written. It opens like the existing notes:
  - a title naming the issue, e.g. `# Emoji as you type (issue #101)`;
  - the date it was written and the branch it was written against;
  - the decisions taken in the issue.
- **One note per feature.** A rule for one CI job, one tool or one script lives as a comment in that
  file, not as a separate document.

## 5. No evidence files in git

Screenshots, screen recordings, test reports, logs and benchmark output that show a change working
go in the **pull request description** or a CI artifact. They never go in a commit, because
everything committed stays in the history and in every clone, permanently.

The only binary files in the repository are ones the apps or the store listings ship: icons,
bundled assets, and store screenshots under `ios/docs/` and `android/fastlane/`. Those are updated
by the maintainer.

## 6. Dependencies and toolchains

- **No new language, runtime or package manager** for a stack: for example no Node/npm project in
  the Rust web client, and no Python in an app. Each stack builds with its own toolchain
  (Cargo/Trunk, Gradle, Xcode, .NET).
- **Every new dependency is named and justified in the issue first.** That covers a crate, a Gradle
  or NuGet package, a Swift package or a GitHub Action.
- **Lock files follow the existing rule. Don't change it in a pull request:**
  - tracked: `server/Cargo.lock`, `linux/Cargo.lock`;
  - not tracked (listed in `.gitignore`): `web/Cargo.lock`, `win/tools/board-oracle/Cargo.lock`.
- **Mind what ships.** Data compiled into a client (`include_str!`, bundled resources) is downloaded
  or installed by every user. Give its size in the pull request. Shared crates such as `web/text` are
  also built into the Linux and Windows code.

## 7. CI

- **CI changes are their own pull request**, agreed in an issue first. A feature adds its tests to
  the suites CI already runs (section 8), not a new job.
- **Edits to `.github/workflows/` follow the file's existing style:**
  - actions pinned by major tag (`actions/checkout@v5`);
  - jobs gated by the `changes` job;
  - each job checks how many tests actually ran, not only the exit code;
  - a comment explains why each job and step exists.
- **CI runs on `push`, not on `pull_request`.** For a pull request from a fork, enable Actions on your
  fork so your branch is tested there, and link the run in the pull request.

## 8. Tests

Tests go into the existing suite of the stack they cover:

| Stack | Suite | Run |
|---|---|---|
| Server | Rust unit + integration tests | `cargo test`, plus `cargo test -- --ignored` against a PostgreSQL |
| Web | `wasm-bindgen-test`, in a browser | `wasm-pack test --headless --chrome` |
| Shared text (`web/text`) | Rust unit tests | `cargo test` in `web/text` |
| Android | JUnit | `./gradlew testStandardDebugUnitTest lintStandardDebug` |
| iOS / macOS | XCTest / Swift Testing | `xcodebuild test …` (see `README.md`) |
| Windows | xUnit | `dotnet test` on `win/tests/*` |
| Linux | Rust tests | `cargo test` in `linux/` |

- **Logic gets unit tests:** ranking, parsing, state and storage.
- **End-to-end browser or device harnesses** (Playwright, WebDriver and the like) aren't added to CI
  or committed as a project. If one is really needed, it is a self-contained script under
  `<stack>/tools/`, like `linux/tools/call-interop`, and its results go in the pull request
  description (section 5).

## 9. Every string in every language

- **Every new user-visible string comes in all nine languages, in the same pull request:** `de`, `en`,
  `es`, `fr`, `ja`, `ru`, `sr`, `sr-Latn`, `zh-Hans`. A pull request with English-only strings fails
  the `strings` CI job.
- **Where the sources live:**
  - Apple apps: `ios/FamilyConnect/Localizable.xcstrings`;
  - the other clients and the server: `web/i18n/web.json`, `win/i18n/win.json`,
    `linux/i18n/linux.json`, `server/i18n/push.json`;
  - Android: its `res/values-*` files.
- **Run the stack's generator with `--check`** (`python3 web/i18n/generate.py . --check` and so on)
  before pushing.
- **Never edit a generated table by hand.** Serbian Latin is generated from the Cyrillic
  (`tools/i18n/`).

## 10. One copy of shared files

- **`web/rich/` and `web/text/data/emoji-names.tsv` are the single originals.** Every client ships a
  byte-identical copy, and CI compares them. Change the original, then copy it over every client's
  copy.
- **A new data file meant for every client** is generated by a script in `tools/` and copied the same
  way. Say so in the pull request, so the copy check can be extended.

## 11. Every client, the same behaviour

A feature is designed for all clients: iOS/macOS, Android, web, Windows and Linux. A pull request
may implement it on fewer, but then:

- the issue lists the clients that follow;
- the pull request doesn't change behaviour the other clients rely on.

## 12. Privacy and security

- **Nothing leaves the server box unless the owner can see it and switch it off.** A new outbound
  request is first added to the "Who can see your messages" table in `README.md` and gets an off
  switch. That covers a third-party service, a CDN, a font or an analytics call.
- **Logs never contain:**
  - message text, names, tokens or passwords;
  - SDP, JWS or URL paths.

  Log ids and counts instead.
- **No secrets, ever:** keys, keystores, `local.properties`, `keystore.properties`,
  `google-services.json`, APNs or FCM credentials, a `config.toml` with real values. No real server
  addresses or accounts in tests or docs either.
- **Report a security problem privately**, through the repository's Security tab ("Report a
  vulnerability"), never in a public issue or pull request.

## 13. Quality bar

- **Zero warnings** on every build you touch, test targets and CI annotations included.
- `cargo fmt` and `cargo clippy` are clean, and Android lint passes.
- **Every existing test still passes.** A new test that is skipped or ignored by default is called out
  in the pull request.
- **Commits reference the issue** (`#116 search the reaction picker`) and contain no build output, no
  editor files and no absolute paths from your machine.

---

## The checklist every pull request is reviewed against

- [ ] It has an issue, its branch is `<issue>-<slug>`, and it targets the open `v1.x` branch (§1).
- [ ] It only contains that issue's change: no version bump, `CHANGELOG` entry, `README` feature text
      or unrelated edits (§2).
- [ ] It adds no new top-level files or directories, and `.gitignore` is unchanged or explained (§3).
- [ ] A wire change is in `docs/protocol.md`, and any design note is `docs/<topic>-YYYY-MM-DD.md` (§4).
- [ ] It commits no screenshots, videos, reports or logs (§5).
- [ ] It adds no new runtime or package manager, every new dependency was agreed in the issue, and the
      lock-file policy is unchanged (§6).
- [ ] It doesn't touch `.github/workflows/`, or that change is its own agreed pull request (§7).
- [ ] Its tests are in the stack's existing suite and pass (§8).
- [ ] Every new string is in all nine languages, and the generators pass with `--check` (§9).
- [ ] Shared files were changed in the original and copied to every client (§10).
- [ ] The clients still missing the feature are listed in the issue (§11).
- [ ] It adds nothing that leaves the box unseen, nothing private in a log, and no secrets (§12).
- [ ] It has zero warnings, and fmt, clippy and lint are clean (§13).
