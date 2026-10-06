# Audio and video messages from the Send button — the plan (issue #79)

**Planned 2026-10-04** (facts re-checked 2026-10-05, then revised the same day after two reviews — UX
completeness and accessibility; feasibility and protocol) against eb1a858 on `79-audiovideo-messages` —
`v1.2` with #58 stickers, #62 transcripts on request, #72 lookups and #74 media preparation — in answer
to issue #79, "Audio/Video messages", opened by the owner: *"For Audio and Video messages,
additionally to text messages, I think we should use send message button. If no text entered we can
have send audio icon and send audio message just by clicking this button, after long press it can be
video message. This video message should be in circle format which is super popular now. Please
double check with possible UX, to have better UX for this task."* The owner's instruction on the day:
*"properly plan UX/UI changes for all platforms: ios, ipados, macos, android, windows, web. Check
these proposals. After, construct the plan and implement it."*

This document says what the request implies, which facts were checked rather than assumed, how the
issue's own proposal was checked and what changed and why, the interaction model chosen, the full
specification on every platform, the change on the wire and on the server, the recording profile,
where each piece plugs in, the tests and the phases. The owner asked for a decision rather than a
menu, so it ends with the **decisions taken** and the few things that are genuinely **blocked** — not
with questions. **No code has been written for it.**

> **Revised 2026-10-06, at the owner's request after testing on his iPhone: the hold is removed.**
> The microphone in the Send slot does ONE thing on every platform: a tap, click, Enter or
> screen-reader activation starts a hands-free voice recording; the slot's Send arrow sends it, Stop
> keeps it for review and a caption, Delete deletes it (asking at 10 s and longer). A long press on it
> is not a gesture: nothing records and nothing opens while the finger is down — no menu, no callout,
> no context menu — and the press is the button's ordinary tap when it lifts inside, however long it
> was held, so a slow or unsteady press is never a dead button. Gone with the hold, because only it
> needed them: the hold threshold H, the tap slop, lock and slide-to-cancel ("Slide to cancel",
> "Release to cancel", the lock pill), the hold row, the 5-second Undo window and its crash-safe
> "sending" entry, the first-release lesson ("Next time, letting go will send it."), "Still recording.
> Tap Send when you're done.", the silent-release check ("We didn't hear anything."), "You can record
> now." after a prompt a hold raised, the coach mark (S7.2), and the **Review Before Sending** setting
> (S9) — with nothing left that a release could send, S9 has nothing to turn off. Everything else
> stands: the recording row, review, the not-sent row, the 600 ms activation guard, the 1.0 s floor,
> the 5:00 cap into review, the live silence warning, ⌥⌘R / Ctrl+Shift+R, the pointer secondary-click
> menu, and "Record Voice Message" in the paperclip. Interruptions never send (protocol.md, amended
> the same day). The shared reducer `fc_text::record::hold_step` keeps its name and loses the hold's
> events, phases and effects; `record-vectors.json` is printed again.
>
> **The same day, the video recorder's layout was fixed.** On the iPhone the composer row showed
> through the recorder — undimmed, overlapping its controls — and the scrim let the chat compete with
> the camera circle. While the recorder is open the composer row is not drawn at all and takes no
> hits; the recorder's controls sit on their own solid dark, safe-area-aware bar; the backdrop is dark
> enough, and blurred where the platform does it cheaply, that the chat does not compete; the status
> line ("Not recording", the red dot and timer, "Video message · 0:23") sits in its own capsule above
> the circle; and the circle is sized so that it never overlaps the status or the controls, nor any
> caption under a round button, at any size — a compact or landscape phone, an iPad, the Mac,
> Android, the web, Windows and large text alike.
>
> The sections below are kept as written, so the reasoning stays readable; where they describe the
> hold, its Undo window, its lesson, its coach mark or Review Before Sending, this note wins.

---

## What is asked, and what it implies

- **The Send button does a second job.** With nothing typed it becomes the way to record. The text
  field must not jump when it changes, and Return in an empty field must never start a recording.
- **"Just by clicking."** One click cannot both start a recording and end it. The nearest honest
  reading is *one place*: click to start, click the same place to send.
- **"After long press it can be video."** A long press already means something on every platform this
  product runs on, and the habit most family members bring from WhatsApp and Telegram is to hold the
  microphone *to talk*. Whether a hold should open a camera is the main question this plan settles.
- **"Circle format."** A round video is a presentation, not a new kind of media: a square clip drawn
  as a circle. It needs a flag on the wire, the way a sticker is a photo with a flag.
- **Capturing video inside the app is new everywhere.** Today the only camera capture is the SYSTEM
  camera on iPhone/iPad and Android; the Mac declined in-chat capture on purpose; Windows and the web
  have none.
- **Nothing sent can be taken back.** There is no route to delete a message (checked below). A slip
  that sends stays in the family chat until retention removes it, 100 days by default. That one fact
  shaped more of this plan than anything else.
- **Everyone in a family.** The same button is used by a teenager with WhatsApp reflexes, a
  grandmother with a slow, unsteady press, a VoiceOver user and somebody on a Windows laptop with a
  mouse.

What is NOT in the issue, and is decided below: whether holding still means "talk"; how video is
reached on a desktop or with a screen reader; when the camera may turn on and when recording starts;
what is shown before a video is sent; what the wire calls the flag; the size, length and bitrate of a
round video; what the assistant chat and threads offer; autoplay and "played" receipts; whether the
Mac records video.

## Checked facts

### External

**What messengers do** (read 2026-10-04 unless dated)

- **Telegram's native apps**: one button, microphone or camera; a quick tap switches between them, a
  hold records, release sends, slide left cancels, slide up locks
  (https://telegram.org/blog/video-messages-and-telescope, 2017-05-18). The "tap" is decided in 150 ms
  on Android and 0.19 s on iOS (DrKLO/Telegram `ChatActivityEnterView.java:3062`;
  TelegramMessenger/Telegram-iOS `ChatTextInputMediaRecordingButton.swift:491`, master, 2026-09). A
  round video stops at 59.5 s into a PREVIEW, never a send (`ChatActivityEnterView.java:14350`).
  Android records 384 × 384 at 1 Mbit/s with 64 kbit/s audio (`MessagesController.java:1635-1637`).
  On the wire it is an ordinary video with a `round_message` attribute, which old clients show square
  (https://core.telegram.org/constructor/documentAttributeVideo); the Bot API's "video note" is "a
  rounded square MPEG4 video of up to 1 minute long", its size "as defined by the sender"
  (https://core.telegram.org/bots/api#sendvideonote).
- **Telegram's own web clients dropped tap-to-switch.** Web A starts recording on a click and switches
  voice/video from the button's context menu, and turns round recording OFF on Safari and every iOS
  browser — "canvas.captureStream produces invalid frames / can hang on stop" (Ajaxy/telegram-tt
  `windowEnvironment.ts:73-81`, 2026-09-29). Web K: "Switching voice ↔ video is done via the button's
  context menu … not by clicking" (morethanwords/tweb `input.ts:4280-4284`, 2026-09-25).
- **WhatsApp walked tap-to-switch back in three steps**: an off-switch because people were
  "mistakenly sending video messages when they wanted to send an audio message"
  (https://wabetainfo.com/whatsapp-is-rolling-out-enhanced-control-for-instant-video-messages-feature/,
  2023-09-04); a chooser instead of automatic switching
  (https://wabetainfo.com/whatsapp-beta-for-ios-23-21-1-71-whats-new/, 2023-10-16); a "Video note"
  camera tab for findability (https://wabetainfo.com/whatsapp-beta-for-ios-24-13-10-76-whats-new/,
  2024-07-03). Screen-reader users got no feedback when the mode changed
  (https://accessibleandroid.com/quick-tip-disabling-video-message-recording-on-whatsapp/,
  2023-11-19). Its 2025 betas start a locked recording on one tap
  (https://wabetainfo.com/whatsapp-beta-for-ios-25-13-10-70-whats-new/, 2025-04-28; whether it reached
  stable is UNCONFIRMED).
- **Tap, review, send is where the rest went.** iMessage: tap +, Audio, speak, Stop, then Send or Play
  (https://support.apple.com/guide/iphone/iph2e42d3117/ios). Apple's Assistive Access "Video Selfie":
  "Tap Record, record the message, tap Stop, then tap Send"
  (https://support.apple.com/guide/assistive-access-iphone/dev4b42da3b1/ios). Google Messages keeps a
  held recording as a draft to play first (https://support.google.com/messages/answer/6159880).
  Signal Desktop records on a click or ⌘/Ctrl+Shift+Y (signalapp/Signal-Desktop
  `ShortcutGuide.dom.tsx:169-171`).

**Accessibility and platform conventions**

- **WCAG 2.2** (https://www.w3.org/WAI/WCAG22/Understanding/): 2.5.1 — a path gesture such as a slide
  needs a single-pointer alternative. **2.5.2, read 2026-10-05**: at least one of "No Down-Event: The
  down-event of the pointer is not used to execute any part of the function", "Abort or Undo:
  Completion of the function is on the up-event, and a mechanism is available to abort the function
  before completion or to undo the function after completion", "Up Reversal", "Essential". 2.5.7 — a
  drag needs a non-drag alternative. 2.1.1 — holding a key down for a time is a timing requirement.
  2.5.8 — targets of at least 24 × 24 CSS px. 2.2.2 — motion that starts by itself and runs over 5 s
  needs a pause. 4.1.3 — status changes are announced.
- **Apple**: a custom gesture must not be "the only way to perform an important action"; touch and
  hold means "Reveal additional controls or functionality", and macOS has no touch and hold
  (https://developer.apple.com/design/human-interface-guidelines/gestures, 2024-09-09). In silent mode
  a device "plays only the audio that people explicitly initiate, like media playback, alarms, and
  audio/video messaging" (https://developer.apple.com/design/human-interface-guidelines/playing-audio).
  VoiceOver's two-finger double tap, Magic Tap, is "start or stop a recording"
  (https://support.apple.com/guide/iphone/iph3e2e2281/ios) — and when an app does not take it, it
  "plays and pauses music playback from the Music app if no Magic Tap implementation is found from the
  current view to the app delegate" (View Controller Programming Guide, "Supporting Accessibility",
  https://developer.apple.com/library/archive/featuredarticles/ViewControllerPGforiPhoneOS/SupportingAccessibility.html,
  read 2026-10-05).
- **Android**: in Compose a long press on an icon button shows its tooltip
  (https://developer.android.com/develop/ui/compose/components/tooltip, 2026-10-01); TalkBack's
  double-tap-and-hold is a discrete long-click action, not a held finger
  (https://support.google.com/accessibility/android/answer/6151827); a gesture-only action needs an
  accessibility action
  (https://developer.android.com/guide/topics/ui/accessibility/views/principles-views, 2026-05-01).
- **Windows**: press and hold is "a menu of secondary, contextual commands"
  (https://learn.microsoft.com/windows/apps/design/input/touch-interactions, 2026-07-14); "Mouse input
  doesn't produce Holding events", and a completed touch hold raises `RightTapped`
  (https://learn.microsoft.com/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.uielement.holding,
  2026-07-28).
- **Browsers**: a touch `pointerdown` is not user activation; only a touch's `pointerup`/`touchend` is
  (https://html.spec.whatwg.org/multipage/interaction.html#activation-triggering-input-event). A long
  press fires `contextmenu` on Android and a callout on iOS; `-webkit-touch-callout: none` is reported
  broken on iOS 26.1 (https://developer.apple.com/forums/thread/808606, UNCONFIRMED).
- **Older users** preferred "click-to" designs and handled LEFTWARD and UPWARD drags worst — exactly
  slide-to-cancel and slide-to-lock (Gao & Sun, Human Factors 57(5), 2015,
  doi:10.1177/0018720815581293); tap-based replacements for hold and drag improved their performance
  (Salman et al., IVIC 2019, doi:10.1007/978-3-030-34032-2_60).

**Recording and playing, per platform**

- **Apple** (iOS 17 / macOS 14): `AVAssetWriter(.mp4)` with `AVVideoScalingModeResizeAspectFill`
  crops to a square inside the writer
  (https://developer.apple.com/documentation/avfoundation/avvideoscalingmoderesizeaspectfill; the
  Xcode 27 SDK's `AVAssetWriterInput.h` says only Fit is unsupported there); `shouldOptimizeForNetworkUse`
  puts `moov` first; `AVCaptureMovieFileOutput` writes QuickTime
  (https://developer.apple.com/documentation/avfoundation/avcapturemoviefileoutput); `RotationCoordinator`
  exists from iOS 17 / macOS 14; `startRunning()` blocks and belongs off the main thread. An iPad
  capture session is interrupted in Split View, Slide Over and Stage Manager unless
  `isMultitaskingCameraAccessEnabled` is set where `isMultitaskingCameraAccessSupported`
  (https://developer.apple.com/documentation/avkit/accessing-the-camera-while-multitasking-on-ipad,
  read 2026-10-05): "Apps that have a deployment target earlier than iOS 16 require the
  `com.apple.developer.avfoundation.multitasking-camera-access` entitlement" — this app's is 17.0
  (`ios/FamilyConnect.xcodeproj/project.pbxproj:461`), so none is needed; after a clip recorded while
  multitasking "the system displays an alert one time only" about lower quality; and `systemPressureState`
  rises before "the capture system shuts down". Gesture reactions are rendered INTO camera frames: on
  iOS by default for an app with the `voip` background mode — this one has it (`Info.plist:27-31`) — and
  on macOS for every app (SDK `AVCaptureDevice.h:2585`); `NSCameraReactionEffectGesturesEnabledDefault`
  = false makes OFF the default only from iOS 17.4 / macOS 14.4, app-wide, and only "until such time
  that the user makes their own selection in Control Center" (`:2590-2602`). Deactivating an audio session while audio
  runs "stops the objects" (https://developer.apple.com/documentation/avfaudio/avaudiosession/setactive(_:options:)).
  SwiftUI's `onLongPressGesture` fails after 10 pt of movement, so it cannot drive a slide.
- **Android**: "For most developers, CameraX is recommended"
  (https://developer.android.com/media/camera/choose-camera-library, 2024-04-08). CameraX crops video
  to a 1:1 `ViewPort` since 1.3.0; stable is 1.6.2 (2026-08-26);
  stable CameraX does not let the audio codec, channels or bitrate be set
  (https://developer.android.com/media/camera/camerax/video-capture, 2026-09-01;
  https://developer.android.com/jetpack/androidx/releases/camera). A `SurfaceView` "punches a hole in
  its window" and cannot be clipped; a `TextureView` can. Since Android 9 a background app "cannot
  access the microphone or camera"
  (https://developer.android.com/about/versions/pie/android-9.0-changes-all). With Android 12's
  global toggles off an app gets "silent audio" or "a blank camera feed" and no error
  (https://developer.android.com/training/permissions/explaining-access, 2026-10-01). Two denials make
  a permission permanent (https://developer.android.com/training/permissions/requesting, 2026-10-01).
  For an app targeting Android 16 — this one targets 36 (`android/app/build.gradle.kts:123`) —
  `screenOrientation` and `setRequestedOrientation()` are ignored on displays whose smallest width is
  600 dp or more (https://developer.android.com/about/versions/16/behavior-changes-16, 2026-10-01). With
  the orientation locked, CameraX says to "update the target rotation of the use cases except Preview"
  from an `OrientationEventListener`
  (https://developer.android.com/media/camera/camerax/orientation-rotation, 2024-01-05). Headphones
  unplugged broadcast `ACTION_AUDIO_BECOMING_NOISY`, and a player with on-screen controls is expected to
  pause (https://developer.android.com/media/platform/output, 2025-04-07).
- **Windows** (WinUI 3): `MediaCapture` works in a packaged app and previews through
  `MediaPlayerElement` + `MediaSource.CreateFromMediaFrameSource`
  (https://learn.microsoft.com/windows/apps/develop/camera/camera-quickstart-winui3, 2026-08-23); there
  is no `CaptureElement` (microsoft-ui-xaml #8214, open); the frame-source preview stays blank on
  RGB24/UYVY/I420 webcams (#9756, open since 2024-06-24); `UIElement.Clip` takes only a rectangle
  (https://learn.microsoft.com/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.uielement.clip);
  `CornerRadius` on `MediaPlayerElement` was reported not to clip (#8264), closed 2023-07-18 as completed
  with "there have been some fixes in WASDK 1.4 … reactivate if you still hit this issue" — the app is on
  Windows App SDK 2.4.0 (`win/src/FamilyConnect.App/FamilyConnect.App.csproj:93`), so whether it clips now
  is UNCONFIRMED; `VideoTransformEffectDefinition` crops and sizes
  (https://learn.microsoft.com/uwp/api/windows.media.effects.videotransformeffectdefinition).
  `DisplayRequest.RequestActive` once threw a `COMException` under the Windows App SDK
  (microsoft/WindowsAppSDK #3002, filed 2022-09-28 against 1.1.4, closed as completed 2025-08-08).
- **Web**: `VideoEncoder` is in Chrome/Edge 94, Safari 16.4 and desktop Firefox 130; `AudioEncoder`
  in Chrome/Edge 94 and Safari 26 (https://webkit.org/blog/17333/webkit-features-in-safari-26-0/,
  2025-09-15). AAC encoding "is not supported in Firefox on any platform, or in any browser on desktop
  Linux" (https://developer.mozilla.org/en-US/docs/Web/API/WebCodecs_API/Codec_selection, 2026-08-12).
  `requestVideoFrameCallback` is in Chrome 83, Safari 15.4, Firefox 132. Chrome's and Safari's
  `MediaRecorder` write FRAGMENTED MP4 with no duration in `moov`
  (https://blog.addpipe.com/duration-in-mp4-files-produced-by-chrome-safari/, 2026-05-15). `beforeunload`
  "is not reliably fired, especially on mobile platforms" — not at all when a phone's browser is closed
  from the app switcher (https://developer.mozilla.org/en-US/docs/Web/API/Window/beforeunload_event,
  last modified 2026-08-21).

**The database**

- `ROUND` is not a key word in PostgreSQL 18.6
  (https://www.postgresql.org/docs/current/sql-keywords-appendix.html, read 2026-10-05), so a column
  named `round` needs no quoting.

### Internal (read in this repository)

**Nothing sent can be removed**

- The only route on a single message is `PATCH` (`server/src/app.rs:162-165`). The REST table's only
  `DELETE`s under a message are a vote and a reaction (`docs/protocol.md:5497`, `:5503`). "There is no
  way to delete somebody else's message" (`:3582`). An edit never adds, removes or replaces an
  attachment (`:669-672`). Retention is 100 days by default (`:4799`).

**The protocol**

- "A tile never downloads a VIDEO to draw itself" (`docs/protocol.md:107-108`). A voice note is "five
  minutes at most, staged so a caption can be added — is the apps' rule" (`:112-113`).
- Clients MUST ignore unknown fields (`:163-166`).
- **The sticker flag is the template**: `"sticker": true` on the Attachment only when true, set by
  the send and never changed, drawn as a photo by a client that does not know it (`:425-429`); not
  editable — `validation`, asked LAST (`:676-683`); one drawing test, the same on every client
  (`:1580-1584`); a box size recommended rather than carried (`:1584-1590`); a server without stickers
  is recognised by the ABSENCE of `max_pack_items` (`:1612-1615`, `:5434`); a sticker counts as a
  photo in statistics (`:1650-1651`).
- "Sticker" already meant a board NOTE, and the protocol had to say so before the pack could use the
  word (`:1338-1345`). "Note" is a board word here.
- The assistant is shown `[video]` and `[voice note]` by kind (`:2320-2321`, `:4071-4073`).
- A video's transcript is made from sound the asking device supplies, never kept or shared
  (`:3901-3911`, `:3988-3994`).
- The server never decodes media (`:4459-4462`). The #74 profile: MP4 with `moov` first, H.264 High,
  short side ≤ 720, ≤ 30 fps, bitrate `2 000 000 × (w × h ÷ 921 600) × (fps ÷ 30)` (`:4497-4522`);
  Rule A leaves alone a file within 1.25 × of its target (`:4523-4527`); `video/quicktime` is never
  within the profile (`:4526-4527`); a voice note is M4A, AAC-LC, mono, 64 000 bit/s (`:4506`); audio
  is drawn "deliberately not a waveform" (`:4560`).
- A media send is an outbox row BEFORE its first byte, and its bytes are kept until the ack
  (`:5858-5868`). Push words: "Photo", "Sticker", "Video", "Audio" (`:5998-6006`).

**The server**

- The sticker flag end to end: the request field (`server/src/handlers_chat.rs:71-75`); the poll
  exclusion (`:626-631`); shape checks before any id is read (`:639-650`); the after-claim check that
  drops the transaction (`:869-882`); the claim `UPDATE … sticker = $5 … RETURNING … width, height,
  duration_ms, … sticker` (`:1186-1207`); the edit refusal asked last (`:1056-1064`); the push word
  (`server/src/push_payload.rs:120-125`); discovery keys always present
  (`server/src/handlers_family.rs:895-901`); config bounds `1 ≤ max_pack_item_bytes ≤
  max_attachment_bytes` (`server/src/config.rs:2285-2291`).
- The call-record edit check runs BEFORE `message_not_found` and is not scoped to the chat
  (`handlers_chat.rs:1011-1021`), so it tells a member whether an id elsewhere is a call record. It is
  the one NOT to copy.
- Width, height and duration are the uploader's declaration (`server/src/handlers_attachment.rs:35-53`);
  an audio upload's name is dropped (`:290`), so a voice note always pushes "Audio" — and nothing on
  the server tells a voice note from a sound file picked from disk, which share `kind=audio`
  (`docs/protocol.md:4554-4556`).
- The claim writes the sticker flag in the same `UPDATE` that claims the upload, BEFORE the code knows
  the kind (`handlers_chat.rs:1195-1205`; the kind is checked after it returns, `:869-882`), and
  migration 0048 put no kind `CHECK` on `sticker`. A database error is `ApiError::Internal`, a 500
  (`server/src/error.rs:307-310`), which is not a terminal code: an outbox keeps such a row queued and
  retries it (`docs/protocol.md:5847-5852`).
- Every integration-test server sets `max_attachment_bytes = 64 * 1024` and then validates its config
  (`server/tests/common/mod.rs:185`, `:196`); the sticker change had to lower `max_pack_item_bytes` there
  for exactly that reason (`:186-191`), and its bound made a server with a smaller
  `max_attachment_bytes` refuse to start (`CHANGELOG:122-124`).
- A report's attachments are kind and name only (`server/src/handlers_report.rs:107-110`,
  `docs/protocol.md:359`).
- Migrations end at `0051_greeting_places.sql`; the next is 0052. PostgreSQL through sqlx
  (`server/Cargo.toml:44-48`).

**Apple** (`ios/FamilyConnect/`)

- Voice notes are tap-to-start ON PURPOSE: "hold-to-talk is a phone gesture that has no sensible
  desktop equivalent" (`Core/AudioRecorder.swift:11-14`); Android says the same
  (`VoiceRecorder.kt:11-13`).
- The recorder: a 300 s cap (`AudioRecorder.swift:73`); at the cap the hardware stops but nothing
  tells the UI — the counter freezes and nothing is staged (`:198-201`); 1024 bytes or less is "too
  short" (`:165`); `stop()` deactivates the shared audio session unconditionally (`:160`), which would
  stop a call's audio.
- Nothing stops a recording when the conversation goes away or the app goes to the background
  (`Views/ConversationView.swift:1039-1049`, `:1056-1071`). The toolbar Call and Video buttons stay
  live while recording (`:955-980`). On iPhone and iPad a call is a full-screen cover whenever its
  phase is not idle (`RootView.swift:39-41`, `:118-121`); on the Mac it has its own window and the
  composer stays usable during a call. A conversation is only ever a navigation destination or the
  split view's detail (`Views/ChatListView.swift:221`, `:304`), never inside a sheet.
- `ConversationView` already tells the BACKGROUND apart from the inactive flicker an alert or Control
  Centre causes (`ConversationView.swift:1062-1070`). The iPhone supports both landscapes
  (`ios/FamilyConnect.xcodeproj/project.pbxproj:567`) and the iPad all four orientations (`:566`).
- Playback: nothing sets a playback category; the only audio route-change observer is in
  `Core/Calls/WebRTCClient.swift`; the `audio` background mode is declared for calls and "used for
  nothing else" (`Info.plist:23-31`).
- The composer: the bar's height must stay CONSTANT or the newest messages slide under it
  (`ConversationView.swift:20-28`); the recording strip is stacked ABOVE the row (`:1503-1505`,
  `:1977-1996`); Send is a 36-pt scaled control with ⌘↩ (`:173`, `:1720-1734`); "Record Audio" is
  offered in every chat, the assistant's included (`:1618-1622`); the sticker button already appears
  only when nothing is typed or staged (`:2925-2929`); `takeComposer` takes caption and reply together
  (`:2162-2168`); a hidden ⌘V button is the pattern for a hidden shortcut (`:1787-1802`).
- The staged chip cannot play a note (`Views/StagedAttachment.swift:105-121`). Only TEXT is parked
  across a view change, and "a recording in progress … [is] deliberately let go"
  (`Views/ComposerDrafts.swift:13-18`).
- The Mac: three Return bindings meet while recording — Stop's `.defaultAction`
  (`MacViews/MacConversationView.swift:1473`), the field's `.onSubmit` (`:1729`) and Send's `.return`
  (`:1751`); Send has no tooltip and no accessibility label (`:1743-1752`); "capture-into-a-chat stays
  a phone thing" (`Views/CameraPicker.swift:17-20`); the media tile asks for the poster AND the
  original at once, so a video downloads its whole MP4 (`MacViews/MacMessageRow.swift:1382-1383`);
  SwiftUI's `VideoPlayer` crashes on macOS (`MacViews/MacAttachmentViewer.swift:217-230`); ⌘R is
  Refresh (`FamilyConnectApp.swift:485-490`), and no other shortcut in the app uses R (every
  `.keyboardShortcut` was read). The window toolbar carries Return to Call, the board, Family and
  Settings (`MacViews/MacChatView.swift:110-165`; Family's sheet sets `selectedChatID`, `:168`) and Call,
  Video Call and Open polls (`MacConversationView.swift:568-640`); toolbar items live in the window's
  title bar, where nothing the content draws can cover them or take their clicks. The app delegate is
  `MacAppDelegate` (`FamilyConnectApp.swift:36`).
- Helpers the round video will need are private today: `MediaPrep.preparedVideo` (`private static`,
  `Core/MediaPrep.swift:392`) and `posterFrame` (`:450`).
- No `AVCaptureSession`, preview layer, `AVPlayerLayer` or press-and-hold exists in the app;
  `AVAssetWriter` already writes MP4 for transcodes (`Core/MediaTranscoder.swift:142`). The usage
  strings and the Mac's camera and audio-input entitlements already exist. A sticker is drawn with no
  balloon in a fixed box (`Views/MessageBubbleView.swift:1050-1051`, `:1117-1130`, `:1189-1192`); the
  chat list says "Sticker" before it says "Video" (`Core/ChatSyncCoordinator.swift:3261-3262`).
- UI tests find the composer by "Attach a photo, video or file" and the "Message" field; nothing
  tests Send.

**Android** (`android/app/src/main/java/me/nettrash/familyconnect/`)

- `VoiceRecorder` is a `@Singleton` (`data/repo/VoiceRecorder.kt:36-37`) driven from `viewModelScope`,
  and no ViewModel overrides `onCleared`: Back mid-recording leaves the microphone open and voice notes
  dead until the process dies. The cap has no `OnInfoListener` (`:84`). The file is named
  `voice-<ms>.m4a` (`:59`) and that name is uploaded.
- Send is a 44-dp `FilledIconButton`, which takes only `onClick` (`ui/chat/ChatScreen.kt:5671-5688`).
  "Record audio" and the SYSTEM camera's "Record video" sit in the attach menu (`:5444-5451`,
  `:5498-5507`; `res/values/strings.xml:188`, `:196`, beside "Take photo" at `:141`). The field is
  Material 3's state-based `TextField` (`:5597-5639`). A focusable popup closes the keyboard and
  shifts the list (`:1298-1303`). A sticker is drawn bare and its branch skips the transcript footer
  (`:2566-2576`, `:3930-3941`).
- The catalog is "deliberately minimal", with three named concessions, and versions are "kept in
  lockstep with Scan.Android's catalog wherever the same library appears"
  (`android/gradle/libs.versions.toml:1-31`); Scan.Android uses CameraX 1.5.1
  (`Scan.Android/gradle/libs.versions.toml:16`). Playback is `VideoView`, a `SurfaceView`
  (`ui/chat/AttachmentViewer.kt:373`). `CallStateSource` has a `NONE` default for injection
  (`calls/CallManager.kt:88-99`).
- `MainActivity` declares no `android:configChanges` (`android/app/src/main/AndroidManifest.xml:86-90`),
  so a fold, an unfold, a split-screen resize, a dark-mode change or a keyboard attached rebuilds it.
  `readVideoMetadata` folds a rotation into the reported size only (`data/repo/MediaPrep.kt:519-526`) —
  on a square file a rotation is invisible to it — and is `private` (`:526`). Only calls ask for audio
  focus (`calls/CallAudio.kt:118-147`), and nothing listens for `ACTION_AUDIO_BECOMING_NOISY`.

**Windows** (`win/src/FamilyConnect.App/`)

- Send is a TEXT button in an Auto-width column (`Views/ChatsView.xaml:177`); the recording bar is a
  row above the card (`:109-120`); the error line is `ComposerError` (`:180`); a polite live region
  already exists (`:100`).
- Enter on an empty box does nothing (`Views/ChatsView.xaml.cs:4452-4455`); "Record Audio" is offered
  in every chat (`:4587-4590`); a chat switch CANCELS a recording (`:815-817`); `callBusy` is ignored
  by the recorder (`:157`); the rail (`MainWindow.xaml.cs:399-411`) and closing to the notification
  area (`:607-616`, where `args.Cancel` already turns a close into a hide) leave the microphone on; the
  `AddHandler(…, true)` precedent is `Views/ChatsView.xaml.cs:3068-3070`; a queued sticker has its own
  pending element (`ChatsView.xaml.cs:1864-1868`, `:4745`); originals are downloaded whole because the
  player cannot send `Authorization` (`:2983-2986`).
- The #74 keep path skips Faststart (`Services/MediaPreparing.cs:221-225`); `PosterAsync` is private
  (`:923`). No `KeyboardAccelerator` and no camera capture exist anywhere. CI builds the MSIX but
  never runs the app (`.github/workflows/ci.yml:788-800`).

**Web** (`web/src/`)

- Send is a text button `disabled={empty || props.busy}` (`views/composer.rs:658-663`), and `busy`
  includes `recording_on` (`views/conversation.rs:1061`); a disabled button receives no pointer
  events. An empty composer's send already returns early (`views/composer.rs:391-396`).
- `Listening::in_the_click()` resumes the AudioContext synchronously inside the click, which Safari
  needs (`recorder.rs:213-244`). The recording bar's Esc CANCELS (`views/attach.rs:324-331`); leaving
  the pane cancels (`views/conversation.rs:775-794`); `visibilitychange` is handled only for reads and
  the network (`main.rs:229-275`); `start_recording` does not check `on_call` (`views/conversation.rs:55-57`,
  `:739-774`). The `beforeunload` guard asks only while the outbox or the staging strip holds something
  (`main.rs:179-206`). Closing the tab is a sign-out, and nothing a person wrote is kept on the device
  (`session.rs:1-13`).
- The typing line is the VISIBLE typing indicator, `aria-live="polite"` (`views/conversation.rs:1329`);
  there is no hidden announcement node. The report inbox's line (`views/family.rs:665-676`) sees a
  report's kind and name only (`model.rs:373-377`). The `MediaRecorder` engine has no audio tap
  (`recorder.rs:444-452`), so it can measure nothing. `h264_config` hard-codes `latencyMode: "quality"`
  inside the configuration it probes (`webcodecs.rs:224-249`), and the module's rule is to probe the
  exact configuration that will be used (`:11-17`).
- `web/text/src/mp4.rs` writes `moov` first; `mp4_read` refuses fragmented files
  (`web/text/src/mp4_read.rs:156-159`). `viewport-fit=cover` is set and no `safe-area-inset` is used
  (`web/index.html:7`). The call self-view is mirrored by CSS (`web/styles.css:1597-1608`). There is
  no web CI job; a change under `web/text/` triggers the Windows job, which regenerates the oracle
  fixtures the other ports check (`ci.yml:117-127`, `:740-767`) — but the copies check loops over three
  hard-coded `media-plan-vectors.json` paths (`:761-763`), the path filter names only those copies
  (`:123`), and nothing runs `web/text`'s own tests: the only `cargo test` in CI is the server's (`:239`).

### Assumed, not checked

- That the thresholds below feel right on devices: the 500 ms hold, the 60- and 100-unit slides, the
  1 s minimum, the 5 s Undo. They are constants, tuned after one device session (Blocked 3).
- The exact frame size CameraX writes under a 1:1 `ViewPort` on a given phone (Blocked 4).
- Everything at runtime on Windows: the frame-source preview on the owner's webcam, `CornerRadius` or an
  ellipse composition clip on `MediaPlayerElement`, a 480 × 480 `MediaTranscoder` output with 64 kbit/s
  AAC, what a touch or pen hold does to a `Button` with `IsHoldingEnabled = false`, whether `MediaCapture`
  initialised for `AudioAndVideo` shows the microphone as in use before recording starts, and which API
  tells a packaged WinUI 3 app that the session locked (Blocked 1).
- That Safari 26 drives the live canvas → `VideoFrame` → `VideoEncoder` path reliably, and that a second
  `getUserMedia({audio})` inside the Record click does not ask again (Blocked 2).
- That adding the microphone to a running capture session at Record costs the Apple preview no visible
  hitch.
- That AVFoundation keeps a 480 × 480 output steady when the capture angle follows a turning iPad
  mid-take, and whether CameraX applies a target rotation changed during a recording (Blocked 5).
- That −60 dBFS separates a muted microphone from a quiet room on real devices. A room's own noise sits
  well above it, which is why "not silent" guards against a muted microphone and nothing else.
- Whether a system or browser permission prompt takes window focus — on the Mac, on Windows and in each
  browser — and whether each browser reports a locked screen through `visibilitychange`. The rules below
  are written so that neither answer can close a preview or keep a recording running.
- How a camera that goes away surfaces: a USB webcam pulled out, a Continuity Camera iPhone moved.
- Whether a mobile browser fires `click` after a long touch press. The plan acts on the touch's
  `pointerup` (S8.8), so it does not depend on the answer.
- How today's iOS playback behaves with the silent switch on: nothing sets a playback category.
- That an old app's cache, rewritten by a later page read, regains the flag — a circle received
  before an upgrade may draw square until then.
- Whether Chrome on Android encodes H.264 + AAC on a given phone: probed at run time, never assumed.

## The hard parts

1. **No undo after Send.** Every gesture that sends needs a way to abort before, and a grace after.
2. **A long press is taken** on every platform, and the strongest habit people bring is "hold the
   microphone to talk".
3. **A camera in a family chat** must never film somebody who did not mean it, and never send what
   they have not seen.
4. **Six kinds of input** — finger, pen, mouse, keyboard, screen reader, switch — on five codebases
   that have to behave the same.
5. **Interruptions** — calls, locks, backgrounding, chat switches — are mishandled today in a
   different way on every client.
6. **Capture is new code everywhere**, and uneven: Windows' preview and clipping, the web's encoders,
   Android's dependency policy.

## How the issue's proposal was checked, and what changed

Three complete interaction models were written and then judged by three independent reviewers, each
through one lens — everyday family use; privacy, safety and accessibility; cross-platform
feasibility:

| Model | In one line | Family use | Safety & accessibility | Feasibility |
|---|---|---|---|---|
| **A. The issue, made robust** | tap = voice; hold = the round camera, ready but not recording; Record → Stop → Send | 6.5 | **7.5** | 6.5 |
| **B. The messenger convention** | a tap switches microphone ↔ camera; a hold records; release sends | 4.5 | 4.5 | 4 |
| **C. Tap to record, hold to talk, video on purpose** | tap = hands-free voice; hold = walkie-talkie; video from a labelled control | **8** | 6.5 | **7.5** |

**C won two lenses of three and is the base.** The plan grafts the best of A and B into it and honours
every dealbreaker any reviewer raised.

### What the issue got right, and is kept

- **The microphone lives in the Send slot.** Nothing new to find, and the field never shifts.
- **A tap — a click — means voice**, on every platform and with every input: a tap starts recording
  and the same spot sends it. That is the issue's "just by clicking", made possible: one place, two
  taps.
- **The circle**: a square clip drawn round, behind a flag old apps ignore.
- **Video is one step from the button.** Not by a hold: a video button sits inside the empty field,
  right beside the microphone, and the microphone's secondary menu and accessibility action offer it
  too.

### What changed, and why

- **A long press does not open the camera.** Checked against the platforms: a long press is a context
  menu on iOS, a tooltip on Material icon buttons, a right-click on Windows touch, and a context menu
  or text callout in mobile browsers, while a mouse and a keyboard have no long press at all. Checked
  against people: "hold the microphone" means TALK to nearly everybody who has used WhatsApp or
  Telegram, so the issue's gesture would light the camera for exactly the people who meant to send
  their voice — and, at the platforms' ~500 ms, for slow pressers too. Checked against the code: a
  first press raises the permission prompts, which break a hold, and on the web a touch `pointerdown`
  is not even user activation. Model A softened all this with a camera that opens "ready, not
  recording", an explainer and a "record voice instead" button, and the safety reviewer rated it the
  safest of the three — but the camera light still came on every time somebody forgot, and video
  stayed the hardest feature to find and the slowest to send.
- **The hold is kept — for talking.** Holding the microphone is the walkie-talkie people already
  know, on touch screens, and only ever as a shortcut: a tap does the same job without holding
  anything.
- **Video is reached on purpose.** A labelled video button opens a preview; nothing records until
  Record; Record turns into Stop, never Send; Send exists only after the clip has been seen.
- **Tap-to-switch (model B) is rejected outright.** It is the design WhatsApp needed three fixes to
  walk back and Telegram's own web clients have dropped, and it puts a hidden mode on the most-used
  button.

### What was grafted in

- **From A**: Record → **Stop** → Send in one slot, so no clip leaves unseen; dimmed controls that
  **explain** when tapped instead of being disabled; **"record a voice message instead"** in the
  preview; **keep the screen awake** while recording; **ask before deleting** a recording of 10 s or
  more; when words are typed or items staged, a recording started from the menu offers **Stop only**;
  a recorder that covers the whole window; the strict server check (equal sides, 1–720).
- **From B**: a hold measured from where it began, in window coordinates; a system touch-cancel
  **locks** the recording instead of losing it; **Esc and Back never delete**; no keyboard shortcut
  ever opens the camera; Magic Tap stops a recording and never starts one; an interrupted
  recording comes back as its own **"not sent"** row; an opt-in, per-device **Review Before
  Sending**; `CHECK (NOT (round AND sticker))`.
- **From the safety review**: recording never starts on touch-down; a one-second floor everywhere; an
  activation guard in both directions after the slot's own activation; the preview closes on inactivity
  and when the window loses focus; the lifecycle fixes come BEFORE any new way in (Phase 0).
- **From the feasibility review**: Windows touch and mobile browsers are tap-only until device trials;
  the shared rules live in `web/text` with vectors the ports check; web video recording on Safari
  waits for a real-device trial; no video entry at all against a server without the discovery keys.

### The one question with no precedent: a release that sends

The reviewers disagreed on whether letting go of the walkie-talkie may send — one allowed it behind
guards, one only with a pre-upload Undo or with review by default. The plan does all of it at once
(S2.3, S2.6): a release counts only after a second of sound; a silent recording is never sent;
sliding toward the field aborts before the release; the FIRST release on a device goes to review with
one line saying what letting go does, and so does every release while a screen reader or Switch
Control runs; and every other release waits **5 seconds with an Undo** before anything is uploaded. Nothing in the product says or implies that a sent message can be
deleted.

## The chosen model

1. **One slot.** The composer's trailing button is **Send** when there is something to send and a
   **microphone** when the composer is empty. A tap, click, Enter/Space, screen-reader activation
   or ⌥⌘R/Ctrl+Shift+R starts a **hands-free** voice recording when the activation completes; the
   same slot, now an arrow, sends it; Stop keeps it to listen to and caption.
2. **Holding is a touch shortcut.** On iPhone, iPad and Android, holding the microphone is a
   walkie-talkie: recording begins at the hold threshold; slide toward the field to cancel, up to
   lock. Letting go after at least a second sends after a 5-second Undo — or, the first time on a
   device and whenever a screen reader or Switch Control runs, keeps it for review.
3. **Video is reached on purpose.** A **video button** in the empty field, the paperclip's **Record
   Video Message**, the microphone's right-click menu and its accessibility action open a window-wide
   preview. Nothing records until **Record**, and the microphone opens only then; Record becomes
   **Stop**; **Send** exists only after Stop; the camera goes off at Stop.
4. **Nothing leaves by accident, and nothing is lost by accident.** Interruptions stop and keep; an
   interrupted recording waits in its own "not sent" row, with its reply and caption, and never leaves
   with anything else; Esc and Android's Back stop, never delete; deleting 10 seconds or more asks
   first, and so does closing a window or quitting over a finished video clip.
5. **Older apps still get the message.** Voice changes nothing on the wire. A round video is an
   ordinary H.264/AAC MP4 carrying `round: true`; a 1.1 or 1.2 app shows a square video, and a server
   without the feature is offered no video at all.

| To … | On a touch screen | With a mouse or keyboard | With a screen reader |
|---|---|---|---|
| send a short voice reply | hold, talk, let go | click, talk, click | double-tap, talk, double-tap |
| record hands-free, then send | tap, talk, tap | click, talk, click (or Return) | double-tap, talk, double-tap |
| listen first, or add words | tap, talk, Stop, play / type, Send | the same | the same, or the action "Stop and listen first" |
| send a round video | video button, Record, talk, Stop, Send | the same | the action "Record video message", then the same |

## The specification

Units: **pt** on Apple platforms, **dp** on Android, **epx** on Windows, **CSS px** on the web —
"units" below means the platform's own. Strings are the English source strings, translated later
like every other; Android uses sentence case for menu items and buttons, shown where it differs.
Every string is listed once more in S10.

### S1. Rules every client follows

#### S1.1 Constants

| Constant | Value | Notes |
|---|---|---|
| **Activation guard** | 600 ms | The slot ignores activation for 600 ms only after its OWN activation changed it: a send that empties the composer, a tap or hold that starts a recording, a Send or a release that ends one (or finds it too short), a Stop in row 3 that stages the note — and Record → Stop → Send in the recorder. A press that goes down while the guard runs is ignored whole: it can become neither a tap nor a hold. A change caused by typing, pasting, a suggestion, deleting text or staging is NEVER guarded, so "ok" or an emoji followed at once by Send still sends. A double tap on Send cannot start a recording; a double tap on the microphone cannot send one. The video button likewise ignores activation for 600 ms after it appears. |
| **Hold threshold H** | max(500 ms, the system long-press duration) | Touch and pen on iPhone, iPad and Android only. iOS: 500 ms (Touch Accommodations' Hold Duration delays a touch before the app sees it — UNCONFIRMED, from Apple's description of the setting). Android: `ViewConfiguration.getLongPressTimeout()`, which follows the person's "Touch & hold delay". |
| **Tap slop** | 20 units | A press that moves farther than this from where it went down, before H, can no longer become a hold. It still counts as a tap when it lifts inside the button's hit area, as any button's does, and does nothing only if it lifts outside — so a slow, unsteady press is never a dead button. |
| **Lock** | 60 units upward | From where the press went down, in WINDOW coordinates, never relative to the button. |
| **Cancel** | armed at 100 units toward the leading edge; disarmed again below 80 | Leading = left in left-to-right layouts, right in right-to-left ones. Window coordinates. |
| **Shortest recording** | 1.0 s | Nothing shorter is ever sent. A hold released sooner keeps recording. |
| **Undo window** | 5 s | After a release that sends. |
| **Voice length** | 5:00; warning at 4:30 | Unchanged limit (`protocol.md:112-113`). |
| **Video length** | `max_round_video_ms` − 500 ms = **59.5 s**; warning at `max_round_video_ms` − 10 000 ms = **50 s** | From the discovery key ("On the wire"), 60 000 ms. |
| **Silence** | no PEAK above −60 dBFS (iOS `peakPower` −60 dB; Android `getMaxAmplitude()` ≤ 32; web sample magnitude ≤ 0.001) | Warned 3 s after a recording starts; a HELD recording that never rose above it is not sent. It is digital silence — a muted microphone — not a quiet room. |
| **Delete asks** | recordings of 10 s or more | |
| **Preview closes** | after 60 s with no Record and no other control used | |
| **Motion** | slot cross-fade 150 ms; recorder fade 200 ms | None under Reduce Motion, Remove animations or `prefers-reduced-motion`. |
| **Targets** | at least 44 pt / 48 dp / 44 epx / 44 CSS px on coarse pointers | Hit areas grow, the visuals and the bar do not: the iPhone's 36-pt Send (`ConversationView.swift:173`) gets a 44-pt hit area. |

#### S1.2 Inputs to the rules

- **empty** — the draft is blank after trimming whitespace AND nothing is staged. A primed reply does
  not count; whatever is recorded carries it (`takeComposer`, `ConversationView.swift:2162-2168`).
- **editing** — an edit is open.
- **call** — a call in any phase but idle or ended: iOS `!CallManager.isIdle`
  (`Core/Calls/CallManager.swift:234-239`), Android `CallManager.state` is `Live`, Windows `callBusy`,
  web `on_call`.
- **busy** — each composer's existing attachment guard: iOS `mediaState.blocksComposer`, Android
  `mediaState.isBusy`, Windows `strip.Preparing || sendingMedia`, web `preparing || locating`.
- **can record** — the platform can record sound at all; on the web that needs `navigator.mediaDevices`,
  i.e. a secure context (`recorder.rs:251-254`).
- **round available** — the server sends `max_round_video_ms` on `GET /families/mine`, AND the device
  has a camera, AND on the web the encoder probe passes (S8.7), AND this build records round video on
  this platform (Phase 3 shipped for it — on Windows after 3d). A build that can only RECEIVE circles
  shows no video entry at all.
- **not sent** — the chat holds a not-sent voice message (S2.8).

#### S1.3 The trailing slot

The first matching row wins.

| # | When | The slot shows | Activating it |
|---|---|---|---|
| 1 | The video recorder is open | — the recorder owns the row (S3) | — |
| 2 | A voice recording runs that started with the composer empty | **Send** arrow, label "Send voice message" — during a hold, the pressed microphone stays under the finger until the recording turns hands-free | stops and sends (S2.5) |
| 3 | A voice recording runs that started with words typed or items staged | **Stop** square, label "Stop recording" | stops; the note is staged beside them (S2.4) |
| 4 | **editing** | **Save**, disabled while the field is blank — today's control | saves the edit. **Never a microphone**, even with the field cleared |
| 5 | not **empty** | **Send** | sends, by today's rules |
| 6 | The assistant chat (`kind = ai`), or not **can record** | **Send**, disabled — today's look | nothing |
| 7 | **call** | **microphone**, dimmed | says "You can record a message after the call." |
| 8 | **busy** | **microphone**, dimmed | says "Wait until the current attachment is done." (an existing string) |
| 9 | **not sent** | **microphone**, dimmed | says "Send or delete the voice message that wasn't sent first." |
| 10 | otherwise | **microphone** | starts a hands-free voice recording (S2.2); on touch, holding it is the walkie-talkie (S2.3) |

- **Thread composers are not changed**: no microphone, no recording, today's Send. None has an attach
  menu, a recorder or staging today (`Views/ThreadView.swift:390-464`, `ui/thread/ThreadScreen.kt:196-238`,
  `Views/ChatsView.xaml:218-240`, `views/thread_panel.rs:7-11`).
- **Dimmed is not disabled.** A dimmed control looks disabled, stays focusable and hittable, and says
  WHY when activated, in the composer's existing notice line (iOS `composerNotice`, Android the media
  strip, Windows `ComposerError`, web `notice`) — the web's paperclip already "says the reason instead
  of opening" (`views/attach.rs:53-58`). Screen readers get the same sentence with the control (S6).
- **The slot never changes size or place.** iOS and Android are already icon buttons. Windows and the
  web replace their text Send/Save with a fixed icon button (S8.6, S8.7). Send ↔ microphone is a
  150 ms cross-fade (instant under Reduce Motion).
- **Glyphs**: iOS/Mac `mic.circle.fill`, `arrow.up.circle.fill` (today's), `stop.circle.fill`;
  Android `Icons.Filled.Mic`, `Icons.AutoMirrored.Filled.Send`, `Icons.Filled.Stop`; Windows Segoe
  Fluent E720, E724, E71A (Save E73E); the web inline SVG. Names are checked against the symbol sets
  when built.
- **Return/Enter in an empty field never records.** iOS ⌘↩ and the Mac's Return bind only in rows 2
  to 5, never to a microphone; Windows (`ChatsView.xaml.cs:4452-4455`) and the web
  (`composer.rs:391-396`) stay no-ops.

#### S1.4 The video button

- **Where**: inside the text field, at its trailing edge, centred on the first line — the space an
  empty field is not using, so the row never needs to fit another control. A 22-unit glyph in a
  44-unit target: iOS/Mac `video.circle`, Android `Icons.Outlined.VideoCameraFront`, Windows Segoe
  Fluent E714, the web an inline SVG. The field gets trailing padding while it shows, so a placeholder
  never runs under it (a long translation of "Message" is truncated, never overlapped).
- **Shown** when the slot is a microphone (rows 7–10) AND the chat is a family or direct chat AND
  **round available**. **Dimmed**, with the same sentence, in rows 7 and 8. Usable in row 9 — the
  not-sent rule is about voice.
- **Hidden** as soon as a character is typed (the field needs its width), while anything is staged,
  while editing, in the assistant chat, in threads, against a server without the keys, on a device
  without a camera, and in a browser whose probe fails (the paperclip item explains there).
- Label "Record video message"; desktop and pointer tooltip "Record a video message".
- **Activation opens the recorder** (S3). A long press on it does nothing special. It ignores
  activation for 600 ms after it appears (S1.1) — it appears beside the slot the moment a text Send
  empties the field, and a second tap that drifts left must not turn the camera on.

#### S1.5 The paperclip menu

| Item | Where | Disabled | Does |
|---|---|---|---|
| **Record Voice Message** (Android "Record voice message") — replaces "Record Audio" | family and direct chats; **removed from the assistant chat** on every client | busy, editing (as today), during a call, and while a not-sent voice message waits | starts a hands-free recording. With words typed or items staged, row 3: the slot is Stop and the note is staged beside them. |
| **Record Video Message** (Android "Record video message") — new, right below | family and direct chats when **round available**. In a browser whose probe fails it is SHOWN and, chosen, says "This browser can't record video messages. Voice messages work." instead of opening | busy, editing, during a call | opens the recorder (S3). Works with words typed or items staged: a round video always travels alone, the words and staged items stay in the composer, and a primed reply goes with the video — the sticker's rule (`ConversationView.swift:2955-2978`). |
| iOS **Camera** (the system camera) | unchanged | | photos and ordinary videos |
| Android **Take video** — was "Record video" | unchanged position, beside "Take photo" | | the system camera; renamed so it cannot be read as the video message |

The Mac, Windows and the web have no system-camera item and get none. The paperclip keeps its label
"Attach a photo, video or file" — two iOS UI tests find the composer by it.

#### S1.6 The microphone's secondary menu, actions and shortcuts

- **Secondary menu** — right-click, Control-click, a two-finger click, Shift+F10, the Menu key, an
  iPad pointer's secondary click; **never a touch hold**. Two items: "Record Voice Message" (the Mac
  shows ⌥⌘R beside it) and "Record Video Message" (only when **round available**; with a failed web
  probe it explains instead). In rows 7–8 both items explain; in row 9 Record Voice Message explains
  and Record Video Message opens the recorder, as the video button and the paperclip item do.
- **Accessibility action** on the microphone: "Record video message", when **round available**.
- **Shortcuts**: ⌥⌘R on the Mac (menu bar) and an iPad's hardware keyboard (a hidden button, the
  ⌘V pattern at `ConversationView.swift:1787-1802`); Ctrl+Shift+R on Windows and on Android with a
  hardware keyboard. It starts a voice recording; pressed during one, it STOPS it into review — a
  shortcut never sends. **No shortcut opens the camera.** None on the web, where Ctrl/⌘+Shift+R
  reloads the page.

#### S1.7 Rules that hold everywhere

- **One recording at a time in the whole app.** Starting one anywhere — another chat, another Mac
  window — stops any other first: a voice recording is parked (S2.8), a video goes to REVIEW.
- **Starting a recording pauses whatever is playing** (S5.3), and no app sound plays while recording:
  every play control — voice notes, circles, a staged or not-sent note — is dimmed and says "You can
  play this after recording."
- **No recording during a call, in any phase.** The toolbar call buttons are disabled while recording
  (`ConversationView.swift:955-980`, `ChatScreen.kt:1102-1114`, Windows' `callBusy` buttons, the
  web's call buttons).
- **Nothing is ever sent by a length limit running out or by a shortcut, and an interruption never sends
  a recording.** What sends is the Send control (or Return/Enter on it), the end of the Undo window
  after a release, and the recorder's Send. The Undo window is the one timer that sends: a grace period
  after a release that had already decided, which Review Before Sending turns off (S9) and which a
  screen reader or Switch Control never meets (S2.6). The one place an interruption sends is INSIDE
  that window, which it only shortens.
- **Keep the screen awake while recording**: iOS `UIApplication.shared.isIdleTimerDisabled`; Android
  `FLAG_KEEP_SCREEN_ON` (as `ui/call/CallScreen.kt:233-235`); Mac `ProcessInfo.beginActivity([.idleDisplaySleepDisabled,
  .userInitiated])`; Windows `DisplayRequest.RequestActive()`; the web `navigator.wakeLock.request("screen")`
  where it exists. On phones and tablets — and in a phone's browser — the same calls also hold while a
  voice note or a circle PLAYS: playback pauses when the app goes to the background (S4), so auto-lock
  must not cut a long note short.
- **The web's slot is never `disabled`** while it is the recording control; dimmed states use
  `aria-disabled="true"` and keep their events.
- **Copy never implies a sent message can be removed.** No string, tip or notice says or suggests it.

### S2. Voice messages

#### S2.1 States

| State | What the composer shows |
|---|---|
| **Idle** | the slot per S1.3 |
| **Pressed** | a finger or pen is down on the microphone, before H; the button shows its pressed state, nothing records |
| **Holding** | recording; the finger is still down; the hold row (S2.3) |
| **Hands-free** | recording; the recording row (S2.4) |
| **Undo** | stopped, about to be sent; the Undo row (S2.6) |
| **Review** | stopped and staged; the chip above the field (S2.7) |
| **Not sent** | stopped by something other than the person; its own row (S2.8) |

| From | Event | To |
|---|---|---|
| Idle | activation (row 10), permission granted | Hands-free |
| Idle | activation, permission not yet asked | the prompt → Allow: Hands-free; Don't Allow: Idle with the denial notice |
| Idle | activation, permission denied | Idle with the denial notice and Open Settings |
| Idle | finger/pen down (iPhone, iPad, Android) | Pressed |
| Pressed | lifts before H, inside the button's hit area | as activation |
| Pressed | moves beyond the slop | still Pressed, but it can no longer become a hold |
| Pressed | lifts outside the button's hit area | Idle (nothing) |
| Pressed | reaches H, still within the slop | Holding — or, without permission, the prompt and then Idle |
| Holding | slides up 60 | Hands-free (locked) |
| Holding | lifts, cancel armed | Idle (deleted) |
| Holding | lifts, under 1.0 s recorded | Hands-free |
| Holding | lifts, silent | Review |
| Holding | lifts, first release on the device, Review Before Sending on, or a screen reader or Switch Control running | Review |
| Holding | lifts, otherwise | Undo |
| Holding | system touch-cancel | cancel armed: Idle (deleted); otherwise Hands-free, or Not sent if the app went to the background |
| Holding, Hands-free | 5:00 | Review |
| Holding, Hands-free | an interruption (S4) | Not sent (under 1.0 s: deleted) |
| Hands-free | Send (row 2) | Idle; sent (under 1.0 s: deleted, "too short") |
| Hands-free | Stop, Esc, VoiceOver's escape, Android's Back, Magic Tap, the shortcut, or the slot in row 3 | Review (under 1.0 s: deleted, "too short") |
| Hands-free | Delete | under 10 s: Idle (deleted); otherwise stopped, then asked: Delete → Idle, Keep → Review |
| Undo | Undo | Review |
| Undo | 5 s pass, or any other action (S2.6) | Idle; sent |
| Review | Send | Idle; sent with the caption |
| Review | ✕ | Idle (10 s or more asks) |
| Review | the person leaves the chat | Not sent, taking its caption along |
| Not sent | its Send | Idle; sent with its own reply and caption, and nothing else |
| Not sent | its ✕ | Idle (10 s or more asks) |

#### S2.2 Starting, and permission

- **With permission**, activation starts recording at once: the recording row replaces the field
  (S2.4), a light haptic on phones, "Recording" announced, the screen kept awake, playback paused.
  With a screen reader running, the microphone opens only once "Recording" has been spoken, and the
  timer starts with it (S6), so the app's own voice does not open every note.
- **Not yet asked**: the system prompt appears. On Allow, recording starts — the tap meant "record".
  On Don't Allow, the denial notice.
- **Denied**: each client's existing denial sentence, plus a button that opens the right place —
  iOS "Family needs permission to use your microphone. Turn it on in Settings." with **[Open
  Settings]** (`UIApplication.openSettingsURLString`); the Mac's System Settings sentence
  (`AudioRecorder.swift`, `message(for:)`) with **[Open System Settings]**
  (`x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone`); Android its own
  sentence, with [Open Settings] once the denial is permanent (`ACTION_APPLICATION_DETAILS_SETTINGS`);
  Windows the iOS sentence with [Open Settings] (`ms-settings:privacy-microphone`, after
  `AppCapability.Create("Microphone").CheckAccess()`); the web its sentence ("… Allow it in your
  browser's settings for this site.", `recorder.rs:66`) and no button, because a page cannot open
  browser settings.
- **From a hold**, see S2.3: a prompt raised by a hold never starts a recording.

#### S2.3 The hold — iPhone, iPad (finger and Pencil), Android (finger and stylus)

| Moment | What happens |
|---|---|
| The finger goes down on the microphone | Nothing records yet. Pressed state. |
| It moves more than 20 units before H | It can no longer become a hold. Lifting inside the button is still a tap (S1.1); lifting outside does nothing. |
| It lifts before H inside the button | It was a tap: hands-free recording starts (S2.2). |
| It is still down at H, within the slop | **Recording starts.** A medium haptic. The hold row replaces the field: red dot, "0:00", the level meter and "‹ Slide to cancel". A lock — a lock glyph over an up chevron — floats 8 units above the slot, outside the bar, so the bar keeps its height. The keyboard, if it was up, stays up: the field stays in place, invisible and still focused, under the row — and hidden from screen readers while the row covers it. |
| It slides 100 units toward the leading edge | **Cancel armed**: the row turns red and reads "Release to cancel"; a selection haptic. Back under 80 disarms it. |
| It slides 60 units up (cancel not armed) | **Locked**: a selection haptic, "Recording locked" announced, the row becomes the recording row (S2.4) and the keyboard goes down. Lifting now does nothing. |
| It lifts, cancel armed | Deleted; "Recording deleted" announced. |
| It lifts with under 1.0 s recorded | **Keeps recording, hands-free**; the row says "Still recording. Tap Send when you're done." for 3 s. |
| It lifts with 1.0 s or more and nothing rose above the silence level | Stops into review (S2.7) with "We didn't hear anything." |
| It lifts with 1.0 s or more — the first release ever on this device, Review Before Sending on (S9), or a screen reader or Switch Control running | Stops into review; the first time only, with "Next time, letting go will send it." |
| It lifts with 1.0 s or more, otherwise | Stops; the Undo window opens (S2.6). |
| The system cancels the touch (an alert, Control Centre, the notification shade, a rotation) | Cancel armed: deleted. Otherwise locked — still recording hands-free — or, if the app has gone to the background, stopped and parked (S2.8). Telegram's rule (`ChatActivityEnterView.java:3074-3089`). |
| 5:00 while held | Stops into review with "Recording stopped at five minutes."; the later lift does nothing. |
| Permission not yet asked, at H | The prompt appears and NOTHING records, whatever the answer; on Allow, "You can record now." |
| A call, busy, or a not-sent message, at H | Nothing records; the dimmed row's sentence (S1.3). |

- **Where the hold exists**: iPhone, iPad, Android phones and tablets — finger, Apple Pencil, stylus.
  **Nowhere** with a mouse, trackpad or keyboard; **not on Windows** with any input; **not in a
  browser** in this version (S8.6, S8.8). There a press of any length is a click.
- **VoiceOver's double-tap-and-hold** passes the touch through, so the hold works there too; it is
  never needed and never advertised (S6), and its release always goes to review — by the time a
  screen reader had said the row out loud, a five-second Undo would have left almost nothing of it.
  TalkBack's double-tap-and-hold is a discrete action the microphone does not expose, so it does
  nothing.

#### S2.4 The recording row (hands-free)

It replaces the field — and the paperclip, sticker, `@ai` and video buttons — in the same row, at the
same height. The reply banner above stays.

| | Phones and tablets | Mac, Windows, web on a desktop |
|---|---|---|
| Leading | trash icon, label "Delete recording" | button "Delete" |
| Middle | red dot, "0:42" in monospaced digits, the level meter | the same |
| Before the slot | stop icon, label "Stop recording" | button "Stop" |
| The slot | Send arrow, label "Send voice message" | the same; tooltip "Send voice message" |

- **Started with words typed or items staged** (from the paperclip or the shortcut): the middle Stop
  is not drawn and **the slot is Stop** (row 3). The words are hidden behind the row and must not
  leave unseen; Stop stages the note beside them.
- **Too narrow** (Slide Over, large text): the level meter goes first; on desktops the text buttons
  then become icons with the same labels.
- **Focus** moves to the slot when recording starts and stays there while it runs. With a keyboard:
  Return/Enter activates the slot, Esc is Stop, Tab walks Delete, Stop and the slot. On an iPad ⌘↩
  sends too. When the recording ends by Send, Delete or "too short", keyboard focus returns to the
  text field, so a second Enter cannot open the microphone again; on iPhone, iPad and Android, where a
  screen reader's focus is its own, that focus stays on the slot, which now says "Record voice message".
  Where the screen reader follows keyboard focus — the Mac, Windows, the web — it goes to the field too.

#### S2.5 Ending a recording

- **Send** (the slot): with 1.0 s or more, it stops and sends now. The note is prepared without
  re-encoding (`Core/MediaPrep.swift:535-552`) and handed to the outbox, which writes the row before
  the first byte (`protocol.md:5858-5868`), with the primed reply; "Voice message sent" is announced.
  Under 1.0 s: discarded with "That recording was too short." If preparing fails, the note lands in
  review with the error — never lost.
- **Stop** (the button, Esc, VoiceOver's escape gesture, Android's Back, Magic Tap, a second ⌥⌘R/Ctrl+Shift+R): with 1.0 s or more it goes
  to review and "Ready to review, 0:42" is announced; under 1.0 s, discarded with "That recording was
  too short."
- **Delete**: under 10 s, deleted at once, "Recording deleted" announced. At 10 s or more the
  recording STOPS FIRST, then "Delete this recording?" [Delete] [Keep]; Keep goes to review.
- **5:00**: stops into review with "Recording stopped at five minutes." — never sent. At 4:30 the
  timer turns orange and "30 seconds left" is SHOWN beside it (in the level meter's place) and
  announced — words as well as colour (WCAG 1.4.1). This fixes `AudioRecorder.swift:198-201` and
  `VoiceRecorder.kt:84`.

#### S2.6 The Undo window (after a release that sends)

- The hold row becomes **"[Undo]  Sending voice message · 0:12"**, with a 2-unit line along its bottom
  edge emptying over 5 s; under Reduce Motion the line gives way to "Sending in 5", counting down once
  a second and not announced. It takes the FIELD's place only — the video button inside the field goes
  with it; the paperclip, sticker and `@ai` buttons stay beside it, usable. The note is NOT in the outbox
  yet: nothing has left the device. The slot shows the microphone (the composer is empty), under the
  activation guard. The field stays focused under the row, as it did under the hold row, and hidden
  from screen readers, so a keyboard that was up stays up and typing simply carries on.
- **Undo** → the note goes to review (S2.7) and nothing is sent; "Ready to review, 0:12" is announced.
- **5 s pass** → the note goes to the outbox exactly as S2.5's Send; "Voice message sent".
- **The window ends early — and the note is sent — on any other action**: a character typed, the
  microphone, the paperclip, a sticker, leaving the chat, the app going to the background, a call
  starting. Letting go was the person's decision; the window is a grace period and must never turn
  into a draft somebody believes was sent.
- **A crash cannot lose it.** At the release the note is written to the parked store (S2.8) marked
  "sending"; the outbox hand-off or Undo removes that entry, and at launch an entry still marked
  "sending" becomes a not-sent row — never an orphan swept away while its sender believes it went.
  (On the web the store is the tab, as in S2.8, and a tab that dies takes it along.)
- **A screen reader or Switch Control never meets this window**: a held release goes to review there
  (S2.3). Review Before Sending (S9) turns the window off for anybody — its WCAG 2.2.1 "turn off".

#### S2.7 Review

- The staged chip above the field: **"[▶] Voice message · 0:42 [✕]"**; while playing, "[❚❚] 0:12 /
  0:42". ▶ plays the LOCAL file and pauses anything else playing — on iPhone and iPad through
  `.playback` (S5.3), never the earpiece a recording session leaves the route on.
  This is new on every client — none can play a staged note today (`StagedAttachment.swift:105-121`,
  `win/src/FamilyConnect.App.Logic/ComposerStaging.cs:200-207`, `ChatScreen.kt:5084-5126`).
- The field comes back, focused where it was, for an optional caption. Send sends both as one message
  by today's rules — the consent question only if the words mention `@ai`.
- ✕ (label "Delete recording") removes it (10 s or more asks "Delete this recording?").

#### S2.8 Not sent

- **When**: a recording is stopped by something other than the person, with 1.0 s or more recorded —
  a call, the app going to the background or the screen locking, a desktop session locking or the
  computer sleeping, a window hidden, minimised or closed, Siri or an alarm or another app taking the
  microphone, the recorder failing; a note a crash left "sending" (S2.6); and a voice note still in
  review when the person leaves the chat, which takes the words in the field along as its caption and
  leaves the field empty.
- **What**: a row above the field, **"Voice message not sent · 0:42 [▶] [Send] [✕]"**, quoting the reply
  it was recorded under and showing its caption, if it has them. Its Send (label "Send voice message")
  sends the note with THAT reply and caption and nothing else — the composer's own reply, possibly
  primed since for some other text, stays with the composer. It is **never carried by another Send**, so
  a recording nobody finished deciding about cannot ride out with the next text, and its caption never
  leaves without it. ✕ (label "Delete recording") deletes; 10 s or more asks.
- **While it exists** the microphone is dimmed with "Send or delete the voice message that wasn't sent
  first." (S1.3 row 9), and so is the recorder's "Record a voice message instead" (S3). Typing and
  sending text work as usual.
- **Where it lives**: per chat, in the app's own storage on Apple, Android and Windows — the file, its
  length, the id of its reply and its caption — so it survives the app being closed, and deleted at
  sign-out; files no entry names are swept at launch. On the web it lives for the life of the tab,
  which asks before closing (`beforeunload`) while one exists, a recording runs or a video clip waits in
  REVIEW. A phone's browser can still lose it without asking: `beforeunload` "is not reliably fired,
  especially on mobile platforms" (MDN), exactly as that browser can lose the web's outbox today —
  the web keeps nothing a person wrote on the device (`session.rs:1-13`), and this plan does not change
  that.
- **It arrives in Phase 0**, before anything else parks into it (Phases).
- This reverses `ComposerDrafts.swift:13-18` — for recordings only, because a photo can be picked
  again and a recording cannot be made again.

#### S2.9 Feedback

- **Timer**: m:ss in monospaced digits, from a monotonic clock (Android's wall clock keeps counting past
  a stopped recorder, `VoiceRecorder.kt:48-49`).
- **Red dot**: 8 units, pulsing between 100 % and 40 % opacity once a second; steady under Reduce
  Motion.
- **Level meter**: five bars, each 3 × 16 units, lit at −50, −40, −30, −20 and −10 dBFS of the PEAK
  level — the same measure as the silence check, so the bars light alike on every client: iOS and Mac
  `AVAudioRecorder.peakPower(forChannel:)` (`isMeteringEnabled`, read in the existing 200 ms ticker),
  Android `getMaxAmplitude()` — itself a peak — in the existing 200 ms tick (`ChatViewModel.kt:1627-1633`),
  the web's largest sample magnitude per block from the worklet tap. Windows draws the dot only in this
  version: its `MediaCapture` audio recording exposes no level. Neither does the web's `MediaRecorder`
  engine (`recorder.rs:444-452`), which has no tap: it too draws the dot only.
- **Silence**: 3 s after recording starts, if no peak has risen above −60 dBFS: "We can't hear
  anything. Is the microphone muted?", shown and announced; the recording continues and the line goes
  when sound arrives. It catches Android 12's toggle, a Mac without the entitlement and a muted USB
  microphone — a muted microphone, not a quiet room. Not on Windows, nor in the web's `MediaRecorder`
  engine, in this version.
- **Haptics** — phones only, none on iPad, Mac, Windows or the web:

  | Moment | iPhone `.sensoryFeedback` | Android `HapticFeedbackType` |
  |---|---|---|
  | recording starts from a tap | `.impact(weight: .light)` | `ToggleOn` |
  | recording starts at H | `.impact(weight: .medium)` | `LongPress` |
  | lock; cancel armed | `.selection` | `GestureThresholdActivate` |
  | sent | `.success` | `Confirm` |
  | too short; deleted | `.warning` | `Reject` |

### S3. Recording a round video

#### S3.1 Ways in

The video button (S1.4), the paperclip's Record Video Message (S1.5), the microphone's secondary menu
and its accessibility action (S1.6). **Never** a hold, a tap on the microphone or a keyboard shortcut.

#### S3.2 Permission

The camera AND the microphone. **Both statuses are read first**: if either is already refused, the
recorder opens straight to that refusal and raises no prompt for the other — a camera grant is no use
to somebody whose microphone is off. Otherwise, the first time, the recorder opens with a neutral
circle and "Video messages need the camera and the microphone." while the system asks — iOS and Mac
the camera, then the microphone; Android one `RequestMultiplePermissions`; Windows `MediaCapture`
initialised for `AudioAndVideo`; the web one `getUserMedia({video, audio})`, whose audio track is
stopped at once (S3.4). Allowed → PREVIEW. **Nothing records**, and the microphone is not opened until
Record. **The camera is off in every refusal state.**

- Camera refused: a slashed-camera glyph in the circle and "Family needs permission to use your
  camera. Turn it on in Settings." (Mac: "… Turn it on in System Settings › Privacy & Security ›
  Camera."; web: "… Allow it in your browser's settings for this site."), with [Open Settings] or [Open
  System Settings] where the platform can open it (`ms-settings:privacy-webcam` on Windows, after
  `AppCapability.Create("Webcam").CheckAccess()`), and [Record a voice message instead] — dimmed with
  row 9's sentence while a not-sent voice message waits (S1.3).
- Microphone refused: the microphone sentence (S2.2), with [Open Settings] or [Open System Settings]
  where the platform can open it — voice needs the microphone too, so nothing else is offered.
- On the web one combined request cannot say which device was refused: the recorder asks
  `navigator.permissions.query` about "camera" and "microphone" where the browser answers, and
  otherwise says "Family needs permission to use your camera and microphone. Allow them in your
  browser's settings for this site."

#### S3.3 The recorder

- **It covers the whole window.** A dark scrim — opaque under Reduce Transparency — lies over
  everything, the sidebar, rail and toolbar included on iPad, Windows, Android tablets and desktop
  browsers, and takes all input there. On compact widths it is black at 70 %. On regular widths it is
  70 % over the sidebar and rail but only 30 % over the conversation pane — still taking all input —
  so the message being answered stays readable. The Mac's window toolbar cannot be covered by anything
  the content draws, so its items are disabled instead (S8.3). The conversation cannot change
  underneath it; the keyboard is dismissed.
- **Layout**, over the conversation pane: a status line on its own backing, the circle, the reply
  banner, and a control row exactly where the composer row is, with the slot in the Send button's
  place — the thumb never moves.
- **The reply** the video will carry — the composer's primed reply (S1.5) — is quoted in a banner
  just above the control row in every state, with its ✕ to drop it; without one, nothing is drawn.
- **Circle diameter** D = min(320, pane width − 48, pane height − 240 − the reply banner's height when
  there is one), at least 160. In a pane shorter than 480 — a phone on its side — the controls stand
  in a column at the trailing edge, the banner above them, and D = min(320, pane height − 96, pane
  width − 200), at least 160. The layout chosen when RECORDING starts is kept until Stop (S3.5).
- **Ring**: 4 units wide, just OUTSIDE the circle, so it never covers a face.
- How each platform presents it is in S8; it is never a sheet, a dialog or a popover where those would
  shift the layout or could be dismissed by accident.

#### S3.4 States

| | PREVIEW | RECORDING | REVIEW |
|---|---|---|---|
| Camera | on | on | **off** — the light goes out |
| Microphone | **off** — it opens at Record, so no system indicator contradicts "Not recording" | on | **off** |
| Circle | the live front camera, mirrored | the live camera, mirrored | the clip AS IT WILL BE SENT (not mirrored), its first frame with a play glyph; a tap plays it with sound, another pauses |
| Ring | 1 unit, white at 40 % | red, filling clockwise from 12 o'clock over the length limit; orange from the warning | while playing: the accent colour, showing progress |
| Status line | "Not recording" — or, on a platform that must keep the microphone open, "Camera and microphone on · Not recording" — and, the first time on this device, below it: "Only you can see this until you start recording." Before the first frame: "Starting camera…" | red dot and "0:12"; from the warning, "10 seconds left" beside it | "Video message · 0:23" |
| Leading | Close (label "Close") | Delete (label "Delete recording") | Delete; Retake |
| Middle | "Switch camera" on phones and tablets, or "Choose camera" on a desktop with more than one; a microphone button, label "Record a voice message instead" — dimmed with row 9's sentence while a not-sent voice message waits | "Switch camera" — iPhone and iPad only | — |
| **The slot** | **Record** (red disc, label "Record") — dimmed until the first frame | **Stop** (label "Stop recording") | **Send** (label "Send video message") |
| Return / Enter | Record | Stop | Send |
| Esc / Back / VoiceOver's escape | Close | Stop | asks "Delete video message?" [Delete] [Keep] |
| Space | the focused control | the focused control | **play/pause wherever focus is**, caught by the recorder before the focused control — Space never sends, deletes or retakes; those answer Return/Enter, a click or a tap |
| Magic Tap | Record | Stop | play/pause |

**Transitions**

- **PREVIEW → RECORDING**: Record, once the camera has delivered its first frame. The microphone opens
  at Record; the clip begins at the first frame after Record; a medium haptic on phones; "Recording
  video" announced.
- **PREVIEW → closed**: Close, Esc, Back or VoiceOver's escape; **60 s** with no control used ("Camera
  turned off" announced); the window losing focus on a desktop — except while a permission request
  the recorder raised is on screen — or the app going to the BACKGROUND, never the inactive state an
  alert, Control Centre or a permission prompt causes (the line `ConversationView.swift:1062-1070`
  already draws); a call starting.
- **PREVIEW → voice**: "Record a voice message instead" closes the camera and starts a hands-free
  voice recording (S2.4) — one tap undoes a mis-tap on the video button. While a not-sent voice
  message waits it is dimmed and says row 9's sentence instead.
- **RECORDING → REVIEW**: Stop (after the 600 ms guard) with 1.0 s or more; the length limit, with
  "Recording stopped at one minute."; an interruption (S4). At 50 s the ring turns orange and "10
  seconds left" is shown in the status line and announced. Stop under 1.0 s goes back to PREVIEW with
  "That video was too short."
- **RECORDING → PREVIEW**: Delete — under 10 s at once; at 10 s or more it stops first and asks
  "Delete video message?"; Keep goes to REVIEW.
- **REVIEW → sent**: Send. The recorder closes, the circle appears in the thread at once from the
  local file (S5.6), and the outbox uploads it — the row written before the first byte, the bytes
  kept until the ack. "Video message sent" is announced.
- **REVIEW → PREVIEW**: Retake — at 10 s or more it asks "Delete video message?" first; the camera
  comes back on.
- **REVIEW → closed**: Delete (10 s or more asks); Esc, Back and VoiceOver's escape ALWAYS ask, and so
  does closing the window or quitting the app (S4).
- **REVIEW plays** through `.playback` on iPhone and iPad (S5.3) — never the earpiece the recording
  session leaves the route on.
- **Focus** starts on the slot and stays on it as it changes. Tab walks, in reading order, the circle
  (REVIEW only), the reply banner's ✕, then the control row from the leading control to the slot, and
  never leaves the recorder. The recorder is a modal region for screen readers
  (`accessibilityViewIsModal`; Compose `paneTitle` with traversal kept inside; the web `role="dialog"
  aria-modal="true" aria-label="Video message"` with the rest of the page `inert`). Closing returns
  focus to the control that opened it.
- **No caption and no editing**: a round video has no balloon to hold words ("On the wire").

#### S3.5 Camera and picture

- **Which camera**: the front camera first, any camera otherwise. Phones and tablets offer "Switch
  camera" (an existing string) in PREVIEW, and during RECORDING only on iPhone and iPad, where the
  capture session swaps inputs without a break. The Mac starts from `systemPreferredCamera`
  (Continuity Camera included); Windows from the front-panel camera, else the default; the web from
  `facingMode: "user"`. A desktop with more than one camera shows "Choose camera", a menu of their
  system names. The choice is remembered on the device.
- **Mirroring**: the PREVIEW is mirrored, like a mirror and the call self-view
  (`web/styles.css:1597-1608`); the FILE is not — it is the true view, the one the other side of a
  call sees, so a drawing held up to the camera reads correctly. REVIEW shows the file as it will be
  sent.
- **Upright**: pixels are written upright with an identity transform (house practice,
  `Core/MediaTranscoder.swift:24-25`), never with a rotation matrix — Android's check after Stop
  requires rotation 0 and bakes any other into the pixels (S8.4). Desktops have nothing to rotate.
- **Turning the device during a take**:
  - **Phones** (iPhone, Android under 600 dp): from Record until Stop the screen is held in its
    CURRENT orientation — never forced to portrait (WCAG 1.3.4) — and the capture angle is fixed at
    Record (Apple `RotationCoordinator`; Android `VideoCapture.targetRotation` from an
    `OrientationEventListener`). The controls cannot move under the thumb; a phone turned mid-take
    records the rest sideways, and REVIEW shows that before anything is sent.
  - **Where a lock is impossible or ignored** — an iPad (multitasking forbids it), Android at 600 dp
    and wider (Android 16 ignores it), a foldable that changes display, multi-window — the recorder's
    layout chosen at Record never switches during RECORDING (on these screens the pane is never shorter
    than 480, so the control row stays at the bottom), and the capture angle follows the device. The
    output stays 480 × 480 either way; that the picture stays steady while the angle changes mid-take is
    UNCONFIRMED until a device trial (Blocked 5). Until it passes, these devices keep the starting
    angle, as phones do, and REVIEW shows any sideways tail before Send.
- **Reaction effects off by default**: `NSCameraReactionEffectGesturesEnabledDefault = false` in both
  `Info.plist` and `Info-macOS.plist`, so a thumbs-up does not burn fireworks into a family message.
  It is a DEFAULT, from iOS 17.4 / macOS 14.4: a choice the person makes in Control Center, or an older
  system, can still trigger them, and it applies app-wide — the app's video calls get gestures off by
  default too (the SDK header, `AVCaptureDevice.h:2585-2602`).
- **iPad multitasking**: `isMultitaskingCameraAccessEnabled = true` wherever
  `isMultitaskingCameraAccessSupported`; no entitlement is needed at a 17.0 deployment target. The
  system's one-time alert after the first clip recorded while multitasking is not an interruption, and
  `systemPressureState` is watched (S4).

#### S3.6 When it cannot work as planned

- **Can't be made round** — Android's or Windows' square pass fails: REVIEW says "Couldn't make it
  round. It will be sent as a regular video." and Send sends it WITHOUT the flag, as an ordinary video,
  in the spirit of the profile's Rule C (`protocol.md`, "Preparing media before upload").
- **Over the byte ceiling** — impossible at the defaults (≈ 4.2 MB against 12 MiB) but an operator can
  lower it, directly or by lowering `max_attachment_bytes`: "Too big for a video message. It will be
  sent as a regular video.", and Send sends it without the flag.
- **The camera is in use**: "The camera is being used by another app." in place of the picture, Record
  dimmed, "Record a voice message instead" offered. An iPad sharing the screen where multitasking
  camera access is NOT supported: "The camera isn't available while other apps are on screen." A
  recording in progress stops into REVIEW with the same sentence.
- **A picture that stays black**: if the first 2 s of PREVIEW are near-black — Android 12's camera
  toggle gives "a blank camera feed" and no error, and so does a laptop's privacy shutter — the status
  line adds "We can't see anything. Is the camera turned off or covered?". Record stays usable: a dark
  room is not an error.
- **Windows, preview blank on this camera** (#9756, RGB24/UYVY/I420 webcams): "Video messages can't be
  recorded with this camera." — never a recording without a picture to frame it. A `MediaFrameReader`
  preview for these cameras waits until one is at hand to try it on: the owner's machine may not have
  one, so it cannot be verified here.

### S4. Interruptions

Under 1.0 s, "not sent" below means deleted instead — there is nothing worth keeping.

| Event | Voice: holding | Voice: hands-free | Undo window | Review / not sent | Video: PREVIEW | Video: RECORDING | Video: REVIEW | Playing (a voice note or a circle) |
|---|---|---|---|---|---|---|---|---|
| A call rings, starts or is placed (any phase but idle) | stop → not sent | stop → not sent | sent now | kept | closes | stop → REVIEW | kept | pause |
| The app goes to the BACKGROUND — never the inactive state an alert, Control Centre or a permission prompt causes — or the screen locks; a browser tab is hidden (`visibilitychange`) | stop → not sent | stop → not sent | sent now | kept | closes | stop → REVIEW | kept while the app runs; a system that ends the app loses it | pause |
| A desktop window only loses focus, still visible | — | keeps recording | — | kept | closes — but not to a permission prompt the recorder raised | keeps recording | kept | plays on |
| A desktop session locks, the screen saver starts or the computer sleeps | — | stop → not sent | — | kept | closes | stop → REVIEW | kept | pause |
| A desktop window is minimised or hidden (Windows: its close button with Keep running on hides it to the notification area) | — | stop → not sent | — | kept | closes | stop → REVIEW | kept while the app runs | a circle pauses; a voice note plays on |
| A window really closes or the app quits: ⌘W, the close button or ⌘Q on the Mac; Windows' Quit, or its close with Keep running off; a web tab closed or reloaded | — | stop → not sent (the web asks first) | — | kept (the web asks first) | closes | stop → REVIEW, then asks as REVIEW does | asks "Delete video message?" [Delete] [Keep]; Keep cancels the close (the web: the browser's own question) | stops |
| Leaving the chat: the back button, swipe back, another chat, the rail, a notification tap (Android's system Back while recording is Stop, S2.5, not leaving) | stop → not sent | stop → not sent | sent now | a review note becomes not sent, taking its caption | cannot happen: the recorder covers the window; a notification tap or shared item waits until it closes | the same; Back is Stop | Back asks to delete | stops |
| Siri, an alarm or another app takes the microphone | stop → not sent | stop → not sent | — | kept | — (the microphone is not open in PREVIEW) | stop → REVIEW | kept | pause |
| Another app starts playing (an iOS audio-session interruption of playback; Android's audio focus lost) | — | — | — | — | — | — | — | pause |
| Another app takes the camera; the camera goes away (a USB webcam pulled, a Continuity Camera iPhone moved — how each surfaces is UNCONFIRMED); the system is under pressure (`systemPressureState`); iPad multitasking where unsupported | — | — | — | — | the camera sentence (S3.6), or the next camera if there is one | stop → REVIEW with what was recorded | kept | — |
| The system cancels the touch | locks (deletes if cancel was armed) | — | — | — | — | — | — | — |
| An Android configuration change rebuilds the activity — a rotation, a fold or unfold, a window resize, dark mode, a keyboard attached | locks (the recorder lives outside the activity) | keeps recording | — | kept | the preview re-attaches | keeps recording: CameraX is bound to the recorder's own lifecycle, not the activity's; the preview re-attaches and the orientation lock is re-applied | kept | plays on (the now-playing owner lives outside the activity) |
| Headphones or Bluetooth come or go | keeps recording | keeps recording | — | — | — | keeps recording | — | going: pause; coming: plays on |
| The network goes | nothing: recording is local and sends wait in the outbox | | | | | | | what has loaded plays; a circle that stalls shows S5.3's failed line |
| The recorder fails or the disk fills | what is readable → not sent, with "The recording stopped unexpectedly." | | | | | → REVIEW if readable, else the same sentence | | — |
| Sign-out | everything recorded and not sent is deleted | | | | | | | stops |

- **Closing over a finished clip asks.** On the Mac, `windowShouldClose` for ⌘W and the close button,
  and `applicationShouldTerminate` in `MacAppDelegate` (`FamilyConnectApp.swift:36`) for ⌘Q. On
  Windows, `OnClosing` (`MainWindow.xaml.cs:607-616`) sets `args.Cancel` on a real close — Quit, or Keep
  running off — while a clip waits in REVIEW, asks, and closes again on Delete. On the web the existing
  `beforeunload` guard (`main.rs:179-206`) also covers a recording, a not-sent note and a clip in REVIEW;
  the browser words its own question.
- **Where a REVIEW clip lives**: a temporary file kept while the app runs. The recorder is never parked,
  so a system that ends the app in the background loses it — at most one minute of video.
- **A desktop session locking or sleeping** is heard through `NSWorkspace.screensDidSleepNotification`,
  `NSWorkspace.willSleepNotification` and the distributed `com.apple.screenIsLocked` on the Mac; on
  Windows through a session-lock notification, which API a packaged WinUI 3 app gets being UNCONFIRMED
  (Blocked 1); on the web through `visibilitychange` where the browser reports a locked screen as
  hidden (UNCONFIRMED). A hands-free recording never runs on behind a lock screen, and a sleeping laptop
  is a "not sent" row, not "The recording stopped unexpectedly."
- **Playback**: headphones gone is heard through `AVAudioSession.routeChangeNotification` with
  `.oldDeviceUnavailable` on iPhone and iPad (today only the call client observes routes), Android's
  `ACTION_AUDIO_BECOMING_NOISY` while something plays, and a change of the default output device on
  desktops and the web. Android asks for `AUDIOFOCUS_GAIN_TRANSIENT` when it starts playing (only calls
  ask for focus today, `calls/CallAudio.kt:118-147`) and pauses when it loses it. The `audio` background
  mode stays for calls only (`Info.plist:23-31`): playback pauses in the background, and the screen is
  kept awake while something plays (S1.7).
- **A call and the recorder** (iPhone, iPad): the call screen is a full-screen cover over the root
  (`RootView.swift:117-121`), and the recorder is a layer of that root beneath it (S8.1) — never a second
  cover. A call stops a recording into REVIEW and its cover simply rises over the recorder, which is
  still there in REVIEW when the call ends. Elsewhere the recorder lies under the call UI in the same
  way.
- **The Undo window ends by sending** on interruptions: the release had already decided.

### S5. Receiving a round video

#### S5.1 Which messages

One test, the same on every client, mirroring the sticker's (`protocol.md:1580-1584`): **exactly one
attachment, `kind = video`, carrying `round: true`.** Anything else — two attachments, the flag on a
photo, a body (which the server refuses anyway) — is drawn as the ordinary message it otherwise is.

#### S5.2 How it looks

- **No balloon**, like a sticker: the circle alone on the chat background, the sender's name and the
  time where that client puts them for a sticker, a reply quote ABOVE the circle as for a sticker
  reply, reactions where a sticker's go.
- **Diameter**: **200** in a compact width — a compact horizontal size class on Apple (every iPhone in
  portrait, an iPad in Slide Over or a narrow Split View: the rule `Views/AttachmentView.swift:61-89`
  already uses); Android windows under 600 dp; the web under 720 px
  (`web/styles.css:177-186`) — and **240** otherwise: iPad regular, Mac, Android at 600 dp and wider,
  Windows, the web at 720 px and wider. Larger than a sticker (160), smaller than a video tile
  (240 × 320, 320 × 400); 480 pixels fill a 240-pt circle at 2×.
- **The square poster fills the circle.** Until it lands, a neutral disc of the final size, so the row
  never changes height (unlike the Mac's 240 × 180 placeholder, `MacMessageRow.swift:1418`). Only the
  poster is fetched to draw it: `protocol.md:107-108` stays true, and the Mac must not copy its
  poster-then-original pair (`:1382-1383`).
- **On it**: a duration capsule ("0:23") at the bottom centre inside the circle; a 44-unit play disc in
  the middle; an 8-unit accent dot beside the capsule until THIS DEVICE has played it. The dot is the
  device's own knowledge — kept per account, never sent, wiped at sign-out, remembered for the newest
  5 000 videos.

#### S5.3 Playing

- **A tap plays it in place, at the same size, with sound.** The play disc goes; a 3-unit accent ring
  runs round the edge; another tap pauses; at the end it returns to the poster and loses its dot. The
  single tap waits out the double-tap window, which stays the heart reaction (the sticker's precedent,
  `MessageBubbleView.swift:1117-1130`).
- **Loading and failure.** The web and Windows fetch the whole MP4 before it can play
  (`views/attachments.rs:405-548`, `ChatsView.xaml.cs:2983-2986`), and any client can stall. A tapped
  circle shows a loading ring over its poster at once; a second tap while it loads gives up. On Safari,
  which refuses to start playing outside the tap, the circle returns to its play glyph when the bytes
  are in and the next tap plays them — the `AudioPlayer` pattern (`views/attachments.rs:420-428`). A
  failure leaves the poster with "Couldn't load the video. Tap to try again."
- **One thing plays at a time** across the app: a round video or voice note starting pauses any other;
  a recording starting pauses playback; a round video stops when its row scrolls out of view, and
  anything playing stops when the chat closes. A new "now playing" owner on Apple, Android and the
  web — two voice notes can play at once today (`Views/AudioPlayerView.swift`); Windows already has one
  shared player (`ChatsView.xaml.cs:117-127`). What else pauses or stops playback is S4's last column:
  a call, the background, lost headphones, another app's sound.
- **Heard with the silent switch on** (iPhone, iPad): starting a voice note or a round video — a
  staged or not-sent note and the recorder's REVIEW included — sets the audio session to `.playback`
  when no call is active, and releases it with `.notifyOthersOnDeactivation` when playback ends — the
  HIG's "audio/video messaging". The session is never touched while a call holds it. Playback pauses
  when the app goes to the background; the `audio` background mode stays the calls' alone.
- **No autoplay, muted or otherwise.** Each viewer would download every circle in view, about 4.2 MB
  a minute, on the family's own server and on metered phones; "a tile never downloads a VIDEO to draw
  itself" (`protocol.md:107-108`) would have to be amended; looping motion needs a pause control
  (WCAG 2.2.2) and system autoplay settings on six clients; and in a family chat, "a message for me"
  means "tap to hear it".
- **Windows in this version**: a tap opens the existing viewer overlay (`Views/ChatsView.xaml:249-308`),
  where the square clip plays inside a ring painted over its corners on the viewer's solid backdrop.
  Playing inside the thread waits for the clipping trial (Blocked 1).

#### S5.4 Menus, full screen, editing

- A long press (touch) or right-click (desktop) opens the message menu, plus **Open Full Screen**
  (Android "Open full screen"); **no Edit**, as for a sticker.
- While it plays, an expand control — a 28-unit glyph in a 44-unit target at the circle's top trailing
  edge — does the same.
- Full screen is each client's existing viewer, with scrubbing: iOS `AttachmentViewer`, the Mac's
  `AVPlayerView` (never SwiftUI's `VideoPlayer`), Android's viewer, Windows' viewer, the web's viewer.

#### S5.5 Show text (#62)

Under the circle, outside its gestures, exactly as under a video tile and under the same rules: sound
the asking device supplies, the answer never kept or shared, one provider call per member who asks
(`protocol.md:3901-3911`, `:3988-3994`). The sticker branches skip the transcript footer, so the round
branches add it themselves (Android `ChatScreen.kt:3930-3967`, Windows `ChatsView.xaml.cs:1977-1998`);
its text uses the colour that reads on the chat background.

#### S5.6 The sender's own circle

Drawn round at once from the local file's poster, with "Sending…" and a thin neutral ring showing the
upload; a failure is today's terminal failed bubble with Try Again and Delete. Windows needs a round
counterpart to `PendingStickerElement` (`ChatsView.xaml.cs:4745`); iOS's `PendingMediaItemEntity`
carries the flag (`Models/PendingMediaItemEntity.swift:73`).

#### S5.7 Chat list, push, quotes

- **Chat-list row**: "Video message", checked BEFORE "Video" (iOS `ChatSyncCoordinator.swift:3261-3262`;
  Android `data/repo/MessageRepository.kt:1943-1944`; web `views/chat_list.rs:57-63`; Windows its
  preview).
- **The report inbox keeps saying "Video"**, as it says "Photo" for a sticker: a report carries kind
  and name only (`server/src/handlers_report.rs:107-110`, web `model.rs:373-377`), so its line
  (`views/family.rs:665-676`) cannot know the flag.
- **The word for a voice note stays as each client has it** — "Audio" on Apple and Android
  (`ChatSyncCoordinator.swift:3263-3265`, `MessageRepository.kt:1945`), "Voice message" on Windows and
  the web (`ChatList.cs:167-169`, `views/chat_list.rs:63`) — and the push stays "Audio": `kind=audio` is
  also a sound file picked from disk, and the server keeps no name to tell the two apart
  (`protocol.md:4554-4556`, `handlers_attachment.rs:290`). Reconciling those words is its own change.
- **Push**: "Video message", written by the server, so every installed app shows it at once.
- **A reply quoting one**: "Video message", with a small round poster where that client shows a quote
  thumbnail.

#### S5.8 Old apps, old servers, caches

- **A 1.1 or 1.2 app** ignores the flag (`protocol.md:163-166`) and draws an ordinary square video tile
  with the square poster; a tap plays it in its viewer. Offered Edit on its own round video, it gets
  `validation` (400), as with stickers.
- **A new app against an old server** sees no `max_round_video_ms` and offers no video at all; voice
  works on every server.
- **Caches**: iOS and Android write cached attachment lists back with only the fields they know
  (`Core/APIModels.swift:885-905`, `data/net/dto/ApiModels.kt:800-809`), so a circle received before an
  upgrade may draw square until it is read again (UNCONFIRMED).

### S6. Accessibility

| | VoiceOver — iPhone, iPad | VoiceOver — Mac | TalkBack | Narrator | Web screen readers |
|---|---|---|---|---|---|
| **Microphone** | label "Record voice message", hint "Starts recording."; action "Record video message"; input labels "Microphone", "Record", "Voice message" (Voice Control) | label and tooltip; VO-Space records; VO-Shift-M opens the secondary menu | `contentDescription` "Record voice message", `onClick(label = "start recording")`, `customActions` ["Record video message"]; **no long-click action and no tooltip** | `AutomationProperties.Name` "Record voice message", HelpText "Press Shift+F10 for a video message." only when **round available** | `aria-label` "Record voice message"; the menu on Shift+F10 / the Menu key |
| **Dimmed** | the reason as the accessibility value | the same | the reason as `stateDescription` | the reason in HelpText | `aria-disabled="true"`, the reason through `aria-describedby` |
| **While recording** | the slot is "Send voice message" (or "Stop recording"); actions "Stop and listen first", "Delete recording" | the same | the same, as `customActions` | the names change; Delete and Stop are real buttons | the same |
| **Video button** | "Record video message" | the same | the same | the same | the same |
| **Recorder** | a modal region; focus on the slot; the escape gesture as Esc | the same | `paneTitle` "Video message"; traversal kept inside | focus on the slot | `role="dialog"`, `aria-modal`, `aria-label="Video message"`, the rest `inert` |
| **Received circle** | label "Video message, 0:23", value "Not played" while the dot shows; actions Play/Pause (the default), "Open Full Screen", "Show text", then the usual | the same | the same | the same | the same |

- **Magic Tap** (iPhone, iPad), in this order: a voice recording running → Stop (review); the recorder
  open → Record in PREVIEW, Stop in RECORDING, play/pause in REVIEW; something of the app's playing →
  pause. **Nothing else**: Magic Tap never STARTS a voice recording. When the app does not take it the
  system "plays and pauses music playback from the Music app" (Checked facts), and a VoiceOver user
  pausing a podcast must not open the microphone instead — the recording session would even stop the
  podcast, so it would seem to have worked.
- **VoiceOver's escape gesture** (two-finger Z, `accessibilityPerformEscape`) does what Esc does: Stop
  on a voice recording, and in the recorder what S3.4's Esc row says.
- **Announcements** — polite, only state changes, never the ticking clock (which stays
  `aria-live="off"`, `views/attach.rs:337`): "Recording" · "Recording locked" · "Recording deleted" ·
  "Voice message sent" · "Ready to review, %@" · "30 seconds left" · "10 seconds left" · "We can't hear
  anything. Is the microphone muted?" · "Camera ready" · "Recording video" · "Video message sent" ·
  "Camera turned off" · and the "too short" and "stopped at" sentences. iOS
  `AccessibilityNotification.Announcement`; Android a polite live region in the composer; Windows a
  status `TextBlock` with `LiveSetting="Polite"` (`ChatsView.xaml:100`); the web a NEW visually hidden
  `aria-live="polite"` node — not the typing line (`views/conversation.rs:1329`), which is the visible
  typing indicator.
- **The app's own speech stays out of the note.** Where the platform says a screen reader is running —
  Apple `isVoiceOverRunning`, Android touch exploration, Windows' screen-reader flag — the microphone
  opens only once "Recording" has been spoken (Apple's `announcementDidFinishNotification`; a fixed 1 s
  elsewhere), and the timer starts with it. While a recording runs the app's other live regions are
  quiet — the web's typing line becomes `aria-live="off"`, so "… is typing" is never spoken into a
  note. A browser cannot tell that a screen reader runs, so on the web that one word can still leak;
  speech during a recording can leak on a speaker anywhere, which is why only these few are spoken.
- **Keyboard only**: Tab reaches the microphone and the video button; Enter or Space records; Shift+F10
  or the Menu key opens the menu; inside the row Return is the slot and Esc is Stop; inside the
  recorder Return is the slot, Esc closes or stops, and in REVIEW Space plays and pauses wherever focus
  is (S3.4).
- **Motor**: no hold and no drag is ever needed — the hold is a shortcut, H follows the system
  setting, slides have buttons, and a press that wanders still taps (S1.1). Targets per S1.1. Deleting
  10 s or more asks. The preview's timeout only turns the camera off and loses nothing. The Undo window
  IS a time limit: Review Before Sending (S9) is its WCAG 2.2.1 "turn off", and with a screen reader or
  Switch Control running a held release always goes to review (S2.3).
- **Reduced motion**: no pulsing dot; the slot swaps instantly; the recorder appears without fading;
  the ring steps once a second instead of sweeping; the Undo line becomes "Sending in 5", counting down
  once a second, not announced.
- **Colour is never the only signal**: the orange warnings come with "30 seconds left" and "10 seconds
  left" on screen (WCAG 1.4.1).
- **Hearing**: Show text under every round video and voice note; no feedback is sound-only — the app
  plays no sounds at all here.
- **Low vision**: controls scale with the text size (iOS already does, `ConversationView.swift:173-179`);
  circle sizes stay fixed; under Increase Contrast the ring and the dot use the system's high-contrast
  colours.

### S7. Teaching

1. **Tooltips** on desktops and pointers: microphone "Record a voice message (⌥⌘R)" on the Mac,
   "Record a voice message (Ctrl+Shift+R)" on Windows, "Record a voice message" on the web and an iPad
   pointer; video button "Record a video message".
2. **One coach mark**, touch devices only (iPhone, iPad, Android), once per device, after the first
   hands-free voice message sent from a touch screen: **"You can also hold the microphone while you
   talk."** Anchored above the microphone; any tap dismisses it; never shown while a screen reader runs.
   Apple: TipKit (iOS 17); Android: a small `Popup` bubble — not a `TooltipBox`, whose long press would
   fight the hold.
3. **The first held release** on a device goes to review with "Next time, letting go will send it."
4. **A hold released too soon**: "Still recording. Tap Send when you're done."
5. **The preview's first-time line**: "Only you can see this until you start recording." — one string
   for a finger, a click and a key.
6. **"That recording was too short."** / **"That video was too short."**

Nothing else: no tour, no explainer screen, no mode bubble.

### S8. Platform by platform

#### S8.1 iPhone

- **Inputs**: finger — tap, and the hold (S2.3); VoiceOver, Switch Control, Voice Control; a hardware
  keyboard — ⌘↩, ⌥⌘R, Esc.
- **The row**: empty — `[paperclip] [stickers] [@ai, family chat] [field "Message" + video button]
  [microphone]`; typing — `[paperclip] [@ai] [field] [Send]` (the sticker button already steps aside,
  `ConversationView.swift:2925-2929`).
- **The microphone** is a UIKit control in a `UIViewRepresentable`: a `UILongPressGestureRecognizer`
  (minimum duration 0.5 s, `allowableMovement` 20, `allowedTouchTypes` direct and Pencil) for the hold,
  and the control's own `.touchUpInside` for the tap — so a press that wanders past 20 points but lifts
  inside the hit area is still a tap (S1.1), and a hold that begins cancels it. `onLongPressGesture`
  cannot slide; `.contextMenu` must never be attached to it, because on iOS it claims the touch long
  press.
- **The recorder** is a layer at the root (`RootView`), drawn over everything in the window, BENEATH the
  call's `.fullScreenCover` (`RootView.swift:117-121`) — not a second cover, because SwiftUI presents one
  cover at a time from a view, and whether it would dismiss one and present the other in one update
  was never proven. A call's cover simply rises over it (S4). It is safe at the root because a
  conversation is never inside a sheet (`ChatListView.swift:221`, `:304`).
- **Orientation**: held in the current orientation from Record until Stop (S3.5), through the
  supported-orientations mask `AppDelegate` (`FamilyConnectApp.swift:34`) reports for the window.
- Haptics; `.playback` for playback; keep-awake; the coach mark.

#### S8.2 iPad

- The iPhone's code, plus: finger and Pencil hold; trackpad and mouse — a click of any length is a tap,
  a secondary click (`UITapGestureRecognizer` with `buttonMaskRequired = .secondary`) opens the two-item
  menu through a `UIEditMenuInteraction` presented at the click — never a `UIContextMenuInteraction`,
  which would take the touch long press; no haptics.
- **Keyboard**: ⌥⌘R appears in the ⌘ overlay as "Record Voice Message"; ⌘↩ sends; Esc stops.
- The bar stays in the 560-pt column (`ConversationView.swift:1739-1750`); the recorder covers the
  whole window, the sidebar included; in Slide Over and narrow Split View the level meter goes first.
- **Camera**: `isMultitaskingCameraAccessEnabled = true` wherever `isMultitaskingCameraAccessSupported`
  — no entitlement at a 17.0 deployment target — and the S3.6 sentence only where it is not supported;
  `systemPressureState` watched (S4); `RotationCoordinator` for front cameras on the landscape edge (iPad
  10th generation, iPad Pro M4, iPad Air M2/M4, https://support.apple.com/en-us/111840).
- **Turning it during a take**: an iPad cannot be locked, so S3.5's rule for such devices applies — the
  layout never switches while RECORDING, and the capture angle follows the device once Blocked 5
  passes.

#### S8.3 Mac

- **A click records hands-free** — no hold, as `AudioRecorder.swift:11-14` already argues.
  Control-click, right-click or a two-finger click opens the menu.
- **Menu bar**: "Record Voice Message ⌥⌘R" and "Record Video Message" (no shortcut) in the File menu
  after the new-item group, acting on the key window's conversation through a focused value, disabled
  where S1.3 would dim them.
- **The slot gets a tooltip and an accessibility label in every state** (it has neither today,
  `MacConversationView.swift:1743-1752`).
- **Return**: empty and idle — nothing; recording — the slot only. Stop loses `.defaultAction` and the
  field is gone, so the three bindings that meet today (`:1473`, `:1729`, `:1751`) cannot. Esc stops a
  recording and otherwise keeps cancelling an edit or a reply (`:1735-1741`).
- **The recorder** is a layer over the whole window, drawn from the window's root so the sidebar is
  covered — not a sheet. "Choose camera" when there are several.
- **The toolbar cannot be covered**, so while the recorder is open every toolbar item in that window is
  disabled — Family (whose sheet would change `selectedChatID`, `MacChatView.swift:168`), Settings, the
  board, Return to Call, Call, Video Call, Open polls — through a `recorderOpen` focused value, as are
  Refresh and the menu commands that act on that conversation; no sheet opens over it. Only then is
  "the chat cannot change under it" true on the Mac.
- **Closing asks**: ⌘W or the close button over a clip in REVIEW asks "Delete video message?" through
  `windowShouldClose`; ⌘Q through `applicationShouldTerminate` in `MacAppDelegate`
  (`FamilyConnectApp.swift:36`); Keep cancels the close or the quit. A RECORDING stops into REVIEW first.
- **Lock and sleep**: `NSWorkspace.screensDidSleepNotification`, `NSWorkspace.willSleepNotification` and
  the distributed `com.apple.screenIsLocked` stop a recording (S4).
- **Calls** open their own window and the composer stays usable beside them, so the dimmed "during a
  call" state matters most here. One recording at a time across windows.
- **Playback**: `AVPlayerLayer` in an `NSViewRepresentable`; the poster-only fetch fixed first.

#### S8.4 Android phone

- **Inputs**: finger and stylus — tap and the hold; a mouse — a click on release, the secondary button
  opens a `DropdownMenu`; a keyboard — Enter or Space on the focused microphone, Ctrl+Shift+R.
- **Back**: while recording, Stop (review); in PREVIEW, Close; in REVIEW, it asks; otherwise it leaves
  the chat as usual.
- **The microphone** keeps the 44-dp visual in a 48-dp target, with the semantics of S6 and no
  `TooltipBox`.
- **The recorder** is a layer at the top of the app's content, above the list and detail panes — not a
  `Dialog`. From Record until Stop the screen is held in its CURRENT orientation
  (`SCREEN_ORIENTATION_LOCKED`, never forced portrait), and `VideoCapture.targetRotation` is set from an
  `OrientationEventListener` when Record is tapped. CameraX is bound to a lifecycle the recorder owns,
  not the activity's, because `MainActivity` declares no `configChanges` (`AndroidManifest.xml:86-90`)
  and a fold, a resize or a dark-mode change rebuilds it mid-take; a rebuilt activity re-attaches
  `Preview.setSurfaceProvider` and re-applies the lock.
- **After Stop** the file must be 480 × 480 H.264 + AAC with rotation 0 — read from
  `METADATA_KEY_VIDEO_ROTATION`, because `readVideoMetadata` folds a rotation into the size and a
  square hides it (`MediaPrep.kt:519-526`); anything else takes the Media3 pass, which bakes the rotation
  into the pixels (`MediaTranscode.kt:93-96`).
- Haptics; `FLAG_KEEP_SCREEN_ON`; the coach mark.

#### S8.5 Android tablet and foldable

- The composer stays in the 560-dp column (`ui/components/WindowClass.kt:69`); the recorder covers both
  panes; circles are 240 from 600 dp.
- A mouse or trackpad clicks; a right-click opens the menu; a keyboard works as on the phone.
- Picking another chat in the list pane parks a voice recording (S2.8); the recorder blocks the list
  while it is open.
- **The orientation lock is ignored here**: the app targets Android 16, which ignores
  `setRequestedOrientation()` at a smallest width of 600 dp or more, and a foldable changes display.
  S3.5's rule for such devices applies — the layout chosen at Record is kept, CameraX keeps recording
  across the rebuilt activity (S8.4), and the capture angle follows the device only once Blocked 5
  passes.

#### S8.6 Windows

- **Every input clicks.** Mouse, touch and pen: a press of any length is a click that records
  hands-free. The microphone sets `IsHoldingEnabled="False"`; a mouse right-click (`RightTapped` with
  `PointerDeviceType` Mouse), a pen tap made with the barrel button down (`IsBarrelButtonPressed`,
  recorded at `PointerPressed`) and Shift+F10 or the Menu key (`ContextRequested` with no position) open
  a `MenuFlyout`. Any other `RightTapped` — a touch or a pen press-and-hold, if Windows raises one
  anyway — is ignored, and a long press simply clicks; trial T4 (Blocked 1) checks both.
- **Keyboard**: Ctrl+Shift+R — the app's first `KeyboardAccelerator`; Enter is the slot; Esc is Stop.
- **The slot** becomes a fixed 40 × 40-epx accent icon button with a 44-epx hit area (Send E724,
  microphone E720, Stop E71A, Save E73E; Record E7C8 in the recorder), each state with a tooltip and an
  `AutomationProperties.Name`, so the Auto-width column at `ChatsView.xaml:177` never jumps.
- The recording row moves from above the card (`:109-120`) into the input row. No level meter and no
  silence warning in this version.
- **The recorder** covers the page and the rail. Recording round video arrives after the trials
  (Blocked 1); playback goes through the viewer (S5.3). If T1 finds that `MediaCapture` initialised for
  `AudioAndVideo` holds the microphone in PREVIEW, the status line says "Camera and microphone on · Not
  recording" (S3.4). A real close — Quit, or the close button with Keep running off — over a clip in
  REVIEW is cancelled in `OnClosing` (`MainWindow.xaml.cs:607-616`, `args.Cancel`) to ask first; a
  session lock stops a recording (S4), through an API trial T7 settles.

#### S8.7 Web on a desktop

- **A click records**; Enter or Space on the focused microphone; a right-click or the Menu key opens a
  small `role="menu"` menu. No shortcut. **The kind of input decides, not the layout**: on every layout
  — a touch laptop or a Chromebook shows the desktop one — a `contextmenu` that follows a touch
  `pointerdown` is prevented, so a touch hold never opens the menu (S1.6), and a touch acts as S8.8
  says.
- **Esc is Stop**, no longer Cancel (`views/attach.rs:324-331`).
- **The Send/Save text button becomes a fixed icon button** that keeps its word as visually hidden text
  — screen readers and the existing tests still read "Send" — with `aria-label` per state. Never
  `disabled` while recording (S1.7).
- **The recorder** is a `position: fixed; inset: 0` layer, `role="dialog"`, `aria-modal="true"`,
  `aria-label="Video message"`, the rest of the app `inert`; REVIEW's Space is a capture-phase `keydown`
  on it (S3.4). The `beforeunload` guard (`main.rs:179-206`) also asks while a recording runs, a
  not-sent note waits or a clip is in REVIEW.
- **Video** is recorded where `VideoEncoder` (H.264 480 × 480) and `AudioEncoder` (AAC) both pass
  `isConfigSupported` and `requestVideoFrameCallback` exists — Chrome and Edge on Windows and macOS —
  **and the browser is not WebKit until the Safari trial passes** (Blocked 2): the one documented
  exception to "probe, never sniff" (`web/src/webcodecs.rs:11-17`), removed with the trial. Firefox and
  desktop Linux record voice through the recorder's existing fallbacks (`web/src/recorder.rs:424-453`,
  down to WAV); their paperclip item explains why there is no video.

#### S8.8 Web on a phone or tablet

- **Tap only** in this version — the hold waits for a device trial: a touch `pointerdown` is not user
  activation, and Safari's AudioContext needs one.
- **A touch acts on its `pointerup` inside the button**, which IS user activation, and the `click` that
  may follow is swallowed — so a press of any length works whether or not a mobile browser sends a
  `click` after a long one. A mouse, a pen and the keyboard keep `click`.
- On the two buttons only — never the page — `user-select: none` and `-webkit-touch-callout: none`,
  and a touch-originated `contextmenu` is prevented (on every layout, S8.7).
- The recorder pads `env(safe-area-inset-*)` (none is used today, `web/index.html:7`).
- **The Record tap is the user activation** that resumes the AudioContext (`Listening::in_the_click()`)
  and starts the encoders.
- iOS browsers record voice; video waits for the trial. Chrome on Android records video when the probe
  passes. The wake lock is held while recording and while something plays.
- A phone's browser can drop a hidden tab, or be closed from the app switcher, without any
  `beforeunload` — a not-sent note or a clip in REVIEW is then lost without asking (S2.8).

### S9. Settings

On iPhone, iPad and Android only — where a release can send:

- A new section **"Voice Messages"** (Android "Voice messages") with one toggle, **"Review Before
  Sending"** (Android "Review before sending"), off by default, per device, never on the wire; footer:
  "When you hold the microphone to talk, letting go keeps the message for you to check instead of
  sending it."
- iOS: `Views/SettingsView.swift` gains the section after Privacy (sections at `:80-86`), stored in
  `Storage/AppSettings.swift`. Android: `ui/settings/SettingsScreen.kt`, after link and map previews
  (`:468-485`), stored in `data/settings/SettingsRepository.kt`.

### S10. Strings

New, in the apps' catalogue first (`ios/FamilyConnect/Localizable.xcstrings`, the source of truth
for the nine languages, checked by `ios/scripts/check-strings.py`), then Android's `strings.xml` in
every locale (its strings test), Windows' `CatalogueTests` and `win/i18n/win.json`, and
`web/i18n/web.json`.

| Group | Strings |
|---|---|
| Menus and buttons | "Record Voice Message" (replaces "Record Audio"; Android "Record voice message") · "Record Video Message" (Android "Record video message") · Android "Take video" (replaces "Record video") · "Record" · "Retake" · "Undo" · "Keep" · "Open Settings" · "Open System Settings" (Mac) · "Open Full Screen" (Android "Open full screen") · "Choose camera" |
| Accessibility | "Record voice message" · "Record video message" · "Send voice message" · "Send video message" · "Stop recording" · "Delete recording" · "Stop and listen first" · "Record a voice message instead" · "Starts recording." (iOS hint) · "start recording" (Android click label) · "Press Shift+F10 for a video message." (Windows) · "Video message, %@" · "Not played" · "Microphone" and "Voice message" (Apple Voice Control input labels, beside "Record") |
| On screen | "Video message" · "Video message · %@" · "Voice message · %@" · "Voice message not sent · %@" · "Sending voice message · %@" · "Sending in %lld" (Reduce Motion) · "Slide to cancel" · "Release to cancel" · "Not recording" · "Camera and microphone on · Not recording" (only where the microphone must stay open) · "Only you can see this until you start recording." · "Starting camera…" · "We can't see anything. Is the camera turned off or covered?" · "Still recording. Tap Send when you're done." · "Next time, letting go will send it." · "You can also hold the microphone while you talk." · "You can play this after recording." · "Couldn't load the video. Tap to try again." · "Record a voice message (%@)" · "Record a voice message" · "Record a video message" · "30 seconds left" and "10 seconds left", shown as well as announced |
| Settings | "Voice Messages" (Android "Voice messages") · "Review Before Sending" (Android "Review before sending") · "When you hold the microphone to talk, letting go keeps the message for you to check instead of sending it." |
| Notices | "You can record a message after the call." · "Send or delete the voice message that wasn't sent first." · "You can record now." · "We can't hear anything. Is the microphone muted?" · "We didn't hear anything." · "Recording stopped at five minutes." · "Recording stopped at one minute." · "That video was too short." · "The recording stopped unexpectedly." · "Delete this recording?" · "Delete video message?" · "Video messages need the camera and the microphone." · "Family needs permission to use your camera. Turn it on in Settings." · "Family needs permission to use your camera. Turn it on in System Settings › Privacy & Security › Camera." · "Family needs permission to use your camera. Allow it in your browser's settings for this site." · "Family needs permission to use your camera and microphone. Allow them in your browser's settings for this site." (web, when the browser cannot say which) · "The camera is being used by another app." · "The camera isn't available while other apps are on screen." · "This browser can't record video messages. Voice messages work." · "Couldn't make it round. It will be sent as a regular video." · "Too big for a video message. It will be sent as a regular video." · "Video messages can't be recorded with this camera." (Windows) |
| Announcements | "Recording" · "Recording locked" · "Recording deleted" · "Voice message sent" · "Ready to review, %@" · "30 seconds left" · "10 seconds left" · "Camera ready" · "Recording video" · "Video message sent" · "Camera turned off" |

"Not sent" is not announced anywhere — it is the name of S2.8's state, and Undo says "Ready to review,
%@". No "Undo available" announcement exists: a screen reader never meets the Undo window (S2.6).

**Reused**: "Send", "Save", "Stop", "Delete", "Close", "Switch camera", "Camera", "Show text", "Play",
"Pause", "Sending…", "Try Again", "That recording was too short.", "Wait until the current attachment
is done.", "Couldn't start recording.", the microphone denial sentences. "Keep" and "Voice message"
already exist in the web and Windows catalogues.

**Renamed**: the web's staged label "Voice note · %@" (`views/attach.rs:222`) becomes "Voice message ·
%@", and its recording row's group label "Recording a voice note" (`views/attach.rs:335`) becomes
"Recording a voice message"; Windows' "Voice message · <size>" (`ComposerStaging.cs:200-207`) shows the
duration instead of the size.

**Kept on purpose**: the chat-list words for `kind=audio` ("Audio" on Apple and Android, Android's
"%d voice notes", "Voice message" on Windows and the web), the push word "Audio", and the transcript and
consent sentences that name "a voice note, an audio file or a video" (e.g. Android `strings.xml:682`,
`:684`, and their Apple keys). They name the KIND, which is also a sound file picked from disk (S5.7);
rewording them is a separate change, not this one.

## On the wire

**Amend `docs/protocol.md` first**, then write code.

### Voice: no wire change

Only the apps' rule at `protocol.md:112-113` changes: a voice note is "five minutes at most, **sent from
the recorder, or staged when the member stops it to listen or add words** — never sent by an
interruption, except that an interruption during the five-second Undo window after a release ends the
window early and sends it". A note sent from the recorder — by Send or after the Undo window — is still
an outbox row written before its first byte, its bytes kept until the ack (`:5858-5868`, one sentence
added).

### Round video: one flag, the sticker's pattern

```
POST /attachments?kind=video&width=480&height=480&duration_ms=23400      (Content-Type: video/mp4)
  → 201 {attachment: {id: 91, …}}
PUT  /attachments/91/preview                                              (the square JPEG poster)
POST /chats/42/messages {client_msg_id, body: "", attachment_ids: [91], round: true, reply_to_message_id?: 41}
  → 201 {message: {…, attachments: [{id: 91, kind: "video", mime: "video/mp4", size: 1649700,
                                     width: 480, height: 480, duration_ms: 23400,
                                     has_preview: true, round: true}]}}
```

```json
{"type": "send", "chat_id": 42, "client_msg_id": "4f9e21c0-…", "body": "",
                 "attachment_ids": [91], "round": true}
```

- **Name: `round`.** It says how the attachment is drawn, exactly as `sticker` does. Not
  `video_note`: in this protocol "note" is a board note (`:1338-1345`), and the sticker section already
  had to untangle one such word. The apps say "Video message".
- **Absent means false**, on the send (`Option<bool>`, `unwrap_or(false)`, as `handlers_chat.rs:1835`)
  and on the echo (`skip_serializing_if`, as `server/src/models.rs:913-921`). `"round": true` appears on
  the Attachment on EVERY read: history pages, threads, the `message` frame, the ack, `last_message`, the
  edits feed, and what the push is built from.
- **What it means**: the message was sent as a VIDEO MESSAGE — a square H.264/AAC MP4 recorded to be
  drawn round. It is still `kind = video` in every other respect. Set by the send; never changed.
- **The drawing test** is S5.1, written into the protocol as the sticker's is, with the 200/240
  diameter as a RECOMMENDATION nothing on the wire carries.

### What the server checks

In `create_message`, the one function REST and WebSocket share, in the sticker's places and order:

1. `round` with a poll → `invalid_poll`.
2. `round` with `sticker` → `validation` ("a message is a sticker or a video message, not both").
3. `round` with not exactly one attachment → `invalid_attachment`.
4. `round` with a non-empty trimmed body → `validation` — a circle has no balloon to hold words.
5. **After the claim** — whose `RETURNING` already gives kind, mime, size, width, height and
   duration (`handlers_chat.rs:1198-1199`): `invalid_attachment`, the transaction dropped so the upload
   stays unclaimed and unflagged, when the REQUEST asked for `round` (as the sticker check tests the
   request's `sticker`) and `kind` is not `video`; `mime` is not `video/mp4` (QuickTime is never within
   the profile); `width` or `height` is missing, below 1, above 720, or they differ; `duration_ms` is
   missing, below 1, or above `max_round_video_ms`; `size` is above `max_round_video_bytes`. The claim
   itself writes `round = ($6 AND kind = 'video')`, so it can never trip the database's kind check —
   a `CHECK` that fired inside the claim would be a 500, which no outbox treats as terminal
   (`docs/protocol.md:5847-5852`): the send would be retried forever instead of being refused.
6. **Replies, threads and every chat.** A round video may be a reply, may be posted into a thread, and
   is accepted in every chat — the assistant's included, under the same consent question as a sticker
   (`protocol.md:1571-1578`). The server has no reason to refuse any of them; the clients offer
   recording only in a family or direct chat's main composer in this version.
7. **Edits are refused**: `PATCH` on a message whose attachment carries `round` is `validation` (400),
   changes nothing, takes no `edit_seq`, fans out nothing — asked LAST, after the body's rules,
   `message_not_found` and `not_message_author`, like the sticker (`handlers_chat.rs:1056-1064`). The
   call-record check (`:1011-1021`) is not the model: it answers before `message_not_found` and is not
   scoped to the chat.

The server still decodes nothing (`protocol.md:4459-4462`): square-ness and length are the sender's
declaration, the sticker's split between server rules and client rules and the Bot API's trust model.

### Discovery and limits

`GET /families/mine` gains two keys, **always present on a server that has round video** — their
absence is how a client knows to offer no video entry at all (the `max_pack_items` precedent,
`:1612-1615`):

| Key | Default | Config |
|---|---|---|
| `max_round_video_ms` | 60 000 | fixed (clients stop at it minus 500 ms) |
| `max_round_video_bytes` | 12 582 912 (12 MiB), or `max_attachment_bytes` when that is lower | `limits.max_round_video_bytes` — when SET, 1 ≤ value ≤ `max_attachment_bytes`; when not set, the default is clamped, never refused |

The default clamps rather than refuses because the sticker's bound, which refuses, made every server
with a `max_attachment_bytes` below 512 KiB fail to start (`CHANGELOG:122-124`); an operator who never
wrote the key believes nothing about it, while one who wrote a value above the attachment ceiling
believes something untrue and is told at startup (`config.rs:2281-2294`'s reasoning).

The limits table (`:6113-6153`) gains the two rows; "a square of at most 720 on a side" stays a declared
check, not a measured one.

### What does not change — said so in the new section

- **Push**: "Video message" (a new arm beside "Sticker", `push_payload.rs:120-125`); "Audio" for a
  voice note is unchanged.
- **The assistant** still sees `[video]` — its SELECTs and placeholder are untouched
  (`server/src/handlers_ai.rs:1060-1077`).
- **Transcripts**: a round video is a video — the supplied form, never kept, one call per asker.
- **Reports** carry kind and name only; **statistics** count it as one `video`; **retention** sweeps it
  like any message; **blocking** hides it like any message.

### Old apps and old servers

A 1.1 or 1.2 app ignores the flag and draws a square video, plays it, and offers Edit on its own round
video to be told `validation`; it shows the server's new push word at once. A server that predates the
feature ignores `round` on a send (no `deny_unknown_fields`) — which is exactly why a client must not
send it without the discovery keys.

### Sections of protocol.md to amend

| Section | Lines | Change |
|---|---|---|
| A browser is a client too — voice notes | 109-114 | the voice-note rule above |
| Attachment object | 413-429 | `"round": true` beside `"sticker": true` |
| Editing | 662-683 | a video message cannot be edited |
| **Video messages** (new) | after "Sticker pack", before "Starting a family" (1656) | what one is; sending one; the checks; the drawing test with the diameters; old clients and old servers; limits; what does not change |
| Preparing media before upload | 4482; 4497-4506 | at 4482, after "the server accepts every listed type at any resolution", a cross-reference to the video message's check at claim time; a "Video message" row: recorded to the profile, never re-planned |
| REST endpoints | 5434, 5457, 5493, 5494 | discovery keys; the upload; `round?` on the send; the edit refusal |
| WebSocket client frames | 5537-5548 | the round `send` example |
| Outbox | 5858-5868 | the one sentence on sends from the recorder |
| Push wording | 5998-6006 | "Video message" |
| Limits | 6113-6153 | two rows |
| Assistant placeholders; transcripts | 2318-2327; 4066-4073 | unchanged — say so |

## The server

The sticker's checklist, step for step:

- **Migration `0052_round_video.sql`**:
  ```sql
  ALTER TABLE attachments ADD COLUMN round BOOLEAN NOT NULL DEFAULT false;
  ALTER TABLE attachments ADD CONSTRAINT attachments_round_is_video CHECK (NOT round OR kind = 'video');
  ALTER TABLE attachments ADD CONSTRAINT attachments_round_not_sticker CHECK (NOT (round AND sticker));
  ```
  appended to `MIGRATIONS` (`server/src/migrate.rs`), checked by `server/tests/migration_flow.rs:155-182`,
  with a constraint test for each CHECK. Both are the database's own guarantee, and no request can
  trip either: check 2 refuses `round` with `sticker` before any id is read, and the claim writes
  `round = ($6 AND kind = 'video')` (check 5) — 0048 put no kind `CHECK` on `sticker`, and a `CHECK`
  firing inside the claim would turn a wrong-kind send into a 500 instead of `invalid_attachment`.
- **`models.rs`**: `round: bool` with `#[serde(skip_serializing_if = "std::ops::Not::not", default)]`
  beside `sticker` (`:913-921`); `from_row` reads it with `try_get` and a default (`:1032-1035`);
  constants `ROUND_VIDEO_MAX_MS = 60_000` and `ROUND_VIDEO_MAX_SIDE = 720`; every full `Attachment`
  literal gains the field (`handlers_chat.rs:1549-1566`, `models.rs:1920-1934`, `handlers_ai.rs:4049-4063`,
  `push_payload.rs:891-905`, `:996-1010`, `:1224-1238`, `ws.rs:1008-1022`, `:1868-1882`).
- **`handlers_chat.rs`**: `round: Option<bool>` on `PostMessageRequest` (`:47-76`); the parameter on
  `create_message` (`:550-577`) and both callers (`:1825-1837`, `ws.rs:629-641`); checks 1–4 beside
  `:626-650`; check 5 beside `:869-882`, testing the REQUEST's `round`; the claim
  `UPDATE … round = ($6 AND kind = 'video') … RETURNING … round` (`:1186-1207`); `attach_attachments`
  (`:329-334`) and the `last_message` SELECT and literal (`:1536-1566`) read it; the edit refusal beside
  `:1056-1064`.
- **`ws.rs`**: the `send` field (`:46-74`), its destructure (`:606-617`), a parse test (`:1832-1856`);
  the five other `ClientFrame::Send` test literals gain `round: None` beside their `sticker: None`
  (`:920`, `:944`, `:970`, `:995`, `:1101`).
- **`push_payload.rs`**: `"video" if attachment.round => "Video message"` and its test.
- **`config.rs`** and `config.example.toml`: `limits.max_round_video_bytes` as an `Option`, with one
  accessor for the effective ceiling — the value when set, otherwise min(12 582 912,
  `max_attachment_bytes`); `validate` bounds only a value that was set, `1 ≤ value ≤ max_attachment_bytes`,
  beside the pack's (`config.rs:2285-2294`). So the test servers' 64 KiB `max_attachment_bytes`
  (`server/tests/common/mod.rs:185`) and the unit test that expects 65 536 to validate (`config.rs:3905-3909`)
  stay green unchanged, and no upgraded server refuses to start. Unit tests: unset → the clamped
  default; set above the attachment ceiling → refused; equal → accepted. **`handlers_family.rs`**: the two
  keys beside `:895-901`, the byte key reporting the effective ceiling.
- **Tests** — a new `server/tests/round_flow.rs` modelled on `pack_flow.rs`: the flag on every read path
  and ABSENT (not false) on an ordinary video (`:1068-1190`); the WebSocket send (`:1193-1239`); every
  refusal leaves the upload unclaimed (`:1242-1349`) — including `round: true` on a photo, an audio, a
  file and a location, each 400 `invalid_attachment` with nothing claimed or flagged (the sticker's
  wrong-kind test, `:1284-1307`); the push word (`:1425`); retention (`:1467`); the edit refusal asked
  last (`:1807`); the keys present and the byte limit configurable — the over-the-ceiling test sets
  `max_round_video_bytes = 8 * 1024` through `spawn_server_with_config` (`server/tests/common/mod.rs:142`)
  and uploads between that and 64 KiB;
  unit tests in `models.rs`, `ws.rs` and `push_payload.rs`.
- **CHANGELOG**: SERVER and OLD CLIENTS paragraphs in the pattern of `CHANGELOG:115-142`, saying that
  the new key defaults to 12 MiB or the attachment ceiling, whichever is lower, and that no existing
  configuration is refused. The version is the owner's to set.

## The recording profile for a round video

| | Video message |
|---|---|
| Container | MP4 (`video/mp4`), `moov` before `mdat` |
| Picture | square **480 × 480**, upright pixels (identity transform), **not mirrored**, 8-bit SDR |
| Frame rate | 30 fps at most; a camera delivering fewer is kept as it is |
| Video | H.264 High — Main where an encoder offers nothing else; the web `avc1.64001e` (level 3.0) |
| Video bitrate | **500 000 bit/s** = 2 000 000 × (230 400 ÷ 921 600) × (30 ÷ 30) — step 3 of the profile (`protocol.md:4521`) at the 30 fps target, whatever rate the camera delivers |
| Keyframes | at most every 2 s (the web's `KEYFRAME_SECONDS = 2`) |
| Audio | AAC-LC, mono, 44.1 or 48 kHz, **64 000 bit/s** — the voice note's row |
| Length | 1.0 s to `max_round_video_ms` − 500 ms = 59.5 s |
| Size | ≈ 70 KB/s; ≈ 4.2 MB for a full clip |
| Poster | a square JPEG from the clip at 0.5 s, else 0 s, else 2 s, at most 600 px — so 480 × 480 — sent with `PUT /attachments/{id}/preview` |
| Upload | `POST /attachments?kind=video&width=480&height=480&duration_ms=…` |

- **Recorded to the profile and uploaded as recorded — never re-planned.** At 24 fps and above it also
  falls inside Rule A; below that it does not — step 3 scales the target by frame rate ÷ 30, so a front
  camera dropping to 20 fps in low light has a target of 333 000 and a Rule A ceiling of 416 250 — and
  even inside it, an encoder overshooting by more than 25 % would trigger a pointless transcode. Both are
  reasons it never meets the planner; voice notes bypass it for the same reason.
- **`moov` first on every client, explicitly**: Apple `AVAssetWriter.shouldOptimizeForNetworkUse`;
  Android `Mp4Faststart`; Windows `Faststart.MoovFirst`, because its keep path skips it today
  (`MediaPreparing.cs:221-225`); the web's `mp4.rs` writes it that way already.
- **Why 480**: the circle is drawn at 200–240 units, 400–720 device pixels; Telegram records 384 and its
  web client 400; 640 × 640 would be 889 000 bit/s and ≈ 7.15 MB a minute — 70 % more data, on the
  family's own server and the viewers' phones, for a circle nobody can tell apart; 480 is the centre crop
  of the 640 × 480 mode front cameras offer, so nothing is upscaled; it is H.264 level 3.0 and a multiple
  of 16.
- **Why 12 MiB**: the largest clip a later client recording at the profile's 720 maximum could keep under
  Rule A for 61 s is (1 125 000 × 1.25 + 64 000) × 61 ÷ 8 ≈ 11.2 MB, so raising the size later needs no
  server change.
- **If CameraX writes stereo or another audio rate** (stable CameraX cannot set it), the clip is still
  sent as recorded; the server checks only kind, mime, size and declared metadata.

## Where it plugs in

### Shared rules — `web/text` (`fc_text`)

- A new `record` module, the reference implementation:
  - `composer_slot(inputs) → Slot` — S1.3, every row;
  - `video_door(inputs) → Hidden | Dimmed | Shown` — S1.4, whose inputs include whether THIS build
    records round video on this platform (S1.2);
  - `hold_step(state, event, constants) → (state, effects)` — S2.3 as a reducer over down, move, up,
    system-cancel, timer, cap, interruption, with effects such as start, lock, arm, disarm, delete,
    review, undo, haptic, hint. (*Revised 2026-10-06:* with the hold gone it keeps its name and reduces
    activate, record (the paperclip, menu and shortcut), stop, delete and its answer, the permission
    answer, cap, interruption, the person's other actions and the slot's own emptying, with the effects
    start, send, review, park, delete, ask-delete, ask-permission, denied, explain, hint, announce and
    haptic; `hold_threshold_ms` is gone from the module and from the vectors);
  - `round_cap_ms(max)`, `round_warning_ms(max)`, `round_diameter(width_class)`, `is_round(message)`.
- Vectors printed by the oracle tool (`win/tools/board-oracle`) and checked by iOS and Android (every
  rule, the hold included) and Windows (the slot, the door and the round helpers — it has no hold) in
  CI, as media-plan vectors are (`ci.yml:740-767`). Leave `display_name`'s signature alone, or every
  port's fixtures move. The hookup, spelled out because today's check names its files one by one:
  - the fixture is `record-vectors.json`, printed by a new oracle subcommand `cargo run -- record`
    beside `media-plan` (`win/tools/board-oracle/src/main.rs:26`);
  - its three copies sit beside the media-plan ones: `win/tests/FamilyConnect.Core.Tests/Fixtures/`,
    `ios/FamilyConnectTests/Fixtures/` and `android/app/src/test/resources/`;
  - the path filter (`ci.yml:123`) gains those three copies, and the comparison loop (`:761-763`) gains
    them, so an edit to a copy alone still runs the check;
  - the `win` job (`ci.yml:633-650`), which already runs whenever `web/text` changes, gains a
    `cargo test` step for `web/text` on its Ubuntu leg — today nothing in CI runs the tests of the
    crate every port is held to (the only `cargo test` is the server's, `:239`).

### iPhone, iPad and Mac (`ios/FamilyConnect/`)

- **Phase 0**
  - `Core/AudioRecorder.swift`: never deactivate the session while a call is active (`:160`, `:188`); report
    the cap (`audioRecorderDidFinishRecording`, or the ticker noticing `!isRecording`, `:198-201`); an
    `AVAudioSession.interruptionNotification` observer that stops and keeps; metering.
  - `Views/ConversationView.swift`: stop and park in `.onDisappear` (`:1039-1049`) and on `.background`
    (`:1056-1071`); refuse while `!calls.isIdle` and stop and park when it turns; disable the toolbar call
    buttons while recording (`:955-980`); delete the leftover `fc-voice-*.m4a` once staged (`:2001-2015`).
    The Mac's `MacConversationView` the same (`:658-668`), its call window included, and lock and sleep
    (S8.3).
  - The not-sent row (S2.8) and `Core/ParkedRecordings.swift` — per account, in Application Support, the
    file with its length, reply id and caption, swept at launch, cleared at sign-out — so that what
    Phase 0 parks can be seen, sent and deleted. In Phase 0 the row offers Send and ✕; its ▶ arrives with
    Phase 1's local playback. "Record Audio" is disabled while a not-sent note waits.
- **Phase 1**
  - `Models/ComposerSlot.swift` and `Models/RecordGesture.swift` — the shared rules as Swift types, beside
    `StickerDoor` (`Core/StickerPack.swift:430-470`), checked against the vectors.
  - `Views/RecordSendButton.swift` replaces Send (`ConversationView.swift:1720-1734`); ⌘↩ stays on the
    Send state; a hidden ⌥⌘R button beside `pasteShortcut` (`:1787-1802`).
  - The hold row, the recording row and the Undo row in place of the field (`:1700-1719`), the field kept
    focused under the hold row; the strip above the row (`:1503-1505`, `:1977-1996`) goes.
  - `Views/StagedAttachment.swift` (`:105-121`) plays a staged note, and the not-sent row gains its ▶;
    the "sending" entry written at a release (S2.6).
  - The video button inside `.composerFieldBackground()`; the paperclip renamed and extended, both items
    gone from the assistant chat (`:1618-1622`); `SettingsView` (S9); TipKit; `.sensoryFeedback`;
    keep-awake.
  - Mac: the slot (`MacConversationView.swift:1743-1752`) with `.help` and labels; `.contextMenu` on it is
    fine on the Mac, which has no touch hold; the Return bindings (`:1473`, `:1729`, `:1751`); the File-menu
    commands beside `FamilyConnectApp.swift:485-490`.
- **Phase 2**
  - `Core/APIModels.swift:785-905`: `isRound` (coding key `round`, so nothing shadows Swift's `round(_:)`),
    decoder, encoder, `withPreviewFlag`.
  - `Models/Snapshots.swift`: `isRoundVideo` beside `isSticker` (`:735-739`); `offersEdit` false (`:754-756`).
  - `Views/MessageBubbleView.swift`: a branch after the sticker's (`:1050`) drawing a new `RoundVideoTile`
    modelled on `stickerTile` (`:1117-1130`), bare (`:1189-1192`), poster only through `AttachmentView.image`
    (`AttachmentView.swift:130-144`), `TranscriptSection` under it.
  - `Views/RoundVideoPlayer.swift`: an `AVPlayerLayer` (`.resizeAspectFill`) clipped to a circle, fed by
    `AttachmentStreamPlayer`, with the loading ring and the failed line (S5.3); `Core/NowPlaying.swift`,
    shared with `AudioPlayerView`, which pauses for a call, the background, an interruption and
    `routeChangeNotification` with `.oldDeviceUnavailable` (the Mac: a change of the default output) and
    keeps the screen awake while it plays (S4, S1.7); `.playback` (S5.3).
  - The tap handlers (`ConversationView.swift:1189-1202`, `MacConversationView.swift:920-940`,
    `ThreadView.swift:324-340`); "Video message" in `ChatSyncCoordinator.preview` (`:3261-3262`).
  - Mac: a branch in `MacMessageRow.attachmentStack` (`:773`); the poster-only fetch (`:1382-1383`,
    `:1549-1550`); `AVPlayerLayer` in an `NSViewRepresentable`.
- **Phase 3**
  - `Core/VideoMessageRecorder.swift`, shared by both platforms: `AVCaptureSession` (front wide-angle camera,
    `systemPreferredCamera` on the Mac) with video and audio data outputs into `AVAssetWriter(.mp4)` at
    480 × 480 `ResizeAspectFill`, H.264 500 kbit/s, AAC mono 64 kbit/s, `shouldOptimizeForNetworkUse`;
    `startRunning()` off the main thread; the audio input added only when Record is tapped (S3.4);
    mirroring off on the data connection (and `automaticallyAdjustsVideoMirroring` off first);
    `RotationCoordinator`, the angle fixed at Record (following the device on an iPad once Blocked 5
    passes); interruption reasons and `systemPressureState`; `isMultitaskingCameraAccessEnabled` when
    `isMultitaskingCameraAccessSupported`, with no entitlement; `NSCameraReactionEffectGesturesEnabledDefault`
    false in both `Info.plist` and `Info-macOS.plist`. **Never `AVCaptureMovieFileOutput`.**
  - `Views/VideoMessageRecorderView.swift`, a layer of the root beneath the call cover (S8.1) and over the
    Mac window's root (S8.3), with the reply banner, the near-black check (S3.6) and REVIEW's Space; the
    poster from `MediaPrep.preparedVideo` (`:392-418`), which with `posterFrame` (`:450`) goes from
    `private` to internal; `ChatSyncCoordinator.sendMedia` (`:2682-2752`) and `PendingMediaItemEntity`
    (`:73`, `:165-202`) carry the flag; `APIClient` (`:779-845`) and a `.sendRound` socket frame beside
    `.sendSticker` (`Core/SocketFrames.swift:47-57`).
  - The phone's orientation hold from Record to Stop (S8.1). Mac: the `recorderOpen` focused value that
    disables the toolbar items, Refresh and the conversation's menu commands; `windowShouldClose` and
    `applicationShouldTerminate` in `MacAppDelegate` (S8.3).

### Android (`android/app/src/main/java/me/nettrash/familyconnect/`)

- **Phase 0**: `VoiceRecorder` behind an interface (so `ChatViewModelTest:246` can fake it); `onCleared`
  stops and parks; `BackHandler` stops; `ON_STOP` beside `LifecycleResumeEffect` (`ChatScreen.kt:823-826`)
  parks; `calls: CallStateSource = CallStateSource.NONE` injected into `ChatViewModel` (the defaulted pattern,
  `ChatViewModel.kt:166-173`) — refuse while live, stop and park when it turns; `OnInfoListener` for the cap;
  a monotonic clock; `FLAG_KEEP_SCREEN_ON`; transient audio focus (`calls/CallAudio.kt:118-147`); no file name
  for voice notes (`data/repo/MediaPrep.kt:626`, `:688`); call buttons disabled while recording
  (`ChatScreen.kt:1102-1114`); the not-sent row (Send and ✕; its ▶ comes in Phase 1) and its parked store in
  `filesDir` and `SettingsRepository` — the file with its length, reply id and caption — so what Phase 0
  parks can be seen, sent and deleted; "Record audio" disabled while a not-sent note waits.
- **Phase 1**: `ui/chat/ComposerSlot.kt` and `ui/chat/RecordGesture.kt` beside `CaptureGate.kt`, with the
  vectors; `RecordSendButton` replaces the `FilledIconButton` (`ChatScreen.kt:5671-5688`) using
  `pointerInput { awaitEachGesture { … } }` (`PointerType.Touch`/`Stylus` → the hold, `Mouse` → a click on
  release, the secondary button → the menu), positions from `positionInRoot`, the S6 semantics; the rows in
  place of the field (`:5597-5639`); the video button as the field's `trailingIcon`; the staged chip's
  audio case with playback (`:5084-5126`) and the not-sent row's ▶; the "sending" entry written at a
  release (S2.6); strings in every locale and a strings test; the coach mark; the setting. A tap is a
  pointer up inside the bounds, wherever the press wandered (S1.1).
- **Phase 2**: `round: Boolean? = null` and `isRound` on `AttachmentDto` (`data/net/dto/ApiModels.kt:622-642`)
  — inside the attachments JSON, so no Room migration; the send field in `ChatApi` and `WsFrames`; every
  sticker-flag path in `MessageRepository`, keeping the poster upload; `roundOf()` beside `stickerOf`
  (`ui/chat/ChatItems.kt:476`) and excluded from `canEditMessage` (`:489`); bare in the bubble
  (`ChatScreen.kt:2566-2576`); a branch before the sticker's (`:3930`) drawing `RoundVideoBubble` and its
  `TranscriptLine`; a `TextureView` + `MediaPlayer` (with the auth header) in an `AndroidView` clipped by
  `CircleShape`; a now-playing owner living outside the activity, which asks for `AUDIOFOCUS_GAIN_TRANSIENT`
  when it plays, pauses when focus goes, and listens for `ACTION_AUDIO_BECOMING_NOISY` while it plays (S4);
  "Video message" in `previewText` (`MessageRepository.kt:1943-1944`).
- **Phase 3**: CameraX at **1.5.1** — Scan.Android's version, keeping the lockstep rule — `camera-core`,
  `camera-camera2`, `camera-lifecycle`, `camera-view`, `camera-video`, recorded in the catalog header as the
  fourth concession and why; `VideoMessageRecorder` owned outside the activity, with CameraX bound to a
  `LifecycleOwner` the recorder owns, so a rebuilt activity (`AndroidManifest.xml:86-90` declares no
  `configChanges`) only re-attaches `Preview.setSurfaceProvider` and re-applies the lock; Preview and
  VideoCapture in a `UseCaseGroup` with a 1:1 `ViewPort`, `QualitySelector` SD at 4:3,
  `setTargetVideoEncodingBitRate(500_000)`, `setDurationLimitMillis(max_round_video_ms − 500)` from the
  discovery key, `VideoCapture.targetRotation` from an `OrientationEventListener` at Record, the file
  unmirrored; `PreviewView` in `COMPATIBLE` mode, `FILL_CENTER`, clipped; on phones the orientation held
  as it is (`SCREEN_ORIENTATION_LOCKED`) from Record to Stop; after Stop, a probe — and only if the file is
  not exactly 480 × 480 H.264 + AAC with rotation 0 (`METADATA_KEY_VIDEO_ROTATION`), a Media3 pass with
  `Presentation.createForWidthAndHeight(480, 480, LAYOUT_SCALE_TO_FIT_WITH_CROP)`, which also bakes the
  rotation into the pixels (`MediaTranscode.kt:93-96`); then `Mp4Faststart`, `readVideoMetadata` (opened
  from `private`, `MediaPrep.kt:526`) for the poster, `declaredMime = "video/mp4"` (the
  `MediaProbe.kt:162-163` trap); CAMERA and RECORD_AUDIO asked together, the microphone opened by the
  recording itself; hidden without `FEATURE_CAMERA_ANY`.

### Windows (`win/src/`)

- **Phase 0**: park instead of cancel on a chat switch (`FamilyConnect.App/Views/ChatsView.xaml.cs:815-817`);
  stop and park in `OpenOverChats` (`MainWindow.xaml.cs:399-411`), in `OnClosing` (`:607-616`), in `Detach`
  (`ChatsView.xaml.cs:459`) and when `callBusy` turns (`:157`, `:2516-2521`), and on a session lock (S4);
  refuse while `callBusy`; pause the shared player when a recording starts; `AppCapability` checks;
  `DisplayRequest`; the not-sent row (Send and ✕; its ▶ comes in Phase 1) and a parked store in
  `LocalState` — the file with its length, reply id and caption — so what Phase 0 parks can be seen, sent
  and deleted. None of it ships before trial T7 (Blocked 1): CI never runs the app.
- **Phase 1**: `FamilyConnect.App.Logic/ComposerButton.cs` (xUnit, the vectors); the icon slot replacing the
  text Send (`ChatsView.xaml:177`; labels at `ChatsView.xaml.cs:206`, `:3842-3868`); the recording row in the
  input row; the menu (a mouse right-click, a pen tap with the barrel button, Shift+F10 or the Menu key —
  S8.6) and `KeyboardAccelerator`; a playable staged chip and the not-sent row's ▶; the video button
  overlaid at the `TextBox`'s trailing edge in the same grid cell; strings in `win.json`. T7 again before
  it ships.
- **Phase 2**: `Round` on `AttachmentDto` (`FamilyConnect.Core/Protocol/Dtos.cs:134-157`) and `RoundVideo`
  beside `StickerPicture` (`:214-215`); the send field through `SendRequest`, `ClientFrames.Send`,
  `ApiClient`, `SendPipeline`, `OutboxRow` and an outbox migration step (`Store/Migrations.cs`, held by
  `DatabaseTests`); `ConversationModel.SendRound` beside `SendSticker`; a bubble branch beside the sticker's
  (`ChatsView.xaml.cs:1074-1082`, `:1125-1132`), bare (`:1194-1204`); `RoundVideoElement` — an `Ellipse` with
  an `ImageBrush` poster, the capsule, the dot — opening the viewer; `TranscriptPanel` added explicitly; a
  `PendingRoundElement` beside `PendingStickerElement` (`:4745`); the shared player (`ChatsView.xaml.cs:117-127`)
  pausing for a call, a session lock and a change of the default output device (S4).
- **Phase 3, after the trials**: `FamilyConnect.App/Services/VideoMessageRecorder.cs` mirroring
  `VoiceRecorder`'s always-release rule — `MediaCapture` for `AudioAndVideo`, the front-panel camera; the
  preview through `MediaPlayerElement` and `MediaSource.CreateFromMediaFrameSource`, mirrored with
  `ScaleX = -1`, clipped by `CornerRadius = D/2` if T2 finds #8264 fixed in 2.4.0, else by a composition
  ellipse, else by a ring painted over an opaque card; for a #9756 camera, S3.6's sentence alone (no
  `MediaFrameReader` fallback until such a camera can be tried); a temporary MP4 at the camera's size; the
  square from the existing `MediaTranscoder` path (`Services/MediaPreparing.cs:798-803`) with
  `VideoTransformEffectDefinition` (centre `CropRectangle`, `OutputSize` 480 × 480) into H.264 500 kbit/s +
  AAC mono (the 64k → 96k ladder, `MediaEncoding.cs:257`); `Faststart.MoovFirst`; `MediaPrep.MatchesMagic`;
  `PosterAsync` made internal; `FamilyConnect.App.Logic/RoundVideoRules.cs` (cap, floor, crop rectangle,
  staged media); the ask in `OnClosing` over a clip in REVIEW (S8.6).

### Web (`web/`)

- **Phase 0**: `start_recording` refuses while `on_call` (`src/views/conversation.rs:739-774`); leaving the pane
  parks instead of cancelling (`:775-794`); a hidden tab stops and parks (`src/main.rs:229-275`);
  `beforeunload` while recording or a not-sent note exists (the guard at `src/main.rs:179-206`); the wake
  lock; the not-sent row (Send and ✕; its ▶ comes in Phase 1), its note, reply and caption kept in the
  tab's state, so what Phase 0 parks can be seen, sent and deleted.
- **Phase 1**: `src/views/composer.rs` — a `records` prop (set only by the conversation, never the thread or an
  edit), the icon slot replacing the text button (`:543-544`, `:658-663`), never `disabled` while recording,
  `Listening::in_the_click()` in the click; the recording row in the composer row (from
  `conversation.rs:1349-1351`); Esc as Stop (`src/views/attach.rs:324-331`); a playable staged chip (a blob
  `<audio>`) and the not-sent row's ▶; the video button; the menu; touch acting on `pointerup` and the
  touch `contextmenu` prevented on every layout (S8.7, S8.8); the meter from the worklet tap (none in the
  `MediaRecorder` engine); a visually hidden announcement node, and the typing line quiet while a
  recording runs (S6); strings in `i18n/web.json`.
- **Phase 2**: `round` in `src/model.rs` (`:764-770`) and `Message::round_video()` beside `sticker()`
  (`:901-910`); `Draft`/`Outgoing` (`src/store.rs:233-258`, `:1277-1284`); `SendRequest` (`src/api.rs:255-280`);
  a branch in `src/views/bubble.rs` (`:777-790`) with the class `is-round-video`; a `RoundVideoTile` — the
  poster through `use_media(…, Variant::Preview, …)`, a tap fetching the original into an inline
  `<video playsinline>` in a `border-radius: 50%` box, an SVG ring, the loading ring and failed line
  (S5.3), the `AudioPlayer` asked/playing pattern for Safari (`src/views/attachments.rs:405-548`),
  `TranscriptBlock` under it; the chat-list labels; a now-playing owner that pauses for a hidden tab, a
  call and a `devicechange` (S4). (No "note" in the names: Decision 16.)
- **Phase 3**: `src/round_video.rs` — `getUserMedia({video: {facingMode: "user", width: {ideal: 640}, height:
  {ideal: 480}}, audio: {channelCount: {ideal: 1}}})` for one prompt, its audio track stopped at once and
  asked for again inside the Record click (S3.4), into a muted `playsinline` `<video>` mirrored by CSS;
  `requestVideoFrameCallback` (bound by hand, like `src/webcodecs.rs`) → a centre-cropped 480 × 480 canvas →
  `VideoFrame` → `VideoEncoder` with `h264_config(480, 480, 30, 500_000, Latency::Realtime)` — the function
  gains a latency argument, `Quality` for transcodes as today and `Realtime` here, so the probe asks about
  the configuration actually used (`webcodecs.rs:11-17`, `:224-249`) — dropping a frame rather than
  queueing past `QUEUE`; a keyframe every 2 s; AAC from the worklet tap; the live encoder written inside
  `src/encode.rs` (its private `assemble`, `Aac::track` and sinks opened as `pub(crate)`); `mp4::layout` +
  `assemble` with a `lead` on the later track; a `prep::round_video` for the poster; the recorder's
  `aria-label`, REVIEW's capture-phase Space and the `beforeunload` guard extended to REVIEW (S8.7).
  **Never `MediaRecorder`.**

## Tests

- **Shared**: the `record` vectors (S2.3 included) in `web/text`, compared against the ports in CI, and
  `web/text`'s own `cargo test` run in CI for the first time (the hookup under "Shared rules").
- **Server**: `round_flow.rs` (with `round: true` refused on a photo, an audio, a file and a location),
  the migration constraint tests, the config tests for the clamped default, and the unit tests above.
- **Apple**: Swift Testing suites for `ComposerSlot`, `RecordGesture` (the vectors), `ParkedRecordings`, the
  Undo window (an injected clock) and `AudioRecorder` (the cap reported; no deactivation during a call,
  through a seam like `permissionProvider`); `RoundWireTests` from `StickerWireTests` (absent versus true);
  `RoundBubbleTests` from `StickerBubbleTests` — pixels transparent outside the circle at 200 and 240; a
  recorder test writing synthetic sample buffers and checking `MediaFixtures.facts` (480 × 480, H.264, AAC
  mono, `moov` first) and `MediaPlan.planVideo(...) == .keep`; `check-strings.py`; `build-for-testing` for
  iOS AND macOS. The UI tests' anchors stay.
- **Android**: JVM tests for `ComposerSlot` and `RecordGesture` (the vectors); `ChatViewModelTest` gains
  refusal during a call, stop-and-park when a call goes live, `onCleared`, the cap into review, the Undo window
  on the test clock (`runCurrent()`, not `advanceUntilIdle()`); `RoundPresentationTest` (`roundOf`, no
  edit); Robolectric Compose tests for the slot's semantics (labels, custom actions, no long-click) and the
  round bubble; the strings test in every language. A device test for CameraX's square output — rotation 0
  after Stop, and a fold mid-recording that keeps recording — run by hand (Blocked 4).
- **Windows**: xUnit for `ComposerButton` (the vectors), `RoundVideoRules`, the park and the cap,
  `CatalogueTests` with `win.json`, `DatabaseTests` for the outbox step, the wire; `xamlcheck`; T7 by hand
  before Phase 0 and Phase 1 ship.
- **Web**: wasm tests for the slot (labels, never disabled while recording, `aria-disabled` when dimmed);
  the recorder's lifecycle (a hidden tab parks; a call refuses); the live encoder under Chrome's fake camera
  (`web/webdriver.json`), its output parsed by `mp4_read` as `moov`-first 480 × 480 H.264 + AAC of the right
  length; `RoundVideoTile`; layout at phone widths with the safe areas. **Tests that change**:
  `views/composer.rs:983` (`staged_attachments_send_with_no_caption` clicks the last button with nothing
  staged, which is now the microphone), `:735`, `:872`; `layout_tests.rs:854`, `:963` (they read the hidden
  word), `:994` (the box against the new button's height); the recording bar's Esc test.
- **Device trials before each release**: every row of S4 by hand on an iPhone, an iPad, a Mac, an Android
  phone, a Windows machine and two browsers, with VoiceOver, TalkBack and Narrator once each — among them
  a call ringing while the recorder is open on an iPhone (the call's cover must rise over it), a phone
  turned mid-take, and closing a window or quitting over a clip in REVIEW.

## Phases

1. **Phase 0 — today's recorder made safe**, on all five clients, before ANY new way in: no recording in
   any call phase, stop-and-keep on a call, the background, a desktop lock or sleep, a hidden window and
   leaving the chat; no deactivating a session a call owns; the singleton fixed on Android; the rail and
   the tray on Windows; `visibilitychange` on the web; the 5:00 cap reported on iOS and Android; Android's
   file name; and the not-sent row with its per-account store (the tab's state on the web), so that what
   Phase 0 keeps is never kept out of sight. **Visible change**: microphones that no longer stay on, and a
   recording stopped by a call, the background, a hidden window or leaving the chat waits in a "Voice
   message not sent" row, to be sent or deleted. Windows waits for T7.
2. **Phase 1 — voice in the Send slot**, on all six clients, no wire change: `protocol.md:109-114`; the
   shared rules and vectors; the slot, the rows, the hold on iPhone, iPad and Android, the Undo window,
   review with playback (the not-sent row's ▶ included), the paperclip renames, the assistant chat cleaned
   up, the setting, the teaching, the accessibility, the strings.
3. **Phase 2 — round video on the wire and on screen**: protocol.md first; migration 0052 and the server;
   then the receiving side on all five clients, so circles draw everywhere before anybody can send one.
   A build at this phase shows no video entry: **round available** needs a build that records (S1.2).
4. **Phase 3 — recording round video**: 3a Apple (iPhone, iPad, Mac — no new dependency); 3b the web on
   Chrome and Edge, Safari after Blocked 2; 3c Android with CameraX; 3d Windows after Blocked 1.

Every phase ships alone: an app that has not reached a phase records voice as before and draws a round
video as a square one, and a server without phase 2 is offered no video at all.

## Decisions

> **Revised 2026-10-06.** The hold — and everything only it needed — was removed at the owner's request
> after device testing: tap-to-record is the one way to record a voice message, on every platform, and a
> long press on the microphone starts nothing and opens nothing (the note at the top). Decision 1 now
> reads "tap to record, video on purpose"; decisions 5 and 7 are withdrawn; decision 6 keeps only its
> completed tap ("recording never starts on touch-down"; an unsteady or slow press still taps when it
> lifts inside); decision 12 loses its Undo-window clause — an interruption never sends; decision 15
> stands. Blocked 3 (tuning the hold) is moot. Decision 41 is added below.

1. **The model is "tap to record, hold to talk, video on purpose".** It is the only one of the three that
   serves both halves of a family at once — tap-to-start for grandparents, mouse, keyboard and screen-reader
   users; hold-to-talk for messenger habits — and it won two of three reviews.
2. **A tap, click, Enter or screen-reader activation starts a hands-free voice recording when the
   activation completes, and the same slot sends it.** It is the owner's "just by clicking" made possible,
   keeps the house's tap-to-start rule (`AudioRecorder.swift:11-14`), and meets WCAG 2.5.2's "No
   Down-Event".
3. **A long press does not open the camera.** It is taken on every platform, it inverts the strongest habit
   people bring, a mouse and keyboard have none, it is undiscoverable, and the camera would light for the
   people who meant to talk.
4. **No tap-to-switch mode.** WhatsApp needed three fixes to walk it back, Telegram's web clients dropped it,
   and it hides a mode on the most-used button.
5. **The hold exists only on iPhone, iPad and Android in this version, with finger, Pencil or stylus.**
   Windows touch and mobile browsers are tap-only until device trials — a Windows touch hold is a right-click
   nothing here can test, and a touch `pointerdown` is not user activation in a browser.
6. **Recording never starts on touch-down.** It starts on a completed tap or at the hold threshold,
   max(500 ms, the system long-press duration), so a brush does not open the microphone, and the person's
   own accessibility setting is respected. A thumb resting past the threshold does open it — and is
   caught by the haptic and the hold row, the first-release review and the Undo window. A press that
   wanders past the slop can no longer become a hold but still taps if it lifts inside the button, as any
   button's does, so an unsteady press is never a dead button.
7. **A release sends only behind five guards**: at least 1.0 s recorded, not silent (which catches a
   muted microphone, not a quiet room), slide-to-cancel not armed, the first release on each device going
   to review with an explanation, and a 5-second Undo before anything is uploaded — plus an opt-in,
   per-device **Review Before Sending**, the Undo window's WCAG 2.2.1 "turn off", and review always while
   a screen reader or Switch Control runs. A sent message cannot be deleted here, so the release needs
   both an abort before and an undo after; the Undo window is crash-safe, kept as "sending" until it ends.
8. **Video is reached through a labelled video button in the empty field, the paperclip, the microphone's
   secondary menu and its accessibility action — never by a shortcut.** The camera turns on only when
   somebody presses something that says video, and inside the empty field the button costs the row no width.
9. **Video always previews before it records, and the microphone opens only at Record; Record turns into
   Stop and Stop into Send, each behind the 600 ms guard; the camera goes off at Stop; in REVIEW, Space
   plays and pauses wherever focus is and never sends; the preview closes after 60 s idle, when the window
   loses focus (never to a permission prompt the recorder raised) or when the app goes to the
   background.** Nobody is filmed or heard unseen, "Not recording" is true, no clip leaves unwatched, and
   a forgotten preview does not leave the camera on.
10. **The video recorder covers the whole window, shows the reply the video will carry, and is never
    parked.** The chat cannot change under it — on the Mac, whose window toolbar nothing can cover, because
    every toolbar item is disabled while it is open — so there is no per-chat video state to keep. On
    iPhone and iPad it is a layer of the root beneath the call's cover, so a call stops it into REVIEW and
    simply rises over it; a system that ends the app loses a REVIEW clip, at most a minute of video.
11. **A voice recording stopped by anything but the person becomes a "not sent" row that is never carried
    by another Send, keeps the reply it was recorded under and any caption, and is kept in the app's
    storage (the tab's lifetime on the web) until it is sent, deleted or the person signs out. It arrives
    in Phase 0, before anything parks into it.** A recording cannot be made again, and a recording nobody
    finished deciding about must not ride out with the next text. This reverses `ComposerDrafts.swift:13-18`
    for recordings only. A phone's browser can still lose it without asking, as it can lose the web's
    outbox: the web keeps nothing a person wrote on the device.
12. **Interruptions stop and keep, and never send or discard** (anything under 1.0 s excepted); the Undo
    window alone ends by sending, because the release had already decided. A desktop lock or sleep is an
    interruption like any other.
13. **Esc, VoiceOver's escape and Android's Back stop a recording and never delete one; Delete and Retake
    ask at 10 s and longer; closing a finished video clip always asks — Esc, Back, closing the window,
    quitting the app or closing the tab (a phone's browser may give no chance to, S8.8).** Reflex keys and
    window chrome must not destroy what cannot be
    re-recorded, while the labelled Delete on a recording under ten seconds is already a decision and
    needs no question.
14. **One recording at a time in the app; recording pauses playback; no recording in any call phase; the
    call buttons are disabled while recording.** Two microphones, or a call over a recording, has no right
    answer.
15. **A 600 ms activation guard in both directions, only after the slot's own activation, and a 1.0 s
    floor everywhere** replace the 1024-byte rule as the floor people see: a double tap can neither start
    nor send a blip, while typing "ok" and sending at once is never slowed — a change made by typing,
    pasting or staging is not guarded.
16. **The flag is `round`.** It names the presentation as `sticker` does, and "note" is already a board word
    in this protocol.
17. **A square 480 × 480 at 30 fps, 500 000 bit/s H.264 and 64 000 bit/s mono AAC, `moov` first, upright,
    not mirrored, never re-planned.** It is the profile's own formula at 480 × 480 and 30 fps, about 4.2 MB a
    minute, kept whatever rate the camera delivers; the true view reads correctly; the planner must not
    re-encode it — below 24 fps it would even fall outside Rule A.
18. **The length is 60 s — fixed, advertised as `max_round_video_ms`, clients stopping 500 ms short; the size
    ceiling is 12 MiB, or the attachment ceiling when that is lower, and configurable — bounded only when
    set, so no existing server is refused at startup.** It matches every messenger's minute, leaves room
    for AAC's padding, and lets a later client record larger without a server change.
19. **The server checks only declared metadata, after a claim that flags only a video: `video/mp4`, equal
    sides from 1 to 720, 1 ms to the length limit, the byte ceiling; nothing is decoded; every refusal is
    a 400, never a 500.** It is the sticker's split and the server's written rule, and an outbox retries a
    500 forever.
20. **A round video has no caption and cannot be edited.** There is no balloon to hold words, and an edit
    cannot change an attachment anyway.
21. **Received circles have no balloon, are 200 or 240 across, draw from the poster only, play inline with
    sound on a tap, and never autoplay.** Data and battery on the family's own server and metered phones,
    the tile rule at `protocol.md:107-108`, WCAG 2.2.2, and "a message for me" meaning "tap to hear it".
22. **No played or watched receipt is sent; an unplayed dot is kept on the device only.** It tells the reader
    what is new without telling anybody when somebody watched.
23. **On iPhone and iPad, voice notes and round videos play through `.playback`, heard with the silent switch
    on, never while a call holds the session and never through the earpiece; on every client playback
    pauses for a call, the background, lost headphones and another app's sound, and on phones and tablets
    the screen stays awake while it plays.** They are audio the person explicitly started — the HIG's own
    example — and the `audio` background mode stays the calls' alone.
24. **The assistant chat gets no microphone and no video button, and its paperclip loses Record Voice
    Message.** The assistant is only ever shown `[voice note]` or `[video]`, and every message there costs a
    consented model call.
25. **Threads get no recording in this version; the server accepts a round video in one, and in any chat —
    the assistant's under the sticker's consent question.** No thread composer has staging or a recorder
    today; the server has no reason to refuse, and clients can add it later.
26. **The Mac records round video**, reversing `CameraPicker.swift:17-20` for this one case. The old reason
    was photos ("a webcam is not how anyone sends a photo from a desktop"); a talking-head circle is exactly
    what a webcam is for, and the entitlement and usage strings already exist.
27. **Android adds CameraX 1.5.1 — Scan.Android's version — as the catalog's fourth concession, bound to the
    recorder's own lifecycle, with a Media3 pass only when the file is not already square, H.264 + AAC and
    at rotation 0.** It keeps the lockstep rule, and CameraX is what Google recommends; a fold or a resize
    must not end a take, and a square file hides its rotation from the size; moving both apps to 1.6 is a
    separate change.
28. **Windows clicks with every input, records round video only after the trials, and plays circles through
    its viewer in this version; its menu opens for a mouse right-click, a pen barrel tap and the keyboard,
    never a hold; Phase 0 and 1 ship only after a smoke test on the owner's machine (T7).** Nothing at run
    time can be checked on this Mac or in CI, and a blank preview must never become a recording without a
    picture — a #9756 camera gets the sentence until one is at hand to try a fallback on.
29. **The web records video only where H.264 and AAC encoding and `requestVideoFrameCallback` exist, and not
    on WebKit until a real-device trial passes.** Firefox and desktop Linux cannot encode AAC, and Telegram
    turned the same pipeline off on Safari.
30. **The push and the chat list say "Video message"; the assistant's `[video]`, transcripts, reports (whose
    inbox keeps saying "Video"), statistics and retention do not change; the word each client already uses
    for a voice note stays.** The new word is the only thing a reader needs; everything else already treats
    it correctly as a video, and `kind=audio` is also a picked sound file the server cannot tell apart.
31. **The paperclip says "Record Voice Message" and "Record Video Message"; Android's system-camera "Record
    video" becomes "Take video".** The pair reads as a pair, and two near-identical names for different
    things would be the confusion this plan exists to avoid.
32. **⌥⌘R on the Mac and an iPad keyboard, Ctrl+Shift+R on Windows and Android, record voice and, pressed
    again, stop into review; nothing on the web.** R is for record, ⌘R is already Refresh, a shortcut never
    sends, and a browser would reload the page.
33. **The preview is mirrored and the file is not.** People frame themselves with a mirror, and the family
    should see what the camera saw.
34. **The screen stays awake while recording, on every client, and while something plays on phones and
    tablets.** Auto-lock would otherwise turn a long story into a parked draft halfway through, or cut a long
    note short now that playback pauses in the background.
35. **Android voice notes stop sending a file name.** The server drops it anyway
    (`handlers_attachment.rs:290`), and until the echo arrives the sender's own chat list shows
    "voice-<milliseconds>.m4a" (`VoiceRecorder.kt:59`, `MessageRepository.kt:1945`).
36. **A clip that cannot be made round, or is too big for a video message, is sent as a regular video, after
    the person has seen why.** It is the profile's Rule C: an optimisation may never turn a send that would
    have worked into one that does not.
37. **Nothing in the product says or suggests that a sent message can be removed.** It cannot be (`app.rs:162-165`).
38. **Magic Tap stops a recording and never starts one.** When an app does not take it, the system plays
    and pauses music (Apple's guide, Checked facts); a VoiceOver user pausing a podcast must not open the
    microphone instead.
39. **On phones the screen holds its current orientation from Record to Stop — never forced to portrait;
    where it cannot be held (an iPad, Android at 600 dp and wider, a foldable) the recorder's layout never
    switches mid-take.** The controls must not move under the thumb, WCAG 1.3.4 forbids forcing one
    orientation, and whatever turns sideways is seen in REVIEW before Send.
40. **Video entry points exist only in a build that records round video on that platform.** A Phase-2 build
    that can only receive circles, or Windows before 3d, would otherwise show a camera button with nothing
    behind it.
41. **(2026-10-06) While the video recorder is open, the composer row is not drawn and takes no hits; the
    recorder's controls have their own solid, safe-area-aware bar, its status its own capsule above the
    circle, and the circle never overlaps either at any size.** On the owner's iPhone the composer showed
    through the recorder — Close over the paperclip, Switch over ✨, the "Voice message" caption over the
    field, the composer's Send over Record — and a thin scrim let the chat compete with the camera. A
    recorder that "covers the whole window" (decision 10) must cover the composer too.

## Blocked

Each item below holds back only its own piece; everything else ships.

1. **Windows runtime trials**, on the owner's ARM64 machine — the only Windows runtime there is
   (`win/README.md:14`): T1, the frame-source preview on its webcam, and whether `MediaCapture` for
   `AudioAndVideo` holds the microphone in PREVIEW; T2, `CornerRadius = D/2` on `MediaPlayerElement` first
   (#8264 was closed after "some fixes in WASDK 1.4"; the app is on 2.4.0), then an ellipse composition clip;
   T3, a 480 × 480 `MediaTranscoder` output with 64 kbit/s AAC, `moov` first after Faststart; T4, touch and
   pen on a microphone with `IsHoldingEnabled = false` (no menu from a touch or pen hold, a click records)
   and a pen barrel tap (the menu); T5, Narrator reading every state; T6, the camera while a WebView2 call
   has it; T7, a smoke test of Phase 0 and Phase 1 — `DisplayRequest` (it once threw under the Windows App
   SDK, #3002), the `AppCapability` checks, the session-lock notification and which API delivers it,
   Ctrl+Shift+R, the menu, the parked store and the icon slot — because CI never runs the app
   (`ci.yml:788-800`). T7 holds Windows' Phase 0 and Phase 1; Phase 3d and inline circles wait for the
   rest. A `MediaFrameReader` fallback for #9756 cameras is not trialled: it waits for such a camera.
2. **A Safari 26 trial on a Mac and an iPhone** of the live canvas → `VideoFrame` → `VideoEncoder` →
   `mp4.rs` path, and of the microphone coming back inside the Record click without a second prompt.
   Web video recording on WebKit waits for it.
3. **One device session to tune the hold** — 500 ms, 60 and 100 units, 1 s, 5 s — on an iPhone, an iPad and
   an Android phone. The values above ship until then.
4. **CameraX's output on two real Android phones**: whether it is square at all, whether its rotation is 0
   after Stop when the phone was held sideways, and whether a split-screen resize — and, on a foldable, a
   fold — mid-take keeps recording.
   The Media3 pass covers a file that is not square or not upright; the trial says whether it is needed.
5. **Turning an iPad, a 600-dp Android tablet and a foldable during RECORDING**: whether AVFoundation keeps
   a 480 × 480 output steady while the capture angle follows the device, and whether CameraX applies a target
   rotation changed mid-take. Until it passes, those devices keep the starting angle and REVIEW shows any
   sideways tail before Send; nothing else depends on it. (iPad multitasking camera access needs no
   entitlement at this deployment target — Apple's page settles what this item used to ask.)

Nothing here changes existing behaviour until Phase 0 is written, and every phase keeps the shipped apps
working.
