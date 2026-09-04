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
//  `.unknown(type:)` and NEVER throws. This is how the voice-call
//  signalling frames (`call_offer`, …) were added server-side without
//  breaking deployed v1 clients — those logged and skipped them. Only a
//  frame that is missing "type" entirely, or malforms a *known* type's
//  payload, throws; the socket's receive loop logs and continues on those
//  too.
//

import Foundation

// MARK: - Client → server

nonisolated enum ClientFrame: Encodable, Equatable, Sendable {
    /// `pollOptions` is what makes the message a poll (docs/protocol.md,
    /// "Polls"): the body is then the QUESTION and the options ride
    /// beside it as `{"poll": {"options": [...]}}`. nil on every ordinary
    /// message, where the key is absent rather than null.
    ///
    /// No default value — Swift does not permit one on an enum case's
    /// associated value — so every construction site spells it out.
    case send(
        chatID: Int64,
        clientMsgID: String,
        body: String,
        replyToMessageID: Int64?,
        /// The attachments this message claims, in the sender's order —
        /// encoded as `attachment_ids`. The legacy `attachment_id` is the
        /// one-element spelling of the same thing and this client no
        /// longer sends it; the server accepts the array.
        attachmentIDs: [Int64]?,
        pollOptions: [String]?)
    case read(chatID: Int64, lastReadMessageID: Int64)
    case typing(chatID: Int64)
    case ping
    /// Voice-call signalling (docs/protocol.md, "Voice calls"). `callID`
    /// is a UUID this client minted, exactly as `clientMsgID` is: it is
    /// what lets every reply be correlated with the call it answers, and
    /// what makes the server's record exactly-once.
    ///
    /// `video` is what makes the call a VIDEO call (docs/protocol.md,
    /// "Video") — decided here, fixed for the call's life. Encoded ONLY
    /// when true: a voice offer must stay byte-identical to what every
    /// deployed client has always sent, absent-not-false like every other
    /// optional field on this wire.
    case callOffer(callID: String, chatID: Int64, sdp: String, video: Bool)
    case callAnswer(callID: String, sdp: String)
    case callIce(callID: String, candidate: IceCandidatePayload)
    /// `reason` is one of `hangup`, `decline`, `cancel`, `failed`.
    case callEnd(callID: String, reason: String)

    private enum CodingKeys: String, CodingKey {
        case type
        case chatID = "chat_id"
        case clientMsgID = "client_msg_id"
        case body
        case replyToMessageID = "reply_to_message_id"
        case attachmentIDs = "attachment_ids"
        case lastReadMessageID = "last_read_message_id"
        case poll
        case callID = "call_id"
        case sdp
        case candidate
        case reason
        case video
    }

    /// The keys of the nested `poll` object on a `send`. Its only member
    /// is `options`: ids, votes and the sequence are the server's, and the
    /// question is the body.
    private enum NewPollKeys: String, CodingKey {
        case options
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .send(let chatID, let clientMsgID, let body, let replyToMessageID, let attachmentIDs, let pollOptions):
            try container.encode("send", forKey: .type)
            try container.encode(chatID, forKey: .chatID)
            try container.encode(clientMsgID, forKey: .clientMsgID)
            try container.encode(body, forKey: .body)
            // encodeIfPresent, not encode: an ordinary message must not
            // carry "reply_to_message_id": null — the protocol writes the
            // field as absent.
            try container.encodeIfPresent(replyToMessageID, forKey: .replyToMessageID)
            try container.encodeIfPresent(attachmentIDs, forKey: .attachmentIDs)
            if let pollOptions {
                var poll = container.nestedContainer(keyedBy: NewPollKeys.self, forKey: .poll)
                try poll.encode(pollOptions, forKey: .options)
            }
        case .read(let chatID, let lastReadMessageID):
            try container.encode("read", forKey: .type)
            try container.encode(chatID, forKey: .chatID)
            try container.encode(lastReadMessageID, forKey: .lastReadMessageID)
        case .typing(let chatID):
            try container.encode("typing", forKey: .type)
            try container.encode(chatID, forKey: .chatID)
        case .ping:
            try container.encode("ping", forKey: .type)
        case .callOffer(let callID, let chatID, let sdp, let video):
            try container.encode("call_offer", forKey: .type)
            try container.encode(callID, forKey: .callID)
            try container.encode(chatID, forKey: .chatID)
            try container.encode(sdp, forKey: .sdp)
            // Only when true — see the case's doc comment. `encode(false)`
            // here would change every voice offer's bytes.
            if video { try container.encode(true, forKey: .video) }
        case .callAnswer(let callID, let sdp):
            try container.encode("call_answer", forKey: .type)
            try container.encode(callID, forKey: .callID)
            try container.encode(sdp, forKey: .sdp)
        case .callIce(let callID, let candidate):
            try container.encode("call_ice", forKey: .type)
            try container.encode(callID, forKey: .callID)
            try container.encode(candidate, forKey: .candidate)
        case .callEnd(let callID, let reason):
            try container.encode("call_end", forKey: .type)
            try container.encode(callID, forKey: .callID)
            try container.encode(reason, forKey: .reason)
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

/// Payload of `member_deleted`: the WHOLE tombstone `Member`, because
/// that is exactly what a client has to overwrite — `deleted: true`, the
/// placeholder display name, `avatar_version: 0` and no birthday.
///
/// This is the one frame in the protocol whose job is to WIPE stored
/// fields, so the coordinator applies it by writing the tombstone
/// deliberately rather than through the ordinary member upsert, which
/// everywhere else must never let an absent field clear a stored one
/// (docs/protocol.md, "Server → client").
/// `familyID` is OPTIONAL, and that is load-bearing: the frame reaches
/// every member of any chat the account was part of, so a direct-chat peer
/// in another family — or in none — gets it too, and the protocol says
/// outright that `family_id` is ABSENT when the deleted account belonged
/// to no family. Decoding it as required threw on exactly those frames,
/// and an undecodable frame is skipped: the peer never learned, and went
/// on drawing the old name against a chat about to vanish. A client keys
/// this frame on the `member`, never on the family — which is why nothing
/// below reads the id at all.
nonisolated struct MemberDeletedPayload: Decodable, Equatable, Sendable {
    let familyID: Int64?
    let member: MemberDTO
    enum CodingKeys: String, CodingKey {
        case familyID = "family_id"
        case member
    }

    /// Hand-written for the reason every other optional on this wire is:
    /// `decodeIfPresent`, so an absent key is absence rather than a
    /// throw. The synthesised initialiser would do the same for an
    /// Optional — this spells it out so it cannot be lost to a later
    /// tidy-up that makes the field non-optional again.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        familyID = try container.decodeIfPresent(Int64.self, forKey: .familyID)
        member = try container.decode(MemberDTO.self, forKey: .member)
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

/// Payload of `poll`: one message's FULL current poll state — never a
/// delta — so re-delivery is idempotent and ordering races resolve locally
/// under the poll_seq guard (the coordinator applies it only when
/// `poll.pollSeq` exceeds what the message row already holds).
///
/// It never notifies and never counts as unread: a vote is not a message.
nonisolated struct PollPayload: Decodable, Equatable, Sendable {
    let chatID: Int64
    let messageID: Int64
    let poll: PollDTO
    enum CodingKeys: String, CodingKey {
        case chatID = "chat_id"
        case messageID = "message_id"
        case poll
    }
}

/// One ICE candidate, in both directions (docs/protocol.md, "Voice
/// calls"). `sdpMid` and `sdpMLineIndex` are each OPTIONAL: a WebRTC stack
/// supplies one, the other, or both, and the receiving stack accepts
/// whichever it was given — so an absent one is absent on the wire, never
/// null, exactly like every other optional field here.
nonisolated struct IceCandidatePayload: Codable, Equatable, Sendable {
    let candidate: String
    let sdpMid: String?
    let sdpMLineIndex: Int32?

    enum CodingKeys: String, CodingKey {
        case candidate
        case sdpMid = "sdp_mid"
        case sdpMLineIndex = "sdp_mline_index"
    }

    init(candidate: String, sdpMid: String?, sdpMLineIndex: Int32?) {
        self.candidate = candidate
        self.sdpMid = sdpMid
        self.sdpMLineIndex = sdpMLineIndex
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        candidate = try container.decode(String.self, forKey: .candidate)
        sdpMid = try container.decodeIfPresent(String.self, forKey: .sdpMid)
        sdpMLineIndex = try container.decodeIfPresent(Int32.self, forKey: .sdpMLineIndex)
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(candidate, forKey: .candidate)
        try container.encodeIfPresent(sdpMid, forKey: .sdpMid)
        try container.encodeIfPresent(sdpMLineIndex, forKey: .sdpMLineIndex)
    }
}

/// Payload of an inbound `call_offer`: somebody is ringing this account.
/// Reaches EVERY connection the callee has, and is replayed to a
/// connection that registers while the call is still ringing — so a
/// device that already holds `callID` treats a second copy as the
/// duplicate it is (docs/protocol.md, "Late arrivals").
nonisolated struct CallOfferPayload: Decodable, Equatable, Sendable {
    let callID: String
    let chatID: Int64
    let fromUserID: Int64
    let sdp: String
    /// True when this is a VIDEO call (docs/protocol.md, "Video"). The
    /// flag is fixed at placement; absent on the wire means voice, so it
    /// decodes with `decodeIfPresent` — a required decode would refuse
    /// every voice offer from every deployed server.
    let video: Bool
    enum CodingKeys: String, CodingKey {
        case callID = "call_id"
        case chatID = "chat_id"
        case fromUserID = "from_user_id"
        case sdp
        case video
    }

    init(callID: String, chatID: Int64, fromUserID: Int64, sdp: String, video: Bool = false) {
        self.callID = callID
        self.chatID = chatID
        self.fromUserID = fromUserID
        self.sdp = sdp
        self.video = video
    }

    /// Hand-written for the reason every defaulted field on this wire is:
    /// a property default is not a decoding fallback, and a voice offer
    /// carries no "video" key at all.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        callID = try container.decode(String.self, forKey: .callID)
        chatID = try container.decode(Int64.self, forKey: .chatID)
        fromUserID = try container.decode(Int64.self, forKey: .fromUserID)
        sdp = try container.decode(String.self, forKey: .sdp)
        video = try container.decodeIfPresent(Bool.self, forKey: .video) ?? false
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
    /// One fragment of the assistant's reply, as it is generated.
    ///
    /// COSMETIC: the row named by `messageID` is the truth, and its final
    /// body arrives as `.messageEdited` whether or not any of these were
    /// seen (docs/protocol.md, "The assistant"). Missing them costs a
    /// live-typing effect, never the answer.
    case aiDelta(chatID: Int64, messageID: Int64, text: String)
    /// The reply stopped early. Whatever arrived is already on the row.
    case aiError(chatID: Int64, messageID: Int64)
    case read(chatID: Int64, userID: Int64, lastReadMessageID: Int64)
    case typing(chatID: Int64, userID: Int64)
    case memberJoined(MemberJoinedPayload)
    case memberLeft(userID: Int64)
    /// A member deleted their account. Carries the tombstone to write; a
    /// `member_left` for the same person is sent alongside it.
    case memberDeleted(MemberDeletedPayload)
    /// The family has a new owner — sent to every member when an owner
    /// deletes their account and ownership passes on. A client that
    /// receives it for ITSELF gains the owner-only screens immediately
    /// rather than at its next `GET /me`.
    case familyOwner(familyID: Int64, userID: Int64)
    /// A block this caller set or cleared, delivered to EVERY CONNECTION OF
    /// THIS USER AND TO NOBODY ELSE — nothing about a block ever reaches
    /// the person blocked (protocol.md, "Blocking a member").
    ///
    /// `blocked` carries full current state rather than an event, so an
    /// unblock is this same frame with `false` and a client applies it as a
    /// state-set. No `familyID`: a block is a pair, not a membership, and
    /// it outlives either of them leaving.
    case memberBlocked(userID: Int64, blocked: Bool)
    case reaction(ReactionPayload)
    /// A poll's full current state after a vote, a retraction or a close.
    /// Dispatched exactly like `reaction`, under its own seq guard.
    case poll(PollPayload)
    /// Voice-call signalling (docs/protocol.md, "Voice calls"). Every one
    /// of these carries the `callID`, and CallManager applies a frame only
    /// to a call it holds — a frame for any other call is ignored in
    /// silence, which is the one rule that makes the multi-device story
    /// work without the server tracking which device is doing what.
    case callOffer(CallOfferPayload)
    case callRinging(callID: String)
    case callAnswer(callID: String, sdp: String)
    case callIce(callID: String, candidate: IceCandidatePayload)
    /// `reason`: `hangup`, `decline`, `cancel`, `timeout`, `failed` or
    /// `answered_elsewhere`.
    case callEnd(callID: String, reason: String)
    case pong
    /// `clientMsgID` is present when the error answers a `send` frame;
    /// `callID` when it answers a call frame. Never both.
    case error(code: String, message: String, clientMsgID: String?, callID: String?)
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
        case member
        case familyID = "family_id"
        case blocked
        case messageID = "message_id"
        case reactionSeq = "reaction_seq"
        case reactions
        case note
        case text
        case poll
        case callID = "call_id"
        case sdp
        case candidate
        case reason
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
        case "ai_delta":
            self = .aiDelta(
                chatID: try container.decode(Int64.self, forKey: .chatID),
                messageID: try container.decode(Int64.self, forKey: .messageID),
                text: try container.decode(String.self, forKey: .text))
        case "ai_error":
            self = .aiError(
                chatID: try container.decode(Int64.self, forKey: .chatID),
                messageID: try container.decode(Int64.self, forKey: .messageID))
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
        case "member_deleted":
            // Re-decode from the top so MemberDeletedPayload owns its keys.
            self = .memberDeleted(try MemberDeletedPayload(from: decoder))
        case "family_owner":
            self = .familyOwner(
                familyID: try container.decode(Int64.self, forKey: .familyID),
                userID: try container.decode(Int64.self, forKey: .userID))
        case "member_blocked":
            self = .memberBlocked(
                userID: try container.decode(Int64.self, forKey: .userID),
                blocked: try container.decode(Bool.self, forKey: .blocked))
        case "reaction":
            // Re-decode from the top so ReactionPayload owns its keys.
            self = .reaction(try ReactionPayload(from: decoder))
        case "poll":
            // Re-decode from the top so PollPayload owns its keys.
            self = .poll(try PollPayload(from: decoder))
        case "call_offer":
            // Re-decode from the top so CallOfferPayload owns its keys.
            self = .callOffer(try CallOfferPayload(from: decoder))
        case "call_ringing":
            self = .callRinging(callID: try container.decode(String.self, forKey: .callID))
        case "call_answer":
            self = .callAnswer(
                callID: try container.decode(String.self, forKey: .callID),
                sdp: try container.decode(String.self, forKey: .sdp))
        case "call_ice":
            self = .callIce(
                callID: try container.decode(String.self, forKey: .callID),
                candidate: try container.decode(IceCandidatePayload.self, forKey: .candidate))
        case "call_end":
            self = .callEnd(
                callID: try container.decode(String.self, forKey: .callID),
                reason: try container.decode(String.self, forKey: .reason))
        case "pong":
            self = .pong
        case "error":
            self = .error(
                code: try container.decode(String.self, forKey: .code),
                message: (try container.decodeIfPresent(String.self, forKey: .message)) ?? "",
                clientMsgID: try container.decodeIfPresent(String.self, forKey: .clientMsgID),
                callID: try container.decodeIfPresent(String.self, forKey: .callID))
        default:
            self = .unknown(type: type)
        }
    }
}
