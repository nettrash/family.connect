//
//  Snapshots.swift
//  FamilyConnect
//
//  Plain value-type mirrors of the SwiftData entities plus the pure
//  presentation rules of the conversation screen (day sectioning, when a
//  sender name shows, when a bubble counts as read).
//
//  Why value types at all: @Model classes are MainActor-bound live objects;
//  the grouping/labelling rules below are pure functions that the unit
//  tests want to hammer with synthetic data and no ModelContainer. Views
//  convert entities → snapshots at render time (cheap: a handful of value
//  copies per visible message) and feed these functions.
//

import Foundation

// MARK: - Value mirrors

nonisolated struct ChatSnapshot: Equatable, Sendable, Identifiable {
    var id: Int64 { chatID }
    let chatID: Int64
    let kind: String
    let title: String
    let peerUserID: Int64?
    let unreadCount: Int
    let othersReadUpTo: Int64

    var isFamilyChat: Bool { kind == "family" }
}

/// One user's reaction to one message. Codable in the wire shape
/// (`{"user_id":9,"emoji":"❤️"}`) because MessageEntity persists its full
/// reaction state as exactly this JSON — one spelling, no translation.
nonisolated struct ReactionSnapshot: Equatable, Sendable, Codable {
    let userID: Int64
    let emoji: String

    enum CodingKeys: String, CodingKey {
        case userID = "user_id"
        case emoji
    }

    init(userID: Int64, emoji: String) {
        self.userID = userID
        self.emoji = emoji
    }
}

nonisolated struct MessageSnapshot: Equatable, Sendable, Identifiable {
    var id: String { localID }
    let localID: String
    let serverID: Int64?
    let chatID: Int64
    let senderID: Int64
    let body: String
    let createdAt: Date
    let state: MessageStatus
    /// Full current reaction state; [] both for "never reacted" and
    /// "cleared" — the view only cares whether there is anything to draw.
    var reactions: [ReactionSnapshot] = []
    /// The quoted message when this one is a reply. nil otherwise.
    var replyTo: ReplyToSnapshot?
    /// True once the body has been edited — the bubble says so.
    var isEdited: Bool = false
    /// The photo or video this message carries.
    var attachment: AttachmentDTO?
}

/// The quote a reply draws above its own text.
///
/// `excerpt` is normally the SERVER's — recomputed on every read — but a
/// client cuts its own while a send is still pending, and must cut it the
/// same way or the quote visibly changes length when the ack lands. A flat snapshot, not a
/// reference: the quoted message may be far outside the cached window, or
/// never have been fetched at all.
nonisolated struct ReplyToSnapshot: Equatable, Sendable {
    let messageID: Int64
    let senderID: Int64
    let excerpt: String
    /// One more level, and no more (docs/protocol.md, "Replies").
    var parent: QuotedParentSnapshot?

    /// Longest excerpt, in Unicode scalar values — matching
    /// `ReplyTo::MAX_EXCERPT_CHARS` on the server and `MAX_EXCERPT_CHARS`
    /// on Android.
    static let maxExcerptScalars = 120

    /// Cut like the server does: by SCALARS, not by Swift Characters.
    /// `String.prefix` counts extended grapheme clusters, so a body of
    /// family emoji (7 scalars each) would keep ~7x more text than the
    /// server returns and the quote would visibly shrink on ack.
    static func excerpt(of body: String) -> String {
        String(String.UnicodeScalarView(body.unicodeScalars.prefix(maxExcerptScalars)))
    }
}

/// The second and LAST level of a quote. A distinct type from
/// `ReplyToSnapshot` on purpose: with no `parent` of its own, there is
/// nothing for a third level to live in.
nonisolated struct QuotedParentSnapshot: Equatable, Sendable {
    let messageID: Int64
    let senderID: Int64
    let excerpt: String
}

nonisolated struct MemberSnapshot: Equatable, Sendable, Identifiable {
    var id: Int64 { userID }
    let userID: Int64
    let username: String
    let displayName: String
    let role: String
    let isCurrentUser: Bool
    let hasLeft: Bool
    var avatarVersion: Int64 = 0
}

// MARK: - Entity → snapshot bridges (MainActor: entities live there)

extension MessageSnapshot {
    init(_ entity: MessageEntity) {
        self.init(
            localID: entity.localID,
            serverID: entity.serverID,
            chatID: entity.chatID,
            senderID: entity.senderID,
            body: entity.body,
            createdAt: entity.createdAt,
            state: entity.state,
            reactions: entity.reactionList,
            replyTo: entity.replySnapshot,
            isEdited: entity.editSeq > 0,
            attachment: entity.attachmentSnapshot
        )
    }
}

extension ChatSnapshot {
    init(_ entity: ChatEntity) {
        self.init(
            chatID: entity.chatID,
            kind: entity.kind,
            title: entity.title,
            peerUserID: entity.peerUserID,
            unreadCount: entity.unreadCount,
            othersReadUpTo: entity.othersReadUpTo
        )
    }
}

extension MemberSnapshot {
    init(_ entity: MemberEntity) {
        self.init(
            userID: entity.userID,
            username: entity.username,
            displayName: entity.displayName,
            role: entity.role,
            isCurrentUser: entity.isCurrentUser,
            hasLeft: entity.hasLeft,
            avatarVersion: entity.avatarVersion
        )
    }
}

// MARK: - Conversation presentation rules

/// One calendar day's worth of messages, in order. `day` is the start of
/// day in the grouping calendar, used both as the section id and to format
/// the "Today / Yesterday / date" pill.
nonisolated struct DaySection: Equatable, Sendable, Identifiable {
    var id: Date { day }
    let day: Date
    let messages: [MessageSnapshot]
}

/// One aggregated chip in the reaction row under a bubble: an emoji, how
/// many members chose it, and whether the current user is among them
/// (tints the chip and flips the tap from set to remove).
nonisolated struct ReactionChip: Equatable, Sendable, Identifiable {
    var id: String { emoji }
    let emoji: String
    let count: Int
    let includesMe: Bool
}

/// One emoji's row in the "who reacted" popover: the emoji plus the
/// display names of everyone who chose it, current user rendered as
/// "You" and listed first.
nonisolated struct ReactionDetail: Equatable, Sendable, Identifiable {
    var id: String { emoji }
    let emoji: String
    let names: [String]
    /// The first reactor listed, so the row can lead with their face.
    /// Nil only when the row somehow has no reactors. Android carries the
    /// same thing as its `reactorIds` map.
    let leadUserID: Int64?

    /// Explicit so the ~dozen construction sites that predate the avatar
    /// (tests included) stay valid.
    init(emoji: String, names: [String], leadUserID: Int64? = nil) {
        self.emoji = emoji
        self.names = names
        self.leadUserID = leadUserID
    }
}

nonisolated enum MessagePresentation {

    /// The quick-set offered by the long-press picker. Client UI only —
    /// the server accepts any emoji (≤ 32 bytes UTF-8), so chips render
    /// whatever arrives; this is just what WE offer to send.
    static let quickReactions = ["❤️", "👍", "👎", "😂", "😮", "😢"]

    /// The reaction a bubble double-tap toggles (the Tapback-heart
    /// idiom). Kept inside the quick set so the capsule shows it
    /// selected. Same value on Android.
    static let doubleTapReaction = "❤️"

    /// Group an already-sorted message list into calendar-day sections.
    /// The input order is preserved inside each section, and sections come
    /// out in the order their first message appears — so a correctly
    /// sorted input yields chronologically sorted sections without a
    /// second sort.
    static func daySections(
        _ messages: [MessageSnapshot],
        calendar: Calendar = .current
    ) -> [DaySection] {
        var sections: [DaySection] = []
        for message in messages {
            let day = calendar.startOfDay(for: message.createdAt)
            if let last = sections.indices.last, sections[last].day == day {
                sections[last] = DaySection(day: day, messages: sections[last].messages + [message])
            } else {
                sections.append(DaySection(day: day, messages: [message]))
            }
        }
        return sections
    }

    /// Whether the sender's name caption shows above a bubble.
    ///
    /// Rules: only in the family chat (a direct chat has exactly one other
    /// person — the name would be noise), never on own bubbles, and only
    /// when the sender *changes* relative to the previous bubble in the
    /// same day section (the first bubble of a section has no previous).
    static func showsSenderName(
        at index: Int,
        in section: [MessageSnapshot],
        isFamilyChat: Bool,
        currentUserID: Int64
    ) -> Bool {
        guard isFamilyChat, section.indices.contains(index) else { return false }
        let message = section[index]
        guard message.senderID != currentUserID else { return false }
        guard index > 0 else { return true }
        return section[index - 1].senderID != message.senderID
    }

    /// Whether an own message counts as read by someone else: it must be
    /// confirmed (has a server id) and covered by the highest read marker
    /// any other member reported. Pending/failed messages are never read.
    static func isRead(_ message: MessageSnapshot, othersReadUpTo: Int64) -> Bool {
        guard let serverID = message.serverID else { return false }
        return serverID <= othersReadUpTo
    }

    /// Aggregate a message's raw reaction list into the chips the bubble
    /// draws: one per distinct emoji, in the order each emoji first
    /// appears in the list (the server preserves reaction order, so this
    /// is stable across re-renders — no popularity re-sorting jumps).
    static func reactionChips(
        _ reactions: [ReactionSnapshot],
        currentUserID: Int64
    ) -> [ReactionChip] {
        var order: [String] = []
        var counts: [String: Int] = [:]
        var mine: Set<String> = []
        for reaction in reactions {
            if counts[reaction.emoji] == nil { order.append(reaction.emoji) }
            counts[reaction.emoji, default: 0] += 1
            if reaction.userID == currentUserID { mine.insert(reaction.emoji) }
        }
        return order.map { emoji in
            ReactionChip(emoji: emoji, count: counts[emoji] ?? 0, includesMe: mine.contains(emoji))
        }
    }

    /// Expand a message's raw reaction list into the "who reacted"
    /// popover's rows: one per distinct emoji, in the same first-seen
    /// order as `reactionChips` (the two views must agree). Within an
    /// emoji, the current user shows as "You" and comes first; everyone
    /// else keeps reaction-list order under their display name from
    /// `names` ("Someone" when a reactor is no longer a known member).
    static func reactionDetails(
        _ reactions: [ReactionSnapshot],
        names: [Int64: String],
        currentUserID: Int64
    ) -> [ReactionDetail] {
        var order: [String] = []
        var others: [String: [String]] = [:]
        var otherIDs: [String: [Int64]] = [:]
        var mine: Set<String> = []
        for reaction in reactions {
            if others[reaction.emoji] == nil {
                order.append(reaction.emoji)
                others[reaction.emoji] = []
            }
            if reaction.userID == currentUserID {
                mine.insert(reaction.emoji)
            } else {
                others[reaction.emoji, default: []].append(
                    names[reaction.userID] ?? String(localized: "Someone"))
                otherIDs[reaction.emoji, default: []].append(reaction.userID)
            }
        }
        return order.map { emoji in
            let rest = others[emoji] ?? []
            let isMine = mine.contains(emoji)
            let all = isMine ? [String(localized: "You")] + rest : rest
            // Same rule as the names: "You" leads its emoji, so my own id
            // leads it too.
            let lead = isMine ? currentUserID : otherIDs[emoji]?.first
            return ReactionDetail(emoji: emoji, names: all, leadUserID: lead)
        }
    }
}
