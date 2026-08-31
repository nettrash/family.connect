# App Store Connect — Family Connect

Every text the App Store listing needs, written against the shipped 1.0 feature set and checked
line by line against the code. Fill every `[PLACEHOLDER]` before submission — they mark the demo
accounts and support address, which live on the server and not in this repository.

**This file describes an iOS-only submission.** macOS ships from the same target and the same
bundle id, is held for a later release, and has its own section at the end. Nothing in the
listing copy or the review notes below is written to be true of the Mac.

Character limits are stated in each heading and were measured, not estimated. App Store Connect
enforces them at entry: a Description over 4,000 characters cannot be saved, and App Review
Information notes over 4,000 are truncated mid-sentence.

## Promotional Text (168/170 chars)

A private messenger for one family. Messages, photos, voice notes and calls. Start on our server, move to your own whenever you like. No ads, no analytics, no tracking.

## Description (3985/4000 chars)

Family Connect is a private messenger for one family and nobody else — no feed, no discovery, no directory of users. The only people who can reach you are the ones your family's owner let in.

It works out of the box: install, pick a username — no email address, no phone number — then create your family, and you become its owner, or join one with an invite code. The owner decides whether that code admits people instantly, needs their approval, or admits nobody at all, and can rotate it, cap how many members the family holds, or remove someone at any time.

What makes Family Connect different is where your messages live. There is no vendor cloud: every installation talks to a single Family Connect server. Out of the box that is the default server we operate, so you can start right away — and on it, we are the ones holding your family's messages and files. But the server software is free and open source (MIT, written in Rust), and "Change server" on the sign-in screen points the app at any other one. Put it on your own hardware and your conversations exist in exactly two places: your server and your family's devices. One server can host several families, so one technically inclined relative can cover everyone.

Day to day it works like any modern messenger:

- One shared chat for the whole family, and a private one-to-one chat with any member of it
- Photos, videos, voice notes and files of any kind — up to ten on a message, with a message's photos drawn as one album you swipe through and pinch to zoom
- Take a photo or video without leaving the chat, or send one in from another app
- Your location sent once, drawn on a map, only when you choose to — never continuously, never in the background
- Polls in the family chat, a board of coloured notes anyone can move, replies, editing what you have sent, and reactions from a picker of hundreds of emoji
- One-to-one voice and video calls that ring on the lock screen and land in the Phone app's Recents; link a member to a contact and call them from their card, with that link never leaving your device
- Real-time delivery with typing indicators, read receipts in one-to-one chats, unread counts
- Notifications when messages arrive while the app is closed — and none while you are already reading
- History kept on the device, so you can read it back with no signal at all
- Report a message or a member to your family's owner, who can remove them — and a report about the owner goes to whoever runs the server instead
- Block anyone: their messages fold behind a row you can tap, their chat leaves your list, their calls never ring you
- Nine languages, from German and Spanish to Japanese, Russian and Simplified Chinese

What it does not have: ads, analytics, tracking, crash reporting, or the attribution SDKs that usually arrive with a free messenger. The app carries exactly one piece of third-party code — Google's open-source WebRTC library, under a BSD licence — used for nothing but carrying calls.

And the honest limits. This is not end-to-end encryption and we will not imply that it is: messages and attachments travel encrypted to the server your family chose — plain http only for a server on your own network — and are stored there in readable form, so whoever runs it can read them. Calls are different: picture and sound go straight between the two devices wherever the network allows; where it does not, the call connects only if your server's operator runs a relay, which forwards the stream encrypted without being able to read it. Two things reach beyond your server and can be switched off in Settings: the map under a shared location and link previews. Placing a call also asks a public STUN server for your device's address. There is no message search yet, and how long the server keeps older messages is a setting for whoever runs it.

If you want your family's conversations off big-tech servers — and, when you are ready, on hardware you own — this is what Family Connect is for.

## Keywords (95/100 chars)

chat,messenger,private,self-hosted,server,calls,video,voice,photos,relatives,grandparents,group

*("family" and "connect" are left out on purpose — the app name is meant to carry them. Confirm the App Store Connect name first: the icon and the app itself are called "Family", and if the store listing is named that too, "connect" is indexed nowhere and should take "group"'s place here; the five free characters are held for exactly that. "kids" is dropped deliberately — a keyword implying a child audience invites a Guideline 1.3 and age-rating question this app cannot answer.)*

## Notes for App Review (paste verbatim — 3730/4000 chars)

DEMO SERVER: https://fc.nettrash.me — compiled into this build, so there is nothing to configure.

DEMO ACCOUNTS.
- Owner: [DEMO_USER] / [DEMO_PASS]. Owns the reviewer family; history seeded (photos, a video, a voice note, an open poll, a shared location, board notes, one 1:1 chat).
- Second account: [DEMO_USER_2] / [DEMO_PASS_2], same family — sign in on a second device for typing indicators, read receipts and a call.
- Invite code [INVITE_CODE]. Support: [SUPPORT_EMAIL].

WHAT IT IS. A client for a small open-source (MIT, Rust) chat server. There is no vendor cloud: each installation talks to exactly one server, and a family's messages live in that server's database and on the family's own devices. On the device the app is named Family; the App Store record is Family Connect. Two modes: (1) the default server above, operated by the developer and online for the whole review period — launch and the sign-in screen appears; (2) "Change server…" on that screen points the app at a family's self-hosted server. Review needs only mode 1. These notes describe the iOS build.

CONTACT IS CONTAINED (guideline 1.2). There is no public feed, no discovery, no user directory and no user search — nothing is posted to a public surface. Registration is open (username, display name, password; no email, no phone), but an account in no family can start nothing: opening a one-to-one chat, reporting and blocking are all refused server-side unless both people are in the same family, and calls exist only inside a one-to-one chat that had to be created that way. The only ways into a family are creating your own or presenting an 8-character invite code, and a new family defaults to "Need approval", so the owner admits each member, and may rotate the code, cap membership, close joining, or remove anyone.

REPORT AND BLOCK. Long-press a message → "Safety" → "Report…" or "Block". A person can also be reported or blocked from Manage Family by long-pressing their member row → "Safety". Four fixed reasons. The moderator is the family owner: reports arrive in chat list → Settings (gear) → Family → Manage Family → Reports, and push the owner. A report naming the owner is never listed to them; that sheet shows the server's support contact instead — [SUPPORT_EMAIL] here, monitored by us, and we can act on the account on that server.

DELETE ACCOUNT (5.1.1(v)). Chat list → Settings (gear) → last section, beside Log Out. It asks for the account password (a live session is not proof) and a second confirmation, then runs immediately and irreversibly — no grace period. It erases the username (freeing it), display name, password, avatar, birthday, and every session and push token. One-to-one chats are deleted for both people; family-chat messages, board notes and reactions stay, re-attributed to "Deleted account". Ownership passes to the longest-standing member; a last member takes the family with them. It works on the demo accounts too — we re-provision them.

NOT END-TO-END ENCRYPTED, and the app does not claim to be. Message text, photos, files and locations are stored readable in the chosen server's database and filesystem; on the default server the developer is that operator. Call media (WebRTC, DTLS-SRTP) travels directly between the two devices wherever the network allows; where it cannot, the call connects only if a relay is configured on our server, and a relay forwards the encrypted stream without being able to read it.

LOCAL NETWORK PROMPT. With both devices on one Wi-Fi, iOS asks for Local Network permission as a call connects — please allow it, or a same-network call cannot connect. Please test calls on two real devices: the Simulator has no APNs to wake a backgrounded phone, and no camera.

## Reviewer walkthrough (supporting detail — not pasted into App Store Connect)

These notes are for the reviewer's own use and for the developer's records. Do not paste this
part into App Store Connect; the field above is the whole submission.

### Registration, and what "open registration" does and does not mean

"Register" takes a username, a display name and a password — no invite, no email address, no
phone number — so a reviewer can create a fresh account instead of using ours. Open registration
means anyone may create an *empty account of their own*. It does not open contact: a brand-new
account is in no family, and the server has no directory, no search and no family discovery, so
there is nobody for it to reach. Leaving a family, or being removed from one, also drops that member from their one-to-one chats
on the server, so they can no longer send or call there. One narrow residue we would rather state
than have found: a member who stayed can still ring a peer who left, and that peer can answer,
because answering is keyed to the ringing call rather than to membership. Everything *new* — a
first one-to-one chat, a report, a block — still requires a shared family.

### Step by step

1. **Launch.** The app is already pointed at the default server, so the sign-in screen appears
   immediately. Sign in as `[DEMO_USER]`, or register and join the reviewer family with
   `[INVITE_CODE]`. A new account can equally create its own family and own it.
2. **Notifications prompt.** The standard alert/sound/badge request is not shown at launch or on
   the sign-in screen. It appears once, at the end of the first successful sync — signed in and
   in a family, so it has context. A refusal is remembered and never re-asked.
3. **The chat list.** The family chat is pinned first, one-to-one chats follow. The toolbar has
   exactly three buttons: New Chat, Settings (gear, top left) and Board. Owner tools are not on
   this toolbar — they live at **Settings → Family → Manage Family** (the same row reads "Family
   Members" for a non-owner, who sees only the roster). Manage Family holds join requests, the
   invite code (shown as selectable text, with **Rotate Code** — there is no copy button; the
   Family section of Settings shows the code too and adds **Share Invite**), the join policy, the
   member cap, per-member birthday, password reset, remove from family, and the Reports inbox.
4. **Photos and video.** Composer attach menu → "Photo or Video". This opens the system photo
   picker and raises **no** permission prompt by design: the picker runs out of process and hands
   over only what you select, so the app declares no photo-library *read* permission at all.
   "Camera" in the same menu opens in-chat capture and raises the camera prompt, verbatim:
   *"Video calls show you to your family while you are in one, and taking a photo or video in a
   chat sends it there, and nowhere else. Family never opens the camera outside a call or a
   capture you start."* (The Camera item is hidden on a device with no camera, including the
   Simulator.) Up to ten attachments per message, 100 MB each; several photos draw as one stacked
   album card. Tapping it opens a full-screen viewer — swipe between items, pinch to zoom. Video
   is streamed rather than downloaded, so a long clip starts almost at once. The viewer's Share
   button hands off to the system share sheet, and choosing "Save Image" or "Save Video" *there*
   is what raises the photo-library *add* prompt: *"Saving a photo or video from a chat puts it in
   your library. Family never reads your library — the picker hands over only what you choose."*
   The app has no save button of its own.
5. **Voice message.** Attach menu → "Record Audio". First use raises the microphone prompt:
   *"Voice calls and voice messages use the microphone, and a video you shoot in a chat records
   sound with it. Family never listens outside a call, a recording or the camera."* A strip in the
   composer shows elapsed time with Cancel and Stop; the sent note plays and scrubs inside the
   bubble. There is deliberately no waveform.
6. **Poll.** Attach menu → the poll item, **in the family chat only**. The server refuses a poll
   in a one-to-one chat on purpose (`invalid_poll`), so its absence there is not a bug. Two to ten
   options, fixed once sent; anyone may vote and retract; the author may close it once, one way.
7. **Location.** Attach menu → "Location" raises the when-in-use prompt: *"Sharing your location
   sends where you are to your family, once, when you choose to. Family never follows you and
   never sends your location on its own."* The app takes a single fix (20 s timeout) and stops —
   there is no live, continuous or background location anywhere in the app, and no Always
   authorisation is declared. The result draws as a map pin; map previews can be switched off in
   Settings, after which the bubble shows pin and label and hands off to the Maps app.
8. **Share Extension.** From Photos or Files, tap Share and choose Family. It accepts up to ten
   images, movies or arbitrary files, stages them in the app group and opens the app, which asks
   which chat they belong in and puts them in that chat's composer. Nothing is sent until you
   press Send. The extension requests no permissions of its own.
9. **Two devices.** Sign the second account in on a second device. Typing indicators appear in
   both kinds of chat. Read receipts — clock, single tick, tinted double tick — are drawn **only
   in one-to-one chats, never in the family chat**; that is deliberate, so please do not test
   receipts there. A chat is marked read conservatively: only when it is open, its newest message
   is on screen, and the app is frontmost.
10. **A call, on two real devices.** Please do not test calls in the Simulator: it has no APNs, so
    a backgrounded callee is never woken to ring, and no camera, so video is unavailable. Open the
    one-to-one chat between the two accounts; the voice and video buttons are at the top of that
    chat. Both buttons appear **only in a one-to-one chat** — never in the family chat, and never
    in a chat with a member you have blocked, because that chat leaves your list entirely. The
    phone button follows the server's calls switch and the video button follows a separate video
    switch; both are on for the review period. Calls are strictly one-to-one, one at a time; there
    is no group or conference call. Placing or answering raises the microphone prompt above; a
    video call also raises the camera prompt. Refusing the camera does not end a video call, it
    proceeds camera-off (answering a video call from the lock screen answers camera-off and asks
    afterwards); refusing the microphone is the one denial that ends a call. On a shared Wi-Fi,
    iOS raises the local-network prompt: *"A voice call connects directly between your devices. On
    a shared Wi-Fi network that direct connection is a local one, and this permission is what
    allows it; without it calls between devices on the same network cannot connect."* Incoming
    calls use CallKit — full-screen ringing on the lock screen, and the call appears in the Phone
    app's Recents — and each call writes a record into the chat. A call's kind is fixed when it is
    placed; the camera toggles, the kind does not.

### Safety and moderation, in more detail (guideline 1.2)

The closed model above is the first line: content is confined to a family whose owner controls
admission, so there is no public surface to post to and no stranger to arrive from. On top of it:

- **Report.** Long-press a message → "Safety" → "Report…" (Report and Block sit one level down
  behind that row, not on the first page of the menu). A person can be reported without naming a
  message from Settings → Family → Manage Family, long-pressing a member row → "Safety" →
  "Report…". Four fixed reasons (spam, harassment, inappropriate, other); no free-text field. A
  message report freezes a copy of the message text when it is raised, so it survives an edit. The
  sheet states plainly, before sending, what the owner will be shown.
- **Block.** Same two menus. Any member may block any other, the owner included. Blocking hides
  that member's messages behind a "Hidden — blocked member" row that reveals on one tap, drops
  their one-to-one chat from the blocker's list, suppresses their notifications, and stops them
  being callable by the blocker. The blocked person is not told and is not otherwise restricted:
  the mechanism suppresses delivery to the person who blocked, rather than pretending to silence
  someone else's device.
- **The owner's inbox.** Settings → Family → Manage Family → Reports, drawn even when empty so an
  owner learns it exists before the first report. Each row carries the reason, both names, and the
  frozen message text where the reported message had any, with "Mark as handled". A report also
  pushes the owner. (See the checklist: a report about a caption-less photo currently reaches the
  inbox without an excerpt, and that is better fixed than described.)
- **When the owner is the problem.** The moderator is the family owner, not the developer. A
  report naming the owner is stored, never listed to them and raises no push for them; the report
  sheet instead shows the server's published support contact under "If the problem is the owner".
  On the default server that is `[SUPPORT_EMAIL]`, monitored by us, and we can act on the account
  directly on that server.
- **The owner's other levers.** Approve each join, rotate the invite code (the old one stops
  working immediately), cap the number of members, or set the policy to "Nobody", after which even
  a valid code is answered exactly as an invalid one; and remove a member from the family. An
  owner cannot delete another member's message — the remedy the protocol offers is removing the
  person, not editing the record.

### Privacy and data

The app is **not** end-to-end encrypted and does not claim to be. Messages, photos, files,
locations and board notes are stored unencrypted in the PostgreSQL database and filesystem of
whichever server the family chose. On the default server the operator holding that data is the
developer; a family that self-hosts shares nothing with the developer at all.

Transport is ordinary HTTPS to any public server. A server on the family's own LAN may be plain
HTTP over that LAN — see ATS below.

Call media runs over WebRTC (Opus, DTLS-SRTP) and travels directly between the two devices
wherever the two networks allow, in which case each device learns the other's IP address, as in
any peer-to-peer call. Where a direct path is impossible, a TURN relay operated alongside the
server forwards the encrypted stream; it carries the media but cannot read it. The chat server
itself carries only small JSON signalling frames.

An **account** is a username, a display name and a password: no email address, phone number or
real name is ever sent to or stored on the server, and a birthday, when set, is a day and a month
with no year, so no age can be derived from it. Separately, on the device: "Link to a Contact…"
in Manage Family lets you name a family member with a contact from this iPhone. The system picker
hands back only the one contact tapped — the app never prompts for Contacts access and never asks
for the address book — and that contact's name, phone numbers and email addresses are then kept in
this device's own storage, so iOS can label the call in CallKit and the Phone app's Recents.
Nothing about that link is sent to the server.

There are no ads, no analytics, no tracking, and no crash-reporting or attribution SDKs; the
privacy manifest declares no tracking and no tracking domains. The binary links exactly one
third-party library, Google's open-source WebRTC (BSD-3), used solely as the call media engine.

Third-party destinations, which we would rather name than have found: Apple's MapKit is asked for
tiles when a shared location is drawn (switchable off in Settings); a link preview is fetched by
the device directly from the linked site, with no cookies, credentials or referrer, the page fetch
stopping at 256 KB and the preview image at 4 MB (also switchable off); the server supplies the
STUN server used to set up a call, which discloses the device's public address to whoever operates
it, and a TURN relay where one is configured. The server also has an **optional assistant**, which
exists only if its operator turns it on; where it is on, an `@ai` mention sends recent family-chat
context — up to 30 days / 200 messages / 40,000 characters, with display names and timestamps — to
Azure OpenAI, and each member additionally gets a private assistant chat. Whether it is enabled on
`fc.nettrash.me` is on the checklist below, and the notes and App Privacy answers must match
whichever it is.

### Background, push and network / ATS

The only background modes declared are `audio` and `voip`, both for calls. The socket is dropped
when the app is backgrounded unless a call is in progress, and new messages then arrive as
ordinary APNs alert pushes. There is no background fetch, no silent push and no background
location. A foregrounded app is never pushed, because the socket has already delivered — so
notifications appear only when the app is backgrounded or closed.

On ATS the single relaxation is `NSAllowsLocalNetworking`; `NSAllowsArbitraryLoads` is not used.
Plain `http://` is therefore accepted only for loopback, RFC1918, link-local and `*.local`
addresses — a self-hosted box on a home network — and any public host must be `https`, which the
app enforces on the server-address screen with a message. One consequence worth knowing: pointing
the app at a server on your own LAN needs the same Local Network permission whose prompt text
speaks about calls.

If the demo server is ever unreachable during review, please contact `[SUPPORT_EMAIL]` and we will
restore it immediately.

> **Settle these before the notes above are pasted into App Store Connect.**
> - Provision on `fc.nettrash.me`: both demo accounts, the reviewer family, the seeded history
>   (photos, video, voice note, poll, location, board notes, one 1:1 chat), and fill
>   `[DEMO_USER]`, `[DEMO_PASS]`, `[DEMO_USER_2]`, `[DEMO_PASS_2]`, `[INVITE_CODE]`,
>   `[SUPPORT_EMAIL]`. Re-count the pasted block afterwards: real values are longer than the
>   placeholders and the field caps at 4,000 characters.
> - **Resolve `[ai]` on the live server** — this is still unknown, and two sentences above and the
>   App Privacy answers both depend on it. If it is enabled, Azure OpenAI is a data destination and
>   `ai_history` defaults to `true`, so an `@ai` mention ships other members' words off the server.
>   If it is disabled, cut the assistant sentence rather than leave a feature half-explained.
> - Confirm the rest of the live `config.toml`: `[calls] enabled` and `video_enabled` are on;
>   `[calls] turn_urls` / `turn_secret` (whether coturn is deployed decides whether the relay
>   sentence stays); `support_contact` is set to the monitored address — it is commented out by
>   default, and with it unset no escalation line is drawn anywhere in the app; `retention_days`
>   (default 100) if any retention claim is made anywhere; `[push] include_message_body` (default
>   `true`, meaning plaintext message text reaches APNs and can appear on a lock screen); and
>   `stun_urls`, which defaults to Google's public STUN server.
> - **Privacy manifest.** `NSPrivacyCollectedDataTypes` is currently an empty array — the binary
>   asserts it collects nothing — while the store build ships pointed at a developer-operated
>   server. Populate it (User Content, User ID; collected, App Functionality, not linked, not used
>   for tracking) so the compiled privacy report matches the App Privacy answers, or expect a
>   5.1.1 follow-up.
> - Two genuine 1.2 gaps to decide on rather than write around: there is no terms-of-service
>   acceptance in the sign-up flow; and a report about a caption-less photo reaches the owner's
>   inbox as a reason and two names with nothing else, because the frozen excerpt is
>   `messages.body`, which is empty for a photo, video, voice note or file. Freeze an attachment
>   placeholder (`[photo]`, `[video]`, `[voice note]`, `[file]`, `[location]` — the vocabulary
>   protocol.md already defines) alongside the body.
> - Decide the response-time undertaking for reports reaching `[SUPPORT_EMAIL]`; Apple's 1.2
>   checklist expects one, and there is no admin console behind it — acting means operating on the
>   server directly.
> - The submitted archive must be rebuilt from current source. The Release-nettrash product in
>   DerivedData is build 68 and carries the pre-calls camera and microphone strings with no
>   local-network key; the verbatim strings quoted above are the build-103 ones.
> - iPad: the target declares device family 1,2 but has no size-class adaptation, so an iPad
>   reviewer sees a stretched iPhone layout. Either adapt it or drop iPad from the target and the
>   screenshot set.

## Beta App Description (TestFlight → Test Information)

Family Connect is a private messenger for one family, and this beta is how we find out what breaks before it reaches the App Store.

On the Home Screen it is called Family, not Family Connect (iPhone or iPad, iOS 17 or later). The build is already pointed at our server, so there is nothing to set up: open it, register a username, a display name and a password — no email, no phone number — then create a family, which makes you its owner, or join one with an invite code. If you run your own Family Connect server, "Change server" on the sign-in screen points the app at it instead, including a plain http:// address on your own network.

Every family has one shared chat with everybody in it, pinned to the top of the list, plus private one-to-one chats between any two members.

In this build:

- Photos, videos, voice notes and files of any kind — up to ten attachments on one message, each up to 100 MB on our server. Several photos on one message are drawn as a stacked pile you tap to open. Photos are cached for offline viewing; video is streamed, so a long clip starts playing without waiting for the whole file, and no copy is kept on the phone.
- One-to-one voice and video calls. The audio and video go straight between the two devices wherever the network allows; the server passes the signalling. Where a direct path is blocked, the call connects only if a relay is configured on our server — which carries the stream encrypted, and cannot read it. On iPhone a call rings on the lock screen and lands in the Phone app's Recents, and a member can be linked to a card in Contacts so the call can be started from there; that link — the name and numbers with it — stays on your device and never reaches the server.
- Sharing where you are, once, as a map pin — never continuously, and never without you asking.
- Polls in the family chat, a family board of sticker notes anyone can move, reactions from the whole emoji picker, replies, and editing your own messages.
- Sharing in from other apps: send photos or files to Family from anywhere, choose the chat, and they arrive waiting in that composer — nothing is sent until you press Send.
- Read receipts in one-to-one chats, unread counts, typing indicators, and history kept on the device, so the app opens to your chats with no network at all.
- Report and Block, both behind the "Safety" row of a message's menu and on a member's row in the family list. The family owner is the moderator and has a Reports inbox.
- Nine languages: English, German, Spanish, French, Japanese, Russian, Serbian in both scripts, and Simplified Chinese.

Some of this cannot be tested alone. Real-time delivery, typing indicators, read receipts, notifications, calls and blocking all need a second person in the same family, and calls need two real devices.

The honest limits: no message search; no drag-and-drop into the composer, though Paste works; the iPad runs the iPhone layout full screen rather than a split view; calls are strictly one to one, so there are no group calls; and this is not end-to-end encryption — messages and files are stored on the server your family chose, which during the beta is ours.

If you were also sent the Mac build (macOS 14 or later), it is the same app with a sidebar and its own windows, but several things are iPhone-only: taking a photo inside a chat, the photo-library picker (the Mac gets a file panel instead), setting a profile picture, linking a member to a contact, and Leave Family. More importantly, a Mac is notified, and rings, only while the app is actually running: a Mac that is quit receives nothing. That is known — no need to spend time reproducing it.

There are no ads, no analytics and no tracking in the app, so the only way we learn about a problem is you telling us. If anything is confusing, slow or broken, use TestFlight's "Send Beta Feedback" — a screenshot helps — or email us. Thank you for testing.

*(3,876 of the 4,000-character limit. Pairs with Feedback Email — set it to the support address.)*

*(Notification permission is asked for in-app, after the tester is in a family and the first sync finishes — never on the sign-in screen. On the Mac it can be asked earlier, at launch, if a session is already stored.)*

## What to Test

Attachments and calls have the most moving parts — spend the time there. Groups are marked ALONE, TWO TESTERS (a second person in your family) or TWO DEVICES (two real phones, a one-to-one chat open between them). Much of it cannot be checked solo, so pair up first. There is no group call and no call from the family chat.

GETTING IN — TWO TESTERS. A fresh install opens on sign-in, with no server screen. Register, create a family, hand the code to your partner, and try all three join policies — on "Nobody" a correct code must be refused exactly like a wrong one.

SENDING THINGS — ALONE, last line needs a partner. Ten attachments of mixed kinds on one message. A long video: it should start playing rather than waiting for the whole file. A voice note — Cancel as well as Stop, then play and scrub it. A file the system knows nothing about. A location. Share ten photos in from Photos, pick a chat, cancel, then repeat and send. Edit a message and check the "edited" marker reaches a device that was closed.

CALLS — TWO DEVICES. Voice and video from a one-to-one chat, answered once from the lock screen. Mute, speaker, camera on/off, the flip, hanging up from each side. Then: answer and immediately background the app; decline; let it ring out; call someone already in a call. On shared Wi-Fi the first call asks for Local Network permission: accept it, then deny it on a spare device. Denying the camera must not end a video call, it carries on camera-off; denying the microphone is the one denial that does. No call button at all is a server switch, not your device.

LOSING THE NETWORK — ALONE. Airplane mode mid-send: a text bubble goes to Failed with tap-to-retry, not a silent outbox — but an attachment never becomes a bubble at all; the composer says "Couldn't send that" and hands the files back. Cold start offline: your cached chats open, not an error. Photos you opened before still open; video will not — streamed, never cached. Back online, watch it catch up.

SAFETY — TWO TESTERS. Report and Block sit under "Safety" in a message's long-press menu, and on a member's row in the family list. Blocking hides their family-chat messages behind "Hidden — blocked member" (one tap reveals), drops your direct chat, and stops calls both ways — but differently: yours to them is refused; theirs to you never reaches you at all, no ring and nothing in the Phone app, while on THEIR phone it rings out the full 45 seconds and ends as an ordinary missed call. Anything that reaches you is the leak — the best bug here; they are never told any of it. Report a message, then edit it: Reports must still show the original. Report the owner — they must never see it.

NOTIFICATIONS — TWO TESTERS. The prompt appears once you are in a family and the first sync finishes, never at launch or on sign-in. Quit the app fully and have your partner message you: the banner should arrive and open that chat from cold. Same for a call. Owners, app closed: a join request and a report should both push.

SHOULD NOT HAPPEN. No banner while you are looking at that chat; reading one clears its badge and its banners. The family chat never shows a double checkmark — read receipts are one-to-one only. Opening a chat or foregrounding the app must not by itself clear the unread count — the newest message has to be on screen, app frontmost. A birthday must never notify anybody. Nothing should ask for camera, microphone or location outside a call, a recording or a location share, and nothing should ever ask for Contacts. A voice call must never become a video call.

DELIBERATE, DON'T REPORT. No message search. No drag-and-drop (Paste works). Polls are family chat only. The iPad is the iPhone layout, full screen. Permission prompts stay English in every language.

DELETE ACCOUNT — LAST, TWO TESTERS, ON AN ACCOUNT YOU CAN LOSE. Settings from the chat list, bottom, beside Log Out: password, then immediate and irreversible. Afterwards, with your partner: your one-to-one chat is gone for them too; your family-chat messages, board notes and reactions stay as "Deleted account".

*(3,991 of the 4,000-character limit — anything added has to displace something.)*

*(Mac testers need three extra points, sent with the build rather than squeezed in here: it only banners and only rings while it is actually running; ⌘, and the Settings menu item do nothing, settings are inside the app; and they should be asked to confirm whether "Save a copy" in the attachment viewer really writes the file — the sandbox grant is `files.user-selected.read-only`, so a failure would be silent until the read-write entitlement lands.)*

## Pre-submission checklist

Two of the old blockers are genuinely retired: account deletion ships on both platforms (Settings → Delete Account, beside Log Out, password-confirmed, backed by `POST /api/v1/me/delete`), and the privacy and support pages exist and are linked in-app from Settings on iOS and macOS. What is left divides cleanly into work in the repo and work only nettrash can do, because it needs his server, his accounts or a decision that the code cannot settle.

Each item is tagged **[code]** (a change in this repository) or **[nettrash]** (his server, his App Store Connect account, or his call).

### Done — do not re-open

- [x] **[code]** In-app account deletion, guideline 5.1.1(v). Shared `DeleteAccountView` on both platforms — the button sits beside Log Out (`SettingsView.swift:402`, `MacSettingsView.swift:131`), not under an "Account" submenu, because that is where people look for it. Password required, immediate, irreversible. What the server does is a **scrub, not a row deletion** (`handlers_auth.rs:321`): the account, password hash, avatar, birthday, direct chats and assistant thread go; family-chat messages, board notes, reactions and the member's own family-chat attachments stay, attributed to "Deleted account". Every sentence in this file that describes deletion must say that — "removes the account and its messages from the server" is the one wording that will not survive a reviewer testing it.
- [x] **[code]** Privacy Policy and Support links ship in Settings on both platforms, pointing at `https://nettrash.me/appstore/familyconnect/privacy.html` and `.../support.html`. The pages themselves exist.
- [x] **[code]** Report and Block ship on iOS, macOS and Android, with the owner's report inbox and the four fixed reasons (`spam`, `harassment`, `inappropriate`, `other`). Together with member removal and the containment the product is built on — one family, membership an owner controls, no public feed, no discovery surface, no user directory, no way to reach a stranger — that is the guideline 1.2 answer. State it in that order everywhere it appears; never describe it as an absence.
- [x] **[code]** Localisation is complete in nine languages (`de`, `en`, `es`, `fr`, `ja`, `ru`, `sr`, `sr-Latn`, `zh-Hans`): 501 keys in `Localizable.xcstrings`, 491 of them translatable, nothing missing and nothing flagged for review. The five `INFOPLIST_KEY_NS*UsageDescription` permission strings are outside that count and are English-only — see the open item below.
- [x] **[code]** iPhone 6.9" and iPad 13" screenshot sets exist in `ios/docs/screenshots/`, six images each. Uploading them is a separate, **[nettrash]**, step.
- [x] **[code]** The Promotional Text, Description, Keywords, Notes for App Review and TestFlight sections above were rewritten in the same edit that produced this checklist, against the shipped feature set. Verifying that nothing stale survived elsewhere in the file is the first open item below, not a done one.

### Before the iOS archive is uploaded

- [x] **[code]** The stale text is gone, because this rewrite replaced the whole file. What it removed: the 5.1.1(v) blocker banner that used to open it ("v1 does not have one yet" — the feature ships, and the route is `POST /api/v1/me/delete`, never `DELETE /me`), the unticked "Implement account deletion" line that headed the old checklist, "text messages only", "Voice and video calls are planned", "third-party SDKs of any kind", and the deletion sentence claiming messages are removed with the account. If any of those phrases reappears in a later edit, it is contradicting either the binary or the position this listing takes.
- [ ] **[code]** Rebuild and re-archive from current source. The `Release-nettrash-iphoneos` product sitting in DerivedData is build 68 from 26 August and carries the pre-calls camera and microphone strings and no `NSLocalNetworkUsageDescription`. The tree is at `CURRENT_PROJECT_VERSION = 103` and the scheme's Build PostAction bumps it again on every build, so anything submitted must be 103 or later — otherwise the binary's own permission prompts will contradict the listing.
- [ ] **[code]** Add `INFOPLIST_KEY_ITSAppUsesNonExemptEncryption` to the target's build settings once nettrash has decided the answer, so it lands on both platforms and every upload stops parking in "Missing Compliance". The decision itself is his (see below); the plist key is ours.
- [x] **[code]** `PrivacyInfo.xcprivacy` is reconciled with the App Privacy answers and verified in a built bundle: ten data types, all Linked, none Tracking, all App Functionality, plus the two required-reason categories. Two defects were fixed in the same pass — the missing `NSPrivacyAccessedAPICategoryFileTimestamp` entry (an **ITMS-91053 rejection at upload**) and the User defaults reason, which declared the App Group code `CA92.1` for defaults the app does not share. The exact constants are listed under **App Privacy answers** below; if you change one, change both.
- [ ] **[code]** Fix what the owner's report inbox shows for reported media. Two defects, one screen. First, the excerpt is frozen from `messages.body` (`handlers_report.rs:132-152`), and a photo, video or voice note sent without a caption **has an empty body** by design (`handlers_chat.rs:487`) — so the frozen excerpt is the empty string, and "inappropriate" is very often exactly that message. Second, `docs/protocol.md:123-128` promises `message_attachments` on a report, `SELECT_REPORT` (`handlers_report.rs:73`) does not select it and `report_from_row` does not emit it, so nothing fills the hole either: the owner gets a reason word and two names. Implement `message_attachments` as the protocol already specifies it — kind and name, no dimensions and no coordinates — or amend `protocol.md` first and accept the gap knowingly. This is precisely the path a reviewer probing guideline 1.2 will walk: report a photo, then look at what the moderator sees.
- [ ] **[code]** Settle the resolve-a-report disagreement. `protocol.md:1562` promises `204` on a repeat resolve, explicitly because "a double tap and a retry after a timeout that actually worked are the same request twice and neither is an error"; `handlers_report.rs:299` returns `409 report_not_pending` and `report_flow.rs:92` locks it in. A literal double tap cannot reach it — both clients disable the button while the request is in flight and drop the row on success (`ReportInboxView.swift:98`, `MacFamilyView.swift:331`) — but an owner resolving from a second device, or retrying after a timeout that had actually worked, gets an error where the protocol promises success. Per the project rule, amend `protocol.md` first if the 409 is meant to stand.
- [ ] **[code]** Resolve whether `[ai]` is enabled on fc.nettrash.me and write the answer into this file. Nothing in the repo can tell you: the whole section is commented out in `server/config.example.toml:262` and the live config is not here. Three separate answers branch on it — the App Privacy disclosure, the privacy policy text, and the age-rating judgement about unfiltered model output in a shared chat — so this file must not be filed with the question open. Whichever way it lands, the sentence that ships has to be true of the server as configured on the day of submission.
- [ ] **[code]** Correct the stale sentences in the authoritative doc before any listing copy quotes it: `protocol.md:1315` ("one to one, voice only, and peer to peer") and its counterpart in `CallManager.swift:6` ("One call at a time, one to one, audio only, peer to peer"), both contradicted by `protocol.md:1481-1509` and the shipping code; and `protocol.md:1030-1032`, which reads as though dedup is the only reason an attachment survives deletion when in fact the member's own family-chat uploads always do.
- [ ] **[code]** Fix the two in-repo files that now make a false data-handling claim: `server/config.example.toml:255` and `server/src/ai.rs:3-8` both say nothing anybody else wrote is ever sent anywhere. That is true of a member's private assistant thread and false of the family chat: with `ai_history` defaulting to true, an `@ai` mention there sends other members' words to Azure OpenAI. Both comments are load-bearing — an operator reads them to decide whether to switch the section on.
- [ ] **[code]** Decide and act on the terms-of-service gap. There is no EULA or terms acceptance anywhere in the sign-up flow, and Apple names agreed terms with no tolerance for objectionable content as an explicit 1.2 requirement, alongside the reporting and blocking that already ship. Either build the acceptance step or record the decision not to, with reasoning, here.
- [ ] **[code]** Decide whether to localise the five usage descriptions. They are English literals in `project.pbxproj:546-550`, and the nine `.lproj` directories carry only `AppIntentVocabulary.plist` — no `InfoPlist.strings` anywhere. A Russian or Japanese user gets an English camera prompt in an otherwise fully localised app. Not a rejection risk; a visible seam.
- [ ] **[code]** Bring README.md into line, or at least stop it being load-bearing. It describes a text-only client on iOS and Android with no mention of macOS at all (`README.md:6`), documents no macOS archive procedure, and its archive snippet targets `generic/platform=iOS` only (`:69`). CHANGELOG is nine batches behind and holds one v0.1.0 entry; neither is usable as a source for listing or "what's new" copy.

### Only nettrash can do these

- [ ] **[nettrash]** Provision the reviewer family and two demo accounts on fc.nettrash.me by hand, seed the family chat and one 1:1 chat with history, and fill `[DEMO_USER]`, `[DEMO_PASS]`, `[DEMO_USER_2]`, `[DEMO_PASS_2]` and `[INVITE_CODE]`. The seeding scripts in the repo (`server/scripts/seed-{store-screenshots,album-uitest,scroll-uitest}.sh`) all build a local `127.0.0.1:8091` fixture for screenshots and UI tests; none of them touches the review server.
- [ ] **[nettrash]** Set `[server] support_contact` in the live config and fill `[SUPPORT_EMAIL]`. It ships commented out at `config.example.toml:24` and is therefore unset by default, and it is the only escalation path the app draws for a report about the family owner — which is the answer Apple will want when it asks who moderates the moderator. Note that the app surfaces it in exactly one place, the report sheet, so if it is unset that sheet has no escalation line at all.
- [ ] **[nettrash]** Read the live server's `config.toml` and write the answers into the review notes rather than assuming defaults. Six values matter and none of them is in this repo: is `[ai]` enabled (and `ai_history` left at its default true) — the one that gates three other answers; are `[calls] enabled` and `video_enabled` on, without which the reviewer never sees a call button; what is `retention_days` actually set to (the shipped default of 100 permanently deletes messages and their media server-side); is `stun_urls` still Google's public STUN (`config.example.toml:214`); do `turn_urls` and `turn_secret` actually point at the coturn 4.6.1 already deployed alongside the server, since an unconfigured `turn_urls` is empty by default and calls that cannot connect directly then simply fail; is `[push] include_message_body` still true (`:135`), which puts plaintext message bodies on lock screens via APNs and FCM. Confirm too that APNs credentials are installed, or push silently logs and never arrives.
- [ ] **[nettrash]** In App Store Connect: paste the Support URL and Privacy Policy URL, enter the owner demo credentials in App Review Information, and upload the two screenshot sets.
- [ ] **[nettrash]** Answer the export-compliance question once, deliberately, and record the reasoning in this file. The app implements no cryptography of its own, but `WebRTC.framework` (stasel/WebRTC 151.0.0) ships BoringSSL for DTLS-SRTP, so the "uses only Apple's built-in encryption" exemption other apps in this workspace rely on does not apply. The likely answer is still exempt as mass-market, but whether a BIS self-classification report is owed is a legal question the repo cannot settle.
- [ ] **[nettrash]** Complete the age-rating questionnaire. The honest inputs are: user-generated content yes, person-to-person messaging with media yes, both bounded to a single family whose membership an owner controls, with in-app reporting, blocking, an owner report inbox and member removal. Web access is restricted: there is no in-app browser — tapped links hand off to the system browser through `openURL` — though the app does fetch metadata from linked hosts for previews. No gambling, contests, purchases or ads. No parental controls and no age verification, since birthdays carry no year. The open judgement is the assistant: if `[ai]` is enabled it writes model output into the shared family chat, and whether that moves the rating is a policy call that cannot be made until the item above resolves.
- [ ] **[nettrash]** Decide the iPad question. `TARGETED_DEVICE_FAMILY` is "1,2" but there is no size-class adaptation anywhere — the iPad is a stretched iPhone. Either adapt the layout, or drop to "1" and drop the iPad screenshot slot. Do not claim an iPad-optimised layout either way.
- [ ] **[nettrash]** Decide which name leads the App Store record. The app's on-device display name is "Family"; the product and this document are called "Family Connect", and the Siri vocabulary teaches "Call Anna on Family".
- [ ] **[nettrash]** Decide whether open registration on fc.nettrash.me is acceptable at launch. There is no invite gate, no email, no allowlist and no config switch, so any App Store customer can create an account and a family on his box. Family isolation bounds the exposure — a new account sees nobody and reaches nobody — but storage (100 MB per attachment), server load and his own position as the operator of record are real. A `[registration]` switch on the server is the obvious follow-up.

### App Privacy answers

"Data Not Collected" is not defensible for this build and must not be filed. The store build is compiled with `FC_DEFAULT_SERVER_URL = https://fc.nettrash.me`, so a reviewer — and every customer who does not change the server — uploads their content to a server the developer administers, keyed to a persistent account, where message bodies are stored in readable form. Everything below is **collected, Linked to the User, used for App Functionality only, and never for tracking**.

`ios/FamilyConnect/PrivacyInfo.xcprivacy` now declares exactly this set and was verified in a built
bundle. **The manifest and the console answers must stay identical** — a manifest that disagrees
with what you type into App Store Connect is a self-inflicted discrepancy, and worse than the empty
array it replaced. The constant beside each bucket below is the one in the manifest; **[nettrash]**
files the form, and each bucket's wording should be confirmed against the console's current
labels rather than against this list.

- **User Content.** Message bodies → `NSPrivacyCollectedDataTypeEmailsOrTextMessages` (plaintext
  `TEXT` in PostgreSQL). Photos and videos → `NSPrivacyCollectedDataTypePhotosorVideos` — note
  Apple's irregular casing, the "or" is lowercase and capitalising it fails validation. Voice
  notes → `NSPrivacyCollectedDataTypeAudioData`. Arbitrary files, board notes, polls and votes,
  reactions, and the excerpt frozen onto a report → `NSPrivacyCollectedDataTypeOtherUserContent`.
- **Location → `NSPrivacyCollectedDataTypePreciseLocation`.** The one that is easiest to get wrong
  in the optimistic direction. A shared location is one-shot and user-initiated — there is no
  continuous or background location anywhere in the app — but the coordinate that reaches the
  server is an unrounded WGS-84 double, orders of magnitude past Apple's three-decimal-place line.
  Precise, not Coarse.
- **Identifiers.** Username and account id → `NSPrivacyCollectedDataTypeUserID`. Push tokens (APNs,
  plus the separate PushKit VoIP token on iOS) → `NSPrivacyCollectedDataTypeDeviceID`. That second
  one is a **judgement, not something the schema compels**: it is a per-device identifier held on
  the server against an account, and Device ID is the conservative reading. It is not an
  advertising identifier — the app has none. Recorded here so the answer can be defended if asked.
- **Contact Info → `NSPrivacyCollectedDataTypeName`**, for the display name, on the ground that it
  is free text and families put first names in it. No email address and no phone number is ever
  required, and none is ever sent to or stored on the server; nothing verifies identity.
- **Other Data → `NSPrivacyCollectedDataTypeOtherDataTypes`**, for the birthday: a day and a month,
  never a year. **Not** Sensitive Info — that category is racial, health, religious, political and
  biometric data, and ticking it would describe a data flow this app does not have. Over-ticking is
  not a free way to be careful; the label is a public statement about the app.
- **Usage Data → `NSPrivacyCollectedDataTypeProductInteraction`**, for the read markers in
  `chat_reads` that drive read receipts and unread counts, and the per-member figures the
  Statistics screen derives from them. A behavioural record of how someone uses the app, not
  content, and not to be folded into User Content. Typing indicators are **not** collected — they
  are relayed live over the socket and never stored.
- **Not collected: Contacts.** The contact-link feature persists the picked card's identifier,
  name, phone numbers and email addresses (`ContactLinks.swift:35-43`), but in this app's own
  `UserDefaults` only — none of it is ever transmitted, the picker runs out of process, and the app
  holds no Contacts permission at all. One nuance worth stating in the privacy policy rather than
  the label: a linked number does reach the **system call log**, because `CallKitController` sets
  `includesCallsInRecents = true` and reports calls with a phone-number handle. That is on-device
  and not collection by the developer, but it is where the number visibly surfaces.
- **Also not collected:** email address, phone number, physical address, health, fitness,
  financial, browsing history, search history, purchases, advertising data, crash and performance
  data. There is no analytics, no crash reporting, no attribution SDK and no IDFA anywhere in the
  build; the only third-party binary is WebRTC, which carries call media.

**Required-reason APIs** are declared in the same manifest, and one of them was previously wrong:

- `NSPrivacyAccessedAPICategoryUserDefaults` → **`54BD.1`**, defaults reachable only by this app
  itself. It used to declare `CA92.1`, which is the **App Group** reason — the app reads and writes
  `UserDefaults.standard` and shares no defaults suite with the Share Extension, which uses the App
  Group container for files only. `CA92.1` described sharing that does not happen.
- `NSPrivacyAccessedAPICategoryFileTimestamp` → **`DDA9.1`** (files in the app's own container or
  temporary directory) and **`C617.1`** (a file the person selected through a picker or the macOS
  open panel). Both paths reach the single `FileManager.attributesOfItem(atPath:)` call in
  `MediaPrep.fileSize(of:)`, and the category is required for that API even though only `.size` is
  read. This entry was **missing entirely**, which is an **ITMS-91053 rejection at upload**, before
  a human ever opens the build. `3B52.1` is deliberately absent: it is the reason for a third-party
  SDK acting on the app's behalf.

Two disclosures sit alongside the label rather than inside it, and the privacy policy has to carry them.

First, the assistant. **If** `[ai]` is enabled on the live server, a single `@ai` mention in the family chat ships up to 30 days, 200 messages or 40,000 characters of the family transcript — other members' words, with display names and UTC timestamps — to the operator's Azure OpenAI resource, because `families.ai_history` defaults to true, only the owner can turn it off, and there is no in-app consent step. If it is disabled, say so explicitly in the review notes and drop the disclosure. Do not file either version until the open item above has resolved which it is.

Second, third-party destinations. Two are reached by default and cannot be switched off from the app: Apple and Google receive plaintext message bodies inside push payloads while `include_message_body` is true, and Google's public STUN server is the shipped default, so placing a call discloses the device's public address to it. Two more are on by default but are genuine user toggles in Settings (`SettingsView.swift:355/359`, `MacSettingsView.swift:97/101`): map previews fetch MapKit tiles from Apple, and link previews fetch metadata from whatever host a member linked to.

### Held until macOS ships

macOS is one platform on one app record, sharing the bundle id `me.nettrash.FamilyConnect` — never a second record and never a second bundle id. It is held for the first release, and these must land before it is added, so that nobody adds the platform by accident during the iOS submission:

- [ ] **[code]** Fix the entitlement key spelling. `FamilyConnect-macOS.entitlements:25` uses the iOS `aps-environment`; macOS needs `com.apple.developer.aps-environment`. The wrong key is filtered out at signing rather than rejected, so nothing fails loudly — a quit Mac simply receives nothing at all, which is exactly the case the Mac review notes would otherwise describe as working.
- [ ] **[code]** Fix or remove "Save a copy" in the Mac attachment viewer. The sandbox grants only `files.user-selected.read-only`, there is no read-write entitlement anywhere, and the copy is wrapped in `try?` so a failure is silent. Verify on a signed build before any Mac screenshot or review note shows that button.
- [ ] **[code]** Set `ENABLE_HARDENED_RUNTIME`. It is absent from every configuration — optional for the Mac App Store, mandatory for Developer ID notarisation.
- [ ] **[code]** Produce Mac screenshots. `ios/docs/screenshots/mac/` does not exist, and the UI-test harness is iOS-only, so `ios/scripts/capture-mac-screenshot.sh` is the route.
- [ ] **[nettrash]** Confirm in the console which App Store Connect fields are record-level (App Privacy, age rating) and which are per-platform (description, keywords, screenshots, what's new, submission), rather than assuming — an answer given for iOS may bind macOS.
- [ ] **[nettrash]** When the Mac description is written, it must drop CallKit, PushKit, Siri, in-chat camera capture, the photo-library picker, contact linking, *setting* a profile picture, and Leave Family. None of those exist on macOS, and a Mac that is not running is never woken for a call by design. Note the distinction on avatars: `InitialsAvatar` resolves through `AvatarStore` on both platforms and the Mac draws real server avatars (`MacSettingsView.swift:36`, `MacChatView.swift:281`, `MacFamilyView.swift:407`) — what the Mac's Profile section lacks is the picker to upload one, offering only Birthday and Change Password.

## Confirm before publishing

None of the following can be settled from this repository — each depends on the live server, a decision, or a device. Every one of them is load-bearing for a sentence above.

### Affecting the store copy

*Six things in the copy above depend on facts outside the repository and must be confirmed before it is published, not assumed:*

- *Calls: the deployed server must have `[calls] enabled` and `video_enabled` true, or the calls bullet describes buttons the customer will never see. Both default to true server-side, but the live config on fc.nettrash.me was not read. Check `turn_urls` / `turn_secret` in the same section while you are there — the relay sentence is written to hold whether or not one is configured, but if coturn is wired in, the media of a NAT-blocked call passes through our box and the privacy policy has to say so.*
- *Push: the deployed server must have `[push.apns]` configured with a key valid for this bundle id, or the notifications bullet and the lock-screen ringing describe nothing the customer will see. That is also why the closed-app notification sentence is scoped to the default server — a family's own server cannot push to the App Store build.*
- *A report about the owner escalates to the server's `support_contact`, which is commented out by default. If it is unset on fc.nettrash.me the app draws no escalation line at all, and that clause promises a path the customer cannot see. Set it before publishing.*
- *Contact-card and Favourites calling is iOS-only and correct as written. Siri is deliberately not mentioned: whether a call intent reaches the app on a fresh device without a Siri authorisation step has not been verified on hardware.*
- *The AI assistant is deliberately absent from this copy. The server ships it off by default and whether the deployed server has it configured is unknown here. If it is on, it needs a sentence of its own — and the privacy policy needs one about what an `@ai` mention in the family chat sends onward, and to whom.*
- *No retention figure appears above on purpose: `retention_days` defaults to 100 days but the live value was not checked. iPad is not claimed either — but not claiming it does not opt out of it, because the target ships device family 1,2 and the App Store page will advertise iPad regardless. Either drop iPad from the target for this submission or open the app on one first; there is no size-class-adaptive layout, so an iPad shows a stretched iPhone screen.*

### Affecting the TestFlight texts

- **The upload must come from the FamilyConnect-nettrash scheme (Release-nettrash).** Only that configuration compiles in `FC_DEFAULT_SERVER_URL = https://fc.nettrash.me`; a plain FamilyConnect/Release archive ships it empty and opens on the server screen, which falsifies "the build is already pointed at our server" and the whole GETTING IN group. Both schemes bumped the build number to 103, so the number does not tell you which one it was.
- **Calls must be on at fc.nettrash.me.** Both texts promise voice and video; the buttons are gated on `calls_enabled` and `video_calls_enabled` from `GET /me`. If either is off on the live box, the Calls group is untestable and the promise is false.
- **TURN wiring — the relay sentence depends on it (issue #18).** coturn 4.6.1 is deployed and healthy on the box, but `handlers_call.rs` only emits a TURN entry when `[calls] turn_urls` is non-empty, and a `turn_secret` that does not byte-match coturn's `static-auth-secret` fails silently at allocate time. Check both. If `turn_urls` is empty the relay sentence is FALSE — with no relay a call that cannot connect directly simply fails (`config.rs`, `turn_urls`). Either wire TURN and place one real relayed call, or delete "Where a direct path is blocked, our relay carries the stream instead" from the beta description. Never restore an unconditional "the call never touches the server".
- **APNs environment (issue #18) — this decides whether the NOTIFICATIONS group works at all.** If `[push.apns] environment` is "sandbox", every alert push to a TestFlight build returns BadDeviceToken, the server treats that as permanently dead and deletes the device row, and PushRegistrar only re-POSTs when the OS token changes — so the tester never gets another push without reinstalling. Confirm `environment = "production"` and `bundle_id = "me.nettrash.FamilyConnect"`, and send one real alert push and one VoIP push to a TestFlight build before inviting anybody.
- **Mac push (issue #7).** "A Mac that is quit receives nothing" holds only while `FamilyConnect-macOS.entitlements` still uses the iOS spelling `aps-environment`; macOS wants `com.apple.developer.aps-environment`, so the key is stripped at signing and the shipped Mac app carries no push entitlement. If the rename lands before the build is cut, delete that sentence and the first Mac tester point — otherwise the text tells testers to ignore a working feature. (The "rings" half stays true either way: there is no PushKit on macOS.)
- **The AI assistant is not mentioned anywhere above, on purpose.** `[ai]` defaults to disabled and the clients hide `@ai` when the server reports no assistant, and whether it is configured on the live box is unknown here. Check it; if it is on, both texts need a paragraph — including that an `@ai` mention in the family chat sends other members' words to the operator's Azure OpenAI, and that `ai_history` defaults to on.
- **Retention.** The server sweeps messages *and their photos, videos, voice notes and files* older than `retention_days` (default 100). Check the live value; if it is not 0, testers should be told their history has an expiry date, because nothing in the app says so.
- **Siri and Apple Watch.** The Contacts sentence deliberately claims only the contact card. No code requests Siri authorisation, and whether the call intents reach the app unprompted is unverified. Watch answering is standard CallKit behaviour but nothing here covers it — no entitlement, no test — and the shipping build logs and discards every `CXTransaction` error with no completion, so a refused `CXStartCallAction` leaves the in-app Hang Up dead. Verify on a real device and a paired Watch before either claim goes into the text.
- **Voice/video wording.** `docs/protocol.md:1315` and `CallManager.swift:6` still say "voice only"/"audio only". Both are stale against `protocol.md:1481` and the shipping code; correct them in the authoritative doc before anything quotes it.
