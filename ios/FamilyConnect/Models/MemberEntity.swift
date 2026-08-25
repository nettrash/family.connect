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

    /// The wire shape, for the screens that take a DTO (the password
    /// reset sheet and the birthday editor, both shared with iOS). A
    /// roster row IS a member — this is a spelling difference, not a
    /// fetch.
    var dto: MemberDTO {
        MemberDTO(
            id: userID,
            username: username,
            displayName: displayName,
            role: role,
            avatarVersion: avatarVersion,
            birthday: birthday)
    }

    init(
        userID: Int64,
        username: String,
        displayName: String,
        role: String,
        isCurrentUser: Bool,
        hasLeft: Bool = false,
        avatarVersion: Int64 = 0,
        birthday: BirthdayDTO? = nil
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
    }
}
