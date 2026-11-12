# Web CI dependencies

Dependency changes in this scope are deliberate repository changes, reviewed together with the
lockfiles, checksums and validation results. CI does not run update commands or follow a browser
release channel. The scope is only the new Web CI job and the server-artifact handoff added for it. Existing
CI jobs retain their previous action references, services, toolchains and dependency policies.

| Input | Pin |
| --- | --- |
| Playwright Test, Playwright and Playwright Core | Exact 1.65.0 packages and integrity hashes in `web/e2e/package-lock.json`; installed with `npm ci --ignore-scripts` |
| Browser and Linux userland | `mcr.microsoft.com/playwright:v1.65.0-noble` by SHA256 digest in the existing CI's Web job; Chromium 158.0.8127.18, revision 1254 |
| Node / bundled npm | Node 26.13.0 in `web/e2e/.node-version`; npm 11.20.0 in `package.json` and the CI version check; `check-latest: false` |
| Rust and host C linker/build libraries | `rust:1.95.0-trixie` by SHA256 digest; explicit 1.95.0 rustfmt/Clippy and wasm32 target |
| Rust bootstrap | Included in the digest-pinned Rust build image; self-updates disabled, exact-version component installation uses `--no-self-update` |
| Web Rust dependencies | Tracked `web/Cargo.lock`, including the Web application’s resolution of path dependencies; the new Web job uses `--locked` |
| Reused server build | Consumed from the existing server job without changing that job’s dependency policy |
| Trunk | Exact 0.22.1 archive and SHA256; release version checking disabled; Trunk runs offline after locked dependencies are fetched |
| wasm-bindgen CLI | Exact 0.2.130 archive and SHA256, matching the crate in Web's lockfile; installed on PATH before Trunk |
| New Web PostgreSQL fixture | PostgreSQL 16.15 by SHA256 digest in the Web job only; the existing server service is untouched |
| New CI steps | Full commit SHAs for actions in the new Web job and the new server-artifact upload step; pre-existing action references are untouched |
| Emoji search data | Existing CLDR source commit and input SHA256 manifest under `tools/emoji/` |

The Web job uses two pinned Docker environments on the existing runner: Rust builds/checks/serves Web, and Playwright runs the existing server binary and browser test. They use the runner loopback network and share the checkout, ephemeral evidence and explicitly pinned Node installation. The server is built once by the existing server job.

The Web job has no `apt upgrade`, `npm update`, `cargo update`, unversioned `npx`, browser-channel
selection or automatic browser installation. Its browser and required OS libraries are already in
the digest-pinned image. Rust dependency or npm manifest/lock mismatch fails instead of resolving a
new version. Standard Playwright reports are checked for the named executed scenario, failures,
skips, retries, locale coverage, transport evidence and screenshots.

No Dependabot/Renovate or scheduled update configuration is added. Automatic security-update PRs
were already disabled in `iBarBuba/family.connect` when checked on 2026-11-11; security alerts are
separate from automatic updates. Updating the Playwright package also requires an explicit matching
image-digest change. Tool updates require a version/checksum edit; Rust/npm changes require an
explicit lockfile change and a passing verification run.

GitHub's hosted runner agent, Docker daemon/kernel and Actions service are platform-managed.
The existing Server job also retains its upstream hosted Ubuntu runner. These pins freeze the
Web job's selected browser/userland and dependency inputs; they do not claim a frozen GitHub
control plane or byte-for-byte reproducibility of every platform's builds.
