# Family Connect for Linux — the GTK shell design

The design for `linux/app` (binary `family-connect`, application id `me.nettrash.FamilyConnect`): the
GTK 4 / libadwaita / WebKitGTK 6.0 / GStreamer shell of Family Connect 1.3 for Ubuntu Desktop 26.04,
written in Rust over `fc-core` (the wire, the cache, sync) and `fc-logic` (everything the window does,
with no window — the port of `win/src/FamilyConnect.App.Logic`). Wave 3 builds from this document; §W
splits the work. Issue #112; the plan and the decisions are `docs/linux-client-2026-10-28.md`, the
binding rules `linux/docs/port-plan.md`.

**Sources of behaviour, in order of authority.** The Mac (`ios/FamilyConnect/MacViews/*.swift`, the
shared `Core/` and `Views/`, `docs/mac-menu-bar-2026-10-07.md`) — **the Mac wins where sources differ**.
Then Windows, which is the Mac already mapped onto a non-Apple desktop (`win/README.md`,
`win/src/FamilyConnect.App/`, `docs/windows-client-2026-09-11.md`). Then md.linux
(`~/Develop/nettrash.me/md.linux/docs/{shell-design,linux-facts,desktop-checklist}.md`), a shipped
gtk4-rs + libadwaita + WebKitGTK app on this same Ubuntu by the same author — cited below as "md".
The web client (`web/src`, `web/rich/fc-rich.js`) for the rich renderer and as a Rust reference.

**Marking.** Every claim about the platform carries one of two marks:

- **(measured: how)** — run in the `fc-linux-dev` image (Ubuntu 26.04.1 snapshot 20261027, arm64,
  Docker Desktop on Apple silicon) on 2026-10-28 for this document, or in md's spike where it says
  "md facts §n". Spikes lived in this session's scratchpad (`crates-spike`, `crates-spike2`,
  `media-spike.sh`, `gui-spike.sh`, `range-spike*.sh`); Appendix A gives each command so it can be run
  again.
- **(unverified — real desktop)** — needs GNOME on Wayland on a real Ubuntu 26.04: it lands in §C,
  the real-desktop checklist.

A design decision is not a claim and carries no mark; the decisions nettrash should look at are
collected in §0 and the deviations from the Mac in §D.

**Conventions.** gtk-rs names as the crates spell them (`gtk::`, `adw::`, `webkit::`, `gst::`, `gio::`,
`glib::`). fc-logic types are `win/`'s names in Rust spelling (`AppSession`, `NotificationRules`,
`ComposerButton` → `fc_logic::composer_button`); **the fc-logic port is authoritative for the exact
signature**, and Appendix B lists what the shell needs from it beyond `win/`'s API. Sizes are GTK
logical pixels, which are the Mac's points (1 pt = 1 px at scale 1). Chords are GTK accelerator
strings where it matters (`<Control><Shift>r`).

---

## 0. Decisions in one screen

| Topic | Decision | Why (one line) |
| --- | --- | --- |
| Process | One `adw::Application` `me.nettrash.FamilyConnect`, single instance over D-Bus, `register-session` on | md proved the hand-off (md facts §1); one socket per account |
| Windows | **The Mac's scene set**: main window, conversation-in-its-own-window, Board window, attachment viewer windows, Call window, Settings window; Family is a dialog (the Mac's sheet) | GTK can do windows (Windows could not, `win/README.md` "What is NOT here") |
| Main window | `adw::NavigationSplitView` (sidebar 240–380, ideal 280) in an `adw::ApplicationWindow`, 1000 × 680 by default, 420 × 400 at least, collapsing to one pane below 680 sp | The Mac's `NavigationSplitView` and `.defaultSize(1000, 680)` |
| Chrome | Header bars, no menu bar; the Mac's toolbar buttons in the content header; the Mac's menu-bar commands in a primary menu with their accelerators | GNOME has no global menu bar; a chat app has eight commands, not eight menus |
| Placement | Size and maximised kept; **no position** | Wayland forbids it; GTK 4 has no API (md §1.4) |
| Close | The main window **hides** while the top-bar icon exists and "Keep running in the top bar" is on; otherwise a close quits | No push on Linux: a closed client cannot ring (`win/README.md`, "SO CLOSING…") |
| Thread, polls | `adw::Dialog` over the conversation — the Mac's sheets | Mac wins over Windows' side panel |
| Timeline | A **bounded, non-lazy** column of row widgets in a `gtk::ScrolledWindow`, anchored to the bottom by hand | Lazy lists estimate heights and lose the anchor on giant rows (the Apple clients' scroll history) |
| Text fields | `gtk::TextView` for every multi-line prose field, typing rules applied at user-action boundaries as md does | md's proven hook pattern (md §3.3) |
| Rich blocks | One hidden **mapped** `webkit::WebView` drawing `fc-rich.js` jobs, photographed with `snapshot_future` | Windows' `RichRenderer` on md's off-screen renderer (md facts §6) |
| Media | GStreamer everywhere: `gstreamer-play` + `gtk4paintablesink` for playback, `pulsesrc` + `avenc_aac` + `mp4mux faststart=true` for voice, camera portal → `pipewiresrc` for video messages, `x264enc` (then `openh264enc`) for transcodes | Every element measured present (§1.3); the profile measured byte-exact (§1.4) |
| Authenticated media | `souphttpsrc extra-headers` carries `Authorization: Bearer`, Range reads included | Measured against a local server (§1.4); the Mac's `AVURLAsset` with header fields |
| Calls | `webrtcbin` behind fc-logic's `CallMedia` seam; Opus + VP8 (H.264 when an offer has no VP8), `webrtcdsp` echo cancelling, ring tones synthesised as Windows and the web do | Phase-0 spike against Chromium; WebKitGTK has no WebRTC |
| D-Bus | **gio's GDBus for everything** (portals, StatusNotifierItem, LauncherEntry, logind) — no zbus | One connection, one thread, one main context; no new crates. `ksni`/`ashpd` measured as fallbacks |
| Tray | StatusNotifierItem + `com.canonical.dbusmenu` exported over GDBus; unread count as `XAyatanaLabel` | Ubuntu's own AppIndicator extension reads it (measured from its source) |
| Notifications | `gio::Application::send_notification` (GNotification); "who, never what"; one per chat, replaced; Answer/Decline on a call | GLib picks the backend; actions come back as `app.` actions |
| Launch at login | An XDG autostart file written by the app (`--background`), **not** the Background portal | Deterministic, desktop-neutral, testable; the portal adds nothing for a host app |
| Badge | `com.canonical.Unity.LauncherEntry.Update` signal (`count`, `count-visible`) | Ubuntu Dock reads exactly these (measured from its source) |
| Global shortcut | GlobalShortcuts portal, preferred trigger Ctrl+Alt+Shift+F, after `org.freedesktop.host.portal.Registry.Register`; plus `family-connect --toggle` for any desktop | Portal 1.21 refuses a host app without an app id (measured: its NEWS) |
| Token | Secret Service through `libsecret` (0.9), one item per server origin | GLib-native async, works on the main context (measured round trip) |
| Keep awake | `gtk::Application::inhibit` with `IDLE` while recording, `IDLE` and `SUSPEND` in a call | GTK talks to the session manager or the Inhibit portal itself |
| Settings on disk | `glib::KeyFile` at `$XDG_CONFIG_HOME/family-connect/settings.conf` | md's choice and reasons (md §9.1) |
| Strings | `fc-core`'s catalogue over Windows' nine JSON tables, plus `linux/i18n/linux.json` for the port's own sentences | The English sentence is the key (`win/README.md`); one source of truth |
| Accent | The Mac's accent (`#0D47A1` light, `#4D7DFC` dark) as libadwaita's `--accent-*` variables | Mac wins; the reader's own balloon must look the same on every client |
| Self-test | `family-connect --selftest OUT` behind the cargo feature `selftest`, headless under Xvfb with `xdotool`, as md | md's harness, proven on this image |

---

## 1. Measured facts

### 1.1 Toolchain and libraries (measured: `rustc --version`, `pkg-config --modversion` in `fc-linux-dev`)

rustc / cargo **1.93.1**; GTK **4.22.4**; libadwaita **1.9.1**; WebKitGTK 6.0 **2.52.6**; GStreamer
**1.28.2**; libsoup **3.6.6** (md facts §0); libsecret **0.21.7**; GLib **2.88.0**; SQLite **3.46.1**;
`gstplay-1.0` 1.28.2; xdg-desktop-portal **1.21.1**, xdg-desktop-portal-gnome **50.0**, GNOME Shell
**50.1**, mutter 50.1, PipeWire **1.6.2**, WirePlumber 0.5.13 (measured: `apt-cache policy`).

### 1.2 Crates — resolve, build and link on rustc 1.93 with `rust-version = "1.93"`, `resolver = "3"`

(measured: a throwaway crate built and linked in `fc-linux-dev`, 0 warnings; `cargo tree`)

| Crate (renamed) | Version resolved | Features | Used for |
| --- | --- | --- | --- |
| `gtk4` (`gtk`) | 0.11.5 | `v4_22` | everything drawn |
| `libadwaita` (`adw`) | 0.9.2 | `v1_9` | windows, split view, dialogs, rows, toasts |
| `webkit6` (`webkit`) | 0.6.1 | `v2_52` | the rich renderer; re-exports `soup` (soup3 0.9.0) and `javascriptcore` |
| `gstreamer` (`gst`) | 0.25.4 | `v1_28` | pipelines |
| `gstreamer-play` | 0.25.0 | `v1_28` | the player (playbin3 under it) |
| `gstreamer-video` / `-audio` / `-app` / `-pbutils` | 0.25.4 / 0.25.4 / 0.25.2 / 0.25.4 | `v1_28` | caps, appsink/appsrc, `Discoverer` |
| `gstreamer-webrtc` / `gstreamer-sdp` | 0.25.4 / 0.25.3 | `v1_28` | calls |
| `libsecret` | 0.9.0 (MSRV 1.92) | default (`v0_18`) | the token |
| `gdk4-wayland` / `gdk4-x11` | 0.11.5 | `v4_20` / `v4_18` | portal parent-window handles |

Not chosen, but measured to build on 1.93 with no tokio (`cargo tree -i tokio` finds none): `ksni` 0.3.7
(`default-features = false, features = ["async-io"]`), `ashpd` 0.13.13 (`async-io`, `global_shortcuts`,
`background`, `camera`, `inhibit`, `gtk4_wayland`, `gtk4_x11`), `oo7` 0.6.0 (`async-std`,
`native_crypto`) — together 175 packages and zbus 5.19. They are the fallback if the GDBus code of §16
and §19 proves too costly; adding them is nettrash's call (port-plan "Dependencies").

API shapes that cost a compile (measured: compiled against the crates above):

- `soup::Session::send_and_read_future(&msg, Priority)` → `glib::Bytes`; `Message::status()` is a
  `soup::Status` — its number is `status().into_glib()`, not `as u16`.
- The WebSocket is `Session::websocket_connect_async_future(&msg, origin, &protocols, Priority)` (not
  `websocket_connect_future`); `WebsocketConnection::connect_message` hands the data type as an `i32`,
  compared with `soup::WebsocketDataType::Text.into_glib()`.
- `gst::ElementFactory::find`, `gst_play::Play::new(None::<gst_play::PlayVideoRenderer>)`,
  `gst_pbutils::Discoverer::new`, `libsecret::password_{store,lookup,clear}_future`,
  `gtk::Accessible::announce`, `gtk::GraphicsOffload`, `adw::{NavigationSplitView, BreakpointBin,
  Dialog, AlertDialog, BottomSheet, Spinner, WrapBox, ToggleGroup, ShortcutsDialog, Banner,
  ToastOverlay, SwitchRow, EntryRow, PasswordEntryRow, ComboRow, AboutDialog}`,
  `webkit::SnapshotOptions::TRANSPARENT_BACKGROUND`, `gtk::ApplicationInhibitFlags::{IDLE, SUSPEND}`,
  `gio::NotificationPriority::Urgent` — all exist (50 types compile-checked).
- `adw::*_version()` panics before GTK is initialised ("Gtk has to be initialized before using
  libadwaita") — print versions after `adw::init`.
- **`gio::DBusConnection::signal_subscribe` is deprecated** in gio 0.22 ("Prefer subscribe_to_signal") — a
  warning, so an error under `-D warnings`: every D-Bus signal in §15–§19 uses `subscribe_to_signal`.
- `gst::DeviceMonitor::add_filter` collides with GTK's `FileChooserExt::add_filter` when both preludes are
  glob-imported: import `gst::prelude::DeviceMonitorExtManual` by name.
- Also compile-checked (a second file, 0 warnings): `gdk::Surface::scale`, `gtk::Window::{set_startup_id,
  is_suspended}`, `gio::Notification::{set_category, add_button_with_target_value}`,
  `gtk::Application::{inhibit, uninhibit, is_screensaver_active, dbus_connection}`, `adw::SpinRow`,
  `adw::StyleManager::is_high_contrast`, `gtk::Settings::is_gtk_enable_animations`, `glib::language_names`,
  `gdk::Clipboard::read_future`, `TextView::connect_preedit_changed`, `connect_paste_clipboard` +
  `stop_signal_emission_by_name`, `TextBuffer::connect_{begin,end}_user_action`,
  `Widget::set_cursor_from_name`, `Pixbuf::save_to_bufferv`, `Texture::save_to_png_bytes`,
  `gdk::TextureDownloader`, `WebView::snapshot_future`, `UserContentManager::register_script_message_handler`,
  `NetworkSession::new_ephemeral`, `FileDialog::open_multiple_future`, `gtk::{FileLauncher, UriLauncher}`,
  `gio::Subprocess::newv`, `gio::bus_watch_name_on_connection`, `DBusConnection::{emit_signal, call_future,
  subscribe_to_signal}`, `gio::bus_get_future(BusType::System)`, `gdk4_wayland::WaylandToplevel::export_handle`.

### 1.3 GStreamer elements on Ubuntu 26.04 (measured: `gst::ElementFactory::find` and `gst-inspect-1.0`)

| Element | Plugin / package | Note |
| --- | --- | --- |
| `gtk4paintablesink` | `gtk4` 0.14.4 / **`gstreamer1.0-gtk4`** (main, section `rust`) | **not in `fc-linux-dev` today** — the image and the `.deb` must add it |
| `pipewiresrc` (`fd`, `path`, `target-object`) | `gstreamer1.0-pipewire` 1.6.2 | the camera from the portal's fd |
| `pulsesrc`, `pulsesink` | `gstreamer1.0-plugins-good` | the microphone and the speaker via PipeWire's Pulse server |
| `v4l2src` | `-good` | fallback camera |
| `webrtcbin`, `nicesrc` | `-bad`, `gstreamer1.0-nice` 0.1.23 | calls |
| `webrtcdsp`, `webrtcechoprobe` | `-bad` (libwebrtc-audio-processing 1.3) | echo cancelling, noise suppression, gain |
| `x264enc` (profiles incl. `high`) | `gstreamer1.0-plugins-ugly` (universe) | H.264 first choice |
| `openh264enc` (`constrained-baseline … high`), `openh264dec` | `-bad` + `libopenh264-8` 2.6.0 (universe) | H.264 second choice |
| `avenc_aac`, `avdec_aac`, `avdec_h264` | `gstreamer1.0-libav` (universe) | AAC-LC; decoders |
| `voaacenc` | `-bad` | AAC second choice |
| `mp4mux` (`faststart`), `qtdemux`, `h264parse`, `aacparse` | `-good`, `-bad` | |
| `opusenc/dec`, `vp8enc/dec`, `vp9dec`, `rtp{opus,vp8,h264}{pay,depay}` | `-base`, `-good` | calls |
| `playbin3`, `uridecodebin3`, `decodebin3`, `souphttpsrc` (`extra-headers`) | `-base`, `-good` | playback |
| `level`, `spectrum`, `scaletempo`, `volume`, `audiomixer`, `videocrop`, `videoflip`, `videorate`, `videoscale`, `appsink`, `appsrc` | `-base`, `-good` | meter, speed, mix, crop, mirror |
| `webpdec`, `gdkpixbufdec`, `jpegdec`, `pngdec` | `-bad`, `-good` | animation frames |
| **missing**: `fdkaacenc`, `isomp4mux` (Rust mp4 muxer), `glsinkbin`/`gtkglsink` (need `gstreamer1.0-gl`), `vah264enc/dec`, `v4l2h264enc` | | no hardware encoder is assumed |

GTK 4.22's own media backend is **built into `libgtk-4-1`** (`GtkGstMediaFileBuiltin`, links
`libgstplay-1.0`; there is no `libgtk-4-media-gstreamer` package) (measured: `ldd`, `strings`). It is
not used: `gtk::MediaFile` cannot set an `Authorization` header or a playback rate.

### 1.4 The media profile, made by GStreamer (measured: `gst-launch-1.0` + `gst-discoverer-1.0` + a box walk)

- **A video message**: a 1280 × 720 camera-like source → `videocrop left=280 right=280 ! videoscale !
  video/x-raw,width=480,height=480 ! x264enc bitrate=500 key-int-max=60 ! video/x-h264,profile=high !
  h264parse ! mp4mux faststart=true` + `avenc_aac bitrate=64000` on mono 48 kHz → boxes **`ftyp moov
  mdat`**, H.264 **High** level 3, 480 × 480, 30/1, 8-bit 4:2:0; AAC-LC mono 48 000 Hz ≈ 65 kbit/s.
- **`faststart=true` is what puts `moov` first**: the same mux without it wrote `ftyp free mdat moov`.
- **A voice note**: `avenc_aac bitrate=64000` takes 64 000 for mono directly — **unlike Media
  Foundation**, whose four documented AAC rates forced Windows into its fallback ladder
  (`win/README.md`). Major brand `mp42`, boxes `ftyp moov mdat`. `voaacenc bitrate=64000` also works.
- **A transcode**: 1920 × 1080 at 60 fps → `videorate ! videoscale ! video/x-raw,width=1280,height=720,
  framerate=30/1 ! x264enc bitrate=2000 ! video/x-h264,profile=high` → 1280 × 720, 30/1, High, `moov`
  first.
- **TRAP — pin the raw format.** An `x264enc` with no caps after it, fed by a source that can offer
  more, negotiated **High 4:4:4, 10-bit** (measured: `rot.mp4`), which no browser and no Apple
  decoder plays. Every encode puts `video/x-raw,format=I420` before the encoder **and**
  `video/x-h264,profile=high` after it (`main` where `openh264enc` is the encoder).
- **Rotation**: a source tagged `image-orientation=rotate-90` comes out of `mp4mux` with the tag
  intact (read back by the discoverer) — the "stored orientation, turn carried as metadata" rule of
  Windows' `MediaEncoding` is available as a tag.
- **Meter / waveform**: `level interval=100000000` on a decoded 10 s note posted 101 messages — one per
  tenth of a second, the cadence `fc_text::waveform` wants.
- **Authorised Range reads**: `souphttpsrc extra-headers="headers, Authorization=(string)\"Bearer\ …\""`
  sent the header on **every** request, including the two `Range` requests `qtdemux` made to fetch a
  `moov` at the end of a 22 MB file (`bytes=22870641-`, then `bytes=40-`); without the header the server
  answered 401 and the element posted "Unauthorized". (measured: a local Python server logging headers)

### 1.5 Pictures, the round clip, the Secret Service (measured: `gui-spike.sh` under Xvfb)

- **Decoding**: `gdk::Texture::from_bytes` and `gdk_pixbuf::Pixbuf::from_stream` both decoded PNG
  (alpha kept), JPEG, WebP, GIF, **HEIC** and TIFF; `PixbufAnimation` reported animated GIF **and
  animated WebP** as not static. gdk-pixbuf 2.44 goes through **glycin** (`glycin-loaders` 2.1.5 is in
  the image), which printed `Glycin running without sandbox` in Docker; its loader formats include
  avif, heic, jxl, webp. On the desktop glycin sandboxes its loaders with bubblewrap — the same user
  namespace question as WebKit's (§9.5) (unverified — real desktop).
- **Round by construction**: a `gtk4paintablesink` paintable in a `gtk::Picture` (`ContentFit::Cover`)
  inside a box with CSS `border-radius: 9999px` and `set_overflow(gtk::Overflow::Hidden)` drew a **disc
  with the corners clipped** — exactly what WinUI could not do (`win/README.md`, microsoft-ui-xaml
  #8264). The picture's natural size is the video's (480): the box took the whole 400 × 400 window
  instead of its 240 request, so the circle needs a fixed-size parent (§10.4).
- **libsecret**: under `dbus-run-session` with `gnome-keyring-daemon --unlock --components=secrets`,
  `password_store_future` (schema `me.nettrash.FamilyConnect.Session`, attributes `server`, `account`,
  collection `COLLECTION_DEFAULT`) → `Ok`; lookup → the token; clear → `Ok`; lookup after → `None`.

### 1.6 D-Bus behaviour of GTK and GLib (measured: `dbus-monitor` on a private session bus, no portal, no Shell)

- An `adw::Application` asks `org.freedesktop.portal.Settings.Read` (five times) at start-up and owns
  its id on the bus. **It did not call `org.freedesktop.host.portal.Registry.Register`** although
  `libgtk-4.so.1` contains that interface name — so the shell registers itself (§19).
- `send_notification` with neither `org.gtk.Notifications` nor a portal on the bus went to
  **`org.freedesktop.Notifications.Notify`** (GLib's freedesktop backend). On GNOME the Shell owns
  `org.gtk.Notifications` and GLib uses that backend; which one 2.88 prefers when both it and the
  portal exist for a host app is (unverified — real desktop) — the code is the same either way.
- `gtk::Application::inhibit` called `org.gnome.SessionManager.IsSessionRunning` and returned cookie 0
  with no session manager: GTK goes to the session manager first.
- `libwebkitgtk-6.0` contains `org.freedesktop.portal.Camera`, `…Notification`, `…Inhibit`;
  `libgio-2.0` contains `GNOTIFICATION_BACKEND`, `org.gtk.Notifications`, `org.freedesktop.portal.Notification`.

### 1.7 The desktop's own code, read from Ubuntu 26.04's packages (measured: `apt-get download` + `dpkg -x`)

- **The AppIndicator extension** is `ubuntu-appindicators@ubuntu.com` in **`gnome-shell-ubuntu-extensions`**
  (a `Depends` of `ubuntu-desktop-minimal`; the package `gnome-shell-extension-appindicator` **no
  longer exists** in 26.04). Its `indicatorStatusIcon.js`: a **right click** toggles the menu; a
  **middle click** calls `SecondaryActivate` (or `XAyatanaSecondaryActivate`); a **left click opens
  the menu** after the double-click time when the item has one, and a **double click calls
  `Activate`**; with no menu, a single click does nothing at all. It reads **`XAyatanaLabel`** and draws
  it beside the icon. So a single click cannot open the window as the Mac's does (§16, §D).
- **Ubuntu Dock** (`ubuntu-dock@ubuntu.com/launcherAPI.js`) subscribes to
  `com.canonical.Unity.LauncherEntry.Update(s app_uri, a{sv})` from any sender, strips the scheme from
  `application://<id>.desktop`, and reads `count`, `count-visible`, `progress`, `progress-visible`,
  `urgent`, `updating`, `quicklist`; a sender leaving the bus clears its entries.
- **xdg-desktop-portal 1.21.1** (NEWS): "Require a valid AppID from apps to use the Global Shortcuts
  Portal (#1817)", "… in `RequestBackground` to enable autostart (#1793)", the host app registry since
  1.19.4, "Make the camera permissions per-App on the host again (#1762)", `ConfigureShortcuts` (1.21.0).
  **xdg-desktop-portal-gnome 50** implements `Access, Account, AppChooser, Background, Clipboard,
  DynamicLauncher, FileChooser, GlobalShortcuts, InputCapture, Lockdown, Notification, Print,
  RemoteDesktop, ScreenCast, Screenshot, Settings, Usb, Wallpaper` (its `gnome.portal`). The Camera
  portal has no backend in that list — the frontend serves it from PipeWire itself; Inhibit is not in it
  either (xdg-desktop-portal-gtk provides that backend), and a host GTK app goes to
  `org.gnome.SessionManager` directly anyway (§1.6).
- **Default key bindings** (GNOME 50 schemas + Ubuntu's `10_ubuntu-settings.gschema.override`):
  **Ctrl+Alt+Shift+F is unbound**. Taken with three modifiers: Ctrl+Shift+Alt+R (screen recording),
  Ctrl+Shift+Alt+arrows (move to workspace), Ctrl+Shift+Alt+Tab/Escape (panels).
- **GTK 4.22** knows `gtk-interface-reduced-motion` and the CSS media query `prefers-reduced-motion`;
  libadwaita 1.9 ships `high-contrast.css` and the accent-colour variables (measured: `strings`).
- `ubuntu-desktop-minimal` **Depends** on `gnome-shell-ubuntu-extensions`, `pipewire-pulse`,
  `wireplumber`; **Recommends** `gnome-keyring`, `libpam-gnome-keyring`, `xdg-desktop-portal-gnome`,
  `ibus`, `orca`. It pulls neither `gstreamer1.0-plugins-ugly` nor `-libav` nor `-gtk4`: the `.deb`
  must.
- Available but not used in release 1: `libayatana-appindicator-glib2` (a GLib-only AppIndicator; C,
  would need `unsafe` FFI, which the workspace forbids), `libportal-gtk4-1` 0.9.1 (C, same),
  `libspelling-1-2` 0.4.9 (spell checking; crate `libspelling` 0.5.0, not built here).

---

## 2. The application, its process and its state

### 2.1 `main` and the `adw::Application`

```rust
let app = adw::Application::builder()
    .application_id("me.nettrash.FamilyConnect")
    .flags(gio::ApplicationFlags::HANDLES_OPEN)     // files from the command line → "Send to" (§8.6)
    .register_session(true)                         // query-end at logout; screensaver-active
    .build();
use glib::{Char, OptionArg, OptionFlags};
app.add_main_option("background", Char::from(0), OptionFlags::NONE, OptionArg::None,
                    "Start with the window hidden (used at login)", None);
app.add_main_option("toggle", Char::from(0), OptionFlags::NONE, OptionArg::None,
                    "Show the window, or put it away if it is in front", None);
app.add_main_option("version", Char::from(0), OptionFlags::NONE, OptionArg::None,
                    "Print the version and exit", None);
app.connect_handle_local_options(..);  // --version prints "Family Connect 1.3.0" and exits;
                                       // --toggle: register(); if is_remote() → activate_action("toggle-window") and exit 0
app.connect_startup(..);               // Shell::start (§2.4)
app.connect_activate(..);              // present the main window — unless this is the first activation
                                       // of a --background launch and StartupSetting::starts_hidden says so
app.connect_open(..);                  // §8.6
app.connect_query_end(..);             // park recordings, flush settings — synchronously
```

`glib::set_application_name("Family Connect")`; `gtk::Window::set_default_icon_name("me.nettrash.FamilyConnect")`.
GTK uses the application id as the Wayland `app_id`, which ties the windows to
`me.nettrash.FamilyConnect.desktop` in the dock (md §1.1).

### 2.2 Single instance and activation

The first process owns `me.nettrash.FamilyConnect` on the session bus; a second launch hands its
activation (and files, and `--toggle`) to it and exits (measured in md: 20 ms, md facts §1). Without a
session bus every launch is its own primary — Docker only.

**Activation routes**, all ending in the one main window (§3.1) — there is never a second one, so the
Mac's #93 machinery (`MenuBarRules.mainWindowStep`, the 2 s grace) is not needed: the window is a
single object the `Shell` owns from start-up to quit, hidden or shown, never destroyed while the
process runs.

| Route | What the shell does |
| --- | --- |
| `activate` (launcher, dock, second launch) | present the main window (Mac: launching a running app activates it) |
| `app.toggle-window` (`--toggle`, the portal shortcut) | `GlobalHotKeyRules::action(window_in_front, can_hide_to_tray)` → present or hide |
| `app.open-chat(x)` (a notification's default action) | present the main window, then route through `ToastArguments::parse` (anything but a positive id opens nothing) — held until the gate reaches `Active` (Windows' `ToastActivation`) |
| `app.answer-call(s)` / `app.decline-call(s)` | the call engine, for that `call_id` only; Answer also presents the Call window |
| `app.open-board(x)`, `app.open-family` | the Mac's routes: board window; Family dialog (owner routes `joinRequests`, `reports`) |
| `app.show-settings` (tray menu) | the Settings window |
| `app.quit` | §3.8 |

**Wayland focus.** A window can only take focus with an xdg-activation token. GTK forwards the token a
launcher or the notification server put in the platform data, so `present()` from those routes raises
the window; one raised by the socket alone (an incoming call) gets GNOME's "is ready" notification
instead of focus — which is why the call's own notification carries Answer (§14.6) (unverified — real
desktop).

**Activation when not running.** The `.desktop` file says `DBusActivatable=true` and the package ships
`/usr/share/dbus-1/services/me.nettrash.FamilyConnect.service` (`Exec=/usr/bin/family-connect
--gapplication-service`), so a click on a notification from an app that has since quit starts it and
delivers the action (unverified — real desktop).

### 2.3 State on disk

| What | Where | Why there |
| --- | --- | --- |
| settings | `$XDG_CONFIG_HOME/family-connect/settings.conf` (`glib::KeyFile`, group `[family-connect]`, atomic save) | md §9.1 |
| the cache (SQLite, numbered migrations), the outbox in it | `$XDG_STATE_HOME/family-connect/cache.db` (`0700` folder) | it holds unsent messages: user data, never `~/.cache` (carry-forward, "Where the cache lives") |
| staged media, parked recordings | `$XDG_STATE_HOME/family-connect/{media,parked}/` | Windows' `AppFolders.StagingPath`, `ParkedPath`; carry-forward (w2 logic-rec, anim) |
| current family, awaiting join | keys `current-family`, `awaiting-join` of `settings.conf` | carry-forward (w2 logic-session): "both memories belong in the shell's settings file" |
| files handed to the app | `$XDG_DATA_HOME/family-connect/inbox/` | §8.6 |
| downloaded attachments, rich drawings, map tiles, avatars | `$XDG_CACHE_HOME/family-connect/{blobs,rich,tiles,avatars}/` | rebuildable; blobs and avatars wiped when the server changes |
| copies made for Open (an attachment handed to the desktop's app for its type) | `$XDG_CACHE_HOME/family-connect/opened/` (one folder per attachment), swept at launch | P2's `attachments::saving::open`; the other app may still read it while this one runs |
| transcode and recorder temporaries | `$XDG_CACHE_HOME/family-connect/tmp/`, swept at launch | **not** `$XDG_RUNTIME_DIR`: a minutes-long transcode in tmpfs is RAM |
| autostart entry | `$XDG_CONFIG_HOME/autostart/me.nettrash.FamilyConnect.desktop` | §17 |
| the token | the Secret Service (§20) | never a file |
| web data | nowhere: one ephemeral `webkit::NetworkSession` | md §4.1 |

`glib::user_{config,data,cache}_dir()`; folders created on first write. A `--selftest` run refuses to
start unless every `XDG_*` variable points into its output folder (md §13.2).

### 2.4 The `Shell` and the seams it implements

One `Shell` per process (`Rc`, reachable from every window), created in `startup`:

| Service | Implements | Over |
| --- | --- | --- |
| `SoupHttp` | `fc_core::rt::Http` | one `soup::Session` (user agent `FamilyConnect-Linux/<version>`); `send_and_read_future`; deadline from `HttpRequest::timeout` via `glib::timeout_future` raced with the send; `TransportError` mapped from `gio::IOErrorEnum`/`gio::TlsError` |
| `SoupSockets` | `fc_core::rt::WebSocketConnector` / `WebSocket` | `websocket_connect_async_future` with `Authorization: Bearer` on the `Message`; a `futures_channel::mpsc` fed by `connect_message`/`connect_closed`; `close_code()` |
| `GlibClock` | `fc_core::rt::Clock` | `OffsetDateTime::now_utc()`, `glib::timeout_future` |
| `SecretTokenStore` | `ITokenStore` (fc-core) | §20 |
| `StringCatalog` | `IStringCatalog` (fc-core) | §23 |
| `BlobFiles`, `MediaFolder` | `IBlobStore`, `IMediaStore` (fc-logic) | whole-or-nothing files (`gio::File::replace_contents`, which renames into place) |
| `FamilyFile`, `JoinFile` | `ICurrentFamily`, `IAwaitingJoin` | small files, §2.3 |
| `GstCallMedia` | `ICallMedia` (fc-logic `calls`) | §14 |
| `Notifier`, `Tray`, `Badge`, `HotKey`, `Autostart`, `KeepAwake`, `SessionWatch` | the desktop (§15–§22) | GDBus, GTK |
| `RichRenderer`, `MapTiles`, `LinkPreviewFetcher`, `Player`, `VoiceRecorder`, `VideoMessageRecorder`, `MediaPreparing` | — | §9, §8, §10 |

As integrated (wave 3): `Shell::numbers` — how the reader writes numbers (`LC_NUMERIC`: the decimal
separator and the thousands group, CLDR's for the nine), used by the file sizes, the staged chips and
Statistics; `Shell::close_guard` — asked by `Shell::quit` first, P3's "Delete video message?" over a
take (§10.6); ONE keep-awake (`desktop::keep_awake::hold`) for recordings and calls, each hold its own
cookie.

Everything runs on the GLib main context: `glib::MainContext::default().spawn_local` drives fc-logic's
futures (port-plan "Async"). GStreamer's streaming threads and WebKit's processes are theirs; every
callback that crosses back (a bus message, a `new-sample`, a `webrtcbin` signal) goes through
`glib::MainContext::default().invoke` or a `futures_channel::mpsc` channel drained by a
`spawn_local` task — never a `Mutex` around fc-logic state.

---

## 3. The windows

### 3.1 The main window (`windows::MainWindow`)

| | Mac | Windows | Linux |
| --- | --- | --- | --- |
| Shape | `NavigationSplitView`: sidebar (min 240, ideal 280, max 380) + detail; toolbar | one window: 48-epx rail + list 300 (min 260) + conversation + thread panel 400 | `adw::ApplicationWindow` → `adw::NavigationSplitView` (`min-sidebar-width` 240, `max-sidebar-width` 380, `sidebar-width-fraction` 0.28) |
| Default size | 1000 × 680 | 1200 × 820 epx, physical pixels scaled | **1000 × 680** (`set_default_size`), last size from `settings.conf` |
| Minimum | detail 420 × 320 + sidebar 240 | rail + list + 200-epx field (#106) | **420 × 400** (`set_size_request`); below **680 sp** an `adw::Breakpoint` sets `collapsed = true`: one pane at a time, the sidebar first, a back button in the content header |
| Position | restored by AppKit | restored | **not restored** — Wayland |
| Title | family name; "(3) Family Connect" on the icon | count + name | `adw::WindowTitle`: family name; the toplevel title `NotificationRules::window_title("Family Connect")` = "(3) Family Connect" (what the dock and Alt+Tab show) |

Content of each pane is an `adw::ToolbarView` with its own `adw::HeaderBar` (the libadwaita split-view
idiom):

- **Sidebar header**: title = the family switcher (`gtk::MenuButton` with the family name and
  `gio::Menu` from `fc_logic::families::FamilySwitcher` rows; plain title with one family and nothing
  pending — the Mac's rule); start: **New Group** (`list-add-symbolic`, the Mac's toolbar button);
  end: the **primary menu** (`open-menu-symbolic`, F10) — Record Voice Message (Ctrl+Shift+R), Record
  Video Message, Refresh (Ctrl+R), New Group…, Settings (Ctrl+,), Keyboard Shortcuts (Ctrl+?), About
  Family Connect, Quit (Ctrl+Q). Under the header: `InvitationBanner` (an `adw::Banner`); at the bottom
  of the sidebar: `ConnectionBanner` (offline / reconnecting line).
- **Content header**: title = the chat's name (subtitle: typing line or "seen", fc-logic's); end:
  **Return to Call** (only while a call is not idle), **Board** with its badge, **Family**; for the open
  chat: **Call**, **Video Call** (direct chats; hidden by `calls_enabled`/`video_calls_enabled`),
  **Open polls**, **Group info** (groups). The badge is a `gtk::Overlay` pill (`.badge` CSS) — the Mac's
  pre-macOS-26 fallback, since GTK has no badge.
- **Detail placeholder** with no chat: `adw::StatusPage` "No conversation selected" / "Pick a chat
  from the sidebar." (the Mac's `ContentUnavailableView`). On launch the family chat is selected but
  **not read** until it is in front of somebody (`ChatPresence`, the Mac's rule).

While the video recorder is open (§10.6) every header button of that window is insensitive and no
dialog opens over it (the Mac's S8.3 rule; `recorder_open` in the window's state).

### 3.2 The other windows

| Window | Mac | Linux | Default / minimum | Keyed by |
| --- | --- | --- | --- | --- |
| Conversation | `WindowGroup(for: chatID)` 720 × 640, min 420 × 320 | `adw::ApplicationWindow` with the same conversation page | 720 × 640 / 420 × 320 | chat id — asking twice presents the existing one |
| Board | `Window` 900 × 620, min 480 × 360 | `adw::ApplicationWindow` | 900 × 620 / 480 × 360 | one — it shows the family shown (P4 correction: `app.open-board(x)` switches the session to `x` first, every board write goes to the family shown through `ApiClient`'s scope, and the Mac's board window follows `currentFamilyKey` the same way) |
| Attachment viewer | `WindowGroup(for: album)` 900 × 700, min 480 × 360 | `adw::ApplicationWindow` | 900 × 700 / 480 × 360 | (message, index) — paging does not move the key |
| Call | `Window` 780 × 560, min 480 × 400; never closed by the person — closing does not hang up | `adw::ApplicationWindow`; `close-request` hides it, the call goes on, Return to Call brings it back | 780 × 560 / 480 × 400 | one |
| Settings | `Settings` scene 460 × 530, resizable | `adw::Window` of `adw::PreferencesPage`s (not the deprecated `PreferencesWindow`; not a `PreferencesDialog`, because the Mac made it a window that does not cover the chats) | 460 × 530 / 360 × 420 | one |
| Family | sheet 520 × 520 | `adw::Dialog` (content 520 × 520) over the window it was asked from | — | — |
| Thread, open polls, poll composer, note editor, reports, consent, new group, group info, sticker pack | sheets | `adw::Dialog`s | the Mac's frames | — |
| Alerts | `.alert` | `adw::AlertDialog`, presented with `present()` and the `response` signal — **never `choose_future`** (md §7.8: libadwaita-rs 0.9 aborts the process when a pending choice is dropped) | — | — |

All windows are `gtk::Application` windows, so they share the process, the cache and the socket (the
Mac's `windowContents` reason). Sizes are remembered for the main window only (as the Mac restores it);
the others open at their defaults.

### 3.3 Closing, hiding and quitting

| Route | Rule (fc-logic) | Linux |
| --- | --- | --- |
| Main window close button, Ctrl+W | `TrayRules::close_hides(keeps_running, tray_available, quitting)` (the Mac's `MenuBarRules.closeHides`) | `connect_close_request` → `Propagation::Stop` + `set_visible(false)` when it hides; otherwise the close proceeds and the app quits (it is the last window that matters: §3.8) |
| Hidden | the window "went away" for recording purposes (`NotSent`, `RecordingEnd::WindowHidden`) | the recorder parks; playback pauses; presence released |
| A real close over a video message in REVIEW or recording | asks "Delete video message?"; Keep cancels | the close request waits for the `adw::AlertDialog` |
| Quit (primary menu, tray menu, Ctrl+Q) | — | §3.8 |

`gio::Application::hold()` is taken while the main window is hidden and the tray icon exists, so GTK
does not end `run()` with no visible window.

### 3.4 Keyboard

| Chord | Mac | Windows | Linux (`set_accels_for_action`) |
| --- | --- | --- | --- |
| Settings | ⌘, | — | **Ctrl+,** (`app.show-settings`) |
| Refresh (resync) | ⌘R | — | **Ctrl+R** (`win.refresh`) |
| Record voice message | ⌥⌘R | Ctrl+Shift+R | **Ctrl+Shift+R** (`win.record-voice`) — a shortcut never sends; pressed during a recording it stops into review |
| Record video message | no shortcut | no shortcut | none — no shortcut ever opens the camera |
| Close window | ⌘W | — | Ctrl+W |
| Quit | ⌘Q | — | Ctrl+Q |
| Keyboard shortcuts | — | — | Ctrl+? (`adw::ShortcutsDialog`) |
| Primary menu | — | — | F10 (libadwaita's) |
| Bring forward / put away | ⌃⌥⌘F | Ctrl+Alt+Shift+F | Ctrl+Alt+Shift+F through the portal (§19) |

Escape is never an accelerator (it belongs to input methods and popovers, md §2.2): the composer, the
recorder and dialogs take it in a bubble-phase `gtk::EventControllerKey`.

### 3.5 What Wayland changes

| Area | Consequence |
| --- | --- |
| Position | none restored, no cascade (all windows) |
| Focus | needs an activation token (§2.2); the hand-off and notification routes carry one |
| Minimised | GTK 4.12+'s `Window::is_suspended()` / `notify::suspended` is the only signal — it is what stops a recording (§10.8) (unverified — real desktop) |
| Global keys | only through the GlobalShortcuts portal (§19) |
| Screen capture of a window | none needed |
| Fractional scaling | `gdk::Surface::scale()` (f64); bitmaps are made at `ceil(scale)` (§25) |

### 3.6 Session gate

`fc_logic::app_session::AppSession` decides which page the main window shows; the shell only swaps the
content of the `adw::ToolbarView` (an `adw::ViewStack` with one page per `Gate`). The Mac's `RootView`
order: booting → needs server → needs auth → needs family → pending approval → active.

| Gate | Mac view | Windows view | Linux page |
| --- | --- | --- | --- |
| Booting | spinner | — | the app's icon and an `adw::Spinner` in the gate's form (where the next page's icon stands) |
| Needs server | `ServerSetupView` | `ServerView` | `adw::Clamp` (360) + `adw::PreferencesGroup` with an `adw::EntryRow` (server address; `fc_logic::doorway::ServerCheck`), the default server offered (`DefaultServer`) |
| Needs auth | `AuthView` | `SignInView` | username `adw::EntryRow`, password `adw::PasswordEntryRow`, Sign In / Create Account (`SignInForm`); refusals from `DoorSentences` |
| Needs family | `FamilyGateView` | `DoorView` | Create a family / Join with a code / invitations (`FamilyDoor`, `Families`) |
| Pending approval | `PendingApprovalView` | `PendingView` | `adw::StatusPage` + Cancel |
| Offline at launch | — | `OfflineView` ("a flaky network is not a sign-out") | `adw::StatusPage` + Try Again |
| Active | `MacChatView` | `ChatsView` | the split view |

A stored token is not a session: `GET /me` decides; an expired one lands where a REST 401 lands, once
(`LiveConnection`). The socket runs only while signed in **and** in a family. A launch that cannot
reach the server but holds chats opens them from the last `/me` this account was answered (kept in
its cache's `meta`, wiped with it) — the Mac's `openOffline`; the Offline page is for a launch with
nothing held, and it tries again by itself when the network comes back (2026-10-30).

### 3.7 Start-up order

`startup`: paths → settings → catalogue (§23) → the Secret Service read (§20; async, the gate stays
Booting until it answers) → cache → `LiveConnection` → services (tray, badge, notifier, hot key) →
CSS (§25) → actions and accels → the main window built, **not presented**. `activate` presents it
unless `StartupSetting::starts_hidden(from_autostart, tray_available)` — which refuses to hide a window
when there is no icon to come back from. As the Mac's `LaunchWindowBackstop`: if no icon has appeared
two seconds after a hidden start (the extension is off, the watcher never answered), the window is
presented after all.

### 3.8 Quitting

Quit — from the primary menu, the tray menu or Ctrl+Q — asks nothing except the Mac's one question
(a video message being recorded or in review). Then: a voice recording running is stopped and parked
(`ParkedRecordings`, written before the process may end; a disk that refuses keeps it in memory and
is tried again here — `win/README.md`); a call is hung up (`call_end` `hangup`/`cancel`); the socket is
closed; the tray icon withdrawn; `app.quit()`. `query-end` at logout does the same synchronously
(no awaits: the parked rows are synchronous file writes).

---

## 4. The chat list (`chats::ChatList`)

| | Mac | Windows | Linux |
| --- | --- | --- | --- |
| Widget | `List(.sidebar)` of `MacChatRow` | a ListView | `gtk::ListBox` with the `.navigation-sidebar` style class in a `gtk::ScrolledWindow` — tens of rows, every row a real widget |
| Row | 34-pt avatar, title (semibold when unread; two lines for the family and groups), relative time, preview, unread capsule | the same, from `ChatListModel` | `ChatRow` widget: `adw::Avatar` 34 (initials; the picture from the avatar cache, keyed by user **and version**), title `gtk::Label` (`.heading` when unread, 2 lines for family/group), time (`RowTimeText`), preview (blocked sender → "Hidden — blocked member", the count untouched), unread pill |
| Order and words | `@Query` by pin rank, date | `ChatListModel` | `fc_logic::chat_list::ChatListModel` — the shell draws its rows and nothing else |
| Open in a window | context menu "Open in New Window" — never a double-click (it stole the List's own clicks) | — | secondary click / long press / Shift+F10 / Menu key → `gtk::PopoverMenu` "Open in New Window" (§3.2); double-click does nothing |
| Updating | — | rows changed in place and a chat opened **a beat later** (the list emptied and refilled inside its own selection handler crashed WinUI) | rows updated in place by id (`ListBox` children reused; `invalidate_sort`), never removed and re-added on a change; a selection opens its chat on `glib::idle_add_local_once` |
| Family switch | the open chat leaves the list → its first chat (the family chat) is selected | — | same |

Keyboard: Up/Down move the selection and Enter opens, as a `ListBox` does; the list is the first focus
stop of the window.

---

## 5. The conversation (`conversation::ConversationPage`)

One page per open chat — in the main window's content pane or in its own window (§3.2) — owning the
chat's `fc_logic::conversation::ConversationModel` (paging back, the read marker, typing), its composer
(§6) and its recorder layer (§10.6). Rebuilt when the selection moves, so a draft, a reply or a scroll
position never leaks into the next chat (the Mac's `.id(selectedChatID)`); the draft itself survives in
`ComposerDrafts`.

### 5.1 The timeline

- **Widget**: a vertical `gtk::Box` of row widgets inside a `gtk::ScrolledWindow` (no horizontal bar),
  centred in a column at most `ConversationColumn::MAX_COLUMN` (1024) wide with `PANE_INSET` (16) clear
  either side; a bubble is at most `BUBBLE_MAX` (560), leaving `BUBBLE_SPACER` (48) on the side it does
  not lean to (fc-logic `composer_row`, Windows' numbers, which are the Mac's).
- **Bounded and not lazy**: the rows built are `ConversationModel`'s window — its newest page (50),
  widened a page at a time when the reader reaches the top (from the cache first, the server only past
  it), and up to `JUMP_LIMIT` (500) when a quote is followed; no row is dropped below the viewport
  (corrected by P1: the model's own window is the bound, as on Windows). A `gtk::ListView` was rejected: its rows are
  recycled (an inline video or a rich drawing would be torn down and rebuilt on every scroll) and its
  height estimates move the anchor when a giant row materialises — the same failure the Apple clients
  had with lazy stacks.
- **Anchoring**: pinned to the bottom while the reader is at the newest (`vadjustment` upper changes →
  value = upper − page size); a prepend keeps the first visible row where it was (value += height
  added). (Corrected at integration, measured:) GtkViewport sends `notify::upper`/`page-size` from
  inside its own allocation, after it has placed its child, and drops the allocation the new value
  asks for — the adjustment then says the newest while the rows stay drawn at the old offset. So
  every move made from those notifications also re-queues the viewport's allocation from an idle
  (`Timeline::move_view`); the self-test compares where the newest row is DRAWN with the view's
  bottom (`chat.conversation_drawn_at_the_newest`, `chat.dark_redraw_stays_at_the_newest`). "Jump to newest" pill when scrolled up with
  unread below (the Mac's `UnreadAnchor`), the unread divider (`UnreadDivider`) where reading stopped.
- **Read marker and presence**: a message is read only when its chat's window is active
  (`gtk::Window::is_active`), the page is the one shown, and the newest message is on screen — the
  Mac's three facts (`ChatPresence`); presence is ONE value app-wide and the frontmost window claims it
  and releases only its own claim (two conversation windows).
- **Rows** come from `fc_logic::timeline::Timeline` / `conversation_rows` (date separators, the
  "Sending…", "Try Again · Delete" rows under the newest, call records, events, the assistant's
  streaming answer).

### 5.2 Bubbles (`conversation::Bubble`)

| Part | Rule (fc-logic) | GTK |
| --- | --- | --- |
| Body | `BubbleBody` runs: text, links, mentions, markdown blocks, tables, rich blocks | a `gtk::Label` per text block with Pango markup built by the shell from the runs (`use_markup`, `selectable`, `wrap`, `WrapMode::WordChar`; links as `<a href>` handled by `activate-link` → `gtk::UriLauncher`; mentions as links to `fc-member:<id>` → the member card), a `gtk::Grid` for a table, a `gtk::Picture` for each rich block (§9) |
| Own balloon | solid accent, white ink (Mac; `Palette.Ink` on Windows — the dark theme's pale accent paired with black ink was wrong) | CSS class `fc-own-bubble` with `--accent-bg-color` and white text in both themes |
| Others' balloon | the Mac's `appSecondaryFill`: a quiet neutral fill, the window's ink | `fc-bubble`: the ink at 8 % (light) / 10 % (dark) over the ground, outlined in high contrast |
| Shape | `TimelineRow::run_start` / `run_end`: corners tighten where a balloon meets its run mates (the Mac's 14/4, Windows' `BalloonCorners`); one to four emoji (`fc_text::emoji`'s ladder) and a message of nothing but pictures stand bare; a sticker and a circle have no balloon at all | `bubble::joins` over the rows: `fc-chat-joined-above` / `-below` (16 px round, 5 px joined) only between two filled balloons of one run with nothing between them — no unread divider, chips, time or thread chip; `fc-chat-bare`; `fc-chat-emoji-1…4` sized from the ladder in proportion to the body |
| Time and its place | under the LAST balloon of a run, and under an edited or a hidden row (the Mac's `showsTimestamp`, Windows' `RunEnd`); the thread's "%lld replies" chip under it | on the window's ground beside the balloon's edge; a balloon inside a run shows its time as a tooltip |
| Reply quote | `Quotes` / `Excerpt` (per scalar); a reply to a blocked member "Replying to a hidden message" | a clickable box above the body that scrolls to the quoted row |
| Reactions | `fc_text::reactions` chips; blocked members out of the names, not the count; the tap is decided here (held → DELETE, else PUT) | a `gtk::FlowBox` of chip buttons under the bubble; a chip's tooltip lists who; the reader's own chip's popover lists them with a remove control |
| Heart | double click on a bubble | `gtk::GestureClick` with `n_press == 2` → ❤️ (Windows' rule: "a double click is the heart") |
| Menu | `MessageMenu` (Reply, Reply in Thread, Copy, Edit, Delete, Show text, Playback speed, Save…, Open Full Screen, Report…, Safety; never Copy/Edit on a voice or video message) with a reactions row | secondary click / long press / Shift+F10 / Menu key → `gtk::PopoverMenu::from_model` whose first item is a custom child (`add_child`, "custom" attribute) holding the reaction row; the rest are `win.message-*` actions with the message id as target |
| Edit | trimmed; unchanged sends nothing; empty refused; only the reader's own words | the composer borrows the field (§6), Esc gives the draft back |
| Delete | the protocol deletes no message: Delete gives up a send that was REFUSED and never reached the server (`ConversationModel::discard`), asking nothing — the Mac's `deleteLocalMessage`, Windows' `Discard` (corrected by P1) | "Try Again · Delete" under the refused row; Delete is `win.message-delete` with the row's negative token as its `x` target |
| Time and ticks | the locale's own short time (the fix of `composer-and-wide-layout` §3 row 6), "seen" in a direct chat only (`PeerReads`) | formatted by fc-logic in the reader's locale (never `strftime`'s `%X`, which follows `LC_TIME` alone) |
| Link preview | `LinkPreviews` (none for a lookup answer or an assistant answer still streaming) | a card under the body, fetched by `LinkPreviewFetcher` over the same `soup::Session` with no credentials, only when "Link Previews" is on |
| Map | Mac: MapKit. Windows: OpenStreetMap tiles (`MapView`, `MapTiles`: the app names itself, tiles kept a month, a failed tile not re-asked for five minutes, attribution always drawn) | **Windows' answer** — Linux has no system map either: `MapView` places the tiles, `MapTiles` fetches over soup into `$XDG_CACHE_HOME/family-connect/tiles`, drawn in a `gtk::Picture` stack with the pin; "Map Previews" off → pin, name and link only. A click opens `https://www.openstreetmap.org/?mlat=…&mlon=…` with `gtk::UriLauncher` |
| Polls | `PollCard`: question, options with bars, vote / change vote, closed state | §5.4 |

**Selection and copy**: labels are `selectable`; Ctrl+C copies a selection; "Copy" in the menu copies the
whole body (`gdk::Clipboard::set_text`).

**Accessibility**: each bubble is a `gtk::Box` with `AccessibleRole::Group` and a label "<sender>,
<time>: <excerpt>" from fc-logic (Windows' `NamedGroup`); its controls are reachable by Tab.

### 5.3 Threads

Mac: `ThreadView` as a **sheet** over the chat (`MacConversationView.swift:671`). Windows and the web:
a 400-wide panel beside the conversation. **Linux follows the Mac**: an `adw::Dialog` (content 520 ×
640, `follows-content-size` off, resizable by the window) showing the root and its replies through
`fc_logic::threads::ThreadModel`, with its own composer — the main capsule and Send disc, Send
insensitive while blank, the sticker glyph in the field, no paperclip and no microphone (the
`composer-and-wide-layout` §2 rule). Esc with a draft clears focus first and closes only on a second
Esc (the web's fix, §3 of that document). On a narrow window libadwaita turns the dialog into a bottom
sheet by itself (unverified — real desktop).

### 5.4 Polls

The poll composer (`PollDraft`, `PollText`) and "Open polls" (`OpenPollsModel`) are `adw::Dialog`s, the
Mac's sheets. A poll's card votes in place (`PollVoting`). Questions and options are prose fields
(§7: capitals in the question, emoji typing in both).

### 5.5 Typing, presence lines, the assistant

`TypingRoster` (five seconds, the reader and blocked members ignored, ordered by member) → the content
header's subtitle. The assistant's chat: consent first (`AssistantConsent`, `ConsentedAsk`), the
streaming answer drawn as it grows, lookups' sources footer as markdown (`SourcesFooter`), no preview
card (`win/README.md`, "Looking things up").

---

## 6. The composer (`composer::Composer`)

### 6.1 Shape

The approved row of `docs/composer-and-wide-layout-2026-10-20.md`: a capsule holding the paperclip
(menu), the field, the glyphs inside it (sticker, emoji, `@ai` / 🎨 where `AttachMenu` offers them, the
video button in an empty field when `RoundVideoRules::recording_enabled`; the sticker and video glyphs
are the app's own, §25), and beside it the **Send
slot** — one fixed 40-px accent disc in a 44-px target (`ComposerButton`: Send, Save, the microphone,
or while recording the Send arrow / Stop), so the row never jumps. Stacked above it, in this order:
notices (`mediaNotice`, picture notices), the reply / edit banner, staged attachments (chips with ✕),
not-sent voice rows, the `@` suggestion list, the sticker panel.

| Part | GTK |
| --- | --- |
| Field | `gtk::TextView` (`WrapMode::WordChar`, `accepts_tab(false)` so Tab leaves it, `top/bottom_margin` 6) in a `gtk::ScrolledWindow` with `propagate_natural_height(true)` and `max_content_height` = 8 lines (the Mac's `lineLimit(1...8)`); placeholder "Message" / "Reply in thread" as an overlay label shown while the buffer is empty |
| Paperclip | `gtk::MenuButton` with `AttachMenu`'s items: Photo or Video…, File…, Paste, Record Voice Message, Record Video Message, Ask the Assistant, Location, Poll — and Show the Assistant a Photo… in the assistant's chat (`AttachItem`) |
| Slot | a 44 × 44 `gtk::Button` (flat, transparent) whose child is the visible 40-px accent disc (`.circular`, `.suggested-action` look) — the target reaches past what is drawn; icon per state; tooltip and accessible label in **every** state ("Record a voice message (Ctrl+Shift+R)", "Send voice message", "Stop recording"…, the Mac's S8.3) |
| Slot menu | secondary click / Shift+F10 / Menu key on the microphone → "Record Voice Message" (and "Record Video Message"); a long press with a mouse is a click, never the menu (holding withdrawn everywhere on 2026-10-06) |
| Limit | 4000 characters (`ComposerText.bodyLimit`, counted as the server counts) clamped on every change with the paste doors' sentence |

### 6.2 Keys (`NewLineKeys`, `ComposerButton`)

A capture-phase `gtk::EventControllerKey` on the field:

- **Enter / KP_Enter alone** → the slot, and only on the rows where `takes_return(slot, can_send)` says so
  (Send or Save); in an empty, idle field it does nothing (never a microphone).
- **Shift, Ctrl or Alt + Enter** → a new line: `buffer.insert_at_cursor("\n")` replacing the selection,
  one undo step (`NewLineKeys::enter_makes_line(shift, ctrl, alt)` — Windows' three; the Mac's ⌥⌘⌃⇧).
  Never U+2028 (the Mac measured AppKit's ⌃↩ producing one).
- **While an input method composes** (the buffer's `preedit-changed` gave a non-empty preedit), Enter
  is the input method's: the controller returns `Propagation::Proceed`.
- **Escape** (bubble phase): stop a recording into review; else cancel an edit (the draft comes back);
  else drop the reply; else nothing.
- **Up** in an empty field: not on the Mac, not added.
- A key held down is one press: `key-pressed` repeats are ignored for Enter and for Ctrl+Shift+R.
- **The 600-ms guard** (`S1.1`): after the slot's own activation changed it, a second activation — a
  click or Enter — is ignored, and so is a press that went down meanwhile however late it lifts;
  changes caused by typing, pasting or staging are never guarded.

### 6.3 Paste

Mac: ⌘V in the field pastes words; ⌘V anywhere else in the window, and the paperclip's Paste, attach
what the clipboard holds — the field editor is never robbed of a text paste. Linux:

- The field's own Ctrl+V pastes text as GTK does.
- The field's `paste-clipboard` signal is intercepted **only** when the clipboard's
  `gdk::ContentFormats` hold no text but a file list (`gdk::FileList`), a texture or an image MIME type:
  then `stop_signal_emission_by_name("paste-clipboard")` and the attach path. The type preference is
  `fc_core::text::paste_rules::PREFERENCE` (GIF and WebP before the flattened PNG beside them, so an
  animation is not pasted as a still of itself), read with `gdk::Clipboard::read_future(&[mime], Priority::DEFAULT)`.
- Ctrl+V with the focus elsewhere in the conversation (the timeline) runs the same attach path through
  a `win.paste-attachment` action whose accelerator is only active while the field is not focused.
- The paperclip's Paste goes through the same rule and answers "There's nothing to paste." only when
  nothing — not even words — is there (the Mac's fix).

### 6.4 Drag and drop

A `gtk::DropTarget` for `gdk::FileList` (`DragAction::COPY`) on the **whole conversation page** — the
composer is one line tall and a strip that thin is a target nobody hits (Mac). While a drag hovers, an
accent outline (an overlay child with `can_target(false)`, the Mac's `RoundedRectangle` stroke).
Selected text dragged from another app offers no file list and is not accepted (it springs back, as on
the Mac); a folder is refused with `DroppedAttachment::decide`'s sentence. Files are staged, never sent:
the caption is typed, then Send. Ten attachments at most per message (the composer says so).

### 6.5 Staging, sending, the outbox

The shell calls fc-logic: `ComposerStaging` (what a picked file becomes — `MediaPlan`, §10.7 — with the
"Preparing…" bar and its Cancel), `MediaOutbox` (rows written before a byte moves), `SendPipeline`
(socket, ack deadline, REST fallback). A sticker is never prepared (`win/README.md`). The Attach dialog
is `gtk::FileDialog::open_multiple_future` with no filter ("Attach" as the accept label) — the protocol's
file kind accepts any type; the assistant's "show a picture" uses an images-only filter.

### 6.6 Drafts

`ComposerDrafts` per chat, kept across chat switches — IN MEMORY and WORDS ONLY, as the Mac's
`ComposerDrafts` keeps them ("it survives navigation, not the process"; staged items, a reply and an
edit are let go) and as fc-logic's `ComposerDrafts` says (corrected by P1: this line said "and launches;
the reply and the edit target are part of the draft"). A new server forgets them. A voice recording
interrupted by leaving the chat parks with the field's words as its caption (`NotSent`).

---

## 7. Typing aids in text fields

Every multi-line prose field — the composer, the edit, the thread reply, a board note, a poll's question,
a report's note — is a `gtk::TextView`; the single-line prose fields (a list line, a poll option) are
`gtk::Text`/`gtk::Entry`. Username, server, codes, search, a sticker's label and a place are never
touched. The shell feeds fc-logic and applies its answers; it decides nothing.

| Aid | Mac | Windows | Linux |
| --- | --- | --- | --- |
| Capital at a sentence start (`Capitals`, `CapitalField`, multi-line fields) | only while macOS "Capitalise words automatically" is on | always on — no Windows keyboard does it | **always on**, as Windows: GNOME has no such switch |
| Emoji as you type (`EmojiTyping`, every prose field) | always | always | always |
| Taking it back | Backspace right after | Backspace right after | Backspace right after, nothing selected → what was typed |

**Mechanics (md §3.3, proven there):** the buffer's `begin-user-action` snapshots the text and the
selection; `end-user-action` judges the edit with fc-logic (`Capitals::typed_character`,
`EmojiTyping::after_key`); the replacement is applied on a `glib::idle_add_local_full(Priority::HIGH)`
as its **own** user action — so Ctrl+Z straight after gives the typed character back, as on every port.
A `gtk::Entry` does the same through `gtk::Editable`'s `insert-text`/`changed` with the replacement on
the same high-priority idle.

**Never judged**: a commit from an input method (`preedit-changed` non-empty before the commit, or a
commit not preceded by a key press), a paste (`paste-clipboard` sets a flag until `paste-done`), a chord
with Ctrl or Super, a text set by the program. Nothing typed is ever logged.

**Caret units** (carry-forward, w1 board): every `fc-text` rule that takes a caret or a selection
counts **UTF-16 code units**; `gtk::TextIter::offset()` and `gtk::Editable::position()` count
**characters**. One helper module, `text::offsets` (pure, unit-tested with emoji and Cyrillic), converts
both ways at the shell's edge; nothing else in the shell does arithmetic on offsets.

**Mentions** (`ComposerMentions`, `MemberMentions`): in the family chat and groups, `@` + letters opens a
`gtk::Popover` list anchored to the caret (`TextView::iter_location` → `buffer_to_window_coords`);
Up/Down/Enter/Tab choose, Esc closes; accepting replaces the query run with the name (UTF-16 ranges
converted, §above). `@ai` and `/draw` from the field's glyphs where `AssistantMention` offers them.

**Emoji picker**: the field's own GTK binding (Ctrl+. and Ctrl+;) opens `gtk::EmojiChooser`, and the
composer's emoji glyph opens the same chooser anchored to it (`emoji-picked` → insert at the caret as a
user action). The Mac uses the system palette; the GTK chooser is GNOME's equivalent.

**Spelling**: none in release 1 — GTK's text view has no checker; `libspelling` exists (§1.7) but is not
built here. See §D.

---

## 8. Attachments and the viewer

### 8.1 Tiles in a bubble

`fc_logic::attachment_files` / `fc_text::media` decide; the shell draws.

- A photo or a video is a tile at **its own shape from metadata** (`MediaText::tile_size`), so a row never
  changes height when its picture lands; several are a grid of four with "+n". A tile draws the
  preview when `has_preview` says there is one, a photo's own bytes when not, and for a video with no
  poster nothing at all (`AttachmentFiles::source_for` — a tile never downloads a whole video).
- Bytes come once through `AttachmentCache` into `BlobFiles` (one file per key, whole or not at all);
  decoding is `gdk::Texture::from_bytes` for stills (glycin underneath: PNG, JPEG, WebP, GIF, HEIC,
  TIFF measured, §1.5) at near the drawn size (`PictureMemory::decode_size` → `gdk_pixbuf::Pixbuf::
  from_stream_at_scale` when the source is much larger than the tile).
- Pointer: `set_cursor_from_name(Some("pointer"))` on clickable tiles (the Mac's `MacCursor`; GTK's
  cursor is per widget, so there is no push/pop stack to leak).
- A file row (`AttachmentText`: name, size in the reader's decimal format, "Zero KB" and "%lld byte(s)"
  are Windows' own translated sentences, reused) → Save… (`gtk::FileDialog::save_future`, the name
  suggested) and Open (`gtk::FileLauncher` on the cached copy, after saving it to the cache's
  `opened/` folder under its real name).
- A voice note → §10.3; a received video message (a circle) → §10.4; a sticker → §8.4; a location →
  the map of §5.2.

### 8.2 The viewer window (`viewer::AttachmentViewer`)

Mac: a message's photos and videos at full size in their own window, one at a time, toolbar arrows and
the arrow keys, Save (a save panel) and Share; zoom by pinch, 1 = fit. Windows: a photo opens whole with
Save…; a video "in whatever plays it on this machine". **Linux follows the Mac:**

- `adw::ApplicationWindow` (§3.2) → `adw::ToolbarView`: header with ‹ › (insensitive at the ends),
  title "2 of 5" while there is somewhere to page, else the item's name (the Mac's rule), Save…
  (`document-save-symbolic`), Open Full Screen for a video (`gtk::Window::fullscreen`).
- A photo: `gtk::Picture` (`ContentFit::Contain`) in a `gtk::ScrolledWindow`; zoom with
  `gtk::GestureZoom` and Ctrl+scroll / Ctrl+plus/minus/0, scale kept between pinches (the Mac's fix:
  a pinch is relative to its start), a page turn starts the next picture at fit.
- A video: the player of §10.1 with its own controls bar (play/pause, a `gtk::Scale` scrubber, time,
  volume); streams from the server with authorised Range reads (§1.4).
- Keys: Left/Right page, Space plays/pauses, Escape leaves full screen — and, out of it, closes the
  window (`MediaAlbum::key`); F11 toggles full screen. *(P2: Open Full Screen is offered for every
  item, not only a video; Windows' zoom buttons (`Zoom out` / `Zoom in`) and a Copy of the photo onto
  the clipboard are in the header too.)*
- **Share**: there is no share sheet on Linux — omitted (§D).
- Save failure is said in an `adw::Toast` rather than silently (the Mac's `saveFailure`).

### 8.3 Avatars

`AvatarRules` (what a picture must be before it is sent — the shell decodes with gdk-pixbuf and
re-encodes a square JPEG via `gdk_pixbuf::Pixbuf::save_to_bufferv("jpeg", [("quality","85")])`) and
`AvatarCache` keyed by **(user, version)** — "no picture" is an answer kept for that version, a transient
failure is not. Drawn with `adw::Avatar::set_custom_image` (initials otherwise).

### 8.4 Stickers (the family's pack) and animated pictures

- The panel: a `gtk::Popover` from the sticker glyph with a `gtk::FlowBox` of 72-px cells, five
  across (`StickerLook::PANEL_CELL`, `PANEL_COLUMNS` — fc-logic's numbers; P2); "recently
  used" (16, per device, wiped at sign-out) first; one click sends (`PackSending`); in the assistant's
  chat through the consent question (`ConsentedAsk`); not offered on a server with no `max_pack_items`.
- Drawn with no balloon in one 160 box (`StickerLook::BOX`) fitted from metadata. A click shows it
  larger in a dialog of its own (not the photo viewer) with "Add to family stickers" where the pack
  does not hold it (`attachments::stickers::show`, the Mac's `StickerViewer`).
- **Animated where this machine can**: `gdk_pixbuf::PixbufAnimation` decodes animated GIF **and**
  animated WebP here (measured, §1.5 — Windows could only hope its WebP extension was installed). Its
  frames are advanced by one `glib::timeout_add_local` per view (the clock kept in
  `StickerAnimation`/`StickerShelf`, whose 96-MB budget per view still holds: past it the sticker drawn
  longest ago becomes its frame zero). Reduce Motion (§24) shows frame zero. *(P2, as built: the
  frames come from fc-logic instead — `animation_prep::layout`, `animation_preparing::frame_source`
  with a gdk-pixbuf `StillDecoder`, `animation_frames::sticker_frames::compose` — one path for GIF,
  WebP and APNG, which gdk-pixbuf does not animate (carry-forward, "Animations"). ONE timer moves
  every sticker, and only those ON SCREEN: a widget scrolled out of a `ScrolledWindow` stays mapped,
  so the timer also asks whether it lies inside its scrolled view's visible part.)*
- Adding to the pack (`PackPicking`): a WebP or PNG within the ceiling goes up as it is; any other still
  picture gdk-pixbuf decodes is fitted into 512 × 512 and written as PNG with alpha
  (`Pixbuf::save_to_bufferv("png")`); an animated picture that is not already an acceptable WebP is
  refused with "Animated stickers must be WebP." (`StickerImaging` on Windows).
- **Animations sent in a chat** (#96): an animated GIF/WebP/PNG becomes the protocol's silent looping
  MP4 — `AnimatedPictures` cuts frames, `AnimationFrames` composites them (fc-logic, pure), and the shell
  encodes the NV12 frames through `appsrc ! video/x-raw,format=NV12,… ! x264enc ! video/x-h264,
  profile=high ! h264parse ! mp4mux` — **without `faststart`**: `moov` must come LAST so
  `fc_logic::animation_mp4::exact_ticks` can rewrite the timing, and fc-logic moves the index to the
  front afterwards (`faststart::moov_first`) — on the 1/30-s grid `AnimationTiming` gives. The full
  contract is `fc_logic::animation_preparing::AnimationEncoder`'s docs (w2 anim). Received ones play
  muted and looping in the tile.

### 8.5 Location

"Location" in the paperclip (`AttachItem::Location`, "where I am, once"): the XDG **Location portal**
(`org.freedesktop.portal.Location.CreateSession` + `Start`, `accuracy` exact, one `LocationUpdated` then
`Close`) over GDBus. The item is offered only when the portal answers `version` (unverified — real
desktop: whether GeoClue on a stock desktop without GPS gives a fix at all).

### 8.6 Files shared into the app

Mac: a share extension; Windows: a share target (`ShareInbox`) → "Send to" (the family first, never the
assistant's chat) → staged in that chat's composer, nothing sent until Send. **Linux has no share sheet.**
What exists:

- Drag and drop onto a conversation (§6.4) and Paste (§6.3) — the everyday ways.
- `family-connect FILE…` (`HANDLES_OPEN`): the files are copied into
  `$XDG_DATA_HOME/family-connect/inbox/` by the process that received them (Windows' rule: the share
  belongs to the receiving process), then the window asks "Send to" (`ShareTargetPicker` as an
  `adw::Dialog`) and stages them. *(P2: staging is P1's composer, reached through
  `ChatRoutes::stage_files(chat_id, files)`, added with a default for that; the copies are deleted
  once its future says staging read them.)*
- **"Open With" — decided by nettrash 2026-10-29: register.** The `.desktop` file gets
  `Exec=family-connect %F` and `MimeType=` the kinds a family sends — pictures, videos, sound and PDF,
  never `application/octet-stream` or `*/*`:
  `image/jpeg;image/png;image/gif;image/webp;image/heic;image/heif;image/avif;image/tiff;image/bmp;video/mp4;video/quicktime;video/webm;video/x-matroska;video/x-msvideo;audio/mp4;audio/x-m4a;audio/mpeg;audio/wav;audio/x-wav;audio/ogg;audio/flac;audio/aac;application/pdf;`
  Ubuntu's own viewers (Loupe, Showtime, Rhythmbox, Papers) stay the defaults: the app only joins the
  "Open With" lists. Each file arrives through `HANDLES_OPEN` exactly like `family-connect FILE…`
  above — copied into the inbox, "Send to", staged, nothing sent until Send. The entry ships in the
  same build as the app's `connect_open` (P7 with P1), never before: a placeholder that cannot open
  files must not be in anybody's list. An "any file" route stays drag and drop, paste and the
  command line.

---

## 9. Rich content through WebKit (`rich::RichRenderer`)

What it draws: formulas (KaTeX), Mermaid, Graphviz and `plot` blocks in a bubble, as pictures — the
same `web/rich` folder every native client ships, driven by `fc-rich.js` (`VERSION =
'1;katex-0.17.0;mermaid-11.16.0;viz-3.24.0;plot-md-1.6'`). Windows' `RichRenderer` is the model; md's
off-screen renderer (md facts §6) is the mechanism.

### 9.1 The page and its origin

- One `webkit::WebContext` (`WebContext::default()`), one ephemeral `webkit::NetworkSession`
  (`NetworkSession::new_ephemeral()`), one `WebView` for the whole process.
- The scheme **`fc-rich`** registered **once** (`register_uri_scheme`), and through the security manager
  as **secure + CORS-enabled, never `local`** (md facts §3: `local` breaks module scripts; registrations
  leak within a process). `fc-rich://rich/rich.html` is `web/rich/rich.html` as shipped (its own
  `<meta>` CSP: `default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'
  'unsafe-inline'; font-src 'self'; img-src 'self' data:`), every other `fc-rich://rich/<path>` read from
  `/usr/share/family-connect/rich/` after md's containment check (no empty, `.`, `..`, `\`, `%`, NUL
  segment), anything else a 404; responses `Cache-Control: no-store`, MIME from the table md uses.
- `connect_decide_policy`: only `fc-rich://rich/rich.html` (exactly — `RichPage::is_page`; the page never
  navigates to a fragment of itself, P2) is allowed; every other
  navigation and every new window is ignored (`connect_create` → `None`); no downloads, no permissions
  (`connect_permission_request` → `deny`). Settings: JavaScript on, `javascript_can_open_windows_
  automatically(false)`, `javascript_can_access_clipboard(false)`, local storage, databases, WebAudio,
  WebRTC, media stream and fullscreen **off**, developer extras in debug builds only.

### 9.2 One job at a time

`fc_logic::rich_drawing::{RichJob, RichPage, RichCapture, RichFit, RichMemory}` decide; the shell:

1. `evaluate_javascript_future("void fcRich.drawAndPost(<id>, <job json>)")` — `void`, because the call
   answers a promise, which WebKit does not hand back as a value (measured: without it every job
   failed); and once per view, before the first job, a warm-up loads KaTeX and EVERY font in
   `document.fonts` (measured: a first formula measured before its fonts loaded was photographed
   cut off on the right; `fc-rich.js` itself is not changed) (P2);
2. the answer arrives through `UserContentManager::register_script_message_handler("fcRich")` —
   `fc-rich.js` already posts to `window.webkit.messageHandlers.fcRich` (its Apple path), which WebKitGTK
   provides (md facts §4);
3. `view.snapshot_future(SnapshotRegion::Visible, SnapshotOptions::TRANSPARENT_BACKGROUND)` → a
   `gdk::Texture` at the view's device scale, cropped to the rectangle the answer reported
   (`gdk::TextureDownloader` → `gdk::MemoryTexture`), kept with its CSS width, height and **baseline**
   (an inline formula sits on the text's baseline);
4. the next job waits for the photograph; fifteen seconds without an answer fails the job and throws
   the web view away (an engine stuck in a loop would fail every job after it).

A failure draws the block's source text instead (Windows' `RichDrawn::Failure`).

### 9.3 Where the view lives

md measured that a never-parented `WebView` has a 0 × 0 viewport and useless snapshots, and that an
**overlay child of a mapped window** with `opacity 0`, `can_target(false)`, `can_focus(false)`,
`halign/valign Start` and a size request gets its full viewport without affecting the window's minimum
(md facts §6). So the renderer is an invisible overlay child of the **main window's** root
`gtk::Overlay` — at opacity **1/255**, not 0 (md measured: at 0 GTK does not draw the view and the page
gets no animation frames; P2) — sized `RichPage::VIEWPORT_WIDTH` × `VIEWPORT_HEIGHT` (1280 × 800, Windows' numbers), its
longest capture side capped at `RichCapture::LONGEST_SIDE` (4096), and moves to whichever window is mapped when
the main window is hidden in the top bar (a hidden window is unmapped: the renderer re-parents to the
conversation or board window that asked; with no window mapped, jobs wait — nothing needs drawing).
*(P2, as built: jobs wait while the main window is unmapped; the re-parenting into another window
needs an overlay accessor on P1's and P4's windows — a request in P2's report.)* A drawing taller
than the viewport makes the view taller before it is photographed.

### 9.4 Kept

In memory (`RichMemory`: 256 entries, 32 Mpx) and on disk (`$XDG_CACHE_HOME/family-connect/rich/`, PNG
via `gdk::Texture::save_to_png_bytes`) under `RichJob::key`, whose version is read from the shipped
`fc-rich.js` — a drawing on disk is shown without WebKit ever starting. A failure is kept in memory only.
At a new scale (a move to a 2× monitor) the key includes the scale, so drawings are redrawn crisp.

### 9.5 The sandbox under AppArmor

Ubuntu 26.04 restricts unprivileged user namespaces (`kernel.apparmor_restrict_unprivileged_userns = 1`)
and WebKit's web processes start inside bubblewrap; a failed bwrap makes WebKit `g_error` and abort the
**whole app** (md facts §9). As md:

- The `.deb` ships `/etc/apparmor.d/family-connect` — `abi <abi/5.0>, include <tunables/global>
  profile family-connect /usr/bin/family-connect flags=(unconfined) { userns, mr @{exec_path},
  include if exists <local/family-connect> }` — with the `postinst`/`postrm` md's packaging §7 gives.
  The same exemption covers **glycin's** bubblewrap loaders (§1.5), which the app's process starts
  too (unverified — real desktop).
- **Preflight**: before the first `WebView` exists, run `bwrap --unshare-user --ro-bind / / true` once as a
  child (`gio::Subprocess`, awaited, ≈ 10 ms). On failure, no rich renderer for the session — blocks show
  their source, one `adw::Banner` (under the conversation's header: a top bar of the content pane's
  `adw::ToolbarView`, P2) says formulas and diagrams cannot be drawn and names the profile — and
  **the app keeps running**: unlike md, a chat client must not quit over its diagrams.
- Never `WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS` outside Docker; `scripts/dev.sh` sets it for the
  container only.
- The tarball has no profile: its README says the rich renderer needs one, and the preflight's banner
  covers it.

---

## 10. Media on GStreamer (`media::*`)

### 10.1 The player (`media::Player`)

One wrapper over `gstreamer-play` (`gst_play::Play` with `gst_play::PlaySignalAdapter::
with_main_context(&play, &glib::MainContext::default())`, so every state, position, end and error signal
lands on the main context — measured to exist in the crate, as are `Play::set_rate` and `Play::pipeline`).
**Trap:** the adapter's `connect_*` closures must still be `Send + 'static`, so they capture a
`glib::SendWeakRef` or push into a `futures_channel::mpsc`, never an `Rc`:

- video sink `gtk4paintablesink` (its `paintable` property → a `gtk::Picture`), wrapped in a
  `gtk::GraphicsOffload` where the picture is full-size (the viewer, a call) so Wayland can hand DMA-BUFs
  to the compositor (unverified — real desktop);
- **authorised streaming**: on the play pipeline's `source-setup` signal, a `souphttpsrc` gets
  `extra-headers` = `Authorization: Bearer <token>` and `user-agent`; seeking issues Range reads with the
  header (measured, §1.4) — the Mac's `AVURLAssetHTTPHeaderFieldsKey`. The token is never logged; the
  app never sets `GST_DEBUG` (souphttpsrc prints request headers at LOG level);
- **local playback** for anything already in the attachment cache (`file://`), which is how voice notes
  and circles play (fetched through the cache on the click, Windows' rule — and replayable offline);
- **rate** (voice speed 1×, 1.5×, 2×): `Play::set_rate`, with `audio-filter = scaletempo` on the pipeline
  so a faster voice keeps its pitch;
- **one thing plays at a time** (`PlaybackPauses`, `Playing`): a process-wide `NowPlaying` registry —
  a circle starting pauses a voice note and the viewer's video and the other way round; a recording, a
  call, a lock, sleep, a hidden window pause everything (§10.8).

### 10.2 Recording a voice note (`media::VoiceRecorder`)

| | Mac / Windows | Linux |
| --- | --- | --- |
| Pipeline | AVAudioRecorder / `MediaCapture` M4A | `pulsesrc` (the default source; PipeWire's Pulse server) `! audioconvert ! audioresample ! audio/x-raw,rate=48000,channels=1 ! tee name=t`, `t. ! queue ! avenc_aac bitrate=64000 ! aacparse ! mp4mux faststart=true ! filesink` (fallback `voaacenc bitrate=64000`), `t. ! queue ! level interval=50000000 post-messages=true ! fakesink` |
| Format | AAC-LC mono 44.1/48 kHz 64 000, M4A | measured (§1.4): AAC-LC mono 48 kHz 64 000, `ftyp moov mdat` |
| Live meter | Windows: a second listen-only reader | the `level` branch of the same pipeline (one microphone open, not two); peaks → `LiveLevels` |
| Stop | EOS, then the file is final | `send_event(gst::event::Eos::new())`, wait for `MessageView::Eos` on the bus, then `Null` |
| Mic pulled out / refused | `MediaCapture.Failed` | a bus `Error` from `pulsesrc` → `RecordingEnd::Failed` → parked (`NotSent`) |
| Permission | macOS asks; Windows `AppCapability` | none for a host app: PipeWire gives the default source; GNOME's privacy indicator shows the microphone in use (unverified — real desktop) |
| Screen reader | "Recording" said a second before the microphone opens | `gtk::Accessible::announce("Recording", Medium)`, then a 1-s wait before the microphone opens when a screen reader runs — the session's `org.a11y.Status.ScreenReaderEnabled` property (on `org.a11y.Bus`, `/org/a11y/bus`) read over GDBus and watched (Windows' `ScreenReader`) (unverified — real desktop) |

The rules are fc-logic's: `VoiceNotes` (one second floor, 5:00 limit, "30 seconds left" from 4:30),
`RecordingStart` (the interruption while the microphone opens lets it go once granted), `NotSent`,
`ParkedRecordings`, `ComposerButton` (the slot), `VoiceLook` (the row, the bubble), `DrawnCopies`. The
recording row takes the field's place in the input row (Delete, the red dot, the clock, the meter, Stop;
the meter scrolls in from the trailing edge, five static bars under Reduce Motion), and the slot is the
Send arrow — the Mac's `MacVoiceRecordingRow`.

**The sender's waveform** is measured from the finished file, not the meter (Windows' `VoiceShape`):
`filesrc ! decodebin3 ! audioconvert ! audio/x-raw,format=S16LE,channels=1 ! appsink`, the peak of each
tenth of a second → `fc_text::waveform::from_peaks` → 48 hex digits sent as `&waveform=` — the same bytes
always give the same digits.

### 10.3 Playing a voice note

The approved bubble (`VoiceLook`): a round accent play button, the sender's waveform (or the
placeholder) drawn by a small `gtk::DrawingArea` with the played bars lit (⌊position · bars /
duration⌋), the time in tabular digits (`font-feature-settings: "tnum"`), the speed chip (1×, 1.5×,
2×, kept in `settings.conf`), the unplayed dot (`PlayedVoiceStore`). **The waveform is the seek**: the
drawing area is also a `gtk::Scale`-like accessible slider (`AccessibleRole::Slider`, value/min/max
properties, Left/Right step) — click or drag seeks.

### 10.4 A received video message (a circle) — in place

Windows needed a frame server copying every frame into a bitmap because WinUI would not clip video to a
circle. GTK does (measured, §1.5), so the circle is simply:

```
RoundClip (a gtk::Widget subclass: measure() returns the diameter, 240 — or the RoundLook size)
  └ gtk::Overlay
      ├ gtk::Picture  (paintable = gtk4paintablesink's; ContentFit::Cover; the poster texture until the first frame)
      ├ play disc 48 · loading spinner · capsule (length + white dot) · expand control (top trailing, 28 in a 44 target)
      └ ring       (a gtk::DrawingArea drawing ONE accent ring OUTSIDE the edge — the clip must not cut it,
                    so the ring is drawn by the parent, not inside the clipped box)
CSS on RoundClip's inner box: border-radius: 9999px; overflow: hidden (set_overflow(Hidden))
```

`RoundClip` exists because the picture's natural size is the video's: in the spike a 240-request box
grew to the window (§1.5). Behaviour is `RoundPlayback` + `RoundInline` (rest → loading → playing ⇄
paused → rest, failed; a click a beat late because a double click is the heart; at the end back to the
poster; scrolled wholly out of view, chat left, window hidden → stop). The MP4 is fetched through the
attachment cache on the click only. `RoundFrames`/`RoundCrashGuard` (Windows' frame-server watchdogs)
are not needed and not wired (§D). With Reduce Motion the ring steps once a second and the disc appears
without fading.

### 10.5 The video message recorder (`media::VideoMessageRecorder`)

**The camera, through the portal.** `org.freedesktop.portal.Camera`: `IsCameraPresent` (property),
`AccessCamera` (Request/Response), then `OpenPipeWireRemote` → a Unix fd; the pipeline is
`pipewiresrc fd=<fd> target-object=<node serial>` — the node chosen from a `gst::DeviceMonitor` started
on that remote, or the first camera. Portal 1.21 made camera permission per app for host apps again
(§1.7); the permission is GNOME's to ask (unverified — real desktop). Fallback when the portal is absent
or refuses with "not supported": a `gst::DeviceMonitor` filtered on `Video/Source` on the default
PipeWire, then `v4l2src` (a host app may open `/dev/video*` through logind's uaccess) — a refusal by the
**person** is never bypassed: it ends in "Video messages can't be recorded without the camera."

**The pipeline.**

```
pipewiresrc fd=… ! videoconvert ! video/x-raw,format=I420
  ! tee name=v
v. ! queue leaky=downstream ! videoflip method=horizontal-flip ! gtk4paintablesink      (PREVIEW, mirrored)
v. ! queue ! valve name=rec drop=true ! videocrop (centre square) ! videoscale ! videorate
     ! video/x-raw,width=480,height=480,framerate=30/1,format=I420
     ! x264enc bitrate=500 key-int-max=60 speed-preset=veryfast tune=zerolatency
     ! video/x-h264,profile=high ! h264parse ! mux.
pulsesrc ! audioconvert ! audioresample ! audio/x-raw,rate=48000,channels=1 ! valve name=arec drop=true
     ! avenc_aac bitrate=64000 ! aacparse ! mux.
mp4mux name=mux faststart=true ! filesink location=$XDG_CACHE_HOME/family-connect/tmp/round-<uuid>.mp4
```

The recorded square is **not mirrored and upright** (the protocol's video message row): only the preview
branch flips. Recording opens both valves at once; Stop sends EOS through the recording branch only.
The profile was measured end to end with a test source (§1.4); the camera itself is (unverified — real
desktop). Whether the microphone shows "in use" during PREVIEW (Windows' T1 question) — the audio branch
starts at Record, so it should not.

**The recorder** is `RoundRecorder`'s three states over a layer covering the window's content (an
`adw::Bin` over the split view in the window's `gtk::Overlay`, not a dialog): PREVIEW (mirrored,
"Not recording", Record insensitive until the first frame, "Choose camera" with more than one, "Record a
voice message instead", a minute untouched closes it), RECORDING (the ring filling red, "10 seconds left"
at 50 s, stopped at 59.5 s into REVIEW, Delete asks from ten seconds), REVIEW (the clip as it will be
sent, Space plays/pauses, Delete, Retake, Send). Positions: `RecorderFrames::frame` (nothing overlaps at
any window size or text scale). The circle is the same `RoundClip`. A take that cannot be made round, or
too long for the server, goes as the regular video the planner makes of it; a square over
`max_round_video_bytes` as a 480 × 480 regular video ("Too big for a video message."). The poster: the
first frame, `appsink` → a JPEG via gdk-pixbuf. `RoundVideoRules::recording_enabled` stays the
switch; it is on for Linux since 2026-10-29, ahead of §C 25 (as Windows' was ahead of its trials).

### 10.6 The recorder layer and the window

While the recorder is up: header buttons insensitive, Ctrl+Shift+R starts nothing, no dialog opens, a
notification's route waits until it closes (the Mac's `consumePendingRoute` guard), a close or quit over
a take asks "Delete video message?" (§3.3).

### 10.7 Preparing picked media (`media::MediaPreparing`)

fc-logic decides everything: `MediaPlan` (from `fc_text::media_plan` — Rule A leaves it alone, the
targets, Rules B–E), `MediaEncoding` (**a Linux variant** — Appendix B), `TranscodeAttempts` (the order
of asking and what ends it), `FrameTurn` (a frame of the result against the same frame of the source),
`ContainerSound` (Rule E's track count from the container's own `trak`/`hdlr`), `Faststart`.

The shell's part:

1. **Read the source**: `gst_pbutils::Discoverer` for the displayed size, rotation tag, frame rate,
   bitrates, codecs; the sound tracks from `ContainerSound` (never from the decoder, which drops what it
   cannot read).
2. **Transcode** through a pipeline built from the plan: `filesrc ! decodebin3`, video
   `! videoconvert ! videoscale ! videorate ! video/x-raw,format=I420,width=…,height=…,framerate=… !
   x264enc bitrate=<kbit> key-int-max=<2 s> ! video/x-h264,profile=high ! h264parse`, audio (all
   enabled tracks mixed: `audiomixer`) `! audioconvert ! audioresample ! avenc_aac bitrate=<64000|128000>
   ! aacparse`, `mp4mux faststart=true`. The pinned raw format (§1.4's trap) is mandatory. A portrait
   source is encoded in its **stored** orientation with the rotation tag carried (`taginject` /
   the demuxer's own tag), as Windows does — `FrameTurn` checks the picture is right.
3. **Encoders in order** (`TranscodeAttempts`): `x264enc` → `openh264enc` (`profile=main`, its High is
   "constrained-high") → give up (Rule C: send the original). AAC: `avenc_aac` → `voaacenc`.
4. **Read back** with the Discoverer: sides, turn, codec and profile (High/Main 8-bit 4:2:0), audio kept
   or not, frame rate ≤ asked, audio rate as asked; a mismatch is a failed transcode.
5. **Cancel**: the "Preparing…" bar's Cancel or the window going sets the pipeline to `Null` and deletes
   the temporary; a cancel is not a failure and stages nothing.
6. Off the main context's critical path: GStreamer runs its own threads; the shell awaits the bus EOS
   from a `spawn_local` future. A transcode never blocks the UI.

HDR → SDR: GStreamer 1.28 has no general tone-mapper in these packages; a 10-bit or HDR source fails the
read-back's SDR check and goes as the original (Rule C) — as Windows does when Media Foundation clips.

### 10.8 Interruptions (`SessionWatch`)

Each of these stops and keeps a voice recording (`NotSent`), stops the video recorder into REVIEW, and
pauses playback (S4):

| Event | Mac | Windows | Linux |
| --- | --- | --- | --- |
| Session locks / screen saver | `com.apple.screenIsLocked` | `WTSRegisterSessionNotification` | `gtk::Application`'s `screensaver-active` notify (`register-session` on) |
| Sleep | `willSleepNotification` | `WM_POWERBROADCAST` | logind `org.freedesktop.login1.Manager.PrepareForSleep(true)` on the **system** bus (`gio::bus_get_future(BusType::System)` + `signal_subscribe`) |
| Window minimised | `willMiniaturize` | `AppWindow.Changed` | `notify::suspended` (GTK 4.12+) |
| Window hidden in the top bar / closed | `windowWentAway` | `RecordingEnd.WindowHidden` | §3.3 |
| Another chat, Board, Family, Settings | yes | yes | yes — the page's own `unmap`/route change |
| A call in any phase | yes | yes | `CallEngine` state ≠ idle |
| The microphone fails | — | `MediaCapture.Failed` | bus error |

(all unverified — real desktop; the container has no session lock, no logind, no compositor)

### 10.9 Keeping the screen on

`gtk::Application::inhibit(Some(&window), ApplicationInhibitFlags::IDLE, Some(reason))` while a voice or
video message records (Windows' `KeepAwake`, S1.7: an idle timer that locks the session would stop the
recording) and `IDLE | SUSPEND` during a call; `uninhibit(cookie)` when it ends, balanced. A cookie of 0
means no inhibitor was available (measured in the container, §1.6) and is logged once.

### 10.10 As built (P3, wave 3) — corrections to §10

Where the package had to differ from the text above; each is in P3's report.

1. **§10.1** — the player watches `gst_play::Play`'s own message bus with `Bus::add_watch_local` on the
   main context (`PlayMessage::parse`) instead of a `PlaySignalAdapter`: no closure crosses a thread,
   so nothing needs `Send`; only playbin's `source-setup` (the bearer) runs on a GStreamer thread, over
   an `Arc<Mutex<Option<String>>>`. Measured in Docker: a stream with the bearer plays, one without it
   fails on the fake server's 401.
2. **§10.4** — there is no `RoundClip` widget subclass: the workspace forbids `unsafe`, which glib's
   subclass macros expand to. The clip (`gtk::Overlay`, `fc-round`, `overflow: hidden`, size request
   = the diameter) sits in two `adw::Clamp`s, one per orientation (`maximum-size` =
   `tightening-threshold` = the diameter), which hold the natural size at the diameter. Measured: the
   corner of the playing circle renders with alpha 0, its centre 255 (`media.circle_is_clipped_round`).
   "Scrolled wholly out of view" is the ancestor `gtk::ScrolledWindow`'s `vadjustment` against
   `compute_bounds` (`round_inline::in_view`), besides `unmap`.
3. **§10.5** — the take is a SECOND pipeline (`appsrc do-timestamp=true is-live=true` → crop →
   480 × 480 → H.264 + `pulsesrc` AAC → `mp4mux faststart=true`) fed from an `appsink` branch of the
   camera's pipeline, not a `valve` in the same one: a take starts, stops (EOS on its own bus) and is
   torn down without touching the preview. The camera's mode is the one it negotiates (no
   `pick_mode` over enumerated caps); the take's rate is the camera's up to 30, never raised; the
   sound is 44.1 kHz (carry-forward); the poster is a frame at 0.5 s, else 0, else 2. The test camera
   is `videotestsrc … ! video/x-raw,… ! queue` (a caps filter must sit between two elements).
4. **§10.5/§10.6** — the layer stacks the status, the circle and the bar of controls (nothing can
   overlap by construction) and takes the circle's diameter from `recorder_frame::frame` over the
   measured parts, recomputed every 100 ms while open; the column layout of a pane shorter than 480
   is not drawn (the controls stay along the bottom).
5. **§10.6** — S0's close and quit (W.3) give the recorder no way to ask "Delete video message?" before
   the window goes: a quit closes the camera and discards a take. A request to S0, open now that
   `RECORDING_ENABLED` is on.
6. **§10.7** — an HDR source asked `to_sdr` is converted (`videoconvert gamma-mode=remap
   primaries-mode=full`, `colorimetry=bt709`) as fc-logic's `MediaPreparing::encode` contract says,
   and the read-back sends the original when the result still says HDR — fc-logic's answer to
   carry-forward's "pick one".
7. **§10.9** — the recording's `IDLE` inhibit is P3's (`media::keep_awake`, one hold shared by the
   voice and the video recorder); a call's `IDLE | SUSPEND` stays P5's/P6's — separate cookies.

---

## 11. The board (`board::BoardWindow`)

Mac: `MacBoardView` in its own resizable window (900 × 620, min 480 × 360) — "a sheet cannot be resized",
and a board cut at a fixed size hides notes. Windows: `BoardView` over the chats in the rail. **Linux: the
Mac's window**, one, showing the family shown (§3.2; P4 correction — it was "one per family").

- **The wall**: a `gtk::Fixed` the WALL's height (`BoardModel::wall_size`) in a vertical
  `gtk::ScrolledWindow` — horizontal policy `External`, so the cards never hold the window wide and a
  narrower window lays the wall out again at its own width — placing each card at its **fraction** of
  the wall (0–1, the note's TOP-LEFT corner, clamped inside, as the protocol and the phones say — the
  Mac's first version used the centre and was half a sticker off) with `Fixed::set_child_transform`
  (translated, then tilted by its id about its own centre), sized by its named size at the Mac's
  metrics (`fc_text::board::Size::frame`); every label in a card has a one-character natural width,
  so the card's own size decides and the words wrap or ellipsize inside it. Drawn in a stable order
  (by id) with recency as the z-order (`wall_text::layers`). (P4 correction: the design had a
  `gtk::Widget` subclass with a custom `gtk::LayoutManager`; a `Fixed` with child transforms is the
  same placement with no subclass.) A pinned photo is a `gtk::Picture` with `CONTENT_FIT = "contain"`
  → `gtk::ContentFit::Contain`. Faces: `note_look::font_family` lists Ubuntu's desktop fonts
  (carry-forward: the casual hand may fall back — §C).
- **Moving is anyone's; rewriting and deleting are the author's**: ONE `gtk::GestureDrag` on the wall
  (P4 correction: not one per note — a card's own coordinates move under the pointer while it travels)
  picks up the card under the pointer and drives its `NoteHand`; it offsets
  it while dragging and reports the fraction **on release only** (one PATCH carrying only x/y — never
  sixty writes a second); the local offset is held until the authoritative position arrives, so it never
  snaps back for a frame.
- **Header**: Add Note (Ctrl+N — the Mac's ⌘N), Add List, Add Event, Pin a Photo (`gtk::FileDialog`
  images only) — the Mac's four, with its tooltips.
- **A note's menu** (secondary click / long press / Shift+F10 / the Menu key): Edit…, Size and Delete
  (author; Delete asks) or "Written by someone else"; a list's lines to tick (anyone); for an event who
  is coming by name, Add to Calendar (`fc_text::calendar::one_event` writes the `.ics` — byte-exact, CRLF,
  folding at 75 octets per code point — into the app's temporary folder under the event's title and
  `gtk::FileLauncher` opens it) and "Are you coming?" (anyone); Reveal (a hidden note from a blocked
  member — the only item it offers). (P4 correction: the Mac's whole menu, not four items.)
- **The editor**: `NoteSheet` as an `adw::Dialog` — text (a `gtk::TextView` with the typing aids, §7;
  "n characters left"), colour, size, hand, the task list (`NoteDraft`), the event's date and time.
- **The badge** (`BoardBadge`): what is new to read since this device last showed the board — never the
  sync cursor; showing the window **in front of somebody** moves the marks (`marksAfterShowing`), a board
  window restored behind everything moves nothing (the Mac's presence rule).
- Accessibility: each note is a focusable `gtk::Button`-like widget labelled "Your note: …" / "Note
  from <author>: …" (the Mac's strings); arrow keys move a focused note one step (a keyboard way to do
  what the drag does — a Linux addition, §D).

---

## 12. The family and its owner's console (`family::FamilyDialog`)

Mac: one sheet (520 × 520) — members, invite code, join policy, member limit, join requests, reports,
family stickers, the assistant's family settings; owner-only controls **hidden**, not disabled. Windows:
`FamilyView` over the chats. **Linux: an `adw::Dialog`** (content 520 × 520) holding an
`adw::PreferencesPage`:

| Group | Rows (fc-logic: `FamilyModel`, `FamilyText`, `Families`, `Groups`) |
| --- | --- |
| Members | `adw::ActionRow` per live member (avatar, name, `@username`, Owner); a row's menu: Message (opens the direct chat in the main window and closes the dialog), Report…, Block/Unblock, Birthday…, and for the owner Reset Password…, Remove from Family (asks first, "Remove %@ from the family?" — P4 correction: the Mac removes on one menu click) |
| Invite code | the code (selectable), Copy (`gdk::Clipboard`), Rotate (asks: "Rotating invalidates the current code immediately."), Invite… (`Families` invitations) |
| Join policy | `adw::ComboRow`: Join immediately / Need approval / Nobody |
| Member limit | `adw::SpinRow` — the LOWER of the family's and the operator's numbers (`Seats`) |
| Join requests | Approve / Decline per request |
| Reports | the inbox, Mark as handled |
| Family Stickers | Manage… → the pack `adw::Dialog` |
| Assistant | the family switches; `ai_history_photos` and `ai_faces` never offered or drawn on while `ai_vision` is off; Looking things up (only where the server names providers); Daily greeting and its up-to-three places (`GreetingPlaces`, saved on leaving a field or Enter) |
| Statistics | Statistics… (rows never summed — the gap is the block) |

The "New Group" and "Group info" dialogs (`GroupModel`, `GroupRules`) follow the same pattern.

---

## 13. Settings (`settings::SettingsWindow`)

The Mac's Settings window, with Windows' rows where the Mac's are macOS's own and Linux needs its own
words (`linux/i18n/linux.json`, §23). An `adw::Window` (460 × 530) of `adw::PreferencesPage` →
`adw::PreferencesGroup`s; every switch is an `adw::SwitchRow`, saved to `settings.conf` at once.

| Group | Rows | Source |
| --- | --- | --- |
| Profile | photo (Add/Change/Remove), name, `@username`, Birthday (Add/Change…), Change Password… | Mac + Windows |
| Family | name, Leave Family (asks; the owner's wording from `LeaveContext`), New Family…, Join Another Family…, Statistics… | Mac + Windows |
| Assistant | consent (Review and Agree… / Stop Sending My Messages), Looking things up | Mac |
| Notifications | **Tell me when a message arrives** (on; this device's). Footnote: the Mac's "When a message arrives in a chat you are not reading, a notification says who wrote — never what they wrote.", and always §15.5's "If no notifications appear, check Settings ▸ Notifications ▸ Family Connect." (P4 correction: the Mac's wording over Windows'; the second line is said always, since the app can never tell — D15) | Mac #84 / Windows |
| Top Bar (Linux's "Menu Bar") | **Keep running in the top bar** (on) — "When you close the window, Family Connect stays in the top bar, so messages and calls still reach you. Quit it from its icon there."; when no top-bar host exists the row reads OFF (what a close then does), is insensitive and says so (§16.4) — the setting itself is kept (P4 correction: "reads off") | Mac #80 / Windows |
| | **Start when I log in** (off) — "Family Connect opens in the top bar when you log in, so a message or a call reaches you without opening it first."; the autostart states of §17 | Mac / Windows |
| | **Open with Ctrl+Alt+Shift+F** (on) — the trigger GNOME actually bound (§19); "Brings Family Connect forward from any app. Press it again to put it back in the top bar."; Change… where the portal offers it; when there is no portal: "Set a shortcut in your desktop's keyboard settings to run `family-connect --toggle`." | Mac / Windows |
| Privacy | Link Previews (on), Map Previews (on) — footnotes naming who sees the request (OpenStreetMap, as Windows) | Mac / Windows |
| Voice and video | "Playback speed" (1×/1.5×/2×, `adw::ToggleGroup`); "Camera" (`adw::ComboRow` of camera names, "Automatic" first) — the apps' existing words under the new group title (P4 correction) | Windows' `VoiceSpeedSetting`, `RoundVideoSetting` |
| Server | address (selectable), Log Out (asks "Log out?" — "Local messages are removed from this device.", plus "What hasn't been sent yet is lost." when the outbox holds something), Delete Account…, and the "isn't remembered" line when the token lives in memory | Mac |
| About | About Family Connect (`app.about`), and "Family Connect for Linux 1.3 (build)" (`AppVersionText`) under it — Privacy Policy and Support are in the Privacy group, as on the Mac (P4 correction) | Windows |

Settings that are this device's (notifications, top bar, start at login, hot key, previews, speed, camera)
survive sign-out; the account's do not (the Mac's rule).

---

## 14. Calls

### 14.1 The engine (`calls::GstCallMedia` implements fc-logic's `ICallMedia`)

fc-logic's `CallEngine` holds the state machine (`CallStage`, `CallState`, the protocol's `call_offer` /
`call_answer` / `call_ice` / `call_end`, candidate buffering before the remote description, the record,
`CallText`); the shell supplies the media. Its methods map onto `webrtcbin`:

| `ICallMedia` | `webrtcbin` / GStreamer |
| --- | --- |
| `open(video)` → `MediaGrant` | microphone: `pulsesrc` (default source) — none, refused, or failing → `MediaGrant::None` (the call ends `microphone_denied`); camera: the portal path of §10.5 — refused or absent → `Microphone` only (a camera is never a reason to miss a call) |
| `connect(servers, receive_video_only)` | `webrtcbin bundle-policy=max-bundle`; ICE from `GET /calls/ice`: the first STUN URL → `stun-server` (`stun://host:port`), each TURN URL → the `add-turn-server` action signal as `turn(s)://<user>:<credential>@host:port[?transport=…]` with **user and credential percent-encoded** — coturn's REST username is `<expiry>:<user_id>`, whose colon would otherwise end the user part (unverified — real desktop, or a coturn container in phase 5); a video call with no camera adds a `recvonly` video transceiver (`add-transceiver` with `WebRTCRTPTransceiverDirection::Recvonly`) |
| send audio | `pulsesrc ! audioconvert ! audioresample ! audio/x-raw,rate=48000 ! webrtcdsp name=dsp echo-cancel=true noise-suppression=true gain-control=true ! audioconvert ! opusenc ! rtpopuspay pt=111 ! application/x-rtp,media=audio,encoding-name=OPUS,payload=111 ! w.` (the canceller wants 10-ms buffers of raw audio at a rate it supports, hence the converters in front) |
| receive audio | `pad-added` → `rtpopusdepay ! opusdec ! audioconvert ! audioresample ! webrtcechoprobe ! audioconvert ! pulsesink` — the probe sits **after** the decoder and right before the speaker, so the canceller hears what the room hears |
| send video | camera `! videoconvert ! videoscale ! video/x-raw,width=640,height=480,framerate=30/1,format=I420 ! tee` → `vp8enc deadline=1 keyframe-max-dist=30 ! rtpvp8pay pt=96` and the self-view (`videoflip method=horizontal-flip ! gtk4paintablesink`). Every offer carries **VP8** (libwebrtc on every peer and Chromium accept it — the phase-0 spike); answering an offer whose video has no VP8, the branch is built with **H.264** instead (`x264enc tune=zerolatency speed-preset=ultrafast ! video/x-h264,profile=constrained-baseline ! rtph264pay config-interval=-1`) |
| receive video | `decodebin3 ! videoconvert ! gtk4paintablesink` (VP8 `vp8dec`, H.264 `avdec_h264`/`openh264dec`) |
| `create_offer` / `create_answer` | `emit_by_name("create-offer"/"create-answer", &[&None::<gst::Structure>, &promise])` → `set-local-description` → SDP text (`gst_sdp::SDPMessage::as_text`) |
| `set_remote(is_offer, sdp)` | `SDPMessage::parse_buffer` → `WebRTCSessionDescription::new` → `set-remote-description` |
| `add_candidate` | `add-ice-candidate(mline_index, candidate)` |
| `candidate_gathered` | `on-ice-candidate` → (via the main context) `call_ice` |
| `connected` / `failed` | `notify::ice-connection-state` (`Connected`/`Completed` → connected; `Failed` → failed; `Disconnected` is NOT failed — no client has a grace for it (w2 calls); see `fc_logic::calls`' contract for the dead-peer rule to measure in phase 5) and `notify::connection-state` |
| `set_muted` | `volume mute=true` before the encoder (the track keeps flowing, silent — never renegotiated) |
| `set_camera(on)` | a `valve drop=true` on the camera branch before the encoder: no frames, no renegotiation — "the far side simply sees the stream stop" (protocol "Video": cameras toggle, the kind does not) |
| `ring(tone)` / `stop_ringing` | §14.4 |
| `close()` | pipeline → `Null`, the camera fd closed, the portal session released — "the microphone light goes out" |

Every `webrtcbin` signal arrives on a streaming thread: the handler only sends a message to the main
context, where `CallEngine` runs — through a futures channel or `glib::idle_add_local_once`, never
`MainContext::invoke`, which runs at once when called on the main thread and would re-enter the
engine (`fc_logic::calls`' contract, w2 calls). The phase-0 spike already did
this pipeline against Chromium in both directions (Opus + VP8; the web client's audio call with a
`recvonly` video) — `docs/linux-client-2026-10-28.md`. What is new here and must be proven in phase 5:
`webrtcdsp` in the loop, an H.264-only offer, TURN credentials (coturn REST, percent-encoded),
Firefox's payload types (Opus 109 / VP8 120), the web client's audio call answered with no video
transceiver (inactive, not failed), dead-peer detection (does webrtcbin's libnice run consent
freshness? if not: no far-side audio RTP for 30 s once connected → failed), and the camera from the
portal. Not ICE restarts: the protocol has no frame for one and every client drops a second
`call_offer` as a duplicate (w2 calls).

### 14.2 Devices

The Mac's call has no device picker (the system default) — neither has Linux: `pulsesrc`/`pulsesink`
with no `device` follow PipeWire's default source and sink, and PipeWire moves a running stream when
the default changes unless the stream was targeted (unverified — real desktop: plugging a headset
mid-call). A picker (`gst::DeviceMonitor` `Audio/Source`, `Audio/Sink`) is a later addition (§D).

### 14.3 The Call window (`calls::CallWindow`)

Mac: its own window (780 × 560, min 480 × 400) opened when the manager leaves idle, closed when it
returns; closing it does not hang up; "Return to Call" in the main window's header. Windows could only
draw a card in a corner. **Linux: the Mac's window** (§3.2):

- Title: the person's name ("The window is the person" — the Mac's `navigationTitle(calls.peerName)`;
  "Someone" when the roster lacks them).
- Voice: the peer's avatar (96) and name, the stage line (`CallText`: Calling…, Ringing…, Connecting…,
  the running duration counted from `AnsweredAt`), the controls.
- Video: the far side's picture fills the window (`gtk::Picture` `ContentFit::Contain` in a
  `gtk::GraphicsOffload`), the self-view in a corner (mirrored, 160 wide, hidden while the camera is off);
  while the picture is up a pill in the top-left corner carries the name and the duration instead of
  the avatar block, and a video call's window is dark whatever the scheme (the Mac's `CallView`:
  black on black for all of connecting otherwise). A video call up without a camera of ours says the
  Mac's footnote ("Camera access is off — the call is voice-only for you.") and its Camera disc is
  insensitive.
- Controls (the Mac's `CallControlButton`s, 56-px circular buttons with labels under them): ringing
  in → **Decline** (`.destructive-action`) and **Accept** (camera icon on a video call); in a call →
  **Mute/Unmute**, **Camera** on/off (video calls), **Hang Up**. No Speaker (iOS only on the Mac too).
- Keys (corrected — the Mac's ring HAS two: `.keyboardShortcut(.cancelAction)` on Decline and
  `.defaultAction` on Accept): while it rings Accept holds the focus, so Enter answers (Windows'
  card does the same), and Escape declines (a bubble-phase `gtk::EventControllerKey` on the window,
  never an accelerator, §3.4). In a call no key ends anything (Tab and Space reach every
  button).
- Ended: the reason for a moment (`EndedReason` → `CallText`), then the window hides.
- Coming forward: the Mac opens the window on every change of the call's phase, so a window the person
  closed returns as the call moves on; Linux does that for every live stage, but an ending only
  updates a window that is up — a closed window is not brought back to say goodbye (and §14.6's
  Decline shows nothing).

### 14.4 Ringing

- **Tones**, synthesised exactly as Windows' `call.html` and the web make them: incoming — two 0.4-s
  beeps of 660 Hz at 0.08 (at 0 and 0.6 s) every 3 s; ringback — 425 Hz at 0.05, 1 s every 5 s; each beep
  with a 20-ms ramp. A pure function (`calls::tones::cycle(tone, rate) -> Vec<i16>`, unit-tested) fills
  one cycle; `appsrc` loops it into `pulsesink` (stream role `phone` via the `stream-properties`
  `media.role=Phone`, so PipeWire's policy can treat it as a ringtone).
- **The notification** (§15): "Incoming call" / "Incoming video call" over "<Name> is calling" (corrected
  to the Mac's `announceIncomingCall`, word for word), `Urgent`,
  category `call.incoming`, buttons **Decline** (`app.decline-call`) and **Answer** (`app.answer-call`),
  replaced by id `call-<call_id>` and withdrawn when the call leaves `Incoming`.
- **The window** opens too (the Mac opens the Call window on any non-idle stage). On Wayland it may not
  take focus (§2.2) — the notification is the reliable door.
- **The dock**: `urgent: true` on the launcher entry while it rings (§18).
- **Busy**: one call per person; nothing records during a call (refused with the reason), and the call
  buttons are insensitive while something records (`win/README.md`).

### 14.5 Keeping the line up

`inhibit(IDLE | SUSPEND)` from the answer to the end — Connecting and Talking, not while it rings
(corrected; the reason is the call's kind, "Voice call" / "Video call") (§10.9). The socket is never suspended while a call exists
(it never is on Linux: no background suspension). Lock and sleep do **not** end a call (an answered
call outlives socket blips — protocol "The sequence"); sleep will break the media and the far side's
`failed` backstop decides.

### 14.6 From a notification

Answer from the notification presents the Call window with the platform data's activation token (§2.2)
and answers; Decline sends `call_end` `decline` without showing anything.

---

## 15. Notifications (`desktop::Notifier`)

### 15.1 What raises one

`fc_logic::notifications::NotificationRules`: only **live** frames (a resync after a night asleep
moves the badge, not a hundred banners); a message unless its chat is being READ — shown in the
window in front, at its newest (`conversation::page::being_read`, the Mac's `ChatPresence`) — so a
message in another chat is announced with the app in front, as on the Mac (Windows muted everything
while its window was in front; changed 2026-10-30); a note that says something new unless the
board's window is in front; nothing from a blocked member, and nothing for the assistant's answer to
a blocked member's question; the "Tell me when a message arrives" switch. A chat's notification is
withdrawn once the chat is read — here, in any window, on another device, or by a resync
(`Shell::refresh_attention`).

### 15.2 What it says — who, never what

Title "<Family> — <Sender>" (or "<Family> — <Sender> mentioned you", or the sender alone in a direct
chat — `NotifyText`), body **"New message"** / "New note" — the body a server with
`include_message_body = false` would send. Never the text, never a picture, never the sender's avatar.
The icon is the app's (`gio::ThemedIcon` `me.nettrash.FamilyConnect`).

### 15.3 How

```rust
let n = gio::Notification::new(&title);
n.set_body(Some(&body));
n.set_priority(gio::NotificationPriority::High);          // Urgent for a ringing call
n.set_category(Some("im.received"));                      // "call.incoming" for a call
n.set_default_action_and_target_value("app.open-chat", Some(&chat_id.to_variant()));
app.send_notification(Some(&NotificationRules::chat_tag(chat_id)), &n);   // "chat-<id>": a second replaces the first
app.withdraw_notification(&tag);                          // when that chat is opened (Windows' rule)
```

GLib chooses the backend: GNOME Shell's `org.gtk.Notifications` on the desktop, the
`org.freedesktop.Notifications` server elsewhere (measured in the container, §1.6), the portal inside a
sandbox. **Correction (P5, measured by the `desktop` self-test's fake `org.gtk.Notifications`):** GLib
2.88's GNOME backend sends `title`, `body`, `icon`, `priority`, `default-action`,
`default-action-target` and `buttons` — **no category**; only the freedesktop backend (a `category`
hint) and the portal carry it. A ringing call's notification has `app.return-to-call` as its default
action (the Call window, which rings with the same two buttons); a wall of no known family is
`app.open-board(0)`. With the GNOME backend, actions come back as `org.freedesktop.Application.ActivateAction` on the
app's bus name — even when the app has quit, through D-Bus activation (§2.2). With the freedesktop
backend they come back only while the app runs (GLib's own `ActionInvoked` handling) (unverified — real
desktop).

### 15.4 Clicking one

`app.open-chat(x)`: the main window comes up (with the activation token), then the route is applied —
held while the gate is not `Active` or the video recorder is open (the Mac's `consumePendingRoute`).
`app.open-board(x)`: the family's board window (and the marks move). Routes of an owner (join requests,
reports) open the Family dialog; a member who somehow gets one stays put. Parameters are untrusted
(`ToastArguments::parse`).

### 15.5 When the desktop shows nothing

The Mac found Macs where notifications were off and every banner went into nothing (#84). GNOME's
per-app switch (Settings ▸ Notifications ▸ Family Connect) and Do Not Disturb are not readable by a host
app through GNotification (no API answers "will this be shown") (unverified — real desktop). So the
Notifications group says where the switch is ("If no notifications appear, check Settings ▸
Notifications ▸ Family Connect.") rather than claiming to know; with the freedesktop backend a failed
`Notify` is logged by GLib (measured: "unable to send notifications through
org.freedesktop.Notifications") and the shell does not see it either.

### 15.6 Logging

Every decision is logged by id with its reason (`notify: chat 42 raised`, `notify: chat 42 skipped:
in front`), never a name or a word — the Mac's `push` category.

---

## 16. Staying alive with no window — the top-bar icon (`desktop::Tray`)

### 16.1 Mac → Windows → Linux

| | Mac (#80) | Windows | Linux |
| --- | --- | --- | --- |
| Icon | `NSStatusItem`, template image, count beside it | notification-area icon | **StatusNotifierItem** shown by Ubuntu's AppIndicator extension (on by default, §1.7) |
| Click | opens the window | opens the window | **double click** opens (`Activate`); a single click opens the menu (the extension's behaviour, §1.7); middle click opens (`SecondaryActivate`) |
| Menu | Open Family Connect · Settings… · — · Quit Family Connect | open / quit | the Mac's three, via `com.canonical.dbusmenu` |
| Count | text beside the icon, "99+" | — | `XAyatanaLabel` = `MenuBarRules::count_text(unread)` ("99+" above 99), empty when 0 |
| Tooltip | "(3) Family Connect" | — | `ToolTip` = ("", [], "Family Connect", "(3) Family Connect") |
| Accessibility | "Family Connect, 3 unread" | — | `Title` = "Family Connect", and the label's guide text |
| Re-added | — | when Explorer restarts | when `org.kde.StatusNotifierWatcher` gets a new owner (GNOME Shell restarted, extension toggled) |

### 16.2 The protocol, over GDBus

The app exports, on its own session-bus connection (`gio::Application::dbus_connection()`), two
objects with gio 0.22's `DBusConnection::register_object(path, &interface_info)` builder
(`.method_call(..)`, `.property(..)`, `.build()` — measured in the crate's source) and introspection XML
compiled in (`gio::DBusNodeInfo::for_xml`):

- `/StatusNotifierItem`, interface **`org.kde.StatusNotifierItem`**: properties `Category`
  ("Communications"), `Id` ("me.nettrash.FamilyConnect"), `Title`, `Status` ("Active"; "NeedsAttention"
  while a call rings), `IconName` ("me.nettrash.FamilyConnect-symbolic"), `IconThemePath` (the install's
  `share/icons`, so the tarball's icon is found), `ToolTip`, `ItemIsMenu` (false), `Menu`
  (`/MenuBar`), `XAyatanaLabel`, `XAyatanaLabelGuide` ("99+"); methods `Activate`, `SecondaryActivate`,
  `XAyatanaSecondaryActivate`, `ContextMenu`, `Scroll` (no-op); signals `NewTitle`, `NewIcon`,
  `NewToolTip`, `NewStatus`, `XAyatanaNewLabel`.
- `/MenuBar`, interface **`com.canonical.dbusmenu`** (version 3): `GetLayout`, `GetGroupProperties`,
  `GetProperty`, `Event` ("clicked" → the item's `app.` action), `EventGroup`, `AboutToShow`,
  `AboutToShowGroup`; signals `LayoutUpdated`, `ItemsPropertiesUpdated`. Three items and a separator,
  labels from the catalogue — a fixed layout, so the implementation is small.
- Registration: `org.kde.StatusNotifierWatcher.RegisterStatusNotifierItem(<unique bus name>)` on
  `/StatusNotifierWatcher`; a `NameOwnerChanged` watch (`gio::bus_watch_name_on_connection`) on the
  watcher re-registers and is also how the shell knows whether an icon can exist.
- **As built (P5):** "an icon can exist" is the watcher owned AND our item accepted AND a host
  registered (`IsStatusNotifierHostRegistered`, followed through `StatusNotifierHost(Un)Registered`;
  an old watcher without the property counts as hosted) — `desktop::host_registry::IconHost`. Before
  the bus has answered at all (the first milliseconds) an icon counts as possible, so a login start
  does not flash its window; the first "no" is raised as a change and §3.7's backstop holds either
  way. The item also carries libayatana's `IconAccessibleDesc` (the Mac's VoiceOver sentence "Family
  Connect, 3 unread") and answers KDE's `ProvideXdgActivationToken` (the token is used for the next
  activation); `Status` is `Passive` when withdrawn at quit (§3.8), which raises no availability
  change.

Why not `ksni`: it would bring zbus, an executor and its own connection beside GTK's, and it does not
expose the Ayatana label the count needs; the hand-written objects are one fixed menu and a dozen
properties. `ksni` 0.3.7 is measured to build on 1.93 without tokio (§1.2) if this proves wrong.

### 16.3 What a click does

`Activate`/`SecondaryActivate` and "Open Family Connect" → present the main window (the platform's
activation token arrives with neither — the extension calls with coordinates only — so GNOME may show
"is ready" instead of raising; unverified — real desktop). "Settings…" → the Settings window. "Quit
Family Connect" → §3.8.

### 16.4 No host

When nothing owns `org.kde.StatusNotifierWatcher` (vanilla GNOME without the extension, the extension
switched off) there is no icon: "Keep running in the top bar" is insensitive with "Your desktop has no
top-bar icons for apps, so closing the window quits Family Connect.", a close quits (Windows' rule: no
icon to come back from, no hidden window), and a `--background` start presents its window (§3.7).

---

## 17. Launch at login (`desktop::Autostart`)

**Decision: the app writes the XDG autostart entry itself.** The Background portal's `RequestBackground`
with `autostart: true` would write the same file for a host app — after requiring a registered app id
(§1.7) and possibly asking — and GNOME's "Background Apps" list only tracks sandboxed apps. Writing it
directly is the XDG Autostart specification every desktop reads, testable in the container, and needs no
consent dialog for something the person just switched on.

`$XDG_CONFIG_HOME/autostart/me.nettrash.FamilyConnect.desktop`, written atomically:

```ini
[Desktop Entry]
Type=Application
Name=Family Connect
Exec=/usr/bin/family-connect --background
Icon=me.nettrash.FamilyConnect
X-GNOME-Autostart-enabled=true
NoDisplay=true
X-FamilyConnect-Autostart=1
```

(`Exec` is the running binary's absolute path, so the tarball's copy starts itself.) The states map onto
fc-logic's `StartupSetting` (Appendix B asks for the Linux spelling):

| Found | State | The row |
| --- | --- | --- |
| no file | `Disabled` | off, may be switched on |
| our file, enabled | `Enabled` | on |
| our file with `Hidden=true` or `X-GNOME-Autostart-enabled=false` (another tool switched it off) | `DisabledByUser` | off, insensitive: "Turned off in your desktop's startup settings." |
| a file of that name in `/etc/xdg/autostart` | `EnabledByPolicy` / `DisabledByPolicy` | "Your organisation decides whether Family Connect starts when you log in." |
| `$XDG_CONFIG_HOME/autostart` not writable | `Unavailable` | the row is not drawn |

A `--background` launch starts hidden only when `StartupSetting::starts_hidden(true, tray_available)`
(§3.7). Gnome-session starts it inside an `app-gnome-me.nettrash.FamilyConnect-autostart` scope, from
which the portal can derive the app id as well (§1.7, the regex in `xdg-desktop-portal`'s strings)
(unverified — real desktop).

---

## 18. The unread badge (`desktop::Badge`)

- **Window title**: "(3) Family Connect" (`NotificationRules::window_title`), which Alt+Tab and the
  dock's window list show.
- **Ubuntu Dock**: on every change of the count, the app's connection emits
  `com.canonical.Unity.LauncherEntry.Update` on an object path of its own
  (`/me/nettrash/FamilyConnect/launcher`) with `("application://me.nettrash.FamilyConnect.desktop",
  {"count": <int64>, "count-visible": <bool: count > 0>, "urgent": <bool: a call rings>})` —
  `gio::DBusConnection::emit_signal(None, path, "com.canonical.Unity.LauncherEntry", "Update", …)`. The
  dock keys entries by sender and clears them when the app leaves the bus (measured from its source,
  §1.7); drawing is (unverified — real desktop). The entry also answers libunity's `Query()` with
  what it last said, for a dock that restarts while the app runs (P5).
- **The top-bar icon**: its label (§16).
- The count is fc-logic's (`ChatList::unread`), **incremented** by live frames and never recomputed
  (`win/README.md`).

---

## 19. The global shortcut (`desktop::HotKey`)

| | Mac | Windows | Linux |
| --- | --- | --- | --- |
| API | Carbon `RegisterEventHotKey` | `RegisterHotKey` | **XDG GlobalShortcuts portal** (GNOME implements it, §1.7) |
| Chord | ⌃⌥⌘F | Ctrl+Alt+Shift+F | preferred trigger **`CTRL+ALT+SHIFT+f`** — unbound in GNOME 50 + Ubuntu's overrides (measured, §1.7) |
| Taken | `eventHotKeyExistsErr` → Settings says so | `ERROR_HOTKEY_ALREADY_REGISTERED` | GNOME shows its own dialog and the person picks; the bound trigger comes back in `BindShortcuts`' results / `ShortcutsChanged` (`trigger_description`) and Settings shows **that** |
| Behaviour | from anywhere: forward; in front: back to the menu bar | same | same: `GlobalHotKeyRules::action(window_in_front, can_hide_to_tray)` on `Activated` |

**Sequence, over GDBus** (all on `org.freedesktop.portal.Desktop`, `/org/freedesktop/portal/desktop`):

1. `org.freedesktop.host.portal.Registry.Register("me.nettrash.FamilyConnect", {})` — portal 1.21 refuses
   GlobalShortcuts to a host app without a valid app id, and GTK did not register in the container
   (§1.6); an "already registered" error is ignored.
2. `org.freedesktop.portal.GlobalShortcuts.CreateSession({handle_token, session_handle_token})` → wait
   for the `Response` signal on the request path (`subscribe_to_signal` **before** calling, on the predicted path
   `/org/freedesktop/portal/desktop/request/<sender>/<token>`, where `<sender>` is the connection's unique
   name without its `:` and with `.` → `_`).
3. `BindShortcuts(session, [("toggle-window", {description: "Show or hide Family Connect",
   preferred_trigger: "CTRL+ALT+SHIFT+f"})], parent_window, {handle_token})` — `parent_window` is
   `wayland:<handle>` from `gdk4_wayland::WaylandToplevel::export_handle` when the window is mapped, or
   `""`.
4. `Activated(session, "toggle-window", timestamp, options)` → the action; the 1.21 portal sends an
   `activation_token` in `options`: `gtk::Window::set_startup_id(token)` then `present()`, so the window
   really comes forward on Wayland.

As built (P5): `ListShortcuts(session)` comes before `BindShortcuts` — a binding GNOME kept is used as
it is, with no dialog; requests are applied one at a time, in order; the portal's name is watched with
auto-start, so the state is `NoPortal` while none runs and the switch is applied again when one
appears. On X11 `parent_window` is `""` (an `x11:<XID>` would need `gdk4-x11`, not a dependency).

"Open with …" off → `Session.Close`. Settings' "Change…" (as integrated): shown while a shortcut is
bound on a portal whose GlobalShortcuts `version` is 2, it calls `ConfigureShortcuts(session,
parent_window, {})` (`HotKeyState::configure`), and the new trigger comes back as `ShortcutsChanged`.
Whether GNOME asks again at every launch or remembers the binding per app id, and whether
xdg-desktop-portal-gnome 50 implements `ConfigureShortcuts`, are (unverified — real desktop).

**Everywhere else** — no portal, another desktop, X11: `family-connect --toggle` does the same through
the single-instance hand-off (§2.1), and Settings says to bind it in the desktop's keyboard settings.

---

## 20. The token in the Secret Service (`desktop::SecretTokenStore`)

| | Mac | Windows | Linux |
| --- | --- | --- | --- |
| Store | Keychain | Credential Locker, one credential per server origin, read once | **Secret Service** via `libsecret` 0.9 |

- Schema `me.nettrash.FamilyConnect.Session`, attributes `server` (the server's **origin** — never a path)
  and `account` ("session"); label "Family Connect (<host>)"; collection `libsecret::COLLECTION_DEFAULT`
  (the login keyring, unlocked by `libpam-gnome-keyring` at login). Store / lookup / clear with
  `password_store_future` / `password_lookup_future` / `password_clear_future` — measured round trip
  (§1.5).
- **Read once** at start-up (the gate stays Booting until it answers), then held in memory; every write
  goes to both (Windows' `LockerTokenStore`). `fc-core`'s `ITokenStore` is synchronous; the shell
  satisfies it from the in-memory copy and persists writes with a `spawn_local`ed store.
- **No Secret Service** (no keyring daemon, a locked collection the person refuses to unlock, a desktop
  without one): the token lives in memory for this run only and the Server group of Settings says
  "Your sign-in isn't remembered on this computer: no keyring is available." Never a plaintext file.
- `oo7` (pure Rust, measured to build, §1.2) is the fallback if libsecret's C side ever becomes a
  problem; it would also read the Flatpak keyring portal, which release 1 does not need.
- **Corrections (P5, measured in `fc-linux-dev`):** (1) libsecret 0.21.7 **ignores**
  `secret_service_open`'s bus-name argument (the proxy names `org.freedesktop.secrets` whatever is
  passed), so "no Secret Service" is proven in a child process on a private bus whose configuration
  names no service folder. (2) `gnome-keyring-daemon --unlock` with NOTHING on stdin creates no login
  keyring (the default alias is `/` and every store waits on a prompt); `printf '\n'` — an empty
  password ended by a newline — creates and unlocks it (`scripts/dev.sh` fixed). (3) The vault uses
  `secret_service_search` (`UNLOCK | LOAD_SECRETS`) rather than `password_lookup`, so an item still
  locked after the unlock prompt — the person refused — reads as `Unavailable`, not as "nothing
  stored"; store and clear are `secret_service_store` / `secret_service_clear` on the same service.

---

## 21. Single instance and activation from a notification

§2.2 holds the whole of it: one process per user session, the hand-off over D-Bus, the
`DBusActivatable` desktop entry and the D-Bus service file for actions that arrive while the app is not
running, and Wayland's activation tokens. Notification clicks are §15.4.

---

## 22. Session watch and keeping awake

§10.8 (lock, sleep, minimise, hide → recordings stop and park, playback pauses) and §10.9 / §14.5
(inhibit while recording or in a call). As built (P5): `desktop::keep_awake::hold(app, window, why,
say)` answers an `AwakeHold` that uninhibits once when dropped; `why` is `VoiceMessage`/`VideoMessage`
(`IDLE`) or `Call { video }` (`IDLE | SUSPEND`), and the reason GNOME shows is the apps' own sentence
("Recording a voice message", "Recording video", "Voice call", "Video call"). The session watch is P3's.

---

## 23. The nine languages

- English, German, Spanish, French, Japanese, Russian, Serbian (Cyrillic), Serbian (Latin), Simplified
  Chinese — the apps' `Localizable.xcstrings` is the source of truth; **the English sentence is the
  key** (`win/README.md`, "THE NINE LANGUAGES ARE JSON").
- `fc-core`'s `StringCatalog` (the port of `Core/Text/StringCatalog.cs`) loads Windows' eight tables
  (`win/src/FamilyConnect.Core/Text/i18n/*.json` + `plurals.json`) and **`linux/i18n/linux.json`** — the
  Linux port's own sentences in Windows' `win.json` shape (top bar, autostart states, "Ctrl+Alt+Shift+F"
  as each language writes it, the keyring line, the sandbox banner, the notifications footnote).
  Sentences Windows already has are reused verbatim (byte sizes, "Couldn't load statistics"…); a
  sentence nobody has translated reads in English and must be named in `linux.json` as `{}`, and the CI
  string gate fails on any other gap.
- The language is the **desktop's**, never the family's (`win/README.md`): `glib::language_names()`
  (LANGUAGE, LC_ALL, LC_MESSAGES, LANG, in order) mapped by the Windows rule — `sr@latin`/`sr_*@latin` →
  `sr-Latn`, `sr*` → `sr`, `zh_CN`/`zh_SG`/`zh_Hans*` → `zh-Hans`, a two-letter prefix otherwise, English
  when nothing matches.
- Formats: dates, times, decimals in the reader's locale through fc-logic (never `strftime`'s locale
  alone, never `{}` for a double — port-plan "Numbers and time").
- GTK's own strings (the file chooser, "Cancel" in a GTK dialog) come from GTK's gettext domain and
  follow the same locale.
- The locale sweep of Windows (`de_DE`, `tr_TR`, `ru_RU`, `fi_FI` with its `.` time separator,
  `ja_JP`) is run on the shell's pure modules too.

---

## 24. Accessibility

GTK 4 speaks AT-SPI to Orca (Recommended by `ubuntu-desktop-minimal`); the shell's duty is to give every
control a name, a role and a keyboard way.

- **Names**: every icon-only button has `update_property(&[Property::Label(..)])` and a tooltip from the
  catalogue (the Mac's #79 gaps and `composer-and-wide-layout` §3 row 7 — paperclip, ✨, 🎨 — are not
  repeated); bubbles, voice notes and circles are `AccessibleRole::Group` with a summary label
  (Windows' `NamedGroup`); a circle says "Video message, 0:23" with "Played"/"Not played"
  (`AccessibleProperty::Description`).
- **Live lines**: what a recording does, "Recording", a failure line, a new message in the open chat
  when the reader is at the newest — `gtk::Accessible::announce(text, AccessibleAnnouncementPriority::
  Medium)` (GTK 4.14+, compile-checked §1.2), the polite line of Windows.
- **Keyboard**: everything reachable by Tab; menus by Shift+F10 / Menu key; the timeline's bubbles are
  focusable (Up/Down move between them when one has focus); the board's notes move with the arrows
  (§11); the waveform is a slider (§10.3); targets at least 44 px where the plan says so (`VoiceLook::
  REACH`).
- **Reduce motion**: `gtk::Settings::is_gtk_enable_animations()` false or GTK 4.22's
  `gtk-interface-reduced-motion` (measured to exist, §1.7) → no cross-fades (150 ms slot, 200 ms
  recorder), the ring steps once a second, the five-bar meter, stickers at frame zero.
- **Contrast and size**: libadwaita's high-contrast style (`adw::StyleManager::is_high_contrast`); text
  follows GNOME's text scaling (`gtk-xft-dpi`); `RecorderFrames` is proven at 100–225 % text, the
  composer at the Mac's widths.
- Orca on the real desktop is §C.

---

## 25. Dark mode, accent, HiDPI

- **Colour scheme**: `adw::StyleManager::default()` follows the system (the settings portal; there is no
  in-app switch — the Mac follows the system appearance). `connect_dark_notify` re-tints the app's own
  CSS and the rich drawings' key includes the scheme (formulas are drawn in the ink colour). WebKit's
  `prefers-color-scheme` follows libadwaita's state (md facts §7).
- **Accent**: the Mac's `AccentColor` — **`#0D47A1` light, `#4D7DFC` dark** — written as libadwaita CSS
  variables (`--accent-bg-color`, `--accent-color`, `--accent-fg-color: white`) at
  `STYLE_PROVIDER_PRIORITY_APPLICATION`, overriding the system accent, so the reader's own balloon, the
  slot and badges are the same blue as on every other client. The own balloon's ink is white in both
  schemes (Windows' `Palette.Ink` lesson). Small accent TEXT (a sender's name, a link, the unread
  divider) uses `Palette::accent_text`: the accent on light, lifted about a third towards white on
  dark, where `#4D7DFC` measures under 4.5:1. Links are coloured by the app's own rules for every
  state: Ubuntu's libadwaita writes its link colours out (`#ac3911` / `#f08762`) instead of reading
  the accent.
- **High contrast and reduced motion**: GTK evaluates a provider's `@media (prefers-contrast: more)` /
  `(prefers-reduced-motion: reduce)` against that provider's own properties, and only libadwaita sets
  its own; `theme::install` sets the app provider's from `adw::StyleManager::is_high_contrast` and
  `gtk-interface-reduced-motion`, and follows both.
- **One CSS file**, generated from a palette (`theme::css(dark)`), loaded with
  `gtk::CssProvider::load_from_string` (md §10).
- **HiDPI**: GTK draws vectors and text at the surface scale; the shell makes bitmaps — avatars, tiles,
  rich drawings, posters, map tiles — at `ceil(surface.scale())` × their logical size, and redraws on
  `notify::scale-factor` (integer) / the surface's `scale` (fractional, Wayland) change. Icons are
  symbolic SVGs from the Adwaita/Yaru theme or the app's own `hicolor/scalable`, except where a
  theme draws two things alike or has nothing: `theme::glyphs` carries those (the sticker — a face
  on a peeled square, as Yaru's emoji and smiley are one face — and the video message's camera in a
  ring, the Mac's `video.circle`, as the header's Video Call is the bare camera), embedded and drawn
  by `gtk::Svg` with `EXTENSIONS | TRADITIONAL_SYMBOLIC` so they take the widget's ink like a theme's
  `-symbolic` icon (`TRADITIONAL_SYMBOLIC` alone drew them black, measured on GTK 4.22).
- Fonts: the system UI font (Ubuntu Sans) everywhere; board hands per `note_look::font_family`.

---

## 26. Logging and diagnostics

- `glib` structured logging (`glib::g_info!`/`g_warning!` under domain `family-connect`) → journald on the
  desktop, stderr in a terminal; `G_MESSAGES_DEBUG=family-connect` for debug lines.
- **Never** a message text, a name, a token, a server URL with a path, a file name a person chose, or a
  sound's bytes — ids, counts, codes and the exception's type only (port-plan "Logs"; Windows'
  `SoundTracks` rule).
- A `diagnostics.log` like Windows' is **not** kept: journald is the desktop's log, and
  `journalctl --user -t family-connect` is what a support conversation asks for.
- GStreamer: the app never sets `GST_DEBUG` (souphttpsrc would print the Authorization header at LOG).

---

## 27. The self-test (`family-connect --selftest OUT`)

md's harness (md §13), cargo feature `selftest` (off in packages): Xvfb, a private session bus
(`dbus-run-session`), `gnome-keyring-daemon --unlock` for the token, a **fake server** on localhost
(S0 built it on libsoup's own `soup::Server`, in the app behind the feature: `selftest/fake_server.rs`,
in process through `Harness::fake_server()` or as `family-connect --fake-server PORT` for several
instances), `xdotool` keystrokes, ImageMagick screenshots, a JSON report. Groups owned by the work packages (§W): `shell` (windows, gate, accels),
`chat` (list, timeline, composer keys, typing aids with real XTEST keys, paste, a drop
simulated by staging the same file list programmatically — Xvfb has no drag source), `media` (record from `audiotestsrc` by `FC_TEST_AUDIO_SOURCE`,
the profile read back, a circle plays clipped round — pixel test as §1.5), `rich` (every engine draws
offline under the CSP), `desktop` (fakes of GNOME Shell's notifications, the freedesktop server, the StatusNotifierWatcher and
the GlobalShortcuts portal on a second connection of the group's own; the LauncherEntry signal; the
autostart file states; the libsecret round trip, and "no Secret Service" in a child process), `calls` (two
instances in one container calling each other through the fake server, then the phase-0 Chromium peer),
`privacy` (only the files of §2.3 exist afterwards).

---

## D. Deliberate deviations from the Mac

Everything not listed is ported as the Mac does it. Each line: what changes, and why.

| # | The Mac | Linux | Why |
| --- | --- | --- | --- |
| D1 | A menu bar (File ▸ Record Voice Message, View ▸ Refresh, the app menu's Settings…) | No menu bar: the commands live in the sidebar's primary menu with the same accelerators; Settings also on Ctrl+, | GNOME has no global menu bar and a per-window bar for eight commands is clutter (md kept one because it has eight *menus*) |
| D2 | A toolbar gear for Settings; New Group in the toolbar | Settings in the primary menu; New Group as the sidebar header's "+" | GNOME HIG puts Preferences in the primary menu and "new" actions at the list's head |
| D3 | A single click on the menu-bar icon opens the window | A single click opens the icon's menu (whose first item opens the window); a double or middle click opens it directly | Ubuntu's AppIndicator extension does not deliver a single click to an item that has a menu (measured from its source, §1.7) |
| D4 | "Menu Bar", "Keep Running in the Menu Bar", "Open at Login" | "Top Bar", "Keep running in the top bar", "Start when I log in" | GNOME's words; Windows already reworded the same rows for its notification area |
| D5 | Open at Login through `SMAppService`; "requires approval" state | An XDG autostart file; "turned off in your desktop's startup settings" / "your organisation decides" states | §17 — the portal adds nothing for a host app and is not on every desktop |
| D6 | ⌃⌥⌘F registered silently; "another app holds it" | Ctrl+Alt+Shift+F *proposed* to GNOME, which may ask the person and may bind something else; Settings shows what was bound; `--toggle` elsewhere | Wayland has no app-registered global keys; the portal is the only door and the desktop owns the decision |
| D7 | Record Voice Message ⌥⌘R | Ctrl+Shift+R | Windows' chord: one keyboard rule for both non-Apple desktops; GNOME's Ctrl+Alt+letters are system chords (Ctrl+Alt+T, Ctrl+Alt+Del) |
| D8 | Capitals only while macOS "Capitalise words automatically" is on | Always on | GNOME has no such setting; Windows made the same call |
| D9 | System spell checking in every field | None in release 1 | GTK's text view has no checker; `libspelling` would be a new C dependency — nettrash decided 2026-10-29: later |
| D10 | Character palette (⌃⌘Space) | GTK's emoji chooser (Ctrl+. / Ctrl+; and the composer's emoji glyph) | GNOME's equivalent, built into `GtkTextView` |
| D11 | MapKit map previews; a location opens in Apple Maps | OpenStreetMap tiles (Windows' `MapTiles`, behind "Map Previews"); a location opens openstreetmap.org | No system map on Linux; Windows' privacy wording reused |
| D12 | Share in the viewer; a share extension into the app | No Share; files come in by drag and drop, paste, `family-connect FILE…`, and "Open With" for pictures, videos, sound and PDF (§8.6) | Linux has no share sheet; registering for those kinds (never for every type) was nettrash's decision, 2026-10-29 |
| D13 | Windows restored where they were | Size and maximised only | Wayland forbids client positioning |
| D14 | A Mac with the app quit still gets pushes (APNs, "who, never what") | A quit app hears nothing; only a running (possibly hidden) app notifies | No push channel for Linux in the protocol — Windows' position (`win/README.md`, "NO PUSH") |
| D15 | macOS says when notifications are off or None (#84) | Settings says where the switch is, without claiming to know | No API tells a host app whether GNOME will show its notifications (§15.5) |
| D16 | A new call window comes to the front | It may not take focus on Wayland; the call's notification (Answer/Decline) is the reliable door | Focus-stealing prevention without an activation token (§2.2) |
| D17 | The Keychain is always there | Without a Secret Service the sign-in is not remembered (in memory only) | Never a plaintext token file |
| D18 | Hardware video encoding (VideoToolbox) | Software `x264enc`; HDR sources go as the original | No VA-API encoder assumed (none in the image, §1.3); GStreamer 1.28 here has no tone-mapper |
| D19 | Dock badge on any Mac | Ubuntu Dock's badge; vanilla GNOME's dash shows none (the window title and the top-bar label still count) | `com.canonical.Unity.LauncherEntry` is Ubuntu Dock's (and KDE's) protocol |
| D20 | `log stream` (os_log) | journald (`journalctl --user -t family-connect`); no `diagnostics.log` | The desktop's own log |
| D21 | — | The board's focused note moves with the arrow keys | A keyboard way to do what only a drag does (accessibility); additive |
| D22 | A diagram engine failure is a source block | The same, and a sandbox failure leaves the app running with a banner (md quits) | A chat client must not quit over its diagrams |

---

## W. Work packages for wave 3

### W.1 Rules for every package

- **Ownership**: a package edits only its files (W.2). A change in another package's file, a frozen
  interface (W.3), `fc-core` or `fc-logic` is a request to its owner — never an edit (port-plan "Files
  and git").
- **Logic placement**: anything testable without a display belongs in `fc-logic` (a request to the
  wave-2 porters) or in a pure module of the app crate with unit tests (`text/offsets.rs`,
  `calls/tones.rs`, `desktop/autostart_file.rs`, `desktop/dbusmenu_layout.rs`, `media/pipelines.rs`
  string builders…).
- **Toolchain and gates**: rustc 1.93, edition 2024, `forbid(unsafe_code)`, `cargo fmt`, clippy
  `-D warnings` on the workspace **and** on `family-connect` with and without `--features selftest`,
  zero warnings; `scripts/dev.sh test` green.
- **Strings**: none of a package's own in code — every visible word from the catalogue; a new
  Linux-only sentence goes into `linux/i18n/linux.json` **with its eight translations** in the same
  change (the CI string gate).
- **Proof**: each package adds its self-test group (§27), runs it green in `fc-linux-dev`, looks at the
  screenshots, and lists its §C lines as "not verified here" in its report.
- **Git**: nothing committed; nettrash commits.

### W.2 The packages

| Pkg | Scope | Files (under `linux/app/src/` unless said) | Needs from fc-core / fc-logic | Order |
| --- | --- | --- | --- | --- |
| **S0 — skeleton** | the crate, the `Shell`, the rt seams, settings, paths, catalogue, CSS, the main window frame with the gate's empty pages, actions and accels, the frozen interfaces (W.3), the self-test harness | `../Cargo.toml` (adds `app` as a member, not a default member), `Cargo.toml`, `main.rs`, `app.rs`, `shell.rs`, `services/{paths,settings_store,catalogue,log,files}.rs`, `services/{soup_http,soup_socket,glib_clock}.rs`, `theme/{palette,css}.rs`, `windows/main_window.rs` (frame), `selftest/{mod,shell}.rs`, `../i18n/linux.json` (created) | fc-core `rt` (done), `protocol` (DTOs, `ApiClient`, `ChatSocket`), `store` (`Database`), `text::StringCatalog`; fc-logic `AppSession`, `LiveConnection` | **first, alone** |
| **P1 — chat** | gate pages, chat list, conversation page, timeline, bubbles, reactions, message menu, threads, polls, typing line, composer (keys, paste, drop, slot without recording, mentions, emoji chooser, typing aids, drafts, staging calls), the conversation window | `windows/{gate,conversation_window}.rs`, `chats/*`, `conversation/*`, `composer/*` (minus `composer/recording_row.rs`), `text/{offsets,typing_hooks}.rs`, `selftest/chat.rs` | fc-logic `Doorway`, `Families`, `ChatList`, `Groups`, `Conversation`, `ConversationRows`, `Timeline`, `BubbleBody`, `BubbleRules`, `Quotes`, `MessageMenu`, `Threads`, `PollDraft`, `PollText`, `PollVoting`, `OpenPolls`, `Typing`, `PeerReads`, `ComposerRow`, `ComposerDrafts`, `ComposerMentions`, `ComposerButton`, `ComposerStaging`, `AttachMenu`, `NewLineKeys`, `Capitals`, `CapitalField`, `CapDraft`, `EmojiTyping`, `LinkPreviews`, `AssistantConsent`, `AssistantAnswers`, `ConsentedAsk`, `Lookups`; fc-core `text::paste_rules`, `SendPipeline`, `MediaOutbox` | after S0; **first among equals** — P2/P3 plug into its bubble and composer slots |
| **P2 — attachments and rich** | tiles, file rows, avatars, the viewer window, stickers and the pack, animations' display, map previews and tiles, link-preview fetcher, location, the inbox and "Send to", the rich renderer with its scheme and sandbox preflight | `attachments/*`, `viewer/*`, `rich/{renderer,scheme,sandbox_probe}.rs`, `windows/viewer_window.rs`, `selftest/{attachments,rich}.rs` | fc-logic `AttachmentCache` (`IBlobStore`), `AttachmentFiles`, `MediaAlbum`, `Avatars`, `Pack`, `StickerLook`, `PictureMemory`, `PhotoPinning`, `PictureHint`, `MapView`, `RichDrawing`, `BackdropConsent`; fc-core `text::{media_text, attachment_text, sticker_file}` | after S0, beside P1 |
| **P3 — media** | the player and `NowPlaying`, voice recording (row, meter, waveform, not-sent rows, parking), voice bubble, "Show text" under a voice note and a circle, the round clip and inline circles, the camera (portal + fallbacks), the video message recorder and its layer, preparing picked media (transcode, read-back, cancel), animation encoding, session watch, keep-awake for recording | `media/*`, `composer/recording_row.rs`, `selftest/media.rs` | fc-logic `VoiceNotes`, `NotSent`, `ParkedRecordings`, `VoiceLook`, `DrawnCopies`, `RoundLook`, `RoundPlayback`, `RoundRecorder`, `RoundVideoRules`, `RecorderFrames`, `MediaEncoding` (Linux variant), `TranscodeAttempts`, `FrameTurn`, `Faststart`, `ContainerSound`, `AnimatedPictures`, `AnimationFrames`, `AnimationTiming`, `AnimationMp4`, `MediaOutbox` (`IMediaStore`), `Transcripts`, `TranscriptSound`; fc-text `waveform`, `media_plan`, `record` | after S0, beside P1; owns `media::camera` which P6 uses |
| **P4 — board, family, settings** | the board window (wall layout, drag, note sheet, calendar), the Family dialog and owner console, group dialogs, the Settings window | `board/*`, `family/*`, `settings/*`, `windows/{board_window,settings_window}.rs`, `selftest/{board,family,settings}.rs` | fc-core `board::*` (done in wave 1), `text::calendar`; fc-logic `Board`, `NoteDraft`, `NoteHand`, `NotePreview`, `WallText`, `EventText`, `Family`, `FamilyConsole`, `Families`, `Groups`, `GreetingWeather`, `Settings` (`LeaveContext`, `BirthdayRules`, `SettingsText`), `AppVersionText`, `StartupSetting` (Linux), `GlobalHotKeyRules` (Linux) for the rows | after S0, beside P1 |
| **P5 — desktop** | notifications and their actions, the tray (SNI + dbusmenu over GDBus), the LauncherEntry badge, autostart, the host registry and GlobalShortcuts, the Secret Service token store, inhibit wrappers, the D-Bus request/response helper every portal call uses | `desktop/{dbus,portal_request,host_registry,notifier,tray,dbusmenu,dbusmenu_layout,badge,autostart,autostart_file,hotkey,secret,keep_awake}.rs`, `selftest/desktop.rs` | fc-logic `Notifications` (`NotificationRules`, `Toast`), `ToastArguments`, `GlobalHotKeyRules`, `StartupSetting`, `TrayRules` (new, Appendix B); fc-core `ITokenStore` | after S0, beside P1; `desktop::secret` is needed early by S0's gate — S0 ships an in-memory store and P5 replaces it |
| **P6 — calls** | `GstCallMedia` (webrtcbin, AEC, codecs, ICE/TURN), tones, the Call window, call notifications (through P5's `Notifier`), keep-awake in a call, the interop harness in Docker (two instances; Chromium; Android emulator) | `calls/{engine,pipeline,ice,tones,call_window,call_notice}.rs`, `selftest/calls.rs`, `../tools/rtc-spike` extended (or `../tools/call-interop`) | fc-logic `Calls` (`CallEngine`, `ICallMedia`, `CallState`, `CallText`, `RingTone`), `CallRecords`; P3's `media::camera`; P5's `Notifier` | after S0; **start early** — interop is the release's largest risk |
| **P7 — packaging** | `.desktop` (`DBusActivatable=true`, `Exec=family-connect %F`, §8.6's `MimeType=` — with P1's `connect_open`, never before), D-Bus service file, icons (app + `-symbolic` tray icon), metainfo, AppArmor profile + maintainer scripts, `/usr/share/family-connect/rich`, deb/rpm/tar.gz, Depends, the release workflow | `linux/packaging/*`, `linux/scripts/package.sh`, `.github/workflows/*` (with nettrash) | none (file list from this document) | alongside S0, from day one |

**Order**: S0 → {P1, P2, P3, P4, P5, P6} in parallel → integration (P2/P3 into P1's slots, P6 onto P3/P5)
→ the §27 self-test across all groups → phase 7, the §C checklist on real machines. P7 runs throughout.

### W.3 Interfaces S0 freezes

- `Shell` (field per service in §2.4) and `Shell::get()`.
- `ConversationSlots` — how P2 and P3 draw inside P1's bubble: `fn tile(&self, attachment, look) ->
  gtk::Widget`, `fn voice(&self, …)`, `fn circle(&self, …)`, `fn sticker(&self, …)`, `fn rich(&self,
  block) -> RichPicture`.
- `ComposerSlots` — P3's recording row and slot states inside P1's composer.
- `Player`, `NowPlaying`, `Camera` (P3) and `Notifier`, `TrayState`, `BadgeState`, `HotKeyState`,
  `AutostartState`, `TokenStore` (P5) as traits with S0's no-op implementations, replaced by the owners.
- Action names (`app.open-chat`, `app.answer-call`, `app.decline-call`, `app.toggle-window`,
  `app.show-settings`, `app.open-board`, `app.open-family`, `app.quit`, `win.refresh`,
  `win.record-voice`, `win.paste-attachment`, `win.message-*`) and their target types.

**Frozen in code by S0** (the rustdoc is the contract; a change is a request to S0's owner):
`app/src/seams/` — the table in `seams/mod.rs`: `actions` (every action, scope, target type,
accelerator, handler), `routes` (`ChatRoutes` P1, `FamilyRoutes` P4, `CallRoutes` P6, `InboxRoutes`
P2, `WindowPart`), `conversation` (`ConversationSlots`, `TileLook`, `RichPicture`), `composer`
(`ComposerSlots`, `ComposerHost`, `ComposerRecording`), `media` (`Players`/`Player`, `NowPlaying`,
`Camera`), `desktop` (`Notifier`, `TrayState`, `BadgeState`, `HotKeyState`, `AutostartState`),
`tokens` (`TokenVault` under fc-core's `TokenStore`: `VaultedTokens`), `calls` (`NoCallMedia`);
`shell.rs` (`Shell`, its `Slot`s, `Connection`); `windows/main_window.rs` (`MainWindow`'s accessors,
`GatePage`, `page_for`, `Route`); `services/settings_store.rs` (`keys`); `theme/css.rs` (the style
classes) and `theme::register`; `selftest/mod.rs` (`Harness`) and `selftest/fake_server.rs` (the fake
server and how a group extends it). A package plugs in with ONE line in `app::install_packages`
and its `mod` line in `main.rs`.

---

## C. The real-desktop checklist (what a container cannot verify)

Run on a fresh **Ubuntu 26.04 Desktop, GNOME on Wayland**, once on amd64 and once on arm64, with the
`.deb` installed, as an ordinary user, the build's SHA-256 written at the top. Ordered by risk.
Each line: how → what correct looks like. A failing line names its section.

**1. Launch blockers (§9.5, §1.5)**
1. `sudo aa-status | grep family-connect`; `cat /proc/sys/kernel/apparmor_restrict_unprivileged_userns`
   → the profile is listed, the sysctl is 1.
2. Open a chat with a formula, a Mermaid and a Graphviz block → all three drawn; no banner; the app
   does not vanish. `pgrep -a WebKitWebProcess` + `grep NSpid /proc/<pid>/status` → two numbers (bwrap on).
3. Receive a HEIC photo and an animated WebP sticker → both drawn, the sticker moving (glycin's
   sandbox under the same profile).
4. `sudo apparmor_parser -R /etc/apparmor.d/family-connect`, relaunch → chats work, the rich-content
   banner appears, blocks show their source; reload the profile.

**2. Calls (§14)** — each against a real peer: iPhone, Mac, Android phone, web (Chrome and Firefox),
Windows; both directions; voice and video; on the same Wi-Fi and across networks with the operator's TURN.
5. Audio both ways within 2 s of Answer; no echo on laptop speakers for the far side (`webrtcdsp`).
6. Video both ways; the far side's picture right way up; self-view mirrored; Camera off → the far side
   sees the stream stop; on again → it resumes; no renegotiation in the server log.
7. Incoming while the window is hidden → the ring tone plays, a notification with Decline/Answer appears;
   Answer from it → the Call window comes up and focused; Decline → the caller sees "declined".
8. A headset plugged in mid-call → audio moves to it (PipeWire default) or note that it does not.
9. Hang up from each side; the record in the chat reads right; the camera and microphone lights go out.
10. A TURN-only network (UDP blocked) → the call connects through the relay (the percent-encoded
    `<expiry>:<user_id>` username works).
11. Close the Call window mid-call → the call goes on; Return to Call brings it back.

**3. Staying alive (§16, §3.3, §17, §19)**
12. The top-bar icon appears after sign-in; the unread count shows beside it; double click opens the
    window; single click shows Open / Settings… / Quit; Quit quits.
13. Close the window → it hides, the icon stays, a message still raises a notification; Ctrl+W the same.
14. Turn the AppIndicator extension off (Extensions app) → Settings' row says there is no icon; a close
    now quits. Turn it back on → the icon returns without a relaunch.
15. With the window hidden, `gnome-extensions disable ubuntu-appindicators@ubuntu.com` then `enable` → the
    icon comes back by itself (the watcher's new owner), and the window is reachable from it again.
16. First launch: GNOME's dialog for Ctrl+Alt+Shift+F appears (or not) — record what it shows; accept →
    the chord opens the window from another app, again puts it back; relaunch → no second dialog (or note).
17. Start when I log in → log out and in: the icon is there, no window, a message arrives; the autostart
    file exists with `--background`.

**4. Notifications and badge (§15, §18)**
18. A message in a chat not in front → banner "<Family> — <Sender>" / "New message", never the text; a
    second message in the same chat replaces it; opening the chat removes it.
19. Click a banner with the window hidden, and with the app quit → the window opens on that chat, once.
20. Do Not Disturb on → no banner; an incoming call's notification — note whether it shows (urgent).
21. Ubuntu Dock shows the unread count on the icon; it clears when everything is read; it pulses while a
    call rings.

**5. Token (§20)**
22. Sign in, quit, relaunch → still signed in; `secret-tool search server <origin>` finds one item.
23. Lock the login keyring (Seahorse), relaunch → GNOME's unlock prompt; cancel → not signed in, the
    "isn't remembered" line; unlock → signed in.

**6. Media (§10)**
24. Record a voice note (Ctrl+Shift+R, and the slot) → the meter moves, GNOME's microphone indicator
    shows only while recording; review plays; send → a phone plays it; its waveform matches.
25. Record a video message → GNOME's camera permission prompt (first time); the preview mirrored; the
    received square on a phone is **not** mirrored, upright, round, with sound; `ffprobe` on the upload:
    480 × 480 High, AAC mono 64 k, `moov` before `mdat`, keyframes ≤ 2 s.
26. A received circle plays inside the circle (no square corners), the ring outside the edge, expand
    opens the viewer; Reduce Animation on → the ring steps.
27. Voice speed 1.5×/2× → faster, same pitch.
28. A long video from a phone in the viewer → starts playing before it has downloaded; seeking works
    (Range reads); Save… writes the file through the portal's chooser.
29. Attach an iPhone HEVC video, a 4K 60 fps clip, an HDR clip, a portrait clip → the first two are
    transcoded (H.264 High 720p30, `moov` first) within reason; the HDR one goes as the original; the
    portrait one is upright on a phone.
30. Lock the screen, suspend, minimise during a recording → it stops and waits as "Voice message not
    sent"; the screen stayed on while recording.

**7. Wayland and input (§3.5, §6, §7, §25)**
31. Fractional scaling 125 % and 200 % → text, avatars, rich drawings and circles crisp.
32. NVIDIA if available: the viewer and calls paint (note whether `WEBKIT_DISABLE_DMABUF_RENDERER=1` or
    `GDK_DISABLE=dmabuf` is needed).
33. IBus Pinyin/Anthy in the composer: Enter commits the composition and does **not** send; the next
    Enter sends; no capital applied to composed text.
34. A dead-key layout: `'` `e` at a sentence start → É; `:)` → 😊 and Backspace takes it back.
35. Ctrl+. opens the emoji chooser; a German or Hungarian layout: Ctrl+Shift+R, Ctrl+, and AltGr letters
    all behave.
36. Paste a screenshot (Print Screen → clipboard) and an image copied from Firefox → staged as a photo;
    paste text → text. Drag a file from Files → staged; drag text → refused.

**8. Accessibility (§24)**
37. Orca: the chat list, a bubble ("<sender>, <time>: …"), the slot in every state, a voice note's slider,
    "Recording" announced before the microphone opens, a circle's name and state.
38. High Contrast and Large Text (GNOME Accessibility) → nothing clipped in the composer, the recorder,
    Settings.

**9. The rest**
39. Location in the paperclip → a fix or a clear refusal (GeoClue without GPS).
40. The board's note faces (carry-forward): the casual hand draws differently from plain — or decide.
41. Log out while recording → the recording is parked and offered at the next launch.
42. Two conversation windows + the board + a viewer open, hide the main window, quit from the icon →
    everything closes, nothing left running (`pgrep family-connect`).

---

## Appendix A — how the facts were measured (re-runnable)

All in `fc-linux-dev` (`docker run --rm -v <dir>:/w -v fc-linux-cargo:/cargo -e CARGO_TARGET_DIR=/w/target …`).

| § | Command |
| --- | --- |
| 1.1 | `rustc --version; pkg-config --modversion gtk4 libadwaita-1 webkitgtk-6.0 gstreamer-1.0 gstreamer-webrtc-1.0 libsoup-3.0 libsecret-1 glib-2.0 sqlite3 gstreamer-play-1.0`; `apt-cache policy <pkg>` after `apt-get update` |
| 1.2 | a crate with the §1.2 dependencies, `rust-version = "1.93"`, `[workspace] resolver = "3"`: `cargo build` (0 warnings), `cargo tree --depth 1`; a second crate with `ksni`/`ashpd`/`oo7`/`gdk4-*`: `cargo tree -i tokio` (none) |
| 1.3 | the first crate's binary printing `gst::ElementFactory::find(name)` and its plugin's file, before and after `apt-get install gstreamer1.0-gtk4`; `gst-inspect-1.0 <element>` for properties and caps |
| 1.4 | `gst-launch-1.0` pipelines as quoted; `gst-discoverer-1.0 -v` (package `gstreamer1.0-plugins-base-apps`); top-level boxes walked with `od`/`dd`; a 15-line Python `http.server` logging `Authorization` and `Range` and answering 206 |
| 1.5 | a GTK binary under `Xvfb :99` (`GDK_BACKEND=x11`), screenshot with `import -window root`, pixels with `convert … -format '%[pixel:p{x,y}]'`; sample images from ImageMagick, `cwebp`, `img2webp`, `heif-enc`; libsecret under `dbus-run-session` + `gnome-keyring-daemon --unlock --components=secrets` |
| 1.2 (APIs) | a third binary naming every GTK/GIO/WebKit/GStreamer call the design relies on, built with 0 warnings (never run) |
| 1.6 | `dbus-run-session -- bash -c 'dbus-monitor --session type=method_call & …'` around an `adw::Application` that presents a window, sends a notification and inhibits |
| 1.7 | `apt-get download gnome-shell-ubuntu-extensions xdg-desktop-portal xdg-desktop-portal-gnome mutter-common gnome-shell-common gnome-settings-daemon-common ubuntu-settings`, `dpkg -x`, read the JS and NEWS; all schemas + overrides compiled with `glib-compile-schemas` and listed with `gsettings list-recursively` |

---

## Appendix B — what the shell needs beyond `win/`'s API

**From fc-logic (wave-2 porters):**

1. `tray_rules` — a port of the Mac's `MenuBarRules` (`ios/FamilyConnect/Core/MenuBarRules.swift`):
   `count_text` ("99+"), `close_hides(keeps_running, tray_available, quitting)`, `starts_hidden`, with
   `MenuBarRulesTests`' cases. Windows keeps these in its window code; the Mac has them pure.
2. `StartupSetting` in Linux spelling: the five states of §17 (`Disabled`, `Enabled`, `DisabledByUser`,
   `EnabledByPolicy`/`DisabledByPolicy`, `Unavailable`) with Windows' rules (`is_offered`, `is_on`,
   `is_changeable`, `note_for`, `starts_hidden`, `request_succeeded`).
3. `GlobalHotKeyRules` without Win32: `action(window_in_front, can_hide_to_tray)` as is; the "taken" test
   replaced by the portal's states (`Bound(trigger)`, `NotBound`, `NoPortal`) and their sentences.
4. `MediaEncoding` for GStreamer: AAC at 64 000 is accepted as asked (no four-rate ladder); the encoder
   order `x264enc` (High) → `openh264enc` (Main); the read-back's checks unchanged.
5. `RichPage` constants for the `fc-rich://rich/rich.html` origin; `RichCapture` unchanged.
6. `ToastArguments::parse` for a `GVariant` target (an `i64` chat id, a `String` call id), keeping "anything
   but a positive id opens nothing".
7. Not needed on Linux (do not port, or port without a caller): `RoundFrames`, `RoundCrashGuard` (no frame
   server), the AAC-rate fallback in `MediaEncoding`, `ScreenReader`'s Narrator specifics.

**From fc-core:** `text::StringCatalog` loading `linux/i18n/linux.json` beside Windows' tables (and the CI
string gate covering it); `ITokenStore` stays synchronous (the shell caches).

**For the dev image (`linux/docker/Dockerfile`, owner: nettrash):** add `gstreamer1.0-gtk4` (the sink is
not installed today, §1.3), `gstreamer1.0-tools` and `gstreamer1.0-plugins-base-apps` (pipelines and the
discoverer in tests), `gnome-keyring` (the token self-test), `python3-minimal` only if the fake server is
not written in Rust.

**For the workspace (`linux/Cargo.toml`, owner: nettrash):** `app` as a member, not a default member (as
md); the app's dependencies — `gtk4` 0.11 (`v4_22`), `libadwaita` 0.9 (`v1_9`), `webkit6` 0.6 (`v2_52`),
`gstreamer`, `-play`, `-video`, `-audio`, `-app`, `-pbutils`, `-webrtc`, `-sdp` 0.25 (`v1_28`),
`libsecret` 0.9, `gdk4-wayland` 0.11 (`v4_20`), and `gdk4-x11` 0.11 (`v4_18`) only if X11 sessions get
portal parent windows — each measured on 1.93 (§1.2) and each needing
nettrash's yes (port-plan "Dependencies").

**For packaging (P7):** `Depends:` `gstreamer1.0-plugins-base, gstreamer1.0-plugins-good,
gstreamer1.0-plugins-bad, gstreamer1.0-plugins-ugly, gstreamer1.0-libav, gstreamer1.0-nice,
gstreamer1.0-pipewire, gstreamer1.0-gtk4, glib-networking` (TLS for libsoup) plus what `dpkg-shlibdeps`
finds; `Recommends:` `gnome-keyring, xdg-desktop-portal-gnome`; the AppArmor profile of §9.5; the D-Bus
service file and `DBusActivatable=true` of §2.2; §8.6's `MimeType=` and `%F` (shipped with `connect_open`).
