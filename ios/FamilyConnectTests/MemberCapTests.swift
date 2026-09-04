//
//  MemberCapTests.swift
//  FamilyConnectTests
//
//  The owner's member cap: the arithmetic the phone and the Mac share, so
//  that a limit set on one reads the same on the other.
//

import Testing
@testable import FamilyConnect

@Suite("Member cap")
struct MemberCapTests {

    // MARK: - What the footer says

    @Test("no cap of your own points at the operator's ceiling")
    func noCap() {
        #expect(MemberCap.state(cap: nil, memberCount: 4, ceiling: 50)
            == .openToCeiling(ceiling: 50))
    }

    @Test("room to spare counts seats")
    func room() {
        #expect(MemberCap.state(cap: 8, memberCount: 4, ceiling: 50)
            == .room(memberCount: 4, cap: 8))
    }

    /// The boundary is the interesting one: a cap EQUAL to the roster is
    /// already a freeze, because the cap is read at the join door and the
    /// next arrival would exceed it.
    @Test("a cap at or below the roster is a freeze, not an error")
    func frozen() {
        #expect(MemberCap.state(cap: 4, memberCount: 4, ceiling: 50)
            == .frozen(memberCount: 4))
        // Below: an owner who inherited a large family and shut the door.
        // Legal and deliberate — nobody is removed by it.
        #expect(MemberCap.state(cap: 2, memberCount: 6, ceiling: 50)
            == .frozen(memberCount: 6))
        // And one more seat is NOT a freeze, so the boundary is real
        // rather than an always-frozen predicate.
        #expect(MemberCap.state(cap: 5, memberCount: 4, ceiling: 50)
            == .room(memberCount: 4, cap: 5))
    }

    // MARK: - Seeding and clamping

    @Test("turning the limit on freezes the family where it stands")
    func seedFreezesAtCurrentSize() {
        #expect(MemberCap.seed(memberCount: 4, ceiling: 50) == 4)
    }

    /// An operator may lower `limits.max_family_members` under a family
    /// that is already larger. Seeding at the roster would then open the
    /// stepper above its own bound.
    @Test("the seed never exceeds the operator's ceiling")
    func seedClampsToCeiling() {
        #expect(MemberCap.seed(memberCount: 60, ceiling: 50) == 50)
    }

    /// The protocol's range starts at 1, so an empty roster must not
    /// propose 0.
    @Test("the seed never falls below one")
    func seedClampsToOne() {
        #expect(MemberCap.seed(memberCount: 0, ceiling: 50) == 1)
    }

    @Test("a stepped value is held inside 1...ceiling")
    func clamping() {
        #expect(MemberCap.clamp(7, ceiling: 50) == 7)
        #expect(MemberCap.clamp(0, ceiling: 50) == 1)
        #expect(MemberCap.clamp(-3, ceiling: 50) == 1)
        #expect(MemberCap.clamp(99, ceiling: 50) == 50)
        #expect(MemberCap.clamp(50, ceiling: 50) == 50)
        // A degenerate ceiling still yields a legal cap rather than a
        // range the stepper would trap on.
        #expect(MemberCap.clamp(5, ceiling: 0) == 1)
    }
}
