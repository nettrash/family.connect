//
//  APIClientTests.swift
//  FamilyConnectTests
//
//  URL construction, auth header, DTO decoding, error mapping and the
//  GET-only transient retry — everything about APIClient short of a real
//  network, which StubURLProtocol replaces.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("APIClient")
struct APIClientTests {

    private func makeClient(host: String, handler: @escaping StubURLProtocol.Handler) -> APIClient {
        StubURLProtocol.register(host: host, handler: handler)
        return APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())
    }

    // MARK: - Leaving a family

    /// Two shapes from one endpoint, and the empty one is the COMMON case:
    /// an ordinary member leaving gets `204` with no body at all, which a
    /// decoder handed straight to it reads as malformed JSON.
    @Test("leaving answers with the successor, or with nothing at all")
    func leaveFamilyBothShapes() async throws {
        let host = "api-leave.test"
        defer { StubURLProtocol.unregister(host: host) }
        // A box rather than captured vars: the stub's handler is a
        // Sendable closure, and every request here is awaited before the
        // script changes, so the box is never touched from two sides.
        final class Script: @unchecked Sendable {
            var body = ""
            var status = 204
        }
        let script = Script()
        let client = makeClient(host: host) { _ in
            script.status == 204 ? .empty(204) : .json(script.status, script.body)
        }

        // 204, no body: nobody inherited. Must not throw.
        #expect(try await client.leaveFamily() == nil)

        // 200 with a successor: the id the leaving owner resolves against
        // the roster it still holds.
        script.status = 200
        script.body = #"{"new_owner_user_id": 11}"#
        #expect(try await client.leaveFamily() == 11)

        // A 200 whose body omits the key — a server that answered 200 for
        // its own reasons — is "nobody", not a decode failure.
        script.body = "{}"
        #expect(try await client.leaveFamily() == nil)
    }

    @Test("URLs are {base}/api/v1{path} with query items")
    func urlBuilding() async throws {
        let host = "api-urls.test"
        defer { StubURLProtocol.unregister(host: host) }
        let client = makeClient(host: host) { request in
            switch (request.method, request.url.path()) {
            case ("GET", "/api/v1/me"):
                return .json(200, Self.meActiveJSON)
            case ("GET", "/api/v1/chats/42/messages"):
                return .json(200, #"{"messages": []}"#)
            case ("POST", "/api/v1/families/join-requests/12/approve"):
                return .json(200, #"{"member": {"id": 9, "username": "kid", "display_name": "Kid", "role": "member"}}"#)
            default:
                return .json(404, #"{"error": {"code": "chat_not_found", "message": "no"}}"#)
            }
        }

        _ = try await client.me()
        _ = try await client.messages(chatID: 42, afterID: 7, limit: 100)
        _ = try await client.approveJoinRequest(id: 12)

        let requests = StubURLProtocol.requests(host: host)
        #expect(requests.count == 3)
        #expect(requests[0].method == "GET")
        #expect(requests[0].url.path() == "/api/v1/me")

        let messagesURL = requests[1].url
        #expect(messagesURL.path() == "/api/v1/chats/42/messages")
        let query = URLComponents(url: messagesURL, resolvingAgainstBaseURL: false)?.queryItems ?? []
        #expect(query.contains(URLQueryItem(name: "after_id", value: "7")))
        #expect(query.contains(URLQueryItem(name: "limit", value: "100")))

        #expect(requests[2].method == "POST")
        #expect(requests[2].url.path() == "/api/v1/families/join-requests/12/approve")
    }

    @Test("Authorization: Bearer <token> once configured; absent before")
    func bearerHeader() async throws {
        let host = "api-auth.test"
        defer { StubURLProtocol.unregister(host: host) }
        let client = makeClient(host: host) { _ in .json(200, Self.meActiveJSON) }

        _ = try? await client.me()
        await client.setToken("sekret-token")
        _ = try await client.me()

        let requests = StubURLProtocol.requests(host: host)
        #expect(requests.count == 2)
        #expect(requests[0].headers["Authorization"] == nil)
        #expect(requests[1].headers["Authorization"] == "Bearer sekret-token")
    }

    @Test("DTOs decode: snake_case keys and RFC3339 dates")
    func dtoDecoding() async throws {
        let host = "api-decode.test"
        defer { StubURLProtocol.unregister(host: host) }
        let client = makeClient(host: host) { _ in .json(200, Self.meActiveJSON) }

        let me = try await client.me()
        #expect(me.user.id == 7)
        #expect(me.user.displayName == "Anna")
        #expect(me.family?.name == "The Smiths")
        #expect(me.family?.joinPolicy == "approval")
        #expect(me.role == "owner")
        #expect(me.pendingJoinRequest == nil)

        let expectedDate = ISO8601DateFormatter().date(from: "2026-08-19T17:03:12Z")
        #expect(me.user.createdAt == expectedDate)
        // The fixture predates profile pictures, exactly like a server
        // that has not been updated yet.
        #expect(me.user.avatarVersion == 0)
    }

    /// The quote rides the ordinary Message shape and is ABSENT — not
    /// null — on a message that is not a reply, which is what lets a
    /// client predating replies decode a server that has them.
    @Test("reply_to decodes when present and is nil when absent")
    func replyToDecoding() throws {
        let decoder = APICoding.decoder()

        let plain = Data(#"""
        {"id": 1338, "chat_id": 42, "sender_id": 7, "client_msg_id": null,
         "body": "Dinner at 7?", "created_at": "2026-08-19T17:03:12Z"}
        """#.utf8)
        #expect(try decoder.decode(MessageDTO.self, from: plain).replyTo == nil)

        let reply = Data(#"""
        {"id": 1339, "chat_id": 42, "sender_id": 9, "client_msg_id": null,
         "body": "Six works", "created_at": "2026-08-19T17:04:00Z",
         "reply_to": {"message_id": 1338, "sender_id": 7, "excerpt": "Dinner at 7?"}}
        """#.utf8)
        let decoded = try decoder.decode(MessageDTO.self, from: reply)
        #expect(decoded.replyTo?.messageID == 1338)
        #expect(decoded.replyTo?.senderID == 7)
        #expect(decoded.replyTo?.excerpt == "Dinner at 7?")
    }

    /// The protocol's compatibility rule, pinned. Swift's synthesized
    /// Decodable does NOT fall back to a property's default for a missing
    /// key — it throws — so a defaulted `avatarVersion` alone would make
    /// every response from a pre-avatars server undecodable. UserDTO and
    /// MemberDTO hand-write init(from:) for exactly this reason; this
    /// test is what catches it if someone deletes them.
    @Test("A missing avatar_version decodes as no picture")
    func avatarVersionIsOptionalOnTheWire() throws {
        let decoder = APICoding.decoder()

        let withoutField = Data(#"{"id": 7, "username": "anna", "display_name": "Anna"}"#.utf8)
        let user = try decoder.decode(UserDTO.self, from: withoutField)
        #expect(user.avatarVersion == 0)

        let withField = Data(
            #"{"id": 7, "username": "anna", "display_name": "Anna", "avatar_version": 5}"#.utf8)
        #expect(try decoder.decode(UserDTO.self, from: withField).avatarVersion == 5)

        let memberWithout = Data(
            #"{"id": 8, "username": "ben", "display_name": "Ben", "role": "member"}"#.utf8)
        #expect(try decoder.decode(MemberDTO.self, from: memberWithout).avatarVersion == 0)

        let memberWith = Data(
            #"{"id": 8, "username": "ben", "display_name": "Ben", "role": "member", "avatar_version": 3}"#.utf8)
        #expect(try decoder.decode(MemberDTO.self, from: memberWith).avatarVersion == 3)
    }

    @Test("401 maps to .unauthorized")
    func unauthorizedMapping() async throws {
        let host = "api-401.test"
        defer { StubURLProtocol.unregister(host: host) }
        let client = makeClient(host: host) { _ in
            .json(401, #"{"error": {"code": "unauthorized", "message": "session expired"}}"#)
        }

        await #expect(throws: APIError.unauthorized) {
            _ = try await client.me()
        }
    }

    @Test("GET retries once on 503; the retry succeeds")
    func getRetriesOnTransient() async throws {
        let host = "api-retry-get.test"
        defer { StubURLProtocol.unregister(host: host) }
        let counter = Counter()
        let client = makeClient(host: host) { _ in
            counter.increment() == 1
                ? .json(503, #"{"error": {"code": "internal", "message": "restarting"}}"#, headers: ["Retry-After": "0"])
                : .json(200, Self.meActiveJSON)
        }

        let me = try await client.me()
        #expect(me.user.id == 7)
        #expect(StubURLProtocol.requests(host: host).count == 2)
    }

    @Test("POST does NOT retry on 503")
    func postDoesNotRetry() async throws {
        let host = "api-retry-post.test"
        defer { StubURLProtocol.unregister(host: host) }
        let client = makeClient(host: host) { _ in
            .json(503, #"{"error": {"code": "internal", "message": "down"}}"#)
        }

        await #expect(throws: APIError.server(status: 503, message: "down")) {
            _ = try await client.sendMessage(chatID: 1, clientMsgID: "u-1", body: "hi")
        }
        #expect(StubURLProtocol.requests(host: host).count == 1)
    }

    @Test("Error body code/message are extracted (409 conflict)")
    func errorBodyExtraction() async throws {
        let host = "api-409.test"
        defer { StubURLProtocol.unregister(host: host) }
        let client = makeClient(host: host) { _ in
            .json(409, #"{"error": {"code": "username_taken", "message": "username is already in use"}}"#)
        }

        await #expect(throws: APIError.conflict(code: "username_taken", message: "username is already in use")) {
            _ = try await client.register(username: "anna", displayName: "Anna", password: "12345678")
        }
    }

    @Test("probe: 401 with parseable error body = ours; anything else = not")
    func probe() async throws {
        let okHost = "probe-ok.test"
        let badHost = "probe-bad.test"
        defer {
            StubURLProtocol.unregister(host: okHost)
            StubURLProtocol.unregister(host: badHost)
        }
        StubURLProtocol.register(host: okHost) { _ in
            .json(401, #"{"error": {"code": "unauthorized", "message": "auth required"}}"#)
        }
        StubURLProtocol.register(host: badHost) { _ in
            StubResponse(status: 401, headers: [:], body: Data("<html>login</html>".utf8))
        }
        let client = APIClient(serverURL: nil, session: StubURLProtocol.makeSession())

        #expect(await client.probe(serverURL: URL(string: "https://\(okHost)")!))
        #expect(!(await client.probe(serverURL: URL(string: "https://\(badHost)")!)))
    }

    @Test("send message body carries client_msg_id and body verbatim")
    func sendMessageBody() async throws {
        let host = "api-send-body.test"
        defer { StubURLProtocol.unregister(host: host) }
        let client = makeClient(host: host) { _ in
            .json(201, Self.messageJSON(id: 1338, chatID: 42, senderID: 7, clientMsgID: "uuid-1", body: "Dinner at 7?"))
        }

        _ = try await client.sendMessage(chatID: 42, clientMsgID: "uuid-1", body: "Dinner at 7?")

        let sent = StubURLProtocol.requests(host: host).first
        let json = sent?.bodyJSON()
        #expect(json?["client_msg_id"] as? String == "uuid-1")
        #expect(json?["body"] as? String == "Dinner at 7?")
    }

    // MARK: - Fixtures

    static let meActiveJSON = """
    {"user": {"id": 7, "username": "anna", "display_name": "Anna", "created_at": "2026-08-19T17:03:12Z"},
     "family": {"id": 3, "name": "The Smiths", "join_policy": "approval", "created_at": "2026-08-01T10:00:00Z"},
     "role": "owner",
     "pending_join_request": null}
    """

    static func messageJSON(id: Int64, chatID: Int64, senderID: Int64, clientMsgID: String, body: String) -> String {
        """
        {"message": {"id": \(id), "chat_id": \(chatID), "sender_id": \(senderID),
         "client_msg_id": "\(clientMsgID)", "body": "\(body)", "created_at": "2026-08-19T17:05:00Z"}}
        """
    }
}

/// Tiny thread-safe counter for handlers that vary by call ordinal.
final class Counter: @unchecked Sendable {
    private let lock = NSLock()
    private var count = 0

    /// How many times it has been incremented so far.
    var value: Int {
        lock.lock(); defer { lock.unlock() }
        return count
    }

    @discardableResult
    func increment() -> Int {
        lock.lock(); defer { lock.unlock() }
        count += 1
        return count
    }
}
