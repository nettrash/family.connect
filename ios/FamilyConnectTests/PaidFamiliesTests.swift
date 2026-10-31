//
//  PaidFamiliesTests.swift
//  FamilyConnectTests
//
//  Paid families (docs/protocol.md, "Paid families", 2026-10-30; the
//  decisions are docs/paid-families-2026-10-30.md): the wire — the new
//  `GET /me` and `Family` keys, the allowance, the `family_plan` frame, the
//  `allowance_used` reason, `PUT /me/subscription` — the rules table, the
//  words of each new refusal, the session's state, and the coordinator's
//  handling of the frame and of an upload refused for storage.
//
//  The server is being written at the same time from the same protocol, so
//  everything here runs against fixtures written from the protocol's text.
//

import Foundation
import SwiftData
import Testing
@testable import FamilyConnect

private enum Fixture {
    /// User 77, not the 7 the other suites use: the family a session shows
    /// is remembered per user in UserDefaults, and suites run side by side.
    static let user = UserDTO(id: 77, username: "anna", displayName: "Anna", createdAt: nil)
    static let utc = TimeZone(identifier: "UTC")!
    static let us = Locale(identifier: "en_US")
    static let gb = Locale(identifier: "en_GB")
    /// 2026-10-30T12:00:00Z — "now" for every table below.
    static let now = Date(timeIntervalSince1970: 1_793_361_600)
    /// 2026-11-14T09:00:00Z
    static let deleteAt = Date(timeIntervalSince1970: 1_794_646_800)

    static func family(
        _ id: Int64 = 3, _ name: String = "The Smiths", role: String? = "owner",
        plan: String? = "paid", ceiling: Int? = 25, deleteAt: Date? = nil
    ) -> FamilyDTO {
        FamilyDTO(
            id: id, name: name, joinPolicy: "open", createdAt: nil, inviteCode: nil,
            aiVision: false, aiHistoryPhotos: false, aiGreeting: false, aiFaces: false,
            aiTranscripts: false, aiLookups: false, greetingPlaces: [], maxMembers: nil,
            plan: plan, memberCeiling: ceiling, deleteAt: deleteAt, role: role)
    }

    static func subscription(active: Bool = true, renews: Bool? = true, expires: Date = deleteAt) -> SubscriptionDTO {
        SubscriptionDTO(
            productID: "me.nettrash.FamilyConnect.family.yearly", environment: "Production",
            expiresAt: expires, active: active, renews: renews)
    }

    static func date(_ text: String) -> Date {
        ISO8601DateFormatter().date(from: text)!
    }
}

// MARK: - The wire

@Suite("Paid families: the wire")
struct PaidFamiliesWireTests {

    @Test("GET /me carries the subscription keys when the server charges")
    func meDecodes() throws {
        let json = #"""
        {"user": {"id": 7, "username": "anna", "display_name": "Anna"},
         "family": null, "role": null, "pending_join_request": null,
         "subscriptions_enabled": true,
         "subscription_products": ["me.nettrash.FamilyConnect.family.monthly", "me.nettrash.FamilyConnect.family.yearly"],
         "app_account_token": "6f1c2d3e-4a5b-4c6d-8e7f-a1b2c3d4e5f6",
         "unpaid_days": 14,
         "paid_family_limits": {"members": 25, "storage_bytes": 10737418240, "ai_tokens": 2000000,
                                "images": 30, "transcript_ms": 7200000},
         "subscription": {"product_id": "me.nettrash.FamilyConnect.family.yearly", "environment": "Sandbox",
                          "expires_at": "2026-11-14T09:00:00Z", "active": true, "renews": false}}
        """#
        let me = try APICoding.decoder().decode(MeResponse.self, from: Data(json.utf8))
        #expect(me.subscriptionsEnabled)
        #expect(me.subscriptionProducts == [
            "me.nettrash.FamilyConnect.family.monthly", "me.nettrash.FamilyConnect.family.yearly"])
        #expect(me.appAccountToken == "6f1c2d3e-4a5b-4c6d-8e7f-a1b2c3d4e5f6")
        #expect(me.unpaidDays == 14)
        #expect(me.paidFamilyLimits == PaidFamilyLimitsDTO(
            members: 25, storageBytes: 10_737_418_240, aiTokens: 2_000_000, images: 30, transcriptMS: 7_200_000))
        #expect(me.subscription == SubscriptionDTO(
            productID: "me.nettrash.FamilyConnect.family.yearly", environment: "Sandbox",
            expiresAt: Fixture.deleteAt, active: true, renews: false))
    }

    @Test("a server from before paid families: off, and nothing else")
    func meWithoutKeys() throws {
        let json = #"""
        {"user": {"id": 7, "username": "anna", "display_name": "Anna"},
         "family": null, "role": null, "pending_join_request": null}
        """#
        let me = try APICoding.decoder().decode(MeResponse.self, from: Data(json.utf8))
        #expect(!me.subscriptionsEnabled)
        #expect(me.subscriptionProducts.isEmpty)
        #expect(me.appAccountToken == nil)
        #expect(me.paidFamilyLimits == nil)
        #expect(me.subscription == nil)
    }

    @Test("limits are each present only while on; a subscription without `renews` is unknown")
    func partialLimits() throws {
        let limits = try APICoding.decoder().decode(
            PaidFamilyLimitsDTO.self, from: Data(#"{"members": 25}"#.utf8))
        #expect(limits.members == 25 && limits.storageBytes == nil && !limits.limitsAssistant)
        let images = try APICoding.decoder().decode(PaidFamilyLimitsDTO.self, from: Data(#"{"images": 30}"#.utf8))
        #expect(images.limitsAssistant)
        let subscription = try APICoding.decoder().decode(SubscriptionDTO.self, from: Data(#"""
            {"product_id": "p", "environment": "Production", "expires_at": "2026-11-14T09:00:00Z", "active": false}
            """#.utf8))
        #expect(subscription.renews == nil)
        #expect(!subscription.active)
    }

    @Test("Family carries plan, member_ceiling and delete_at; absent is the old behaviour")
    func familyDecodes() throws {
        let paid = try APICoding.decoder().decode(FamilyDTO.self, from: Data(#"""
            {"id": 3, "name": "The Smiths", "join_policy": "open", "plan": "paid",
             "member_ceiling": 25, "delete_at": "2026-11-14T09:00:00Z"}
            """#.utf8))
        #expect(paid.plan == "paid")
        #expect(paid.memberCeiling == 25)
        #expect(paid.deleteAt == Fixture.deleteAt)
        let old = try APICoding.decoder().decode(FamilyDTO.self, from: Data(#"""
            {"id": 3, "name": "The Smiths", "join_policy": "open"}
            """#.utf8))
        #expect(old.plan == nil && old.memberCeiling == nil && old.deleteAt == nil)
    }

    @Test("statistics carry the allowance for a paid family, and nothing for a free one")
    func allowanceDecodes() throws {
        let totals = #"""
        "totals": {"members": 2, "messages": 10, "board_notes": 0,
                   "attachments": {"count": 0, "bytes": 0, "photo": 0, "video": 0, "audio": 0, "file": 0},
                   "ai": {"questions": 0, "prompt_tokens": 0, "completion_tokens": 0}},
        "members": []
        """#
        let paid = try APICoding.decoder().decode(FamilyStatsDTO.self, from: Data(#"""
            {\#(totals), "allowance": {"renews_at": "2026-12-01T00:00:00Z",
              "storage_bytes": {"used": 3221225472, "limit": 10737418240},
              "ai_tokens": {"used": 812345, "limit": 2000000},
              "images": {"used": 4, "limit": 30},
              "transcript_ms": {"used": 600000, "limit": 7200000}}}
            """#.utf8))
        let allowance = try #require(paid.allowance)
        #expect(allowance.renewsAt == Fixture.date("2026-12-01T00:00:00Z"))
        #expect(allowance.storageBytes == AllowanceUsageDTO(used: 3_221_225_472, limit: 10_737_418_240))
        #expect(allowance.aiTokens?.used == 812_345)
        #expect(allowance.images == AllowanceUsageDTO(used: 4, limit: 30))
        #expect(allowance.transcriptMS?.limit == 7_200_000)
        let free = try APICoding.decoder().decode(FamilyStatsDTO.self, from: Data("{\(totals)}".utf8))
        #expect(free.allowance == nil)
    }

    @Test("family_plan: delete_at sets, and its absence clears")
    func familyPlanFrame() throws {
        let set = try APICoding.decoder().decode(ServerFrame.self, from: Data(#"""
            {"type": "family_plan", "family_id": 3, "plan": "paid", "delete_at": "2026-11-14T09:00:00Z"}
            """#.utf8))
        #expect(set == .familyPlan(familyID: 3, plan: "paid", deleteAt: Fixture.deleteAt))
        let cleared = try APICoding.decoder().decode(ServerFrame.self, from: Data(#"""
            {"type": "family_plan", "family_id": 3, "plan": "paid"}
            """#.utf8))
        #expect(cleared == .familyPlan(familyID: 3, plan: "paid", deleteAt: nil))
    }

    @Test("ai_error allowance_used carries renews_at; an unknown reason is still absent")
    func allowanceReason() throws {
        let used = try APICoding.decoder().decode(ServerFrame.self, from: Data(#"""
            {"type": "ai_error", "chat_id": 42, "message_id": 1339, "reason": "allowance_used",
             "renews_at": "2026-12-01T00:00:00Z"}
            """#.utf8))
        #expect(used == .aiError(
            chatID: 42, messageID: 1339, reason: .allowanceUsed(renewsAt: Fixture.date("2026-12-01T00:00:00Z"))))
        let undated = try APICoding.decoder().decode(ServerFrame.self, from: Data(#"""
            {"type": "ai_error", "chat_id": 42, "message_id": 1339, "reason": "allowance_used"}
            """#.utf8))
        #expect(undated == .aiError(chatID: 42, messageID: 1339, reason: .allowanceUsed(renewsAt: nil)))
        let refused = try APICoding.decoder().decode(ServerFrame.self, from: Data(#"""
            {"type": "ai_error", "chat_id": 42, "message_id": 1339, "reason": "refused"}
            """#.utf8))
        #expect(refused == .aiError(chatID: 42, messageID: 1339, reason: .refused))
        let unknown = try APICoding.decoder().decode(ServerFrame.self, from: Data(#"""
            {"type": "ai_error", "chat_id": 42, "message_id": 1339, "reason": "quota_moon"}
            """#.utf8))
        #expect(unknown == .aiError(chatID: 42, messageID: 1339, reason: nil))
    }

    @Test("PUT /me/subscription sends both JWS, leaves out a missing renewal, and reads the answer")
    func putSubscription() async throws {
        let host = "paid-put-subscription.test"
        StubURLProtocol.register(host: host) { _ in
            .json(200, #"""
            {"subscription": {"product_id": "me.nettrash.FamilyConnect.family.yearly", "environment": "Production",
                              "expires_at": "2026-11-14T09:00:00Z", "active": true, "renews": true}}
            """#)
        }
        defer { StubURLProtocol.unregister(host: host) }
        let api = APIClient(serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())

        let answer = try await api.putSubscription(transaction: "t.jws", renewal: "r.jws")
        #expect(answer == Fixture.subscription(active: true, renews: true))
        _ = try await api.putSubscription(transaction: "t2.jws", renewal: nil)

        let requests = StubURLProtocol.requests(host: host)
        #expect(requests.count == 2)
        #expect(requests[0].method == "PUT")
        #expect(requests[0].url.path() == "/api/v1/me/subscription")
        #expect(requests[0].bodyJSON()?["transaction"] as? String == "t.jws")
        #expect(requests[0].bodyJSON()?["renewal"] as? String == "r.jws")
        #expect(requests[1].bodyJSON()?["transaction"] as? String == "t2.jws")
        #expect(requests[1].bodyJSON()?.keys.contains("renewal") == false)
    }

    @Test("the server's verdicts arrive as the codes the manager finishes on")
    func putSubscriptionRefusals() async throws {
        let host = "paid-put-refusals.test"
        StubURLProtocol.register(host: host) { request in
            request.bodyJSON()?["transaction"] as? String == "taken"
                ? .json(409, #"{"error": {"code": "subscription_taken", "message": "no"}}"#)
                : .json(400, #"{"error": {"code": "subscription_invalid", "message": "no"}}"#)
        }
        defer { StubURLProtocol.unregister(host: host) }
        let api = APIClient(serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())
        await #expect(throws: APIError.conflict(code: "subscription_taken", message: "no")) {
            _ = try await api.putSubscription(transaction: "taken", renewal: nil)
        }
        await #expect(throws: APIError.conflict(code: "subscription_invalid", message: "no")) {
            _ = try await api.putSubscription(transaction: "forged", renewal: nil)
        }
    }
}

// MARK: - The rules

@Suite("Paid families: the rules")
struct PaidFamiliesRulesTests {

    @Test("starting a family needs the screen only on a server that charges, without an active subscription")
    func needsSubscription() {
        #expect(!PaidFamilies.needsSubscription(subscriptionsEnabled: false, subscription: nil))
        #expect(PaidFamilies.needsSubscription(subscriptionsEnabled: true, subscription: nil))
        #expect(PaidFamilies.needsSubscription(
            subscriptionsEnabled: true, subscription: Fixture.subscription(active: false)))
        #expect(!PaidFamilies.needsSubscription(
            subscriptionsEnabled: true, subscription: Fixture.subscription(active: true)))
    }

    @Test("the next UTC month starts on the 1st at midnight, across a year too")
    func nextMonth() {
        #expect(PaidFamilies.nextMonthStartUTC(after: Fixture.now) == Fixture.date("2026-11-01T00:00:00Z"))
        #expect(PaidFamilies.nextMonthStartUTC(after: Fixture.date("2026-12-31T23:59:59Z"))
            == Fixture.date("2027-01-01T00:00:00Z"))
        #expect(PaidFamilies.nextMonthStartUTC(after: Fixture.date("2026-11-01T00:00:00Z"))
            == Fixture.date("2026-12-01T00:00:00Z"))
    }

    @Test("a day is day and month, with the year only when it is not this one")
    func dayFormat() {
        #expect(PaidFamilies.day(Fixture.deleteAt, timeZone: Fixture.utc, now: Fixture.now, locale: Fixture.gb)
            == "14 November")
        #expect(PaidFamilies.day(Fixture.deleteAt, timeZone: Fixture.utc, now: Fixture.now, locale: Fixture.us)
            == "November 14")
        #expect(PaidFamilies.day(
            Fixture.date("2027-01-02T09:00:00Z"), timeZone: Fixture.utc, now: Fixture.now, locale: Fixture.gb)
            == "2 January 2027")
        // The deletion day is the reader's: 23:30 UTC is already the next day in Belgrade.
        let belgrade = TimeZone(identifier: "Europe/Belgrade")!
        #expect(PaidFamilies.day(
            Fixture.date("2026-11-14T23:30:00Z"), timeZone: belgrade, now: Fixture.now, locale: Fixture.gb)
            == "15 November")
    }

    /// The lead's rule (docs/protocol.md, "UTC calendar day"): an allowance's
    /// renewal is shown as its UTC calendar day — always the 1st, never "30
    /// November" west of Greenwich — while `delete_at` and the subscription's
    /// own dates are moments, shown in the reader's zone.
    @Test("an allowance renews on its UTC calendar day; delete_at and expires_at are the reader's")
    func renewalDayIsUTC() {
        let renews = Fixture.date("2026-12-01T00:00:00Z")
        let losAngeles = TimeZone(identifier: "America/Los_Angeles")!
        // The same moment, read as a moment in Los Angeles, is still November…
        #expect(PaidFamilies.day(renews, timeZone: losAngeles, now: Fixture.now, locale: Fixture.gb) == "30 November")
        // …but the allowance renews on the 1st, whatever the reader's zone.
        #expect(PaidFamilies.renewalDay(renews, now: Fixture.now, locale: Fixture.gb) == "1 December")
        #expect(PaidFamilies.assistantAllowanceUsed(renewsAt: renews, now: Fixture.now)
            == "This family has used this month’s assistant allowance. It renews on \(PaidFamilies.renewalDay(renews, now: Fixture.now)).")
        let allowance = AllowanceDTO(renewsAt: renews)
        #expect(PaidFamilies.allowanceRenews(allowance, now: Fixture.now)
            == "Renews on \(PaidFamilies.renewalDay(renews, now: Fixture.now))")
        // A banner and a Settings row read their moment in the zone they are given.
        let late = Fixture.date("2026-11-15T03:00:00Z")
        #expect(PaidFamilies.banner(
            family: Fixture.family(deleteAt: late), isOwner: true, ownerName: nil,
            timeZone: losAngeles, now: Fixture.now, locale: Fixture.gb)
            == .owner("“The Smiths” will be deleted on 14 November because your subscription has ended."))
        #expect(PaidFamilies.subscriptionStatus(
            Fixture.subscription(active: true, renews: false, expires: late),
            timeZone: losAngeles, now: Fixture.now, locale: Fixture.gb) == "Ends on 14 November")
    }

    @Test("the banner: none without delete_at; the owner's; a member's naming the owner")
    func banner() {
        #expect(PaidFamilies.banner(family: Fixture.family(), isOwner: true, ownerName: "Anna") == nil)
        #expect(PaidFamilies.banner(family: nil, isOwner: true, ownerName: "Anna") == nil)
        let unpaid = Fixture.family(deleteAt: Fixture.deleteAt)
        #expect(PaidFamilies.banner(
            family: unpaid, isOwner: true, ownerName: nil,
            timeZone: Fixture.utc, now: Fixture.now, locale: Fixture.gb)
            == .owner("“The Smiths” will be deleted on 14 November because your subscription has ended."))
        #expect(PaidFamilies.banner(
            family: unpaid, isOwner: false, ownerName: "Olga",
            timeZone: Fixture.utc, now: Fixture.now, locale: Fixture.gb)
            == .member("“The Smiths” will be deleted on 14 November because its subscription has ended. Only Olga, the owner, can renew it."))
        // A member's banner waits for the roster rather than drawing a hole.
        #expect(PaidFamilies.banner(family: unpaid, isOwner: false, ownerName: nil) == nil)
    }

    @Test("the Settings row: renews, ends, ended")
    func subscriptionStatus() {
        func status(_ subscription: SubscriptionDTO) -> String {
            PaidFamilies.subscriptionStatus(subscription, timeZone: Fixture.utc, now: Fixture.now, locale: Fixture.gb)
        }
        #expect(status(Fixture.subscription(active: true, renews: true)) == "Renews on 14 November")
        #expect(status(Fixture.subscription(active: true, renews: nil)) == "Renews on 14 November")
        #expect(status(Fixture.subscription(active: true, renews: false)) == "Ends on 14 November")
        #expect(status(Fixture.subscription(active: false, expires: Fixture.date("2026-10-02T09:00:00Z")))
            == "Ended on 2 October")
        // Refunded before its expiry: it ended, not on a day still ahead.
        #expect(status(Fixture.subscription(active: false, expires: Fixture.deleteAt)) == "Ended on 30 October")
    }

    @Test("who sees the row, and who is told the subscription keeps renewing")
    func whoIsTold() {
        let paid = Fixture.family()
        let free = Fixture.family(plan: "free")
        let active = Fixture.subscription(active: true)
        let ended = Fixture.subscription(active: false)
        #expect(!PaidFamilies.showsSubscriptionRow(
            subscriptionsEnabled: false, subscription: active, isOwner: true, family: paid))
        #expect(PaidFamilies.showsSubscriptionRow(
            subscriptionsEnabled: true, subscription: active, isOwner: false, family: free))
        #expect(PaidFamilies.showsSubscriptionRow(
            subscriptionsEnabled: true, subscription: nil, isOwner: true, family: paid))
        #expect(!PaidFamilies.showsSubscriptionRow(
            subscriptionsEnabled: true, subscription: nil, isOwner: true, family: free))
        #expect(!PaidFamilies.showsSubscriptionRow(
            subscriptionsEnabled: true, subscription: nil, isOwner: false, family: paid))

        #expect(PaidFamilies.keepsRenewingOnDelete(subscriptionsEnabled: true, subscription: active))
        #expect(!PaidFamilies.keepsRenewingOnDelete(subscriptionsEnabled: true, subscription: ended))
        #expect(!PaidFamilies.keepsRenewingOnDelete(subscriptionsEnabled: true, subscription: nil))
        #expect(!PaidFamilies.keepsRenewingOnDelete(subscriptionsEnabled: false, subscription: active))

        #expect(PaidFamilies.keepsRenewingOnLeave(
            subscriptionsEnabled: true, subscription: active, isOwner: true, family: paid))
        #expect(!PaidFamilies.keepsRenewingOnLeave(
            subscriptionsEnabled: true, subscription: active, isOwner: true, family: free))
        #expect(!PaidFamilies.keepsRenewingOnLeave(
            subscriptionsEnabled: true, subscription: active, isOwner: false, family: paid))
        #expect(!PaidFamilies.keepsRenewingOnLeave(
            subscriptionsEnabled: true, subscription: ended, isOwner: true, family: paid))
    }

    @Test("the screen promises only the limits that are on, and the assistant only where there is one")
    func paywallLines() {
        let all = PaidFamilyLimitsDTO(
            members: 25, storageBytes: 10_000_000_000, aiTokens: 2_000_000, images: 30, transcriptMS: 7_200_000)
        let lines = PaidFamilies.paywallLines(limits: all, serverHasAssistant: true)
        #expect(lines.map(\.id) == ["members", "storage", "assistant"])
        #expect(lines[0].text == "Up to 25 members")
        #expect(lines[1].text == "\(PaidFamilies.bytes(10_000_000_000)) for photos, videos and files")
        #expect(lines[2].text == "The assistant, within a monthly allowance")
        #expect(PaidFamilies.paywallLines(limits: all, serverHasAssistant: false).map(\.id) == ["members", "storage"])
        #expect(PaidFamilies.paywallLines(
            limits: PaidFamilyLimitsDTO(members: 25), serverHasAssistant: true).map(\.id) == ["members"])
        #expect(PaidFamilies.paywallLines(limits: nil, serverHasAssistant: true).isEmpty)
    }

    @Test("allowance rows: sizes, a whole percentage, whole numbers, minutes")
    func allowanceRows() {
        let allowance = AllowanceDTO(
            renewsAt: Fixture.date("2026-12-01T00:00:00Z"),
            storageBytes: AllowanceUsageDTO(used: 3_221_225_472, limit: 10_737_418_240),
            aiTokens: AllowanceUsageDTO(used: 812_345, limit: 2_000_000),
            images: AllowanceUsageDTO(used: 4, limit: 30),
            transcriptMS: AllowanceUsageDTO(used: 600_001, limit: 7_259_999))
        let rows = PaidFamilies.allowanceRows(allowance)
        #expect(rows.map(\.label) == ["Storage", "Assistant", "Pictures", "Transcripts"])
        #expect(rows[0].value == "\(PaidFamilies.bytes(3_221_225_472)) of \(PaidFamilies.bytes(10_737_418_240))")
        #expect(rows[1].value == "40% used")
        #expect(rows[2].value == "4 of 30")
        #expect(rows[3].value == "11 of 120 min")
        #expect(PaidFamilies.allowanceRenews(allowance, now: Fixture.now).hasPrefix("Renews on "))

        let onlyImages = AllowanceDTO(renewsAt: Fixture.now, images: AllowanceUsageDTO(used: 0, limit: 30))
        #expect(PaidFamilies.allowanceRows(onlyImages).map(\.id) == ["pictures"])
    }

    @Test("the percentage rounds down and stops at 100")
    func percent() {
        #expect(PaidFamilies.percentUsed(used: 0, limit: 2_000_000) == 0)
        #expect(PaidFamilies.percentUsed(used: 1_999_999, limit: 2_000_000) == 99)
        #expect(PaidFamilies.percentUsed(used: 2_000_000, limit: 2_000_000) == 100)
        #expect(PaidFamilies.percentUsed(used: 2_400_000, limit: 2_000_000) == 100, "one answer may overshoot")
        #expect(PaidFamilies.percentUsed(used: -5, limit: 100) == 0)
        #expect(PaidFamilies.percentUsed(used: 5, limit: 0) == 100)
    }

    @Test("minutes: used rounds up, the limit down")
    func minutes() {
        #expect(PaidFamilies.minutes(usedMS: 0, limitMS: 7_200_000) == (0, 120))
        #expect(PaidFamilies.minutes(usedMS: 1, limitMS: 7_200_000) == (1, 120))
        #expect(PaidFamilies.minutes(usedMS: 60_000, limitMS: 7_259_999) == (1, 120))
        #expect(PaidFamilies.minutes(usedMS: 60_001, limitMS: 59_999) == (2, 0))
    }

    @Test("what each new refusal says")
    func refusals() {
        #expect(PaidFamilies.failureMessage(APIError.forbidden(code: "subscription_required"))
            == "A new family on this server needs a subscription.")
        #expect(PaidFamilies.failureMessage(APIError.conflict(code: "subscription_invalid", message: nil))
            == "The App Store purchase couldn’t be confirmed. Try Restore Purchases.")
        #expect(PaidFamilies.failureMessage(APIError.conflict(code: "subscription_taken", message: nil))
            == "This App Store subscription belongs to another account on this server.")
        #expect(PaidFamilies.failureMessage(APIError.forbidden(code: "storage_allowance_used"))
            == "This family has used its storage allowance.")
        let assistant = PaidFamilies.failureMessage(APIError.forbidden(code: "ai_allowance_used"), now: Fixture.now)
        #expect(assistant == PaidFamilies.assistantAllowanceUsed(
            renewsAt: Fixture.date("2026-11-01T00:00:00Z"), now: Fixture.now))
        #expect(assistant?.hasPrefix("This family has used this month’s assistant allowance. It renews on ") == true)
        #expect(PaidFamilies.failureMessage(APIError.forbidden(code: "family_registration_disabled")) == nil)
        #expect(PaidFamilies.failureMessage(APIError.conflict(code: "validation", message: nil)) == nil)
        #expect(PaidFamilies.subscriptionRefusal(code: "validation") == nil)
    }

    @Test("an assistant answer past the allowance says when it renews, and is remembered as such")
    func assistantFailure() {
        let renews = Fixture.date("2026-12-01T00:00:00Z")
        let failure = AssistantFailure(reason: .allowanceUsed(renewsAt: renews))
        #expect(failure == .allowanceUsed(renewsAt: renews))
        #expect(failure.sentence.key == "This family has used this month’s assistant allowance. It renews on %@.")
        // A frame without renews_at renews when the server's month does.
        #expect(AssistantFailure(reason: .allowanceUsed(renewsAt: nil), now: Fixture.now)
            == .allowanceUsed(renewsAt: Fixture.date("2026-11-01T00:00:00Z")))
        #expect(AssistantFailure(reason: .refused) == .refused)
        #expect(AssistantFailure(reason: nil) == .stopped)
    }

    @Test("a transcript or a backdrop past the allowance is terminal, dated the 1st of next month UTC")
    func restAllowance() {
        let error = APIError.forbidden(code: "ai_allowance_used")
        let renews = Fixture.date("2026-11-01T00:00:00Z")
        let transcript = TranscriptOutcome(error: error, now: Fixture.now)
        #expect(transcript == .failed(.allowanceUsed(renewsAt: renews)))
        #expect(!TranscriptFailure.allowanceUsed(renewsAt: renews).offersRetry)
        #expect(TranscriptFailure.allowanceUsed(renewsAt: renews).message
            == PaidFamilies.assistantAllowanceUsed(renewsAt: renews))
        // Other 403s are as they were.
        #expect(TranscriptOutcome(error: APIError.forbidden(code: "transcript_not_allowed")) == .failed(.notAvailable))

        let backdrop = BackdropOutcome(error: error, now: Fixture.now)
        #expect(backdrop == .allowanceUsed(renewsAt: renews))
        #expect(backdrop.failureMessage == PaidFamilies.assistantAllowanceUsed(renewsAt: renews))
        #expect(!backdrop.asksForConsent(processor: "Azure"))
    }

    @Test("a pin or a sticker refused for storage says so")
    func storageRefusals() {
        let error = APIError.forbidden(code: "storage_allowance_used")
        #expect(ChatSyncCoordinator.pinFailure(error) == "This family has used its storage allowance.")
        #expect(ChatSyncCoordinator.packFailure(error) == "This family has used its storage allowance.")
    }

    @Test("the cap picker's bound: the family's ceiling, else the server's")
    @MainActor
    func ceiling() {
        #expect(MemberCap.ceiling(family: 25, server: 50) == 25)
        #expect(MemberCap.ceiling(family: nil, server: 50) == 50)
        #expect(MemberCap.ceiling(family: nil, server: nil) == nil)
    }

    @Test("a family_plan push opens in its family; a deleted family's opens on the list")
    func push() {
        let unpaid: [AnyHashable: Any] = ["kind": "family_plan", "family_id": 3, "stage": "unpaid"]
        let deleted: [AnyHashable: Any] = ["kind": "family_plan", "family_id": 3, "stage": "deleted"]
        #expect(PushRoute.parse(userInfo: unpaid) == .familyPlan)
        #expect(PushRoute.familyID(userInfo: unpaid) == 3)
        #expect(PushRoute.parse(userInfo: deleted) == .familyPlan)
        #expect(PushRoute.familyID(userInfo: deleted) == nil)
        #expect(PushRoute.familyID(userInfo: ["kind": "family_plan", "family_id": 3, "stage": "tomorrow"]) == 3)
    }

    /// The keys are what Swift extracts, and the catalogue must hold them in
    /// every language — a literal `%` included, which Swift doubles.
    @Test("the new sentences resolve in another language")
    func translationsResolve() throws {
        let path = try #require(Bundle.main.path(forResource: "ru", ofType: "lproj"))
        let russian = try #require(Bundle(path: path))
        let percent = 40
        #expect(String(localized: "\(percent)% used", bundle: russian) == "Использовано 40%")
        let day = "14 ноября"
        #expect(String(localized: "Subscription renews on \(day)", bundle: russian) == "Продлится 14 ноября")
        #expect(String(localized: "Renews on \(day)", bundle: russian) == "Обновится 14 ноября")
        let used = "3 ГБ", limit = "10 ГБ"
        #expect(String(localized: "\(used) of \(limit)", bundle: russian) == "3 ГБ из 10 ГБ")
        let (minutesUsed, minutesLimit): (Int64, Int64) = (11, 120)
        #expect(String(localized: "\(minutesUsed) of \(minutesLimit) min", bundle: russian) == "11 из 120 мин")
        let family = "Ивановы", owner = "Ольга"
        #expect(String(
            localized: "“\(family)” will be deleted on \(day) because its subscription has ended. Only \(owner), the owner, can renew it.",
            bundle: russian)
            == "«Ивановы» будет удалена 14 ноября: её подписка закончилась. Продлить её может только владелец — Ольга.")
    }
}

// MARK: - The session

@MainActor
@Suite("Paid families: the session", .serialized)
struct PaidFamiliesSessionTests {

    private static func me(
        enabled: Bool, subscription: SubscriptionDTO? = nil, families: [FamilyDTO] = [Fixture.family()],
        maxFamilyMembers: Int? = 50
    ) -> MeResponse {
        MeResponse(
            user: Fixture.user, family: families.first, role: families.first?.role, pendingJoinRequest: nil,
            maxFamilyMembers: maxFamilyMembers, families: families,
            subscriptionsEnabled: enabled,
            subscriptionProducts: ["me.nettrash.FamilyConnect.family.monthly"],
            appAccountToken: "6f1c2d3e-4a5b-4c6d-8e7f-a1b2c3d4e5f6",
            unpaidDays: 14,
            paidFamilyLimits: PaidFamilyLimitsDTO(members: 25),
            subscription: subscription)
    }

    @Test("a server that charges: held, and handed to the manager")
    func holdsWhenEnabled() {
        AppSettings.setCurrentFamilyID(nil, forUser: 77)
        defer { AppSettings.setCurrentFamilyID(nil, forUser: 77) }
        let session = AppSession(api: APIClient(serverURL: nil), defaultServerURL: { nil })
        var handed: [MeResponse] = []
        session.applySubscriptionConfig = { handed.append($0) }
        session.apply(me: Self.me(enabled: true))
        #expect(session.subscriptionsEnabled)
        #expect(session.subscriptionProducts == ["me.nettrash.FamilyConnect.family.monthly"])
        #expect(session.appAccountToken == UUID(uuidString: "6f1c2d3e-4a5b-4c6d-8e7f-a1b2c3d4e5f6"))
        #expect(session.paidFamilyLimits?.members == 25)
        #expect(session.needsSubscriptionToCreate)
        #expect(handed.count == 1 && handed[0].subscriptionsEnabled)

        session.applySubscription(Fixture.subscription(active: true))
        #expect(!session.needsSubscriptionToCreate, "a subscriber creates as before")
    }

    @Test("a server that does not charge: nothing held, nothing needed")
    func ignoresWhenDisabled() {
        AppSettings.setCurrentFamilyID(nil, forUser: 77)
        defer { AppSettings.setCurrentFamilyID(nil, forUser: 77) }
        let session = AppSession(api: APIClient(serverURL: nil), defaultServerURL: { nil })
        session.apply(me: Self.me(enabled: false, subscription: Fixture.subscription()))
        #expect(!session.subscriptionsEnabled)
        #expect(session.subscription == nil)
        #expect(session.subscriptionProducts.isEmpty)
        #expect(session.appAccountToken == nil)
        #expect(!session.needsSubscriptionToCreate)
        session.applySubscription(Fixture.subscription())
        #expect(session.subscription == nil)
    }

    @Test("family_plan sets and clears delete_at on the family it names, and no other")
    func familyPlan() {
        AppSettings.setCurrentFamilyID(nil, forUser: 77)
        defer { AppSettings.setCurrentFamilyID(nil, forUser: 77) }
        let session = AppSession(api: APIClient(serverURL: nil), defaultServerURL: { nil })
        let other = Fixture.family(5, "The Ivanovs", role: "member", plan: "free")
        session.apply(me: Self.me(enabled: true, families: [Fixture.family(), other]))
        session.applyFamilyPlan(familyID: 3, plan: "paid", deleteAt: Fixture.deleteAt)
        #expect(session.family?.deleteAt == Fixture.deleteAt)
        #expect(session.families.last?.deleteAt == nil)
        session.applyFamilyPlan(familyID: 3, plan: nil, deleteAt: nil)
        #expect(session.family?.deleteAt == nil, "an absent delete_at clears it")
        #expect(session.family?.plan == "paid", "an absent plan leaves the one held")
        session.applyFamilyPlan(familyID: 99, plan: "paid", deleteAt: Fixture.deleteAt)
        #expect(session.families.allSatisfy { $0.deleteAt == nil })
    }

    @Test("the cap picker's bound follows the current family's ceiling")
    func memberCeiling() {
        AppSettings.setCurrentFamilyID(nil, forUser: 77)
        defer { AppSettings.setCurrentFamilyID(nil, forUser: 77) }
        let session = AppSession(api: APIClient(serverURL: nil), defaultServerURL: { nil })
        session.apply(me: Self.me(enabled: true, families: [Fixture.family(ceiling: 25)], maxFamilyMembers: 50))
        #expect(session.memberCeiling == 25)
        session.apply(me: Self.me(enabled: false, families: [Fixture.family(ceiling: nil)], maxFamilyMembers: 50))
        #expect(session.memberCeiling == 50)
    }
}

// MARK: - The coordinator

@MainActor
@Suite("Paid families: the coordinator")
struct PaidFamiliesCoordinatorTests {

    @MainActor
    private struct Harness {
        let container: ModelContainer
        let coordinator: ChatSyncCoordinator
        let session: AppSession
        let host: String

        func messages() -> [MessageEntity] {
            (try? container.mainContext.fetch(FetchDescriptor<MessageEntity>())) ?? []
        }
    }

    private func makeHarness(host: String, handler: @escaping StubURLProtocol.Handler) throws -> Harness {
        StubURLProtocol.register(host: host, handler: handler)
        let container = try ModelContainer(
            for: ChatEntity.self, MessageEntity.self, MemberEntity.self, PendingMediaItemEntity.self,
            configurations: ModelConfiguration(isStoredInMemoryOnly: true))
        let api = APIClient(serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())
        let coordinator = ChatSyncCoordinator(modelContainer: container, api: api)
        coordinator.currentUserIDOverride = 77
        coordinator.ackTimeout = 0.2
        let session = AppSession(api: api, defaultServerURL: { nil })
        coordinator.bind(session: session)
        container.mainContext.insert(ChatEntity(chatID: 42, kind: "family", pinRank: 0, title: "The Smiths"))
        try container.mainContext.save()
        return Harness(container: container, coordinator: coordinator, session: session, host: host)
    }

    @Test("the family_plan frame reaches the session: the banner appears and goes live")
    func frame() throws {
        AppSettings.setCurrentFamilyID(nil, forUser: 77)
        defer { AppSettings.setCurrentFamilyID(nil, forUser: 77) }
        let harness = try makeHarness(host: "paid-frame.test") { _ in .empty(204) }
        defer { StubURLProtocol.unregister(host: harness.host) }
        harness.session.apply(me: MeResponse(
            user: Fixture.user, family: Fixture.family(), role: "owner", pendingJoinRequest: nil,
            families: [Fixture.family()], subscriptionsEnabled: true))

        harness.coordinator.handle(frame: .familyPlan(familyID: 3, plan: "paid", deleteAt: Fixture.deleteAt))
        #expect(harness.session.family?.deleteAt == Fixture.deleteAt)
        #expect(PaidFamilies.banner(family: harness.session.family, isOwner: true, ownerName: nil) != nil)
        harness.coordinator.handle(frame: .familyPlan(familyID: 3, plan: "paid", deleteAt: nil))
        #expect(harness.session.family?.deleteAt == nil)
        #expect(PaidFamilies.banner(family: harness.session.family, isOwner: true, ownerName: nil) == nil)
    }

    @Test("an allowance_used ai_error is remembered with its day")
    func allowanceRemembered() throws {
        let harness = try makeHarness(host: "paid-ai-error.test") { _ in .empty(204) }
        defer { StubURLProtocol.unregister(host: harness.host) }
        let renews = Fixture.date("2026-12-01T00:00:00Z")
        harness.coordinator.handle(frame: .aiError(chatID: 42, messageID: 1339, reason: .allowanceUsed(renewsAt: renews)))
        #expect(harness.coordinator.assistantFailure(messageID: 1339) == .allowanceUsed(renewsAt: renews))
        #expect(!harness.coordinator.isAwaitingAssistant(messageID: 1339))
    }

    @Test("a send refused for storage fails visibly, is not retried, and says why once")
    func storageRefusal() async throws {
        let harness = try makeHarness(host: "paid-storage.test") { _ in
            .json(403, #"{"error": {"code": "storage_allowance_used", "message": "no"}}"#)
        }
        defer { StubURLProtocol.unregister(host: harness.host) }
        let localID = try #require(harness.coordinator.enqueue(body: "photo caption", in: 42))
        await harness.coordinator.deliver(localID: localID)
        let row = try #require(harness.messages().first)
        #expect(row.state == .failed)
        #expect(row.nextAttemptAt == nil, "terminal: not retried")
        #expect(harness.coordinator.sendRefusalNotice == "This family has used its storage allowance.")
    }

    @Test("any other terminal refusal says nothing extra")
    func otherRefusalSilent() async throws {
        let harness = try makeHarness(host: "paid-other-refusal.test") { _ in
            .json(400, #"{"error": {"code": "validation", "message": "no"}}"#)
        }
        defer { StubURLProtocol.unregister(host: harness.host) }
        let localID = try #require(harness.coordinator.enqueue(body: "doomed", in: 42))
        await harness.coordinator.deliver(localID: localID)
        #expect(harness.messages().first?.state == .failed)
        #expect(harness.coordinator.sendRefusalNotice == nil)
    }
}
