# Voice and video transcripts — what it would take (issue #62)

**Assessed 2026-09-13** against `v1.1`, in answer to issue #62: *"We have to add video & voice
transcription to have text of audio channel. And it can be automatically on server side and added as
a second layer to the message."* This document says what the request implies, which facts were
checked rather than assumed, where the hard parts are, what design is recommended and why, and what
has to be decided before any code is written. **No code has been written for it.**

> **Revised 2026-10-02 — on request, not automatically.** The owner has decided the shape: a member
> gets the text of a voice note, an audio file or a video **when they ask for it**, not every message
> all the time. The checked facts below still hold (one correction: the Apple apps' minimum is
> iOS 17 / macOS 14, not 26, so on-device `SpeechTranscriber` is not available to every install
> either). The automatic design — the worker, the pending marker, the frame and the feed — is
> **superseded** by "The design on request" at the end of this document, which has its own
> decisions. The sections between are kept as the record of why.

---

## What is asked, and what it implies

- **Text of the audio channel** — of a voice note, an audio file, and a video's sound.
- **Automatically, on the server** — nobody presses "transcribe"; every eligible message gets one.
  That means the audio of every member's message goes to a speech model without anybody asking for
  it that time, which this protocol has so far allowed only behind an owner's switch.
- **A second layer on the message** — the message is not changed; something is ADDED to it after
  it was sent, and every device, including one that was offline, has to learn that it was.

## Checked facts

**The provider**

- The server talks to Azure OpenAI deployments: `[ai]` (text), `[ai.vision]`, `[ai.images]`. A
  sub-section inherits the endpoint, key and api-version of `[ai]`; naming a `deployment` turns the
  capability on, and `GET /families/mine` tells clients which ones exist (`assistant.vision`,
  `assistant.images`). A fourth deployment is the same shape.
- **Azure's transcription contract** (Microsoft Learn, "Speech to text with transcription models",
  updated 2026-07-31): `POST {endpoint}/openai/deployments/{deployment}/audio/transcriptions?api-version=…`
  as **`multipart/form-data`**; **files must be 25 MB or smaller** (larger ones need Azure Speech
  batch transcription, a different service); formats **mp3, mp4, mpeg, mpga, m4a, wav and webm** —
  "other formats return an error". Only the documented request shape was read; like the images
  deployment, it has to be confirmed against a live endpoint.
- **The server's HTTP client has no multipart support compiled in** — `reqwest` carries `json`,
  `rustls-tls`, `http2` and `stream`. One Cargo feature.
- **`ai_usage` records tokens only** (`prompt_tokens`, `completion_tokens`). Transcription is billed
  by audio length, so family statistics need a column for it.
- The server's AI tests already stand a fake provider up on a local listener
  (`server/tests/assistant_flow.rs`, `common/mod.rs`), so a transcription stub fits the existing
  pattern.

**What is recorded**

- Voice notes are AAC in MPEG-4 on every client that records: Apple `kAudioFormatMPEG4AAC`,
  Android `OutputFormat.MPEG_4` + `AudioEncoder.AAC` into `.m4a`, the web `audio/mp4` (or
  `audio/wav` where a browser cannot record MP4). **All inside the provider's list.**
- An audio FILE picked from disk may also be `audio/mpeg` or `audio/ogg`. **Ogg is not in the
  provider's list.**
- A video may be up to 100 MB (`limits.max_attachment_bytes`) and `video/mp4` or
  `video/quicktime`. **QuickTime is not in the provider's list, and most videos are over 25 MB.**

**What the server will not do**

- "The server never decodes an image or a video", and under the assistant: "**Video, in either
  direction, is not here and is not planned.** The server decodes no media."
- `tokio` is built without its process feature, deliberately ("so a compromised transitive feature
  can't drag in process/fs machinery we never asked for"), and the systemd unit runs with
  `MemoryMax=768M` and `MemoryDenyWriteExecute=true`. **Cutting the sound out of a video on the
  server, or running a speech model there, would reverse written decisions**, not just add code.

**Privacy rules already written**

- "Nothing anybody else said leaves the server unasked" is the invariant that lets the server talk
  to a hosted model at all. Every widening since has been an **owner's switch, off by default**:
  `ai_history`, `ai_vision`, `ai_history_photos`, `ai_faces`.
- "**A direct chat is never consulted by the ASSISTANT.** Not by a mention, not by a private
  thread, at any setting, ever."
- Today the assistant is shown a voice note as the placeholder `[voice note]` and a video as
  `[video]` (`handlers_ai.rs`).
- A push carries the message TEXT as its body unless the operator sets `[push] include_message_body
  = false` — so a voice note's push says what its body says (usually nothing). A transcript is not
  the body and is recommended never to be pushed: it arrives after the push has already gone.

**How something is added to a sent message today — and why transcripts cannot use it**

- The edit feed (`message_edited`, `edit_seq`, `GET /chats/{id}/edits`) carries whole `Message`
  objects, and it already ADDS something once: the assistant's generated picture. protocol.md says
  that reply "is the only message for which it ever does".
- **The shipped phone apps draw "edited" from `edit_seq`, not `edited_at`**: Apple's snapshot is
  `isEdited: entity.editSeq > 0` (`Snapshots.swift`) and Android's bubble checks
  `entity.editSeq > 0` (`ChatScreen.kt`). The web reads `edited_at`. So a transcript that took an
  `edit_seq` would mark every transcribed voice note **"edited" on every installed iPhone, Mac and
  Android phone** — and no amendment to protocol.md reaches an app already installed.
- Clients MUST ignore unknown fields and unknown frame `type`s (Compatibility rules). A NEW frame
  and a NEW field are therefore safe for every shipped app.

**The family's language**

- One of nine locales, owner-set, unset until chosen ("unset is not English"), and "What it is FOR,
  today, is one thing": the language the assistant answers in. A language HINT for transcription
  would be its second use.

**Transcribing on the device instead — checked, and uneven**

- **Apple**: `SpeechTranscriber` (`SpeechAnalyzer`, iOS 26.0 / macOS 26.0 — the apps' minimum) takes
  an `AVAudioFile`, on-device. On this Mac (26.6.2) it offers en, de, es, fr, ja, ru and zh-Hans, and
  **nothing for `sr` or `sr-Latn`**.
- **Android**: an on-device recogniser is API 31, and feeding it a FILE (`EXTRA_AUDIO_SOURCE`) is
  API 33. The app's minSdk is 26.
- **Web**: no standard on-device API transcribes a file; not relied on.
- **Windows**: not checked.

## The hard parts

1. **Consent.** "Automatically" means every member's voice reaches a provider unasked. The written
   rule says that needs the family's owner to have chosen it, and a direct chat has a rule of its own.
2. **Video.** The server may not cut the sound out, the provider takes 25 MB, and a `.mov` is
   refused. Sending a whole `.mp4` would also send its PICTURES to a model under a switch about
   speech — the same objection that gave photographs their own switch.
3. **Delivery.** The obvious path — the edit feed — marks the message "edited" on installed apps.
   A transcript needs its own sequence, the way reactions and edits each got one.
4. **Durability.** Transcription runs after the send commits; a restart in between must not leave a
   message silently untranscribed for good.
5. **Cost** — every eligible message, every family, on the operator's bill.
6. **Language** — auto-detected unless told. Whether the deployment takes a language hint, and in
   which spelling, is part of the request shape still to confirm; and Serbian is written in two
   scripts, which a bare `sr` hint does not choose between.

## Recommended design

### Where it runs: on the server, through the operator's provider

A fourth deployment, **`[ai.transcribe]`**, inheriting like the other two, reported to clients as
`assistant.transcribe`. It matches the issue, draws the same text on all four clients whoever sent
the message, and needs no speech model on a phone that may not have one. On-device transcription is
kept as a possible later mode for a family that keeps the switch off; it is not the first version,
because on today's devices whether a message gets a transcript would depend on who sent it.

A speech model on the server itself is not recommended: it needs media decoding and a process the
server was deliberately built without, on a unit capped at 768 MB.

### Consent: an owner's switch, off by default

**`ai_transcripts`**, a boolean on `Family`, owner-only through `PATCH /families/mine`, `false` by
default for new and existing families — the shape of `ai_vision`. It depends on no other switch,
because it widens nothing the assistant is SHOWN; it sends a message's sound to a speech model and
brings text back. It transcribes messages sent AFTER it is turned on, and nothing earlier.

Which chats, in the recommended first version:

- **the family chat** — yes;
- **the member's own assistant thread** — yes: the member's own voice, in a thread whose words
  already go to the provider;
- **direct chats** — **no**. The written rule is about the assistant, but its reason — two people
  talking one-to-one are the conversation nothing else reads — applies unchanged to a transcription
  deployment on the same provider.

### What is transcribed

- **`kind=audio`** whose type is in the provider's list and whose size is within
  `[ai.transcribe] max_bytes` (default 25 MiB). Every recorded voice note qualifies. An Ogg file or
  an oversized one simply gets no transcript.
- **Video, in a second phase, through a sound track the SENDER'S device makes** — exactly as it
  already makes the poster frame. The uploader sends the video's audio as an `.m4a` beside it
  (`PUT /attachments/{id}/audio`, idempotent, uploader-only, like the preview); the server
  transcribes that and never sees a picture or decodes anything. Apple can export the track without
  re-encoding; Android can remux it (`MediaExtractor` + `MediaMuxer`); a web sender would send none
  and its videos would go untranscribed.

### On the wire: a field, a frame and a feed

```
Attachment … — plus "transcript": {"text": "…"} when (and only when) one exists
```

Absent means none — not eligible, not yet, silent, or failed — and never an empty string. A
transcript is written once and never changes.

It arrives as **its own frame**, which installed apps ignore:

```json
{"type": "attachment_transcript", "chat_id": 42, "message_id": 1338, "attachment_id": 34,
 "transcript": {"text": "…"}, "transcript_seq": 91}
```

and has **its own catch-up**, in the exact shape of edits: a `transcript_seq` from a new
server-wide sequence stamped on the message, `max_transcript_seq` per chat in `GET /chats`, and
`GET /chats/{id}/transcripts?after_seq=` returning whole `Message` objects. A history page carries
the field like any other, so a client that was offline reads it from the page. Only a frame or a
feed page moves the new cursor, the rule every other feed already follows.

**Not the edit feed**, for the reason checked above: the shipped Apple and Android apps would label
every transcribed voice note "edited".

### On the server

After a message with an eligible attachment commits, the server marks it pending (a column the wire
never sees) and a bounded worker sends the audio — with the family's language as a hint when one is
set and the deployment takes one. On a transcript it writes the text, takes a `transcript_seq`, fans out the frame and records the
audio's `duration_ms` in `ai_usage`. A boot sweep picks up rows left pending by a restart, within
`attachment_grace_hours`. A failure is written down and not retried in a loop. Retention, account
deletion and a departing member's direct chats take the transcript with the attachment row it lives
on. A report's `message_attachments` stays kind and name only.

### What the assistant is shown

Unchanged in the first version: still `[voice note]`. Showing it the transcript instead would widen
what a mention sends, and every such widening so far has been written down and switched on its own
(decision 4).

## Phases

1. **Protocol and server** — protocol.md first (the switch, the deployment, the field, the frame and
   the feed, which chats), then the migration, the worker, the multipart request, usage, and
   integration tests against a scratch Postgres with a stub provider on a local listener.
2. **Clients draw it**, starting with the web: the transcript under the player, the owner's switch in
   family settings in nine languages, the new feed in each client's resync. Windows' core gets it
   beside the others.
3. **Video**, if decided: the sender's device uploads the sound track (Apple, Android).
4. **The assistant reads transcripts**, if decided, under its own written rule.

Every phase ships alone: an app that has not reached phase 2 ignores the frame and the field and draws
exactly what it draws today.

## What has to be decided

1. **Server-side through the operator's provider, behind an owner's switch that is off by default?**
   Recommended: yes.
2. **Which chats?** Recommended: the family chat and the member's own assistant thread; **not**
   direct chats.
3. **Video?** Recommended: a later phase, through a sound track the sender's device uploads — not the
   whole video sent to the model, and not decoding on the server.
4. **Should the assistant see transcripts instead of `[voice note]`?** Recommended: not in the first
   version.
5. **Use the family's language as a hint?** Recommended: yes when it is set; auto-detect otherwise.
6. **Messages sent before the switch was turned on?** Recommended: not transcribed.

---

## The design on request (2026-10-02)

### What changes when a member has to ask

- **Delivery gets simple.** A transcript is the ANSWER to a request, so it comes back in that
  request's response. There is no frame, no `transcript_seq`, no catch-up feed and no edit-feed
  problem: nothing is added to the message for everyone, so no installed app has anything to
  misread.
- **No worker, no pending marker, no boot sweep.** Nothing is queued; a restart loses at most the
  one request in flight, which the member sees fail and asks again.
- **Cost follows use.** Only what somebody asked to read is sent, not every voice note of every
  family.
- **Consent gets a person.** The member who asks is the member sending the sound to the provider,
  so their own consent to the assistant is required — the same consent a `/draw` needs. Whose
  VOICE it is still matters, and is decision R2.
- **Old messages are fine.** The question "before the switch was turned on?" disappears: any
  message still held can be asked about.

### The request

```
POST /chats/{chat_id}/messages/{message_id}/attachments/{attachment_id}/transcript
→ 200 {"transcript": {"text": "…", "language": "ru"}}
```

- For a **voice note or audio file** whose stored type is in the provider's list (m4a, mp3, mp4,
  wav, webm …) and whose size is within `[ai.transcribe] max_bytes` (default 25 MiB): the server
  sends the STORED bytes. It decodes nothing.
- **Kept once, per attachment.** The first answer is written down (a `transcripts` row keyed by
  attachment), and anyone in the chat who asks later gets it back at once, with no second provider
  call and no second bill. It is still shown only to whoever asked: the text is not pushed, not in
  history pages, not in `Attachment` — every member taps for it themselves.
- **One provider call per attachment at a time.** Two members, or two devices, asking together wait
  on the same call.
- **Finished even if the asker gives up.** Unlike a backdrop, the call is not dropped with the
  connection: the next request gets the stored answer instead of paying again.
- **Slow by nature**, so a client gives this request its own timeout of at least 90 s (the backdrop's
  rule, for the same reason).
- **Language**: the family's language as a hint when set, auto-detect otherwise; the detected
  language comes back when the provider reports one.
- **Statistics**: one `transcript` per provider call and its audio `duration_ms` in `ai_usage` —
  billing is by audio length, not tokens. A stored answer handed out again counts nothing.
- **Never logged**: not the text, not the language.
- **Errors**: `transcripts_unavailable` (no deployment), `assistant_consent_required`,
  `not_transcribable` (wrong kind, unsupported type, over `max_bytes`), `transcript_refused` (the
  provider's filter). Silence is not an error: it comes back as `{"transcript": {"text": ""}}` and
  is drawn as "No speech", because "nothing was said" is an answer.

### Video, and audio the provider will not take

A video's sound cannot be cut out on the server (checked above), and most videos are over 25 MB.
On request there is a better source than the sender: **the asking device already holds the file** —
it downloaded it to play it — and since #74 every client carries media code that can take the sound
track out without decoding the pictures (Apple AVFoundation, Android `MediaExtractor` +
`MediaMuxer`, the web's own MP4 reader and writer in `web/text`, Windows Media Foundation), and
re-encode it to 64 kbit/s mono AAC where needed (≈ 50 minutes in 25 MB). The same path covers an
Ogg file or an audio file over `max_bytes`.

The client then sends that sound WITH the request (`multipart`, the same endpoint). The catch: the
server cannot check that uploaded sound is really this video's. If the answer were kept and handed to
other members, one member could put words in another's video. So **an answer made from sound the
client supplied is returned to the asker and NOT kept** — the asker's device keeps it. Only an
answer made from the server's own stored bytes is shared.

The sender-side sound track (phase 3 of the automatic design) is not needed any more: it would only
help videos sent by 1.3 apps, and on request the asker's device can do it for every video ever sent.

### Phases

1. **Protocol and server** — protocol.md first, then `[ai.transcribe]` + `assistant.transcribe`,
   the switch (if R2 says so), the endpoint (stored bytes and supplied sound), the `transcripts`
   table, the single-flight, usage, multipart in `reqwest`, and integration tests with a stub
   provider. Old clients see nothing new.
2. **Clients** — a "Show text" action on voice notes, audio and video on web, iOS/macOS, Android
   and Windows; the text appears under the player, selectable, with "Hide text"; the device keeps
   what it was given; nine languages.
3. **Video and unsupported audio** — the sound-track extraction per client, built on the #74 code.

### What has to be decided (replaces the list above)

- **R1. Server-side through the operator's provider, as a fourth deployment `[ai.transcribe]`?**
  Recommended: yes. On-device is uneven (no Serbian on Apple, iOS 26+ only; Android API 33+;
  nothing in browsers) and would give different members different answers.
- **R2. Whose voice may be sent?** Recommended: **your own messages, wherever you sent them, need
  only your consent; other members' messages also need an owner's switch, `ai_transcripts`, off by
  default** — the written rule that nothing anybody else said leaves the server unasked, now with
  the asker doing the asking.
- **R3. Which chats?** Recommended: the family chat, threads, and your own assistant chat. **Direct
  chats: your own messages only** — the other person's voice in a one-to-one chat stays under the
  rule that nothing reads a direct chat. (Allowing it would need that rule rewritten, and is yours
  to choose.)
- **R4. Share a stored answer with other members who ask?** Recommended: yes, for answers made from
  the server's own bytes — one bill per voice note, not one per reader. Never for answers made from
  sound a client supplied.
- **R5. Video in the first release, or after voice notes?** Recommended: voice notes and audio
  first (phases 1–2), video right after (phase 3), so the server and the action are proven on the
  simplest case.
- **R6. Should the assistant see transcripts instead of `[voice note]`?** Recommended: no, not in
  this work.

### A limit of R2, found in review (2026-10-02)

The owner's switch and the sender's consent are checked on the attachment a request names, so they
hold for the server's stored copy and for every client that follows the protocol. They cannot hold
against a member who sets out to get round them: anyone who can play another member's recording —
a direct-chat partner's included — can download it and send those bytes as the supplied sound of a
recording of their own (or upload them again as their own voice note), and the server cannot tell a
sound track from somebody else's voice. That reaches the provider on the asker's consent alone and is
billed to the family. It is the reach a member always has over what they can see (forwarding it,
playing it into another app); closing it would need the server to decode and fingerprint sound. The
switch is therefore what the family's own apps promise to do, not a lock, and protocol.md ("What the
rule can and cannot hold") says so for the owner.
