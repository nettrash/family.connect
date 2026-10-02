//
//  StickerWireTests.swift
//  FamilyConnectTests
//
//  The sticker pack on the wire (docs/protocol.md, "Sticker pack"): the
//  one new field on an attachment, the PackItem and its tombstone, the
//  `pack_item` frame, the flag on a send, and the three new fields of
//  `GET /families/mine` whose ABSENCE is how this client knows a server
//  predates the pack.
//
//  Every optional here is absent-not-false, and half of these tests are
//  about what is NOT written: an ordinary photo and an ordinary send must
//  stay byte-identical to what they were before stickers existed, or every
//  old server and every old client is handed a field it never asked for.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("Sticker wire shapes")
struct StickerWireTests {

    private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        try APICoding.decoder().decode(type, from: Data(json.utf8))
    }

    private func fields(of frame: ClientFrame) throws -> [String: Any] {
        let data = Data(try frame.encodedString().utf8)
        return try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    // MARK: - The flag on an attachment

    @Test("sticker: true decodes; its absence is false")
    func attachmentFlagDecodes() throws {
        let sticker = try decode(AttachmentDTO.self, """
            {"id": 90, "kind": "photo", "mime": "image/webp", "size": 38,
             "width": 8, "height": 8, "has_preview": false, "sticker": true}
            """)
        #expect(sticker.sticker)
        let photo = try decode(AttachmentDTO.self, """
            {"id": 91, "kind": "photo", "mime": "image/jpeg", "size": 4096,
             "width": 1600, "height": 1200, "has_preview": true}
            """)
        #expect(!photo.sticker)
    }

    @Test("the flag is written only when true, so a stored photo is unchanged")
    func attachmentFlagEncodesOnlyWhenTrue() throws {
        func keys(sticker: Bool) throws -> Set<String> {
            let dto = AttachmentDTO(
                id: 90, kind: "photo", mime: "image/webp", size: 38, width: 8, height: 8,
                durationMS: nil, hasPreview: false, name: nil, latitude: nil, longitude: nil,
                accuracyM: nil, sticker: sticker)
            let object = try JSONSerialization.jsonObject(with: JSONEncoder().encode(dto))
            return Set(try #require(object as? [String: Any]).keys)
        }
        #expect(try keys(sticker: true).contains("sticker"))
        // Exactly the keys a photo has always stored — no `sticker`, and no
        // nulls for the fields it has no value for.
        #expect(try keys(sticker: false) == ["id", "kind", "mime", "size", "width", "height", "has_preview"])
    }

    @Test("a stored message keeps the flag through the store and back")
    func theRowKeepsTheFlag() {
        let dto = AttachmentDTO(
            id: 90, kind: "photo", mime: "image/webp", size: 38, width: 8, height: 8,
            durationMS: nil, hasPreview: false, name: nil, latitude: nil, longitude: nil,
            accuracyM: nil, sticker: true)
        let row = MessageEntity(
            localID: "s:1", serverID: 1, chatID: 42, senderID: 9, body: "",
            createdAt: Date(timeIntervalSince1970: 0), status: .sent, attachments: [dto])
        #expect(row.attachmentList.first?.sticker == true)
        #expect(row.attachmentList == [dto])
    }

    // MARK: - PackItem

    @Test("a live pack item decodes, and its attachment carries no sticker flag")
    func packItemDecodes() throws {
        let item = try decode(PackItemDTO.self, """
            {"id": 5, "added_by": 7, "label": "party cat", "created_at": "2026-09-13T10:00:00Z",
             "pack_seq": 12,
             "attachment": {"id": 71, "kind": "photo", "mime": "image/webp", "size": 38,
                            "width": 512, "height": 512, "has_preview": false}}
            """)
        #expect(item.id == 5)
        #expect(item.addedBy == 7)
        #expect(item.label == "party cat")
        #expect(item.packSeq == 12)
        #expect(item.attachment?.id == 71)
        #expect(item.attachment?.sticker == false)
        #expect(!item.isTombstone)
    }

    @Test("a tombstone is an id, a seq and deleted — and nothing else is required")
    func packTombstoneDecodes() throws {
        let item = try decode(PackItemDTO.self, #"{"id": 5, "deleted": true, "pack_seq": 14}"#)
        #expect(item.isTombstone)
        #expect(item.packSeq == 14)
        #expect(item.attachment == nil)
        #expect(item.addedBy == nil)
        #expect(item.label == nil)
    }

    @Test("the full read and the change feed decode")
    func packEnvelopesDecode() throws {
        let full = try decode(PackResponse.self, #"{"items": [], "max_pack_seq": 0}"#)
        #expect(full.items.isEmpty)
        #expect(full.maxPackSeq == 0)
        let page = try decode(PackChangesResponse.self, """
            {"items": [{"id": 5, "deleted": true, "pack_seq": 14}]}
            """)
        #expect(page.items.count == 1)
    }

    // MARK: - GET /families/mine

    private static let family = """
        "family": {"id": 1, "name": "The Smiths", "join_policy": "open"},
        "members": [], "blocked_user_ids": []
        """

    @Test("a server with packs reports both limits, and the cursor once the pack was written to")
    func familyCarriesThePack() throws {
        let mine = try decode(FamilyMineResponse.self, """
            {\(Self.family), "max_pack_seq": 14, "max_pack_items": 200,
             "max_pack_item_bytes": 524288}
            """)
        #expect(mine.maxPackSeq == 14)
        #expect(mine.maxPackItems == 200)
        #expect(mine.maxPackItemBytes == 524_288)

        let untouched = try decode(FamilyMineResponse.self, """
            {\(Self.family), "max_pack_items": 200, "max_pack_item_bytes": 524288}
            """)
        #expect(untouched.maxPackSeq == nil)
        #expect(untouched.maxPackItems == 200)
    }

    @Test("a server that predates the pack omits the limits, and that absence is the answer")
    func anOldServerHasNoPack() throws {
        let mine = try decode(FamilyMineResponse.self, "{\(Self.family)}")
        #expect(mine.maxPackItems == nil)
        #expect(mine.maxPackItemBytes == nil)
        #expect(mine.maxPackSeq == nil)
    }

    // MARK: - Frames

    @Test("a sticker goes as a send frame with the flag, one attachment and an empty body")
    func stickerSendFrame() throws {
        let json = try fields(of: .sendSticker(
            chatID: 42, clientMsgID: "abc", replyToMessageID: nil, attachmentID: 90))
        #expect(json["type"] as? String == "send")
        #expect(json["chat_id"] as? Int == 42)
        #expect(json["client_msg_id"] as? String == "abc")
        #expect(json["body"] as? String == "")
        #expect(json["attachment_ids"] as? [Int] == [90])
        #expect(json["sticker"] as? Bool == true)
        #expect(json["reply_to_message_id"] == nil)
        #expect(json["poll"] == nil)
        #expect(json["mentions"] == nil)
    }

    @Test("a sticker may be a reply")
    func stickerReplyFrame() throws {
        let json = try fields(of: .sendSticker(
            chatID: 42, clientMsgID: "abc", replyToMessageID: 600, attachmentID: 90))
        #expect(json["reply_to_message_id"] as? Int == 600)
    }

    @Test("an ordinary send frame says nothing about stickers")
    func ordinarySendFrameIsUnchanged() throws {
        let json = try fields(of: .send(
            chatID: 42, clientMsgID: "abc", body: "hello", replyToMessageID: nil,
            attachmentIDs: [34], pollOptions: nil, mentions: nil))
        #expect(json["sticker"] == nil)
    }

    @Test("pack_item decodes, live and tombstone")
    func packItemFrame() throws {
        let live = try decode(ServerFrame.self, """
            {"type": "pack_item", "item": {"id": 5, "added_by": 7, "created_at": "2026-09-13T10:00:00Z",
              "pack_seq": 12,
              "attachment": {"id": 71, "kind": "photo", "mime": "image/png", "size": 900,
                             "has_preview": false}}}
            """)
        guard case .packItem(let item) = live else {
            Issue.record("pack_item decoded as \(live)")
            return
        }
        #expect(item.id == 5)
        #expect(item.attachment?.mime == "image/png")

        let gone = try decode(ServerFrame.self, """
            {"type": "pack_item", "item": {"id": 5, "deleted": true, "pack_seq": 14}}
            """)
        guard case .packItem(let tombstone) = gone else {
            Issue.record("pack_item tombstone decoded as \(gone)")
            return
        }
        #expect(tombstone.isTombstone)
    }

    // MARK: - REST

    private func client(host: String, handler: @escaping StubURLProtocol.Handler) -> APIClient {
        StubURLProtocol.register(host: host, handler: handler)
        return APIClient(
            serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())
    }

    private static let messageJSON = """
        {"message": {"id": 900, "chat_id": 42, "sender_id": 7, "body": "",
         "created_at": "2026-09-13T10:00:00Z", "client_msg_id": "abc",
         "attachments": [{"id": 90, "kind": "photo", "mime": "image/webp", "size": 38,
                          "has_preview": false, "sticker": true}]}}
        """

    @Test("POST /messages carries sticker: true for a sticker and no such key otherwise")
    func sendMessageCarriesTheFlag() async throws {
        let host = "sticker-rest-send.test"
        let api = client(host: host) { _ in .json(201, Self.messageJSON) }
        defer { StubURLProtocol.unregister(host: host) }

        let sent = try await api.sendMessage(
            chatID: 42, clientMsgID: "abc", body: "", attachmentIDs: [90], sticker: true)
        #expect(sent.attachmentList.first?.sticker == true)
        _ = try await api.sendMessage(
            chatID: 42, clientMsgID: "def", body: "", attachmentIDs: [34])

        let bodies = StubURLProtocol.requests(host: host).compactMap { $0.bodyJSON() }
        #expect(bodies.count == 2)
        #expect(bodies[0]["sticker"] as? Bool == true)
        #expect(bodies[0]["body"] as? String == "")
        #expect(bodies[0]["attachment_ids"] as? [Int] == [90])
        #expect(bodies[1]["sticker"] == nil)
    }

    private static let itemJSON = """
        {"item": {"id": 5, "added_by": 7, "created_at": "2026-09-13T10:00:00Z", "pack_seq": 12,
          "attachment": {"id": 71, "kind": "photo", "mime": "image/webp", "size": 38,
                         "has_preview": false}}}
        """

    @Test("claiming an upload POSTs its id, and a 200 is the pack already holding it")
    func addPackItemTellsNewFromHeld() async throws {
        let host = "sticker-rest-add.test"
        let api = client(host: host) { request in
            let id = request.bodyJSON()?["attachment_id"] as? Int
            return .json(id == 71 ? 201 : 200, Self.itemJSON)
        }
        defer { StubURLProtocol.unregister(host: host) }

        let fresh = try await api.addPackItem(attachmentID: 71, label: "party cat")
        #expect(!fresh.alreadyHeld)
        // A second upload of the same bytes: the answer names the item that
        // was there, with the attachment id the PACK has — not the one sent.
        let again = try await api.addPackItem(attachmentID: 88, label: nil)
        #expect(again.alreadyHeld)
        #expect(again.item.attachment?.id == 71)

        let requests = StubURLProtocol.requests(host: host)
        #expect(requests.allSatisfy { $0.method == "POST" && $0.url.path() == "/api/v1/families/mine/pack" })
        #expect(requests[0].bodyJSON()?["label"] as? String == "party cat")
        // No label is no key, never a null or an empty string.
        #expect(requests[1].bodyJSON()?["label"] == nil)
    }

    @Test("the pack's reads and its removal go to the documented paths")
    func packPaths() async throws {
        let host = "sticker-rest-paths.test"
        let api = client(host: host) { request in
            switch (request.method, request.url.path()) {
            case ("GET", "/api/v1/families/mine/pack"):
                return .json(200, #"{"items": [], "max_pack_seq": 0}"#)
            case ("GET", "/api/v1/families/mine/pack/changes"):
                return .json(200, #"{"items": []}"#)
            default:
                return .empty(204)
            }
        }
        defer { StubURLProtocol.unregister(host: host) }

        _ = try await api.pack()
        _ = try await api.packChanges(afterSeq: 12, limit: 200)
        try await api.deletePackItem(id: 5)

        let requests = StubURLProtocol.requests(host: host)
        #expect(requests.map { "\($0.method) \($0.url.path())" } == [
            "GET /api/v1/families/mine/pack",
            "GET /api/v1/families/mine/pack/changes",
            "DELETE /api/v1/families/mine/pack/5",
        ])
        #expect(requests[1].url.query() == "after_seq=12&limit=200")
    }
}
