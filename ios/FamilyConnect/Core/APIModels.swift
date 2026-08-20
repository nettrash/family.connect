//
//  APIModels.swift
//  FamilyConnect
//
//  Codable DTOs matching docs/protocol.md exactly. Field names on the wire
//  are snake_case; we spell out explicit CodingKeys on every type rather
//  than flipping on `.convertFromSnakeCase`, because the protocol document
//  is authoritative byte-for-byte and an explicit key table is the only
//  form a reviewer can diff against it. Unknown fields are ignored by
//  Codable's default behaviour, which is the compatibility rule the
//  protocol demands of clients.
//
//  Dates: the server emits RFC 3339 UTC with a trailing "Z" and no
//  fractional seconds (e.g. "2026-08-19T17:03:12Z"), which is precisely
//  the shape Foundation's plain `.iso8601` date strategy parses — so the
//  shared decoder below uses it directly. If the server ever grew
//  fractional seconds this would need `.custom` with two formatters; it
//  deliberately doesn't until the protocol does.
//

import Foundation

// MARK: - Shared coders

/// Factories for the one JSON dialect this app speaks. Functions rather
/// than shared instances because JSONDecoder/JSONEncoder are not Sendable
/// and both the MainActor coordinator and the socket/API actors need one.
nonisolated enum APICoding {
    static func decoder() -> JSONDecoder {
        let decoder = JSONDecoder()
        // Server dates are RFC 3339 "Z" with no fractional seconds — see
        // the file header for why plain .iso8601 is exactly right.
        decoder.dateDecodingStrategy = .iso8601
        return decoder
    }

    static func encoder() -> JSONEncoder {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        return encoder
    }
}

// MARK: - Objects (protocol.md §Objects)

nonisolated struct UserDTO: Codable, Equatable, Sendable {
    let id: Int64
    let username: String
    let displayName: String
    let createdAt: Date?

    enum CodingKeys: String, CodingKey {
        case id
        case username
        case displayName = "display_name"
        case createdAt = "created_at"
    }
}

nonisolated struct MemberDTO: Codable, Equatable, Sendable {
    let id: Int64
    let username: String
    let displayName: String
    /// "owner" | "member"
    let role: String

    enum CodingKeys: String, CodingKey {
        case id
        case username
        case displayName = "display_name"
        case role
    }
}

nonisolated struct FamilyDTO: Codable, Equatable, Sendable {
    let id: Int64
    let name: String
    /// "open" | "approval"
    let joinPolicy: String
    let createdAt: Date?
    /// Present when (and only when) the caller is the owner.
    let inviteCode: String?

    enum CodingKeys: String, CodingKey {
        case id
        case name
        case joinPolicy = "join_policy"
        case createdAt = "created_at"
        case inviteCode = "invite_code"
    }
}

nonisolated struct PendingJoinRequestDTO: Codable, Equatable, Sendable {
    let familyID: Int64
    let familyName: String
    let createdAt: Date?

    enum CodingKeys: String, CodingKey {
        case familyID = "family_id"
        case familyName = "family_name"
        case createdAt = "created_at"
    }
}

nonisolated struct JoinRequestDTO: Codable, Equatable, Sendable, Identifiable {
    let id: Int64
    let user: UserDTO
    let createdAt: Date?

    enum CodingKeys: String, CodingKey {
        case id
        case user
        case createdAt = "created_at"
    }
}

nonisolated struct ChatDTO: Codable, Equatable, Sendable {
    let id: Int64
    /// "family" | "direct"
    let kind: String
    let title: String
    let peerUserID: Int64?

    enum CodingKeys: String, CodingKey {
        case id
        case kind
        case title
        case peerUserID = "peer_user_id"
    }
}

nonisolated struct ReactionDTO: Codable, Equatable, Sendable {
    let userID: Int64
    let emoji: String

    enum CodingKeys: String, CodingKey {
        case userID = "user_id"
        case emoji
    }
}

nonisolated struct MessageDTO: Codable, Equatable, Sendable {
    let id: Int64
    let chatID: Int64
    let senderID: Int64
    let clientMsgID: String?
    let body: String
    let createdAt: Date
    /// Both reaction fields are present when (and only when) the message
    /// has ever been reacted to — after clearing, `reactions` is [] with
    /// the seq still present. Optional matters: ABSENCE means "no data"
    /// and must never wipe locally-held reaction state.
    let reactions: [ReactionDTO]?
    let reactionSeq: Int64?

    enum CodingKeys: String, CodingKey {
        case id
        case chatID = "chat_id"
        case senderID = "sender_id"
        case clientMsgID = "client_msg_id"
        case body
        case createdAt = "created_at"
        case reactions
        case reactionSeq = "reaction_seq"
    }

    /// Explicit memberwise init so the reaction fields default to absent
    /// — pre-reaction construction sites (tests included) stay valid.
    init(
        id: Int64,
        chatID: Int64,
        senderID: Int64,
        clientMsgID: String?,
        body: String,
        createdAt: Date,
        reactions: [ReactionDTO]? = nil,
        reactionSeq: Int64? = nil
    ) {
        self.id = id
        self.chatID = chatID
        self.senderID = senderID
        self.clientMsgID = clientMsgID
        self.body = body
        self.createdAt = createdAt
        self.reactions = reactions
        self.reactionSeq = reactionSeq
    }
}

// MARK: - Endpoint envelopes

nonisolated struct AuthResponse: Codable, Equatable, Sendable {
    let token: String
    let user: UserDTO
}

nonisolated struct MeResponse: Codable, Equatable, Sendable {
    let user: UserDTO
    let family: FamilyDTO?
    /// "owner" | "member" | nil
    let role: String?
    let pendingJoinRequest: PendingJoinRequestDTO?

    enum CodingKeys: String, CodingKey {
        case user
        case family
        case role
        case pendingJoinRequest = "pending_join_request"
    }
}

nonisolated struct FamilyResponse: Codable, Equatable, Sendable {
    let family: FamilyDTO
}

nonisolated struct JoinResponse: Codable, Equatable, Sendable {
    /// "joined" | "pending"
    let status: String
}

nonisolated struct FamilyMineResponse: Codable, Equatable, Sendable {
    let family: FamilyDTO
    let members: [MemberDTO]
}

nonisolated struct InviteCodeResponse: Codable, Equatable, Sendable {
    let inviteCode: String

    enum CodingKeys: String, CodingKey {
        case inviteCode = "invite_code"
    }
}

nonisolated struct JoinRequestsResponse: Codable, Equatable, Sendable {
    let requests: [JoinRequestDTO]
}

nonisolated struct MemberResponse: Codable, Equatable, Sendable {
    let member: MemberDTO
}

nonisolated struct ChatListItemDTO: Codable, Equatable, Sendable {
    let chat: ChatDTO
    let lastMessage: MessageDTO?
    let unreadCount: Int
    /// Max reaction_seq over the chat's messages; omitted by the server
    /// while no message in the chat has ever been reacted to (treat as 0).
    let maxReactionSeq: Int64?

    enum CodingKeys: String, CodingKey {
        case chat
        case lastMessage = "last_message"
        case unreadCount = "unread_count"
        case maxReactionSeq = "max_reaction_seq"
    }

    /// Explicit memberwise init so `maxReactionSeq` defaults to absent —
    /// pre-reaction construction sites stay valid.
    init(chat: ChatDTO, lastMessage: MessageDTO?, unreadCount: Int, maxReactionSeq: Int64? = nil) {
        self.chat = chat
        self.lastMessage = lastMessage
        self.unreadCount = unreadCount
        self.maxReactionSeq = maxReactionSeq
    }
}

nonisolated struct ChatsResponse: Codable, Equatable, Sendable {
    let chats: [ChatListItemDTO]
}

nonisolated struct ChatResponse: Codable, Equatable, Sendable {
    let chat: ChatDTO
}

nonisolated struct MessagesResponse: Codable, Equatable, Sendable {
    let messages: [MessageDTO]
}

nonisolated struct MessageResponse: Codable, Equatable, Sendable {
    let message: MessageDTO
}

/// One message's full reaction state: the body of the PUT/DELETE
/// reaction endpoints and the page entry of GET /chats/{id}/reactions.
nonisolated struct ReactionStateDTO: Codable, Equatable, Sendable {
    let messageID: Int64
    let reactionSeq: Int64
    let reactions: [ReactionDTO]

    enum CodingKeys: String, CodingKey {
        case messageID = "message_id"
        case reactionSeq = "reaction_seq"
        case reactions
    }
}

nonisolated struct MessageReactionsResponse: Codable, Equatable, Sendable {
    let messageReactions: [ReactionStateDTO]

    enum CodingKeys: String, CodingKey {
        case messageReactions = "message_reactions"
    }
}

nonisolated struct DeviceResponse: Codable, Equatable, Sendable {
    let deviceID: Int64

    enum CodingKeys: String, CodingKey {
        case deviceID = "device_id"
    }
}

// MARK: - Error shape (protocol.md §Error shape)

nonisolated struct APIErrorBody: Codable, Equatable, Sendable {
    struct Payload: Codable, Equatable, Sendable {
        let code: String
        let message: String?
    }
    let error: Payload
}
