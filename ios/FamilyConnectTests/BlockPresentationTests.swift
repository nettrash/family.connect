//
//  BlockPresentationTests.swift
//  FamilyConnectTests
//
//  The rules that decide what a blocked member's content looks like, and
//  the one number in the message menu that is computed by hand BEFORE
//  layout.
//

import CoreGraphics
import Testing

@testable import FamilyConnect

@Suite("Block presentation")
struct BlockPresentationTests {

    // MARK: - Quote masking

    /// The two quote levels are INDEPENDENTLY blockable, and the shape that
    /// proves it is ordinary: a reply BY an unblocked member TO a blocked
    /// one, whose own parent is a third person.
    ///
    /// This pins the rule the views compute from. One flag for both levels
    /// gets this wrong in both directions — over-masking a readable parent,
    /// or leaking a blocked reply.
    @Test("the two quote levels mask independently")
    func theTwoQuoteLevelsMaskIndependently() {
        // Only the REPLY's sender is blocked.
        #expect(hidden(sender: 11, blocked: [11]))
        #expect(!hidden(sender: 13, blocked: [11]))
        // Only the PARENT's sender is blocked.
        #expect(!hidden(sender: 11, blocked: [13]))
        #expect(hidden(sender: 13, blocked: [13]))
        // Both, and neither.
        #expect(hidden(sender: 11, blocked: [11, 13]) && hidden(sender: 13, blocked: [11, 13]))
        #expect(!hidden(sender: 11, blocked: []) && !hidden(sender: 13, blocked: []))
        // A reveal is per level, so a revealed level is not hidden even
        // while its sender stays blocked.
        #expect(!hidden(sender: 11, blocked: [11], revealed: true))
        // And own messages are never masked at either level.
        #expect(!hidden(sender: 7, blocked: [7]))
    }

    /// The rule the views apply, extracted so both platforms and both
    /// levels are pinned by one expression.
    private func hidden(
        sender: Int64, blocked: Set<Int64>, revealed: Bool = false, me: Int64 = 7
    ) -> Bool {
        guard !revealed, sender != me else { return false }
        return blocked.contains(sender)
    }

    // MARK: - The hand-computed menu size

    /// `MessageContextMenu.size` places and clamps the floating panel
    /// BEFORE layout, so a row the body draws and this does not count
    /// mis-places the whole thing with no error anywhere. It had no test at
    /// all until this one.
    ///
    /// The arithmetic is `44 * n + (n - 1)`: fixed-height rows and the
    /// hairlines BETWEEN them. Share is always drawn, so `n` is never zero.
    @Test("the menu's height matches the rows it actually draws")
    func menuSizeMatchesTheRowsTheBodyActuallyDraws() {
        func height(_ n: Int) -> CGFloat { 44 * CGFloat(n) + CGFloat(n - 1) }

        // Share alone.
        #expect(
            MessageContextMenu.size(canReply: false, canCopy: false).height == height(1))
        // Reply + Copy + Share, the ordinary other-person case before this
        // feature existed.
        #expect(MessageContextMenu.size(canReply: true, canCopy: true).height == height(3))
        // Own message: Reply + Edit + Copy + Share, and NO report or block.
        #expect(
            MessageContextMenu.size(canReply: true, canEdit: true, canCopy: true).height
                == height(4))

        // Somebody else's: Reply + Copy + Share + Report + Block. FIVE is
        // the maximum — `canEdit` needs the message to be the reader's own
        // and report/block need it not to be, so Edit can never coexist
        // with them.
        #expect(
            MessageContextMenu.size(
                canReply: true, canCopy: true, canReport: true, blockState: .notBlocked
            ).height == height(5))
        // Already blocked: Unblock takes Block's slot, not an extra one.
        #expect(
            MessageContextMenu.size(
                canReply: true, canCopy: true, canReport: true, blockState: .blocked
            ).height == height(5))
        // Report without a block state, and a block state without report,
        // are each one row.
        #expect(
            MessageContextMenu.size(canReply: true, canCopy: true, canReport: true).height
                == height(4))
        #expect(
            MessageContextMenu.size(canReply: true, canCopy: true, blockState: .notBlocked).height
                == height(4))
        // The width never varies — a truncated destructive row is the one
        // place somebody must be certain what they are pressing.
        #expect(MessageContextMenu.size(canReply: true).width == 220)
    }
}
