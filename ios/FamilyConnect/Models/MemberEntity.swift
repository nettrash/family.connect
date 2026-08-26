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
