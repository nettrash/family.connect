//
//  MemberEntity.swift
//  FamilyConnect
//
//  The family roster, mirrored locally so sender names on bubbles resolve
//  without a network hop. Rows are upserted from GET /families/mine during
//  resync and patched live by `member_joined` / `member_left` frames.
//
//  A member who leaves (or is removed) is NOT deleted — `hasLeft` flips to
//  true instead. Their old bubbles still need a display name, and the
//  protocol retains history across leave/rejoin, so throwing the row away
//  would orphan every message they ever sent. Left members are simply
//  filtered out of pickers (New Chat) and the members list.
//
//  An account that was DELETED is the same idea taken one step further:
//  `accountDeleted` flips instead, the row keeps only what it takes to put
//  a name on old messages, and `GET /families/mine` keeps sending it in a
//  second array (`former_members`) for exactly that purpose. Such a row is
//  stored here beside the live roster — one place, as the protocol asks —
//  and every screen that lists PEOPLE filters it out.
//

import Foundation
import SwiftData

@Model
final class MemberEntity {
    /// Server user id — the natural key; upserts match on it.
    @Attribute(.unique) var userID: Int64
    var username: String
    var displayName: String
    /// "owner" | "member" (wire values, kept as String).
    var role: String
    /// True for the signed-in user's own row, so pickers can exclude self
    /// without threading the current user id through every view.
    var isCurrentUser: Bool
    /// True once the member left / was removed. Kept for name resolution
    /// on old bubbles; hidden from pickers and the roster UI.
    var hasLeft: Bool = false

    /// True once the account behind this row was deleted (protocol.md,
    /// "Deleting an account"). Defaulted, so this is a lightweight
    /// SwiftData migration for an existing store.
    ///
    /// NOT named `isDeleted`, and that is load-bearing: a `@Model` is
    /// backed by an `NSManagedObject`, which already has an `isDeleted`
    /// of its own (is this object deleted from its context). A property
    /// of that name collides with it, and the failure is silent — the
    /// build is clean, the write appears to happen, and every read comes
    /// back `false` for ever. Leave the name alone.
    ///
    /// A deleted account is not a member: it holds no role, has no
    /// picture and no birthday, can never be signed into again, and is
    /// never offered as somebody to chat with. What it still is, is the
    /// sender of messages, notes and reactions the family can still see —
    /// which is the whole reason the row survives at all.
    var accountDeleted: Bool = false

    /// `0` = no profile picture. Defaulted, so this is a lightweight
    /// SwiftData migration for anyone upgrading over an existing store.
    var avatarVersion: Int64 = 0

    /// A day and a month, or neither. Two defaulted Optionals rather than
    /// one stored struct for the reason `avatarVersion` is defaulted: a
    /// property with a default IS the lightweight migration, and an
    /// existing store must open without one being written by hand.
    ///
    /// They are only ever read through `birthday` below, which insists on
    /// both — "month but no day" is not a state the protocol has and not
    /// one any screen should have to draw.
    var birthdayMonth: Int?
    var birthdayDay: Int?

    /// Which of this device's families list this person, and in what role
    /// — `"3=owner;7=member"` (docs/protocol.md, "Several families",
    /// 2026-10-22). ONE row per person, because a person is one account:
    /// the name, the picture and the birthday are the account's and are
    /// shown in every family. What is per family is only this.
    ///
    /// `role` and `hasLeft` above are kept as the PROJECTION of this onto
    /// the family on screen (`project(onto:)`), so every roster, picker and
    /// predicate that reads them reads the current family without knowing
    /// there is more than one. "" is "nobody said" — a row from before.
    var familyRolesText: String = ""
    /// The families whose owner's roster said `password_reset: false` for
    /// this person, as comma-joined ids: the families where this reader may
    /// NOT reset their password (docs/protocol.md, "Several families").
    /// Stored as the refusal so that a server that predates the flag —
    /// which never refuses — leaves the reset offered as it always was.
    var passwordResetDeniedText: String = ""

    /// The pair, when it is a pair.
    var birthday: BirthdayDTO? {
        guard let birthdayMonth, let birthdayDay else { return nil }
        return BirthdayDTO(month: birthdayMonth, day: birthdayDay)
    }

    /// What a screen actually draws for this person.
    ///
    /// The stored `displayName` of a deleted account is the server's
    /// ENGLISH placeholder ("Deleted account"), and the protocol says a
    /// client that understands the flag SHOULD draw its own translation
    /// instead — so every name a view puts on screen goes through here
    /// rather than reading `displayName` directly.
    /// family id → role, for every family of this device that lists them.
    var familyRoles: [Int64: String] {
        get { FamilyIDList.decodeRoles(familyRolesText) }
        set { familyRolesText = FamilyIDList.encodeRoles(newValue) }
    }

    /// Whether this person is on `familyID`'s roster as this device holds it.
    func isMember(of familyID: Int64) -> Bool {
        familyRoles[familyID] != nil
    }

    /// Re-point `role` and `hasLeft` at one family: what a switch does to
    /// every row, and what every roster write does to the rows it touched.
    /// A deleted account is in nothing and stays so.
    func project(onto familyID: Int64) {
        // A row nobody has described yet (from before several families) is
        // left exactly as it was; the first roster read describes it.
        guard !accountDeleted, !familyRolesText.isEmpty else { return }
        // Somebody not on this family's roster keeps the role they last
        // had: the row is drawn as having left, and nothing reads the role
        // of a row that has.
        if let newRole = familyRoles[familyID] {
            if role != newRole { role = newRole }
            if hasLeft { hasLeft = false }
        } else if !hasLeft {
            hasLeft = true
        }
    }

    /// Whether the owner reading `familyID` may reset this person's password.
    func passwordResetAllowed(in familyID: Int64) -> Bool {
        !FamilyIDList.decode(passwordResetDeniedText).contains(familyID)
    }

    var resolvedDisplayName: String {
        accountDeleted ? MemberDisplay.deletedAccountName : displayName
    }

    /// The wire shape, for the screens that take a DTO (the password
    /// reset sheet and the birthday editor, both shared with iOS). A
    /// roster row IS a member — this is a spelling difference, not a
    /// fetch.
    var dto: MemberDTO {
        MemberDTO(
            id: userID,
            username: username,
            displayName: displayName,
            // A deleted account carries no role on the wire and must not
            // be given one back on the way out.
            role: accountDeleted ? nil : role,
            avatarVersion: avatarVersion,
            birthday: birthday,
            deleted: accountDeleted)
    }

    init(
        userID: Int64,
        username: String,
        displayName: String,
        role: String,
        isCurrentUser: Bool,
        hasLeft: Bool = false,
        avatarVersion: Int64 = 0,
        birthday: BirthdayDTO? = nil,
        accountDeleted: Bool = false
    ) {
        self.userID = userID
        self.username = username
        self.displayName = displayName
        self.role = role
        self.isCurrentUser = isCurrentUser
        self.hasLeft = hasLeft
        self.avatarVersion = avatarVersion
        self.birthdayMonth = birthday?.month
        self.birthdayDay = birthday?.day
        self.accountDeleted = accountDeleted
    }
}

/// The one place the app spells a deleted account's name.
///
/// The server sends the English placeholder "Deleted account" as that
/// row's `display_name` so a client that knows nothing of the flag still
/// draws something honest; a client that DOES know it draws this instead
/// (protocol.md, Objects → User). Localised, and looked up per read
/// rather than stored, so it follows the device's language rather than
/// freezing whatever it was when the tombstone arrived.
nonisolated enum MemberDisplay {
    static var deletedAccountName: String {
        String(
            localized: "Deleted account",
            comment: "Stands in for the name of somebody who deleted their account.")
    }
}
