//
//  MemberCap.swift
//  FamilyConnect
//
//  The owner's member cap, as arithmetic rather than as two copies of the
//  same `min(max(...))` in two view files.
//
//  Shared because the phone and the Mac draw the same control and must
//  agree about it: an owner who sets a limit on one and sees a different
//  number on the other has been told the setting is unreliable. The
//  strings stay in the views — the catalogue already guarantees those are
//  the same words — but the branching that CHOOSES between them is here,
//  where it can be tested once.
//

import Foundation

enum MemberCap {

    /// What the footer says, which is three different sentences.
    enum State: Equatable {
        /// No cap of the owner's own; the operator's ceiling is what binds.
        case openToCeiling(ceiling: Int)
        /// The cap is at or below the current roster. Legal and deliberate:
        /// an owner who inherits a large family must still be able to shut
        /// the door, and the cap is read at the join door and never
        /// enforced over the room — so NOBODY is removed
        /// (docs/protocol.md, `PATCH /families/mine`).
        case frozen(memberCount: Int)
        /// Room to spare.
        case room(memberCount: Int, cap: Int)
    }

    static func state(cap: Int?, memberCount: Int, ceiling: Int) -> State {
        guard let cap else { return .openToCeiling(ceiling: ceiling) }
        return cap <= memberCount
            ? .frozen(memberCount: memberCount)
            : .room(memberCount: memberCount, cap: cap)
    }

    /// The cap to propose when the owner first turns the limit on: freeze
    /// the family where it stands, which is what reaching for "limit
    /// members" almost always means in the moment.
    ///
    /// Clamped at BOTH ends. The floor is 1 because the protocol's range
    /// starts there and an empty roster would otherwise propose 0; the
    /// ceiling because an operator may lower `limits.max_family_members`
    /// under a family that is already larger than the new ceiling, and a
    /// stepper seeded above its own bound is a control that opens invalid.
    static func seed(memberCount: Int, ceiling: Int) -> Int {
        clamp(memberCount, ceiling: ceiling)
    }

    /// A cap the owner typed or stepped to, held inside 1...ceiling.
    static func clamp(_ value: Int, ceiling: Int) -> Int {
        min(max(value, 1), max(ceiling, 1))
    }
}
