//
//  SocketFrameTests.swift
//  FamilyConnectTests
//
//  Golden-JSON tests in both directions against the byte shapes in
//  docs/protocol.md §WebSocket protocol. Client frames are encoded and
//  compared field-by-field (JSON key order isn't stable, so no string
//  comparison); server frames are decoded from the document's examples
//  verbatim. The compatibility rule — unknown type never throws — gets
//  its own test because deployed clients depend on it.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("SocketFrames")
struct SocketFrameTests {

    private func fields(of frame: ClientFrame) throws -> [String: Any] {
        let data = try APICoding.encoder().encode(frame)
        return (try JSONSerialization.jsonObject(with: data)) as? [String: Any] ?? [:]
    }

    // MARK: - Client → server

    @Test("send frame encodes per protocol.md")
    func encodeSend() throws {
        let json = try fields(of: .send(
            chatID: 42,
            clientMsgID: "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
            body: "Dinner at 7?",
            replyToMessageID: nil,
            attachmentIDs: nil,
            pollOptions: nil))
        #expect(json["type"] as? String == "send")
        #expect(json["chat_id"] as? Int == 42)
        #expect(json["client_msg_id"] as? String == "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01")
        #expect(json["body"] as? String == "Dinner at 7?")
        // Four keys, not five: an ordinary message must not carry
        // "reply_to_message_id": null — the protocol writes it as absent,
        // and the server's frame test asserts the same on its side.
        #expect(json.count == 4)
    }

    @Test("a reply's send frame carries the quoted message id")
    func encodeSendReply() throws {
        let json = try fields(of: .send(
            chatID: 42,
            clientMsgID: "1c4a9b02-0000-4000-8000-000000000001",
            body: "Six works",
            replyToMessageID: 1337,
            attachmentIDs: nil,
            pollOptions: nil))
        #expect(json["type"] as? String == "send")
        #expect(json["reply_to_message_id"] as? Int == 1337)
        #expect(json["body"] as? String == "Six works")
        #expect(json.count == 5)
    }

    @Test("a photo's send frame carries the attachment id array and an empty body")
    func encodeSendAttachment() throws {
        let json = try fields(of: .send(
            chatID: 42,
            clientMsgID: "1c4a9b02-0000-4000-8000-000000000002",
            body: "",
            replyToMessageID: nil,
            attachmentIDs: [34],
            pollOptions: nil))
        #expect(json["type"] as? String == "send")
        // The ARRAY spelling, even for one: the legacy `attachment_id`
        // is a one-element alias this client no longer sends.
        #expect(json["attachment_ids"] as? [Int] == [34])
        #expect(json["attachment_id"] == nil)
        // A photo needs no caption: the empty body is sent, not omitted.
        #expect(json["body"] as? String == "")
        #expect(json.count == 5)
    }

    @Test("a multi-attachment send frame matches the protocol.md literal")
    func encodeSendManyAttachments() throws {
        // The literal in protocol.md §Client → server:
        // {"type": "send", "chat_id": 42, "client_msg_id": "9d3f1e77-…",
        //  "body": "", "attachment_ids": [34, 35, 36]}
        let json = try fields(of: .send(
            chatID: 42,
            clientMsgID: "9d3f1e77-0000-4000-8000-000000000004",
            body: "",
            replyToMessageID: nil,
            attachmentIDs: [34, 35, 36],
            pollOptions: nil))
        #expect(json["type"] as? String == "send")
        #expect(json["chat_id"] as? Int == 42)
        #expect(json["body"] as? String == "")
        // In the SENDER'S order — the server preserves it on every read.
        #expect(json["attachment_ids"] as? [Int] == [34, 35, 36])
        #expect(json.count == 5)
    }

    @Test("a poll's send frame carries its options and the question as the body")
    func encodeSendPoll() throws {
        // The literal in protocol.md §Client → server:
        // {"type": "send", "chat_id": 42, "client_msg_id": "5b2e0c14-…",
        //  "body": "Pizza or pasta?", "poll": {"options": ["Pizza", "Pasta"]}}
        let json = try fields(of: .send(
            chatID: 42,
            clientMsgID: "5b2e0c14-0000-4000-8000-000000000003",
            body: "Pizza or pasta?",
            replyToMessageID: nil,
            attachmentIDs: nil,
            pollOptions: ["Pizza", "Pasta"]))
        #expect(json["type"] as? String == "send")
        // The QUESTION is the body — there is no question field, which is
        // the whole reason a poll costs no new case anywhere else.
        #expect(json["body"] as? String == "Pizza or pasta?")
        let poll = json["poll"] as? [String: Any]
        #expect(poll?["options"] as? [String] == ["Pizza", "Pasta"])
        // Five keys: the poll object and nothing else new, and still no
        // "reply_to_message_id": null.
        #expect(json.count == 5)
        #expect(poll?.count == 1)
    }

    @Test("read frame encodes per protocol.md")
    func encodeRead() throws {
        let json = try fields(of: .read(chatID: 42, lastReadMessageID: 1337))
        #expect(json["type"] as? String == "read")
        #expect(json["chat_id"] as? Int == 42)
        #expect(json["last_read_message_id"] as? Int == 1337)
        #expect(json.count == 3)
    }

    @Test("typing frame encodes per protocol.md")
    func encodeTyping() throws {
        let json = try fields(of: .typing(chatID: 42))
        #expect(json["type"] as? String == "typing")
        #expect(json["chat_id"] as? Int == 42)
        #expect(json.count == 2)
    }

    @Test("ping frame encodes per protocol.md")
    func encodePing() throws {
        let json = try fields(of: .ping)
        #expect(json["type"] as? String == "ping")
        #expect(json.count == 1)
    }

    // MARK: - Server → client

    private func decode(_ json: String) throws -> ServerFrame {
        try APICoding.decoder().decode(ServerFrame.self, from: Data(json.utf8))
    }

    @Test("ack decodes with the embedded message")
    func decodeAck() throws {
        let frame = try decode("""
        {"type": "ack", "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
         "message": {"id": 1338, "chat_id": 42, "sender_id": 7,
                     "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
                     "body": "Dinner at 7?", "created_at": "2026-08-19T17:03:12Z"}}
        """)
        guard case .ack(let clientMsgID, let message) = frame else {
            Issue.record("expected .ack, got \(frame)")
            return
        }
        #expect(clientMsgID == "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01")
        #expect(message.id == 1338)
        #expect(message.chatID == 42)
        #expect(message.senderID == 7)
        #expect(message.body == "Dinner at 7?")
        #expect(message.createdAt == ISO8601DateFormatter().date(from: "2026-08-19T17:03:12Z"))
    }

    @Test("message decodes")
    func decodeMessage() throws {
        let frame = try decode("""
        {"type": "message", "message": {"id": 1339, "chat_id": 42, "sender_id": 9,
         "client_msg_id": "0e7b2a01-1111-2222-3333-444455556666", "body": "Yes!",
         "created_at": "2026-08-19T17:04:00Z"}}
        """)
        guard case .message(let message) = frame else {
            Issue.record("expected .message, got \(frame)")
            return
        }
        #expect(message.id == 1339)
        #expect(message.senderID == 9)
    }

    @Test("read decodes")
    func decodeRead() throws {
        let frame = try decode(#"{"type": "read", "chat_id": 42, "user_id": 9, "last_read_message_id": 1338}"#)
        #expect(frame == .read(chatID: 42, userID: 9, lastReadMessageID: 1338))
    }

    @Test("typing decodes")
    func decodeTyping() throws {
        let frame = try decode(#"{"type": "typing", "chat_id": 42, "user_id": 9}"#)
        #expect(frame == .typing(chatID: 42, userID: 9))
    }

    @Test("member_joined decodes the reduced user object")
    func decodeMemberJoined() throws {
        let frame = try decode("""
        {"type": "member_joined", "family_id": 3,
         "user": {"id": 11, "username": "junior", "display_name": "Junior"}}
        """)
        guard case .memberJoined(let payload) = frame else {
            Issue.record("expected .memberJoined, got \(frame)")
            return
        }
        #expect(payload.familyID == 3)
        #expect(payload.user.id == 11)
        #expect(payload.user.username == "junior")
        #expect(payload.user.displayName == "Junior")
        // Absent on a server older than profile pictures.
        #expect(payload.user.avatarVersion == 0)
    }

    /// The frame is the one place a picture change reaches a client
    /// without a roster refresh (protocol: a frame carries at most the
    /// avatar_version, never the bytes).
    @Test("member_joined carries the avatar version")
    func decodeMemberJoinedWithAvatar() throws {
        let frame = try decode("""
        {"type": "member_joined", "family_id": 3,
         "user": {"id": 11, "username": "junior", "display_name": "Junior", "avatar_version": 7}}
        """)
        guard case .memberJoined(let payload) = frame else {
            Issue.record("expected .memberJoined, got \(frame)")
            return
        }
        #expect(payload.user.avatarVersion == 7)
    }

    @Test("member_left decodes")
    func decodeMemberLeft() throws {
        let frame = try decode(#"{"type": "member_left", "family_id": 3, "user_id": 11}"#)
        #expect(frame == .memberLeft(userID: 11))
    }

    @Test("reaction decodes the full-state payload")
    func decodeReaction() throws {
        let frame = try decode("""
        {"type": "reaction", "chat_id": 42, "message_id": 1338, "reaction_seq": 124,
         "reactions": [{"user_id": 9, "emoji": "❤️"}]}
        """)
        guard case .reaction(let payload) = frame else {
            Issue.record("expected .reaction, got \(frame)")
            return
        }
        #expect(payload.chatID == 42)
        #expect(payload.messageID == 1338)
        #expect(payload.reactionSeq == 124)
        #expect(payload.reactions == [ReactionDTO(userID: 9, emoji: "❤️")])
    }

    @Test("reaction decodes an emptied state (last reaction removed)")
    func decodeReactionCleared() throws {
        let frame = try decode(#"{"type": "reaction", "chat_id": 42, "message_id": 1338, "reaction_seq": 125, "reactions": []}"#)
        guard case .reaction(let payload) = frame else {
            Issue.record("expected .reaction, got \(frame)")
            return
        }
        #expect(payload.reactions.isEmpty)
        #expect(payload.reactionSeq == 125)
    }

    @Test("poll decodes the protocol.md literal")
    func decodePoll() throws {
        // Verbatim from protocol.md §Server → client.
        let frame = try decode("""
        {"type": "poll", "chat_id": 42, "message_id": 1340,
                         "poll": {"poll_seq": 89, "closed": false,
                                  "options": [{"id": 5, "text": "Pizza", "votes": [7, 9]},
                                              {"id": 6, "text": "Pasta", "votes": []}]}}
        """)
        guard case .poll(let payload) = frame else {
            Issue.record("expected .poll, got \(frame)")
            return
        }
        #expect(payload.chatID == 42)
        #expect(payload.messageID == 1340)
        #expect(payload.poll.pollSeq == 89)
        #expect(payload.poll.closed == false)
        #expect(payload.poll.options.count == 2)
        #expect(payload.poll.options[0] == PollOptionDTO(id: 5, text: "Pizza", votes: [7, 9]))
        // Empty is NOT absent: a Poll is complete state, so an option
        // nobody chose carries an empty list rather than no list.
        #expect(payload.poll.options[1] == PollOptionDTO(id: 6, text: "Pasta", votes: []))
        // One choice per member, derived from the lists — the wire cannot
        // carry a "did I vote" field, because one frame is serialised once
        // and sent to everybody.
        #expect(payload.poll.optionHeld(by: 9)?.id == 5)
        #expect(payload.poll.optionHeld(by: 11) == nil)
    }

    @Test("a closed poll decodes as closed")
    func decodeClosedPoll() throws {
        let frame = try decode("""
        {"type": "poll", "chat_id": 42, "message_id": 1340,
         "poll": {"poll_seq": 91, "closed": true,
                  "options": [{"id": 5, "text": "Pizza", "votes": [7]}]}}
        """)
        guard case .poll(let payload) = frame else {
            Issue.record("expected .poll, got \(frame)")
            return
        }
        #expect(payload.poll.closed)
        #expect(payload.poll.pollSeq == 91)
    }

    @Test("a message carrying a poll decodes it, and one without stays nil")
    func decodeMessageWithPoll() throws {
        let frame = try decode("""
        {"type": "message",
         "message": {"id": 1340, "chat_id": 42, "sender_id": 7, "client_msg_id": null,
                     "body": "Pizza or pasta?", "created_at": "2026-08-19T17:05:00Z",
                     "poll": {"poll_seq": 88, "closed": false,
                              "options": [{"id": 5, "text": "Pizza", "votes": []},
                                          {"id": 6, "text": "Pasta", "votes": []}]}}}
        """)
        guard case .message(let message) = frame else {
            Issue.record("expected .message, got \(frame)")
            return
        }
        // The question IS the body, so a client that knew nothing of polls
        // would still show the message and lose only the buttons.
        #expect(message.body == "Pizza or pasta?")
        #expect(message.poll?.pollSeq == 88)
        #expect(message.poll?.options.map(\.text) == ["Pizza", "Pasta"])

        let plain = try decode("""
        {"type": "message",
         "message": {"id": 1341, "chat_id": 42, "sender_id": 7, "client_msg_id": null,
                     "body": "Pasta then", "created_at": "2026-08-19T17:06:00Z"}}
        """)
        guard case .message(let ordinary) = plain else {
            Issue.record("expected .message, got \(plain)")
            return
        }
        #expect(ordinary.poll == nil)
    }

    @Test("a message with attachments decodes them, legacy attachment being the first")
    func decodeMessageWithAttachments() throws {
        // The doc's shape: "attachments" is 1–10 in the sender's order,
        // and "attachment" is its FIRST element, kept for clients that
        // predate plurality — a client that reads the array ignores it.
        let frame = try decode("""
        {"type": "message",
         "message": {"id": 1342, "chat_id": 42, "sender_id": 7, "client_msg_id": null,
                     "body": "", "created_at": "2026-08-19T17:07:00Z",
                     "attachments": [
                       {"id": 34, "kind": "photo", "mime": "image/jpeg", "size": 100, "has_preview": true},
                       {"id": 35, "kind": "photo", "mime": "image/jpeg", "size": 200, "has_preview": true},
                       {"id": 36, "kind": "video", "mime": "video/mp4", "size": 300, "has_preview": false}],
                     "attachment": {"id": 34, "kind": "photo", "mime": "image/jpeg", "size": 100, "has_preview": true}}}
        """)
        guard case .message(let message) = frame else {
            Issue.record("expected .message, got \(frame)")
            return
        }
        // The read rule: the array wins, in the sender's order.
        #expect(message.attachmentList.map(\.id) == [34, 35, 36])
        #expect(message.attachment?.id == 34)

        // A legacy server (or field) still reads as a one-element list.
        let legacy = try decode("""
        {"type": "message",
         "message": {"id": 1343, "chat_id": 42, "sender_id": 7, "client_msg_id": null,
                     "body": "", "created_at": "2026-08-19T17:08:00Z",
                     "attachment": {"id": 37, "kind": "photo", "mime": "image/jpeg", "size": 1, "has_preview": false}}}
        """)
        guard case .message(let single) = legacy else {
            Issue.record("expected .message, got \(legacy)")
            return
        }
        #expect(single.attachmentList.map(\.id) == [37])
    }

    @Test("pong decodes")
    func decodePong() throws {
        let frame = try decode(#"{"type": "pong"}"#)
        #expect(frame == .pong)
    }

    @Test("error decodes with and without client_msg_id")
    func decodeError() throws {
        let withID = try decode(#"{"type": "error", "code": "not_chat_member", "message": "nope", "client_msg_id": "8f14e45f"}"#)
        #expect(withID == .error(code: "not_chat_member", message: "nope", clientMsgID: "8f14e45f", callID: nil))

        let withoutID = try decode(#"{"type": "error", "code": "internal", "message": "boom"}"#)
        #expect(withoutID == .error(code: "internal", message: "boom", clientMsgID: nil, callID: nil))
    }

    @Test("unknown type NEVER throws — forward compatibility")
    func decodeUnknown() throws {
        // `call_offer` used to be the example here; it is a known frame now,
        // so the next thing that does not exist yet stands in for it.
        let frame = try decode(#"{"type": "video_offer", "chat_id": 42, "sdp": "v=0..."}"#)
        #expect(frame == .unknown(type: "video_offer"))
    }

    // MARK: - Voice calls (docs/protocol.md, "Voice calls")

    @Test("call_offer / call_answer / call_end encode per protocol.md")
    func encodeCallFrames() throws {
        let offer = try fields(of: .callOffer(callID: "6a1f0c3e-0000-4000-8000-000000000001", chatID: 42, sdp: "v=0\r\n"))
        #expect(offer["type"] as? String == "call_offer")
        #expect(offer["call_id"] as? String == "6a1f0c3e-0000-4000-8000-000000000001")
        #expect(offer["chat_id"] as? Int == 42)
        #expect(offer["sdp"] as? String == "v=0\r\n")
        #expect(offer.count == 4)

        let answer = try fields(of: .callAnswer(callID: "6a1f0c3e-0000-4000-8000-000000000001", sdp: "v=0\r\n"))
        #expect(answer["type"] as? String == "call_answer")
        #expect(answer["sdp"] as? String == "v=0\r\n")
        #expect(answer.count == 3)

        let end = try fields(of: .callEnd(callID: "6a1f0c3e-0000-4000-8000-000000000001", reason: "hangup"))
        #expect(end["type"] as? String == "call_end")
        #expect(end["reason"] as? String == "hangup")
        #expect(end.count == 3)
    }

    @Test("call_ice encodes a candidate, with sdp_mid / sdp_mline_index absent rather than null")
    func encodeCallIce() throws {
        let full = try fields(of: .callIce(
            callID: "6a1f0c3e-0000-4000-8000-000000000001",
            candidate: IceCandidatePayload(candidate: "candidate:1 1 udp 1 1.2.3.4 5 typ host", sdpMid: "0", sdpMLineIndex: 0)))
        #expect(full["type"] as? String == "call_ice")
        let candidate = full["candidate"] as? [String: Any]
        #expect(candidate?["candidate"] as? String == "candidate:1 1 udp 1 1.2.3.4 5 typ host")
        #expect(candidate?["sdp_mid"] as? String == "0")
        #expect(candidate?["sdp_mline_index"] as? Int == 0)
        #expect(candidate?.count == 3)

        let bare = try fields(of: .callIce(
            callID: "6a1f0c3e-0000-4000-8000-000000000001",
            candidate: IceCandidatePayload(candidate: "candidate:…", sdpMid: nil, sdpMLineIndex: nil)))
        let bareCandidate = bare["candidate"] as? [String: Any]
        #expect(bareCandidate?.count == 1, "absent, not null: \(String(describing: bareCandidate))")
    }

    @Test("call_offer decodes the protocol example")
    func decodeCallOffer() throws {
        let frame = try decode(#"{"type": "call_offer", "call_id": "6a1f0c3e-0000-4000-8000-000000000001", "chat_id": 42, "from_user_id": 7, "sdp": "v=0\r\n"}"#)
        #expect(frame == .callOffer(CallOfferPayload(
            callID: "6a1f0c3e-0000-4000-8000-000000000001", chatID: 42, fromUserID: 7, sdp: "v=0\r\n")))
    }

    @Test("call_ringing / call_answer / call_end decode")
    func decodeCallLifecycle() throws {
        #expect(try decode(#"{"type": "call_ringing", "call_id": "abc"}"#) == .callRinging(callID: "abc"))
        #expect(try decode(#"{"type": "call_answer", "call_id": "abc", "sdp": "v=0"}"#) == .callAnswer(callID: "abc", sdp: "v=0"))
        #expect(try decode(#"{"type": "call_end", "call_id": "abc", "reason": "answered_elsewhere"}"#)
                == .callEnd(callID: "abc", reason: "answered_elsewhere"))
    }

    @Test("call_ice decodes with and without the optional candidate fields")
    func decodeCallIce() throws {
        let full = try decode(#"{"type": "call_ice", "call_id": "abc", "candidate": {"candidate": "candidate:…", "sdp_mid": "0", "sdp_mline_index": 0}}"#)
        #expect(full == .callIce(callID: "abc", candidate: IceCandidatePayload(candidate: "candidate:…", sdpMid: "0", sdpMLineIndex: 0)))
        let bare = try decode(#"{"type": "call_ice", "call_id": "abc", "candidate": {"candidate": "candidate:…"}}"#)
        #expect(bare == .callIce(callID: "abc", candidate: IceCandidatePayload(candidate: "candidate:…", sdpMid: nil, sdpMLineIndex: nil)))
    }

    @Test("an error answering a call frame carries call_id and no client_msg_id")
    func decodeCallError() throws {
        let frame = try decode(#"{"type": "error", "code": "peer_busy", "message": "…", "call_id": "abc"}"#)
        #expect(frame == .error(code: "peer_busy", message: "…", clientMsgID: nil, callID: "abc"))
    }
}
