# Message and attachment sending from Shortcuts (#118)

*Design note and plan, 2026-11-19, written against the `v1.4` branch. Apple clients only
(iOS, iPadOS, macOS); the server, the protocol and the other four clients are untouched, so
`docs/protocol.md` needs no amendment — every request a Shortcut makes is one the composer
already makes.*

The issue: *"We already have calling shortcuts to call via Family with Siri. We need to have
others to have an opportunity automate more, like sending message by schedule, like every hour,
or attach voice, or photo, or any other file. And send to recipient via Family."*

## 1. What ships

Two Shortcuts actions, in the app (not in an extension), on iPhone, iPad and Mac:

| Action | Parameters |
|---|---|
| **Send a Message** | Recipient, Recipient name, Message, Attachments |
| **Send an Attachment** | Recipient, Recipient name, Attachments, Message (a caption) |

A **recipient** is a family member or the family chat, offered by the Shortcuts picker as a
list read live from the app's own roster. Either action goes to one chat and produces **one
message**, exactly as the composer's "up to ten attachments as ONE message" does.

Nothing new is sent over the wire: the message is queued by `ChatSyncCoordinator.send` and
`sendMedia`, the two every composer button already uses.

## 2. Why the send is the composer's send — **D1**

The scheduled case is the reason the whole feature exists, and it is the reason to reuse the
outbox: `sendMedia` writes the message row and its `PendingMediaItemEntity` rows, and moves the
bytes into `PendingMediaStaging`, **before the first byte leaves** (docs/protocol.md, "Sending
on an unreliable network"). So a Shortcut that runs at 03:00, with the app launched into the
background by the system and killed the moment the send is queued, still delivers — and a
failure is a red bubble with tap-to-retry in the chat, not a vanished intention. Reimplementing
the send for Shortcuts would have thrown that away to save nothing.

## 3. It sends without a composer, deliberately — **D2**

`ShareImport` states *"NOTHING AUTO-SENDS"*: a share lands in a composer and the person presses
Send. That rule is about the composer a share lands in, and this path has no composer — a
Shortcut on a schedule has no window and nobody at the phone. The consent is the Shortcut
itself: built once, by hand, aimed at one recipient, and re-runnable. What replaces the pressed
button is a set of refusals, all of them in `MessageIntentPlan` (pure, tested) rather than
scattered through the UI:

- nothing is sent while the app is signed out;
- an empty body with no attachment is **refused**, never sent — this is the "every hour" case,
  and a blank row hourly is not what anybody means;
- more attachments than one message holds is refused, not truncated;
- an unknown, ambiguous or blocked recipient is refused rather than guessed.

The action also runs with `openAppWhenRun = false`, so an hourly Shortcut never drags the app
across the screen.

## 4. Who can be addressed — **D3**

Family members (a direct chat, created get-or-create when this device has never opened one) and
the **family chat**, which the issue does not name but which is where a scheduled "good
morning", a reminder or a sensor reading most often belongs.

Not addressable, and refused if an old Shortcut still points at one:

- **the assistant's chat** — text-only on the server, so a file there would be refused by the
  server anyway (the same rule `ShareImport.isEligible` enforces);
- **a group** — a group is addressable by id, not by a person's name, and neither action has a
  group concept yet. Follow-up, not an omission;
- **a member who left, deleted their account, or is blocked** — the roster gate is the member
  pickers' rule, so an id that outlived the membership is not choosable at all.

## 5. Naming somebody with no picker — **D4**

A Shortcut frequently has a name rather than a person: the output of *Ask for Input*, a column
of a table, a name in a schedule. So the recipient may arrive as text, and is matched by
`CallRequestRouter.match` — the matcher Siri's spoken name already uses, whole-name equality
first and then a word of the name, with diacritics and case folded.

Two members called Anna produce a **refusal**, not a coin toss; unless blocking has already
answered it, which is the call router's reasoning transplanted (one callable Anna is not
ambiguous; none callable is a block). The picker's entity id wins when both forms are filled
in, because it is exact.

The member's picker id **is** their `familyconnect:<id>` handle — `CallHandle`, the string the
calling side already mints — so a person who resolves for a Siri call resolves for a message,
and one identity is minted in one place. The family chat takes the handle-shaped id a member id
can never be (`familyconnect:family`; a user id is a number), which keeps the two namespaces
apart without a second scheme.

## 6. Attachments are prepared, not forwarded — **D5**

Files arrive from Shortcuts as a URL or as bytes. Either way they go through `MediaPrep`, so the
rules are the composer's and not a second set that can drift: a photo downscaled to the edge
with a preview, a clip brought to the profile, audio under the audio rules (which is what makes
a recorded `.m4a` a playable voice bubble rather than a document row), an animated GIF or WebP
as an animation (#96), and everything else as the file it is. The ceiling is
`MediaPrep.sizeLimit`; the count ceiling is `ShareHandoff.maxAttachmentsPerMessage`, shared with
the share hand-off rather than restated.

Files are prepared **before** any chat is created, so a Shortcut holding an unreadable photo
does not leave behind a direct chat made for a message that never arrived; a failure discards
what was already prepared, which is the same litter rule `MediaPrep.discard` exists for.

## 7. In the app, not in an App Intents extension — **D6**

The sending half needs the keychain (the session token), the SwiftData store and the outbox. An
extension would need the App Group and shared keychain access to answer from another process —
a second place the session is read, for a faster launch. The calling side chose the same way for
the same reason ("no Intents extension, the roster is right here", `CallIntents`). The cost is
that the system launches the app to run an action; `openAppWhenRun = false` keeps that in the
background, and `ensureReady` does what the login screens otherwise do (bootstrap, connect),
because a background launch has neither.

## 8. Strings, and one known hole — **D7 / D8**

20 new keys in `ios/FamilyConnect/Localizable.xcstrings`, in all nine languages, with the
Serbian Latin generated from the Cyrillic by `tools/i18n` as the repo requires. The refusal for
a blocked member deliberately mirrors the existing call refusal's wording, register included, so
the two doors do not say the same thing two ways.

**Siri phrases, and the rule that hid the whole feature (2026-11-20).** Each action is an App
Shortcut with two phrases: the bare one ("Send a message with Family") and one per recipient
("Send a message to Anna with Family"), which the system builds from the recipient query each
time `MessageIntents.recipientsChanged()` is called — at launch and after every roster write
(`ChatSyncCoordinator.onRosterChanged`). Every phrase must contain `\(.applicationName)`: the
metadata step fails the target on one that does not, and a target that fails it carries no
`Metadata.appintents`, so **neither action existed anywhere** — not in the Shortcuts app, not in
Spotlight, not for Siri. That is what the first build of this feature shipped.

The phrases are translated in their own table, `ios/FamilyConnect/AppShortcuts.xcstrings` (nine
languages; keys spell the placeholders `${applicationName}` and `${recipient}`), not in
`Localizable.xcstrings`. The recipient's name is substituted as the roster spells it, so the
Russian and Serbian phrases put it after "получатель" / "прималац", where it needs no case.

An action started with a value missing (said to Siri, tapped in Spotlight) now **asks** for it
— `needsValueError()` with a `requestValueDialog` — instead of refusing; with nobody there to
answer, the same throw is the action's error, so §3's refusals hold.

`LM_SKIP_METADATA_EXTRACTION = YES` moved from the project to the three targets that declare no
intents (tests, UI tests, share extension), where it silences "no AppIntents.framework
dependency found". It must never be set on the app target again.

**The known hole:** `ios/scripts/check-strings.py` reads the positions it knows
(`Text`, `Button`, `navigationTitle`, …) and cannot see `IntentDescription`, `@Parameter(title:)`
or `IntentDialog`. A string added there next time ships English in nine languages with the job
green. Extending `CALLS`/`MODIFIERS` is a separate change (§7: tooling of its own), and the
three files here are the ones to check when it happens.

## 9. Privacy and logs

Nothing new leaves the box: a Shortcut's message travels the same `POST /chats/{id}/messages`
and `POST /attachments` the composer uses, to the server the user configured. The README's
"Who can see your messages" table therefore needs no new row. The logs added here carry a count
or nothing at all — no message text, no name, no recipient, no file name (§12).

## 10. How it was tested

`ios/FamilyConnectTests/MessageIntentPlanTests.swift` — 15 tests, both lanes (iPhone simulator
and `platform=macOS,arch=arm64`), pinning every branch of `decide` in the order the plan asks
them, the attachment cap at and above the limit, the name matcher's four outcomes, and the
picker's ids against `CallHandle`. The intent framework itself is not testable on the simulator,
which is why every decision above lives in `MessageIntentPlan`.

## 11. Follow-ups, listed rather than assumed

- Groups as recipients (§4), and a Shortcut that opens a chat rather than posting to it.
- `check-strings.py` covering the App Intents positions (§8).
- `tools/i18n/regenerate_serbian.py` currently dies on four `SR` keys the catalogue renamed
  during the groups work and around "Attach a File…" — unrelated to this issue, and worth its
  own. Until it is fixed, Serbian for a new Apple string has to be written by the generator's
  own functions rather than by running it.
