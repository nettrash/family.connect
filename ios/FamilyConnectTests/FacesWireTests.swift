//
//  FacesWireTests.swift
//  FamilyConnectTests
//
//  The fifth switch's wire field, byte for byte (docs/protocol.md, "Profile
//  pictures of members").
//
//  `ai_faces` is one line of hand-written decoder and the PATCH that sets it
//  is one line of hand-written encoder, exactly as its four neighbours are —
//  and each of those has this pin. The absent-key case matters most: a
//  server that predates the field sends no face, which is what `false`
//  reports; a decoder that defaulted it the other way would show a family a
//  switch their server does not have.
//

import Foundation
import Testing
@testable import FamilyConnect

struct FacesWireTests {

    private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        try APICoding.decoder().decode(T.self, from: Data(json.utf8))
    }

    @Test("ai_faces reads as off from a server that predates it")
    func aiFacesDefaultsOff() throws {
        let family = try decode(
            FamilyDTO.self,
            """
            {"id": 3, "name": "The Smiths", "join_policy": "open",
             "created_at": "2026-08-19T17:03:12Z"}
            """)
        #expect(!family.aiFaces)
        #expect(!family.aiGreeting)
        #expect(!family.aiHistoryPhotos)
    }

    @Test("ai_faces is read when the server does send it")
    func aiFacesDecodes() throws {
        let on = try decode(
            FamilyDTO.self,
            """
            {"id": 3, "name": "The Smiths", "join_policy": "open",
             "created_at": "2026-08-19T17:03:12Z", "ai_vision": true, "ai_faces": true}
            """)
        #expect(on.aiFaces)
        let off = try decode(
            FamilyDTO.self,
            """
            {"id": 3, "name": "The Smiths", "join_policy": "open",
             "created_at": "2026-08-19T17:03:12Z", "ai_faces": false}
            """)
        #expect(!off.aiFaces)
    }

    /// One key and nothing else — a request to set the faces must not carry
    /// an `ai_vision: false` along that would switch a family's pictures off
    /// as a side effect.
    @Test("setting the faces switch sends exactly {ai_faces: …}")
    func setAIFacesSendsOneKey() async throws {
        let host = "faces-wire.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { _ in
            .json(200, """
                {"family": {"id": 3, "name": "The Smiths", "join_policy": "open",
                            "created_at": "2026-08-19T17:03:12Z",
                            "ai_vision": true, "ai_faces": true}}
                """)
        }
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())

        let family = try await api.setAIFaces(true)
        #expect(family.aiFaces)

        let patch = try #require(StubURLProtocol.requests(host: host).first { $0.method == "PATCH" })
        let body = try #require(patch.bodyJSON())
        #expect(body["ai_faces"] as? Bool == true)
        #expect(body.count == 1, "one key and nothing else, got \(body)")
    }
}
