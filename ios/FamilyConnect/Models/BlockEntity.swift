//
//  BlockEntity.swift
//  FamilyConnect
//
//  One member this account has decided not to read (docs/protocol.md,
//  "Blocking a member").
//
//  A SEPARATE ENTITY, deliberately, rather than a flag on MemberEntity.
//  Three reasons, and the first is structural:
//
//  1. `upsertMember` rewrites username, displayName, role, isCurrentUser,
//     hasLeft and avatarVersion on EVERY roster upsert. A flag living there
//     would be silently reset by any resync — the same failure the
//     birthday's double-Optional exists to avoid — and the bug would show
//     up as "the block came back off after I opened the app", days later
//     and nowhere near the code that caused it.
//  2. A block outlives the membership. It survives either party leaving the
//     family and the two of them meeting again, so it has to be able to
//     name somebody who is in no roster at all — which a row on the roster
//     cannot.
//  3. It matches the wire: a top-level array of ids, not a field on Member.
//     The server cannot put it on Member either, because whether one member
//     has blocked another depends on WHO IS READING, and that object is
//     serialised once and sent to many.
//
//  The whole set is REPLACED from `blocked_user_ids` on every `GET /me` and
//  `GET /families/mine` — it is a complete state-set and never a delta —
//  and the `member_blocked` frame applies the same way, as a state-set with
//  a boolean rather than an event.
//
//  Android counterpart: BlockEntity in data/db/Entities.kt.
//

import Foundation
import SwiftData

@Model
final class BlockEntity {
    /// The blocked member's server id — the natural key, and the whole row.
    ///
    /// There is nothing else to store: the blocker is always this account,
    /// and *when* the block was set is a fact no screen shows and the wire
    /// does not carry.
    @Attribute(.unique) var userID: Int64

    init(userID: Int64) {
        self.userID = userID
    }
}
