# A Linux client — what it would take (issue #112)

**Assessed 2026-10-28** against `v1.3` (after #59, PR #111), in answer to issue #112: *"a Linux Desktop
version of Family Connect. We can use macOS version as a source. Linux version should work in Ubuntu
Desktop 26.04. We should have deb rpm and tar.gz versions as artefacts after CI. These artefacts must be
added to Release as well."* This document says what "the macOS version as a source" can mean, which
Ubuntu facts are measured rather than assumed, where the hard parts are, and what has to be decided.

**Decided 2026-10-28:** Rust + gtk4-rs (b), calls in the first release and proven first, amd64 and
arm64. The sections below "The stack" are written for that answer.

---

## What "the macOS version as a source" can mean

**Not its code.** The Mac client is SwiftUI over SwiftData, AVFoundation, UserNotifications, the
Keychain and Google's `WebRTC` framework, sharing its `Core` with iOS. Swift runs on Linux, but none of
those do, and the Mac's model types ARE SwiftData models — there is no layer below them that would
compile there.

**Its behaviour — and that already exists in portable form.** The Windows client (#64) was written as a
copy of the Mac, on md.win's split, and two of its three assemblies are platform-free `net10.0`:

| `win/` | lines | runs on |
| --- | --- | --- |
| `FamilyConnect.Core` — the wire, the cache (SQLite, numbered migrations), sync, outbox, board rules | ~19,900 | anywhere; tested on `ubuntu-24.04` in CI on every push |
| `FamilyConnect.App.Logic` — everything the window DOES with none of the window | ~20,600 | anywhere; same |
| tests of the two | ~38,000 | anywhere; same |
| `FamilyConnect.App` — WinUI 3, XAML, the services (tray, toasts, hot key, recorder, call media) | ~32,100 | Windows only |

What the Mac does as a desktop app, beyond the shared screens, and what each becomes on Ubuntu:

| Mac | Ubuntu 26.04 |
| --- | --- |
| Keeps running in the menu bar when the window closes (#80) | Keeps running behind an icon in the top bar (Ubuntu's AppIndicator extension, on by default) |
| Its own notifications, one window however many (#84, #93) | Desktop notifications with actions (Answer / Decline for a call) |
| Dock badge | Ubuntu Dock badge (`com.canonical.Unity.LauncherEntry`, which Ubuntu's dock reads) |
| Login item | XDG autostart entry, through the Background portal |
| Global hot key (`MacHotKey`) | The XDG GlobalShortcuts portal (GNOME has it since 48) |
| Keychain | Secret Service (GNOME Keyring), libsecret |
| A chat or a call in a window of its own | GTK windows — something Windows could not do |
| Menu commands and keys (`MacNewLineKeys`, `MacCapitalKeys`, `MacVoiceCommands`) | Application menu, shortcuts, and the same key rules |

## Measured on Ubuntu 26.04 (Docker `ubuntu:26.04.1`, arm64, 2026-10-28)

- **Versions in the archive:** GTK 4.22.4, libadwaita 1.9.1, WebKitGTK 2.52.6, GStreamer 1.28.2,
  .NET 10 (`dotnet-runtime-10.0` 10.0.13, `dotnet-sdk-10.0` 10.0.113).
- **WebKitGTK has NO WebRTC.** `RTCPeerConnection` is undefined with `enable-webrtc` set and every one
  of WebKit's switchable features turned on; `getUserMedia` exists. So the Windows way of calling — the
  browser engine's own WebRTC in a page of the app (`WebViewCallMedia`, `Assets/Call/call.html`) — is
  closed here, and so is "the web client in a WebKitGTK window" for anything that rings.
- **GStreamer's `webrtcbin` works.** A pipeline with Opus audio and VP8 video produced an offer with a
  DTLS fingerprint (`gstreamer1.0-plugins-bad` + `gstreamer1.0-nice`).
- **A call from Rust works against Chromium, both ways** (phase 0, 2026-10-28). A gstreamer-rs 0.25.4
  peer — `webrtcbin`, Opus and VP8, signalling as JSON over a WebSocket, no STUN, two containers on
  one network — against Chromium 140 (Playwright, fake camera and microphone), 14 s each:

  | | Chromium received | the Rust peer decoded |
  | --- | --- | --- |
  | Rust offers audio + video | 698 Opus packets, 419 VP8 frames | 695 audio buffers, 277 video frames |
  | Chromium offers audio + video | 699 / 419 | 698 / 279 |
  | Chromium offers the web client's audio call (audio + a `recvonly` video transceiver) | 699 / 419 | 698 audio; no video, as it should be |

  Chromium's WebRTC IS libwebrtc, the engine of every other peer (Google's framework on iOS, macOS
  and Android, WebView2 on Windows); those builds still get a real call in phase 5.
  `gstreamer-rs` 0.25.4 builds on Ubuntu's `rustc` 1.93 only with the MSRV-aware resolver
  (`rust-version = "1.93"`, `resolver = "3"`) — without it cargo refuses a dependency.
- **Encoders for the protocol's formats exist** (`docs/protocol.md`, the media profile: H.264/AAC-LC in
  MP4, `moov` first): `x264enc` (`gstreamer1.0-plugins-ugly`) and `openh264enc` for H.264;
  `avenc_aac` (`gstreamer1.0-libav`) and `voaacenc` for AAC-LC; `mp4mux`; `pipewiresrc`, `pulsesrc`,
  `v4l2src` for capture. `fdkaacenc` is not packaged. Ugly and libav are in `universe`, which Ubuntu
  Desktop enables.
- **.NET bindings (GirCore 0.8.1)** exist for GTK 4, Adw 1, WebKit 6.0, JavaScriptCore 6.0, Gst, GstApp,
  GstAudio, GstVideo, GstPbutils, Secret 1 and Soup 3 — **but not GstWebRTC or GstSdp.** A C# call
  engine needs about a dozen hand-written bindings for those. Rust's `gstreamer-webrtc` and
  `gstreamer-sdp` are maintained by the GStreamer project itself.
- **Not measurable in a container** — the real-desktop list: the session is Wayland only (Ubuntu
  removed GNOME's X11 session in 25.10), so a window cannot restore its position, only its size; the
  top-bar icon; the GlobalShortcuts and Background portals; the camera through PipeWire's portal; and
  WebKit's sandbox under AppArmor's user-namespace restriction (md.linux ships a profile for exactly
  this).

## The three hard parts

**1. Calls.** The signalling is the protocol's (`call_offer`, `call_answer`, `call_ice`, `call_end`)
and the state machine is already written (`App.Logic/Calls.cs`); only the media engine is new: a
`webrtcbin` pipeline with Opus, VP8 (and H.264 where the peer prefers it), PipeWire capture and
playback. The risk is interop, and it is proved first, not last: against libwebrtc (iOS, macOS,
Android), Chromium (web) and WebView2 (Windows). Android's is reachable here on a throwaway emulator,
Chromium in a container; the Apple peers need a real call.

**2. No push — the same answer as Windows, and the protocol already gives it.** Linux has no APNs or
FCM token; `POST /devices` takes `ios`, `macos` or `android`. So: the socket while the app runs,
closing the window hides it, the top-bar icon opens or quits it, and the app raises its own
notifications (`NotificationRules`, already in `App.Logic`). `docs/protocol.md` names Windows where it
says this (the "Who is woken" paragraph and its siblings) and gets "Linux" beside it. **No wire change.**

**3. Media.** Recording voice notes and video messages, playing them, transcoding before upload, the
round video — on Windows that is Media Foundation, here it is GStreamer, with the same rules (the
media profile, rule C, cancellation). The `.deb` depends on the `universe` encoders above. The `.rpm`
is a second-class target: on Fedora, H.264 comes from Cisco's `openh264` repository (on by default on
Workstation) and AAC needs checking — the issue names Ubuntu, and that is what is verified.

## Packaging and release

md.linux shipped exactly what the issue asks for this week (v1.6.0: `.deb`, `.rpm` and `.tar.gz` for
amd64 and arm64, attached to the GitHub Release by its tag run). Its scripts are the template:

- `.deb` and the tarball built in Docker `ubuntu:26.04`, checked with lintian; the `.rpm` in Fedora 44,
  checked with rpmlint; reproducible at a commit (`SOURCE_DATE_EPOCH` = the commit's time).
- **Every push** that touches `linux/` (or the shared `win/src` projects) builds the packages in CI as
  artifacts, behind `ci.yml`'s "What changed" gate.
- **A `v*` tag** builds them again and attaches all six plus `SHA256SUMS` to that tag's Release
  (`v1.0`–`v1.2` have no assets today; the workflow adds to a Release you created, or creates it).
- The repository is public, so arm64 builds natively on GitHub's `ubuntu-24.04-arm`, not under QEMU.
- Traps already paid for once: Docker's classic image store cannot hold one pinned digest for two
  platforms on a runner (tag per architecture); `dpkg-deb --contents` prints local time.

## The stack — the one decision that shapes the rest

| | (a) C# + GirCore | (b) Rust + gtk4-rs | (c) web client in a shell |
| --- | --- | --- | --- |
| Logic | `win/`'s Core + App.Logic **as they are** — one implementation for both desktops | ~40,000 lines + ~38,000 lines of tests ported to Rust (the web client's `fc-text` reused directly) | the web client as it is |
| New code | the GTK shell (WinUI's ~32,000 lines' worth) + a call engine | the same shell + the call engine + the port | a shell, tray, notifications |
| Calls | `webrtcbin` through ~12 hand-written bindings | `webrtcbin` through first-class crates | needs a bundled Chromium (Electron/CEF): WebKitGTK has no WebRTC |
| Every later feature | logic written once for Windows and Linux | logic written a seventh time | free, but web-looking, not GNOME |
| Package | ~5 MB + `dotnet-runtime-10.0` from the archive (self-contained tarball ~70 MB) | ~5–10 MB | 100 MB + |
| Precedent here | md.win's split, the Windows client's CI on Linux | md.linux (your choice for md) | none |

I recommended (a). md.linux went to Rust over this same recommendation, and that was right for md: a
core that changes slowly. family.connect's portable client logic is nearly twice md.win's (40,400 lines
against 23,800), and it grows with every issue (#79, #88, #96–#107 each touched every client). Sharing it with Windows is the difference
between six implementations to keep in step and seven. Against it: GirCore is younger than gtk4-rs, and
calls need the hand-written bindings.

**Decided: (b), Rust + gtk4-rs.** What makes the port smaller than the table says: the web client's
`fc-text` (`web/text`) is already a pure-Rust crate of ~39,000 lines with 552 tests — the message-text
rules, markdown, mentions, emoji, links, the board's arithmetic, notifications' words, the media plan,
MP4 reading and writing, WAV, waveforms, transcripts and families — and it covers most of what
`FamilyConnect.Core/Text` (~10,200 lines) is. It is used as it is, by path, so the Linux client and the
web client share one copy of those rules. What is ported from C# is `Core/Protocol` (~4,900),
`Core/Store` (~4,000), `Core/Board` (~700), the parts of `Core/Text` that `fc-text` lacks, and
`App.Logic` (~20,600) — each with its tests, and with the web client's own Rust (`web/src`: `sync`,
`outbox`, `timeline`, `live`, `calls`, …) as a second reference where it says the same thing in Rust.

## Where it lives

`linux/`, beside `win/`, on md.linux's layout:

```
linux/
  Cargo.toml          workspace; default-members are the crates, so plain `cargo test` needs no GTK
  crates/fc-core/     port of FamilyConnect.Core: the wire, the SQLite cache, sync, outbox, board
  crates/fc-logic/    port of FamilyConnect.App.Logic: everything the window does, with no window
  app/                the `family-connect` binary: gtk4-rs, libadwaita, webkit6, gstreamer-rs
  packaging/ scripts/ md.linux's deb / rpm / tarball scripts, adapted
  docs/               the port plan, the shell design, the desktop checklist
```

`fc-text` comes in by path (`../web/text`). The nine languages come from the JSON catalogues Windows
uses (`win/i18n`; the English string IS the key) and the CI string gate covers them. The toolchain is
Ubuntu 26.04's own `rustc` (1.93), as md.linux's is, so the packages build natively on a plain Ubuntu
as well as in Docker; `fc-text` must keep building on it. Calls are `gstreamer-webrtc` and
`gstreamer-sdp`; media is `gstreamer-rs`; the token store is the Secret Service.

## Phases

0. **Spikes, before anything is built on them:** a `webrtcbin` call from Rust against Chromium — DONE,
   both directions, above; `fc-text` on Ubuntu's `rustc` 1.93.1 — DONE, 552 tests pass, no warnings;
   a gtk4-rs window with WebKit drawing `fc-rich.js` — md.linux has shown it.
1. **Shell and chat:** sign-in, the family gate, the chat list, the conversation, the composer.
2. **Attachments and media** on GStreamer: playback, voice notes, video messages, transcoding.
3. **Board, family, settings.**
4. **The desktop:** top-bar icon, notifications, autostart, Dock badge, global shortcut, Secret Service,
   the AppArmor profile.
5. **Calls**, proven against every peer.
6. **Packages and the release workflow** (can start alongside 1).
7. **The real-desktop checklist** for an Ubuntu 26.04 machine: everything a container cannot show.

## Decided (2026-10-28)

1. **The stack** — (b), Rust + gtk4-rs.
2. **Calls in the first release** — yes, proven against every peer before the rest is built on them.
3. **Architectures** — amd64 and arm64: six packages and `SHA256SUMS` per release.
