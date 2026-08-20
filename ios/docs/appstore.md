# App Store Connect — Family Connect

Texts for the App Store listing. Fill every `[PLACEHOLDER]` before submission.

> **BLOCKER before submitting** (guideline 5.1.1(v)): the app supports account creation, so
> Apple requires an **in-app account deletion** flow. v1 does not have one yet — it needs a
> `DELETE /me` server endpoint plus a Settings → Account → Delete Account screen. The review
> notes below already describe that flow; implement it first or expect a rejection.

## Promotional Text (139/170 chars)

Private chat for just your family. Works out of the box; move it to your own server anytime. No ads, no tracking, no company in the middle.

## Description

Family Connect is a private messenger for one family — grandparents to kids — and nobody else. No feed, no strangers, no noise.

It works out of the box: install, pick a username, then create your family — you become its owner — or join one with an invite code. The owner decides whether the code admits people instantly or each request needs approval, and can rotate the code or remove members at any time.

What makes Family Connect different is where your messages live. There is no vendor cloud: every installation talks to a single Family Connect server. Out of the box that is the default server we operate, so you can start right away. But the server software is free and open source (MIT, written in Rust), and the app switches to your own server at any time — a home machine, a spare computer, a small box in a closet ("Change server" on the sign-in screen). Point your family's apps at your own hardware and your conversations exist in exactly two places: your server and your family's devices. No company can read, mine, or sell them, because no company has them. One server can host several families, so one technically inclined relative can cover everyone.

Day to day it works like any modern messenger:

- One shared chat for the whole family
- Private one-to-one chats between any two members
- Real-time delivery while the app is open, with typing indicators
- Notifications when messages arrive while the app is closed
- Read receipts in one-to-one chats, plus unread counts
- History kept on the device for offline reading

What it does not have: ads, analytics, tracking, or third-party SDKs of any kind. And the honest limits of version 1: text messages only. Voice and video calls are planned.

If you want your family's conversations off big-tech servers and on hardware you own, this is what Family Connect is for.

## Keywords (93/100 chars)

chat,messenger,private,self-hosted,home,server,secure,messages,household,text,group,relatives

("family" and "connect" are omitted on purpose — the app name is already indexed.)

## Notes for App Review

Family Connect is a client for an open-source chat server (MIT, Rust). There is no vendor cloud: every installation talks to one Family Connect server. The App Store build ships pre-pointed at the default server we operate (https://fc.nettrash.me), so the app is fully testable out of the box with no setup — launch it and the sign-in screen appears directly. Families who prefer full self-hosting switch to their own server at any time via "Change server" on the sign-in screen; the App Store description explains both modes.

DEMO/DEFAULT SERVER: https://fc.nettrash.me (pre-configured in this build)
This server will stay online for the entire review period.

DEMO ACCOUNTS (the owner account is also entered in the App Review Information demo-account fields):
- Owner: username [DEMO_USER], password [DEMO_PASS]. This account owns the pre-made reviewer family, which already contains members and message history, so the family chat, 1:1 chats, and the owner tools (Settings > Family: approve/decline join requests, rotate the invite code, remove members, change join policy) are visible immediately.
- Second account: username [DEMO_USER_2], password [DEMO_PASS_2]. Signing in with this account on a second device lets you watch real-time delivery, read receipts (1:1 chats), and typing indicators between two live sessions.

Registration is also fully open: tap Register and create any username and password. No email, phone number, or other personal data is required or collected.

HOW TO REVIEW:
1. Launch the app — it is already pointed at the default server; the sign-in screen appears directly. (The "Change server" footer on that screen is where self-hosted servers are entered; any URL can be tried there and reverted.)
2. Sign in with the owner demo account, or register a new account.
3. A newly registered account can create its own family (becoming owner, with an invite code under Settings > Family) or join the reviewer family with invite code [INVITE_CODE]. This code is set to instant join, so there is no approval wait. To see the approval flow, switch the reviewer family's join policy to "owner approval" from the owner account, then join with a fresh account.
4. Chats update in real time over a WebSocket while the app is in the foreground. Push notifications (APNs) cover the rest: members without a live connection are notified of new messages, family owners of join requests, and requesters of approvals. A foregrounded app is never pushed — the socket already delivers — so notifications only appear when the app is backgrounded or closed.

ACCOUNT DELETION (guideline 5.1.1(v)): Settings > Account > Delete Account permanently removes the account and its messages from the server. It is available to every account, including the demo accounts (we re-provision those if deleted).

PRIVACY:
- No third-party SDKs, no analytics, no ads, no tracking.
- Message and account data exist only on the server the user chose and on the devices. On the pre-configured default server that operator is the developer; users who switch to a self-hosted server share nothing with the developer at all. The App Privacy label and privacy policy reflect this.
- No email, phone number, or real name is required — accounts are a free-form username and password.
- Text messages only in this version.

NETWORK / ATS: Plain http:// addresses are accepted only for private LAN hosts, via the scoped NSAllowsLocalNetworking exception. Public servers require https. NSAllowsArbitraryLoads is not used.

If the demo server is ever unreachable, contact [SUPPORT_EMAIL] and we will restore it immediately.

## Beta App Description (TestFlight → Test Information)

Family Connect is a private messenger for one family — and this beta is how we make sure it feels right before it goes live.

The app comes pre-connected to our family server, so there is nothing to configure: open it, register a username and password (no email or phone number needed), then create a family or join one with an invite code. Every family gets one shared chat for everyone, plus private one-to-one chats between any two members. The family owner manages who gets in: the invite code can admit people instantly or require the owner's approval, and it can be rotated at any time.

What works in this beta: real-time messaging while the app is open, push notifications when it is closed (new messages, plus join-request alerts if you own the family), offline reading of your history, sending with automatic retry, unread counts, read receipts in one-to-one chats, and typing indicators. If you run your own Family Connect server, "Change server" on the sign-in screen points the app at it.

What is not here yet, on purpose: photos and other media, and voice/video calls. Text only for now.

There are no ads, no analytics, and no tracking in the app — so the only way we learn about problems is you telling us. If anything feels confusing, slow, or broken, use TestFlight's "Send Beta Feedback" (a screenshot helps) or email us directly. Thank you for testing.

*(~1,450 chars of the 4,000 limit. Pairs with Feedback Email — set it to your support address.)*

*(Notification permission is requested in-app once the user is in a family, so TestFlight reviewers will see the standard iOS prompt.)*

## What to Test (first TestFlight build)

Fresh install: register, create a family, and share the invite code with a second tester. Second tester: join with the code (try both join policies — the owner can switch between instant and approval in Settings → Family). Then exchange messages in the family chat and a one-to-one chat: check messages arrive in real time both ways, read receipts appear in the 1:1 chat, unread badges clear when you read, and history is still there after force-quitting the app or going offline. Owners: try rotating the invite code and removing a member. Background the app for a few minutes, return, and confirm the chat catches up and the "Connecting…" banner clears within a few seconds.

Push notifications, new in this build: allow notifications when the app asks (it asks once you're in a family). Close the app fully (swipe it away) and have the other tester message you — a notification should arrive, and tapping it should open that exact chat, even from a cold start. Owners with the join policy on approval: with the app closed, have someone request to join — the join-request notification should open the approval screen. Also check the negatives: notifications should NOT appear while you're actively in the app (messages arrive live instead), and the red badge on the app icon should clear as soon as you open the app.

## Pre-submission checklist

- [ ] Implement account deletion (server `DELETE /me` + Settings → Account → Delete Account) — see blocker above
- [ ] Create the two demo accounts on fc.nettrash.me and fill `[DEMO_USER]/[DEMO_PASS]`, `[DEMO_USER_2]/[DEMO_PASS_2]`
- [ ] Create the reviewer family from the owner account, seed a few messages (family + one 1:1 chat), set join policy to open, fill `[INVITE_CODE]`
- [ ] Enter the owner demo credentials in App Review Information → demo account fields too
- [ ] Fill `[SUPPORT_EMAIL]`; set Support URL and Privacy Policy URL (a page on nettrash.me explaining both modes: data on the developer-operated default server vs. fully self-hosted)
- [ ] App Privacy label — because the store build defaults to the developer-operated server, "Data Not Collected" is NOT defensible any more. Declare: User Content (messages) and User ID (username), collected, App Functionality only, NOT used for tracking, "Data Not Linked to You" is reasonable since no email/phone/real name is required. (A build without a default server could declare "Data Not Collected".)
- [ ] Decide whether open registration on fc.nettrash.me is acceptable long-term — any App Store user can register and create their own family on your box (join policy protects your family; server load and content are the concern). A [registration] config switch on the server is a possible follow-up.
