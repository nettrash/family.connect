# Carried forward

What a port found that a LATER wave must act on. Each line names who found it and who must act.
Delete a line when it is done, saying where.

## For the fc-logic port (wave 2) — the shell design's Appendix B

Done in layer 0 (w2 logic-base): `tray_rules` (the Mac's MenuBarRules), `startup_setting` (§17's
states, the autostart entry read and written), `global_hot_key_rules` (portal states),
`rich_drawing::RichPage` (the `fc-rich://rich/rich.html` origin), `notification_target::parse` (the
GVariant target), and `date_culture` (a seam for CultureInfo/TimeZoneInfo: the shell implements
`TimeZone` with `glib::TimeZone` and picks `DateCulture::for_locale`).

- (`MediaEncoding` for GStreamer: done in `media_encoding`, w2 logic-rec. `ToastArguments.For`/
  `IsNews`: done in `toast_arguments`, w2 logic-session.)
- Not ported (no caller on Linux): `RoundFrames`, `RoundCrashGuard`, the AAC-rate fallback,
  Narrator specifics.
- Every x264 encode pins `format=I420` before the encoder and `profile=high` after it (measured: left
  free it negotiates High 4:4:4 10-bit, which Apple devices and browsers cannot play), and `mp4mux
  faststart=true` (else `moov` is last).
- (Linux's own sentences: done in S0 — `linux/i18n/linux.json` translates them into all eight,
  `linux/i18n/generate.py` writes `linux/data/i18n`, `ENGLISH_FOR_NOW` is empty and the two ignored
  catalogue tests run; "Settings…" was in the apps' xcstrings all along.)

## Decided by nettrash before wave 3 (2026-10-29)

1. **"Open With": REGISTER** — pictures, videos, sound and PDF (`MimeType=` + `Exec=family-connect
   %F`, shell-design §8.6), shipped together with the app's `connect_open`, never before.
2. **Spell checking: later**, not in release 1 (D9).
3. **No top-bar icon: closing the window QUITS** (the design's default; Settings says why).
4. **Release: HELD** until the client is ready — `release-linux.yml` skips a tag unless
   `linux/packaging/release-ready` exists (added in the commit that finishes the client).
5. "Top Bar" as the settings group's name, and the app crate's dependencies of shell-design
   Appendix B with D-Bus through gio — decided by me as the design proposed (he left them to me).
6. Still open for him: the support/privacy pages behind the `.deb`'s Homepage, and the AppStream
   OARS rating ("intense" for chat, audio, location).

## For fc-logic's `LiveConnection` and the app's composition (w1 sync: router, pipeline, resync)

Done in the core (w1 sync): `protocol::frame_router`, `protocol::send_pipeline`, `protocol::resync`.

- fc-logic: App.Logic tests that relied on the C#'s any-mention "@" mark need the protocol's rule
  (the store now marks only an unread message that names the reader, from somebody not blocked).
- Stores are shared as `Rc<ChatStore>`, `Rc<BoardStore>`, `Rc<PackStore>`, `Rc<OutboxStore>`; the
  client as `Rc<ApiClient>`; the socket as `Rc<ChatSocket>` (it is the `Rc<dyn FrameSender>`).
- Build the pipeline FIRST (`SendPipeline::from_api(socket, outbox, chats, api, clock)
  .with_uploads(media pump)`) — it subscribes to the socket's frames itself, as the C# did in its
  constructor, and unsubscribes when dropped. Then `FrameRouter::new(chats, board, Some(pack))` and
  `socket.on_frame().subscribe(move |f| if let Err(e) = router.hear(f) { log e })` — in that
  order, as `Connection.cs` builds them. `router.hear` answers the cache's error instead of
  throwing; log it (ids and counts only) and go on.
- `pipeline.on_cache_failed()` reports a frame that answered a send but could not be written; log
  it. The row stays queued and the next flush re-sends it under the same key.
- `Resync::new(api, chats, board).with_sending(pipeline).with_pack(pack)`. On
  `socket.on_connected()` call `resync.snapshot()` AT ONCE (synchronously, before any frame of the
  connection is heard), then signal a pass. The passes are ONE future spawned for the session
  (`PassesAsync`): wait for the signal, `resync.run().await` — never two at a time — and apply
  `Report::me`, then the assistant if `has_family` and the video limits if `family_read`, each only
  when `shown_family` is `None` or still the family shown. An `Err` is a failed pass; the next
  connection starts another.
- The media pump's signature for `with_uploads` is `Rc<dyn Fn() -> LocalBoxFuture<'static,
  StoreResult<()>>>`: a cache error ends the flush (and so the pass's flush step) with `Err`, as
  the C#'s exception did; anything else the pump decides itself (MEDIA_MISSING, …).
- Flushes: spawn `pipeline.flush(trigger)` for connectivity, window activation, the timer, a retry
  and a row just queued (`FlushTrigger::Queued`). A flush that finds one running answers 0 at once.
- `testing::FakeFrameSender`, `testing::FakeServer` (routes, `/api/v1` prefix in `always`) and
  `testing::StillClock` (the C# tests' `now: () => Now, wait: CompletedTask`) are there for the
  App.Logic tests.

## For the shell (wave 3) — the session layer (w2 logic-session)

(Done in S0: `app/src/shell.rs`, `Connection::open` composes exactly this; the two memories are
`settings.conf` keys; zones are `services::glib_clock::GlibZone`; notifications are raised by
`shell::Attention` through the `Notifier` seam P5 implements.)

- **Composition** (`fc_logic::live_connection`'s module comment has it whole): one
  `Rc<AppSession>` (`AppSession::new(api, tokens, cache).with_awaiting_join(…)
  .with_current_family(…)` — both memories belong in the shell's settings file), then
  `LiveParts::compose(Rc<ChatSocket>, api, chats, board, Some(pack), outbox, clock,
  Some(media_outbox.pump()))` (pipeline FIRST, router, resync) and `LiveConnection::new(session,
  parts, spawn)` with `spawn` = `glib::MainContext::spawn_local`. Subscribe
  `live.on_cache_failed()` to the log (ids and counts only); call `live.start()` on every window
  appearance; spawn `live.flush(trigger)` for connectivity, activation, the timer, a retry and a
  queued row. `ChatStore`/`BoardStore`'s `with_current` read `session.state().current_family_id`
  (and `with_home_family` its `home_family_id()`).
- **Zones and collation:** `ChatListModel::with_zone(glib local zone)` (UTC otherwise);
  `InviteModel::with_collation(…)` with `g_utf8_collate` over case-folded names (code-point order
  otherwise — "Émile" after "Zoe"). `DateCulture` gained `short_date_pattern` (standard `d`, measured
  on .NET 10.0.13) for the chat list's older rows (`doorway::row_time_text`).
- **Notifications:** raise a `GNotification` from `notifications::Toast` (title, body, `tag` as the
  id) with `toast_arguments::for_toast(&toast)`'s action and target; the body is never message
  text.

## For the shell (wave 3) — the recording package (w2 logic-rec)

- **The platform traits** (each method's contract is in its rustdoc): `voice_notes::VoiceRecorder` /
  `VoiceRecording` (the §10.2 pipeline; `voice_look::VoiceMeter` is its supertrait — the `level`
  branch of the same pipeline), `voice_notes::Inhibitor` (§10.9, held by `KeepAwake`),
  `voice_look::VoiceDecoder` (the sender's waveform: run `voice_waveform::shaped` on a recorded note
  before it is staged or sent — Windows' `VoiceShape.ShapedAsync`), `round_recorder::
  VideoMessageRecorder` (the camera portal, §10.5; the module doc maps every `RecorderAction` to what
  the shell does), `round_playback::RoundPlayers`/`RoundPlayer` (§10.1/§10.4), `transcript_sound::
  SoundTrackMaker` (use `transcript_sound::device_source` for a "Show text" press) and
  `media_encoding::preparing::MediaPreparing` (§10.7, driven by `prepare_video`/`prepare_sound`).
- **Take the numbers from the logic, not from §10's examples:** the voice note and the video
  message's sound are `media_encoding::voice_note_attempts()` — AAC-LC mono 64 000 at **44.1 kHz**
  (the Mac's and Windows' rate; §10.2/§10.5 wrote `rate=48000` — both are in the protocol); the video
  message's frame rate is the camera's up to 30, **never raised** (§10.5's `framerate=30/1` would
  raise a 24 fps camera — the profile forbids it); the poster is taken at **0.5 s, else 0 s, else 2 s**
  (docs/protocol.md "Video messages"; §10.5 said "the first frame").
- **`round_video_rules::RECORDING_ENABLED` is `true`** since 2026-10-29, ahead of §C 25 (§10.5), as
  Windows' was ahead of its trials: the camera itself is still to be checked on a real desktop.
- **HDR (§10.7 vs the port):** `MediaPreparing::encode`'s contract follows Windows — a `to_sdr` ask
  converts to BT.709 (highlights clip, no tone-mapper) and the read-back judges the result — where
  §10.7 says an HDR source goes as the original. Pick one.
- **Parked recordings** live under `$XDG_STATE_HOME/family-connect/parked/`
  (`ParkedRecordings::for_account(root, server, account_id)`): pass the server spelled the same way
  every time — the owner key hashes it (Windows used `Uri.AbsoluteUri`).
- **Settings pages:** a refusal names `round_video_rules::SettingsPage` (`gnome_panel()` → "camera" /
  "microphone") and `voice_notes::MICROPHONE_SETTINGS_PANEL`; whether `gnome-control-center`'s
  `launch-panel` takes them on 26.04 is unverified. The slot's `SlotIcon::icon_name()` (Adwaita
  symbolic names) and how GDK reports a pen's barrel button are real-desktop checks too.
- **`composer_row::conversation_column`** keeps WinUI's rail, frame and minimum-window numbers so
  its arithmetic and tests carry over; the Linux window takes only the column, bubble and `arrange`
  from it (§3.1 sets its own minimum).

## For the shell (wave 3) and the editor ports
- **Blob and media stores** (w2 found): the shell implements `fc_logic::attachment_cache::BlobStore`
  (whole-or-nothing writes, as Windows' `FileBlobStore`; a failed write is logged and forgotten — the
  next look downloads again) and `fc_logic::media_outbox::MediaStore` (under `$XDG_STATE_HOME`).
- **Session wiring** (w2 found): `TypingRoster::heard`/`spoke` from the socket's frames;
  `SendPipeline::with_uploads(media_outbox.pump())`.
- **Where the cache lives** (w1 store): the outbox is IN the cache file, so it belongs under
  `$XDG_STATE_HOME/family-connect/`, never `~/.cache` (tools purge caches).
- **SQLite on the main thread** (w1 store): Windows ran it on worker threads; here every store call
  is synchronous on the GLib main context. Measure a large page apply on a real desktop.

- **Caret units** (w1 board): `fc_text::board::cap_at_caret` — and every `fc-text` rule that takes a
  caret or a selection — counts in UTF-16 code units, as the Mac and Windows do. GTK reports
  `gtk::TextIter` offsets and `gtk::Editable` positions in **characters**. Convert both ways at the
  shell's edge, or the caret is off by one for every emoji before it.
- **Picture fit** (w1 board): `fc_core::board::board_picture::CONTENT_FIT` is `"contain"` —
  `gtk::ContentFit::Contain`.

- **Languages** (w1 text): every wording `fc-text` produces (call records, notifications, sizes,
  the cap footer, picture notices) reads ONE thread-local language, so the shell must call
  `fc_text::i18n::use_lang` at start-up and on every change. Its own sentences go through a
  `fc_core::text::string_catalog::StringCatalog` set to the same language. (Done in S0:
  `services::catalogue` calls `use_lang` and loads `linux/data/i18n`, written by
  `linux/i18n/generate.py` from the xcstrings + `linux/i18n/linux.json` + `win/i18n/win.json`, in
  the CI string gate.) Catalogue calls must be made on a value named `say`
  (`say.get/format/plural("…")`) — the catalogue tests and the generator scan `linux/crates` and
  `linux/app/src` for that.
- **Page charsets** (w1 text): `text::link_preview::page_reader::decode` knows UTF-8/16, ASCII,
  Latin-1, windows-1252/1251 and reads anything else as UTF-8; the shell may hand other charsets to
  GLib's `g_convert` instead.

- **Rich page** (w2 logic-base): `RichPage::is_page` is exact, while shell-design §9.1 says "and its
  own fragments" — pick one. §9.4's scale is not in `RichJob::key` (it matches Windows): fold the
  scale into the version string the shell passes.
- **Keys** (w2 logic-base): `CapitalField::classify_keysym(keysym, ctrl, alt, super)` is the Linux
  entry; Ctrl (with or without Alt) and Super are chords, AltGr (ISO_Level3_Shift) is not.
- **Compositing time** (w2 anim): an animation is composited on the main context, yielding between
  frames — time a long, large GIF on a real desktop.
- **Autostart** (w2 logic-base): an administrator's `/etc/xdg/autostart` entry decides first (the
  design's table), though XDG lets a user file override it — check on a real desktop.

- **Animations** (w2 anim): the shell implements `animation_preparing::StillDecoder` (gdk-pixbuf/glycin,
  straight BGRA) and `AnimationEncoder` (GStreamer, NV12 in, **no `faststart`**, read back with the
  discoverer) from their docs, and uses `animation_preparing::frame_source` for animated stickers too
  (gdk-pixbuf does not animate APNG). Link previews need their OWN soup session with
  `SOUP_MESSAGE_NO_REDIRECT`, no cookies and no credentials (`LinkPreviews` follows https redirects
  itself — `HttpResponse` carries no final URL). `FolderMediaStore` gets
  `$XDG_STATE_HOME/family-connect/media/`.

## For the real-desktop checklist

- **The note faces** (w1 board): `fc_core::board::note_look::font_family` lists Ubuntu 26.04's
  desktop fonts. The "casual" hand falls back to Comic Neue / URW Gothic, which a stock install
  may not have, so it may draw like "plain". Check on a real 26.04 desktop and decide.

## For the shell's libsoup seams (w1 api/socket — implement `fc_core::rt` in the app)

(Done in S0: `app/src/services/{soup_http,soup_socket,glib_clock}.rs`, each rule below tested —
the socket against libsoup's own server on loopback. Kept here as their specification.)

Done in the core: the no-envelope rule (`ApiClient::failure`), `escape_data_string` for every query.

- **HTTP (`rt::Http`):** one `SoupSession`, its `timeout` left at 0 — the core races every request
  against its own deadline (100 s ordinary, 120 s backdrop, 310 s transcript) and also passes it in
  `HttpRequest::timeout` (enforcing it in the seam is optional). A non-ASCII host goes through
  `g_hostname_to_ascii` (IDN). Headers appended as given; the body via
  `soup_message_set_request_body_from_bytes(msg, content_type, bytes)` — do not append
  Content-Type twice. `send_and_read_async` with a `GCancellable` cancelled when the future is
  dropped (dropping IS cancelling — a deadline winning drops it). Any status is `Ok(HttpResponse)`
  with all headers and the whole body. Errors: TLS/certificate → `Tls`; resolver, refused,
  unreachable, reset, closed → `Unreachable`; `G_IO_ERROR_TIMED_OUT` → `Timeout`; else `Other`.
  Error text never carries a token or a URL with a path. System trust store, no pinning, no
  invalid certificates; plain http to any host allowed; default redirects; no cookie jar.
- **WebSocket (`rt::WebSocketConnector`/`rt::WebSocket`):** `soup_session_websocket_connect_async`,
  `Authorization: Bearer` only when `bearer` is `Some`, no subprotocols, origin NULL, never the
  token in the URL, cancelled on drop. A refused upgrade with an HTTP status ≠ 101 →
  `TransportError::UpgradeRefused(status)` — **401 MUST arrive as `UpgradeRefused(401)`: it is what
  signs the user out.** On success: `keepalive-interval` 30, `keepalive-pong-timeout` ≈ 45 (a dead
  peer closes inside the server's 75 s idle window — the protocol's and the Mac's rule; Windows never
  notices), `max-incoming-payload-size` 0 (the 128 KiB default closes with 1009). Receiving is a
  QUEUE owned by the socket, filled by the "message" signal (BINARY decoded as lossy UTF-8),
  including messages before the first `receive()` (Connected fires first); a dropped `receive`
  future loses nothing; `None` once closed and drained. "closed" records the code (0 → `None`) and
  wakes the waiter; only 4401 ends the session. `send` = `send_text` when OPEN and `Ok` at once,
  `Err` otherwise. `close()` idempotent: 1001 if OPEN; the same on drop. Never log the socket URL.
- **Clock:** GLib timeouts; `now()` = `OffsetDateTime::now_utc()`. **TokenStore:** libsecret; the
  client reads the token on every attempt and every connection.

## Decided while porting (tell nettrash)

- **Linux follows `fc-text` where Windows diverged from it** (w1 text — `fc-text` is the shared Rust
  the web client runs and matches the Mac): greeting places drop a repeated name on the client
  (Windows leaves it to the server — same result on the server); an assistant's sources footer
  takes any number of links and credits, as the Mac's `AssistantSources` does (Windows: at most 3);
  `hr`/`bs`/`sh` read as Serbian Latin (Windows: English). The two C# tests that pin Windows' side
  are ported and `#[ignore]`d with the reason.

## Found in the other clients — for nettrash to decide (not fixed by this port)

Measured by running the C# on .NET 10 (w1 protocol; oracle copied to the session scratchpad):

1. **Windows: a malformed frame drops the socket.** `ServerFrame.Parse` throws on a wrong JSON type
   in a string or bool field (`"call_id":7`); `ChatSocket.RunAsync` treats it as a transport
   failure — disconnect, back off, reconnect, full resync. The Linux port reads the field as absent.
2. **Windows: a frame with a duplicate key ends the socket loop** — `JsonNode`'s ArgumentException
   is outside `RunAsync`'s catch filter.
3. **Windows: a non-2xx JSON body without `"error"` throws a NullReferenceException** in
   `ApiClient.Failure` that nothing catches.
4. **Windows: an absent `ai_history` reads as false**; docs/protocol.md ("Objects") says the default
   is true, and the Mac and the web client read true. Linux follows the protocol.
5. **Windows: missing non-nullable arrays decode to null** (`PollDto.Options`, `PollOptionDto.Votes`,
   `MessageReactionsDto.Reactions`) and can throw later (`Text/Polls.cs`). Linux reads `[]`.
6. **Windows: `ReconnectBackoff`** gives a negative wait for a negative random value and throws on
   NaN (only an injected source can do either). Linux waits zero.
7. **Windows: the SQLite cache's JSON also carries computed properties** (`is_photo`, `media`, …).
   Harmless — nothing reads them back.
8. **`ServerUrl.cs`'s comment is wrong**: the Mac keeps a pasted path and refuses plain http to a
   non-local host; Windows drops the path and accepts any http host. Linux follows Windows (its
   tests pin it). Whether the two should agree is a protocol question.
9. **Windows vs `fc-text` (w1 text):** greeting-place repeats, the sources footer's 3-link limit,
   `hr`/`bs`/`sh` → English, `%d`/`%s` in AppleFormat, the decimal separator in file sizes, and four
   animated-sticker edge cases (`VP8X` not first, a non-animated GIF, a cut-short GIF's loop block,
   `ANMF` durations without `VP8X`). Windows and the web client disagree on these today.
10. **Windows: a WebSocket upgrade refused with 401 is retried for ever and never signs out**
    (measured: `ClientWebSocket` throws with no close code; waits 1, 2, 4 … ≤ 30 s, no expiry). The
    protocol says a 401 means the session is gone, and the Mac signs out. Linux signs out.
11. **Windows: `Avatar` and `Download` never retry**, though the class promises one retry for every
    GET that fails transiently. Ported as is — decide.
12. **Windows: uploads run under the ordinary 100 s deadline** — a 90 MB video needs ~7.2 Mbit/s of
    uplink to make it. Ported as is — decide.
13. **Windows: no socket liveness check** — only `KeepAliveInterval` 30 s without `KeepAliveTimeout`,
    so .NET never notices a dead peer (from .NET's documentation, not measured). The protocol asks
    for one; the Mac pings every 25 s with a 75 s limit. Linux sets libsoup's pong timeout.

14. **Windows: the chat's "@" mark is raised for ANY mention** — of somebody else, by a blocked member,
    in the reader's own message — and reading past one's own mention can leave it set with 0 unread
    (measured). docs/protocol.md ("The mark on the chat row"), the Mac and the web client mark only an
    unread message naming the reader from somebody not blocked. Linux follows the protocol.
15. **Windows: a negative `user_version` crashes the cache open** (IndexOutOfRangeException); Linux
    reports an unknown schema. **`Times.Rfc3339` throws outside years 1–9999** (damaged cache only).
16. **Windows: a cache failure while a frame is applied ends the socket for good** (w1 sync, from
    reading `ChatSocket.cs`: its catch filter takes WebSocket/HTTP/IO/Timeout/InvalidOperation
    exceptions only, and `router.Hear` and `SendPipeline.Heard` run inside it). A `SqliteException`
    (disk full, a locked file) escapes `RunAsync`: no reconnect, no resync, until the app restarts.
    Linux: `FrameRouter::hear` answers the error for the subscriber to log, and the pipeline reports
    it on `on_cache_failed`; the socket goes on.
17. **Windows: `TranscodeAttempts.FirstTakenAsync` ignores a cancel raised during the last attempt**
    (measured) — it can stage the original for somebody who just cancelled. Linux: a cancel is always
    a cancel.
18. **Windows: `AnimationMp4.ExactTicks` throws on an empty `tkhd`, and its `long` arithmetic wraps
    silently** (a huge v1 `elst` media time is written back wrong); `StickerAnimation.DecodedBytes`
    wraps too; `RichJob.Json` throws on a non-finite font size. Measured; Linux returns nothing,
    saturates, or writes `null` (which fc-rich.js reads as 16).
19. **Windows calls (w2 calls, each measured with the C# suite's own fakes):** (D1) a placement
    cancelled while the mic/camera prompt is open closes the media of an incoming call that started
    ringing meanwhile — it rings silent; (D2) a late `call_ice` for an ended call is added to the NEXT
    call's connection; (D3) `call_end` plus the error refining it records the call twice, so the
    oldest of 16 remembered calls is forgotten and rings again on a replayed offer; (D5) a media that
    reports its own close as failed ends the call twice (recursing to a stack overflow if every close
    fails). Linux fixes all four. Kept and pinned: (D4) Decline on an ended call sends `call_end`
    again (the server ignores it).
20. **Mac vs Windows call behaviour differs** (refusal wording, ICE-fetch failure, when Connecting
    starts, offer/candidate retries, frames for a call already over). Linux follows Windows — its
    tests pin it. Whether the clients should agree is yours.
21. **Windows: a cancelled flush loses owed previews** (w2 found, by reading): `ApiClient.Unreached`
    lets a real cancel out of `UploadPreview` as an exception, losing a fresh poster in flight and,
    in `RetryPreviewsAsync`, every due preview after it. Linux keeps them owed.
22. **Windows: `LinkPreviews` leaks card images** (measured): stored under the address the page landed
    on, evicted by the address asked for, so a redirected link keeps its image (up to 4 MB) for the
    life of the process. **`FolderMediaStore.Read` throws** on a `meta.json` that is not an object,
    out of its catch filter into the media pump. **`GifPicture.Pixels` throws `OverflowException`**
    for huge images, and APNG canvases past 2³¹−1 wrap. Linux fixes all four.
23. **The session layer (w2 logic-session):** (a) Windows' `AppSession.Clear` cleared the token and
    then let a failed `WipeAll` throw before publishing — a signed-in screen with no token behind
    it; Linux publishes the signed-out state and raises the ending first, then answers the wipe's
    error (by reading). (b) Windows never translates the four report reasons ("Spam",
    "Harassment", "Inappropriate", "Something else"): `FamilyText.ReasonLabel` passes the key
    through a variable, which `win/i18n/generate.py` does not collect — measured: absent from all 8
    of `win/`'s tables though `Localizable.xcstrings` has them; Linux reads the same tables, so the
    same gap until its catalogue generator exists. (c) The roster sorts by lower-cased name,
    code point by code point, on Windows, the web and Linux; the Mac's SwiftData `SortDescriptor`
    sorts localized-standard ("Bob 10" before "Bob 2" here, after it on the Mac) — by reading.
    (d) `BubbleBody.Openable` also needs `Uri.TryCreate(…, Absolute)`; `fc_text::links::is_openable`
    checks the scheme alone — Linux follows .NET (30 vectors measured, `bubble_body_tests.rs`).
    (e) `fc-text` differs from `win/` in rules this package ports as the C# writes them:
    `families::shows_switcher` (draws it with NO family and something pending), `groups::may`
    (also asks the reader be in `member_ids`), `groups::create_refusal`/`change_refusal` (other
    sentences), `pack::may_remove` (reader 0 may remove an item added by 0), `pack::could_be` (an
    unknown size may match), `pack::plan` (no "full" step; remakes a WebP/PNG over the byte ceiling
    that already fits 512 × 512, which the C# refuses). (f) `PackModel.AddAsync`: a claim answered
    without an attachment is "no answer" as a transport error, so the window says the sticker
    "didn't reach the server" though the server answered — ported as is (by reading).
24. **The recording package (w2 logic-rec):** (a) Windows' rate ladder, its AAC pass-through
    (`KeepsAudio`) and the doubled "upright" mix attempts exist only because of Media Foundation and
    `MediaComposition`; Linux asks each target once at its own rate, High (`x264enc`) then Main
    (`openh264enc`), never passes a track through, and judges a mix like a transcode. It also refuses
    a result whose H.264 profile no player plays (High 10/4:2:2/4:4:4), which Windows never checks.
    (b) `fc_text::transcript` disagrees with the C#, the Mac AND the server: it compares a stored type
    by its essence (the server compares exactly), lets an unstated or 0 size use the stored copy,
    does not hold `transcribe_max_bytes` to 25 MiB, and says "Try again" for 404
    `message_not_found`/`attachment_not_found` and 409 `blocked` (the C# and the Mac: "Not
    available"). Linux keeps the C#/Mac answers there and calls fc-text only for `may_ask` — so for a
    reader not yet known (id 0) Linux does not offer "Show text" where the C# and the Mac do. (c) The
    Mac refuses "Show text" on an animation (#96); Windows does not, and Linux ported Windows.
    (d) `container_sound` keeps Windows' strict `trak` reader: on five malformed files it answers
    "unknown" where `fc_text::mp4_read::playing_sound_tracks` answers 1, 0, 1, 1, 1 (measured).
    (e) Measured/by reading, fixed on Linux: `RoundLook.ViewerDiameter` answers NaN for a NaN chrome
    and `VoiceLook.BarHeight` for a NaN height; `MediaEncoding.TranscodeCeiling` throws on a huge
    duration (and the Rust `animation_preparing` copy overflowed — it now delegates to
    `media_encoding::transcode_ceiling`, which saturates); `ComposerStaging` waits for a preparer that
    ignores the cancel, and a finishing batch clears a later batch's Preparing state;
    `ParkedRecordings.Park` answers ticks the disk keeps as milliseconds. (f) Windows never translates
    the video button's label (`DoorLabel` keeps the key — no Windows build drew it); Linux does.

## From P4 (board, family, settings)

Built in wave 3 (P4): `app/src/settings/` (install, routes, Settings rows, dialogs), `app/src/family/`
(the Family dialog and the owner's console, member menu, birthday editor, New Group / Group Info),
`app/src/board/` (wall, note sheet, calendar file), `app/src/windows/{settings_window,board_window}.rs`,
self-test groups `settings`, `family`, `board` (32 checks, green). What a later step must act on:

- (Done at integration: "Group info" is `FamilyRoutes::group_info` → `family::groups::group_info`
  from P1's header; P2's pictures in every P4 face (`settings::ui::face`); the Family dialog's
  **Family Stickers** → Manage… (`attachments::pack::present`, where `pack::offered`); Settings'
  shortcut "Change…" (`HotKeyState::configure`, `ConfigureShortcuts`); the profile photo
  Add/Change/Remove through `attachments::avatars::prepare_upload`. Self-tested: `chat.group_info_*`,
  `family.member_row_shows_the_picture`, `family.family_stickers_opens_the_pack`,
  `desktop.hot_key_change_asks_the_desktop`, `settings.profile_photo_added_and_removed`.)
- (Done at integration: `Shell::numbers` — the decimal separator and digit grouping of `LC_NUMERIC`
  (`services::catalogue::Numbers`, unit-tested) — replaced the three copies of the language's
  separator in Statistics, P2's file rows and P1's chips; Statistics groups its counts.)
- (Done in the Mac-parity pass, 2026-10-30: Settings' **Assistant** and **Looking things up** groups
  (`settings::rows::{assistant_consent, lookup_consent}`, `composer::consent::ask_lookups`); the
  Family dialog's Assistant settings and the greeting's places (`family::assistant`); an event's
  backdrop drawn on its card and "Draw a backdrop" (`board::wall`, `backdrop_consent::run`);
  Statistics' "Recording time".)
- **Not done yet:** the note sheet's live preview (`note_preview::of` — the card builder `Wall::card`
  needs a connection and would have to be factored out first) and its `@` suggestion strip.
- **The string gate and variables:** `family_text::REASONS` and `house_rules::POLICIES` are looked up by
  `say.get(variable)`, which `linux/i18n/generate.py` cannot collect — so they were English-only on Linux
  too (23(b) below). `family::words` says them as literals, pinned by a test to fc-logic's and fc-core's
  lists; fc-logic/fc-core could do the same and drop the workaround.
- **Real desktop (§C, not verifiable in the container):** "Add to Calendar" opening GNOME Calendar's
  import through `gtk::FileLauncher` (nothing in the image takes `text/calendar`); Pin a Photo's portal
  file chooser and a real upload; Privacy Policy / Support through `gtk::UriLauncher`; the board's drag
  with a touchpad and a touchscreen (long press for the menu); Orca reading a card's label; the casual
  hand's face (the line above).

## From P2 (attachments and rich)

Done by P2 (wave 3), so these earlier lines can go at integration: "Rich page" (is_page kept EXACT,
shell-design §9.1 corrected; the scale is folded into the version string, `rich::renderer::key_for`),
"Picture fit" for the viewer (`ContentFit::Contain`), "Animations" for animated STICKERS
(`animation_preparing::frame_source` + a gdk-pixbuf `StillDecoder` in `attachments::stickers`), and
the link previews' own session (used through `Shell::link_previews`, untouched).

For the integration — done (P1 draws the whole block with `tiles::media_block`, every piece through
`shell.slots()`; avatars, the sticker panel and a pick that sends, Location, Open Full Screen,
Save…, `ChatRoutes::stage_files`, a circle's expand, the viewer's video and the animation tiles on
P3's real player, the Family dialog's pack — each checked in the `chat`/`family` groups'
integration checks, `selftest/chat/wiring.rs`). Still for others:

- **P7**: install `linux/data/rich/` (an exact copy of `web/rich`, 5.4 MB, `fc-rich.js` VERSION
  `1;katex-0.17.0;mermaid-11.16.0;viz-3.24.0;plot-md-1.6`) as `/usr/share/family-connect/rich/` in
  the .deb, the .rpm and the tarball's `share/family-connect/rich/`; keep it byte-identical to
  `web/rich` (a CI diff is cheap). The `.desktop` `MimeType=` of §8.6 now has its receiving side
  (`Inbox::receive`).
- **S0** (request, not blocking, still open): a banner slot on the main window's content pane (P2
  puts the sandbox banner into `content_header().parent()` as an `adw::ToolbarView` top bar). (The
  shared number format is done: `Shell::numbers`.)
- **Re-parenting the renderer** (still open) (§9.3) into a conversation or board window when the main window is
  hidden needs an overlay accessor on those windows (P1's, P4's); until then jobs wait for the main
  window to be mapped.

- **A JPEG is written from a pixbuf WITHOUT an alpha channel** (measured in `fc-linux-dev`, P2's
  avatar upload): `save_to_bufferv("jpeg", …)` of an alpha pixbuf (from `composite_color_simple`)
  produced nothing; compositing onto an RGB pixbuf filled white works
  (`attachments::avatars::prepare_upload`). P3's posters and anything else writing JPEG: the same.

For the real-desktop checklist (§C), from P2: §C 1–4 (the renderer under a real bwrap, the banner
when the profile is unloaded, HEIC and animated WebP under glycin's sandbox), 28 (the viewer's video
streams, seeks and Save… through the portal's chooser — needs P3's player), 31 (avatars, tiles, maps
and drawings crisp at 125 % / 200 %; the drawing key carries the integer scale), 32 (the viewer's
picture on NVIDIA), 39 (the Location portal on a stock desktop), 42 (viewer windows close on Quit).
Also: whether "Open With" from Files lists Family Connect for the §8.6 types and a file arrives in
"Send to" (Docker has no Files).

Found while building P2 — for nettrash to decide (not changed in the shared clients):

1. **`fc-rich.js` measures a formula before its fonts have loaded**: the first formula of a fresh
   page was photographed cut off on the right on Linux (measured). Linux warms the page (KaTeX and
   every `document.fonts` face) before its first job; WKWebView and WebView2 may show the same cut on
   a cold first formula (by reading — not measured there). A fix in `fc-rich.js` itself
   (`await document.fonts.ready` before measuring) would cover every client but changes VERSION.
2. **Windows opens a shared place in Apple Maps on the web** (`MediaText.MapsUrl` →
   `maps.apple.com`, by reading `LocationElement`) though its map is OpenStreetMap's; Linux opens
   openstreetmap.org (§5.2, D11).
3. **Windows' `ShareInbox.Unique` keeps Win32 device names** (a shared `CON.png` cannot be created
   on Windows; by reading) — Linux names copies with `attachment_files::safe_file_name`.
4. **Windows' `MapTiles` names itself `FamilyConnect-Windows/1.0`** whatever the build's version
   (by reading); Linux sends the workspace version.

## From P3 (media)

Found while building `app/src/media/` (wave 3, P3); each line names who must act.

- (Done at integration: `ComposerHost`'s four defaulted methods — `reply_to`/`release_reply` (a
  parked recording keeps its reply), `notice` (P3's refusals on P1's line), `send_video_message`
  (`round: true`, P1's `send_media(…, round)`); `Shell::close_guard` asked by `Shell::quit` →
  `media::recorder_layer::may_close` ("Delete video message?"); `prepare_picked` in P1's
  `prepare_file`; a note in REVIEW parked by P1's `Composer::park_review`; the circle's expand;
  `AccessCamera` through `desktop::portal_request`; ONE keep-awake, `desktop::keep_awake::hold`, for
  recordings and calls (each hold its own cookie). Self-tested: `chat.picked_video_*`,
  `chat.note_in_review_*`, `chat.recording_refusal_*`, `chat.video_message_sent_round_*`,
  `chat.circle_expand_*`, `media.a_quit_over_a_take_asks_first`.)
- **Not proven in the container:** `AccessCamera` through `portal_request` (no Camera portal on the
  self-test's bus; the swap is compiled and the fallbacks are what runs).
- **P6 — the camera for calls**: `Shell::get().camera.get().open(device)` answers a source `Element`
  (a `pipewiresrc` that owns the portal's fd, a monitor device's element, a `v4l2src` — which may
  deliver MJPEG, so put `decodebin ! videoconvert` after it — or, in a `selftest` build, a `Bin` with a
  ghost `src` pad). Its caps are not fixed: add `videoconvert ! video/x-raw,format=I420` (or your
  own). `Denied` is the person's no — never fall back; `Absent` no camera; `Failed` anything else.
  Each `open` asks the portal again (GNOME prompts only the first time). The video recorder closes
  its camera on a call (the session watch's `Call`), before a call reaches for it.
- **"Show text" (transcripts), done 2026-10-29**: `media::transcript` draws it under a voice note
  and a circle (and answers the message menu's `offers_text`/`show_text`) from `TranscriptModel`;
  the consent question is `composer::consent::ask`, a free function the composer uses too; a
  circle's sound for the request is `media::sound_tracks::GstSoundTracks` (`SoundTrackMaker`, via
  `transcript_sound::device_source`). Self-tests: `media.voice_text_shown_and_folded` (held text,
  no server), `media.circle_sound_for_its_text`. Against a real server's transcriber: not yet.
- **Real desktop (§C)**: whether `pipewiresrc` posts `RESOURCE_ERROR_BUSY` for a camera another app
  holds through PipeWire; whether `notify::suspended` fires on GNOME's minimise; the portal's prompt
  text; Orca on the waveform slider and the circle.

## From P5 (desktop)

Built in wave 3 (`app/src/desktop/`, self-test group `desktop`, 53 checks green in `fc-linux-dev`).

- (Done at integration: P3's recordings and P6's calls hold `desktop::keep_awake::hold`; nothing
  else calls `gtk::Application::inhibit`.)
- **P6 — ringing** (§14.4): `shell.notifier.get().ring(call_id, title, body)` when a call starts to
  ring and `stop_ringing(call_id)` when it leaves Incoming; P5 also sets the icon's `NeedsAttention`
  and the dock's `urgent` from them, so P6 calls `TrayState::set_ringing` / `BadgeState::set_urgent`
  never. The call notification's body click is `app.return-to-call`: `CallRoutes::return_to_call`
  must present the Call window while it rings, too.
- **P4 — Settings rows**: "Keep running in the top bar" is insensitive while
  `!shell.tray.get().available()` (follow `on_available_changed`); the shortcut row reads
  `shell.hot_key.get().state()` / `on_state_changed` with `global_hot_key_rules::{title, footnote,
  is_changeable}` and only writes the `hot-key` setting (S0 calls `apply`); the login row reads
  `shell.autostart.get().state()` and calls `set_enabled`, judged by `startup_setting::*`. §15.5's
  footnote ("If no notifications appear, check Settings ▸ Notifications ▸ Family Connect.") is P4's
  sentence (the main tree's tables held it when P5 finished).
- **P7 — packaging**: the tray's icon `me.nettrash.FamilyConnect-symbolic` (an SVG in
  `hicolor/symbolic/apps/`, and in the tarball's `share/icons`, which the item names as
  `IconThemePath`); `DBusActivatable=true` in the `.desktop` entry; and
  `/usr/share/dbus-1/services/me.nettrash.FamilyConnect.service` (`[D-BUS Service]`,
  `Name=me.nettrash.FamilyConnect`, `Exec=/usr/bin/family-connect --gapplication-service`) — without
  both, a notification clicked after a quit opens nothing (§2.2).
- **S0**: `scripts/dev.sh`'s keyring was never unlocked — `printf ''` creates no login keyring
  (measured); P5 changed it to `printf '\n'`. (`Notifier::show`'s rustdoc corrected at integration:
  the GNOME backend carries no category.)
- **For the real-desktop checklist** (§C 12–23): whether Ubuntu's AppIndicator extension reads
  `IconAccessibleDesc` (Orca on the icon) and sends `ProvideXdgActivationToken` before `Activate`;
  whether GNOME's portal returns a kept binding from `ListShortcuts` on a NEW session (else it may ask
  at every launch, §C 16); the GLib freedesktop backend path (served by the self-test's fake, but GLib
  chose the GNOME backend in every run).

## From P1 (chat)

For the integration (W.2 order "P2/P3 into P1's slots"), S0, P2, P3, P4 and the real desktop.

- **MEASURED — glycin's JPEG encoder refuses RGBA** (Ubuntu 26.04, gdk-pixbuf 2.44 over glycin):
  `Pixbuf::save_to_bufferv("jpeg", …)` on a pixbuf with alpha fails with "The encoder or decoder for
  Jpeg does not support the color type `Rgba8`". Every PNG decodes with alpha, so a JPEG made from a
  picture must be flattened onto an opaque pixbuf first (`composer::integration::opaque`, tested). P2's
  avatars (`AvatarRules`' square JPEG, shell-design §8.3) and anything else that re-encodes JPEG hit
  the same wall.
- **MEASURED — never parent a popover to a `gtk::TextView`.** GtkTextView takes every child for one of
  its own; on dispose it loops for ever warning "GtkPopover is not a child of GtkTextView" (the
  self-test's log grew 13 GB in six minutes). The composer's `@` list is parented to the composer's box.
- (Done at integration: the fake answers `401 unauthorized`, and the `chat` group reaches the server
  through the probe (`chat.gate_server_probe_opens_the_server`); the `shell` group puts back the
  routes it found; `win.message-delete`'s row in `seams/actions.rs` says what it means; every
  integration point of `composer::integration` and `chats::avatar` is wired but `offers_text` /
  `show_text`; `FamilyRoutes::group_info` and the header's Group Info, Call and Video Call; a call
  record's "Call back".)
- **Selectable labels take the focus and select their words** (`gtk-label-select-on-focus`): a window
  or a navigation push that focuses "the first focusable widget" lands on a bubble's words and
  highlights them. P1 gives the composer's field the focus where that would happen (a conversation
  window, the one-pane push) and clears the focus before replacing the content pane. Watch for it in
  P2's viewer and P4's dialogs.
- **Drafts are in memory, words only** (the Mac's `ComposerDrafts`; shell-design §6.6 corrected).
- **For the real desktop (§C)**: IBus composition and Enter (C33), the dead-key capitals (C34),
  Ctrl+. (C35), pasting a screenshot / an image from Firefox and dragging from Files (C36), Orca on
  the list and the bubbles (C37), large text in the composer (C38) — none can be seen under Xvfb.

Found and fixed at integration (P1's files): the composer never released the media store's pinned
handles after a send was queued (`FolderMediaStore::release` — the sweep could never judge them);
the chat list kept a person's initials when their picture's version changed (a new picture changes
no chat row — the row now re-asks its face on every redraw); and a long conversation redrawn whole
(`Timeline::clear` — the dark scheme, the roster, the block list) showed its FIRST rows while the
adjustment said the newest, and a newest row that grew after `scroll_to_bottom` stayed below the
composer. Cause: GtkViewport notifies `upper` from inside its own allocation and drops the
allocation the timeline's new value asks for; `Timeline::move_view` re-queues it from an idle
(shell-design §5.1; two self-test checks compare the drawn row with the view's bottom and fail
without the fix).

Found in the other clients (for nettrash):

1. **The Mac's (and iOS's) Pending page says "…or pull down to check right now"**
   (`PendingApprovalView`, shared with the Mac): a desktop has no pull to refresh. Linux shows the same
   translated sentence (it refreshes every 15 s, as Windows does); a desktop wording is the owner's call.

## From the Mac-parity pass (2026-10-30)

A review of the Linux client against the Mac (`ios/FamilyConnect/MacViews`, shared `Views`/`Core`)
found and fixed, each verified in code and by the gates:

- **Bugs:** a hidden (blocked) message's menu offered Copy/Reply/React (now Safety only, and no
  double-click heart); "Close poll" did nothing in a thread; the unread divider re-read the live count
  ("1 new messages" after reading); the Open polls badge never updated; a "not sent" voice note was
  sent without its own reply and caption; a voice note whose send could not be queued was lost (it
  goes back into review); the note sheet did not re-judge after a list or date change; pasted video,
  sound and GIF skipped `prepare_picked` (now through a temporary file).
- **Behaviour now the Mac's:** notifications muted only for the chat being read (§15.1) and withdrawn
  once a chat is read anywhere; a returning network dials at once (`ChatSocket::kick`,
  `LiveConnection::reconnect_now`) and retries the Offline page; Record Voice/Video Message greyed
  where they would do nothing (`composer_button::{records_voice, records_video}`); Settings… on every
  page's menu; F11 full screen (`win.toggle-fullscreen`, a Linux-only "Full Screen" string).
- **Added from the Mac:** "See who reacted" and "See who voted"; a tint on the message a quote jumps
  to; a spinner while older messages load; a thread's loading / "Couldn't load the thread" states; the
  Open polls dialog drawn with the full poll card and "Nothing to decide"; the reply's excerpt in the
  composer banner; the assistant's notices over the composer (consent line with Review…, picture
  notices, the `/draw` hint — `composer::notices`); poll sending with consent, reply and mentions;
  staged thumbnails and a voice-note review chip; "We can't hear anything…" while recording; "Show
  text" under videos (numbered in a pile); call records' direction/outcome glyphs; Delete Account's
  full consequences; "How to run your own server"; live pending invitations; note words fitted by
  scaling the type (`fitted_scale`, floor 0.6).
- **For nettrash (not changed):** Linux keeps Windows' rules where the Mac differs on (a) the top-bar
  icon — always registered, and a login launch starts hidden whenever an icon exists, where the Mac
  shows no icon and never starts hidden unless "Keep running" is on; (b) one main window (no File ▸
  New Window). Still not done from the review: opening the cached chats offline at launch, "Add to
  family stickers" from a sticker, dragging an attachment out, a diagram's zoom sheet, pausing on a
  headphone unplug, `drawn_copies` (a voice note in the thread and the conversation at once), the
  video recorder's reply banner, dropped web links, the note sheet's preview.

## From P6 (call media) — merged 2026-10-29

Done: `app/src/calls/` (`GstCallMedia` on `webrtcbin`, the ring tones) and `tools/call-interop/`
(a workspace member, not a default one: `scripts/dev.sh test` builds, lints and tests it; `run.sh`
runs the matrix — Linux↔Linux, ↔Chromium 140, ↔Firefox 141, H.264-only, mute/camera-off,
teardown, a vanished peer, TURN through coturn: 29/29 in Docker). The measured facts and the six
deviations are in `calls/mod.rs` and in fc-logic's "The webrtcbin `CallMedia`" (updated). The
engine now repeats a mute or a camera turned off while the microphone was being asked for once the
media is connected (`repeat_toggles`; a pipeline did not exist to take it before).

Still to build — DONE by the second half (see "From P6 (Call window)" below): the Call window, the
ring's notice through `shell.notifier`, `CallRoutes`, keep-awake in a call, ONE `GstCallMedia` in
`shell.call_media` with `camera::ShellCamera` over `shell.camera`, the call buttons' functions for
P1, and the self-test group `calls`. `main.rs` no longer allows anything on `mod calls`; the media's
items only the harness or the self-test use carry their own `allow(dead_code, reason = …)`.

Not taken: an fmtp-aware H.264 chooser in fc-logic's `sdp` (`payload_type(…, "H264")` answers the
first H.264 line; the media's own `pipeline::h264_payload` prefers constrained baseline with
packetization mode 1 — enough while only the media chooses).

## From P6 (Call window) — 2026-10-29

Done, under `app/src/calls/` beside the media: `call_view` (pure, unit-tested: what the window draws,
when it comes forward, when a call rings on the desktop, when the machine is kept awake, which route
applies), `call_window` (the Mac's window, §14.3), `call_notice` (the ring told to P5's seams),
`keep_awake`, `camera` (`ShellCamera`: P3's `Camera` as the media's `CallCamera`, the `video-camera`
setting picking the device, a refusal a camera-off call), `routes` (`WindowCallRoutes`, Return to
Call), `hub` (the one part following the current connection's engine), and `calls::install` — one
line in `app::install_packages`. `selftest/calls.rs` is the group `calls`.

- **`tools/call-interop` compiles the four media files only** (`ice`, `media`, `pipeline`, `tones`,
  through an inline `#[path]` module in its `main.rs`), no longer `calls/mod.rs`: the folder's other
  half needs the app. The media is unchanged but for `allow(dead_code)` on what the app never calls
  (`NoCamera`, `SilenceRule::Always`, `with_silence_rule`, `with_offered_video`, `pipeline`,
  `agent_facts`, and `stats`/`ringing` outside the self-test).
- (Done at integration: P1's header Call / Video Call and a record's "Call back" use `calls::{offered,
  can_place, place, on_changed}`, recording read from P3 (`conversation::page::recording_now`) —
  `chat.call_buttons_*`, `chat.call_back_places_a_call`.)
- **P3's camera in a call** (`calls::camera::ShellCamera`): whatever `Camera::open` answers is handed
  to the media inside `source ! decodebin ! videoconvert` (one raw `src` pad), because a `v4l2src`
  may deliver MJPEG and the media's camera branch starts with `videoconvert`; the self-test's camera
  for Alice SENDS MJPEG (`jpegenc`) to prove it. `Denied` is never retried on another device. The
  media files are untouched by this.
- (Done at integration: the Call window's face gets P2's picture (`CallWindow::avatar`, set by the hub
  for the peer); `calls/keep_awake.rs` folded into P5's `desktop::keep_awake` — each `AwakeHold` has
  its own cookie, so a recording ending in a call never releases the call's.)
- **For P5**: `Notifier::ring(call_id, title, body)` / `stop_ringing(call_id)` are called exactly once
  per change (the stand-in recorded them); title "Incoming call" / "Incoming video call", body
  "<Name> is calling" (the Mac's words). `TrayState::set_ringing` and `BadgeState::set_urgent` follow
  the same edge.
- **Decided here, each from the Mac (shell-design §14 corrected)**: the window's title is the person;
  Escape declines a ring and Enter answers it (the Mac's `.cancelAction` / `.defaultAction`; §14.3
  said there were no keys); a video call's window is dark, with a pill carrying the name and the
  duration once the far side's picture is up; a video call up without a camera of ours shows the
  Mac's footnote and an insensitive Camera disc; the window comes forward on a new call and on every
  live stage, but an ending only updates a window that is up (the Mac reopens a closed one to say
  goodbye — Linux does not); keep-awake from the answer to the end, not while it rings.
- **Not verified here** (§C, real desktop): 5–11 (audio/video against real peers, the notification's
  buttons through GNOME Shell and their activation token, a headset mid-call, TURN-only, Return to
  Call), 20 (Do Not Disturb vs an urgent call), 21 (the dock's urgency).

