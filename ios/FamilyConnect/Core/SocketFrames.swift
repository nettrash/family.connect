//
//  SocketFrames.swift
//  FamilyConnect
//
//  The WebSocket wire protocol (docs/protocol.md §WebSocket protocol):
//  JSON text frames tagged by a "type" string. Client → server frames are
//  Encodable only, server → client frames Decodable only — neither
//  direction ever round-trips, so symmetric Codable would just invite
//  encoding a frame the server never defined.
//
//  COMPATIBILITY RULE, load-bearing: an unknown server "type" decodes to
//  `.unknown(type:)` and NEVER throws. This is how voice/video signaling
//  frames (`call_offer`, …) get added server-side later without breaking
//  deployed v1 clients — the coordinator logs and skips them. Only a frame
//  that is missing "type" entirely, or malforms a *known* type's payload,
//  throws; the socket's receive loop logs and continues on those too.
//

import Foundation

// MARK: - Client → server

nonisolated enum ClientFrame: Encodable, Equatable, Sendable {
    case send(chatID: Int64, clientMsgID: String, body: String, replyToMessageID: Int64?, attachmentID: Int64?)
    case read(chatID: Int64, lastReadMessageID: Int64)
    case typing(chatID: Int64)
    case ping

    private enum CodingKeys: String, CodingKey {
        case type
        case chatID = "chat_id"
        case clientMsgID = "client_msg_id"
        case body
        case replyToMessageID = "reply_to_message_id"
        case attachmentID = "attachment_id"
        case lastReadMessageID = "last_read_message_id"
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .send(let chatID, let clientMsgID, let body, let replyToMessageID, let attachmentID):
            try container.encode("send", forKey: .type)
            try container.encode(chatID, forKey: .chatID)
            try container.encode(clientMsgID, forKey: .clientMsgID)
            try container.encode(body, forKey: .body)
            // encodeIfPresent, not encode: an ordinary message must not
            // carry "reply_to_message_id": null — the protocol writes the
            // field as absent.
            try container.encodeIfPresent(replyToMessageID, forKey: .replyToMessageID)
            try container.encodeIfPresent(attachmentID, forKey: .attachmentID)
        case .read(let chatID, let lastReadMessageID):
            try container.encode("read", forKey: .type)
            try container.encode(chatID, forKey: .chatID)
            try container.encode(lastReadMessageID, forKey: .lastReadMessageID)
        case .typing(let chatID):
            try container.encode("typing", forKey: .type)
            try container.encode(chatID, forKey: .chatID)
        case .ping:
            try container.encode("ping", forKey: .type)
        }
    }

    /// The frame as the JSON text message the socket sends.
    func encodedString() throws -> String {
        String(decoding: try APICoding.encoder().encode(self), as: UTF8.self)
    }
}

// MARK: - Server → client

/// Payload of `member_joined`. The embedded user carries id/username/
/// display_name only (no created_at, no role) — a deliberate subset, so it
/// gets its own type rather than a mostly-nil UserDTO.
nonisolated struct MemberJoinedPayload: Decodable, Equatable, Sendable {
    struct JoinedUser: Decodable, Equatable, Sendable {
        let id: Int64
        let username: String
        let displayName: String
        /// `0` = no profile picture. Defaulted so a server older than the
        /// avatars release still decodes.
        var avatarVersion: Int64 = 0
        enum CodingKeys: String, CodingKey {
            case id
            case username
            case displayName = "display_name"
            case avatarVersion = "avatar_version"
        }

        /// Hand-written for the same reason as UserDTO's: a property
        /// default is not a decoding fallback, and older servers do not
        /// send this field.
        init(from decoder: Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            id = try container.decode(Int64.self, forKey: .id)
            username = try container.decode(String.self, forKey: .username)
            displayName = try container.decode(String.self, forKey: .displayName)
            avatarVersion = try container.decodeIfPresent(Int64.self, forKey: .avatarVersion) ?? 0
        }
    }
    let familyID: Int64
    let user: JoinedUser
    enum CodingKeys: String, CodingKey {
        case familyID = "family_id"
        case user
    }
}

/// Payload of `reaction`: one message's FULL current reaction state —
/// never a delta — so re-delivery is idempotent and ordering races
/// resolve locally under the reaction_seq guard (the coordinator applies
/// it only when reaction_seq exceeds what the message row already holds).
nonisolated struct ReactionPayload: Decodable, Equatable, Sendable {
    let chatID: Int64
    let messageID: Int64
    let reactionSeq: Int64
    let reactions: [ReactionDTO]
    enum CodingKeys: String, CodingKey {
        case chatID = "chat_id"
        case messageID = "message_id"
        case reactionSeq = "reaction_seq"
        case reactions
    }
}

nonisolated enum ServerFrame: Decodable, Equatable, Sendable {
    case ack(clientMsgID: String, message: MessageDTO)
    case message(MessageDTO)
    /// An edit of an existing message. A SEPARATE case from `.message`
    /// because that one bumps unread counts and raises notifications, and
    /// an edit must do neither (docs/protocol.md, "Editing").
    case messageEdited(MessageDTO)
    /// One board note in whatever state it now has — created, edited,
    /// moved, or a tombstone. Never notifies, never counts as unread.
    case boardNote(NoteDTO)
    case read(chatID: Int64, userID: Int64, lastReadMessageID: Int64)
    case typing(chatID: Int64, userID: Int64)
    case memberJoined(MemberJoinedPayload)
    case memberLeft(userID: Int64)
    case reaction(ReactionPayload)
    case pong
    /// `clientMsgID` is present when the error answers a `send` frame.
    case error(code: String, message: String, clientMsgID: String?)
    /// Any type this client does not know. See the file header.
    case unknown(type: String)

    private enum CodingKeys: String, CodingKey {
        case type
        case clientMsgID = "client_msg_id"
        case message
        case chatID = "chat_id"
        case userID = "user_id"
        case lastReadMessageID = "last_read_message_id"
        case code
        case user
        case familyID = "family_id"
        case messageID = "message_id"
        case reactionSeq = "reaction_seq"
        case reactions
        case note
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let type = try container.decode(String.self, forKey: .type)
        switch type {
        case "ack":
            self = .ack(
                clientMsgID: try container.decode(String.self, forKey: .clientMsgID),
                message: try container.decode(MessageDTO.self, forKey: .message))
        case "message":
            self = .message(try container.decode(MessageDTO.self, forKey: .message))
        case "message_edited":
            self = .messageEdited(try container.decode(MessageDTO.self, forKey: .message))
        case "board_note":
            self = .boardNote(try container.decode(NoteDTO.self, forKey: .note))
        case "read":
            self = .read(
                chatID: try container.decode(Int64.self, forKey: .chatID),
                userID: try container.decode(Int64.self, forKey: .userID),
                lastReadMessageID: try container.decode(Int64.self, forKey: .lastReadMessageID))
        case "typing":
            self = .typing(
                chatID: try container.decode(Int64.self, forKey: .chatID),
                userID: try container.decode(Int64.self, forKey: .userID))
        case "member_joined":
            // Re-decode from the top so MemberJoinedPayload owns its keys.
            self = .memberJoined(try MemberJoinedPayload(from: decoder))
        case "member_left":
            self = .memberLeft(userID: try container.decode(Int64.self, forKey: .userID))
        case "reaction":
            // Re-decode from the top so ReactionPayload owns its keys.
            self = .reaction(try ReactionPayload(from: decoder))
        case "pong":
            self = .pong
        case "error":
            self = .error(
                code: try container.decode(String.self, forKey: .code),
                message: (try container.decodeIfPresent(String.self, forKey: .message)) ?? "",
                clientMsgID: try container.decodeIfPresent(String.self, forKey: .clientMsgID))
        default:
            self = .unknown(type: type)
        }
    }
}
