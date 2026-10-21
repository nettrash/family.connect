# The message row and the wide screens (issue #106)

**Written 2026-10-20** against `v1.3` (`d6f7a94`), in answer to issue #106: *"We should think and make some changes to
make message input row with buttons around more useful than now. Chain of buttons from the left side reduce useful
space. On iPad we're using not full available space for chat."* The owner also asked for a UX/UI check of the whole
app, the message row first, and a review of iPadOS.

**Status: DECIDED 2026-10-20.** The owner accepted all four recommendations of section 4 — `@ai` leaves the row (the
`@` list and the paperclip menu), stickers inside the field always, the 1024 column with 560 bubbles on all six
clients, hardware Return sends on iPad and Android — and decision 6 stays as recommended (the paperclip keeps "Record
Voice Message"). `docs/protocol.md` ("Mentioning a member") was amended first. This supersedes the iPhone row of
`docs/audio-video-messages-2026-10-04.md` S8.1, S8.2's 560-pt column, and the `@ai` button of
`docs/attachment-menu-2026-10-07.md`.

How it was measured: every control, size and condition of the row was read from the code on all six clients (file
and line in the notes behind this doc), then photographed on fresh builds of the current branch against the store
fixture ("The Harpers", with a four-sticker pack and an assistant, so every button shows): iPhone 17 and iPad Pro 13"
simulators (iOS 27.0), the Mac app at 1000×680 and 1280×800, an Android 16 phone (1080×2400) and tablet (2560×1600)
emulator, and Chrome 155 at 1440, 1920, 1024, 390 and 375 px. Windows cannot run on the Mac; its row is measured from
the code only. The iPhone screenshots can be retaken with `FamilyConnectUITests/ComposerAuditUITests` (new, soft,
env-gated, see its header).

## 1. What is wrong today

The row is the same on every client, left to right: **paperclip · stickers · `@ai` (the paintbrush in the assistant
chat) · the field, with the video-message button inside it · the microphone or Send**. Three buttons stand before
the field; the field gets what is left.

| Client, narrowest common width | Text run in the field, family chat, empty | …while typing |
|---|---|---|
| iPhone, 393 pt | **141 pt** (36 % of the width) | 213 pt — the sticker button and the video button leave, the row jumps 72 pt |
| Android phone, 412 dp | **120 dp**; 68 dp on a 360-dp phone | ≈170 dp — only the video button leaves |
| Web, 393 px | the row needs 408 px: **the microphone is clipped ~31 px off-screen** (no `min-width: 0` on the textarea) | — |
| Android, 600-dp two-pane | **35 dp** — the video button alone needs 68 | — |
| Mac, 420-pt minimum window | 216 pt | 216 pt |
| Windows | W − 545 epx: 255 at 800 wide, **negative below ~560** (no minimum window) | — |

1. **The left chain starves the field on every phone** (table above). Nothing folds by width; under iOS Dynamic Type
   AX3 the controls alone are wider than the row and the field is 0 pt.
2. **The row moves under the thumb.** On iPhone/iPad the sticker button disappears at the first character and the
   `@ai` button slides into its place; Android, the web and the Mac keep it. The same family member meets two rules.
3. **Targets are under 44.** iOS 36 pt, Mac 24 pt, web ≈36×38 px, Windows ≈32 epx with 0 spacing; only the
   microphone/Send slot (#79) got 44.
4. **The iPad uses half the pane.** #44 capped the whole thread container — composer included — at 560 pt, to stop a
   240-pt photo floating in a 960-pt balloon. On a 13" iPad in landscape that leaves **496 pt of the 1056-pt detail
   empty (47 %)**, 816 pt (59 %) with the sidebar hidden; the composer is squeezed to 360 pt while its bar spans the
   pane. Android copied the rule (`READABLE_COLUMN` 560 dp): 379 dp of a 1280-dp tablet empty.
5. **The desktops fail the other way.** Mac, web and Windows cap nothing: at 1920 wide the field is ~1400 px long
   while bubbles stop at 460/600/600 — 60–70 % of every row empty and the field far from the text it answers.
6. **The web at phone and tablet widths**: the chat list stays as a 35 %-high band above the conversation (no back,
   about three bubbles visible), and with the thread or polls panel open at 721–1024 px the conversation is squeezed
   to 154 px (834-px iPad portrait) and its composer is clipped.
7. **Thread composers are a second product**: a bare 22-pt Send glyph on iOS/Mac, an outlined field and grey Send on
   Android, a text "Send" on the web and Windows (lit while blank on Windows), Return sends in the iOS thread sheet
   but not in the iOS main composer, and Esc on the web closes the thread and loses the draft.
8. **Return/Enter means five things**: newline (iOS main), send (iOS thread, Mac, web, Windows), Ctrl/⌘+Enter
   (Android). An iPad with a keyboard meets two meanings in one app.
9. **Smaller**: the web field has no visible placeholder; the Mac's paperclip, ✨ and 🎨 have no accessibility label;
   Windows draws ✨/🎨 as colour-emoji text; the attach menu repeats "Record Voice Message" beside a microphone; the
   video button is a filled disc beside the mic on the Mac but an outline glyph in the field elsewhere, and never
   appears on Windows; the strips above the row stack in four different orders.

## 2. The proposal

### 2.1 One row, three controls

```
iPhone 393, family chat, empty
┌──────────────────────────────────────────────┐
│ 📎  ( Message                   ☺   🎥 )  🎤 │
└──────────────────────────────────────────────┘
typing
│ 📎  ( Sunday works for us, we'll  ☺   )  ➤  │
│       bring the cake                         │
assistant chat
│ 📎  ( Message               ☺   🖌 )      ➤  │
thread
│     ( Reply in thread             ☺ )      ➤  │
```

- **Left of the field: the paperclip, alone.** Its menu is unchanged (#78), with one item added in the family chat
  (below).
- **Inside the field, at its trailing edge: stickers (☺), always** — empty or typing, so nothing moves under the
  thumb and a sticker is one tap away wherever a message can be written, as `protocol.md` requires. This is where
  WhatsApp, Telegram and Signal put it.
- **The video-message button stays where #79 put it** (inside the field, trailing, only while the field is empty and
  the slot is a microphone). An empty field needs room only for its placeholder.
- **The assistant chat keeps its paintbrush as a button**, inside the field next to the stickers (that chat has no
  microphone and no video button, so there is room). The 2026-10-07 decision that the paintbrush is a button and not
  a menu line stands.
- **`@ai` leaves the row.** It is reached two ways: typing `@` lists **the assistant first**, above the members; and
  the paperclip menu gains **"Ask the Assistant"** as the first item of its last group in the family chat (it inserts
  `@ai ` as the button does today). This is the one earlier decision the proposal revisits (section 4).
- **The microphone/Send slot is unchanged** (#79, S1.3), in every chat.
- **Every control has a 44-pt / 48-dp / 44-px / 44-epx target**; glyphs stay their current size.
- **Dynamic Type / narrow windows**: when the three controls plus a 120-pt text run do not fit (iOS AX3 and above, a
  web window under ~340 px), the field takes its own line above the controls rather than shrinking to nothing
  (`ViewThatFits` on iOS; a CSS wrap on the web).
- **Threads get the same capsule and the same Send disc**, Send disabled while blank, sticker glyph in the field, no
  paperclip and no microphone (threads have neither today; unchanged).

**After the change**, worst case (family chat, every capability on):

| | Today, empty | Proposal, empty | Today, typing | Proposal, typing |
|---|---|---|---|---|
| iPhone 393 pt | 141 | **201** | 213 | **233** |
| iPhone, direct chat | 185 | 201 | 257 | 233 |
| Android 412 dp | 120 | **184** | ≈170 | **232** |
| Web 393 px | clipped | **161** | clipped | **205** |
| Mac 420 pt | 216 | **248** | 216 | **280** |

Arithmetic (iPhone): 393 − 24 bar insets = 369; − paperclip 36 − 8 − 8 − slot 36 = 281-pt capsule; − 12 leading
padding − sticker glyph 28 − 8 trailing padding = 233 while typing; − video button 28 − 4 = 201 while empty. The
44-pt targets extend 4 pt into the gaps on either side of the 36-pt glyph frames, so they cost no width.

The direct chat on iPhone is the one case that gets narrower (24 pt) while typing: today it hides the sticker button
while typing; the proposal keeps it. That is the price of a row that never moves and a sticker that is always there.

### 2.2 Wide screens: cap the column, not the pane, and cap the bubble

One rule on all six clients:

- **The conversation column** (messages AND composer) is `min(pane width − 2×16, 1024)` and centred. The composer is
  exactly as wide as the messages above it.
- **A bubble** is at most `min(column − 48, 560)` — the iPhone's bubble is unchanged (its limit today is the 48-pt
  spacer), and 560 keeps a line of text readable on a wide screen.
- **Media in a bubble**: 320 on a wide window, 240 on a narrow one, as today.

| Window | Today: column / empty | Proposal: column / empty |
|---|---|---|
| iPad 13" landscape, sidebar shown (detail 1056) | 560 / 47 % | **1024 / 3 %** |
| iPad 13" landscape, sidebar hidden (1376) | 560 / 59 % | 1024 / 26 % |
| iPad 11" landscape (874) | 560 / 36 % | 842 / 4 % |
| Android tablet 1280 dp (939) | 560 / 40 % | 907 / 3 % |
| Mac 1280×800 (1000) | none; field 796 | 968; field 880 |
| Web / Windows 1920 (≈1620) | none; field ≈1400 | 1024, centred; field ≈ 936 |

#44's own concern — a phone-sized card inside a pane-wide balloon — stays solved, because the balloon is capped at
560, not by the column. The album balloon's grey to the right of a 320-pt card is 240 pt, as today's 236.

### 2.3 The keyboard

- **A hardware keyboard sends with Return/Enter and breaks the line with Shift+Return** — on the iPad and on Android,
  as the Mac, the web and Windows already do. ⌘↩ (iPad) and Ctrl/⌘+Enter (Android) keep working.
- **An on-screen keyboard's Return is a newline on phones and tablets** (unchanged).
- **Esc in the web thread panel no longer closes it while the field holds a draft**; it clears focus first.

### 2.4 What stacks above the row

One order on every client, top to bottom, the nearest to the field being what is being written: notices (assistant
consent, picture notice, `/draw` hint) · voice messages not sent · reply or edit banner · staged attachments ·
mention suggestions · **the row**. The busy/failure line ("Sending…", errors) sits directly above the row on every
client (Windows draws it below the card today).

As built (2026-10-20): the busy/failure line includes "Preparing…" and "Finding your location…" (and the web's
text-too-long notice); a recording's own hints (too quiet, how to lock — the Apple voice line) sit between the mention
suggestions and that line, since they are about the microphone in the row. "… is typing" is about somebody else and
stays above the composer. Threads have only suggestions and the failure line, already in this order. Windows draws the
busy lines inside the card, so they stay directly above the row while staged items or suggestions show. The order is
pinned by tests on Android (`InputBarStackOrderTest`) and the web (`what_stacks_above_the_row_is_in_the_one_order`).

## 3. Fixes that need no decision

These are defects whatever the row becomes, and can land first:

| | Client | Fix |
|---|---|---|
| 1 | Web | `min-width: 0` on the textarea: the microphone is clipped on every phone today; a visible "Message" / "Reply in thread" placeholder |
| 2 | Web | Below the two-pane breakpoint show one surface at a time (list, or conversation with a back control); at 721–1024 px open the thread/polls panel OVER the conversation instead of beside it |
| 3 | Android | Two-pane only from a pane width that leaves the conversation ≥ 400 dp (today 600-dp windows leave 259 dp and a 35-dp field) |
| 4 | Android | Landscape with the keyboard up clips the composer to a sliver (the list collapses and the row loses height) |
| 5 | Android | **Crash**: the family chat dies on open when a shared location is on screen and Map previews is on (`setMapColorScheme not supported in Lite mode`, maps-compose 9.0.1 / play-services-maps 20.1.0 on an API 36 Play image). A fix is in `Attachments.kt` (uncommitted), awaiting its emulator check |
| 6 | iOS, Mac | **Message times on a 12-hour dial with AM/PM dropped** (`.hour(.twoDigits(amPM: .omitted))`, `MessageBubbleView.swift:423`, `MacMessageRow.swift:302`): 20:58 reads 08:58 beside a 24-hour status bar. Use the locale's own short time |
| 7 | Mac | Accessibility labels on the paperclip, ✨ and 🎨 (tooltips only today) |
| 8 | Windows | A minimum window width (or the chat list collapsing) so the composer never goes negative; ✨/🎨 as line glyphs, not colour emoji |
| 9 | All | Thread composers: the main capsule and Send disc (2.1), Send disabled while blank |

## 4. What the owner decides

**Protocol (amended first if agreed)**

1. *May typing `@` in the family chat list the assistant, first, when the server has one?* `protocol.md`
   ("Mentioning a member") says the list is "every current member but the reader, the blocked and the assistant".
   **Recommended: yes** — it is the natural way to find `@ai` once its button leaves the row, and the rule for
   offering it at all (assistant + named processor) is unchanged.

**Earlier choices to revisit**

2. *May the `@ai` button leave the row* (into the `@` list and the paperclip menu), against the 2026-10-07 decision
   that it is its own button? **Recommended: yes** — it is the button that costs the field most and the one used
   least; the paintbrush and the stickers stay buttons.
3. *Should the sticker button stay while typing?* iOS hides it (S8.1); four clients keep it. **Recommended: keep it,
   inside the field** — the row stops moving under the thumb.
4. *May the iPad (and the Android tablet) use the column rule of 2.2 instead of #44's 560-pt container?*
   **Recommended: yes.** Alternative: the full pane with no column cap at all (Apple Messages on iPad), bubbles capped
   at 560 — simpler, but on a 13" landscape iPad a reply sits 900 pt from the message it answers.

**Design-doc conventions to change** (no earlier owner decision; noted in the docs when built)

5. Hardware Return sends on iPad and Android (2.3).
6. Drop "Record Voice Message" from the paperclip menu while the slot IS a microphone? **Recommended: no** — it is
   the worded, screen-reader-friendly door to the same action, and the menu is the place people look for words.

## 5. Plan, once decided

1. **Shared first**: a pure `ComposerRow` rule per client (which glyphs sit in the field, for which chat, state and
   width), with the same vectors on four ports in the manner of `AttachMenu`; the column/bubble rule as two
   constants per client.
2. **Fixes of section 3** (no decision needed): web 1–2, Android 3–5, iOS/Mac 6–7, Windows 8.
3. **The row**, per client: iOS/iPad + Mac (`ConversationView` composer controls, `MacConversationView` composer,
   `ThreadView`; never adding modifiers to `ConversationView.body`'s chain), Android (`ChatScreen` input bar,
   `ThreadScreen`), web (`views/composer.rs`, `thread_panel.rs`, `styles.css`), Windows (`ChatsView.xaml` +
   code-behind; unrun here).
4. **The wide rule**, per client, then the screenshots retaken on every device class (iPad 13" and 11", Android
   tablet, Mac 1280, web 1440/1920/390).

| Client | Effort | Risk | Tests to update |
|---|---|---|---|
| iOS/iPad + Mac | 2 days | medium (scroll anchoring around the column, #43/#44 history) | UI tests that find the composer by "Attach a photo, video or file" keep working; `RecordSendButtonTests` unchanged |
| Android | 1.5 days | low | `InputBarVoiceTest`, `AttachMenuTest` (new item) |
| Web | 1.5 days | low | `layout_tests.rs` composer cases (placeholder, phone width) |
| Windows | 1.5 days | medium — cannot run on the Mac | `ComposerButtonTests`, `AttachMenuTests` |

## 6. Beyond the row (from the same screenshots)

Worth fixing in #106 (they are the chat screen's own problems):

- **iOS long-press covers the message it is about**: the dimmed scrim also dims the pressed bubble and the emoji
  capsule and the Reply/Copy/Share panel land on top of it (`ConversationView.swift:3372-3380`, `3598-3633`). Draw
  the pressed bubble above the scrim and place the panels around it.
- **The jump-to-newest button covers outgoing text** on iOS (the trailing edge where own bubbles end) and on the web
  (a solid disc over the last bubble).
- **Own bubbles look different per client**: solid accent with white text on iOS/Mac/Windows, a pale tint on Android
  and the web (nearly indistinguishable from incoming in web dark mode). Canonical: iOS.
- **Multi-line text inside own bubbles is right-aligned** on iPhone and Android, left-aligned on the web. Canonical:
  left-aligned text in a right-anchored bubble.
- **The opening anchor parks the last-read message under the Liquid Glass bar** on iOS, so the title sits on a
  blurred blue (or green, or orange) block; anchor the unread divider a little below the bar.

Later, separate issues:

- iOS sign-in and server-setup screens: "Log In" and "Connect" look like empty fields until something is typed (no
  prominent button style).
- Board toolbar (four unlabelled glyphs) and its landscape layout (notes overlap).
- Chats list separators start at a different x in every row.
- Settings: Log Out and Delete Account side by side; Birthday as two rows; long assistant footnotes.
- Location cards print raw coordinates and accuracy under the place name.
- The thread sheet repeats the root's quote in every reply and does not show the root.
- The Mac screenshot harness passes `-v1.currentUserID` as a string, which the app reads as nil — every Mac store
  screenshot so far showed Nora's own messages as somebody else's (`capture-mac-screenshot.sh`,
  `AppSettings.swift:187`).
- Windows: re-shoot from a current build before #106 closes; the store screenshots are from September.
