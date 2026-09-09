//
//  ThreadWireTests.swift
//  FamilyConnectTests
//
//  The chain's two wire fields and its read, byte for byte
//  (docs/protocol.md, "Threads").
//
//  Both fields are ABSENT, never null or 0, on a message that is neither a
//  reply nor a root somebody answered — and a server that predates threads
//  sends neither, which must decode as "no chain" rather than fail.
//

import Foundation
import Testing
@testable import FamilyConnect

struct ThreadWireTests {

    private func decode(_ json: String) throws -> MessageDTO {
        try APICoding.decoder().decode(MessageDTO.self, from: Data(json.utf8))
    }

    @Test("a message with neither field decodes as no chain")
    func neitherFieldIsNoChain() throws {
        let message = try decode(
            """
            {"id": 1338, "chat_id": 42, "sender_id": 7,
             "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
             "body": "Dinner at 7?", "created_at": "2026-08-19T17:03:12Z"}
            """)
        #expect(message.threadRootID == nil)
        #expect(message.replyCount == nil)
    }

    @Test("a reply names its root and a root carries its count")
    func fieldsDecode() throws {
        let reply = try decode(
            """
            {"id": 1339, "chat_id": 42, "sender_id": 9,
             "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a02",
             "body": "Works for me", "created_at": "2026-08-19T17:04:12Z",
             "reply_to": {"message_id": 1338, "sender_id": 7, "excerpt": "Dinner at 7?"},
             "thread_root_id": 1338}
            """)
        #expect(reply.threadRootID == 1338)
        #expect(reply.replyCount == nil, "a reply counts nothing of its own")

        let root = try decode(
            """
            {"id": 1338, "chat_id": 42, "sender_id": 7,
             "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
             "body": "Dinner at 7?", "created_at": "2026-08-19T17:03:12Z",
             "reply_count": 3}
            """)
        #expect(root.replyCount == 3)
        #expect(root.threadRootID == nil, "a root names no root")
    }

    /// The path names the message asked about — root or reply, the server
    /// resolves it — and `after_id` rides only when the caller pages.
    @Test("the thread read asks the right path, with and without after_id")
    func threadReadPath() async throws {
        let host = "thread-wire.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { _ in .json(200, #"{"messages": []}"#) }
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())

        _ = try await api.thread(chatID: 42, messageID: 1340)
        _ = try await api.thread(chatID: 42, messageID: 1340, afterID: 1339, limit: 2)

        let sent = StubURLProtocol.requests(host: host)
        #expect(sent.count == 2)
        #expect(sent[0].method == "GET")
        #expect(sent[0].url.path.hasSuffix("/chats/42/messages/1340/thread"))
        #expect(sent[0].url.query?.contains("after_id") == false)
        #expect(sent[1].url.query?.contains("after_id=1339") == true)
        #expect(sent[1].url.query?.contains("limit=2") == true)
    }
}
