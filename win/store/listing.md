# Microsoft Store — Family Connect for Windows

Every text and image the Partner Center submission needs, written against what the Windows client actually does
(`win/README.md`) and against the Apple copy in `ios/docs/appstore.md`, whose rules it keeps: never "secure",
"encrypted", "end-to-end" or "safe" — messages are stored readable on the server and the description says so — and
every place something leaves the family's server is named rather than found.

**English (en-US) only for now.** The other eight app languages follow once this copy is approved; Partner Center takes
a listing per language (Store listings → Add/remove languages), or all of them at once through its CSV export/import.

Limits are Partner Center's (Microsoft Learn, "Add and edit Store listing info", checked 2026-09-15). Counts in headings
are measured with `win/store/count.ps1`, not estimated.

---

## Product name

FamilyConnect

*The name reserved in Partner Center, written as one word because that is how it is reserved; the manifest's
`<Properties><DisplayName>` carries the same string, which certification matches against the reservation. The copy
below still says "Family Connect" in prose, as the App Store and Google Play listings do, and the Start menu and tiles keep
"Family Connect" too. The package identity in `win/src/FamilyConnect.App/Package.appxmanifest` is Partner Center's own
(Product identity): `nttrsh.FamilyConnect`, publisher `CN=28EC49E1-8A19-45EF-A576-FF596547E069`, shown as `nttrsh`.*

## Short description (≤ 1000; keep ≤ 270)

A private messenger for one family and nobody else: chats, photos, voice notes, polls, a family board and one-to-one calls. Start on our server, or move your family to one you run yourself — the server is free and open source.

## Description (≤ 10,000)

Family Connect is a private messenger for one family and nobody else — no feed, no discovery, no directory of users. The only people who can reach you are the ones your family's owner let in.

On Windows it is a Windows app, not a phone screen made wide: your chats in a list beside the conversation you are reading, a family board, the family's own settings for its owner, and a call in the corner of whatever you are doing. Close the window and it keeps running in the notification area, so a message or a call still reaches you.

It works out of the box: install, pick a username — no email address, no phone number — then create your family, and you become its owner, or join one with an invite code. The owner decides whether that code admits people instantly, needs approval, or admits nobody, and can rotate it, cap the membership, or remove someone.

What makes Family Connect different is where your messages live. There is no vendor cloud: every installation talks to a single Family Connect server. Out of the box that is the default server we operate — and on it, we are the ones holding your family's messages and files. But the server software is free and open source (MIT, written in Rust), and "Change server" on the sign-in screen points the app at any other one. Put it on your own hardware and your conversations exist in exactly two places: your server and your family's devices.

Day to day it works like any modern messenger:

- One shared chat for the whole family, and a private one-to-one chat with any member of it
- Photos, videos, voice notes and files of any kind — up to ten on a message, picked, pasted, dragged in, or shared from another app; a message's photos open as one album you page through
- One-to-one voice and video calls
- Your location sent once, drawn on a map, only when you choose — never continuously, never in the background
- Replies, threads, editing what you have sent, @mentions, and reactions from hundreds of emoji
- Polls in the family chat, and a board of coloured notes, events and lists anyone can move
- Birthdays, so nobody has to remember
- Real-time delivery, typing indicators, read receipts in one-to-one chats, and unread counts on the taskbar
- History kept on the PC, so you can read it back with no network
- Report a message or a member to your family's owner, who can remove them — a report about the owner goes to whoever runs the server instead
- Block anyone: their messages fold behind a row you can click, their chat leaves your list, their calls never reach you
- Eight languages in nine localisations, Serbian in both alphabets

An optional assistant can answer questions in the family chat when the person running your server turns it on and supplies their own provider. It is off unless they enable it.

What it does not have: ads, analytics, tracking, crash reporting, or the attribution SDKs that usually arrive with a free messenger.

And the honest limits. Windows notifies you only while Family Connect is running — in its window or in the notification area. A quit app is not notified and does not ring, so leave it running if you want to be reachable on this PC. There is no camera capture inside a chat and no message search yet. This is not end-to-end encryption and we will not imply that it is: messages and attachments travel encrypted to the server your family chose — plain http only for a server on your own network — and are stored there in readable form, so whoever runs it can read them. Calls are different: picture and sound go straight between the two devices wherever the network allows; where it does not, the call connects only if your server's operator runs a relay, which forwards the stream encrypted and cannot read it. Some things necessarily reach past your server, and each is switchable in Settings or named here: the map under a shared location, drawn from OpenStreetMap; previews under links, fetched from the linked site; and a public STUN server asked for your address when you place a call. How long old messages are kept is a setting for whoever runs the server.

If you want your family's conversations off big-tech servers — and, when you are ready, on hardware you own — this is what Family Connect is for.

## Product features (≤ 20, each ≤ 200)

1. One private chat for the whole family, plus one-to-one chats with any member
2. Photos, videos, voice notes and files — up to ten on a message
3. One-to-one voice and video calls
4. Share files into a chat from any app with the Windows Share menu
5. Polls, replies, threads, edits, mentions and emoji reactions
6. A family board of notes, events and task lists anyone can move
7. Location sent once, when you choose, drawn on a map
8. Keeps running in the notification area when you close the window
9. Unread counts on the taskbar and notifications while the app runs
10. History kept on your PC for reading offline
11. The owner approves who joins, rotates the invite code and can remove members
12. Report and block, with a report inbox for the family's owner
13. Works with our server out of the box, or with your own open-source server
14. No ads, no analytics, no tracking
15. Username and password only — no email address or phone number
16. Nine languages

## What's new in this version

*Leave blank for the first submission (Partner Center's own instruction).*

## Additional system requirements

*Minimum hardware* — leave empty. *Recommended hardware* (≤ 11 items, each ≤ 200):

1. A microphone for voice notes and calls, and a camera for video calls
2. An internet connection to your family's Family Connect server

## Screenshots (desktop, ≥ 1366 × 768, PNG; captions ≤ 200)

Captured at 3200 × 1800 (16:9) from the invented "Harpers" family on a local server — `server/scripts/seed-store-screenshots.sh`,
or its Windows port `win/store/seed-store-screenshots.ps1` — never from a real family. Files in `win/store/images/screenshots/`.

| # | File | Caption |
|---|---|---|
| 1 | `01-family-chat.png` | One private chat for the whole family, and a one-to-one chat with anyone in it. |
| 2 | `02-photos-and-poll.png` | Photos as one album, polls with everyone's votes, and reactions. |
| 3 | `03-location.png` | A place sent once, when you choose, drawn on a map. |
| 4 | `04-board.png` | A family board of notes anyone can move. |
| 5 | `05-family.png` | The owner decides who joins, and can remove anyone. |
| 6 | `06-settings.png` | Your privacy switches, and a notification area that keeps you reachable. |

## Store logos and additional art

| Image | File | Why |
|---|---|---|
| 1:1 App tile icon, 300 × 300 | `win/store/images/app-tile-icon-300x300.png` | Strongly recommended; the Store prefers it over the package's own logo. |
| 16:9 Super hero art, 1920 × 1080 | `win/store/images/super-hero-art-1920x1080.png` | Recommended; used at the top of the listing and in promotions. No text and no UI, as required. |

*The 2:3 poster art and 1:1 box art are for games and Xbox; this app needs neither.*

## Copyright and trademark info

© 2026 nettrash

## Privacy policy URL, website, support

- Privacy policy: `https://nettrash.me/appstore/familyconnect/privacy.html` — **update the page first**, see below
- Website: `https://github.com/nettrash/family.connect`
- Support contact: `https://nettrash.me/appstore/familyconnect/support.html`

## Category

Social. (Subcategory: none.)

## Notes for certification

*Partner Center → Submission options → Notes for certification. Fill the placeholders from the live server first.*

DEMO SERVER: https://fc.nettrash.me — built into this package, so the app opens on the sign-in screen with nothing to configure.

DEMO ACCOUNTS: owner [DEMO_USER] / [DEMO_PASS]; second member [DEMO_USER_2] / [DEMO_PASS_2] in the same family, for typing, read receipts and a call from a second PC. Invite code [INVITE_CODE].

WHAT IT IS: a client for a small open-source (MIT, Rust) chat server. Each installation talks to one server; a family's messages live in that server's database and on the family's devices. There is no public feed, no user directory and no search for people — an account in no family can reach nobody, and the only ways into a family are creating one or an invite code the owner controls.

REPORT AND BLOCK: right-click a message → Safety → Report… or Block; or Family → a member's Safety menu. Reports reach the family owner under Family → Reports.

DELETE ACCOUNT: Settings → Delete Account… (password, then a confirmation). Immediate and irreversible.

GENERATIVE AI (policy 11.16), and the answer to the 2026-09-18 certification comment. The product has an assistant: a member can ask it questions in a private chat of their own, and can address it in the family chat by writing @ai. Both halves of what 11.16 asks are now in place.
- REPORTING WHAT THE AI GENERATED: right-click any assistant reply → Safety → "Report this reply…". It offers the same four reasons the rest of the product uses and a free-text box, and it is on both surfaces the assistant speaks on. The report goes to the people who run the server — NOT to the family owner, deliberately: a private assistant thread belongs to its member alone, and the owner can neither read it nor change what a model said. The sheet says so before anything is sent. (Server: `POST /api/v1/reports/assistant`; the reply is frozen into the row so it survives retention. docs/protocol.md, "Reporting the assistant".)
- THE DECLARATION: Properties → Product Declarations → "This product incorporates generative AI features…" is ticked for this submission.
- The assistant is also the operator's switch: a server run by somebody else can have it off entirely, in which case the app shows no assistant chat and no @ai.

RUNFULLTRUST: a WinUI 3 desktop app packaged as MSIX; full trust is the Windows App SDK's standard model, not a special use.

NOT END-TO-END ENCRYPTED, and the app does not claim to be. Calls are peer to peer; please test them between two PCs.

## Age rating (IARC questionnaire)

Answer as a communication app, and answer truthfully rather than to a target rating:

- Users can interact / exchange content with other users: **yes** — messages, photos, videos, voice notes, files, calls, bounded to one family whose membership its owner controls.
- Shares the user's location with other users: **yes** — once, when the user chooses.
- Unrestricted web access: **no** — links open in the system browser; the app fetches only a linked page's preview metadata.
- User-generated content moderation: in-app reporting to the family owner, blocking, member removal.
- Digital purchases, gambling, ads: **no**.
- The assistant, where a server enables it, writes model output into the family chat — answer the "AI-generated content" question according to whether it is on for `fc.nettrash.me`.

## Privacy policy — what the page must add for Windows

The page at `nettrash.me/appstore/familyconnect/privacy.html` (in the nettrash.me site's own repository) names iOS, iPadOS,
macOS and Android, Apple Maps, Google Maps and Firebase — and not Windows. Certification checks the policy against the app,
so add, in its own words:

- **Platforms:** "the iOS, iPadOS, macOS, Windows and Android apps".
- **Maps on Windows:** the map under a shared location is drawn from **OpenStreetMap**'s tile servers, which receive
  requests for the map around the shared place; switchable off with Map Previews in Settings.
- **Notifications on Windows:** no push service is used — the app notifies from its own connection while it runs, so no
  message text reaches Microsoft.
- **Calls on Windows:** media runs in **Microsoft Edge WebView2**, part of Windows, peer to peer as on the other platforms.
- **Stored on the PC:** message history, downloaded attachments and map tiles in the app's own local folder; the session
  token in the Windows Credential Locker. Uninstalling removes them.

## Submission checklist

- [x] **Reserve the name** — reserved as "FamilyConnect".
- [x] **Copy the identity** — `nttrsh.FamilyConnect`, `CN=28EC49E1-8A19-45EF-A576-FF596547E069`, `nttrsh`. From Partner Center → Product identity: **Package/Identity/Name**, **Publisher** and
      **PublisherDisplayName** from Product identity into `Package.appxmanifest` (`<Identity Name=… Publisher=…>` and
      `<PublisherDisplayName>`). Keep `Version` hand-written.
- [ ] **Build the Store packages**, pointed at the default server, unsigned (the Store signs them):
      `powershell -NoProfile -ExecutionPolicy Bypass -File win/store/build-store-packages.ps1` — it writes
      `FamilyConnect.App_<version>_x64.msixupload` and `…_arm64.msixupload` under `win/AppPackages/`. Upload **both** to
      the one submission (Packages step); the Store gives each PC its own architecture, so there is nothing to bundle.
      The `_Test` folders beside them are sideload copies, not for the Store. Raise `Version` in `Package.appxmanifest`
      before every later submission — the Store refuses a version it has already seen.
- [ ] **Run the Windows App Certification Kit** on the package before uploading.
- [ ] **Update the privacy policy page** as above, then fill the URL.
- [ ] **Provision the demo accounts** on fc.nettrash.me and fill the certification notes.
- [ ] **Pricing and availability:** free; markets as for the other stores.
- [ ] **Properties:** category Social; privacy policy URL; system requirements — Windows 11, version 21H2 (22000) or later,
      which is the package's `MinVersion`.
- [ ] **Age ratings:** the IARC questionnaire, as above.
- [ ] **Store listing (en-US):** the texts, captions and images in this file.
