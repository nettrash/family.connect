<!--
Thank you for contributing. Please read CONTRIBUTING.md first: this pull request is reviewed
against the checklist below. Tick each item, or say next to it why it does not apply.
-->

## Issue

Closes #

## What changes

<!-- What this pull request does, and which clients it covers (iOS/macOS, Android, web, Windows, Linux, server). -->

## How it was tested

<!-- The suites you ran and how many tests executed. Link your fork's CI run (CONTRIBUTING.md §7). -->

## Screenshots or recordings

<!-- Attach them here, not in a commit (CONTRIBUTING.md §5). Delete this section if there is nothing to show. -->

## Checklist ([CONTRIBUTING.md](https://github.com/nettrash/family.connect/blob/master/CONTRIBUTING.md))

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
