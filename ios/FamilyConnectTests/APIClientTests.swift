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
    private var value = 0

    @discardableResult
    func increment() -> Int {
        lock.lock(); defer { lock.unlock() }
        value += 1
        return value
    }
}
