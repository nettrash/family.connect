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

/// A day and a month, with no year — deliberately, so nobody has to
/// publish their age to be wished a happy birthday (protocol.md,
/// "Birthdays").
///
/// Two integers and NOT a `Date`: the shared decoder above is `.iso8601`,
/// and there is no year here for a calendar to anchor to. 29 February is
/// a perfectly good birthday for the same reason.
///
/// One object rather than two sibling fields on `User`/`Member`, because
/// the two halves are a single fact: a birthday is either there or it is
/// not, and "month but no day" is not a state anything should handle.
nonisolated struct BirthdayDTO: Codable, Equatable, Hashable, Sendable {
    /// 1–12.
    let month: Int
    /// A day the month actually has — 1–29 for February, since no year.
    let day: Int

    init(month: Int, day: Int) {
        self.month = month
        self.day = day
    }
}

nonisolated struct UserDTO: Codable, Equatable, Sendable {
    let id: Int64
    let username: String
    let displayName: String
    let createdAt: Date?
    /// `0` = no profile picture. Absent on a server older than the
    /// avatars release, hence the default rather than an Optional.
    var avatarVersion: Int64 = 0
    /// Present when (and only when) one is set — absence IS "unset".
    var birthday: BirthdayDTO?
    /// True when this account has been deleted (protocol.md, "Deleting an
    /// account"). ABSENT on the wire when false — never `false` — so this
    /// is a defaulted Bool rather than an Optional. Such a user has no
    /// usable username, no picture and no birthday, and `displayName` is
    /// the server's ENGLISH placeholder, which a client that understands
    /// this flag replaces with its own translation.
    var deleted: Bool = false

    enum CodingKeys: String, CodingKey {
        case id
        case username
        case displayName = "display_name"
        case createdAt = "created_at"
        case avatarVersion = "avatar_version"
        case birthday
        case deleted
    }

    init(
        id: Int64,
        username: String,
        displayName: String,
        createdAt: Date?,
        avatarVersion: Int64 = 0,
        birthday: BirthdayDTO? = nil,
        deleted: Bool = false
    ) {
        self.id = id
        self.username = username
        self.displayName = displayName
        self.createdAt = createdAt
        self.avatarVersion = avatarVersion
        self.birthday = birthday
        self.deleted = deleted
    }

    /// Hand-written because a property default is NOT what Swift's
    /// synthesized decoder falls back to — a missing key throws. The
    /// field is absent on any server older than the avatars release, and
    /// the protocol's compatibility rule says that must still decode.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(Int64.self, forKey: .id)
        username = try container.decode(String.self, forKey: .username)
        displayName = try container.decode(String.self, forKey: .displayName)
        createdAt = try container.decodeIfPresent(Date.self, forKey: .createdAt)
        avatarVersion = try container.decodeIfPresent(Int64.self, forKey: .avatarVersion) ?? 0
        birthday = try container.decodeIfPresent(BirthdayDTO.self, forKey: .birthday)
        deleted = try container.decodeIfPresent(Bool.self, forKey: .deleted) ?? false
    }
}

nonisolated struct MemberDTO: Codable, Equatable, Identifiable, Sendable {
    let id: Int64
    let username: String
    let displayName: String
    /// "owner" | "member" — OPTIONAL, because a deleted account carries no
    /// role at all (protocol.md, "Deleting an account"). Present on every
    /// live member.
    let role: String?
    /// `0` = no profile picture. Absent on a server older than the
    /// avatars release, hence the default rather than an Optional.
    var avatarVersion: Int64 = 0
    /// Present when (and only when) one is set — absence IS "unset".
    var birthday: BirthdayDTO?
    /// True on a tombstone: an account deleted while in this family. Such
    /// a member is never in `members`, only in `former_members`, and
    /// exists there for one reason — so a stored message, note or reaction
    /// can still be given a sender. Absent on the wire when false.
    var deleted: Bool = false

    enum CodingKeys: String, CodingKey {
        case id
        case username
        case displayName = "display_name"
        case role
        case avatarVersion = "avatar_version"
        case birthday
        case deleted
    }

    init(
        id: Int64,
        username: String,
        displayName: String,
        role: String?,
        avatarVersion: Int64 = 0,
        birthday: BirthdayDTO? = nil,
        deleted: Bool = false
    ) {
        self.id = id
        self.username = username
        self.displayName = displayName
        self.role = role
        self.avatarVersion = avatarVersion
        self.birthday = birthday
        self.deleted = deleted
    }

    /// See UserDTO.init(from:) — a defaulted property is not a decoding
    /// fallback, and this field is absent on older servers.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(Int64.self, forKey: .id)
        username = try container.decode(String.self, forKey: .username)
        displayName = try container.decode(String.self, forKey: .displayName)
        role = try container.decodeIfPresent(String.self, forKey: .role)
        avatarVersion = try container.decodeIfPresent(Int64.self, forKey: .avatarVersion) ?? 0
        birthday = try container.decodeIfPresent(BirthdayDTO.self, forKey: .birthday)
        deleted = try container.decodeIfPresent(Bool.self, forKey: .deleted) ?? false
    }

    /// The same member wearing a different birthday. The roster screens
    /// patch the row they already hold after an edit rather than re-reading
    /// the whole family — a birthday raises no frame and no push
    /// (protocol.md, "Birthdays"), so the editing device is the only one
    /// that can show it before the next resync.
    func withBirthday(_ birthday: BirthdayDTO?) -> MemberDTO {
        MemberDTO(
            id: id,
            username: username,
            displayName: displayName,
            role: role,
            avatarVersion: avatarVersion,
            birthday: birthday,
            deleted: deleted)
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
    /// One of the nine tags in `FamilyLanguage`, or nil for UNSET — and
    /// unset is not English (protocol.md, "The family's language"). The
    /// wire says so by omitting the key, so this must stay an Optional.
    let language: String?
    /// Whether a mention of `@ai` in the family chat is shown the last
    /// month of it. ALWAYS present on the wire — a switch has no "unset" —
    /// but `true` here is the fallback for a server that predates it.
    let aiHistory: Bool

    enum CodingKeys: String, CodingKey {
        case id
        case name
        case joinPolicy = "join_policy"
        case createdAt = "created_at"
        case inviteCode = "invite_code"
        case language
        case aiHistory = "ai_history"
    }

    init(
        id: Int64,
        name: String,
        joinPolicy: String,
        createdAt: Date?,
        inviteCode: String?,
        language: String? = nil,
        aiHistory: Bool = true
    ) {
        self.id = id
        self.name = name
        self.joinPolicy = joinPolicy
        self.createdAt = createdAt
        self.inviteCode = inviteCode
        self.language = language
        self.aiHistory = aiHistory
    }

    /// Hand-written for the reason UserDTO's is, and this type had no
    /// decoder at all until the two fields below arrived: Swift does NOT
    /// fall back to a property's default for a missing key, it THROWS. A
    /// synthesized decoder plus `aiHistory = true` would therefore make
    /// every response from a server that predates the setting — /me
    /// included, which is what the app bootstraps from — undecodable.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(Int64.self, forKey: .id)
        name = try container.decode(String.self, forKey: .name)
        joinPolicy = try container.decode(String.self, forKey: .joinPolicy)
        createdAt = try container.decodeIfPresent(Date.self, forKey: .createdAt)
        inviteCode = try container.decodeIfPresent(String.self, forKey: .inviteCode)
        language = try container.decodeIfPresent(String.self, forKey: .language)
        aiHistory = try container.decodeIfPresent(Bool.self, forKey: .aiHistory) ?? true
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

/// One option of a poll, carrying the FULL current list of user ids that
/// chose it (docs/protocol.md, "Polls").
///
/// The list is complete state and never a delta, which is what lets a
/// client draw who voted for what — and, more usefully in a family, who
/// has not voted at all. It has to be a list rather than a "did I vote"
/// flag because one frame is serialised once and sent to every
/// connection: a field whose value depends on who is reading it cannot
/// exist on this wire.
nonisolated struct PollOptionDTO: Codable, Equatable, Sendable, Identifiable {
    let id: Int64
    let text: String
    /// Empty is normal — nobody has chosen this one yet — and is NOT the
    /// same as absent, which never happens: a Poll is complete state.
    var votes: [Int64] = []

    init(id: Int64, text: String, votes: [Int64] = []) {
        self.id = id
        self.text = text
        self.votes = votes
    }

    /// Hand-written for the reason UserDTO's is: a property default is
    /// not a decoding fallback, and an option nobody has voted for could
    /// legitimately arrive with the key omitted by a future server.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(Int64.self, forKey: .id)
        text = try container.decode(String.self, forKey: .text)
        votes = try container.decodeIfPresent([Int64].self, forKey: .votes) ?? []
    }
}

/// What makes a message votable (docs/protocol.md, "Polls").
///
/// The QUESTION is deliberately not in here: it is the message body, which
/// is the whole reason a poll costs so little — a chat-list preview, a push
/// alert and a reply excerpt all read it already, with no new case between
/// them, and a client that has never heard of polls draws the question as
/// an ordinary message and loses only the buttons.
///
/// `pollSeq` and `closed` are ALWAYS on the wire, unlike `reaction_seq`: a
/// poll has a sequence from the moment it exists, and `closed` is a boolean
/// with a real default, so there is no "unset" for a missing key to mean.
/// They are therefore non-optional here.
nonisolated struct PollDTO: Codable, Equatable, Sendable {
    let pollSeq: Int64
    let closed: Bool
    /// 2–10, in creation order, and fixed for the life of the poll — the
    /// votes already cast were cast against the list as it was read.
    let options: [PollOptionDTO]

    enum CodingKeys: String, CodingKey {
        case pollSeq = "poll_seq"
        case closed
        case options
    }

    init(pollSeq: Int64, closed: Bool, options: [PollOptionDTO]) {
        self.pollSeq = pollSeq
        self.closed = closed
        self.options = options
    }

    /// The option this user currently holds, if any. One choice per
    /// member — there is no multiple choice — so the first hit is it.
    func optionHeld(by userID: Int64) -> PollOptionDTO? {
        options.first { $0.votes.contains(userID) }
    }
}

/// The quoted message on a reply — as much of it as a bubble needs to draw
/// the quote without holding the original. The server recomputes this on
/// every read, so it follows the quoted message rather than freezing at
/// send time (docs/protocol.md, "Replies").
nonisolated struct ReplyToDTO: Codable, Equatable, Sendable {
    let messageID: Int64
    let senderID: Int64
    let excerpt: String
    /// What the quoted message was itself answering — one level, and no
    /// more. Absent is normal: the quoted message was not a reply, or its
    /// own parent has been swept by retention.
    var parent: QuotedParentDTO?

    enum CodingKeys: String, CodingKey {
        case messageID = "message_id"
        case senderID = "sender_id"
        case excerpt
        case parent
    }

    init(messageID: Int64, senderID: Int64, excerpt: String, parent: QuotedParentDTO? = nil) {
        self.messageID = messageID
        self.senderID = senderID
        self.excerpt = excerpt
        self.parent = parent
    }

    /// Hand-written because a defaulted property is NOT a decoding fallback:
    /// the synthesized initialiser THROWS on a missing key, so every message
    /// from a server that predates two-level quotes would fail to decode.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        messageID = try container.decode(Int64.self, forKey: .messageID)
        senderID = try container.decode(Int64.self, forKey: .senderID)
        excerpt = try container.decode(String.self, forKey: .excerpt)
        parent = try container.decodeIfPresent(QuotedParentDTO.self, forKey: .parent)
    }
}

/// The second and LAST level of a quote (docs/protocol.md, "Replies").
///
/// Deliberately not the same type as `ReplyToDTO`: it has no `parent`, so
/// there is nothing for a third level to decode into.
nonisolated struct QuotedParentDTO: Codable, Equatable, Sendable {
    let messageID: Int64
    let senderID: Int64
    let excerpt: String

    enum CodingKeys: String, CodingKey {
        case messageID = "message_id"
        case senderID = "sender_id"
        case excerpt
    }
}

/// A photo or video carried by a message.
///
/// The bytes are never in here — they come from `GET /attachments/{id}`.
/// This is what a bubble needs to lay itself out BEFORE a single byte
/// arrives: what it is, how big, what shape, and whether a preview exists
/// yet (docs/protocol.md, "Photos, videos and files").
// Hashable so a Mac window can be keyed by the attachment it shows.
nonisolated struct AttachmentDTO: Codable, Hashable, Identifiable, Sendable {
    let id: Int64
    /// "photo" | "video" | "file".
    let kind: String
    let mime: String
    let size: Int64
    let width: Int?
    let height: Int?
    let durationMS: Int?
    let hasPreview: Bool
    /// Required on a file, where it is the attachment's whole identity — a
    /// photo renders itself, where "attachment 34" tells nobody anything.
    /// Optional on audio and on a location, where it is a label.
    let name: String?
    /// Locations only, and always both: a location IS its coordinates
    /// (docs/protocol.md, "Locations"). They ride on the attachment rather
    /// than in bytes so a bubble can draw the pin the moment the message
    /// arrives, without a download that might fail.
    let latitude: Double?
    let longitude: Double?
    /// Locations only, and only when the sending device knew one: the
    /// radius in metres it believed the fix good to. Absent means UNKNOWN,
    /// which is drawn as a plain pin — never as perfect precision.
    let accuracyM: Int?

    enum CodingKeys: String, CodingKey {
        case id
        case kind
        case mime
        case size
        case width
        case height
        case durationMS = "duration_ms"
        case hasPreview = "has_preview"
        case name
        case latitude
        case longitude
        case accuracyM = "accuracy_m"
    }

    var isVideo: Bool { kind == Kind.video }
    var isFile: Bool { kind == Kind.file }
    var isAudio: Bool { kind == Kind.audio }
    var isLocation: Bool { kind == Kind.location }

    /// The pin, when there is one. Both halves or neither — a location with
    /// one coordinate is not a place.
    var coordinate: (latitude: Double, longitude: Double)? {
        guard isLocation, let latitude, let longitude else { return nil }
        return (latitude, longitude)
    }

    enum Kind {
        static let photo = "photo"
        static let video = "video"
        static let audio = "audio"
        static let file = "file"
        static let location = "location"
    }

    /// The same attachment with `hasPreview` replaced — the one field a
    /// client legitimately knows better than the upload response, which
    /// was minted BEFORE the preview upload ran. Every other field is
    /// fixed at upload time, so this is a copy rather than a mutation.
    func withPreviewFlag(_ hasPreview: Bool) -> AttachmentDTO {
        AttachmentDTO(
            id: id,
            kind: kind,
            mime: mime,
            size: size,
            width: width,
            height: height,
            durationMS: durationMS,
            hasPreview: hasPreview,
            name: name,
            latitude: latitude,
            longitude: longitude,
            accuracyM: accuracyM)
    }

    /// What the bubble calls it: the name for a file, a word for the rest.
    var displayName: String {
        if let name, !name.isEmpty { return name }
        if isVideo { return "Video" }
        if isAudio { return "Audio" }
        if isLocation { return "Location" }
        return isFile ? "File" : "Photo"
    }

    /// "1.2 MB" — the other half of what a file row shows.
    var displaySize: String {
        ByteCountFormatter.string(fromByteCount: size, countStyle: .file)
    }

    /// Aspect ratio for the bubble, or a sane default when the uploader
    /// could not determine the dimensions.
    var aspectRatio: Double {
        guard let width, let height, width > 0, height > 0 else { return 4.0 / 3.0 }
        return Double(width) / Double(height)
    }
}

nonisolated struct AttachmentResponse: Codable, Equatable, Sendable {
    let attachment: AttachmentDTO
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
    /// Present when (and only when) this message is a reply. Optional, so
    /// the synthesized decoder treats a missing key as "not a reply"
    /// rather than throwing — which is also what makes a server that
    /// predates replies decode.
    let replyTo: ReplyToDTO?
    /// Both present when (and only when) the body has been edited. The
    /// seq is the guard: a body only overwrites a stored one when it is
    /// at least as new (docs/protocol.md, "Editing").
    let editedAt: Date?
    let editSeq: Int64?
    /// Present when the message carries attachments — 1 to 10 of them, in
    /// the order the sender chose, and absent (never an empty array) on a
    /// message that carries none (docs/protocol.md, "Photos, videos,
    /// audio, files and locations").
    let attachments: [AttachmentDTO]?
    /// The FIRST element of `attachments`, kept on the wire for clients
    /// that predate plurality. A client that reads `attachments` ignores
    /// it — see `attachmentList`, which is the one read rule everywhere.
    let attachment: AttachmentDTO?
    /// Present when (and only when) this message is a poll — and polls
    /// exist in the FAMILY CHAT only (docs/protocol.md, "Polls"). The
    /// body is then the question.
    ///
    /// ABSENCE never means "the poll went away": a poll dies only with
    /// its message, so an incoming copy without one says nothing about a
    /// poll already stored — exactly the rule the reaction fields follow.
    let poll: PollDTO?
    /// Present when (and only when) this message is the record of a voice
    /// call (docs/protocol.md, "Voice calls"). The body is then an English
    /// placeholder a client that knows this object never shows.
    let call: CallDTO?

    enum CodingKeys: String, CodingKey {
        case id
        case chatID = "chat_id"
        case senderID = "sender_id"
        case clientMsgID = "client_msg_id"
        case body
        case createdAt = "created_at"
        case reactions
        case reactionSeq = "reaction_seq"
        case replyTo = "reply_to"
        case editedAt = "edited_at"
        case editSeq = "edit_seq"
        case attachments
        case attachment
        case poll
        case call
    }

    /// THE read rule for what a message carries: prefer the plural field,
    /// fall back to the legacy singular (a one-element list), and answer
    /// [] when the message carries nothing. Every consumer goes through
    /// this — reading `attachment` beside `attachments` would double the
    /// first element somewhere eventually.
    var attachmentList: [AttachmentDTO] {
        if let attachments, !attachments.isEmpty { return attachments }
        if let attachment { return [attachment] }
        return []
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
        reactionSeq: Int64? = nil,
        replyTo: ReplyToDTO? = nil,
        editedAt: Date? = nil,
        editSeq: Int64? = nil,
        attachments: [AttachmentDTO]? = nil,
        attachment: AttachmentDTO? = nil,
        poll: PollDTO? = nil,
        call: CallDTO? = nil
    ) {
        self.id = id
        self.chatID = chatID
        self.senderID = senderID
        self.clientMsgID = clientMsgID
        self.body = body
        self.createdAt = createdAt
        self.reactions = reactions
        self.reactionSeq = reactionSeq
        self.replyTo = replyTo
        self.editedAt = editedAt
        self.editSeq = editSeq
        self.attachments = attachments
        self.attachment = attachment
        self.poll = poll
        self.call = call
    }
}

/// The record of a voice call, riding on the message the server writes
/// into the direct chat afterwards (docs/protocol.md, "Voice calls").
/// `outcome` is `completed`, `missed`, `declined` or `failed`;
/// `durationSecs` is present when (and only when) the call was ever
/// answered. The record's sender is the CALLER, so which side of the call
/// the reader was on is `senderID == currentUserID`.
nonisolated struct CallDTO: Codable, Equatable, Hashable, Sendable {
    let outcome: String
    let durationSecs: Int?
    /// True when the record is of a VIDEO call (docs/protocol.md,
    /// "Video"). Present on the wire when (and only when) true — absent
    /// means voice, like every optional field there.
    let video: Bool

    enum CodingKeys: String, CodingKey {
        case outcome
        case durationSecs = "duration_secs"
        case video
    }

    init(outcome: String, durationSecs: Int? = nil, video: Bool = false) {
        self.outcome = outcome
        self.durationSecs = durationSecs
        self.video = video
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        outcome = try container.decode(String.self, forKey: .outcome)
        durationSecs = try container.decodeIfPresent(Int.self, forKey: .durationSecs)
        video = try container.decodeIfPresent(Bool.self, forKey: .video) ?? false
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(outcome, forKey: .outcome)
        try container.encodeIfPresent(durationSecs, forKey: .durationSecs)
        if video { try container.encode(true, forKey: .video) }
    }

    enum Outcome {
        static let completed = "completed"
        static let missed = "missed"
        static let declined = "declined"
        static let failed = "failed"
    }
}

/// One STUN or TURN server from `GET /calls/ice`. `username` and
/// `credential` on a TURN server only, and only when the operator
/// configured credentials.
nonisolated struct IceServerDTO: Codable, Equatable, Sendable {
    let urls: [String]
    let username: String?
    let credential: String?

    init(urls: [String], username: String? = nil, credential: String? = nil) {
        self.urls = urls
        self.username = username
        self.credential = credential
    }
}

/// `GET /calls/ice` reply. Fetched at the start of every call, never
/// cached across calls: a TURN credential is minted for the caller and
/// expires, and a stale one is a call that silently cannot relay.
nonisolated struct IceServersResponse: Codable, Equatable, Sendable {
    let iceServers: [IceServerDTO]
    let ttlSecs: Int

    enum CodingKeys: String, CodingKey {
        case iceServers = "ice_servers"
        case ttlSecs = "ttl_secs"
    }
}

// MARK: - Endpoint envelopes

nonisolated struct AuthResponse: Codable, Equatable, Sendable {
    let token: String
    let user: UserDTO
}

/// `PUT /me/avatar` reply — the caller with their new `avatar_version`.
nonisolated struct AvatarResponse: Codable, Equatable, Sendable {
    let user: UserDTO
}

/// `PUT /me/birthday` reply — the caller with the birthday they just set.
/// The same shape as AvatarResponse and deliberately not the same type:
/// this file's rule is one named envelope per endpoint, so a reviewer can
/// diff it against the protocol table line by line.
nonisolated struct UserResponse: Codable, Equatable, Sendable {
    let user: UserDTO
}

nonisolated struct MeResponse: Codable, Equatable, Sendable {
    let user: UserDTO
    let family: FamilyDTO?
    /// "owner" | "member" | nil
    let role: String?
    let pendingJoinRequest: PendingJoinRequestDTO?
    /// Whether this server signals voice calls at all (docs/protocol.md,
    /// "Voice calls"). ALWAYS present on a current server; defaulted to
    /// false for one that predates calls, which is also the right answer
    /// there — it has no `call_offer` to accept.
    var callsEnabled: Bool = false
    /// Whether this server accepts VIDEO calls (`[calls] video_enabled`,
    /// docs/protocol.md, "Video"). ALWAYS present on a current server,
    /// like `calls_enabled`; defaulted to false for one that predates
    /// video — which is also the right answer there, it would refuse the
    /// offer with `video_calls_disabled`.
    var videoCallsEnabled: Bool = false

    enum CodingKeys: String, CodingKey {
        case user
        case family
        case role
        case pendingJoinRequest = "pending_join_request"
        case callsEnabled = "calls_enabled"
        case videoCallsEnabled = "video_calls_enabled"
    }

    init(
        user: UserDTO,
        family: FamilyDTO?,
        role: String?,
        pendingJoinRequest: PendingJoinRequestDTO?,
        callsEnabled: Bool = false,
        videoCallsEnabled: Bool = false
    ) {
        self.user = user
        self.family = family
        self.role = role
        self.pendingJoinRequest = pendingJoinRequest
        self.callsEnabled = callsEnabled
        self.videoCallsEnabled = videoCallsEnabled
    }

    /// Hand-written for the reason every other defaulted field on this
    /// wire is: a property default is not a decoding fallback.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        user = try container.decode(UserDTO.self, forKey: .user)
        family = try container.decodeIfPresent(FamilyDTO.self, forKey: .family)
        role = try container.decodeIfPresent(String.self, forKey: .role)
        pendingJoinRequest = try container.decodeIfPresent(PendingJoinRequestDTO.self, forKey: .pendingJoinRequest)
        callsEnabled = try container.decodeIfPresent(Bool.self, forKey: .callsEnabled) ?? false
        videoCallsEnabled = try container.decodeIfPresent(Bool.self, forKey: .videoCallsEnabled) ?? false
    }
}

nonisolated struct FamilyResponse: Codable, Equatable, Sendable {
    let family: FamilyDTO
}

nonisolated struct JoinResponse: Codable, Equatable, Sendable {
    /// "joined" | "pending"
    let status: String
}

/// The assistant, as `GET /families/mine` reports it.
///
/// Not a `MemberDTO` and deliberately not in `members`: it belongs to no
/// family, cannot be messaged one-to-one, removed, made owner or given a
/// password, and every screen that lists people would need a special case
/// for it. What it IS good for is naming its messages in the family chat —
/// its reserved account is absent from the roster on purpose, so a lookup
/// there finds nothing — and telling the composer the feature exists.
nonisolated struct AssistantDTO: Codable, Equatable, Sendable {
    let userID: Int64
    let displayName: String
    /// The token that summons it, from the server rather than hard-coded:
    /// the grammar is shared (AssistantMention) but the spelling belongs to
    /// the protocol, and a client inventing its own would be unanswerable.
    let mention: String

    enum CodingKeys: String, CodingKey {
        case userID = "user_id"
        case displayName = "display_name"
        case mention
    }
}

nonisolated struct FamilyMineResponse: Codable, Equatable, Sendable {
    let family: FamilyDTO
    let members: [MemberDTO]
    /// The accounts deleted while in this family, each with `deleted: true`
    /// and no role. Omitted by the server when there are none, hence the
    /// hand-written decoder below rather than an Optional nobody wants to
    /// unwrap at every call site.
    ///
    /// They are NOT members: they hold no role, are offered to nobody as
    /// somebody to chat with, and count towards nothing. A client stores
    /// both arrays in one place — that is what lets a stored message still
    /// name its sender — and draws only `members`.
    let formerMembers: [MemberDTO]
    /// Absent when the server has no assistant configured — which is the
    /// whole of the capability check.
    let assistant: AssistantDTO?
    /// The board cursor, omitted while the board has never been written to.
    /// It rides along on the call every client already makes on resync, so
    /// learning whether a board catch-up is needed costs no extra request.
    let maxBoardSeq: Int64?

    enum CodingKeys: String, CodingKey {
        case family
        case members
        case formerMembers = "former_members"
        case assistant
        case maxBoardSeq = "max_board_seq"
    }

    init(
        family: FamilyDTO,
        members: [MemberDTO],
        formerMembers: [MemberDTO] = [],
        assistant: AssistantDTO? = nil,
        maxBoardSeq: Int64? = nil
    ) {
        self.family = family
        self.members = members
        self.formerMembers = formerMembers
        self.assistant = assistant
        self.maxBoardSeq = maxBoardSeq
    }

    /// Hand-written for the reason UserDTO's is: a property default is not
    /// a decoding fallback, and `former_members` is absent from every
    /// response of a family that has never lost anybody — which is most of
    /// them, and all of them on a server that predates account deletion.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        family = try container.decode(FamilyDTO.self, forKey: .family)
        members = try container.decode([MemberDTO].self, forKey: .members)
        formerMembers = try container.decodeIfPresent([MemberDTO].self, forKey: .formerMembers) ?? []
        assistant = try container.decodeIfPresent(AssistantDTO.self, forKey: .assistant)
        maxBoardSeq = try container.decodeIfPresent(Int64.self, forKey: .maxBoardSeq)
    }
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

/// One sticker note on the family board.
///
/// A TOMBSTONE is the same object with `deleted: true` and no content: the
/// change feed has to be able to say "this note is gone", and an absent row
/// cannot say anything (docs/protocol.md, "Board"). Every content field is
/// therefore optional — a tombstone carries only `id`, `deleted` and
/// `boardSeq`.
///
/// `size` is optional for a second reason: a server from before the field
/// existed never sends it, and such a note is `medium` — the one size every
/// note had then — rather than a decode failure.
nonisolated struct NoteDTO: Codable, Equatable, Sendable {
    let id: Int64
    let authorID: Int64?
    let text: String?
    let color: String?
    let size: String?
    let x: Double?
    let y: Double?
    let createdAt: Date?
    let updatedAt: Date?
    let boardSeq: Int64
    let deleted: Bool?

    enum CodingKeys: String, CodingKey {
        case id
        case authorID = "author_id"
        case text
        case color
        case size
        case x
        case y
        case createdAt = "created_at"
        case updatedAt = "updated_at"
        case boardSeq = "board_seq"
        case deleted
    }

    var isTombstone: Bool { deleted == true }
}

nonisolated struct BoardResponse: Codable, Equatable, Sendable {
    let notes: [NoteDTO]
    let maxBoardSeq: Int64

    enum CodingKeys: String, CodingKey {
        case notes
        case maxBoardSeq = "max_board_seq"
    }
}

nonisolated struct BoardChangesResponse: Codable, Equatable, Sendable {
    let notes: [NoteDTO]
}

nonisolated struct NoteResponse: Codable, Equatable, Sendable {
    let note: NoteDTO
}

nonisolated struct ChatListItemDTO: Codable, Equatable, Sendable {
    let chat: ChatDTO
    let lastMessage: MessageDTO?
    let unreadCount: Int
    /// Max reaction_seq over the chat's messages; omitted by the server
    /// while no message in the chat has ever been reacted to (treat as 0).
    let maxReactionSeq: Int64?
    /// Max edit_seq over the chat's messages; omitted while nothing in the
    /// chat has been edited (treat as 0). The twin of maxReactionSeq, and
    /// the reason a client knows whether an edit catch-up is worth a
    /// request at all.
    let maxEditSeq: Int64?
    /// Max poll_seq over the chat's messages; omitted while the chat
    /// holds no poll at all (treat as 0). The third of the same shape,
    /// and what tells a client whether a poll catch-up is worth a
    /// request — a family that has never run a poll costs none.
    let maxPollSeq: Int64?
    /// The CALLER'S OWN read marker for this chat — the value `POST
    /// /chats/{id}/read` and the `read` frame maintain, monotonic and
    /// shared across every device this person owns. The other half of
    /// `unread_count`, off the same row of the same query, so the two
    /// always describe one instant.
    ///
    /// Unlike the three `max_*_seq` cursors it is ALWAYS present and `0`
    /// is a real answer — "has never reported reading anything here" —
    /// rather than an absent one. It is Optional here for exactly one
    /// reason: a server binary older than the field omits it, and this
    /// client must go on reading its chat list. Nothing else has to care,
    /// because the marker is applied monotonically (`max(stored,
    /// received)`) and absent therefore lands in the same place as 0 — on
    /// the stored value, unchanged.
    ///
    /// It is an id THRESHOLD and never a reference: retention may already
    /// have swept the message it names, so nothing may try to fetch it.
    let lastReadMessageID: Int64?

    enum CodingKeys: String, CodingKey {
        case chat
        case lastMessage = "last_message"
        case unreadCount = "unread_count"
        case maxReactionSeq = "max_reaction_seq"
        case maxEditSeq = "max_edit_seq"
        case maxPollSeq = "max_poll_seq"
        case lastReadMessageID = "last_read_message_id"
    }

    /// Explicit memberwise init so the seq fields default to absent —
    /// construction sites that predate them stay valid.
    init(
        chat: ChatDTO,
        lastMessage: MessageDTO?,
        unreadCount: Int,
        maxReactionSeq: Int64? = nil,
        maxEditSeq: Int64? = nil,
        maxPollSeq: Int64? = nil,
        lastReadMessageID: Int64? = nil
    ) {
        self.chat = chat
        self.lastMessage = lastMessage
        self.unreadCount = unreadCount
        self.maxReactionSeq = maxReactionSeq
        self.maxEditSeq = maxEditSeq
        self.maxPollSeq = maxPollSeq
        self.lastReadMessageID = lastReadMessageID
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

/// One message's full poll state: the body of the vote / retract / close
/// endpoints and the page entry of GET /chats/{id}/polls.
///
/// The seq lives INSIDE the poll here, unlike the reaction shape where it
/// is a sibling of the array — the protocol puts it there because a poll
/// always has one, so there is no state without a sequence to guard it.
nonisolated struct PollStateDTO: Codable, Equatable, Sendable {
    let messageID: Int64
    let poll: PollDTO

    enum CodingKeys: String, CodingKey {
        case messageID = "message_id"
        case poll
    }
}

nonisolated struct ChatPollsResponse: Codable, Equatable, Sendable {
    let polls: [PollStateDTO]
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

// MARK: - Family statistics

/// What the family has actually sent (docs/protocol.md, "Family
/// statistics"). Every member sees the same numbers.
nonisolated struct FamilyStatsDTO: Codable, Equatable, Sendable {
    let totals: StatsTotalsDTO
    let members: [MemberStatsDTO]
}

nonisolated struct StatsTotalsDTO: Codable, Equatable, Sendable {
    let members: Int
    let messages: Int
    let boardNotes: Int
    let attachments: AttachmentStatsDTO
    let ai: AiStatsDTO

    enum CodingKeys: String, CodingKey {
        case members
        case messages
        case boardNotes = "board_notes"
        case attachments
        case ai
    }
}

nonisolated struct MemberStatsDTO: Codable, Equatable, Sendable, Identifiable {
    var id: Int64 { userID }
    let userID: Int64
    let displayName: String
    let messages: Int
    let attachments: AttachmentStatsDTO
    let ai: AiStatsDTO

    enum CodingKeys: String, CodingKey {
        case userID = "user_id"
        case displayName = "display_name"
        case messages
        case attachments
        case ai
    }
}

nonisolated struct AttachmentStatsDTO: Codable, Equatable, Sendable {
    let count: Int
    let bytes: Int64
    let photo: Int
    let video: Int
    let audio: Int
    let file: Int
    /// Each distinct file counted once. Family totals only — a file two
    /// members both sent belongs to neither alone, so there is no
    /// per-member share of it.
    var storedBytes: Int64?

    enum CodingKeys: String, CodingKey {
        case count, bytes, photo, video, audio, file
        case storedBytes = "stored_bytes"
    }
}

nonisolated struct AiStatsDTO: Codable, Equatable, Sendable {
    let questions: Int
    let promptTokens: Int
    let completionTokens: Int

    enum CodingKeys: String, CodingKey {
        case questions
        case promptTokens = "prompt_tokens"
        case completionTokens = "completion_tokens"
    }
}
