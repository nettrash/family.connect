//
//  GroupRules.swift
//  FamilyConnect
//
//  Groups (docs/protocol.md, "Groups", decided 2026-10-22 — the assessment
//  and its four decisions are docs/groups-2026-10-21.md): a chat of kind
//  "group" inside one family, with a name and an explicit member list. Inside
//  it is the family chat with fewer people, so most of what this file says is
//  WHICH of the family chat's rules a group shares — and the three it does
//  not: calls, the "seen" tick, and whose consent a transcript needs.
//
//  Pure logic and no view, for the reason AssistantConsent gives: the phone's
//  composer, the Mac's, the thread sheet and both chat lists ask the same
//  questions, and one of them answering differently is a poll button that
//  appears in one window and not the other. `ChatRules` is the table the
//  tests pin (GroupRulesTests); every view asks it rather than spelling a
//  kind out.
//
//  Android counterpart: model/GroupRules.kt.
//

import Foundation

/// What a chat OFFERS, by its kind.
nonisolated enum ChatRules {
    static let family = "family"
    static let direct = "direct"
    static let ai = "ai"
    static let group = "group"

    /// The family chat's surface: the family chat itself, or a group — "the
    /// family chat with fewer people", where every rule the protocol states
    /// for the family chat applies unless it says otherwise. nil (a chat not
    /// held yet) and an unknown future kind are neither.
    static func isFamilyOrGroup(_ kind: String?) -> Bool {
        kind == family || kind == group
    }

    /// Sender names above other people's bubbles — wherever more than one
    /// other person can be talking.
    static func showsSenderNames(_ kind: String?) -> Bool { isFamilyOrGroup(kind) }

    /// `@name` mentions of members (docs/protocol.md, "Mentioning a member").
    /// In a group the candidates are THAT GROUP's members (`mentionRoster`).
    static func offersMemberMentions(_ kind: String?) -> Bool { isFamilyOrGroup(kind) }

    /// Starting a poll (docs/protocol.md, "Polls", amended 2026-10-22), and
    /// the open-polls button that finds them again.
    static func offersPolls(_ kind: String?) -> Bool { isFamilyOrGroup(kind) }

    /// `@ai`, "Ask the Assistant" and the assistant at the top of the `@`
    /// list. A group's mention reads THAT GROUP alone, under the family
    /// owner's switches (D3); whether this server has an assistant to offer
    /// is a separate question the composer asks beside this one.
    static func offersAssistantMention(_ kind: String?) -> Bool { isFamilyOrGroup(kind) }

    /// Voice notes and the video-message door: every chat but the
    /// assistant's own, which has no microphone (#79, S1.5).
    static func offersRecordings(_ kind: String?) -> Bool { kind != ai }

    /// A call lives in a direct chat and nowhere else: the protocol has no
    /// group calls (`invalid_call`).
    static func offersCalls(_ kind: String?) -> Bool { kind == direct }

    /// The "seen" tick on one's own bubbles. Never where several people
    /// read: `othersReadUpTo` is a max over all of them and means nobody
    /// (`MessagePresentation.isRead`).
    static func drawsSeen(_ kind: String?) -> Bool { !isFamilyOrGroup(kind) }

    /// Whose say-so another member's recording needs before its text can be
    /// asked for (docs/protocol.md, "Transcripts on request").
    enum TranscriptRule: Equatable, Sendable {
        /// That sender's consent alone — a direct chat, and since 2026-10-22
        /// a group (D4: the owner's `ai_transcripts` is about the family chat
        /// and does not reach a group, which the owner cannot see).
        case sendersConsent
        /// The owner's `ai_transcripts` switch AND that sender's consent.
        case ownersSwitch
        /// Never — the assistant's own chat holds nobody else's recordings.
        case never
    }

    static func transcriptRule(_ kind: String?) -> TranscriptRule {
        switch kind {
        case direct, group: .sendersConsent
        case family: .ownersSwitch
        default: .never
        }
    }

    /// The members a mention may name here: the whole live roster in the
    /// family chat, and in a group only those of it who are in THAT GROUP —
    /// the server refuses anybody else with `validation`, and they would not
    /// even receive the message. The reader and the blocked are taken out
    /// later, by `MemberMentions.suggestions`, exactly as in the family chat.
    static func mentionRoster(
        _ roster: [MentionDTO], kind: String?, groupMemberIDs: [Int64]
    ) -> [MentionDTO] {
        guard kind == group else { return roster }
        let inGroup = Set(groupMemberIDs)
        return roster.filter { inGroup.contains($0.userID) }
    }

    /// How many people a poll could hear from — the "of N" in "4 of 5
    /// voted". The family's live roster in the family chat; the group's
    /// members in a group, whom the server keeps to current members of the
    /// family.
    static func pollAudience(kind: String?, groupMemberIDs: [Int64], liveFamilyMembers: Int) -> Int {
        kind == group ? groupMemberIDs.count : liveFamilyMembers
    }
}

/// Who may do what to a group (docs/protocol.md, "Groups" — "Who may do
/// what"; D1: the creator is the admin).
nonisolated enum GroupRules {
    enum Action: CaseIterable, Equatable, Sendable {
        case rename
        case addMembers
        case removeMember
        case delete
        case leave
    }

    /// A group's name, trimmed, is 1 to this many characters.
    static let maxTitleLength = 64
    /// The most groups one family may hold (`too_many_groups`).
    static let maxGroupsPerFamily = 50

    /// May `userID` do `action` here? Only a member may do anything at all;
    /// leaving is every member's, the rest is the admin's. The server is the
    /// one that decides — this decides what is OFFERED, so nobody is shown a
    /// button that answers `not_group_admin`.
    static func may(_ action: Action, userID: Int64, adminUserID: Int64?, memberIDs: [Int64]) -> Bool {
        guard userID > 0, memberIDs.contains(userID) else { return false }
        switch action {
        case .leave: return true
        case .rename, .addMembers, .removeMember, .delete: return userID == adminUserID
        }
    }

    /// May the reader remove THIS member? The admin, and never themself —
    /// their own way out is Leave, which hands the group on.
    static func mayRemove(
        _ memberID: Int64, userID: Int64, adminUserID: Int64?, memberIDs: [Int64]
    ) -> Bool {
        memberID != userID && memberIDs.contains(memberID)
            && may(.removeMember, userID: userID, adminUserID: adminUserID, memberIDs: memberIDs)
    }

    /// The name as it would be stored — trimmed, and 1 to 64 characters — or
    /// nil when the server would refuse it as `validation`. Counted in
    /// Unicode scalars, which is what the server's `chars().count()` counts.
    static func validTitle(_ raw: String) -> String? {
        let title = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        let count = title.unicodeScalars.count
        guard count >= 1, count <= maxTitleLength else { return nil }
        return title
    }

    /// Why a New Group form cannot be sent yet, checked in the order a
    /// person fills it in; nil when it can.
    static func creationProblem(title: String, memberIDs: Set<Int64>) -> String? {
        if validTitle(title) == nil {
            return String(localized: "A group's name can be 1 to 64 characters.")
        }
        if memberIDs.isEmpty {
            return String(localized: "Choose at least one member.")
        }
        return nil
    }

    /// Why a create did not happen, as a person is told it.
    ///
    /// A failure that never got an answer is NOT a refusal (docs/protocol.md,
    /// "Transient or terminal"): it says the server could not be reached, and
    /// the form keeps everything so the same tap tries again.
    static func createFailure(_ error: Error) -> String {
        switch error as? APIError {
        case .conflict(code: "too_many_groups"?, message: _):
            return String(localized: "Your family already has 50 groups.")
        case .conflict(code: "validation"?, message: _):
            return String(localized: "A group's name can be 1 to 64 characters.")
        case let failure? where failure.isTransient:
            return String(localized: "Can't reach the server. Try again.")
        default:
            return String(localized: "Couldn't create the group.")
        }
    }

    /// Why a rename, add, remove, leave or delete did not happen.
    static func changeFailure(_ error: Error) -> String {
        switch error as? APIError {
        case .forbidden(code: "not_group_admin"?):
            return String(localized: "Only the group's admin can do this.")
        case let failure? where failure.isTransient:
            return String(localized: "Can't reach the server. Try again.")
        default:
            return String(localized: "Couldn't change the group.")
        }
    }
}
