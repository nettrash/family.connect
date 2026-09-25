//
//  GreetingWireTests.swift
//  FamilyConnectTests
//
//  The daily greeting's two wire fields, byte for byte (docs/protocol.md,
//  "The daily greeting").
//
//  `ai_greeting` on the Family object and `greetings_enabled` on GET /me are
//  each one line of hand-written decoder, and the PATCH that sets the first
//  is one line of hand-written encoder. The sibling switches pin all three
//  shapes; this one shipped with none, which is how a renamed key or a
//  defaulted-true decode would have gone unnoticed with every other test
//  green. The absent-key cases are the ones that matter most: a server that
//  predates the field must read as "off", not as "missing".
//

import Foundation
import Testing
@testable import FamilyConnect

struct GreetingWireTests {

    private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        try APICoding.decoder().decode(T.self, from: Data(json.utf8))
    }

    // MARK: - ai_greeting on the Family object

    /// FALSE when absent, and for the protocol's own reason rather than a
    /// compatibility guess: a server that predates the field posts no
    /// greeting, which is exactly what `false` reports.
    @Test("ai_greeting reads as off from a server that predates it")
    func aiGreetingDefaultsOff() throws {
        let family = try decode(
            FamilyDTO.self,
            """
            {"id": 3, "name": "The Smiths", "join_policy": "open",
             "created_at": "2026-08-19T17:03:12Z"}
            """)
        #expect(!family.aiGreeting)
        // Its neighbours are untouched by the new field.
        #expect(family.aiHistory)
        #expect(!family.aiVision)
        #expect(!family.aiHistoryPhotos)
    }

    @Test("ai_greeting is read when the server does send it")
    func aiGreetingDecodes() throws {
        let on = try decode(
            FamilyDTO.self,
            """
            {"id": 3, "name": "The Smiths", "join_policy": "open",
             "created_at": "2026-08-19T17:03:12Z", "ai_greeting": true}
            """)
        #expect(on.aiGreeting)

        let off = try decode(
            FamilyDTO.self,
            """
            {"id": 3, "name": "The Smiths", "join_policy": "open",
             "created_at": "2026-08-19T17:03:12Z", "ai_greeting": false}
            """)
        #expect(!off.aiGreeting)
    }

    // MARK: - greetings_enabled on GET /me

    @Test("greetings_enabled reads as false from a server that predates it")
    func greetingsEnabledDefaultsOff() throws {
        let me = try decode(
            MeResponse.self,
            """
            {"user": {"id": 7, "username": "anna", "display_name": "Anna"},
             "family": null, "role": null, "pending_join_request": null}
            """)
        #expect(!me.greetingsEnabled)
    }

    @Test("greetings_enabled is read when the server does send it")
    func greetingsEnabledDecodes() throws {
        let me = try decode(
            MeResponse.self,
            """
            {"user": {"id": 7, "username": "anna", "display_name": "Anna"},
             "family": null, "role": null, "pending_join_request": null,
             "greetings_enabled": true}
            """)
        #expect(me.greetingsEnabled)
    }

    // MARK: - The PATCH

    /// One key and nothing else. `FamilyPatchRequest` encodes only the fields
    /// that were touched, so a request to set the greeting must not carry a
    /// neighbour it was not asked about — an `ai_vision: false` riding along
    /// would switch a family's pictures off as a side effect of turning a
    /// greeting on.
    @Test("setting the greeting sends exactly {ai_greeting: …}")
    func setAIGreetingSendsOneKey() async throws {
        let host = "greeting-wire.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { _ in
            .json(200, """
                {"family": {"id": 3, "name": "The Smiths", "join_policy": "open",
                            "created_at": "2026-08-19T17:03:12Z", "ai_greeting": true}}
                """)
        }
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())

        let family = try await api.setAIGreeting(true)
        #expect(family.aiGreeting)

        let sent = StubURLProtocol.requests(host: host)
        let patch = try #require(sent.first { $0.method == "PATCH" })
        #expect(patch.url.path.hasSuffix("/families/mine"))
        let body = try #require(patch.bodyJSON())
        #expect(body["ai_greeting"] as? Bool == true)
        #expect(body.count == 1, "one key and nothing else, got \(body)")
    }
}
