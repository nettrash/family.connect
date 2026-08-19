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

nonisolated struct MessageSnapshot: Equatable, Sendable, Identifiable {
    var id: String { localID }
    let localID: String
    let serverID: Int64?
    let chatID: Int64
    let senderID: Int64
    let body: String
    let createdAt: Date
    let state: MessageStatus
}

nonisolated struct MemberSnapshot: Equatable, Sendable, Identifiable {
    var id: Int64 { userID }
    let userID: Int64
    let username: String
    let displayName: String
    let role: String
    let isCurrentUser: Bool
    let hasLeft: Bool
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
            state: entity.state
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
            hasLeft: entity.hasLeft
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

nonisolated enum MessagePresentation {

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
}
