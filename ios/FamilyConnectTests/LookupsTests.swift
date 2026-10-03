//
//  LookupsTests.swift
//  FamilyConnectTests
//
//  The assistant looking things up (docs/protocol.md, "Looking things up",
//  and "Consenting to the assistant", amended 2026-10-03), from this
//  client's side:
//
//  - the WIRE: `ai_lookups`, `assistant.lookups`,
//    `assistant_lookup_consent_at` and `ai.searches` read tolerantly — an
//    older server omits every one of them, and that must read as "nothing
//    is looked up here", never as a decoding failure;
//  - the CALLS: the owner's PATCH carries one key, the consent endpoint is
//    called at its path with its body, and agreeing to both writes the
//    assistant consent FIRST (the server refuses the second without it);
//  - the RULES the screens share: when lookups are offered, what the
//    consent screen asks, how the providers are named, what Settings shows;
//  - the FOOTER: an answer ending in the server's sources footer draws its
//    links tappable and gets no link-preview card.
//
//  No real provider is contacted anywhere here: every request goes to a
//  stub host on StubURLProtocol.
//

import Foundation
import SwiftUI
import Testing

@testable import FamilyConnect

// MARK: - The wire

@Suite("Lookups — the wire")
struct LookupsWireTests {

    private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        try APICoding.decoder().decode(T.self, from: Data(json.utf8))
    }

    @Test("ai_lookups reads as off from a server that predates it, and is read when sent")
    func familySwitch() throws {
        let old = try decode(FamilyDTO.self, #"{"id": 3, "name": "S", "join_policy": "open"}"#)
        #expect(!old.aiLookups)
        let on = try decode(
            FamilyDTO.self, #"{"id": 3, "name": "S", "join_policy": "open", "ai_lookups": true}"#)
        #expect(on.aiLookups)
        // Bound to nothing: it moves no other switch, and none moves it.
        #expect(!on.aiVision && !on.aiTranscripts && !on.aiFaces)
        let off = try decode(
            FamilyDTO.self, #"{"id": 3, "name": "S", "join_policy": "open", "ai_lookups": false}"#)
        #expect(!off.aiLookups)
    }

    @Test("assistant_lookup_consent_at: absent and null both mean not agreed")
    func memberConsent() throws {
        let user = #""user": {"id": 7, "username": "anna", "display_name": "Anna"}"#
        let older = try decode(MeResponse.self, "{\(user)}")
        #expect(older.assistantLookupConsentAt == nil)
        let null = try decode(
            MeResponse.self,
            "{\(user), \"assistant_consent_at\": \"2026-10-03T10:00:00Z\", \"assistant_lookup_consent_at\": null}")
        #expect(null.assistantConsentAt != nil)
        #expect(null.assistantLookupConsentAt == nil)
        let agreed = try decode(
            MeResponse.self,
            "{\(user), \"assistant_consent_at\": \"2026-10-03T10:00:00Z\", \"assistant_lookup_consent_at\": \"2026-10-03T10:05:00.123456Z\"}")
        #expect(agreed.assistantLookupConsentAt != nil)
    }

    private static let assistantHead =
        #""user_id": 1, "display_name": "Assistant", "mention": "@ai", "processor": "P""#

    @Test("assistant.lookups keeps the server's names in the server's order")
    func providerNames() throws {
        let dto = try decode(
            AssistantDTO.self,
            "{\(Self.assistantHead), \"lookups\": [\"Brave Search\", \"Open-Meteo\", \"Wikipedia\"]}")
        #expect(dto.lookups == ["Brave Search", "Open-Meteo", "Wikipedia"])
        let searx = try decode(AssistantDTO.self, "{\(Self.assistantHead), \"lookups\": [\"SearXNG\"]}")
        #expect(searx.lookups == ["SearXNG"])
    }

    /// Absent is the server's one spelling of "no source"; an empty list,
    /// blank names or a malformed value read the same way rather than
    /// failing the whole roster — the assistant object rides on
    /// `GET /families/mine`, and a client that could not decode it would
    /// lose the family screen over a footnote.
    @Test("assistant.lookups absent, empty, blank or malformed reads as no source")
    func noProvider() throws {
        for tail in ["", ", \"lookups\": []", ", \"lookups\": [\"  \", \"\"]", ", \"lookups\": 3",
                     ", \"lookups\": null", ", \"lookups\": {\"a\": 1}"] {
            let dto = try decode(AssistantDTO.self, "{\(Self.assistantHead)\(tail)}")
            #expect(dto.lookups == nil, "\(tail)")
            #expect(dto.processor == "P")
        }
    }

    @Test("ai.searches is zero from a server that predates it, and read when sent")
    func searchesStatistic() throws {
        let old = try decode(
            AiStatsDTO.self, #"{"questions": 4, "prompt_tokens": 10, "completion_tokens": 5}"#)
        #expect(old.searches == 0)
        let new = try decode(
            AiStatsDTO.self,
            #"{"questions": 4, "prompt_tokens": 10, "completion_tokens": 5, "images": 1, "searches": 7}"#)
        #expect(new.searches == 7)
        #expect(new.images == 1)
    }
}

// MARK: - The calls

@Suite("Lookups — the calls")
struct LookupsAPITests {

    private func api(_ host: String) -> APIClient {
        APIClient(serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())
    }

    /// One key and nothing else: turning lookups on must not carry another
    /// switch along with it.
    @Test("the owner's switch sends exactly {ai_lookups: …} to PATCH /families/mine")
    func ownerSwitch() async throws {
        let host = "lookups-switch.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { _ in
            .json(200, #"{"family": {"id": 3, "name": "S", "join_policy": "open", "ai_lookups": true}}"#)
        }
        let family = try await api(host).setAILookups(true)
        #expect(family.aiLookups)

        let patch = try #require(StubURLProtocol.requests(host: host).first)
        #expect(patch.method == "PATCH")
        #expect(patch.url.path() == "/api/v1/families/mine")
        let body = try #require(patch.bodyJSON())
        #expect(body["ai_lookups"] as? Bool == true)
        #expect(body.count == 1, "one key and nothing else, got \(body)")
    }

    @Test("the owner's switch turned off sends false, and a member is refused as forbidden")
    func ownerSwitchOffAndRefusal() async throws {
        let host = "lookups-switch-off.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { _ in
            .json(403, #"{"error": {"code": "not_family_owner", "message": "owner only"}}"#)
        }
        await #expect(throws: APIError.forbidden(code: "not_family_owner")) {
            _ = try await api(host).setAILookups(false)
        }
        let body = try #require(StubURLProtocol.requests(host: host).first?.bodyJSON())
        #expect(body["ai_lookups"] as? Bool == false)
    }

    @Test("granting the lookup consent posts {granted: true} and returns the server's stamp")
    func grant() async throws {
        let host = "lookups-consent.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { _ in
            .json(200, #"{"assistant_lookup_consent_at": "2026-10-03T10:05:00Z"}"#)
        }
        let stamp = try await api(host).setAssistantLookupConsent(true)
        #expect(stamp == ISO8601DateFormatter().date(from: "2026-10-03T10:05:00Z"))

        let post = try #require(StubURLProtocol.requests(host: host).first)
        #expect(post.method == "POST")
        #expect(post.url.path() == "/api/v1/me/assistant-lookup-consent")
        let body = try #require(post.bodyJSON())
        #expect(body["granted"] as? Bool == true)
        #expect(body.count == 1)
    }

    @Test("withdrawing posts {granted: false} and comes back nil")
    func withdraw() async throws {
        let host = "lookups-consent-off.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { _ in
            .json(200, #"{"assistant_lookup_consent_at": null}"#)
        }
        let stamp = try await api(host).setAssistantLookupConsent(false)
        #expect(stamp == nil)
        let body = try #require(StubURLProtocol.requests(host: host).first?.bodyJSON())
        #expect(body["granted"] as? Bool == false)
    }

    @Test("the server's two refusals arrive as themselves")
    func refusals() async throws {
        let host = "lookups-consent-refused.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { request in
            // A grant without the assistant consent, or a server with no
            // lookup source — told apart by the body only for this test.
            if request.bodyJSON()?["granted"] as? Bool == true {
                return .json(403, #"{"error": {"code": "assistant_consent_required", "message": "x"}}"#)
            }
            return .json(404, #"{"error": {"code": "not_found", "message": "x"}}"#)
        }
        await #expect(throws: APIError.forbidden(code: "assistant_consent_required")) {
            _ = try await api(host).setAssistantLookupConsent(true)
        }
        await #expect(throws: APIError.notFound(code: "not_found")) {
            _ = try await api(host).setAssistantLookupConsent(false)
        }
    }
}

// MARK: - Agreeing, through the session

@Suite("Lookups — agreeing", .serialized)
@MainActor
struct LookupsSessionTests {

    /// A stub server that records which consent endpoint was called, in
    /// order, and answers each from `lookupStatus`.
    private func session(
        host: String, lookupStatus: Int = 200
    ) -> AppSession {
        StubURLProtocol.register(host: host) { request in
            let path = request.url.path()
            let granted = request.bodyJSON()?["granted"] as? Bool == true
            if path.hasSuffix("/me/assistant-consent") {
                return .json(
                    200,
                    granted
                        ? #"{"assistant_consent_at": "2026-10-03T10:00:00Z"}"#
                        : #"{"assistant_consent_at": null}"#)
            }
            if path.hasSuffix("/me/assistant-lookup-consent") {
                if lookupStatus == 404 {
                    return .json(404, #"{"error": {"code": "not_found", "message": "x"}}"#)
                }
                if lookupStatus == 403 {
                    return .json(403, #"{"error": {"code": "assistant_consent_required", "message": "x"}}"#)
                }
                return .json(
                    200,
                    granted
                        ? #"{"assistant_lookup_consent_at": "2026-10-03T10:00:01Z"}"#
                        : #"{"assistant_lookup_consent_at": null}"#)
            }
            return .json(404, #"{"error": {"code": "not_found", "message": "x"}}"#)
        }
        return AppSession(
            api: APIClient(serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession()),
            defaultServerURL: { nil })
    }

    private func paths(_ host: String) -> [String] {
        StubURLProtocol.requests(host: host).map { $0.url.path() }
    }

    @Test("Agree With Lookups: the assistant consent first, then the lookup consent")
    func bothInOrder() async throws {
        let host = "agree-both.test"
        defer { StubURLProtocol.unregister(host: host) }
        let session = session(host: host)

        try await session.agreeToAssistant(.assistantAndLookups)

        #expect(paths(host) == ["/api/v1/me/assistant-consent", "/api/v1/me/assistant-lookup-consent"])
        #expect(session.assistantConsentAt != nil)
        #expect(session.assistantLookupConsentAt != nil)
    }

    @Test("Agree Without Lookups: the assistant consent only — the lookup endpoint is never called")
    func assistantOnly() async throws {
        let host = "agree-assistant.test"
        defer { StubURLProtocol.unregister(host: host) }
        let session = session(host: host)

        try await session.agreeToAssistant(.assistant)

        #expect(paths(host) == ["/api/v1/me/assistant-consent"])
        #expect(session.assistantConsentAt != nil)
        #expect(session.assistantLookupConsentAt == nil)
    }

    @Test("the lookup screen of somebody who agreed already writes the lookup consent alone")
    func lookupsOnly() async throws {
        let host = "agree-lookups.test"
        defer { StubURLProtocol.unregister(host: host) }
        let session = session(host: host)

        try await session.agreeToAssistant(.lookups)

        #expect(paths(host) == ["/api/v1/me/assistant-lookup-consent"])
        #expect(session.assistantLookupConsentAt != nil)
    }

    /// The operator removed every source since the screen was drawn: the
    /// assistant consent the member gave stands, their message can go, and
    /// the lookup surfaces disappear.
    @Test("a 404 on the second write of Agree With Lookups keeps the first and throws nothing")
    func sourceGoneMidAgreement() async throws {
        let host = "agree-gone.test"
        let saved = AppSettings.assistantLookups
        defer {
            StubURLProtocol.unregister(host: host)
            AppSettings.assistantLookups = saved
        }
        AppSettings.assistantLookups = ["Wikipedia"]
        let session = session(host: host, lookupStatus: 404)

        try await session.agreeToAssistant(.assistantAndLookups)

        #expect(session.assistantConsentAt != nil)
        #expect(session.assistantLookupConsentAt == nil)
        #expect(AppSettings.assistantLookups == nil)
    }

    @Test("asked for lookups alone, any refusal is the member's to see")
    func lookupsOnlyRefusalThrows() async throws {
        let host = "agree-lookups-refused.test"
        defer { StubURLProtocol.unregister(host: host) }
        let session = session(host: host, lookupStatus: 403)
        await #expect(throws: APIError.forbidden(code: "assistant_consent_required")) {
            try await session.agreeToAssistant(.lookups)
        }
        #expect(session.assistantLookupConsentAt == nil)
    }

    /// The 404 is forgiven only where it would block a message the member
    /// already agreed to send; asked for lookups alone, they must be told.
    @Test("asked for lookups alone, a server with no source answers as itself")
    func lookupsOnlyNotFoundThrows() async throws {
        let host = "agree-lookups-gone.test"
        let saved = AppSettings.assistantLookups
        defer {
            StubURLProtocol.unregister(host: host)
            AppSettings.assistantLookups = saved
        }
        AppSettings.assistantLookups = ["Wikipedia"]
        let session = session(host: host, lookupStatus: 404)
        await #expect(throws: APIError.notFound(code: "not_found")) {
            try await session.agreeToAssistant(.lookups)
        }
        #expect(AppSettings.assistantLookups == ["Wikipedia"])
    }

    @Test("Stop Lookups clears only the lookup consent")
    func stopLookups() async throws {
        let host = "stop-lookups.test"
        defer { StubURLProtocol.unregister(host: host) }
        let session = session(host: host)
        try await session.agreeToAssistant(.assistantAndLookups)

        try await session.setAssistantLookupConsent(false)

        #expect(session.assistantConsentAt != nil)
        #expect(session.assistantLookupConsentAt == nil)
    }

    /// The server withdraws the lookup consent with the assistant consent;
    /// the session says so at once rather than at the next /me.
    @Test("withdrawing the assistant consent withdraws the lookup consent with it")
    func stopAssistantStopsLookups() async throws {
        let host = "stop-assistant.test"
        defer { StubURLProtocol.unregister(host: host) }
        let session = session(host: host)
        try await session.agreeToAssistant(.assistantAndLookups)

        try await session.setAssistantConsent(false)

        #expect(session.assistantConsentAt == nil)
        #expect(session.assistantLookupConsentAt == nil)
    }

    @Test("/me carries the lookup consent into the session, and its absence clears it")
    func meCarriesIt() throws {
        let session = AppSession(api: APIClient(serverURL: nil), defaultServerURL: { nil })
        let user = UserDTO(id: 7, username: "anna", displayName: "Anna", createdAt: nil)
        let stamp = Date(timeIntervalSince1970: 1_790_000_000)
        session.apply(me: MeResponse(
            user: user, family: nil, role: nil, pendingJoinRequest: nil,
            assistantConsentAt: stamp, assistantLookupConsentAt: stamp))
        #expect(session.assistantLookupConsentAt == stamp)
        session.apply(me: MeResponse(
            user: user, family: nil, role: nil, pendingJoinRequest: nil,
            assistantConsentAt: stamp))
        #expect(session.assistantLookupConsentAt == nil)
    }
}

// MARK: - The rules the screens share

@Suite("Lookups — what is offered and said")
struct LookupsRulesTests {

    static let processor = "Microsoft — Azure OpenAI"
    static let three = ["Brave Search", "Open-Meteo", "Wikipedia"]

    @Test("an answer's writes, in order")
    func steps() {
        #expect(AssistantConsent.steps(for: .assistant) == [.assistant])
        #expect(AssistantConsent.steps(for: .assistantAndLookups) == [.assistant, .lookups])
        #expect(AssistantConsent.steps(for: .lookups) == [.lookups])
    }

    @Test("lookups are offered only with a named processor and a named provider")
    func offered() {
        #expect(AssistantConsent.offersLookups(processor: Self.processor, lookups: Self.three))
        #expect(AssistantConsent.offersLookups(processor: Self.processor, lookups: ["SearXNG"]))
        #expect(!AssistantConsent.offersLookups(processor: Self.processor, lookups: nil))
        #expect(!AssistantConsent.offersLookups(processor: Self.processor, lookups: []))
        #expect(!AssistantConsent.offersLookups(processor: Self.processor, lookups: [" "]))
        // No processor, no assistant — and so nothing to look up for.
        #expect(!AssistantConsent.offersLookups(processor: nil, lookups: Self.three))
        #expect(!AssistantConsent.offersLookups(processor: "  ", lookups: Self.three))
    }

    @Test("the consent screen asks one question, two, or only the second")
    func sheetMode() {
        #expect(AssistantConsent.sheetMode(assistantAgreed: false, lookups: nil) == .assistant)
        #expect(AssistantConsent.sheetMode(assistantAgreed: false, lookups: []) == .assistant)
        guard case .assistantWithLookups(let both) = AssistantConsent.sheetMode(
            assistantAgreed: false, lookups: Self.three)
        else {
            Issue.record("expected both questions")
            return
        }
        #expect(both.contains("Brave Search") && both.contains("Open-Meteo") && both.contains("Wikipedia"))
        guard case .lookupsOnly(let only) = AssistantConsent.sheetMode(
            assistantAgreed: true, lookups: ["SearXNG"])
        else {
            Issue.record("expected the lookup question alone")
            return
        }
        #expect(only == "SearXNG")
        // Somebody who agreed, on a server with no source: there is no
        // second question to ask.
        #expect(AssistantConsent.sheetMode(assistantAgreed: true, lookups: nil) == .assistant)
    }

    @Test("providers are joined for a sentence, every name kept, blanks dropped")
    func providerList() {
        #expect(AssistantConsent.providerList([]) == nil)
        #expect(AssistantConsent.providerList(["", "  "]) == nil)
        #expect(AssistantConsent.providerList(["Wikipedia"]) == "Wikipedia")
        #expect(AssistantConsent.providerList([" Wikipedia ", ""]) == "Wikipedia")
        for names in [["Open-Meteo", "Wikipedia"], Self.three, Self.three + ["MET Norway"]] {
            let joined = AssistantConsent.providerList(names)
            for name in names {
                #expect(joined?.contains(name) == true, "\(name) missing from \(joined ?? "nil")")
            }
        }
    }

    /// The one that would be a lie by omission: with `ai_history` on the
    /// query may be shaped by the chat's recent messages, and the line
    /// must say so; with it off it must not.
    @Test("the consent line names the providers, and says recent messages only with history on")
    func disclosureLine() {
        let providers = "Brave Search, Open-Meteo and Wikipedia"
        let withHistory = AssistantConsent.lookupDisclosure(providers: providers, familyHistory: true)
        let without = AssistantConsent.lookupDisclosure(providers: providers, familyHistory: false)
        #expect(withHistory.contains(providers))
        #expect(without.contains(providers))
        #expect(withHistory != without)
        #expect(withHistory.count > without.count)
    }

    @Test("Settings: absent until there is something to agree to and the assistant is agreed")
    func settingsSection() {
        let stamp = Date(timeIntervalSince1970: 1_790_000_000)
        #expect(AssistantConsent.lookupSettings(
            processor: Self.processor, lookups: nil, assistantAgreedAt: stamp, lookupAgreedAt: nil) == .absent)
        #expect(AssistantConsent.lookupSettings(
            processor: nil, lookups: Self.three, assistantAgreedAt: stamp, lookupAgreedAt: nil) == .absent)
        // Not agreed to the assistant yet: its own "Review and Agree…" asks both.
        #expect(AssistantConsent.lookupSettings(
            processor: Self.processor, lookups: Self.three, assistantAgreedAt: nil, lookupAgreedAt: nil) == .absent)
        #expect(AssistantConsent.lookupSettings(
            processor: Self.processor, lookups: ["Wikipedia"], assistantAgreedAt: stamp, lookupAgreedAt: nil)
            == .notAgreed(providers: "Wikipedia"))
        #expect(AssistantConsent.lookupSettings(
            processor: Self.processor, lookups: ["Wikipedia"], assistantAgreedAt: stamp, lookupAgreedAt: stamp)
            == .agreed(at: stamp, providers: "Wikipedia"))
    }

    /// The assistant consent's own rule is untouched: lookups never make a
    /// message wait for a second answer.
    @Test("the lookup consent is never required to send")
    func neverRequired() {
        #expect(!AssistantConsent.isRequired(
            chatKind: "ai", body: "weather tomorrow?", processor: Self.processor,
            agreedAt: Date(timeIntervalSince1970: 1_790_000_000)))
    }
}

// MARK: - Statistics

@Suite("Lookups — statistics")
@MainActor
struct LookupsStatisticsTests {

    private func member(searches: Int, questions: Int = 0) -> MemberStatsDTO {
        let json = """
            {"user_id": 7, "display_name": "Anna", "messages": 3,
             "attachments": {"count": 0, "bytes": 0, "photo": 0, "video": 0, "audio": 0, "file": 0},
             "ai": {"questions": \(questions), "prompt_tokens": 0, "completion_tokens": 0, "searches": \(searches)}}
            """
        return try! APICoding.decoder().decode(MemberStatsDTO.self, from: Data(json.utf8))
    }

    @Test("a member's searches are said with the right plural, and not at all at zero")
    func memberLine() {
        let none = StatisticsView.summary(for: member(searches: 0))
        #expect(!none.contains("search"))
        let one = StatisticsView.summary(for: member(searches: 1, questions: 1))
        #expect(one.contains("1 web search"), "\(one)")
        #expect(!one.contains("1 web searches"), "\(one)")
        let five = StatisticsView.summary(for: member(searches: 5, questions: 5))
        #expect(five.contains("5 web searches"), "\(five)")
    }
}

// MARK: - The footer

@Suite("Lookups — the sources footer")
struct LookupsFooterTests {

    static let english = """
        Tomorrow in Oslo: 12 °C and light rain.

        Sources: [Oslo weather](https://a.example.org/1) · [Oslo — Wikipedia](https://en.wikipedia.org/wiki/Oslo) · [News](https://b.example.net/x?y=1)
        [Weather data by Open-Meteo.com](https://open-meteo.com/) · Wikipedia, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/) · Powered by Brave
        """

    static let russian = """
        Завтра в Москве +8 °C.

        Источники: [Москва — Википедия](https://ru.wikipedia.org/wiki/Москва)
        [Данные о погоде: Open-Meteo.com](https://open-meteo.com/) · Википедия, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/)
        """

    /// A weather-only answer: nothing linkable to list, only the credit.
    static let weatherOnly = """
        Sunny tomorrow.

        [Weather data by Open-Meteo.com](https://open-meteo.com/)
        """

    /// SearXNG gets no credit, so its footer is the sources line alone.
    static let searxng = """
        Here is what I found.

        Sources: [First](https://a.example.org/1) · [Second](https://b.example.org/2)
        """

    @Test("the server's footer is recognised in every shape it comes in")
    func recognised() {
        for body in [Self.english, Self.russian, Self.weatherOnly, Self.searxng] {
            #expect(AssistantSources.hasFooter(body), "\(body)")
        }
        // The other translations of the label.
        for label in ["Quellen", "Fuentes", "出典", "Извори", "Izvori", "来源", "Sources"] {
            #expect(AssistantSources.hasFooter("Answer.\n\n\(label): [T](https://a.example/x)"), "\(label)")
        }
        // Credit lines in other languages.
        #expect(AssistantSources.hasFooter(
            "Ответ.\n\n[Подаци о времену: Open-Meteo.com](https://open-meteo.com/)"))
        #expect(AssistantSources.hasFooter(
            "答え。\n\nウィキペディア, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/)"))
        #expect(AssistantSources.hasFooter("Answer.\n\nPowered by Brave"))
    }

    @Test("ordinary messages are not mistaken for one")
    func notRecognised() {
        for body in [
            "see https://example.com",
            "Sources: [T](https://a.example/x)",  // nothing above it
            "Answer.\nSources: [T](https://a.example/x)",  // no blank line
            "Answer.\n\nSources: none of your business",  // no links
            "Answer.\n\nNotes: [T](https://a.example/x)",  // not a server label
            "Answer.\n\n[Weather](https://evil.example/)",  // not the provider's link
            "Answer.\n\nPowered by Brave and friends",
            "Answer.\n\nSources: [T](https://a.example/x) and more",
            "Answer.\n\nSources: [T](javascript:alert(1))",
            "",
        ] {
            #expect(!AssistantSources.hasFooter(body), "\(body)")
        }
    }

    @Test("an answer with sources gets no preview card; an ordinary link still does")
    func noPreviewCard() {
        for body in [Self.english, Self.russian, Self.weatherOnly, Self.searxng] {
            #expect(MessageLinks.firstWebLinkAsDrawn(in: body) == nil, "\(body)")
        }
        // A source the model cited inline is a source too: still no card.
        let inline = "See [this](https://a.example.org/1) for more.\n\n"
            + "Sources: [Oslo weather](https://a.example.org/1)"
        #expect(MessageLinks.firstWebLinkAsDrawn(in: inline) == nil)
        // The rule is the footer, not the link: everything else previews.
        #expect(
            MessageLinks.firstWebLinkAsDrawn(in: "see https://example.com/menu")?.absoluteString
                == "https://example.com/menu")
        #expect(
            MessageLinks.firstWebLinkAsDrawn(in: "Answer.\n\nNotes: [T](https://a.example/x)")?
                .absoluteString == "https://a.example/x")
    }

    /// Nothing to build for the footer to be tappable — it is markdown the
    /// bubble already renders — but that has to stay true: every source
    /// and every credit is a link run, and the labels are what is drawn.
    @Test("every source and credit is a tappable link, under its own label")
    func footerLinksAreTappable() {
        let attributed = MessageLinks.attributedBody(Self.english, isMine: false)
        let links = Set(attributed.runs.compactMap { $0.link?.absoluteString })
        for url in [
            "https://a.example.org/1", "https://en.wikipedia.org/wiki/Oslo",
            "https://b.example.net/x?y=1", "https://open-meteo.com/",
            "https://creativecommons.org/licenses/by-sa/4.0/",
        ] {
            #expect(links.contains(url), "\(url) not a link in \(links)")
        }
        let drawn = String(attributed.characters)
        #expect(drawn.contains("Oslo weather"))
        #expect(drawn.contains("Powered by Brave"))
        #expect(!drawn.contains("](https://"), "markdown left raw: \(drawn)")
    }

    @Test("a Wikipedia link in Cyrillic is tappable and opens that article")
    func cyrillicSource() {
        let attributed = MessageLinks.attributedBody(Self.russian, isMine: false)
        let links = attributed.runs.compactMap(\.link)
        let article = links.first { $0.host() == "ru.wikipedia.org" }
        #expect(article != nil, "links: \(links)")
        #expect(article?.path().removingPercentEncoding == "/wiki/Москва"
            || article?.path() == "/wiki/Москва")
        let drawn = String(attributed.characters)
        #expect(drawn.contains("Москва — Википедия"))
    }

    @Test("a percent-encoded source URL stays one link")
    func encodedSource() {
        let body = "A.\n\nSources: [Rust (language)](https://en.wikipedia.org/wiki/Rust_%28programming_language%29)"
        #expect(AssistantSources.hasFooter(body))
        let links = MessageLinks.attributedBody(body, isMine: false).runs.compactMap { $0.link?.absoluteString }
        #expect(links == ["https://en.wikipedia.org/wiki/Rust_%28programming_language%29"], "\(links)")
    }
}
