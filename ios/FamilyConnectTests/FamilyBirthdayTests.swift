//
//  FamilyBirthdayTests.swift
//  FamilyConnectTests
//
//  The family's two owner settings and everybody's birthdays: what
//  decodes, what goes on the wire, and what survives a roster upsert.
//
//  Three of these tests exist because of a difference that is invisible
//  in Swift and total on the wire:
//
//    - a property DEFAULT is not a decoding fallback (Swift throws on a
//      missing key), so `ai_history` needs a hand-written decoder or the
//      app stops decoding /me against a server that has not been upgraded;
//    - clearing the family's language needs an explicit JSON `null`, which
//      `encodeIfPresent` would silently turn back into "leave it alone" —
//      so the encoded body is inspected, not the Swift type;
//    - `member_joined` carries no birthday at all, so a frame must not be
//      allowed to wipe one the roster already reported.
//

import Foundation
import SwiftData
import Testing
@testable import FamilyConnect

@Suite("Family settings and birthdays")
struct FamilyBirthdayTests {

    private func makeClient(host: String, handler: @escaping StubURLProtocol.Handler) -> APIClient {
        StubURLProtocol.register(host: host, handler: handler)
        return APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())
    }

    // MARK: - Decoding

    /// The protocol's compatibility rule, pinned for FamilyDTO — which had
    /// no hand-written decoder at all until these two fields arrived.
    /// `/me` carries a Family, so getting this wrong does not degrade one
    /// screen: it stops the app bootstrapping against an older server.
    @Test("A family without language or ai_history decodes as unset and on")
    func familyCompatibilityDecoding() throws {
        let decoder = APICoding.decoder()

        let old = Data(#"""
        {"id": 3, "name": "The Smiths", "join_policy": "open",
         "created_at": "2026-08-01T10:00:00Z"}
        """#.utf8)
        let decoded = try decoder.decode(FamilyDTO.self, from: old)
        #expect(decoded.language == nil)
        // `true` by default, for families created after it existed and for
        // every family that existed before.
        #expect(decoded.aiHistory)

        let current = Data(#"""
        {"id": 3, "name": "The Smiths", "join_policy": "approval",
         "created_at": "2026-08-01T10:00:00Z", "invite_code": "ABCD2345",
         "language": "sr-Latn", "ai_history": false}
        """#.utf8)
        let now = try decoder.decode(FamilyDTO.self, from: current)
        #expect(now.language == "sr-Latn")
        #expect(!now.aiHistory)
        #expect(now.inviteCode == "ABCD2345")
    }

    /// Absence IS unset, on both objects that carry it — and the pair is
    /// one object, so there is no "month but no day" to decode.
    @Test("birthday decodes on User and Member, and is nil when absent")
    func birthdayDecoding() throws {
        let decoder = APICoding.decoder()

        let userWithout = Data(#"{"id": 7, "username": "anna", "display_name": "Anna"}"#.utf8)
        #expect(try decoder.decode(UserDTO.self, from: userWithout).birthday == nil)

        let userWith = Data(#"""
        {"id": 7, "username": "anna", "display_name": "Anna",
         "birthday": {"month": 3, "day": 14}}
        """#.utf8)
        #expect(try decoder.decode(UserDTO.self, from: userWith).birthday
            == BirthdayDTO(month: 3, day: 14))

        let memberWithout = Data(
            #"{"id": 8, "username": "ben", "display_name": "Ben", "role": "member"}"#.utf8)
        #expect(try decoder.decode(MemberDTO.self, from: memberWithout).birthday == nil)

        let memberWith = Data(#"""
        {"id": 8, "username": "ben", "display_name": "Ben", "role": "member",
         "birthday": {"month": 2, "day": 29}}
        """#.utf8)
        #expect(try decoder.decode(MemberDTO.self, from: memberWith).birthday
            == BirthdayDTO(month: 2, day: 29))
    }

    // MARK: - PATCH /families/mine

    /// The three states, proved against the encoded body rather than the
    /// Swift value: a tag SETS, a nil CLEARS with a real JSON null, and an
    /// untouched field is simply not there.
    @Test("language: a tag sets it, nil clears it with an explicit null")
    func languagePatchBody() async throws {
        let host = "family-language.test"
        defer { StubURLProtocol.unregister(host: host) }
        let client = makeClient(host: host) { _ in .json(200, Self.familyJSON) }

        _ = try await client.setFamilyLanguage("ru")
        _ = try await client.setFamilyLanguage(nil)

        let sent = StubURLProtocol.requests(host: host)
        #expect(sent.count == 2)
        #expect(sent[0].method == "PATCH")
        #expect(sent[0].url.path() == "/api/v1/families/mine")
        #expect(sent[0].bodyJSON()?["language"] as? String == "ru")

        // The key is PRESENT and its value is null. An omitted key means
        // "leave it alone" and would be a silent no-op.
        let cleared = try #require(sent[1].bodyJSON())
        #expect(cleared.keys.contains("language"))
        #expect(cleared["language"] is NSNull)
        // …and nothing else was touched by a language change.
        #expect(cleared["join_policy"] == nil)
        #expect(cleared["ai_history"] == nil)
    }

    /// `ai_history` is a boolean with a real default and no third state,
    /// so it never travels as a null — and never drags the language with it.
    @Test("ai_history patches alone, and false is sent as false")
    func aiHistoryPatchBody() async throws {
        let host = "family-ai-history.test"
        defer { StubURLProtocol.unregister(host: host) }
        let client = makeClient(host: host) { _ in .json(200, Self.familyJSON) }

        _ = try await client.setAIHistory(false)

        let sent = try #require(StubURLProtocol.requests(host: host).first)
        let body = try #require(sent.bodyJSON())
        #expect(body["ai_history"] as? Bool == false)
        #expect(body.keys.contains("language") == false)
        #expect(body.keys.contains("join_policy") == false)
    }

    /// The join policy shares the endpoint now; it must not have picked up
    /// a stray null for the language on the way.
    @Test("join policy still patches alone")
    func joinPolicyPatchBody() async throws {
        let host = "family-policy.test"
        defer { StubURLProtocol.unregister(host: host) }
        let client = makeClient(host: host) { _ in .json(200, Self.familyJSON) }

        _ = try await client.setJoinPolicy("approval")

        let body = try #require(StubURLProtocol.requests(host: host).first?.bodyJSON())
        #expect(body["join_policy"] as? String == "approval")
        #expect(body.count == 1)
    }

    @Test("the member cap patches alone, and null clears it")
    func memberCapPatchBody() async throws {
        let host = "family-cap.test"
        defer { StubURLProtocol.unregister(host: host) }
        let client = makeClient(host: host) { _ in .json(200, Self.familyJSON) }

        _ = try await client.setMemberCap(12)
        var body = try #require(StubURLProtocol.requests(host: host).first?.bodyJSON())
        #expect(body["max_members"] as? Int == 12)
        #expect(body.count == 1, "one field per patch: \(body)")

        // Clearing is a real JSON null, not an omitted key — the two mean
        // different things here, exactly as they do for the language.
        StubURLProtocol.unregister(host: host)
        let clearing = makeClient(host: host) { _ in .json(200, Self.familyJSON) }
        _ = try await clearing.setMemberCap(nil)
        body = try #require(StubURLProtocol.requests(host: host).first?.bodyJSON())
        #expect(body.count == 1)
        #expect(body["max_members"] is NSNull, "null CLEARS the cap: \(body)")
    }

    // MARK: - Birthday endpoints

    @Test("my birthday PUTs two integers and DELETEs by itself")
    func myBirthdayRequests() async throws {
        let host = "birthday-me.test"
        defer { StubURLProtocol.unregister(host: host) }
        let client = makeClient(host: host) { request in
            request.method == "PUT"
                ? .json(200, #"""
                  {"user": {"id": 7, "username": "anna", "display_name": "Anna",
                            "birthday": {"month": 3, "day": 14}}}
                  """#)
                : .empty(204)
        }

        let user = try await client.setMyBirthday(month: 3, day: 14)
        #expect(user.birthday == BirthdayDTO(month: 3, day: 14))
        try await client.clearMyBirthday()

        let sent = StubURLProtocol.requests(host: host)
        #expect(sent[0].method == "PUT")
        #expect(sent[0].url.path() == "/api/v1/me/birthday")
        #expect(sent[0].bodyJSON()?["month"] as? Int == 3)
        #expect(sent[0].bodyJSON()?["day"] as? Int == 14)
        // No year anywhere on the wire — there is nothing to compute an
        // age from, deliberately.
        #expect(sent[0].bodyJSON()?.count == 2)
        #expect(sent[1].method == "DELETE")
        #expect(sent[1].url.path() == "/api/v1/me/birthday")
    }

    /// The owner filling one in for somebody else — including, on purpose,
    /// their own row: the endpoint accepts their id, so no roster screen
    /// needs a special case for exactly one person.
    @Test("a member's birthday is written under their user id")
    func memberBirthdayRequests() async throws {
        let host = "birthday-member.test"
        defer { StubURLProtocol.unregister(host: host) }
        let client = makeClient(host: host) { request in
            request.method == "PUT"
                ? .json(200, #"""
                  {"member": {"id": 9, "username": "kid", "display_name": "Kid",
                              "role": "member", "birthday": {"month": 2, "day": 29}}}
                  """#)
                : .empty(204)
        }

        let member = try await client.setMemberBirthday(userID: 9, month: 2, day: 29)
        #expect(member.birthday == BirthdayDTO(month: 2, day: 29))
        try await client.clearMemberBirthday(userID: 9)

        let sent = StubURLProtocol.requests(host: host)
        #expect(sent[0].method == "PUT")
        #expect(sent[0].url.path() == "/api/v1/families/members/9/birthday")
        #expect(sent[1].method == "DELETE")
        #expect(sent[1].url.path() == "/api/v1/families/members/9/birthday")
    }

    /// The client bounds its own pickers, but the SERVER owns the rule —
    /// so its refusal has to arrive as an error the sheet can show, not as
    /// a shrug.
    @Test("a validation refusal reaches the caller")
    func validationSurfaces() async throws {
        let host = "birthday-invalid.test"
        defer { StubURLProtocol.unregister(host: host) }
        let client = makeClient(host: host) { _ in
            .json(400, #"{"error": {"code": "validation", "message": "day out of range"}}"#)
        }

        await #expect(throws: APIError.conflict(code: "validation", message: "day out of range")) {
            _ = try await client.setMyBirthday(month: 2, day: 30)
        }
    }

    // MARK: - The day-vs-month rule

    /// February is 29 here and not 28, because there is no year for the
    /// 29th to fail to exist in.
    @Test("29 February is a birthday; 30 February and 31 April are not")
    func validity() {
        #expect(BirthdayDTO(month: 2, day: 29).isValid)
        #expect(!BirthdayDTO(month: 2, day: 30).isValid)
        #expect(!BirthdayDTO(month: 4, day: 31).isValid)
        #expect(BirthdayDTO(month: 4, day: 30).isValid)
        #expect(BirthdayDTO(month: 12, day: 31).isValid)
        #expect(!BirthdayDTO(month: 0, day: 1).isValid)
        #expect(!BirthdayDTO(month: 13, day: 1).isValid)
        #expect(!BirthdayDTO(month: 1, day: 0).isValid)
        #expect(BirthdayDTO.daysIn(month: 2) == 29)
        #expect(BirthdayDTO.daysIn(month: 6) == 30)
        #expect(BirthdayDTO.daysIn(month: 7) == 31)
    }

    /// A roster row is not the place to find out the server did not
    /// validate a date, and an editor is not the place to crash over one.
    ///
    /// `daysIn` answers 0 for a month the calendar does not have, and the
    /// birthday sheet built its day picker as `1...daysIn(month:)` — which
    /// in Swift is not an empty range but a TRAP, taken the instant the
    /// owner opened the sheet on that member. Android coerces the same
    /// input in the same place; a wrong date shown beats a crash on both.
    @Test("an impossible month cannot produce an impossible picker")
    func impossibleMonthsAreClamped() {
        for month in [-1, 0, 13, 99] {
            let range = BirthdayDTO.dayRange(forMonth: month)
            #expect(range.lowerBound <= range.upperBound, "inverted range for month \(month)")
            #expect(range == 1...1, "got \(range) for month \(month)")
        }
        // The real months are untouched, February's 29 included.
        #expect(BirthdayDTO.dayRange(forMonth: 2) == 1...29)
        #expect(BirthdayDTO.dayRange(forMonth: 4) == 1...30)
        #expect(BirthdayDTO.dayRange(forMonth: 12) == 1...31)

        // And what the sheet seeds its pickers with, which is where the
        // impossible value arrives from.
        #expect(BirthdayDTO(month: 0, day: 5).clamped == BirthdayDTO(month: 1, day: 5))
        #expect(BirthdayDTO(month: 13, day: 31).clamped == BirthdayDTO(month: 12, day: 31))
        #expect(BirthdayDTO(month: 2, day: 30).clamped == BirthdayDTO(month: 2, day: 29))
        #expect(BirthdayDTO(month: 4, day: 0).clamped == BirthdayDTO(month: 4, day: 1))
        #expect(BirthdayDTO(month: 0, day: 99).clamped.isValid)
        // A birthday the server wrote is left exactly as it is.
        for birthday in [
            BirthdayDTO(month: 2, day: 29), BirthdayDTO(month: 12, day: 31),
            BirthdayDTO(month: 1, day: 1),
        ] {
            #expect(birthday.clamped == birthday)
        }
    }

    /// Day and month in the reader's own order and words — and never a
    /// year, because a year is the only part of a date that carries an age.
    @Test("formatting is the reader's locale, day and month only")
    func formatting() {
        let march14 = BirthdayDTO(month: 3, day: 14)
        let british = march14.formatted(locale: Locale(identifier: "en_GB"))
        let american = march14.formatted(locale: Locale(identifier: "en_US"))
        let russian = march14.formatted(locale: Locale(identifier: "ru_RU"))

        #expect(british.contains("March"))
        #expect(british.contains("14"))
        // The template is asked which way round the locale writes a date;
        // "MMMM d" hard-coded would put the month first for everyone.
        #expect(british.hasPrefix("14"))
        #expect(american.hasPrefix("March"))
        #expect(russian.contains("14"))
        #expect(!russian.contains("March"))

        // No year, in any of them — not "2024", not any four-digit run.
        for text in [british, american, russian] {
            #expect(text.range(of: #"\d{4}"#, options: .regularExpression) == nil)
        }

        // 29 February formats rather than rolling into March.
        let leap = BirthdayDTO(month: 2, day: 29).formatted(locale: Locale(identifier: "en_GB"))
        #expect(leap.contains("29"))
        #expect(leap.contains("February"))
    }

    // MARK: - The nine

    /// A fixed list, spelled the way the protocol spells it — a client
    /// compares what comes back against this without normalising first.
    @Test("the nine languages are exactly the protocol's nine")
    func languageCatalogue() {
        #expect(FamilyLanguage.all.map(\.tag)
            == ["en", "de", "es", "fr", "ja", "ru", "sr", "sr-Latn", "zh-Hans"])
        #expect(FamilyLanguage.all.allSatisfy { !$0.name.isEmpty })

        // Two of the nine name a SCRIPT, and they are different choices:
        // a family that reads Cyrillic cannot read the other one.
        #expect(FamilyLanguage.named("sr")?.name != FamilyLanguage.named("sr-Latn")?.name)

        // Casing is not significant coming in, so a tag stored before the
        // server canonicalised it still finds its row.
        #expect(FamilyLanguage.named("sr-latn")?.tag == "sr-Latn")
        #expect(FamilyLanguage.named("ZH-HANS")?.tag == "zh-Hans")
        #expect(FamilyLanguage.named("kl") == nil)
        // Unset is not a language, and is certainly not English.
        #expect(FamilyLanguage.named(nil) == nil)
    }

    // MARK: - The roster row

    @MainActor
    private func makeCoordinator(host: String, handler: @escaping StubURLProtocol.Handler) throws
        -> (ChatSyncCoordinator, ModelContainer) {
        StubURLProtocol.register(host: host, handler: handler)
        let container = try ModelContainer(
            for: ChatEntity.self, MessageEntity.self, MemberEntity.self,
            configurations: ModelConfiguration(isStoredInMemoryOnly: true))
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())
        let coordinator = ChatSyncCoordinator(modelContainer: container, api: api)
        coordinator.currentUserIDOverride = 7
        return (coordinator, container)
    }

    @MainActor
    private func member(_ container: ModelContainer, _ userID: Int64) -> MemberEntity? {
        let descriptor = FetchDescriptor<MemberEntity>(predicate: #Predicate { $0.userID == userID })
        return (try? container.mainContext.fetch(descriptor))?.first
    }

    /// The Mac's roster draws SwiftData while the phone's draws the DTOs
    /// straight off the API, so a birthday has to land on both — and
    /// `dto` is the bridge the shared editor sheet is handed.
    @Test("a roster read stores the birthday, and the entity's dto carries it")
    @MainActor
    func rosterUpsertStoresBirthday() async throws {
        let host = "roster-birthday.test"
        defer { StubURLProtocol.unregister(host: host) }
        let (coordinator, container) = try makeCoordinator(host: host) { request in
            switch request.url.path() {
            case "/api/v1/me":
                return .json(200, Self.meJSON)
            case "/api/v1/families/mine":
                return .json(200, #"""
                {"family": {"id": 3, "name": "The Smiths", "join_policy": "open",
                            "created_at": "2026-08-01T10:00:00Z"},
                 "members": [{"id": 7, "username": "anna", "display_name": "Anna",
                              "role": "owner", "birthday": {"month": 3, "day": 14}},
                             {"id": 9, "username": "kid", "display_name": "Kid",
                              "role": "member"}]}
                """#)
            case "/api/v1/chats":
                return .json(200, #"{"chats": []}"#)
            default:
                return .empty(204)
            }
        }

        await coordinator.resync()

        let anna = try #require(member(container, 7))
        #expect(anna.birthday == BirthdayDTO(month: 3, day: 14))
        #expect(anna.dto.birthday == BirthdayDTO(month: 3, day: 14))
        // Absent on the roster means unset, and stays unset.
        #expect(member(container, 9)?.birthday == nil)
        #expect(member(container, 9)?.dto.birthday == nil)
    }

    /// `member_joined` carries no birthday (protocol.md, "Server →
    /// client"), so it must leave the stored one alone. Anything else and
    /// a member who leaves and rejoins loses theirs until the next resync.
    @Test("member_joined does not wipe a birthday it was never told about")
    @MainActor
    func memberJoinedLeavesBirthdayAlone() throws {
        let host = "roster-joined.test"
        defer { StubURLProtocol.unregister(host: host) }
        let (coordinator, container) = try makeCoordinator(host: host) { _ in .empty(204) }

        container.mainContext.insert(MemberEntity(
            userID: 9, username: "kid", displayName: "Kid", role: "member",
            isCurrentUser: false, hasLeft: true,
            birthday: BirthdayDTO(month: 2, day: 29)))
        try container.mainContext.save()

        let frame = try APICoding.decoder().decode(ServerFrame.self, from: Data(#"""
        {"type": "member_joined", "family_id": 3,
         "user": {"id": 9, "username": "kid", "display_name": "Kid", "avatar_version": 0}}
        """#.utf8))
        coordinator.handle(frame: frame)

        let row = try #require(member(container, 9))
        #expect(!row.hasLeft)
        #expect(row.birthday == BirthdayDTO(month: 2, day: 29))
    }

    /// A birthday raises no frame and no push, so the device that edited
    /// one is the only thing that can show it before the next resync.
    @Test("applyMemberBirthday writes the row, and clearing empties it")
    @MainActor
    func applyMemberBirthday() throws {
        let host = "roster-apply.test"
        defer { StubURLProtocol.unregister(host: host) }
        let (coordinator, container) = try makeCoordinator(host: host) { _ in .empty(204) }

        container.mainContext.insert(MemberEntity(
            userID: 9, username: "kid", displayName: "Kid", role: "member",
            isCurrentUser: false))
        try container.mainContext.save()

        coordinator.applyMemberBirthday(userID: 9, birthday: BirthdayDTO(month: 12, day: 25))
        #expect(member(container, 9)?.birthday == BirthdayDTO(month: 12, day: 25))

        coordinator.applyMemberBirthday(userID: 9, birthday: nil)
        #expect(member(container, 9)?.birthday == nil)
        // Half a birthday is not a state anything should have to draw.
        #expect(member(container, 9)?.birthdayMonth == nil)
        #expect(member(container, 9)?.birthdayDay == nil)

        // A member this device has never heard of is a no-op, not a crash.
        coordinator.applyMemberBirthday(userID: 404, birthday: BirthdayDTO(month: 1, day: 1))
        #expect(member(container, 404) == nil)
    }

    /// The roster screens patch the row they hold after an edit rather
    /// than re-reading the whole family.
    @Test("withBirthday changes only the birthday")
    func withBirthday() {
        let member = MemberDTO(
            id: 9, username: "kid", displayName: "Kid", role: "member", avatarVersion: 4)
        let updated = member.withBirthday(BirthdayDTO(month: 6, day: 1))

        #expect(updated.birthday == BirthdayDTO(month: 6, day: 1))
        #expect(updated.id == 9)
        #expect(updated.username == "kid")
        #expect(updated.displayName == "Kid")
        #expect(updated.role == "member")
        #expect(updated.avatarVersion == 4)
        #expect(updated.withBirthday(nil).birthday == nil)
    }

    // MARK: - Fixtures

    static let familyJSON = """
    {"family": {"id": 3, "name": "The Smiths", "join_policy": "open",
                "created_at": "2026-08-01T10:00:00Z", "invite_code": "ABCD2345",
                "language": "ru", "ai_history": true}}
    """

    static let meJSON = """
    {"user": {"id": 7, "username": "anna", "display_name": "Anna",
              "created_at": "2026-08-19T17:00:00Z"},
     "family": {"id": 3, "name": "The Smiths", "join_policy": "open",
                "created_at": "2026-08-01T10:00:00Z"},
     "role": "owner"}
    """
}
