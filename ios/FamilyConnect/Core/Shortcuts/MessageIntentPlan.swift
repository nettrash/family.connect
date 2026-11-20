//
//  MessageIntentPlan.swift
//  FamilyConnect
//
//  What a Shortcut asked for, reduced to whether anything may be sent and
//  where it lands (issue #118: a message on a schedule, or a photo, a voice
//  recording or any other file, to somebody in the family).
//
//  WHY THIS IS ITS OWN TYPE. There is no new send here: the message goes
//  through the ordinary composer pipeline (`ChatSyncCoordinator.send` and
//  `sendMedia`), with its outbox, its retry and its red bubble. What a
//  Shortcut takes away is the window, and with it the two questions a
//  person answers by looking at that window — IS THERE ANYTHING TO SEND,
//  and WHO. Both are decided here, in the open, and pinned by
//  MessageIntentPlanTests. The intent framework is not testable on the
//  simulator, which is exactly why CallRequestRouter is pure too.
//
//  THE RULES, IN THE ORDER THEY ARE ASKED.
//
//  1. Nothing goes anywhere while the app is signed out. Asked first,
//     because it is the one refusal the user can fix without touching the
//     Shortcut: whatever else is wrong with it, opening the app and signing
//     in is the answer.
//  2. An empty body with nothing attached is refused, never sent. This is
//     the scheduled case the issue asks for — "every hour" — and a blank
//     row posted hourly is not what anybody means by it.
//  3. More attachments than one message holds is refused, not truncated. A
//     Shortcut that hands over twelve files means twelve; half of them
//     arriving, silently, is the bad answer. The cap is the composer's own
//     (ShareHandoff.maxAttachmentsPerMessage).
//  4. The recipient. An id from the Shortcuts picker is exact. A NAME —
//     what a phrase or another action's text gives us — is matched the way
//     Siri's spoken name is (CallRequestRouter.match), so two "Anna"s are
//     a refusal to guess rather than a coin toss, and a blocked member is
//     refused out loud. That refusal reaches nobody but the blocker, as it
//     does for a call (docs/protocol.md, "Calls").
//  5. The chat, last, because it is the only answer that costs a request: a
//     direct chat this device holds is used as it is, and one it does not
//     hold has to be created first, which is the runner's business.
//

import Foundation

nonisolated enum MessageIntentPlan {

    /// Who the Shortcut named.
    enum Recipient: Equatable, Sendable {
        /// The family's own chat — the ordinary target for a scheduled
        /// "good morning" or a daily reminder.
        case familyChat
        /// Somebody the picker resolved, or a handle the system handed back.
        case member(userID: Int64)
        /// A name as text: what a phrase or another action's output gives
        /// us, with no id behind it.
        case named(String)
    }

    /// Why nothing was sent, with nothing identifying in it beyond a member
    /// id the caller already knows (CONTRIBUTING §12: ids and counts).
    enum Refusal: Equatable, Sendable {
        case signedOut
        case nothingToSend
        case tooManyAttachments
        case unknownRecipient
        /// Two members share the name. Guessing is worse than refusing: a
        /// message that lands in the wrong person's chat cannot be taken
        /// back out of it.
        case ambiguous([CallRequestRouter.Candidate])
        case blocked(Int64)
        /// No family chat on this device — signed out of the family, or a
        /// store that has never synced one.
        case noFamilyChat
    }

    enum Decision: Equatable, Sendable {
        /// Send into a chat this device already holds. `peerUserID` is the
        /// member on the other end of a direct chat and nil for the family
        /// chat; the runner needs it only to say who it told ("Sent to
        /// Anna."), never to decide anything.
        case send(chatID: Int64, peerUserID: Int64?)
        /// The member has no direct chat here yet, so it has to be created
        /// (get-or-create, `ChatSyncCoordinator.openDirectChat`) and the
        /// message goes into that.
        case openDirectThenSend(userID: Int64)
        case refuse(Refusal)
    }

    /// What the decision consults — all of it read, never written, and all
    /// of it handed in. The same shape as `CallRequestRouter.Directory`,
    /// for the same reason: a rule that asked the store itself could not be
    /// tested without one.
    struct Directory {
        var isActiveMember: (Int64) -> Bool
        /// Separate from `isActiveMember` on purpose: a blocked member is
        /// still on the roster and still nameable, and the two questions
        /// have different right answers everywhere else.
        var isBlocked: (Int64) -> Bool
        var roster: () -> [CallRequestRouter.Candidate]
        var directChatID: (Int64) -> Int64?
        var familyChatID: () -> Int64?
    }

    /// The whole decision. `attachmentCount` is what the caller was handed,
    /// not what survives preparation — a file that turns out to be
    /// unreadable is the runner's failure to report, not a smaller send.
    static func decide(
        recipient: Recipient,
        body: String?,
        attachmentCount: Int,
        signedIn: Bool,
        in directory: Directory
    ) -> Decision {
        guard signedIn else { return .refuse(.signedOut) }
        guard body?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false
            || attachmentCount > 0 else {
            return .refuse(.nothingToSend)
        }
        guard attachmentCount <= ShareHandoff.maxAttachmentsPerMessage else {
            return .refuse(.tooManyAttachments)
        }
        switch recipient {
        case .familyChat:
            guard let chatID = directory.familyChatID() else { return .refuse(.noFamilyChat) }
            return .send(chatID: chatID, peerUserID: nil)
        case .member(let userID):
            guard directory.isActiveMember(userID) else { return .refuse(.unknownRecipient) }
            if directory.isBlocked(userID) { return .refuse(.blocked(userID)) }
            return chat(with: userID, in: directory)
        case .named(let name):
            let roster = directory.roster().filter { directory.isActiveMember($0.userID) }
            switch CallRequestRouter.match(name: name, in: roster) {
            case .none:
                return .refuse(.unknownRecipient)
            case .one(let member):
                if directory.isBlocked(member.userID) { return .refuse(.blocked(member.userID)) }
                return chat(with: member.userID, in: directory)
            case .several(let members):
                // As in a call: one callable Anna out of two Annas is not
                // ambiguous, and none callable is a block rather than a
                // question nobody here can answer.
                let callable = members.filter { !directory.isBlocked($0.userID) }
                if callable.count == 1 { return chat(with: callable[0].userID, in: directory) }
                if let onlyBlocked = callable.isEmpty ? members.first : nil {
                    return .refuse(.blocked(onlyBlocked.userID))
                }
                return .refuse(.ambiguous(members))
            }
        }
    }

    private static func chat(with userID: Int64, in directory: Directory) -> Decision {
        directory.directChatID(userID).map { Decision.send(chatID: $0, peerUserID: userID) }
            ?? .openDirectThenSend(userID: userID)
    }

    // MARK: - The picker

    /// The id a Shortcuts picker hands back, and the recipient behind it.
    ///
    /// A member's id IS their handle (CallHandle) — the very string Siri
    /// carries for a call, so one identity is minted in one place and a
    /// person who resolved for a call resolves for a message. The family
    /// chat gets the handle-shaped id a member id can never be: a user id is
    /// a number, so `familyconnect:family` cannot collide with one.
    enum RecipientID {
        static let familyChat = "\(CallHandle.scheme):family"

        static func member(_ userID: Int64) -> String { CallHandle.value(userID: userID) }

        /// nil for anything that is not one of ours. A Shortcut left
        /// pointing at a member who deleted their account arrives here as
        /// an unknown id and is refused as one.
        static func recipient(from id: String) -> Recipient? {
            if id.caseInsensitiveCompare(familyChat) == .orderedSame { return .familyChat }
            guard let userID = CallHandle.userID(from: id) else { return nil }
            return .member(userID: userID)
        }
    }

    /// One row of the Shortcuts picker: the id to hand back, the name to
    /// choose by.
    struct Choice: Equatable, Sendable {
        let id: String
        let name: String
    }

    /// What the picker offers: the family chat first, where a scheduled
    /// message most often belongs, then the members this app can start a
    /// chat with. The roster arrives already gated by the caller — the
    /// member pickers' rule (not me, not left, not deleted, never the
    /// assistant) — because an id that outlived a membership must not be
    /// choosable at all.
    static func choices(familyChatTitle: String?, roster: [CallRequestRouter.Candidate]) -> [Choice] {
        var offered: [Choice] = []
        if let familyChatTitle, !familyChatTitle.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            offered.append(Choice(id: RecipientID.familyChat, name: familyChatTitle))
        }
        offered.append(contentsOf: roster.map { Choice(id: RecipientID.member($0.userID), name: $0.name) })
        return offered
    }
}
