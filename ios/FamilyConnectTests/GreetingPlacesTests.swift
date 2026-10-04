//
//  GreetingPlacesTests.swift
//  FamilyConnectTests
//
//  Today's weather in the daily greeting, for places the owner chose
//  (docs/protocol.md, "Today's weather, for places the owner chose"), from
//  this client's side:
//
//  - the WIRE: `greeting_places` on the Family object and
//    `assistant.greeting_weather` read tolerantly — an older server omits
//    both, and that must read as "no list, no editor", never as a decoding
//    failure;
//  - the CALL: the owner's PATCH carries the one key, `[]` clears, and the
//    list in the answer is the one to show;
//  - the RULES: the server's per-name rules mirrored, the editor's input
//    limit, and when the editor is offered at all;
//  - the CREDIT: a greeting ending in the weather credit, in each of the
//    server's nine languages, draws it as a tappable link and gets no
//    link-preview card.
//
//  No weather provider is contacted anywhere here: every request goes to a
//  stub host on StubURLProtocol.
//

import Foundation
import SwiftUI
import Testing

@testable import FamilyConnect

// MARK: - The wire

@Suite("Greeting places — the wire")
struct GreetingPlacesWireTests {

    private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        try APICoding.decoder().decode(T.self, from: Data(json.utf8))
    }

    private static let familyHead = #""id": 3, "name": "S", "join_policy": "open""#

    @Test("greeting_places is read in the server's order when sent")
    func placesDecode() throws {
        let family = try decode(
            FamilyDTO.self,
            "{\(Self.familyHead), \"ai_greeting\": true, \"greeting_places\": [\"Moscow\", \"Belgrade\"]}")
        #expect(family.greetingPlaces == ["Moscow", "Belgrade"])
        #expect(family.aiGreeting)
        let empty = try decode(FamilyDTO.self, "{\(Self.familyHead), \"greeting_places\": []}")
        #expect(empty.greetingPlaces == [])
    }

    /// A server that predates the field — and a value that is not a list of
    /// strings — reads as no places rather than failing the Family object,
    /// which rides on /me.
    @Test("greeting_places absent, null or malformed reads as no places")
    func placesTolerant() throws {
        for tail in ["", ", \"greeting_places\": null", ", \"greeting_places\": 3",
                     ", \"greeting_places\": [1, 2]", ", \"greeting_places\": {\"a\": 1}"] {
            let family = try decode(FamilyDTO.self, "{\(Self.familyHead), \"ai_lookups\": true\(tail)}")
            #expect(family.greetingPlaces == [], "\(tail)")
            // Its neighbours are untouched.
            #expect(family.aiLookups, "\(tail)")
            #expect(family.aiHistory, "\(tail)")
        }
    }

    @Test("the Family object encodes the list under its wire name")
    func placesEncode() throws {
        let family = FamilyDTO(
            id: 3, name: "S", joinPolicy: "open", createdAt: nil, inviteCode: nil,
            aiVision: false, aiHistoryPhotos: false, aiGreeting: true, aiFaces: false,
            aiTranscripts: false, aiLookups: false, greetingPlaces: ["Oslo"], maxMembers: nil)
        let data = try APICoding.encoder().encode(family)
        let json = try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        #expect(json["greeting_places"] as? [String] == ["Oslo"])
        let back = try APICoding.decoder().decode(FamilyDTO.self, from: data)
        #expect(back == family)
    }

    private static let assistantHead =
        #""user_id": 1, "display_name": "Assistant", "mention": "@ai", "processor": "P""#

    @Test("assistant.greeting_weather is read when sent")
    func weatherFlag() throws {
        let on = try decode(AssistantDTO.self, "{\(Self.assistantHead), \"greeting_weather\": true}")
        #expect(on.greetingWeather)
        let off = try decode(AssistantDTO.self, "{\(Self.assistantHead), \"greeting_weather\": false}")
        #expect(!off.greetingWeather)
        // Independent of lookups, either way round.
        let alone = try decode(
            AssistantDTO.self, "{\(Self.assistantHead), \"greeting_weather\": true}")
        #expect(alone.lookups == nil)
        let lookupsOnly = try decode(
            AssistantDTO.self, "{\(Self.assistantHead), \"lookups\": [\"Open-Meteo\"]}")
        #expect(!lookupsOnly.greetingWeather)
        #expect(lookupsOnly.lookups == ["Open-Meteo"])
    }

    @Test("assistant.greeting_weather absent, null or malformed reads as no greeting weather")
    func weatherFlagTolerant() throws {
        for tail in ["", ", \"greeting_weather\": null", ", \"greeting_weather\": \"yes\"",
                     ", \"greeting_weather\": 1"] {
            let dto = try decode(AssistantDTO.self, "{\(Self.assistantHead)\(tail)}")
            #expect(!dto.greetingWeather, "\(tail)")
            #expect(dto.processor == "P", "\(tail)")
        }
    }

    @Test("GET /families/mine carries the flag through the assistant object")
    func familyMine() throws {
        let mine = try decode(
            FamilyMineResponse.self,
            """
            {"family": {\(Self.familyHead), "greeting_places": ["Belgrade"]},
             "members": [],
             "assistant": {\(Self.assistantHead), "greeting_weather": true}}
            """)
        #expect(mine.family.greetingPlaces == ["Belgrade"])
        #expect(mine.assistant?.greetingWeather == true)
    }
}

// MARK: - The call

@Suite("Greeting places — the call")
struct GreetingPlacesAPITests {

    private func api(_ host: String) -> APIClient {
        APIClient(serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())
    }

    @Test("saving sends exactly {greeting_places: [...]} to PATCH /families/mine")
    func save() async throws {
        let host = "greeting-places.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { _ in
            .json(200, #"{"family": {"id": 3, "name": "S", "join_policy": "open", "greeting_places": ["Moscow", "Belgrade"]}}"#)
        }
        let family = try await api(host).setGreetingPlaces(["Moscow", "Belgrade"])
        #expect(family.greetingPlaces == ["Moscow", "Belgrade"])

        let patch = try #require(StubURLProtocol.requests(host: host).first)
        #expect(patch.method == "PATCH")
        #expect(patch.url.path() == "/api/v1/families/mine")
        let body = try #require(patch.bodyJSON())
        #expect(body["greeting_places"] as? [String] == ["Moscow", "Belgrade"])
        #expect(body.count == 1, "one key and nothing else, got \(body)")
    }

    /// `[]` is the clear; a null is refused by the server, so it must never
    /// be what an empty list turns into.
    @Test("clearing sends an empty array, not null and not a missing key")
    func clear() async throws {
        let host = "greeting-places-clear.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { _ in
            .json(200, #"{"family": {"id": 3, "name": "S", "join_policy": "open", "greeting_places": []}}"#)
        }
        let family = try await api(host).setGreetingPlaces([])
        #expect(family.greetingPlaces == [])
        let raw = try #require(StubURLProtocol.requests(host: host).first?.body)
        let text = String(decoding: raw, as: UTF8.self)
        #expect(text.replacingOccurrences(of: " ", with: "") == #"{"greeting_places":[]}"#, "\(text)")
    }

    /// The answer is the truth: shorter (a repeat dropped) or spelt with
    /// less whitespace than what was sent.
    @Test("the list in the answer, not the one sent, is what comes back")
    func answerWins() async throws {
        let host = "greeting-places-kept.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { _ in
            .json(200, #"{"family": {"id": 3, "name": "S", "join_policy": "open", "greeting_places": ["New York"]}}"#)
        }
        let family = try await api(host).setGreetingPlaces(["New York", "new york"])
        #expect(family.greetingPlaces == ["New York"])
    }

    /// A server that predates the field ignores the key and answers without
    /// it — which reads as no places, so the screen shows what is true.
    @Test("an older server's answer reads as no places")
    func olderServer() async throws {
        let host = "greeting-places-old.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { _ in
            .json(200, #"{"family": {"id": 3, "name": "S", "join_policy": "open"}}"#)
        }
        let family = try await api(host).setGreetingPlaces(["Oslo"])
        #expect(family.greetingPlaces == [])
    }

    @Test("a member is refused as forbidden, and a bad list as validation")
    func refusals() async throws {
        let host = "greeting-places-refused.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { request in
            if (request.bodyJSON()?["greeting_places"] as? [String])?.count == 1 {
                return .json(403, #"{"error": {"code": "not_family_owner", "message": "owner only"}}"#)
            }
            return .json(400, #"{"error": {"code": "validation", "message": "greeting place 2 is empty"}}"#)
        }
        await #expect(throws: APIError.forbidden(code: "not_family_owner")) {
            _ = try await api(host).setGreetingPlaces(["Oslo"])
        }
        await #expect(throws: APIError.conflict(code: "validation", message: "greeting place 2 is empty")) {
            _ = try await api(host).setGreetingPlaces(["Oslo", "Bergen"])
        }
    }

    /// The other owner switches send their one key, and the places are not
    /// carried along with them (a stale list sent with a switch would
    /// silently overwrite the stored one).
    @Test("the greeting switch does not carry the places")
    func switchesLeavePlacesAlone() async throws {
        let host = "greeting-switch-no-places.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { _ in
            .json(200, #"{"family": {"id": 3, "name": "S", "join_policy": "open", "ai_greeting": true, "greeting_places": ["Oslo"]}}"#)
        }
        let family = try await api(host).setAIGreeting(true)
        #expect(family.greetingPlaces == ["Oslo"])
        let body = try #require(StubURLProtocol.requests(host: host).first?.bodyJSON())
        #expect(body["greeting_places"] == nil)
        #expect(body.count == 1, "\(body)")
    }
}

// MARK: - The rules

@Suite("Greeting places — the rules")
struct GreetingPlacesRulesTests {

    @Test("offered only to the owner, and only when the server fetches greeting weather")
    func offered() {
        #expect(GreetingPlaces.isOffered(isOwner: true, serverGreetingWeather: true))
        #expect(!GreetingPlaces.isOffered(isOwner: false, serverGreetingWeather: true))
        #expect(!GreetingPlaces.isOffered(isOwner: true, serverGreetingWeather: false))
        #expect(!GreetingPlaces.isOffered(isOwner: false, serverGreetingWeather: false))
    }

    @Test("Add place while fewer than three; the limit sentence at three")
    func canAdd() {
        #expect(GreetingPlaces.maxPlaces == 3)
        #expect(GreetingPlaces.canAdd(count: 0))
        #expect(GreetingPlaces.canAdd(count: 1))
        #expect(GreetingPlaces.canAdd(count: 2))
        #expect(!GreetingPlaces.canAdd(count: 3))
        #expect(!GreetingPlaces.canAdd(count: 4))
    }

    /// Unicode White_Space, as Rust's `split_whitespace` reads it — which
    /// includes the no-break and ideographic spaces and excludes the
    /// zero-width space (a format character, not whitespace, on both sides).
    @Test("trimmed, with every inner run of whitespace folded to one space")
    func normalize() {
        #expect(GreetingPlaces.normalize("  Moscow ") == "Moscow")
        #expect(GreetingPlaces.normalize("New \t  York") == "New York")
        #expect(GreetingPlaces.normalize("Rio\nde\r\nJaneiro") == "Rio de Janeiro")
        #expect(GreetingPlaces.normalize("Novi\u{00A0}\u{00A0}Sad") == "Novi Sad")
        #expect(GreetingPlaces.normalize("\u{3000}東京\u{3000}") == "東京")
        #expect(GreetingPlaces.normalize("San\u{2003}José\u{0085}") == "San José")
        #expect(GreetingPlaces.normalize("a\u{200B}b") == "a\u{200B}b")
        #expect(GreetingPlaces.normalize(" \t\n ") == "")
    }

    @Test("kept as the server keeps them: folded, repeats dropped, first spelling kept")
    func prepare() {
        #expect(GreetingPlaces.prepare(["Moscow", " Belgrade "]) == .success(["Moscow", "Belgrade"]))
        #expect(GreetingPlaces.prepare(["Moscow", "moscow", "MOSCOW "]) == .success(["Moscow"]))
        #expect(GreetingPlaces.prepare(["new  york", "New York"]) == .success(["new york"]))
        #expect(GreetingPlaces.prepare(["Москва", "МОСКВА", "Белград"]) == .success(["Москва", "Белград"]))
        // Different places to the lower-casing, as they are to the server.
        #expect(GreetingPlaces.prepare(["Straße", "STRASSE"]) == .success(["Straße", "STRASSE"]))
        #expect(GreetingPlaces.prepare([]) == .success([]))
    }

    /// Swift's `==` and `Set` treat canonically equivalent strings as one —
    /// "Café" with a precomposed é and with e + U+0301 — while the server
    /// compares the lower-cased bytes. The editor must agree with the server
    /// (protocol.md: code point for code point), or it sends one name where
    /// the server would keep two, and never re-sends a stored name retyped
    /// in the other encoding.
    @Test("repeats are compared scalar by scalar, never by canonical equivalence")
    func canonicalEquivalentsAreTwoNames() throws {
        let composed = "Caf\u{E9}"
        let decomposed = "Cafe\u{301}"
        #expect(composed == decomposed, "Swift's own == calls them one")
        let kept = try GreetingPlaces.prepare([composed, decomposed]).get()
        #expect(kept.map { Array($0.unicodeScalars) }
                == [Array(composed.unicodeScalars), Array(decomposed.unicodeScalars)])
        // Lower-casing first changes nothing about that.
        #expect(try GreetingPlaces.prepare(["CAF\u{C9}", decomposed]).get().count == 2)
        // ...and a real repeat, same scalars once lower-cased, is still one.
        #expect(try GreetingPlaces.prepare(["CAF\u{C9}", composed]).get().count == 1)
        // A stored name retyped in the other encoding is a change to send.
        #expect(GreetingPlaces.needsSave(drafts: [decomposed], stored: [composed]))
        #expect(!GreetingPlaces.needsSave(drafts: [composed], stored: [composed]))
        // The editor's own comparison of lists is the same one.
        #expect(!GreetingPlaces.same([composed], [decomposed]))
        #expect(GreetingPlaces.same([composed], [composed]))
        #expect(!GreetingPlaces.same([composed], [composed, "Oslo"]))
        #expect(!GreetingPlaces.same([composed], nil as [String]?))
        #expect(GreetingPlaces.same([composed], Optional([composed])))
    }

    /// The one difference from the server, on purpose: a field the owner
    /// added and left blank is not a refusal.
    @Test("empty fields are dropped, not refused")
    func emptyDropped() {
        #expect(GreetingPlaces.prepare(["", "  ", "Oslo"]) == .success(["Oslo"]))
        #expect(GreetingPlaces.prepare(["", "\t"]) == .success([]))
    }

    @Test("at most three, counted after repeats are dropped")
    func limit() {
        #expect(GreetingPlaces.prepare(["A", "a", "B", "C"]) == .success(["A", "B", "C"]))
        #expect(GreetingPlaces.prepare(["A", "", "B", "C"]) == .success(["A", "B", "C"]))
        #expect(GreetingPlaces.prepare(["A", "B", "C", "D"]) == .failure(.tooMany))
    }

    /// Characters as Rust counts them — scalars — so a Cyrillic name of 80
    /// letters (160 bytes) passes and a flag emoji counts two.
    @Test("at most 80 characters, counted as Unicode scalars after the fold")
    func length() {
        let eighty = String(repeating: "я", count: 80)
        #expect(GreetingPlaces.prepare([eighty]) == .success([eighty]))
        #expect(GreetingPlaces.prepare(["Oslo", eighty + "я"]) == .failure(.tooLong(position: 2)))
        // Folding happens first: a run of spaces does not count against it.
        let padded = String(repeating: "a", count: 40) + "     " + String(repeating: "b", count: 39)
        #expect(GreetingPlaces.prepare([padded]) == .success([String(repeating: "a", count: 40) + " " + String(repeating: "b", count: 39)]))
        // 40 flags are 40 graphemes to Swift and 80 scalars to the server.
        let flags = String(repeating: "🇷🇸", count: 40)
        #expect(flags.count == 40)
        #expect(GreetingPlaces.length(flags) == 80)
        #expect(GreetingPlaces.prepare([flags]) == .success([flags]))
        #expect(GreetingPlaces.prepare([flags + "x"]) == .failure(.tooLong(position: 1)))
        // A decomposed é is two scalars.
        #expect(GreetingPlaces.length("e\u{0301}") == 2)
    }

    /// A tab or line break is whitespace and is folded away BEFORE the
    /// control check, as on the server; any other control character is a
    /// refusal, by position.
    @Test("control characters are refused after the fold, by position")
    func control() {
        #expect(GreetingPlaces.prepare(["Oslo\t"]) == .success(["Oslo"]))
        #expect(GreetingPlaces.prepare(["Oslo", "Ber\u{0007}gen"]) == .failure(.controlCharacter(position: 2)))
        #expect(GreetingPlaces.prepare(["\u{0000}"]) == .failure(.controlCharacter(position: 1)))
        #expect(GreetingPlaces.prepare(["Oslo\u{007F}"]) == .failure(.controlCharacter(position: 1)))
        // A format character is not a control character.
        #expect(GreetingPlaces.prepare(["a\u{200B}b"]) == .success(["a\u{200B}b"]))
    }

    @Test("the field drops control characters but keeps whitespace while typing")
    func inputKeepsTyping() {
        #expect(GreetingPlaces.limitInput("New ") == "New ")
        #expect(GreetingPlaces.limitInput("New  York") == "New  York")
        #expect(GreetingPlaces.limitInput("Ber\u{0007}gen") == "Bergen")
        #expect(GreetingPlaces.limitInput("Oslo\n") == "Oslo\n")
    }

    @Test("the field stops at the 80th character the server will count")
    func inputLimit() {
        let long = String(repeating: "x", count: 100)
        #expect(GreetingPlaces.limitInput(long) == String(repeating: "x", count: 80))
        // Spaces the fold will remove are not counted against it.
        let spaced = "a" + String(repeating: " ", count: 10) + String(repeating: "b", count: 100)
        let limited = GreetingPlaces.limitInput(spaced)
        #expect(GreetingPlaces.length(GreetingPlaces.normalize(limited)) == 80)
        #expect(limited.hasPrefix("a" + String(repeating: " ", count: 10) + "b"))
        // Scalars, not graphemes.
        let flags = String(repeating: "🇷🇸", count: 50)
        #expect(GreetingPlaces.length(GreetingPlaces.limitInput(flags)) == 80)
        // Whatever the field holds, the server would keep.
        for raw in [long, spaced, flags, "  \u{0001}Oslo\u{0002}  ", String(repeating: "я ", count: 60)] {
            let held = GreetingPlaces.limitInput(raw)
            guard case .success = GreetingPlaces.prepare([held]) else {
                Issue.record("limited input refused: \(held.debugDescription)")
                continue
            }
        }
    }

    @Test("a list the server already holds is not sent again")
    func needsSave() {
        #expect(!GreetingPlaces.needsSave(drafts: ["Moscow"], stored: ["Moscow"]))
        #expect(!GreetingPlaces.needsSave(drafts: [" Moscow ", ""], stored: ["Moscow"]))
        #expect(!GreetingPlaces.needsSave(drafts: ["Moscow", "moscow"], stored: ["Moscow"]))
        #expect(GreetingPlaces.needsSave(drafts: ["Moscow", "Belgrade"], stored: ["Moscow"]))
        #expect(GreetingPlaces.needsSave(drafts: ["Belgrade", "Moscow"], stored: ["Moscow", "Belgrade"]))
        #expect(GreetingPlaces.needsSave(drafts: [], stored: ["Moscow"]))
        #expect(!GreetingPlaces.needsSave(drafts: [""], stored: []))
    }
}

// MARK: - The credit

@Suite("Greeting places — the weather credit")
struct GreetingWeatherCreditTests {

    /// The credit as the server writes it into a greeting, in each of its
    /// nine languages (`FooterWords` in server/src/lookups.rs).
    static let credits = [
        "Weather data by Open-Meteo.com",
        "Wetterdaten von Open-Meteo.com",
        "Datos meteorológicos de Open-Meteo.com",
        "Données météo par Open-Meteo.com",
        "気象データ: Open-Meteo.com",
        "Данные о погоде: Open-Meteo.com",
        "Подаци о времену: Open-Meteo.com",
        "Podaci o vremenu: Open-Meteo.com",
        "天气数据：Open-Meteo.com",
    ]

    static func greeting(_ credit: String) -> String {
        "Good morning! Moscow (Russia) will be sunny, 12 °C at most; Belgrade (Serbia) may see rain.\n\n"
            + "[\(credit)](https://open-meteo.com/)"
    }

    @Test("the credit under a greeting is a tappable link under its own words, in every language")
    func tappable() {
        for credit in Self.credits {
            let attributed = MessageLinks.attributedBody(Self.greeting(credit), isMine: false)
            let links = attributed.runs.compactMap { $0.link?.absoluteString }
            #expect(links == ["https://open-meteo.com/"], "\(credit): \(links)")
            let drawn = String(attributed.characters)
            #expect(drawn.contains(credit), "\(credit): \(drawn)")
            #expect(!drawn.contains("](https://"), "markdown left raw: \(drawn)")
        }
    }

    /// Recognised as the server's footer, so no Apple device fetches the
    /// provider's page to build a card for a greeting nobody asked about.
    @Test("a greeting carrying the credit gets no link-preview card")
    func noPreviewCard() {
        for credit in Self.credits {
            let body = Self.greeting(credit)
            #expect(AssistantSources.hasFooter(body), "\(credit)")
            #expect(MessageLinks.firstWebLinkAsDrawn(in: body) == nil, "\(credit)")
        }
    }

    @Test("a greeting without weather is untouched")
    func plainGreeting() {
        let body = "Good morning, Leos and Virgos!"
        #expect(!AssistantSources.hasFooter(body))
        #expect(MessageLinks.attributedBody(body, isMine: false).runs.compactMap(\.link).isEmpty)
    }
}
