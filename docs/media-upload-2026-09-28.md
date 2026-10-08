# Optimised video and audio upload — what it would take (issue #74)

**Assessed 2026-09-28** against `v1.2` (branched from the 1.1 release, `d5a45bf`), in answer to issue
#74: *"Client should transform the source to minimal allowed resolution and format to keep quality
and minimise size."* Labelled for iOS, macOS, Android, Windows and web. This document says what the
request implies, which facts were checked rather than assumed, where the hard parts are, what design
is recommended and why, and what has to be decided before any code is written. **No code has been
written for it.**

**Decided 2026-09-28: every recommendation below was accepted as written.** The profile and rules are now normative in `docs/protocol.md`, "Preparing media before upload", in the exact arithmetic four ports need to agree on; this document stays as the reasoning.

---

## What is asked, and what it implies

- **Every client transforms before it uploads** — not only when a file is too big, which is what
  happens today on two clients and not at all on the other two.
- **A "minimal allowed" target** — one resolution and one format that every client aims at. That
  target has to be written down somewhere every client reads, which in this project is
  `docs/protocol.md`: four codebases converging on the same numbers by accident is how iOS and
  Android already came to disagree (below).
- **"Keep quality"** — so the target is a floor as well as a ceiling. A client must not upscale,
  must not raise a bitrate, and must not re-encode something that already meets the target, because
  every lossy re-encode costs quality and buys nothing when the file was already small.

**It reverses two decisions that are written into the code**, both of them yours, so this document
names them rather than quietly undoing them:

- `ios/FamilyConnect/Core/MediaPrep.swift:223` — *"Re-encode only when the original is over the
  ceiling — nettrash's choice: keep what the sender shot when it fits, compress when it does not,
  and refuse only if compression was not enough."*
- `ios/FamilyConnect/Core/MediaPrep.swift:365` (prepareAudio) — *"re-encoding someone's music to
  save a few megabytes would be a worse trade than refusing it."*

The first is reversed outright by #74. The second is reversed only in part by the recommendation
below: the large wins are uncompressed audio, and a rule can take those without re-encoding music
that is already compressed.

## Checked facts

**The server** (`server/src/handlers_attachment.rs`, `server/src/models.rs:892`, protocol "Photos,
videos, audio, files and locations")

- **Never decodes anything.** It checks the declared type against the file's magic number and
  stores the bytes. Posters and photo previews are made by clients. So #74 needs **no server
  change**: whatever a client produces has only to be an accepted type.
- **Accepted video types:** `video/mp4`, `video/quicktime`. Anything else as `kind=video` is
  `invalid_attachment`.
- **Accepted audio types:** `audio/mp4`, `audio/m4a`, `audio/mpeg`, `audio/wav`, `audio/ogg`.
  **This list is not in `docs/protocol.md`** — the Audio section says the magic-number check
  applies but never says to what. It is a documentation gap independent of #74, and the amendment
  below closes it.
- **Ceiling 100 MB** (`limits.max_attachment_bytes`); **`GET /attachments/{id}` honours `Range`**
  with `206`, which is how players seek — so an MP4 whose `moov` box is at the END can still be
  played, but only after the player has fetched the tail. Moving it to the front ("faststart") is
  what lets playback start on the first bytes.

**What each client does today**

| | Photo | Picked video | Picked audio | Voice note |
|---|---|---|---|---|
| **iOS / macOS** (shared `MediaPrep.swift`) | always: 2048 px edge, JPEG 0.85 | untouched if ≤ 100 MB; else `AVAssetExportPreset1920x1080` → **1080p** MP4 (`:281`) | never re-encoded | AAC mono 44.1 kHz, quality `.medium` (`AudioRecorder.swift:114`) |
| **Android** (`MediaPrep.kt`) | always: `PHOTO_EDGE = 2048` | untouched if ≤ 100 MB and MP4/MOV; else Media3 `Transformer` → H.264/AAC, **short side 720** (`:712`) | never re-encoded | AAC mono 44.1 kHz **64 kbps** (`VoiceRecorder.kt:79`) |
| **Windows** (`MediaPreparing.cs`) | always: `MediaPrep.PhotoEdge` | **always untouched**; over 100 MB refused (`:90`) | never re-encoded | `CreateM4a(AudioEncodingQuality.Auto)` (`VoiceRecorder.cs:62`) |
| **Web** (`prep.rs`, `recorder.rs`) | always downscaled | **always untouched**; over 100 MB refused | never re-encoded | `audio/mp4` where the browser records it, **else PCM WAV** (`recorder.rs:212`) |

Three things stand out:

1. **iOS compresses to 1080p and Android to 720p** — the same product disagreeing about the same
   job, because nothing wrote the target down.
2. **Windows and the web never compress a video at all.** A 95 MB phone clip goes up as 95 MB.
3. **The web client's WAV fallback is about twelve times the size of everyone else's voice note**:
   mono 16-bit PCM at the context's rate (typically 48 kHz) is ~5.8 MB a minute, against ~0.5 MB for
   AAC at 64 kbps.

**What a phone actually hands us.** iOS's own estimates under Settings → Camera → Record Video are
roughly 60 MB a minute for 1080p30 HEVC and 170 MB for 4K30 HEVC. So today a 1080p clip up to about
1½ minutes, or a 4K clip up to about 35 seconds, is uploaded exactly as shot.

**What every client can PLAY** — which, not what each can encode, is what decides the format:

- **H.264 + AAC-LC in MP4** plays everywhere this product runs: Safari, Chrome, Edge and Firefox;
  AVFoundation on iOS and macOS; Media3 on Android; `MediaPlayerElement` on Windows. It is also
  what every one of those platforms can *encode* with hardware.
- **HEVC** is smaller for the same quality but does not play in Firefox and plays in Chrome only
  where the OS provides a decoder. A family member on the web client would get a black box.
- **AV1** decodes in hardware on Apple only from A17 Pro / M3 onward.
- **Opus** is the best speech codec but its usual container, **Ogg, has no demuxer in
  AVFoundation** — an Ogg voice note from a browser would not play on an iPhone.

So the format is not really a choice: **MP4 with H.264 video and AAC-LC audio, and M4A (MP4) with
AAC-LC for audio alone.** The choices are the numbers.

## The hard parts

1. **The web client.** A browser can *encode* (WebCodecs `VideoEncoder`/`AudioEncoder`, where the
   browser supports them) but it cannot *demux* or *mux*: WebCodecs takes and returns raw frames and
   packets, not files. Transcoding a picked MP4 in the browser therefore needs an MP4 reader and an
   MP4 writer in the page — in this client, Rust compiled to WASM. The alternative
   (`<video>` → canvas → `MediaRecorder`) needs no muxer but runs in real time (a three-minute clip
   takes three minutes) and records MP4 only in some browsers. Either way, support differs by
   browser and has to be **detected at runtime** (`isConfigSupported`), never assumed from a
   version number — and where it is missing, the original has to go up as today, or the web
   client would lose sends that work now.
2. **Re-encoding what is already good.** A 720p H.264 clip at 1.5 Mbps re-encoded to "720p, 2 Mbps"
   comes out *bigger and worse*. The target needs a "leave it alone" rule, and each client has to
   read the source's codec, dimensions, frame rate and bitrate before deciding.
3. **Lossy → lossy audio.** Your own objection in `prepareAudio`, and it is right for music that is
   already compressed: a 192 kbps MP3 re-encoded to 128 kbps AAC loses quality twice for a ~33%
   saving. It is wrong for WAV, where the saving is ~10× and there is no first generation to lose.
4. **Time and battery.** Transcoding a long 4K clip on a phone takes tens of seconds. The 1.1 uploads
   already run in the background with progress (`BackgroundUploads`), so the transcode has to live
   inside that same pipeline — after the message is queued, before the bytes go — and survive the
   app being backgrounded, exactly as the upload does.
5. **Rotation and HDR.** A portrait phone video is a landscape track with a rotation. The output
   must carry the rotation (or bake it in) and report the TURNED size, which both existing
   transcoders already do. HDR (HLG/Dolby Vision from recent iPhones) has to be tone-mapped to SDR
   H.264, or a recipient sees washed-out grey; AVFoundation and Media3 both do this when asked for
   an 8-bit output, and it has to be asked for.

## Recommended design

### One profile, written into the protocol

| | Target | Why |
|---|---|---|
| Container | MP4, `moov` at the front | the only container every client plays; faststart lets playback begin on the first bytes |
| Video codec | H.264, High profile (Main where the encoder offers nothing else) | universal decode and hardware encode |
| Resolution | **short side ≤ 720**, never upscaled | a 720p clip is indistinguishable from 1080p in a chat bubble; the one client that already compresses by default in this family (Android) uses it |
| Frame rate | **≤ 30 fps**, never raised | 60 fps doubles the bits for smoothness a family clip rarely needs |
| Video bitrate | **≈ 2 Mbps at 720p30**, scaled down with the pixel count | ~16 MB a minute with the audio, against ~60 MB for the 1080p HEVC a phone records |
| Audio in video | AAC-LC, **128 kbps**, stereo stays stereo | transparent for speech and ambient sound |
| Picked audio | M4A, AAC-LC **128 kbps** | — but see the rule below: only when it is worth it |
| Voice note | M4A, AAC-LC mono, **64 kbps** (unchanged on iOS and Android) | already small; the win is replacing the web's WAV |

### Four rules around it

- **A — leave it alone when it already fits.** Same container and codecs, within the resolution and
  frame-rate caps, and a bitrate no more than ~25% over target: upload the original. Re-encoding a
  file that already meets the profile only loses quality.
- **B — never upscale, never raise a bitrate or a frame rate.** A 480p clip stays 480p.
- **C — a transcode that fails, or a platform that cannot, sends what it would send today.** An
  accepted container within the ceiling goes up untouched; anything else follows today's path. No
  send that works in 1.1 may stop working in 1.2.
- **D — a result bigger than its source is thrown away**, and the source goes instead.

**Audio specifically:** re-encode uncompressed and lossless audio (WAV, AIFF — and anything else the
platform can decode that is not already lossy), and lossy audio above **192 kbps**. Leave MP3 and AAC
at or below 192 kbps untouched. That keeps your 1.1 objection for music that is already compressed
and takes the ~10× win where there is one.

### Per platform

- **iOS and macOS** — one implementation in the shared `MediaPrep.swift`. Replace the preset export
  with `AVAssetReader` → `AVAssetWriter`, which is the only AVFoundation path that sets an exact
  bitrate and frame rate (`AVVideoAverageBitRateKey`, `AVVideoExpectedSourceFrameRateKey`,
  `AVVideoProfileLevelKey`); `shouldOptimizeForNetworkUse = true` writes the `moov` first. Picked
  audio uses the same reader/writer with an AAC `AVAudioSettings`.
- **Android** — the existing Media3 `Transformer` already does H.264/AAC at short side 720. Add a
  `DefaultEncoderFactory` with `VideoEncoderSettings` for the bitrate, a frame-drop effect for the
  30 fps cap, and call it for every video that fails rule A instead of only oversized ones. Picked
  audio goes through the same `Transformer` as an audio-only item.
- **Windows** — `Windows.Media.Transcoding.MediaTranscoder` with a `MediaEncodingProfile` built from
  `CreateMp4(VideoEncodingQuality.HD720p)` and then given the bitrate and frame rate explicitly;
  audio with `CreateM4a` and an explicit bitrate. Hardware-accelerated, and the voice recorder's
  `AudioEncodingQuality.Auto` becomes the same explicit 64 kbps mono the other recorders use.
- **Web** — progressive enhancement, last. Voice notes first, because that is the big web win and
  the easy half: replace the WAV fallback with `AudioEncoder` AAC where the browser has it, keep WAV
  only where it does not — and even then record it at a speech rate (16–22 kHz), which alone cuts it
  by more than half. Picked video only after that, with WebCodecs plus an MP4 reader/writer in the
  WASM bundle, detected per browser, falling back under rule C.

### The protocol amendment (to be applied FIRST, before any code, once the numbers are agreed)

Under "Photos, videos, audio, files and locations", after the paragraph beginning **"The server
never decodes an image or a video"**:

> **Clients prepare media before they upload it.** The server stores what it is given and never
> transcodes, so the size of a family's history is decided on the sending device. A client SHOULD
> bring a video to the profile below before uploading it, and audio where the rules say so. The
> profile is a target for SENDERS: the server accepts any listed type at any resolution, and every
> client MUST still play anything it receives, including uploads from clients that predate it.
>
> *(the profile table and rules A–D, as above)*
>
> The format is fixed by what every client can PLAY, not by what one can encode: H.264 and AAC-LC in
> MP4 play in every browser and on every platform this protocol has a client on; HEVC does not play
> in Firefox, AV1 does not decode on older Apple hardware, and AVFoundation cannot read Ogg.

And in "Audio", the list it has never had:

> The accepted types for `kind=audio` are `audio/mp4`, `audio/m4a`, `audio/mpeg`, `audio/wav` and
> `audio/ogg`, each checked against its magic number. A client SHOULD NOT send `audio/ogg` for
> something it expects every member to play: AVFoundation has no Ogg reader, so an Ogg recording
> does not play on iOS or macOS.

### Tests

Each platform gets the same four fixtures, generated once and committed small:

1. **A 4K60 HDR-flagged clip, portrait** → out: short side 720, ≤ 30 fps, H.264 + AAC-LC, `moov`
   before `mdat`, SDR, the TURNED size reported, smaller than the source.
2. **A 720p30 H.264 clip at 1.5 Mbps** → rule A: the original bytes, unchanged.
3. **A clip the encoder would grow** → rule D: the original.
4. **A WAV and a 128 kbps MP3** → the WAV becomes AAC 128 kbps M4A; the MP3 is untouched.

Plus the failure path (the transcoder throws) → rule C, and on the web the feature-detected branch
both ways. And one cross-client check worth more than all of these: **a file produced by each
client plays on each of the others** — done by hand once per platform pair before release, because
that is the property the whole format decision rests on.

## Phases

1. **Protocol** — the amendment above, including the audio-type list that is missing regardless.
2. **iOS + macOS** — the largest share of senders, and one implementation covers two platforms.
3. **Android** — mostly configuration of the `Transformer` it already has.
4. **Windows** — `MediaTranscoder`.
5. **Web** — voice-note WAV first; picked video last, behind feature detection.

Each phase ships on its own: the server does not change, and a client that has not been updated
yet keeps sending what it sends today, which every other client already plays.

## What has to be decided

1. **Resolution cap: 720p (recommended) or 1080p?** 1080p roughly doubles the size for detail that
   shows only full-screen on a large display.
2. **Video bitrate: ≈ 2 Mbps at 720p30 (recommended)?**
3. **Frame-rate cap of 30 (recommended)?** It costs slow-motion and 60 fps clips their smoothness.
4. **Audio: re-encode only uncompressed/lossless and lossy above 192 kbps (recommended), or every
   picked audio file?** The first keeps your 1.1 objection for compressed music.
5. **Voice notes stay AAC-LC mono 64 kbps (recommended)?** The only change then is the web WAV
   fallback and Windows's `Auto`.
6. **Rule C — send the original when a transcode fails (recommended) — or refuse?**
7. **Web in 1.2: voice notes only, or picked video too?** Voice notes are the cheap, large win;
   video needs an MP4 reader and writer in the WASM bundle.
8. **Should the server advertise the profile** (on `GET /me`) so an operator could change it? Not
   recommended for 1.2 — constants in the clients and one table in the protocol are simpler, and
   nobody has asked to change it.
