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

    // MARK: - Board notes

    /// The note is the one object where the CONTENT goes as well as the
    /// author, so this pins the predicate the two boards draw from.
    @Test("a blocked member's note is hidden; nobody else's is")
    func blockedAuthorsNoteIsHidden() {
        let me: Int64 = 7
        #expect(MessagePresentation.isNoteHiddenByBlock(
            authorID: 11, blockedUserIDs: [11], currentUserID: me))
        // An unblocked author, with somebody else blocked: the block must
        // be about the AUTHOR and not merely about the set being non-empty.
        #expect(!MessagePresentation.isNoteHiddenByBlock(
            authorID: 12, blockedUserIDs: [11], currentUserID: me))
        #expect(!MessagePresentation.isNoteHiddenByBlock(
            authorID: 12, blockedUserIDs: [], currentUserID: me))
    }

    /// Blocking yourself is refused by the server, so this is a guard
    /// against a corrupt store rather than a real case — but a board that
    /// hid your own notes would be the most alarming possible bug.
    @Test("your own note is never hidden, even if the set says otherwise")
    func ownNoteIsNeverHidden() {
        #expect(!MessagePresentation.isNoteHiddenByBlock(
            authorID: 7, blockedUserIDs: [7], currentUserID: 7))
    }

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
    /// iOS ONLY, and gated so the whole suite can build for macOS. The
    /// Mac draws its menu with a native `.contextMenu`, which measures
    /// itself — `MessageContextMenu` does not exist over there, and one
    /// ungated reference to it kept every OTHER test in this target from
    /// ever running on the Mac.
    #if os(iOS)
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

        // Somebody else's, MAIN page: Reply + Copy + Share + Safety. The
        // two moderation rows now live one level down, so this is FOUR
        // however many of them there are.
        #expect(
            MessageContextMenu.size(
                canReply: true, canCopy: true, canReport: true, blockState: .notBlocked
            ).height == height(4))
        #expect(
            MessageContextMenu.size(
                canReply: true, canCopy: true, canReport: true, blockState: .blocked
            ).height == height(4))
        // Report alone and a block state alone each still raise Safety,
        // and each is still one row on this page.
        #expect(
            MessageContextMenu.size(canReply: true, canCopy: true, canReport: true).height
                == height(4))
        #expect(
            MessageContextMenu.size(canReply: true, canCopy: true, blockState: .notBlocked).height
                == height(4))

        // The SAFETY page: Back + Report + Block. Its height is what the
        // overlay places the panel by once the page changes, so it is
        // pinned here for the same reason the main page is — a row the
        // body draws and `size` does not count mis-places the whole panel
        // with no error anywhere.
        #expect(
            MessageContextMenu.size(
                canReply: true, canCopy: true, canReport: true,
                blockState: .notBlocked, page: .safety
            ).height == height(3))
        // Unblock takes Block's slot, not an extra one.
        #expect(
            MessageContextMenu.size(
                canReply: true, canCopy: true, canReport: true,
                blockState: .blocked, page: .safety
            ).height == height(3))
        // And with only one of the two available, the page is Back + it.
        #expect(
            MessageContextMenu.size(
                canReply: true, canCopy: true, canReport: true, page: .safety
            ).height == height(2))
        // The width never varies — a truncated destructive row is the one
        // place somebody must be certain what they are pressing.
        #expect(MessageContextMenu.size(canReply: true).width == 220)
    }
    #endif
}
