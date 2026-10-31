//
//  PaidFamilies.swift
//  FamilyConnect
//
//  Paid families (docs/protocol.md, "Paid families", 2026-10-30; the plan
//  and its decisions are docs/paid-families-2026-10-30.md, D1–D6). A server
//  whose operator turns `[subscriptions] enabled` on hosts a NEW family only
//  for its owner's App Store subscription; a paid family whose owner stops
//  paying is deleted `unpaid_days` later, with a banner to every member
//  until then; and a paid family has limits — members, storage, and a
//  monthly assistant allowance.
//
//  This app is the one that SELLS the subscription (iPhone, iPad and Mac:
//  one target, one bundle id, one purchase — D1). The selling is
//  `SubscriptionManager`; every DECISION about what is shown is here, as
//  pure functions over values, so it can be read and tested as tables:
//  whether starting a family needs the subscription screen first, which
//  banner a family's chat list carries, what the Settings row says, what
//  the subscription screen promises, the allowance rows of Family
//  Statistics, and what each new refusal says.
//
//  With `subscriptions_enabled` false or absent nothing here is consulted
//  and nothing on screen changes.
//

import Foundation

nonisolated enum PaidFamilies {

    // MARK: - Starting a family

    /// Whether "Create a family" and "New Family…" go through the
    /// subscription screen before `POST /families`: only on a server that
    /// charges, and only for somebody whose subscription is not active. A
    /// subscriber starts a family exactly as before.
    static func needsSubscription(subscriptionsEnabled: Bool, subscription: SubscriptionDTO?) -> Bool {
        subscriptionsEnabled && subscription?.active != true
    }

    // MARK: - Dates

    /// The server's calendar: a month of allowance is a UTC calendar month.
    static let utc = TimeZone(identifier: "UTC") ?? .gmt

    /// A day as the person reads it — day and month, and the year only when
    /// it is not this one ("14 November", "2 January 2027"), in their own
    /// language and calendar.
    static func day(
        _ date: Date, timeZone: TimeZone = .current, now: Date = .now,
        locale: Locale = .autoupdatingCurrent
    ) -> String {
        var calendar = locale.calendar
        calendar.timeZone = timeZone
        var style = Date.FormatStyle(locale: locale, calendar: calendar, timeZone: timeZone)
            .day().month(.wide)
        if calendar.component(.year, from: date) != calendar.component(.year, from: now) {
            style = style.year()
        }
        return date.formatted(style)
    }

    /// The day a monthly allowance starts again. In UTC, deliberately: the
    /// server's month turns over at midnight UTC, and `renews_at` is that
    /// moment — read in a time zone west of Greenwich it would be the last
    /// day of the old month, and "renews on 30 November" names the wrong
    /// month for an allowance of December's.
    static func renewalDay(_ date: Date, now: Date = .now, locale: Locale = .autoupdatingCurrent) -> String {
        day(date, timeZone: utc, now: now, locale: locale)
    }

    /// The start of the next UTC calendar month after `date` — what the
    /// server's `renews_at` is, and what a REST `ai_allowance_used` (which
    /// carries no date) means.
    static func nextMonthStartUTC(after date: Date) -> Date {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = utc
        let thisMonth = calendar.dateComponents([.year, .month], from: date)
        let start = calendar.date(from: thisMonth) ?? date
        return calendar.date(byAdding: .month, value: 1, to: start) ?? date
    }

    // MARK: - The unpaid banner

    /// What the banner over a family's chat list says.
    enum Banner: Equatable, Sendable {
        /// The owner, who can renew: the Apple apps put a Renew button
        /// beside it.
        case owner(String)
        /// Everybody else, told who CAN fix it.
        case member(String)
    }

    /// The banner for the family on screen, or nil for none: a family
    /// without `delete_at` has nothing to warn about. A member's banner
    /// names the owner from the roster, so it waits for the roster to say
    /// who that is rather than drawing a sentence with a hole in it.
    static func banner(
        family: FamilyDTO?, isOwner: Bool, ownerName: String?,
        timeZone: TimeZone = .current, now: Date = .now, locale: Locale = .autoupdatingCurrent
    ) -> Banner? {
        guard let family, let deleteAt = family.deleteAt else { return nil }
        let date = day(deleteAt, timeZone: timeZone, now: now, locale: locale)
        if isOwner {
            return .owner(String(
                localized: "“\(family.name)” will be deleted on \(date) because your subscription has ended."))
        }
        guard let ownerName, !ownerName.isEmpty else { return nil }
        return .member(String(
            localized: "“\(family.name)” will be deleted on \(date) because its subscription has ended. Only \(ownerName), the owner, can renew it."))
    }

    // MARK: - Settings

    /// Whether Settings carries the subscription row: for a subscriber, and
    /// for the owner of a paid family who is not one (an inherited family,
    /// say), who is the person able to pay for it.
    static func showsSubscriptionRow(
        subscriptionsEnabled: Bool, subscription: SubscriptionDTO?, isOwner: Bool, family: FamilyDTO?
    ) -> Bool {
        guard subscriptionsEnabled else { return false }
        return subscription != nil || (isOwner && family?.plan == "paid")
    }

    /// The row's value: when it renews, when it ends, or when it ended. A
    /// subscription whose renewal the server does not know about is shown
    /// as renewing, which is what an auto-renewable subscription does until
    /// somebody cancels it.
    static func subscriptionStatus(
        _ subscription: SubscriptionDTO, timeZone: TimeZone = .current, now: Date = .now,
        locale: Locale = .autoupdatingCurrent
    ) -> String {
        guard subscription.active else {
            // Revoked or refunded before its expiry: it ended then, not on
            // a day still ahead.
            let ended = day(min(subscription.expiresAt, now), timeZone: timeZone, now: now, locale: locale)
            return String(localized: "Ended on \(ended)")
        }
        let date = day(subscription.expiresAt, timeZone: timeZone, now: now, locale: locale)
        if subscription.renews == false {
            return String(localized: "Ends on \(date)")
        }
        // Its own key: "Renews on %@" is the allowance's, and the two read
        // differently in German, Japanese, Russian and Chinese.
        return String(localized: "Subscription renews on \(date)")
    }

    /// Apple goes on charging after an account is deleted, and asks every
    /// app that sells a subscription to say so in that flow — while the
    /// subscription is active.
    static func keepsRenewingOnDelete(subscriptionsEnabled: Bool, subscription: SubscriptionDTO?) -> Bool {
        subscriptionsEnabled && subscription?.active == true
    }

    /// The same, in the owner's leave dialog for a PAID family: leaving
    /// hands the family on and cancels nothing at Apple.
    static func keepsRenewingOnLeave(
        subscriptionsEnabled: Bool, subscription: SubscriptionDTO?, isOwner: Bool, family: FamilyDTO?
    ) -> Bool {
        subscriptionsEnabled && isOwner && family?.plan == "paid" && subscription?.active == true
    }

    // MARK: - The subscription screen

    /// One line of what a family gets.
    struct PaywallLine: Equatable, Identifiable, Sendable {
        let id: String
        let text: String
        let systemImage: String
    }

    /// What a paid family gets on THIS server: each limit only when the
    /// operator has it on, and the assistant only on a server that has one
    /// and limits it — a line about an assistant this server does not run
    /// would be a promise it cannot keep.
    static func paywallLines(limits: PaidFamilyLimitsDTO?, serverHasAssistant: Bool) -> [PaywallLine] {
        guard let limits else { return [] }
        var lines: [PaywallLine] = []
        if let members = limits.members {
            lines.append(PaywallLine(
                id: "members", text: String(localized: "Up to \(members) members"), systemImage: "person.3"))
        }
        if let storage = limits.storageBytes {
            lines.append(PaywallLine(
                id: "storage",
                text: String(localized: "\(bytes(storage)) for photos, videos and files"),
                systemImage: "externaldrive"))
        }
        if serverHasAssistant && limits.limitsAssistant {
            lines.append(PaywallLine(
                id: "assistant", text: String(localized: "The assistant, within a monthly allowance"),
                systemImage: "sparkles"))
        }
        return lines
    }

    /// The privacy policy the app already links from Settings.
    static let privacyPolicyURL = URL(string: "https://nettrash.me/appstore/familyconnect/privacy.html")!
    /// Apple's standard licence agreement — the terms of use App Review asks
    /// a subscription screen to link (guideline 3.1.2).
    static let termsOfUseURL = URL(string: "https://www.apple.com/legal/internet-services/itunes/dev/stdeula/")!
    /// Where a Mac manages its subscriptions: StoreKit offers no sheet there.
    static let manageSubscriptionsURL = URL(string: "https://apps.apple.com/account/subscriptions")!

    // MARK: - Family Statistics

    /// One row of "This Month’s Allowance".
    struct AllowanceRow: Equatable, Identifiable, Sendable {
        let id: String
        let label: String
        let value: String
    }

    /// One row per limit the server reported, in the protocol's order.
    static func allowanceRows(_ allowance: AllowanceDTO) -> [AllowanceRow] {
        var rows: [AllowanceRow] = []
        if let storage = allowance.storageBytes {
            rows.append(AllowanceRow(
                id: "storage", label: String(localized: "Storage"),
                value: String(localized: "\(bytes(storage.used)) of \(bytes(storage.limit))")))
        }
        if let tokens = allowance.aiTokens {
            let percent = percentUsed(used: tokens.used, limit: tokens.limit)
            rows.append(AllowanceRow(
                id: "assistant", label: String(localized: "Assistant"),
                value: String(localized: "\(percent)% used")))
        }
        if let images = allowance.images {
            rows.append(AllowanceRow(
                id: "pictures", label: String(localized: "Pictures"),
                value: String(localized: "\(images.used.formatted()) of \(images.limit.formatted())")))
        }
        if let transcripts = allowance.transcriptMS {
            let minutes = minutes(usedMS: transcripts.used, limitMS: transcripts.limit)
            rows.append(AllowanceRow(
                id: "transcripts", label: String(localized: "Transcripts"),
                value: String(localized: "\(minutes.used) of \(minutes.limit) min")))
        }
        return rows
    }

    /// The section's last line.
    static func allowanceRenews(_ allowance: AllowanceDTO, now: Date = .now) -> String {
        String(localized: "Renews on \(renewalDay(allowance.renewsAt, now: now))")
    }

    /// Tokens used as a whole percentage of the limit — rounded DOWN, so it
    /// never says 100 before the allowance is actually spent — and capped
    /// at 100, since one answer may overshoot.
    static func percentUsed(used: Int64, limit: Int64) -> Int {
        guard limit > 0 else { return 100 }
        let used = max(0, used)
        guard used < limit else { return 100 }
        let (scaled, overflow) = used.multipliedReportingOverflow(by: 100)
        return Int(overflow ? used / max(1, limit / 100) : scaled / limit)
    }

    /// Transcript time as whole minutes: what was used rounded UP (half a
    /// minute is a minute gone), the limit rounded DOWN (never promising a
    /// minute that is not there).
    static func minutes(usedMS: Int64, limitMS: Int64) -> (used: Int64, limit: Int64) {
        let minute: Int64 = 60_000
        let used = max(0, usedMS)
        return ((used + minute - 1) / minute, max(0, limitMS) / minute)
    }

    /// `3 GB`, in the reader's own units and language — the file-size style
    /// Statistics already uses.
    static func bytes(_ count: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: count, countStyle: .file)
    }

    // MARK: - What a refusal says

    /// `subscription_required` (403) from `POST /families`: the caller's
    /// subscription lapsed between the screen and the create. Terminal.
    static var subscriptionRequired: String {
        String(localized: "A new family on this server needs a subscription.")
    }

    /// `storage_allowance_used` (403) on an upload. Terminal: the send fails
    /// visibly and is not retried.
    static var storageAllowanceUsed: String {
        String(localized: "This family has used its storage allowance.")
    }

    /// The assistant's allowance is used until `renewsAt`.
    static func assistantAllowanceUsed(renewsAt: Date, now: Date = .now) -> String {
        String(localized: "This family has used this month’s assistant allowance. It renews on \(renewalDay(renewsAt, now: now)).")
    }

    /// What `PUT /me/subscription`'s two terminal refusals say, or nil for
    /// any other code.
    static func subscriptionRefusal(code: String) -> String? {
        switch code {
        case "subscription_invalid":
            String(localized: "The App Store purchase couldn’t be confirmed. Try Restore Purchases.")
        case "subscription_taken":
            String(localized: "This App Store subscription belongs to another account on this server.")
        default:
            nil
        }
    }

    /// The words for the paid-family refusals, or nil for any other error —
    /// the caller keeps its own copy for those.
    static func failureMessage(_ error: Error, now: Date = .now) -> String? {
        switch error {
        case APIError.forbidden(let code) where code == "subscription_required":
            return subscriptionRequired
        case APIError.conflict(let code?, _) where subscriptionRefusal(code: code) != nil:
            return subscriptionRefusal(code: code)
        case APIError.forbidden(let code) where code == "storage_allowance_used":
            return storageAllowanceUsed
        case APIError.forbidden(let code) where code == "ai_allowance_used":
            return assistantAllowanceUsed(renewsAt: nextMonthStartUTC(after: now), now: now)
        default:
            return nil
        }
    }
}
