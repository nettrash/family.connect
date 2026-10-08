//
//  DesktopNotificationRules.swift
//  FamilyConnect
//
//  When the Mac raises a notification of its own, and what it says (issue
//  #84) — the Windows client's NotificationRules, the same answers:
//
//  - THE BODY IS NEVER THE MESSAGE. It is what a server with
//    `include_message_body = false` would send — "New message", "New note".
//    A desktop banner sits on a screen other people walk past and stays in
//    Notification Center afterwards, and the app cannot know whether the
//    family's operator wanted their words there, so it never puts them there.
//  - THE BLOCK REACHES ONE STEP FURTHER THAN THE SENDER. A blocked member's
//    message raises nothing, and neither does the ASSISTANT'S answer to one —
//    that would light up a banner for a thread its reader cannot read.
//  - A NOTE IS NEWS ONLY WHEN THE BOARD BADGE SAYS SO, so the two never
//    disagree (BoardBadge.isUnread).
//  - Nothing while the person has switched it off, and nothing about what
//    they are looking at: a chat they are READING (ChatPresence — the same
//    question the unread rule asks), or the board window in front of them.
//
//  Pure and platform-neutral, so both test runs pin it; ChatSyncCoordinator
//  is the only caller, on the Mac.
//

import Foundation

nonisolated enum DesktopNotificationRules {

    /// Whether a notification is raised, and when not, why — the reason goes
    /// to the log (ids only), so a Mac that stays silent can say what it
    /// decided.
    enum Verdict: Equatable, Sendable {
        case announce
        case switchedOff
        case own
        case blocked
        case answersBlocked
        case beingRead
        case unknownChat
        case notNews
    }

    static func message(
        senderID: Int64,
        me: Int64?,
        blocked: Set<Int64>,
        assistantID: Int64?,
        repliesTo quotedSenderID: Int64?,
        isReading: Bool,
        chatKnown: Bool,
        wanted: Bool
    ) -> Verdict {
        guard wanted else { return .switchedOff }
        if senderID == me { return .own }
        if blocked.contains(senderID) { return .blocked }
        // The block, one step further: the assistant answering somebody this
        // reader has blocked.
        if let assistantID, senderID == assistantID,
           let quotedSenderID, blocked.contains(quotedSenderID) {
            return .answersBlocked
        }
        if isReading { return .beingRead }
        // A chat this device does not hold yet: a banner that cannot be
        // opened onto anything is worse than none.
        guard chatKnown else { return .unknownChat }
        return .announce
    }

    static func note(
        authorID: Int64,
        me: Int64?,
        blocked: Set<Int64>,
        isNews: Bool,
        boardInFront: Bool,
        wanted: Bool
    ) -> Verdict {
        guard wanted else { return .switchedOff }
        if authorID == me { return .own }
        if blocked.contains(authorID) { return .blocked }
        guard isNews else { return .notNews }
        if boardInFront { return .beingRead }
        return .announce
    }
}
