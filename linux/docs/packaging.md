# Family Connect for Linux — the packages

Family Connect 1.3.0 for Linux ships, per release, as six packages and a checksum file (issue #112):

| | amd64 | arm64 |
| --- | --- | --- |
| Ubuntu 26.04 ("resolute") and later | `family-connect_1.3.0_amd64.deb` | `family-connect_1.3.0_arm64.deb` |
| Fedora 44 and later | `family-connect-1.3.0-1.fc44.x86_64.rpm` | `family-connect-1.3.0-1.fc44.aarch64.rpm` |
| any distribution with the libraries (§12) | `family-connect-1.3.0-linux-x86_64.tar.gz` | `family-connect-1.3.0-linux-aarch64.tar.gz` |

plus `SHA256SUMS`, attached to the tag's GitHub Release by `.github/workflows/release-linux.yml`
(§13). The issue names Ubuntu Desktop 26.04, and that is what is verified; the `.rpm` is a
second-class target (docs/linux-client-2026-10-28.md, "Media").

This document is adapted from md.linux's `docs/packaging.md`, which shipped the same deliverable
(md 1.6.0) on 2026-10-28; its scripts are the template, and the traps it paid for are kept fixed
here. Facts were measured on 2026-10-28 in the images named below.

---

## 0. Decisions

| Topic | Decision | Why |
| --- | --- | --- |
| Tooling | `dpkg-deb --root-owner-group` plus `dpkg-shlibdeps` (`dpkg-dev`, in the dev image through `build-essential`); `rpmbuild` in Fedora; no debhelper, no fakeroot | md.linux's: no new tools, and `--root-owner-group` makes every entry root-owned |
| Package name | `family-connect` — the binary, the `.deb`, the `.rpm`, the AppArmor profile, the doc folder | The workspace's binary crate; Debian names are lower case |
| Desktop id | `me.nettrash.FamilyConnect` — the GTK application id, the `.desktop` name, the icon name, the AppStream id | GNOME matches windows to the launcher by `app_id`; the family's bundle ids are `me.nettrash.*` |
| Version | `1.3.0`, from `linux/Cargo.toml`'s `[workspace.package]` (native: no Debian revision; the `.rpm` is `-1`) | The family's number: the release tag `v1.3` is the Linux client's 1.3.0 (§13) |
| Architectures | `amd64` and `arm64`, each built natively on its own architecture (GitHub's `ubuntu-24.04` and `ubuntu-24.04-arm`; on an Apple-silicon Mac arm64 natively, amd64 under emulation) | One Dockerfile, two platforms; no cross toolchain |
| Reproducible | pinned base image digests, an Ubuntu archive snapshot, `SOURCE_DATE_EPOCH` = HEAD's commit time, `--remap-path-prefix`, `cargo --locked`, normalised mtimes and modes, `gzip -9n`, one xz thread | Two builds of one commit give one SHA-256 (measured: §9.5) |
| Sandbox | `/etc/apparmor.d/family-connect` grants `userns` (an unconfined profile), as Ubuntu does for every WebKitGTK app | Without it WebKit's bubblewrap cannot start on Ubuntu 24.04 and later (§7) |
| Runtime plugins | The GStreamer plugin packages for calls, media and video in the window are hard `Depends` (`Requires` on Fedora) | The client cannot place a call, record a message or draw video without them; they are not linked, so no tool would find them |
| Recommends | `gnome-keyring`, `xdg-desktop-portal-gnome` (both in `main`) | The session token lives in the Secret Service; the camera, notifications and the global shortcut go through GNOME's portals. Ubuntu Desktop has both; a minimal system still installs the package |

---

## 1. File layout

| Path | Mode | Source in the tree | Notes |
| --- | --- | --- | --- |
| `/usr/bin/family-connect` | 0755 | `target/release/family-connect`, stripped (and `.comment`, `.note` removed, as dh_strip does) | ≈ 390 KB on arm64 |
| `/usr/share/applications/me.nettrash.FamilyConnect.desktop` | 0644 | `packaging/me.nettrash.FamilyConnect.desktop` | §2 |
| `/usr/share/metainfo/me.nettrash.FamilyConnect.metainfo.xml` | 0644 | `packaging/me.nettrash.FamilyConnect.metainfo.xml` | §5 |
| `/usr/share/icons/hicolor/<size>/apps/me.nettrash.FamilyConnect.png` | 0644 | `packaging/icons/hicolor/…` (generated once, committed) | §4, 13 sizes |
| `/etc/apparmor.d/family-connect` | 0644, conffile | `packaging/apparmor/family-connect` | §7 |
| `/usr/share/man/man1/family-connect.1.gz` | 0644 | `packaging/family-connect.1` → `gzip -9n` | lintian wants a manual page |
| `/usr/share/doc/family-connect/copyright` | 0644 | `packaging/copyright` | §6 |
| `/usr/share/doc/family-connect/changelog.gz` | 0644 | `packaging/changelog` (Debian format) → `gzip -9n` | the native package's changelog |
| `/usr/share/doc/family-connect/CHANGELOG.gz` | 0644 | the repository's `CHANGELOG` → `gzip -9n` | the family's user-facing release history |
| `/usr/share/dbus-1/services/me.nettrash.FamilyConnect.service` | 0644 | `packaging/dbus/me.nettrash.FamilyConnect.service` | §2: `DBusActivatable` |
| `/usr/share/icons/hicolor/symbolic/apps/me.nettrash.FamilyConnect-symbolic.svg` | 0644 | `packaging/icons/hicolor/symbolic/…` | the top-bar icon (shell-design §16.2) |
| `/usr/share/family-connect/…` | 0644 (folders 0755) | `linux/data/…`, copied whole | the data folder: `i18n/` (the catalogue's tables, written by `linux/i18n/generate.py`), and later the rich page |
| `DEBIAN/control`, `md5sums`, `conffiles`, `postinst`, `postrm` | 0644 / scripts 0755 | `packaging/control.in`, `packaging/postinst`, `packaging/postrm` | §7, §8 |

Nothing is installed under `/usr/lib`, `/var` or `/opt`; nothing in the user's home is created by the
package. The data folder `/usr/share/family-connect` is `linux/data` copied whole (the layout checks
of the .deb, the .rpm and the archive expect every file in it), and the binary finds it **beside the
executable** — `<prefix>/share/family-connect` for `<prefix>/bin/family-connect` — before
`/usr/share` (`app/src/services/paths.rs`), so the portable archive works too (§12). Without it the
client runs in English and logs a warning.

The desktop and icon caches are refreshed by the dpkg file triggers of `desktop-file-utils` and the
GTK icon cache, so the package ships no maintainer-script code for them.

The binary is built from the exported source (`scripts/package-lib.sh`, `SOURCE_PATHS`): `linux/`
without `linux/tools/rtc-spike` (`linux/tools/call-interop` is a workspace member, so it stays),
`web/text/` (the `fc-text` crate, by path) and the `CHANGELOG`. A file
outside those that the build starts to read must be added there and to the `linux` path filter of
`ci.yml`.

---

## 2. The desktop entry — `me.nettrash.FamilyConnect.desktop`

```ini
[Desktop Entry]
Type=Application
Name=Family Connect
GenericName=Family Chat
Comment=Chat and call with your family on your own server
Icon=me.nettrash.FamilyConnect
Exec=family-connect %F
TryExec=family-connect
Terminal=false
Categories=Network;InstantMessaging;Chat;
Keywords=family;chat;messages;calls;video;voice;board;
StartupNotify=true
DBusActivatable=true
MimeType=image/jpeg;image/png;image/gif;image/webp;image/heic;image/heif;image/avif;image/tiff;image/bmp;video/mp4;video/quicktime;video/webm;video/x-matroska;video/x-msvideo;audio/mp4;audio/x-m4a;audio/mpeg;audio/wav;audio/x-wav;audio/ogg;audio/flac;audio/aac;application/pdf;
StartupWMClass=me.nettrash.FamilyConnect
X-GNOME-UsesNotifications=true
```

`MimeType` and `%F`: nettrash decided (2026-10-29) that release 1 registers for pictures, videos,
sound and PDF ("Open With", docs/shell-design.md §8.6) — never "any file", and never as the default
(Ubuntu's own viewers stay the defaults). Added together with the receiving side (P2's inbox:
"Send to" a chat, then staged there). `DBusActivatable=true` with
`/usr/share/dbus-1/services/me.nettrash.FamilyConnect.service` (`Exec=/usr/bin/family-connect
--gapplication-service`): GNOME Shell starts the app over D-Bus, so a notification clicked after a
quit opens it (P5). The portable archive's copy of that file names `/usr/bin`; its README says how
to point it at `~/.local/bin`. The `.deb`'s install check asserts the `mimeinfo.cache` lines and
the service file. No URL scheme: an invitation link handler, if one
is added later, is an `x-scheme-handler/…` entry here and a `%U` in `Exec`.
`desktop-file-validate` (desktop-file-utils 0.28) reports nothing, on Ubuntu 26.04 and Fedora 44.

## 3. MIME types

The 23 of §2, for "Open With" only.

---

## 4. Icons

The source is the Mac's 1024 px icon, `ios/FamilyConnect/Assets.xcassets/AppIcon.appiconset/
AppIcon-macOS-512x512@2x.png`: the rounded square on a transparent margin that the Dock shows —
also the shape GNOME draws app icons in, so it is used as it is. `scripts/make-icons.sh` (run once
by hand, output committed under `packaging/icons/`) writes the 13 hicolor sizes Ubuntu 26.04's
hicolor-icon-theme index has for apps — 16, 16@2, 24, 32, 32@2, 48, 64, 128, 128@2, 256, 256@2, 512,
512@2 — every one from that single file: `512x512@2` is a byte copy, the others are
`magick SRC -filter Lanczos -resize NxN -strip -define png:exclude-chunks=date,time DST`
(ImageMagick 7.1.2 from the dev image), so the committed bytes carry no timestamp; running the script
twice gives the same bytes (checked). The Mac's own smaller PNGs are not used, so there is one
source of truth. 416 KB in all. AppStream wants an icon of at least 64 px; it gets 128 and up.

---

## 5. AppStream — `me.nettrash.FamilyConnect.metainfo.xml`

A `desktop-application` component: id, CC0 metadata, `MIT` project licence, name, summary,
description, the launchable desktop id, the stock icon, homepage/help (the support page, §10) and
the repository, categories `Network`, `InstantMessaging`, `Chat`, `<provides><binary>`, OARS 1.1
content rating, branding colours sampled from the icon (`#095CAF` light, `#0B4F9C` dark), and the
release `1.3.0`. The copy follows the family's rules: no "no third-party dependencies", "zero
permissions" or "no network", and no `<` or `>` in prose.

**Content rating (OARS 1.1)** — nettrash should confirm: `social-chat` **intense** and `social-audio`
**intense** (unmoderated chat and voice/video between members — OARS's word for "not moderated by
the publisher", which a family's own server is not), `social-location` **intense** (a member can
share a location). Everything else none.

`appstreamcli validate --no-net --pedantic` (appstream 1.1.2 on Ubuntu 26.04) passes with one
pedantic note, `cid-contains-uppercase-letter`: the id is the GTK application id
`me.nettrash.FamilyConnect`, which must equal the desktop file's name and is the family's
spelling; lower-casing it would split the launcher from the window. Screenshots are added when the
store copy is written (they need a real desktop). The release date moves with the real release.

---

## 6. Copyright — `/usr/share/doc/family-connect/copyright`

Machine-readable (`copyright-format/1.0`). Everything is `Expat` (MIT, © 2026 nettrash), the
repository's licence: Family Connect's code, desktop entry, AppStream data, manual page, AppArmor
profile and icon. The binary's paragraph (`Expat and Zlib`) names every crate compiled into it (94
on 2026-10-29: the gtk-rs and gstreamer-rs bindings, libsecret, rusqlite, serde, time, …) and the
Rust standard library; each declares MIT or offers it (`MIT OR Apache-2.0`, `Unlicense OR MIT`, the
old-style `MIT/Apache-2.0`), and the package takes the MIT option — all but ONE: `foldhash`, the
hash rusqlite's statement cache uses through hashlink and hashbrown, is Zlib only. Zlib asks
nothing of a binary (an acknowledgment "would be appreciated but is not required"); its text is the
file's last paragraph, and the .rpm says `License: MIT AND Zlib`. `scripts/package.sh`
(`check_crates`) **refuses to build** when the binary links a crate the file does not name, or one
that offers no MIT — Zlib is accepted for the crates in `ZLIB_CRATES` (foldhash) alone, so a new
licence still stops the build and is a decision, not a drift. There are no bundled
third-party files (unlike md's preview engines), so there is nothing else to declare.

---

## 7. AppArmor — `/etc/apparmor.d/family-connect` and the maintainer scripts

Ubuntu 24.04 and later set `kernel.apparmor_restrict_unprivileged_userns = 1`; WebKitGTK's web
processes run inside bubblewrap, which needs an unprivileged user namespace, and when it cannot get
one WebKit aborts the whole UI process (md.linux, facts §9). The client draws rich text in a
WebKitGTK view (docs/linux-client-2026-10-28.md), so it needs the same profile md ships — Ubuntu's
own profile for a WebKitGTK app, which confines nothing and only allows `userns`:

```
abi <abi/5.0>,
include <tunables/global>

profile family-connect /usr/bin/family-connect flags=(unconfined) {
  userns,
  mr @{exec_path},

  # Site-specific additions and overrides. See local/README for details.
  include if exists <local/family-connect>
}
```

It is a conffile. `postinst` and `postrm` reproduce what `dh_apparmor` generates: on configure,
create `/etc/apparmor.d/local/family-connect` and load the profile when AppArmor is enabled; on
purge, remove the local include and cache. The check container compiles the profile
(`apparmor_parser --skip-kernel-load`), but no container can load it: loading it, and seeing
WebKit's sandbox start under it, is the first item of the real-desktop test (§11). Today's binary
creates no web view yet, so it would also run without the profile; the profile is installed now so
that the package's shape does not change when the shell adds one.

---

## 8. Control data and lintian

### 8.1 `DEBIAN/control` and Depends

```
Package: family-connect
Version: 1.3.0
Architecture: amd64 | arm64
Maintainer: nettrash <nettrash@nettrash.me>
Installed-Size: <du of the staging tree in KiB, as dpkg-gencontrol counts>
Depends: <see below>
Recommends: gnome-keyring, xdg-desktop-portal-gnome
Section: net
Priority: optional
Homepage: https://nettrash.me/linux/familyconnect/support.html
Description: family chat and calls on the family's own server
 …
```

The description says what Family Connect is and does as a product (the same words as the AppStream
copy); today's binary is the shell's first window, so it describes the client the 1.3 release is
meant to ship.

`Depends` is assembled by `scripts/package.sh` (`compute_depends`) from three parts. On arm64,
2026-10-28:

```
libadwaita-1-0 (>= 1.9), libc6 (>= 2.34), libgcc-s1 (>= 4.2), libglib2.0-0t64 (>= 2.88),
libgstreamer1.0-0 (>= 1.28), libgtk-4-1 (>= 4.22), libwebkitgtk-6.0-4 (>= 2.52),
libsqlite3-0, libsecret-1-0,
gstreamer1.0-plugins-base (>= 1.28), gstreamer1.0-plugins-good (>= 1.28),
gstreamer1.0-plugins-bad (>= 1.28), gstreamer1.0-plugins-ugly (>= 1.28),
gstreamer1.0-libav (>= 1.28), gstreamer1.0-nice, gstreamer1.0-pipewire, gstreamer1.0-gtk4,
glib-networking
```

1. **The linked libraries — `dpkg-shlibdeps`.** From the binary's `NEEDED` entries (today:
   libwebkitgtk-6.0, libgtk-4, libgio/libgobject/libglib-2.0, libgstreamer-1.0, libadwaita-1,
   libgcc_s, libc). Its floors reflect only the symbols the binary references, but the crates are
   built with `v4_22`, `v1_9`, `v2_52`, `v2_88` and `v1_28` and the client is tested on nothing
   older, so the floors are raised: libgtk-4-1 4.22, libadwaita-1-0 1.9, libwebkitgtk-6.0-4 and
   libjavascriptcoregtk-6.0-1 2.52, libglib2.0-0t64 2.88, libgstreamer1.0-0 and GStreamer's base
   and bad libraries 1.28. The script **fails** if `dpkg-shlibdeps` names a package outside
   `ALLOWED_DEPENDS`, so a new linkage is reviewed, not shipped silently.
2. **Libraries the client will link and does not yet** — `libsqlite3-0` (fc-core's cache, the
   system SQLite) and `libsecret-1-0` (the Secret Service token store). Appended unless
   `dpkg-shlibdeps` already names them; once the shell links them, its entry (with a floor) wins.
3. **What the client loads at run time** — GStreamer's plugins: base (Opus, playback), good (VP8,
   RTP, MP4 mux/demux, PulseAudio, V4L2), bad (`webrtcbin`, DTLS/SRTP, H.264 parsing), ugly
   (`x264enc`, the H.264 encoder), libav (`avenc_aac`, `avdec_h264`), nice (ICE), pipewire (camera
   and microphone through the portal), gtk4 (`gtk4paintablesink`, which draws video in the window —
   docs/shell-design.md; gst-plugins-rs, 0.14.4 on 26.04), and `glib-networking` (TLS for GIO: the
   server's https and wss). Ugly and libav are in `universe`, which Ubuntu Desktop enables; the
   rest are in `main`. Floors 1.28 where the package is GStreamer's own release (not gtk4, which
   is versioned by gst-plugins-rs).

The check container proves (3) delivers what the plan reads: after `apt install`, every element
of `webrtcbin nicesrc nicesink dtlssrtpenc dtlssrtpdec rtpopuspay rtpvp8pay opusenc opusdec vp8enc
vp8dec x264enc avdec_h264 h264parse avenc_aac mp4mux qtdemux pipewiresrc gtk4paintablesink` exists
(`gst-inspect-1.0 --exists`). `bubblewrap` and `xdg-dbus-proxy` arrive through
`libwebkitgtk-6.0-4`'s own Depends.

**Recommends** — `gnome-keyring` (the Secret Service that keeps the session token) and
`xdg-desktop-portal-gnome` (the camera, notifications and the global shortcut through the portals).
The check installs with `--no-install-recommends`, so it only proves they resolve
(`apt-get install -s`).

**md5sums**: `md5sum` of every file under the staging root except `DEBIAN/`, paths relative, sorted.

### 8.2 lintian (2.129.0ubuntu2.1, in a throwaway 26.04 container — not part of the build)

Run at **every level**: `--display-info --pedantic --display-experimental --show-overrides`.
The package ships **no overrides**; lintian reports these tags and no other (the script's
`ACCEPTED_TAGS`, each pinned to its own files):

| Tag | Why it is right here |
| --- | --- |
| `X: hardening-no-fortify-functions [usr/bin/family-connect]` (experimental, info visibility) | Rust code calls no fortified (`_chk`) libc function; there is no C source to fortify. The binary is PIE with full RELRO and BIND_NOW (checked with `readelf`). |
| `I: font-in-non-font-package`, `I: font-outside-font-dir` — only `usr/share/family-connect/rich/fonts/KaTeX_*` | The rich page's web fonts: WebKit loads them through the app's own `fc-rich` scheme. Page assets, not system fonts; `web/rich` is one set shared byte for byte by every client (CI's rich job). |
| `I: package-contains-documentation-outside-usr-share-doc` — only `usr/share/family-connect/rich/rich.html` | The renderer's page, which lintian takes for documentation. |
| `X: spelling-error-in-binary` — only `usr/bin/family-connect` | The nine languages' sentences are compiled into the binary (fc-text); lintian's English dictionary reads German "Assistent"/"Tage", French "dont"/"autorisation", Spanish "funcion"… as misspellings. |

The script fails on any other tag at any level, and on these anywhere else, and also when that one is **missing**: it is the
proof that lintian analysed the binary (lintian 2.129 prints nothing at all for a clean package, and
`--verbose` prints no "processing" line), as md.linux used its font overrides.

---

## 9. Building it

### 9.1 `scripts/package.sh [--clean] [--no-check] [--allow-dirty] [--native] [amd64|arm64 …]`

Per architecture (default: the host's):

1. **Images.** `docker buildx build --platform linux/$ARCH --load -t fc-linux-dev:$ARCH docker/` —
   `docker/Dockerfile`: `ubuntu:26.04` pinned by its multi-arch index digest and the archive
   pinned to the snapshot `20261027T000000Z` (snapshot.ubuntu.com), so the compiler (rustc 1.93.1),
   GTK 4.22.4, libadwaita 1.9.1, WebKitGTK 2.52.6 and GStreamer 1.28.2 headers do not move under a
   rebuild. The check images `fc-linux-ubuntu:$ARCH` are the pinned base and nothing else, **tagged
   per architecture** by `docker buildx build` (`package-lib.sh`, `base_image`): running the pinned
   digest itself for a second platform fails in Docker's classic image store on GitHub's runners
   ("cannot overwrite digest").
2. **Source.** Exported from git (`SOURCE_PATHS`): HEAD itself when those paths are clean; with
   `--allow-dirty`, their tracked and untracked-but-not-ignored files. Changes elsewhere in the
   repository (ios/, server/, …) do not count as dirty: they cannot change a Linux package.
3. **Build and stage** in `fc-linux-dev:$ARCH` (target and stage in named volumes
   `fc-linux-package-{target,stage}-<checkout>-$ARCH`, the registry in `fc-linux-cargo`):
   version checks (the Debian changelog, the metainfo release, the rpm `%changelog` and the man
   page must all name the workspace version), `check_crates` (§6), `cargo build --release --locked
   -p family-connect` with `CARGO_ENCODED_RUSTFLAGS` remapping the source, cargo home and target
   paths to fixed ones, `CARGO_PROFILE_RELEASE_STRIP=true`, `…_DEBUG=false` (the workspace already
   has `lto = "thin"`, `codegen-units = 1`); stage §1, compute Depends (§8.1) and the control
   files; every mtime set to `SOURCE_DATE_EPOCH`.
4. **`dpkg-deb`** — in `fc-linux-dev` of the **machine's own** architecture
   (`--root-owner-group --threads-max=1 -Zxz -z9`): under Docker Desktop's Rosetta, GNU tar cannot
   stat a path given by name, and dpkg-deb always names its files to tar; what dpkg-deb writes
   depends on its version and input, not the processor, and both images carry the same dpkg. Then
   the layout check (`dpkg-deb --info` / `--contents` against §1: fields, members, owners, modes,
   every mtime — listed with `TZ=UTC0`, since tar prints local time — and the exact file list), and
   the portable archive (§12) from the same staged files.

### 9.2 `SOURCE_DATE_EPOCH`

The commit time of `HEAD` (`git log -1 --format=%ct`), also with `--allow-dirty`: a build of an
uncommitted tree carries the time of the commit it sits on. **Today the whole of `linux/` is
uncommitted**, so the packages built for this work carry HEAD's time — d9a75ef, 2026-10-28
20:21:09 UTC (`1793218869`) — and their bytes will change once `linux/` is committed. In a
monorepo every commit moves HEAD, so packages of two commits differ in their timestamps even when
nothing under `linux/` changed; reproducibility is per commit.

### 9.3 Checks before success (`--check-inside`, then `--lintian-inside`)

In a fresh `fc-linux-ubuntu:$ARCH`: the checking tools first (`desktop-file-utils appstream
apparmor gstreamer1.0-tools`), then `apt-get install ./family-connect_1.3.0_$ARCH.deb` resolving
every Depends from the live archive, with no `W:`/`E:` line; `ldd` resolves everything;
`family-connect --version` prints `family-connect 1.3.0` (no display, no D-Bus: the option is
answered before the application registers); the GStreamer elements (§8.1); postinst created
`/etc/apparmor.d/local/family-connect` and `apparmor_parser` compiles the profile; the Recommends
resolve;
`desktop-file-validate` prints nothing; `appstreamcli validate --no-net --pedantic` passes (§5); the
user's files under `~/.config`, `~/.local/state`, `~/.local/share/family-connect` survive
`apt remove` (which keeps the conffile and its local include) and `apt purge` (which removes both
and leaves dpkg knowing nothing); the portable archive unpacked anywhere runs `--version` and has
its icons beside `bin/`; reinstall works. Then lintian (§8.2) in a throwaway container of the
machine's own architecture.

**Under emulation** (amd64 on an Apple-silicon Mac, Docker Desktop's Rosetta), GNU tar 1.35 cannot
create a file or a folder at all ("Cannot mkdir: Function not implemented") — the trap md.linux
met in `dpkg-deb`. So `dpkg-deb` runs in the machine's own architecture (§9.1 step 4), the check
container unpacks the archive with `bsdtar` (libarchive-tools), and lintian — and, for the `.rpm`,
rpmlint, which unpacks with `rpm2archive | tar` — run in a container of the machine's own
architecture: they inspect the package and never run it. apt, dpkg, rpm and dnf unpack without
tar and work under emulation. On GitHub's runners every step is native.

### 9.4 Without Docker — `scripts/package.sh --native`

On an Ubuntu 26.04 machine with `docker/Dockerfile`'s build packages (`build-essential pkg-config
git rustc cargo libgtk-4-dev libadwaita-1-dev libwebkitgtk-6.0-dev libgstreamer1.0-dev
libgstreamer-plugins-base1.0-dev libgstreamer-plugins-bad1.0-dev libsecret-1-dev libsqlite3-dev`;
the script names what is missing), steps 2–4 run on the machine itself, for its own architecture,
with the target directory `linux/target/package` and `~/.cargo` remapped to the containers' paths,
so the binary holds the same paths as a Docker build's. Of §9.3 only lintian runs (if installed):
the other checks would install the package on the machine that builds it. The bytes equal a Docker
build's only while the machine's packages match the image's snapshot; the release build stays the
Docker one. `FC_DEV_NATIVE=1 scripts/dev.sh package` is the same command.

### 9.5 Measured (2026-10-28, an Apple-silicon Mac, `--allow-dirty`, `SOURCE_DATE_EPOCH=1793218869`)

arm64 natively, amd64 under Docker Desktop's Rosetta (each package's whole build and checks took
under three minutes once the images existed; building the amd64 dev image under emulation took
about fifteen):

| | bytes | SHA-256 |
| --- | --- | --- |
| `family-connect_1.3.0_arm64.deb` | 456 680 | `3e49b4bd3db025c84d7f80137c750676cf2a9eb28ec8b6f73934e41888177775` |
| `family-connect_1.3.0_amd64.deb` | 474 516 | `d6d3ba479b1d1eca5d470fe6de3147f45ed0dff62f274cbcc6dc9024b6a83b0e` |
| `family-connect-1.3.0-linux-aarch64.tar.gz` | 590 858 | `b5ef32a4999f466340e4329b0483f416772d8131184db0cdb83745a42e60e356` |
| `family-connect-1.3.0-linux-x86_64.tar.gz` | 599 474 | `06cc71f69860d9f5984b882c59bff9184cb4f612efd278aa8d565693e4d5cb36` |
| `family-connect-1.3.0-1.fc44.aarch64.rpm` | 475 429 | `6e6c8254942bc9283a18593ad398343953b2a7f8e2d296136e8f8291eae041c1` |
| `family-connect-1.3.0-1.fc44.x86_64.rpm` | 484 833 | `7a4e755141b39073d3e19e9e86277366cb5a9769fd584b2ee6f09f9633f8de12` |

Reproducibility, measured: the arm64 `.deb` came out identical from an incremental build, a
`--clean` build and a `--native` build run inside the dev image on a scratch git repository of the
same files committed at the same time (so through `git archive HEAD`, the path CI takes), and the
arm64 tarball identical from the Docker and the `--native` build; the amd64 `.deb` and tarball and
the arm64 `.rpm` each identical from two successive builds (Fedora's archive did not move between
them). The installed binary is 397 944 bytes on arm64; `Installed-Size` is 846 KiB (arm64) and 866
KiB (amd64). dpkg-shlibdeps' libc6 floor is 2.34 on both.

---

## 10. Open questions for nettrash

1. **Support and privacy pages**: `https://nettrash.me/linux/familyconnect/support.html` is the
   Homepage of every package and the AppStream homepage/help URL, after the site's pattern
   (`appstore/`, `play/`, `msstore/` `familyconnect/`, `linux/md/`). It does not exist yet.
2. **The OARS content rating** (§5).
3. **The release and the placeholder**: a `v1.3` tag runs the release workflow and attaches what
   the Linux client is at that commit. If v1.3 is tagged before the shell is done, the attached
   packages are today's one-window build.
4. **The `.rpm`'s H.264**: Fedora has no `x264enc`; `openh264enc` is a weak dependency (§12).

---

## 11. Manual install and uninstall test — a real Ubuntu 26.04 desktop

Once on amd64 and once on arm64 (a VM is fine), GNOME on Wayland, the user's own account; note the
build's SHA-256.

1. `sudo apt install ./family-connect_1.3.0_<arch>.deb` — dependencies resolve; no warning.
2. `sudo aa-status | grep -w family-connect` lists the profile; `/etc/apparmor.d/local/family-connect`
   exists.
3. Family Connect is in the app grid with its icon at every zoom; Super, type "family" or "chat"
   finds it.
4. Launch: one window titled "Family Connect"; the dock shows the app's icon (not a generic one),
   grouped with the window (`StartupWMClass`, the application id).
5. A second launch brings the first window forward instead of opening another process's window.
6. Once the shell has a WebKit view: `ps -ef | grep WebKitWebProcess`, and
   `grep NSpid /proc/<pid>/status` shows a namespace (the sandbox is on, under the profile).
7. `sudo apt remove family-connect`: the app leaves the grid; `/etc/apparmor.d/family-connect`
   remains (a conffile); the user's `~/.config/family-connect`, `~/.local/share/family-connect`
   remain.
8. `sudo apt purge family-connect`: the profile and its local include are gone; `dpkg -L` reports
   nothing; reinstalling works.

---

## 12. The `.rpm` and the portable `.tar.gz`

**`family-connect-<version>-linux-<x86_64|aarch64>.tar.gz`** — written by `scripts/package.sh`
beside each `.deb`, from the same staged files (§1): the staged `/usr` as
`family-connect-<version>/` (`bin/family-connect`, `share/…`), plus `packaging/tarball/README`
(requirements, running in place, installing into `~/.local`) and the AppArmor profile as
`share/doc/family-connect/apparmor-profile`, a template whose path is changed to wherever
`bin/family-connect` lives. `bin/family-connect` adds the `share/icons` beside it to GTK's icon
search path, so its window has its icon wherever the folder is (verified: unpacked under `/opt` in a
system with no Family Connect icon, under Xvfb). Reproducible like the `.deb`: sorted names,
`root:root`, every time `SOURCE_DATE_EPOCH`, gzip without a name or time. The binary is the Ubuntu
26.04 build: it needs the libraries of §8.1 at their floors (Ubuntu 26.04, Fedora 44 and later).

**`family-connect-<version>-1.fc44.<x86_64|aarch64>.rpm`** — `scripts/package-rpm.sh [amd64|arm64 …]`.
Built natively in Fedora 44 (`docker/Dockerfile.fedora`, the base pinned by its multi-arch index
digest — md.linux's), the oldest Fedora with every library the client is built against (measured
2026-10-28: GTK 4.22.5, libadwaita 1.9.4, WebKitGTK 2.54.2, GStreamer 1.28.8, Rust 1.99.0), so it
links Fedora's own libraries and `rpmbuild`'s generators write its library Requires (by soname).
Fedora keeps no archive snapshots, so a rebuild may use newer packages than the last one; the build
time and file times are still `SOURCE_DATE_EPOCH`.

| | `.deb` (§1) | `.rpm` (`packaging/rpm/family-connect.spec`) |
| --- | --- | --- |
| Binary, desktop entry, metainfo, icons, man page | as §1 | the same paths (rpm gzips the man page) |
| Documents | `copyright`, `changelog.gz`, `CHANGELOG.gz` | `CHANGELOG` (`%doc`); `/usr/share/licenses/family-connect/copyright` (`%license`); `License: MIT` |
| AppArmor profile | `/etc/apparmor.d/family-connect` | none: Fedora confines with SELinux and does not restrict user namespaces |
| Linked libraries | `dpkg-shlibdeps`, floors raised | rpm's soname generators |
| SQLite, libsecret (not yet linked) | `libsqlite3-0`, `libsecret-1-0` | `libsqlite3.so.0()(64bit)`, `libsecret-1.so.0()(64bit)` |
| GStreamer plugins | the packages (§8.1) | by the **element capability** each Fedora plugin package provides: `gstreamer1(element-webrtcbin)()(64bit)` (bad-free), `…nicesrc…` (libnice-gstreamer1), `…opusenc…` (base), `…vp8enc…`, `…mp4mux…` (good), `…avenc_aac…` (gstreamer1-plugin-libav), `…pipewiresrc…` (pipewire-gstreamer); `glib-networking%{?_isa}` |
| Video in the window | `gstreamer1.0-gtk4` | `gstreamer1(element-gtk4paintablesink)()(64bit)` — `gstreamer1-plugin-gtk4` (0.15.2 on 2026-10-28) |
| H.264 | `x264enc` (ugly), `avdec_h264` (libav) | Fedora has neither. **Recommends** `gstreamer1(element-openh264enc)()(64bit)` (gstreamer1-plugin-openh264, whose library comes from Cisco's repository — on by default on Workstation and in the container): weak, so the package still installs where that repository is off |
| Recommends | `gnome-keyring`, `xdg-desktop-portal-gnome` | the same two, and the openh264 capability above |
| Caches | dpkg triggers | the file triggers of desktop-file-utils and hicolor-icon-theme; no scriptlets |
| Checks before success | §9.3 and lintian | the header, Requires, Recommends and exact file list; **rpmlint 2.8.0: 0 errors, 0 warnings** (run in `fc-linux-rpm` of the machine's own architecture, §9.3) (Fedora's own configuration filters 7 messages: `no-signature`, `no-packager-tag`, `no-group-tag` — Fedora policy — and 4 `cross-directory-hard-link`, rpm hard-linking the byte-identical icon pairs such as 32x32 and 16x16@2); in a fresh Fedora 44: `dnf install` with every Requires from the archive, `ldd`, `--version`, the GStreamer elements, every Recommends provided by the archive and the openh264 one installed on its own with `openh264enc`/`openh264dec` present, `desktop-file-validate`, `appstreamcli validate --no-net --pedantic`, `dnf remove` leaving nothing of the package's and all of the user's |

---

## 13. CI and the release workflow

**Held until the client is ready** (nettrash, 2026-10-29): `release-linux.yml` builds and attaches
nothing unless `linux/packaging/release-ready` exists at the tag — its first step checks, and every
later step and job is skipped (not failed, not annotated) when the file is absent. The file is added
in the commit that finishes the Linux client; until then a family tag such as `v1.3` is safe, and
`ci.yml` still builds all six packages as artifacts on every push.

**Every push** that touches `linux/` or `web/text/` (or `.github/`, `docs/protocol.md`, which run
every job) runs `ci.yml`'s `linux` job, once per architecture — amd64 on `ubuntu-24.04`, arm64 on
`ubuntu-24.04-arm` (the repository is public, so arm64 is native): `scripts/dev.sh image`,
`scripts/dev.sh test` (fmt, clippy `-D warnings`, every test, with the count read back as the rest
of `ci.yml` does), then `scripts/package.sh <arch>` and `scripts/package-rpm.sh <arch>` with all
their checks, and the three packages uploaded as the artifact `family-connect-linux-<arch>`.

**A `v*` tag** runs `.github/workflows/release-linux.yml` (or `workflow_dispatch` with an existing
tag, to attach again after a fix):

1. **The tag and the tests** on `ubuntu-24.04`: the tag's major.minor must equal the workspace
   version's (`v1.3` and `v1.3.x` for `1.3.0`; a tag of a family release the Linux client has not
   been moved to fails here), then `dev.sh image` and `dev.sh test`, counted.
2. **Packages**, four jobs: `.deb`+`.tar.gz` and `.rpm` × amd64/arm64, each on its own
   architecture's runner, each with every check.
3. **Attach**: the six packages and `SHA256SUMS` go to the tag's GitHub Release with
   `softprops/action-gh-release` — added to the Release nettrash created (its title and notes left
   alone), or the Release is created. `contents: write` is granted to this job alone.

The actions are the first majors that run on Node 24 (checkout v5, upload-artifact v6,
download-artifact v7, action-gh-release v3), so the runs carry no deprecation annotations.
