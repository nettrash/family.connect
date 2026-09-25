//
//  MentionWireTests.swift
//  FamilyConnectTests
//
//  The mention's wire shapes, byte for byte (docs/protocol.md, "Mentioning
//  a member"): `mentions` on a Message, `mentioned` on a chat-list entry,
//  and the list on the send frame and the REST body — absent, never null,
//  when a message names nobody.
//

import Foundation
import Testing
@testable import FamilyConnect

struct MentionWireTests {

    private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        try APICoding.decoder().decode(T.self, from: Data(json.utf8))
    }

    @Test("a message's mentions decode, and are nil when absent")
    func messageMentions() throws {
        let named = try decode(
            MessageDTO.self,
            """
            {"id": 1338, "chat_id": 42, "sender_id": 7,
             "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
             "body": "@Anna are you in?", "created_at": "2026-08-19T17:03:12Z",
             "mentions": [{"user_id": 9, "name": "Anna"}]}
            """)
        #expect(named.mentions == [MentionDTO(userID: 9, name: "Anna")])
        let plain = try decode(
            MessageDTO.self,
            """
            {"id": 1338, "chat_id": 42, "sender_id": 7,
             "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
             "body": "Dinner at 7?", "created_at": "2026-08-19T17:03:12Z"}
            """)
        #expect(plain.mentions == nil)
    }

    @Test("a chat-list entry's mentioned reads true, and nil when absent")
    func chatListMentioned() throws {
        let marked = try decode(
            ChatListItemDTO.self,
            """
            {"chat": {"id": 42, "kind": "family", "title": "The Smiths", "peer_user_id": null},
             "last_message": null, "unread_count": 2, "last_read_message_id": 10, "mentioned": true}
            """)
        #expect(marked.mentioned == true)
        let plain = try decode(
            ChatListItemDTO.self,
            """
            {"chat": {"id": 42, "kind": "family", "title": "The Smiths", "peer_user_id": null},
             "last_message": null, "unread_count": 2, "last_read_message_id": 10}
            """)
        #expect(plain.mentioned == nil)
    }

    @Test("the send frame carries mentions, and omits the key without them")
    func sendFrame() throws {
        let named = ClientFrame.send(
            chatID: 42, clientMsgID: "e7a1d9c3-0000-4000-8000-000000000001",
            body: "@Anna are you in?", replyToMessageID: nil, attachmentIDs: nil,
            pollOptions: nil, mentions: [MentionDTO(userID: 9, name: "Anna")])
        let json = try JSONSerialization.jsonObject(with: APICoding.encoder().encode(named)) as? [String: Any] ?? [:]
        let list = try #require(json["mentions"] as? [[String: Any]])
        #expect(list.count == 1)
        #expect(list[0]["user_id"] as? Int == 9)
        #expect(list[0]["name"] as? String == "Anna")
        #expect(json.count == 5, "type, chat_id, client_msg_id, body, mentions")

        let plain = ClientFrame.send(
            chatID: 42, clientMsgID: "x", body: "hi", replyToMessageID: nil,
            attachmentIDs: nil, pollOptions: nil, mentions: nil)
        let plainJSON = try JSONSerialization.jsonObject(with: APICoding.encoder().encode(plain)) as? [String: Any] ?? [:]
        #expect(plainJSON["mentions"] == nil)
        #expect(plainJSON.count == 4)
    }

    @Test("the REST body carries the same list")
    func restBody() async throws {
        let host = "mention-wire.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { _ in
            .json(201, """
                {"message": {"id": 1338, "chat_id": 42, "sender_id": 7, "client_msg_id": "u1",
                             "body": "@Anna?", "created_at": "2026-08-19T17:03:12Z",
                             "mentions": [{"user_id": 9, "name": "Anna"}]}}
                """)
        }
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())
        let message = try await api.sendMessage(
            chatID: 42, clientMsgID: "u1", body: "@Anna?",
            mentions: [MentionDTO(userID: 9, name: "Anna")])
        #expect(message.mentions?.first?.userID == 9)
        let sent = try #require(StubURLProtocol.requests(host: host).first)
        let body = try #require(sent.bodyJSON())
        let list = try #require(body["mentions"] as? [[String: Any]])
        #expect(list.first?["name"] as? String == "Anna")
    }
}
