//
//  Families.swift
//  FamilyConnect
//
//  Several families (docs/protocol.md, "Several families", 2026-10-22;
//  the decisions are docs/families-2026-10-22.md, D1–D8). One account may
//  belong to several families on one server, and the app shows ONE of them
//  at a time — the CURRENT family — with a switcher listing every family and
//  its unread count. Everything family-level (the roster, the board, the
//  pack, the owner's screens) is the current family's; chats say which
//  families they are listed in (`family_ids`), and a direct chat is listed
//  in every family its two people share.
//
//  Every DECISION about that lives here, as pure functions over values, so
//  the rules can be read and tested as tables: which family is current,
//  when the switcher shows, what it offers, which chats a family lists, how
//  many of them are unread, which family an upload or a push belongs to,
//  and what each new refusal says. The session, the coordinator and the
//  views only execute them.
//
//  An account in ONE family sees nothing of this — no switcher, no family
//  name added anywhere — except the two new ways in to a second family on
//  the Family screen (`offersNewFamily`, `offersJoinAnother`).
//

import Foundation

/// A list of family ids as the store keeps it — decimal ids joined by
/// commas — and the per-family roles of a roster row. Strings rather than
/// array columns for the reason `ChatEntity.groupMemberIDsText` gives: a
/// plain attribute migrates and compares without a transformer.
nonisolated enum FamilyIDList {

    static func encode(_ ids: [Int64]) -> String {
        ids.map(String.init).joined(separator: ",")
    }

    static func decode(_ text: String) -> [Int64] {
        text.split(separator: ",").compactMap { Int64($0) }
    }

    /// `[3: "owner", 7: "member"]` ↔ `"3=owner;7=member"`, ids ascending so
    /// one set of memberships always writes the same text.
    ///
    /// No family at all is `"-"`, never `""`: the empty string is "nobody
    /// has said" — a row from before several families — and the two must
    /// stay apart, because a row in no family is drawn as having left and a
    /// row nobody has described is left exactly as it was.
    static func encodeRoles(_ roles: [Int64: String]) -> String {
        guard !roles.isEmpty else { return noFamily }
        return roles.keys.sorted().map { "\($0)=\(roles[$0] ?? "")" }.joined(separator: ";")
    }

    /// The stored text of a roster row that is on no family's roster.
    static let noFamily = "-"

    static func decodeRoles(_ text: String) -> [Int64: String] {
        var roles: [Int64: String] = [:]
        for pair in text.split(separator: ";") {
            let parts = pair.split(separator: "=", maxSplits: 1)
            guard parts.count == 2, let id = Int64(parts[0]) else { continue }
            roles[id] = String(parts[1])
        }
        return roles
    }
}

nonisolated enum FamilyRules {

    // MARK: - What GET /me says

    /// The families a `/me` answer describes: `families` from a server that
    /// knows several, and — from one that predates them — the one `family`,
    /// wearing the `role` that arrived beside it. `[]` is an account in none.
    static func families(from me: MeResponse) -> [FamilyDTO] {
        if let families = me.families { return families }
        guard let family = me.family else { return [] }
        return [family.withRole(family.role ?? me.role)]
    }

    /// Whether this server predates several families, and must be spoken to
    /// about the home family alone, by the old paths: `families` is ALWAYS
    /// present on a server that knows them.
    static func isLegacy(_ me: MeResponse) -> Bool {
        me.families == nil
    }

    /// The caller's pending join requests: the list, or — from an older
    /// server — its one request.
    static func pendingRequests(from me: MeResponse) -> [PendingJoinRequestDTO] {
        if me.families != nil { return me.pendingJoinRequests }
        return me.pendingJoinRequest.map { [$0] } ?? []
    }

    // MARK: - The current family

    /// The family to show: the one this device last showed the account,
    /// while the account is still in it, and otherwise the first — the home
    /// family, which `/me` always lists first. Nil only for an account in
    /// none, which is the gate's business.
    static func currentFamilyID(stored: Int64?, families: [FamilyDTO]) -> Int64? {
        if let stored, families.contains(where: { $0.id == stored }) { return stored }
        return families.first?.id
    }

    /// The family id a request names in its path: the current family's own
    /// id, or nil — the word `mine` — on a server that predates several
    /// families and knows no other form.
    static func pathFamilyID(_ familyID: Int64?, legacy: Bool) -> Int64? {
        legacy ? nil : familyID
    }

    // MARK: - The switcher and the ways in

    /// "New Family…": only while the server lets members start families,
    /// the account owns none of its families (ownership that came by
    /// hand-off counts), and it is under the per-account cap.
    static func offersNewFamily(
        memberFamiliesEnabled: Bool, families: [FamilyDTO], maxFamilies: Int
    ) -> Bool {
        memberFamiliesEnabled
            && !families.contains { $0.role == "owner" }
            && families.count < maxFamilies
    }

    /// "Join Another Family…": while under the cap.
    static func offersJoinAnother(families: [FamilyDTO], maxFamilies: Int) -> Bool {
        families.count < maxFamilies
    }

    /// The switcher exists for an account in two or more families, or with
    /// something pending to show — a join request or an invitation. With one
    /// family and nothing pending, nothing on screen changes.
    static func showsSwitcher(familyCount: Int, pendingRequests: Int, invitations: Int) -> Bool {
        familyCount >= 2 || pendingRequests > 0 || invitations > 0
    }

    /// One family as the switcher lists it.
    struct SwitcherEntry: Equatable, Identifiable, Sendable {
        let id: Int64
        let name: String
        /// The unread of the chats listed in this family — a direct chat
        /// counts in every family that lists it.
        let unread: Int
        let isCurrent: Bool
    }

    /// One chat's share of the counting: which families list it, and how
    /// many of its messages are unread.
    struct ChatCount: Equatable, Sendable {
        let familyIDs: [Int64]
        let unread: Int
    }

    static func switcherEntries(
        families: [FamilyDTO], chats: [ChatCount], current: Int64?
    ) -> [SwitcherEntry] {
        families.map { family in
            SwitcherEntry(
                id: family.id,
                name: family.name,
                unread: unread(in: family.id, chats: chats),
                isCurrent: family.id == current)
        }
    }

    static func unread(in familyID: Int64, chats: [ChatCount]) -> Int {
        chats.filter { isListed(chatFamilyIDs: $0.familyIDs, in: familyID) }
            .reduce(0) { $0 + $1.unread }
    }

    // MARK: - Which chats a family lists

    /// A chat is shown in the current family when its `family_ids` name it.
    /// A chat that names none is from a server, or a store, that predates
    /// several families — there is one family, and it is listed in it. With
    /// no family known at all (an offline launch before the first `/me`),
    /// everything held is shown, as it always was.
    static func isListed(chatFamilyIDs: [Int64], in familyID: Int64?) -> Bool {
        guard let familyID, !chatFamilyIDs.isEmpty else { return true }
        return chatFamilyIDs.contains(familyID)
    }

    /// Whether a chat survives the account leaving `familyID`: a direct chat
    /// stays while its `family_ids` still name another family this device
    /// holds; everything else listed only there goes.
    static func survivesLeaving(
        chatFamilyIDs: [Int64], left familyID: Int64, held: Set<Int64>
    ) -> Bool {
        guard chatFamilyIDs.contains(familyID) else { return true }
        return chatFamilyIDs.contains { $0 != familyID && held.contains($0) }
    }

    // MARK: - Which family a request is for

    /// The family an upload for a message in this chat is for
    /// (`POST /attachments?family_id=`): the family on screen when the chat
    /// is listed there, else the first that lists it. A message may only
    /// use an attachment uploaded for a family in its chat's `family_ids`.
    static func uploadFamily(chatFamilyIDs: [Int64], current: Int64?) -> Int64? {
        guard !chatFamilyIDs.isEmpty else { return current }
        if let current, chatFamilyIDs.contains(current) { return current }
        return chatFamilyIDs.first
    }

    /// The family a tapped push opens in, or nil to stay where the app is.
    /// A push that names a family this device holds opens there; one that
    /// names none (a direct chat's) stays in the family on screen when the
    /// chat is listed there, else opens in the first family that lists it.
    static func pushFamily(
        pushFamilyID: Int64?, chatFamilyIDs: [Int64], current: Int64?, held: [Int64]
    ) -> Int64? {
        if let pushFamilyID {
            return held.contains(pushFamilyID) && pushFamilyID != current ? pushFamilyID : nil
        }
        if isListed(chatFamilyIDs: chatFamilyIDs, in: current) { return nil }
        return chatFamilyIDs.first { held.contains($0) }
    }

    /// The name a local notification titles a family-chat message with:
    /// THAT chat's family, from the family list by its first id — not "the"
    /// family's, which with several is whichever one happens to be on screen.
    static func familyName(chatFamilyIDs: [Int64], families: [FamilyDTO], fallback: String?) -> String? {
        if let first = chatFamilyIDs.first, let family = families.first(where: { $0.id == first }) {
            return family.name
        }
        return fallback
    }

    // MARK: - The owner's invitations

    /// Somebody the owner could invite: a member of one of the owner's
    /// OTHER families who is not in this one (docs/protocol.md, "Several
    /// families": "and nobody else on the server").
    struct Invitee: Equatable, Identifiable, Sendable {
        let id: Int64
        let name: String
        /// Already invited and not yet answered.
        let isInvited: Bool
    }

    /// One roster row as the picker reads it.
    struct RosterRow: Equatable, Sendable {
        let userID: Int64
        let name: String
        let families: Set<Int64>
        let accountDeleted: Bool
    }

    static func invitees(
        rows: [RosterRow], into familyID: Int64, currentUserID: Int64,
        pendingInvitedUserIDs: Set<Int64>
    ) -> [Invitee] {
        rows.filter { row in
            row.userID != currentUserID
                && !row.accountDeleted
                && !row.families.contains(familyID)
                && row.families.contains { $0 != familyID }
        }
        .sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
        .map { Invitee(id: $0.userID, name: $0.name, isInvited: pendingInvitedUserIDs.contains($0.userID)) }
    }

    /// The owner's reset of a member's password is offered only where it
    /// will work: the roster's `password_reset`, true when the owner owns
    /// every family the member is in. Absent — a server that predates
    /// several families — is the old rule, where it always worked.
    static func offersPasswordReset(_ passwordReset: Bool?) -> Bool {
        passwordReset ?? true
    }

    /// "Invite…" is offered only to an owner who has another family to
    /// invite from.
    static func offersInvite(isOwner: Bool, familyCount: Int) -> Bool {
        isOwner && familyCount >= 2
    }

    // MARK: - Leaving

    /// What the leave dialog says. One family keeps today's dialog and texts
    /// exactly (nil here); with several, it names the family and says what
    /// stays.
    static func leaveTitle(familyName: String, familyCount: Int) -> String? {
        guard familyCount >= 2 else { return nil }
        return String(localized: "Leave “\(familyName)”?")
    }

    static var leaveSeveralMessage: String {
        String(localized: "You'll lose access to this family's chats. Direct chats stay with anyone you share another family with.")
    }

    // MARK: - What a refusal says

    /// The words for the several-families refusals, or nil for any other
    /// error — the caller keeps its own copy for those.
    static func failureMessage(_ error: Error, maxFamilies: Int) -> String? {
        let code: String?
        switch error {
        case APIError.conflict(let conflictCode, _): code = conflictCode
        case APIError.forbidden(let forbiddenCode): code = forbiddenCode
        default: code = nil
        }
        switch code {
        case "too_many_families":
            return String(localized: "You've reached the most families one account can be in (\(maxFamilies)).")
        case "already_own_a_family":
            return String(localized: "You already own a family, so you can't start another.")
        case "member_families_disabled":
            return String(localized: "This server doesn't let members start new families.")
        case "member_in_other_families":
            return String(localized: "This member is also in a family you don't own, so only they can change their password.")
        case "invitation_not_pending":
            return String(localized: "This invitation is no longer available.")
        default:
            return nil
        }
    }

    /// `family_not_found`: the family is gone for this account (left on
    /// another device, removed, deleted) — treated as gone, by a resync.
    static func isFamilyGone(_ error: Error) -> Bool {
        if case APIError.notFound(let code) = error { return code == "family_not_found" }
        return false
    }
}
