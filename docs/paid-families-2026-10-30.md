# Paid family hosting (#89)

*Plan, 2026-10-30. **DECIDED 2026-10-30 — D1–D6 all (a), as recommended:** bought on iPhone, iPad and Mac; everything keeps working through the 14 unpaid days, with a banner; deleted on day 14; limits for paid families only; existing families stay free; neutral wording on Android. Built 2026-10-30 on the server and all clients; what changed from the plan
while building it is marked *(built)*.*

The issue: *"We should allow to create Families via Registration by subscription on iOS. This will be an initial
limitation for this. And we should show the proper message about it. If subscription exists and active (paid) family can
be created and members can use family apps. subscription should be attached to owner. If subscription is gone (not paid)
family can be on a server not more than 2 weeks. Each family has limitation for usage of AI and disk resources. Also
paid families can't have more than 25 members."*

The owner's words (2026-10-30): *"for now we can collect payments only on Apple. we should have configuration switch to
allow and use paid families on a server. if paid registration is allowed we should check active subscription on iOS for
each family except root family and family which was marked as available without subscription. outside of iOS creating
paid families is not possible because we can collect money only with Apple for now. and this fact means that the owner
of the paid family should register paid family from ios."*

## 1. What it means

- **An operator switch.** `[subscriptions] enabled`, off by default. A server that leaves it off behaves exactly as
  today: no client change is visible on it, and nothing below applies.
- **With it on, a new family needs its owner's App Store subscription.** That covers both ways of starting one: the
  first family after registering, and #88's "New Family…" from inside the app. Joining a family stays free everywhere,
  by code or by invitation. Members never pay.
- **Two kinds of family.** A *free* family needs nothing. A *paid* family needs its owner's subscription to be active.
  The root family is a free family, and the operator can mark any other family free (§4).
- **The subscription belongs to the owner, not the family.** If ownership passes to somebody without a subscription,
  the family is unpaid until they subscribe.
- **Unpaid for 14 days, then deleted.** The day the owner's subscription ends, every member is told, and the family is
  deleted 14 days later unless the owner renews before then (D2, D3).
- **Limits for paid families:** at most 25 members, a disk allowance and a monthly assistant allowance (§6, D4).
- **Only Apple's apps can buy.** iPhone and iPad, and the Mac too if D1 says so: it is the same app and the same
  bundle ID, so one purchase works on all three. Android, web, Windows and Linux say that a family on this server is
  started from the Apple app, and still join families as before.

## 2. What is there today

- `POST /families` has two doors, `[families] registration` (an account's first family) and `member_families` (another
  one, while owning none). Both answer 403 with their own code when shut. `GET /me` carries both flags, so a client
  shows a shut door instead of meeting the 403.
- No family is special. "Root family" means the operator's own family, which is simply the oldest one; nothing in the
  code knows it.
- A family's member cap is `min(the owner's max_members, [limits] max_family_members)`, checked at every door by
  `family_is_full`.
- AI usage is already recorded per family and member in `ai_usage`: prompt and completion tokens, pictures,
  transcripts and their milliseconds, and searches. The one existing per-family limit is
  `[ai.lookups] daily_searches_per_family`.
- Disk: since #88 every upload names its family (`POST /attachments?family_id=`). Family Statistics already sums the
  family's attachment bytes.
- Ownership moves on its own: an owner who leaves or deletes their account hands the family to the longest-standing
  member (`family_owner` frame).
- The last member leaving deletes the family. #59 forgets its key, and a backup can still read it for 7 days.
- iOS and macOS are ONE target with ONE bundle ID (`me.nettrash.FamilyConnect`, iOS 17 / macOS 14), so StoreKit 2 and
  `SubscriptionStoreView` are available on both. Nothing uses StoreKit yet.
- The server already has `ring` and `rustls-webpki` (through rustls) for checking Apple's signatures.

## 3. Buying, and how the server knows

**In App Store Connect (the owner's part).** Create one subscription group, "Family hosting", with a monthly and a
yearly auto-renewable product: `me.nettrash.FamilyConnect.family.monthly` and `….family.yearly`. Prices are yours to
set. Turn on Billing Grace Period (16 days is a good value): during it Apple keeps the subscription active while it
retries a failed payment. Set the App Store Server Notifications V2 URL, for both Production and Sandbox, to
`https://fc.nettrash.me/api/v1/appstore/notifications`.

**On the device.** The app shows the products the server lists in `GET /me`, using Apple's `SubscriptionStoreView`.
That view draws the price, the period, the renewal terms and Restore, which is what App Review checks (guideline
3.1.2). The purchase carries `appAccountToken`: a random UUID the server generates once per account, so Apple never
learns who the account is. After a purchase, and on every launch, the app sends the newest signed transaction and
renewal info (`Transaction.jwsRepresentation`, `RenewalInfo.jwsRepresentation`) to `PUT /me/subscription`.
`Transaction.updates` covers a renewal or refund while the app is running.

**On the server — checked offline.** Each value is a JWS signed by Apple. The server checks every one before
believing it:

- the `x5c` chain: leaf, intermediate, and a root byte-equal to Apple Root CA – G3, pinned in the binary (SHA-256
  `63343abf…3e9179`, the same certificate macOS ships in its system roots);
- each signature in the chain, and the JWS's own ES256 signature;
- Apple's two marker extensions: `1.2.840.113635.100.6.11.1` on the leaf and `1.2.840.113635.100.6.2.1` on the
  intermediate. These are NOT optional. Apple Root CA – G3 also signs certificates whose private keys belong to
  developers, such as Apple Pay merchant identities, so without the markers anybody with a developer account could
  sign a "subscription" themselves;
- validity at the payload's `signedDate`, as Apple's own library does;
- the payload: `bundleId` equals `[subscriptions] bundle_id`, `productId` is one of `product_ids`, `environment` is
  allowed, the type is an auto-renewable subscription, there is no `revocationDate`, and `appAccountToken`, when
  present, is this account's token.

No call to Apple is needed for any of it. OCSP revocation checks are left out, as Apple's library leaves them out
when offline.

**Apple tells the server, too.** Apple sends App Store Server Notifications V2 to `POST /appstore/notifications` for
every renewal, failed payment, grace-period end, expiry, refund and revocation. The server verifies the
`signedPayload` the same way, then applies the transaction and renewal info inside it. The handler works by STATE,
not by event: it stores the newest expiry, grace end and renewal status, and ignores anything older than what it
already holds (by `signedDate`). A notification for a transaction no account has claimed yet gets `200` and is
dropped. Anything that fails verification gets `400`; Apple retries a non-2xx, and a forged one does no harm. The
body runs to tens of KiB, because each nested JWS carries its own three certificates, so this route takes up to 64 KiB rather
than the usual 16 KiB.

**Missed news is safe.** If the server is down past Apple's retries (five, over several days), it misses that news. The next launch
of the owner's app sends the current state, and the 14-day rule (§5) gives that time to happen. The App Store Server
API, which pulls status with an In-App Purchase key, is left for later. The doc names it in case missed renewals
ever turn out to matter.

**Sandbox.** App Review buys in the Sandbox environment against the production server, and so do TestFlight testers.
`environments = ["Production", "Sandbox"]` is therefore the default. This is safe because Sandbox subscriptions stop
renewing on their own within hours (TestFlight ones within a week), so a Sandbox family becomes unpaid and is deleted 14
days later. The environment is stored with the subscription, and the operator CLI shows it.

*(built)* The transaction ids are kept sealed beside their HMAC lookups (under the account's key), because
`storage-key rotate` recomputes every lookup from the value next to it — an HMAC alone would be lost to a rotation.

**One subscription, one account.** A transaction belongs to the first account that presents it, keyed by
`originalTransactionId`. Another account presenting it gets `409 subscription_taken`. Restoring on another device of
the SAME account just refreshes it.

**One subscription covers every family its owner owns.** An account can CREATE a family only while it owns none
(#88), but a hand-off can still give it a second one. Apple sells an auto-renewable subscription once per Apple ID
per group, so "one subscription per family" would leave that owner no way to pay for the second family at all.

**Who could abuse this?** Only a server whose operator turns the switch on accepts the subscription, and the money
goes to the app's publisher. Another operator gains nothing by turning it on. One subscription presented to two such
servers would count on both; the doc says so rather than pretending otherwise.

## 4. Free families

- `families.plan`: `free` or `paid`. A family created while subscriptions are on is `paid`; one created while they
  are off is `free`. The migration makes every existing family `free`, so the root family, and any family that exists
  when an operator first turns the switch on, keeps working with nothing to do (D5).
- The operator changes a family's plan with the CLI:

  ```
  family-connect families list
  family-connect families plan <id> free|paid
  ```

  `list` prints each family's id, name, plan, owner, member count, disk used, this month's assistant use, and, for a
  paid family, its owner's subscription state and any deletion date. It prints to the operator's own terminal, never
  to the journal.
- Turning a free family `paid` starts its 14 days at once if its owner has no subscription. Turning a paid family
  `free` stops any clock.
- With the switch off, plans are kept but nothing reads them: no limits, no clocks, no deletions.

## 5. When a subscription ends

- **Coverage.** A paid family is *covered* while its owner holds a subscription that is not revoked and whose expiry,
  or Apple's grace-period end when that is later, is in the future.
- **The hourly sweep** already deletes old messages and familyless accounts. It now also does three things:
  1. For a paid family that is not covered: set `unpaid_since` (when it was unset) and send every member a push and
     a `family_plan` frame.
  2. For a family that is covered again: clear `unpaid_since`, and send the frame again.
  3. Delete a family once `unpaid_since + [subscriptions] unpaid_days` has passed (14 by default). The day before,
     every member gets one more push.

  The clock starts when the server NOTICES, never earlier. The expiry is known in advance, so that is within the
  hour, and members always get their 14 days of warning.
- **Wire.** `Family` gains `plan` and, while the family is unpaid, `delete_at`. The new frame
  `{"type": "family_plan", "family_id", "plan", "delete_at"}` keeps open apps current.
- **What members see.** Every app shows a banner on that family's chat list: *"“Ivanovs” will be deleted on
  14 November: its subscription has ended."* The owner sees "Renew" (on Apple's apps, the subscription screen; on the
  others, a sentence saying where). A member sees who the owner is.
- **What still works during the 14 days** is D2.
- **Deletion (D3)** is the existing "last member leaves" ending, reached by a new door. Every member is removed
  (frames as for a removal, so their devices drop the family), and then the family is deleted with its chats, board,
  pack and files. A member left with no family starts the ordinary familyless clock. #59 forgets the family's key, so
  a week later no backup can read it.
- **Apple keeps charging until the person cancels.** A deleted family, an account deletion, or a hand-off away from
  the subscriber does not cancel anything at Apple. Wherever one of those happens to a subscriber, the Apple apps
  say so and offer "Manage Subscription" (`AppStore.showManageSubscriptions`). Apple also asks for this in the
  account-deletion flow.

## 6. Limits for paid families (D4)

All of these are in `[subscriptions]`, so the operator tunes them without a release. The numbers below are first
guesses to argue with; the costs are at today's Azure prices.

| Limit | Default | Past it |
|---|---|---|
| Members | 25 | `family_full`, as today. The cap becomes `min(owner's cap, [limits] max_family_members, 25)`, and `Family` gains `member_ceiling`, so the owner's cap picker draws its range from the family instead of from `GET /me` |
| Disk | 10 GiB: every file the family holds — attachments, board photos, stickers, unsent uploads — each distinct file once (previews are not counted, and profile pictures are the account's, not the family's) | `POST /attachments` answers `403 storage_allowance_used` before reading the body. This is a different thing from `storage_full` (507), which means the SERVER's disk is full |
| Assistant tokens a month | 2 000 000 (under $1 on gpt-oss-120b) | the assistant answers `ai_allowance_used` instead of calling the model, as an error bubble that says when the allowance renews |
| Pictures a month | 30 | the same |
| Transcript minutes a month | 120 | the same |
| Web searches a day | the existing `daily_searches_per_family` | as today: `web_search` is not offered |

- *(built)* A renewal date (`renews_at`) is shown as its UTC calendar day — always the 1st — while `delete_at` is
  shown in the reader's own zone; the clients had disagreed, and protocol.md now says so.
- A month is the UTC calendar month. Usage is the sum of the family's `ai_usage` rows since the 1st.
- The check runs BEFORE a call, so one answer may overshoot by its own size and no call is ever cut off.
- Free families have no allowance, as today.
- **Family Statistics** gains an `allowance` block, shown only for a paid family: each limit with what is used and
  when it renews. Every member sees it, as they see the rest of the statistics.

Lowering a limit removes nothing. A family already past it simply cannot add more until it is back under, the same
rule `max_pack_items` follows.

## 7. On the wire (to be written into protocol.md after the decisions)

- `GET /me`, ALWAYS present: `subscriptions_enabled: bool`. When true, also:
  - `subscription_products: ["…family.monthly", "…family.yearly"]`;
  - `app_account_token: "uuid"`;
  - `subscription: {product_id, environment, expires_at, active, renews?} | null`, which is the caller's own;
  - `paid_family_limits: {members, storage_bytes, ai_tokens, images, transcript_ms}`, so the purchase screen can say
    what a family gets.
- `PUT /me/subscription {transaction, renewal?}` → `200 {subscription}`. Errors:
  - `subscription_invalid` (400): verification failed, or the transaction is for another app or product;
  - `subscription_taken` (409);
  - `subscriptions_disabled` (403).
- `POST /families`, with subscriptions on and the caller not covered → `403 subscription_required`. This is checked
  after `registration` and `member_families`, so a closed door still answers with its own code. An app from before
  shows its generic error, which is the usual compatibility cost.
- `Family` gains `plan`, `delete_at` (only while unpaid) and `member_ceiling`. Statistics gain `allowance`.
- New frame: `family_plan`.
- New codes: `storage_allowance_used` (403) and `ai_allowance_used`, the latter an `ai_error` reason with its own
  words.
- `POST /appstore/notifications`: unauthenticated, 64 KiB, `200` or `400`.

## 8. The apps

**iPhone, iPad, and the Mac if D1 says so.**
- When `subscriptions_enabled`, the family gate's "Create a family" and "New Family…" open the subscription screen
  first. It shows the products, what a family gets (from `paid_family_limits`), and the terms and privacy links. It
  explains that the subscription stays with the owner, and that members join free.
- After the purchase: `PUT /me/subscription`, then `POST /families`.
- Restore Purchases (`AppStore.sync()`).
- Launch and `Transaction.updates` refresh.
- In Settings, the owner's subscription row: state, renewal date, Manage Subscription.
- The unpaid banner with Renew.
- The allowance in Statistics.
- The two new errors.
- The cancellation reminders from §5.

**Android, web, Windows, Linux.**
- With `subscriptions_enabled`, the gate replaces "Create a family" and "New Family…" with *"On this server a new
  family is started in the Family Connect app on iPhone, iPad or Mac."* "Join a family" stays as it is.
- The unpaid banner (the owner's version says where to renew).
- The allowance in Statistics.
- The two new errors.
- The cap picker's range from `member_ceiling`.

**Android is the exception to that wording.** Google Play's payments policy forbids steering users to another way of
paying for digital goods. Naming the iPhone app as the place to buy is close to that line, so Android says only that a
new family cannot be started from this app (recommended), unless you would rather take the risk.

**Strings.** Every new string in all nine languages, plus the server's pushes in `server/i18n/push.json`.

**Outside the code (the owner).**
- App Store Connect: the products, grace period, notification URL and review notes. A reviewer needs a test account
  on fc.nettrash.me.
- The App Privacy answers: "Purchases", linked to the user, for app functionality.
- The privacy policy page: the server now keeps a transaction id, product, environment and expiry per subscriber.

## 9. Order of work

1. `protocol.md`, `operations.md` and `config.example.toml`.
2. Server: the JWS verifier and its tests. A test certificate chain carrying Apple's marker extensions is made with
   `rcgen` (dev-only). Negative tests cover a missing leaf marker, a missing intermediate marker, the wrong root, a
   bad signature, an expired certificate, the wrong bundle, product or environment, and a token mismatch. One test
   pins the embedded root's SHA-256.
3. Server: migration 0061, `PUT /me/subscription`, the notification route, the door in `POST /families`, the plans,
   the sweep and the deletion, the limits and allowance, the CLI. Then tests on PostgreSQL.
4. iOS/Mac: StoreKit, behind a protocol so unit tests use a fake store, plus `SKTestSession` for the purchase flow.
5. Android, web, Windows, Linux: the gate, banner, statistics and errors.
6. On a real device, the owner's step: a Sandbox purchase from a development build against a test server. Then
   check the journal, the banner and the expiry, which Sandbox makes take minutes, not a month.

It ships in two steps without breaking anybody: the server first (the switch is off by default, and old apps never
see a difference), then the apps. Turn the switch on fc.nettrash.me only once the Apple app that can buy is on the
App Store.

## 10. Decisions

- **D1 — Where a family can be bought.**
  - **(a) iPhone, iPad and Mac.** One app and one purchase, so the Mac costs no extra code, only its own App Review
    pass.
  - (b) iPhone and iPad only, as the owner wrote. The Mac shows the "start it on iPhone" sentence, but still sends the
    subscription it can see.

  **DECIDED (a).**
- **D2 — What still works during the 14 unpaid days.**
  - **(a) Everything**, with the banner to every member. That is what the issue's words say, and it is the cheapest.
  - (b) Read-only: history stays readable and files can still be saved, but nothing new can be sent. Every write
    endpoint and every app's composer has to learn it.
  - (c) Messages only: no assistant, no new uploads. These are the two things that cost money.

  **DECIDED (a).**
- **D3 — Day 14.**
  - **(a) The family is deleted**, as the issue says.
  - (b) It is frozen until the operator decides.

  **DECIDED (a).**
- **D4 — Limits.**
  - **(a) Paid families only**, at the §6 defaults. Free families stay unlimited, as today.
  - (b) Every family, with the operator lifting them per family.

  **DECIDED (a).**
- **D5 — Families that exist when the switch first turns on.**
  - **(a) All stay free**, and the operator moves any to `paid` with the CLI.
  - (b) Only the oldest family is free, and every other family gets its 14 days on the day the switch turns on.

  **DECIDED (a).**
- **D6 — Android's wording.**
  - **(a) Neutral:** "can't be started from this app".
  - (b) Name the iPhone app, as the other platforms do.

  **DECIDED (a).**
