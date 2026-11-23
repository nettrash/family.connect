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
//  SEVERAL FAMILIES (docs/protocol.md, "Several families"). An account may
//  be in more than one family, and the app shows one of them at a time —
//  but a Shortcut runs with no window, so "the family on screen" means
//  nothing to it. Everything here is therefore EVERY family's: each family
//  chat this device holds is its own recipient, named by its chat id, and a
//  member is reachable while any family this device holds lists them
//  (`isOnAHeldRoster`). Reading the on-screen projection instead (`hasLeft`)
//  posted "the family chat" into whichever family's chat the store returned
//  first, and refused a member of the other family as unknown until the
//  person switched back.
//

import Foundation

nonisolated enum MessageIntentPlan {

    /// Who the Shortcut named.
    enum Recipient: Equatable, Sendable {
        /// One family's own chat — the ordinary target for a scheduled
        /// "good morning" or a daily reminder. By chat id, because an
        /// account in several families has one of these per family.
        case familyChat(chatID: Int64)
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
        /// That family chat is not on this device — the account left the
        /// family, or the store has never synced it.
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
        /// Whether this chat id is a family chat this device holds — of
        /// any of its families, not only the one on screen.
        var isFamilyChat: (Int64) -> Bool
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
        case .familyChat(let chatID):
            guard directory.isFamilyChat(chatID) else { return .refuse(.noFamilyChat) }
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

    // MARK: - Who is reachable

    /// Whether a roster row is somebody this device can message: on the
    /// roster of ANY family it holds, not only the one on screen.
    ///
    /// `MemberEntity.hasLeft` is the projection onto the current family
    /// (`project(onto:)`), so a member of the other family reads as having
    /// left — right for a roster screen, wrong for a Shortcut, which has no
    /// screen. The per-family roles are the whole truth: a family this
    /// account leaves is taken out of every row's roles (`dropFamily`), so
    /// a row whose roles name no family (`"-"`) is on no roster here. A row
    /// nobody has described yet (`""`, from before several families) has
    /// only the projection to go by, and keeps it.
    static func isOnAHeldRoster(familyRolesText: String, hasLeft: Bool) -> Bool {
        guard !familyRolesText.isEmpty else { return !hasLeft }
        return !FamilyIDList.decodeRoles(familyRolesText).isEmpty
    }

    // MARK: - The picker

    /// The id a Shortcuts picker hands back, and the recipient behind it.
    ///
    /// A member's id IS their handle (CallHandle) — the very string Siri
    /// carries for a call, so one identity is minted in one place and a
    /// person who resolved for a call resolves for a message. A family chat
    /// gets a handle-shaped id a member id can never be,
    /// `familyconnect:family:<chat id>`: what follows the scheme is not a
    /// number, so `CallHandle` reads it as no member at all. The chat id,
    /// not the family id, because it is what the send needs and what the
    /// store answers for every family alike, a server from before several
    /// families included.
    enum RecipientID {
        static let familyChatPrefix = "\(CallHandle.scheme):family:"

        static func familyChat(_ chatID: Int64) -> String { "\(familyChatPrefix)\(chatID)" }

        static func member(_ userID: Int64) -> String { CallHandle.value(userID: userID) }

        /// nil for anything that is not one of ours. A Shortcut left
        /// pointing at a member who deleted their account arrives here as
        /// an unknown id and is refused as one.
        static func recipient(from id: String) -> Recipient? {
            let trimmed = id.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            if trimmed.hasPrefix(familyChatPrefix) {
                guard let chatID = Int64(trimmed.dropFirst(familyChatPrefix.count)), chatID > 0
                else { return nil }
                return .familyChat(chatID: chatID)
            }
            guard let userID = CallHandle.userID(from: id) else { return nil }
            return .member(userID: userID)
        }
    }

    /// A family chat the picker can offer: its id for the send, its title
    /// (the family's name) to choose by.
    struct FamilyChat: Equatable, Sendable {
        let chatID: Int64
        let title: String
    }

    /// One row of the Shortcuts picker: the id to hand back, the name to
    /// choose by.
    struct Choice: Equatable, Sendable {
        let id: String
        let name: String
    }

    /// What the picker offers: the family chats first — one per family this
    /// device holds, in the order given (the home family's first) — where a
    /// scheduled message most often belongs, then the members this app can
    /// start a chat with. The roster arrives already gated by the caller —
    /// not me, not deleted, on a roster this device holds
    /// (`isOnAHeldRoster`), never the assistant — because an id that
    /// outlived a membership must not be choosable at all.
    static func choices(familyChats: [FamilyChat], roster: [CallRequestRouter.Candidate]) -> [Choice] {
        var offered: [Choice] = []
        for chat in familyChats where !chat.title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            offered.append(Choice(id: RecipientID.familyChat(chat.chatID), name: chat.title))
        }
        offered.append(contentsOf: roster.map { Choice(id: RecipientID.member($0.userID), name: $0.name) })
        return offered
    }
}
