# One attachment menu (issue #78)

**Written 2026-10-07** against `v1.2`, in answer to issue #78: *"We should have the same attachment menu for all
platforms. Now I like iOS attachment menu, but on Android, for example, this menu looks strange, if we really need more
items, maybe better to group some of them."*

## What there was

iOS, the Mac, Windows and the web already shared one ORDER (assistant photo, file, paste, voice, video, location,
poll) but no grouping, and each said it differently: the Mac, Windows and the web had no "Photo or Video" and called
the file item "Attach a File…"; Windows' button was a "+", the web's menu had no icons. Android was the outlier:
eleven ungrouped lines, "Take photo" five rows away from "Take video", `/draw` ("Ask for a picture") inside the menu
in two chats, a two-line assistant item, and BOTH the assistant's photo and "Photo or video" in the assistant chat.
iOS laid its menu out automatically (bottom-up from the paperclip) while every other client read top-down.

## The menu, on every client

The same items, in the same order, in the same three groups, read TOP TO BOTTOM everywhere (owner's choice,
2026-10-07 — iOS gets `.menuOrder(.fixed)` so it no longer turns the list round):

```
Photo or Video          the photo/video picker   — "Show the Assistant a Photo…" INSTEAD, in the assistant chat
Camera                  the system camera        — iPhone/iPad and Android only (see below)
File                    the file picker
Paste                   the clipboard
──────────────
Record Voice Message    hands-free voice note    — not in the assistant chat
Record Video Message    the round-video recorder — where round video is offered
──────────────
Location                share where I am, once
Poll                    a poll                   — the family chat only
```

- **The button is a paperclip** on every client, with the label/tooltip "Attach a photo, video or file" (the iOS UI
  tests find the composer by it; Windows' "+" becomes Segoe's Attach glyph).
- **Groups are separators**, not headings: SwiftUI `Divider()` in the `Menu`, NSMenu separators on the Mac, a
  `HorizontalDivider` in Android's `DropdownMenu`, `MenuFlyoutSeparator` on Windows, an `<hr role="separator">` on
  the web. A group that would be empty draws no separator (the assistant chat has no recording group).
- **Photo or Video** opens the platform's own media picker: PhotosPicker on iPhone/iPad AND the Mac (the same SwiftUI
  modifier; the Mac gains it), Android's `PickVisualMedia(ImageAndVideo)`, a picture-and-video `FileOpenPicker` on
  Windows (Pictures library first), an `<input type=file accept="image/*,video/*" multiple>` on the web. In the
  assistant chat it is REPLACED by "Show the Assistant a Photo…" (images only) when the server and family allow
  pictures, and absent when they do not — never both lines.
- **Camera** only where there is a system camera to hand off to: iPhone/iPad (one item, the system picker toggles
  photo/video) and Android, where the system camera takes a photo OR a video by intent, so "Camera" opens its two
  choices — "Take photo" and "Take video" — in place of the menu (a back row returns). Hidden without a camera, and in
  the assistant chat unless it accepts pictures. The Mac, Windows and the web have none (S1.5 of
  docs/audio-video-messages-2026-10-04.md).
- **File** is "File" everywhere ("Attach a File…" retired): the full file picker, photos and videos included.
- **The labels are the same words**, in each platform's own casing (Android's sentence case, Apple's and Windows'
  title case), with icons on every client — SF Symbols `photo.on.rectangle` / `photo`, `camera`, `doc`,
  `doc.on.clipboard`, `mic`, `video.circle`, `mappin.and.ellipse`, `chart.bar`, and their Material, Segoe and emoji
  counterparts.
- **What is NOT in the menu**, on every client: stickers (their own button — a sticker is sent by the tap that picks
  it), `@ai` (its own button), `/draw` "Ask for a picture" (its own paintbrush button in the assistant chat — Android
  moves it out of the menu, owner's choice), the video-call button. Availability rules are unchanged (call in
  progress, unsent voice message, busy composer, server features).

## Where it lives

One pure rule per client decides the items and groups, and the menu only draws it — so the five cannot drift apart
again without a test noticing:

| Client | The rule | Its tests |
|---|---|---|
| iOS + Mac | `ios/FamilyConnect/Models/AttachMenu.swift` (and `Core/PickedMediaPrep.swift`, the photo-picker staging both now share) | `FamilyConnectTests/AttachMenuTests.swift` |
| Android | `ui/chat/AttachMenu.kt` | `AttachMenuTest.kt`, `InputBarVoiceTest.kt` |
| Windows | `FamilyConnect.App.Logic/AttachMenu.cs` (`PhotoOrVideoTypes` is tied to `MediaPrep.MimeFor`) | `AttachMenuTests.cs` |
| Web | `web/text/src/attach_menu.rs` | its own unit tests |

Two details settled while building it: in the ASSISTANT chat, where only pictures are accepted, Android shows "Take
photo" directly rather than a Camera page with one choice in it; and Android's `canAskForPicture` still gates the
picture-description hint in the family chat (somebody may type `@ai /draw …` there), while the paintbrush BUTTON is
the assistant chat's alone, as on every other client.
