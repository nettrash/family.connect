//
//  ThreadFollowTests.swift
//  FamilyConnectTests
//
//  The Mac thread's follow rules, pinned as arithmetic.
//
//  Each of these has been a live bug. They are here rather than in a UI test
//  because none of them is visible from outside: the thread's TOTAL height and
//  its accessibility tree are identical whether the rule fires or not — the
//  only difference is where the scroll view is looking, which is the one thing
//  a test cannot ask it.
//

import Foundation
import Testing

@testable import FamilyConnect

@Suite("Thread follow rules")
struct ThreadFollowTests {

    // MARK: - Is the newest message on screen?

    @Test("A thread scrolled to its end reports the newest message on screen")
    func sentinelAtTheEnd() {
        // Where the Mac's sentinel lands when the scroll view is at its
        // maximum offset: one point tall, above 12pt of content padding.
        #expect(ThreadFollow.isAtNewest(sentinelMinY: 600 - 13, viewportHeight: 600))
    }

    @Test("The band reaches a little past the bottom edge, and then stops")
    func belowTheBottomEdge() {
        #expect(ThreadFollow.isAtNewest(sentinelMinY: 600 + ThreadFollow.belowSlack,
                                        viewportHeight: 600))
        #expect(!ThreadFollow.isAtNewest(sentinelMinY: 600 + ThreadFollow.belowSlack + 1,
                                         viewportHeight: 600))
    }

    @Test("A sentinel far ABOVE the viewport is not on screen")
    func aboveTheTopEdge() {
        // The bound this rule used to be missing entirely: with only an upper
        // limit, a sentinel scrolled hundreds of points above the viewport
        // satisfied `minY <= height + slack` and reported the newest message
        // as visible — which marks the conversation read.
        #expect(ThreadFollow.isAtNewest(sentinelMinY: -ThreadFollow.aboveSlack,
                                        viewportHeight: 600))
        #expect(!ThreadFollow.isAtNewest(sentinelMinY: -ThreadFollow.aboveSlack - 1,
                                         viewportHeight: 600))
        #expect(!ThreadFollow.isAtNewest(sentinelMinY: -400, viewportHeight: 600))
    }

    @Test("No scroll geometry answers NO, never yes")
    func noViewport() {
        // The safe direction, and not a matter of taste: this value marks a
        // conversation read, and the server's read marker is monotonic.
        #expect(!ThreadFollow.isAtNewest(sentinelMinY: 0, viewportHeight: nil))
        #expect(!ThreadFollow.isAtNewest(sentinelMinY: 587, viewportHeight: nil))
    }

    // MARK: - Following an arrival

    @Test("A reader at the newest message follows the next one")
    func followsWhenAtNewest() {
        #expect(ThreadFollow.followsArrival(isAtNewest: true, didSend: false))
    }

    @Test("A reader in history is left where they are")
    func staysInHistory() {
        #expect(!ThreadFollow.followsArrival(isAtNewest: false, didSend: false))
    }

    @Test("Sending goes to the new message from anywhere in history")
    func ownSendAlwaysFollows() {
        // The case the old `messages.last?.senderID == currentUserID` test
        // could answer wrongly: it asked which row sorts last, and the
        // optimistic row carries this device's clock while acked rows carry
        // the server's. Declaring the send cannot be wrong about it.
        #expect(ThreadFollow.followsArrival(isAtNewest: false, didSend: true))
        #expect(ThreadFollow.followsArrival(isAtNewest: true, didSend: true))
    }

    // MARK: - The render window on arrival

    @Test("A reader at the newest message does not widen the window")
    func noWidenWhilePinned() {
        // The suffix already ends at the arriving row, so widening shows
        // nothing new — and lands one update later, moving the thread a
        // second time for one message.
        #expect(ThreadFollow.windowAfterArrival(
            current: 60, cached: 101, arrived: 1, isAtNewest: true, cap: 300) == 60)
    }

    @Test("A reader in history widens instead of having the window slide")
    func widensWhileReadingHistory() {
        #expect(ThreadFollow.windowAfterArrival(
            current: 60, cached: 101, arrived: 1, isAtNewest: false, cap: 300) == 61)
        #expect(ThreadFollow.windowAfterArrival(
            current: 60, cached: 105, arrived: 5, isAtNewest: false, cap: 300) == 65)
    }

    @Test("The window never passes the cap, and past it the suffix slides")
    func respectsTheCap() {
        #expect(ThreadFollow.windowAfterArrival(
            current: 300, cached: 4000, arrived: 1, isAtNewest: false, cap: 300) == 300)
        #expect(ThreadFollow.windowAfterArrival(
            current: 298, cached: 4000, arrived: 5, isAtNewest: false, cap: 300) == 300)
    }

    @Test("The window never renders more rows than are cached")
    func neverPassesTheCache() {
        #expect(ThreadFollow.windowAfterArrival(
            current: 60, cached: 12, arrived: 1, isAtNewest: false, cap: 300) == 12)
    }

    @Test("Nothing arriving changes nothing")
    func noArrival() {
        #expect(ThreadFollow.windowAfterArrival(
            current: 60, cached: 100, arrived: 0, isAtNewest: false, cap: 300) == 60)
        // Deletions must not shrink the window out from under the reader.
        #expect(ThreadFollow.windowAfterArrival(
            current: 60, cached: 99, arrived: -1, isAtNewest: false, cap: 300) == 60)
    }

    // MARK: - The render window when paging back

    @Test("Paging back grows the window by one step")
    func pagingBackWidens() {
        #expect(ThreadFollow.windowAfterPagingBack(
            current: 60, cached: 500, step: 60, cap: 300) == 120)
    }

    @Test("At the cap, paging back returns the window unchanged")
    func pagingBackAtTheCap() {
        // The caller reads this as "nothing to show" and does nothing else.
        // Widening by zero and restoring the reading position anyway put the
        // oldest rendered row back at the TOP of a window that had not grown,
        // which threw the reader up there — and the top sentinel, still on
        // screen, asked again straight away.
        #expect(ThreadFollow.windowAfterPagingBack(
            current: 300, cached: 4000, step: 60, cap: 300) == 300)
    }

    @Test("Paging back stops at what is cached")
    func pagingBackStopsAtTheCache() {
        #expect(ThreadFollow.windowAfterPagingBack(
            current: 60, cached: 80, step: 60, cap: 300) == 80)
    }

    // MARK: - The jump-to-newest button

    @Test("The button is up exactly when a settled thread is away from the newest message")
    func jumpButtonShowsAwayFromNewest() {
        #expect(ThreadFollow.showsJumpToNewest(isAtNewest: false, hasSettled: true))
        #expect(!ThreadFollow.showsJumpToNewest(isAtNewest: true, hasSettled: true))
    }

    @Test("Nothing shows before the opening routine has settled")
    func jumpButtonWaitsForSettle() {
        // Both threads spend their opening window with `isAtNewest` at
        // whatever their state defaults to — the phone's false, the Mac's
        // optimistic true — so without the settle gate the button blinks
        // on and off through every open, and after an anchored open it
        // would blink at the one moment it is genuinely needed.
        #expect(!ThreadFollow.showsJumpToNewest(isAtNewest: false, hasSettled: false))
        #expect(!ThreadFollow.showsJumpToNewest(isAtNewest: true, hasSettled: false))
    }

    // MARK: - What a re-run of the opening routine still owes

    @Test("A finished open is not opened again")
    func openingIsDoneOnceItHasSettled() {
        // The reader owns the thread's position from here. Re-running the
        // opening scroll would throw somebody who has been reading for five
        // minutes back to a boundary they passed long ago.
        #expect(ThreadFollow.openingStep(hasSettled: true, hasAnchor: true) == .done)
        // `hasAnchor: false` is the ordinary open (it resolves to `.newest`,
        // which the phone stores and the Mac does too) — settled is settled
        // either way.
        #expect(ThreadFollow.openingStep(hasSettled: true, hasAnchor: false) == .done)
    }

    @Test("A first pass decides and places")
    func openingDecidesOnTheFirstPass() {
        #expect(ThreadFollow.openingStep(hasSettled: false, hasAnchor: false)
            == .decideAndPlace)
    }

    @Test("An open interrupted after deciding keeps its answer and finishes the scroll")
    func openingResumesAnInterruptedPlacement() {
        // THE CASE THIS RULE EXISTS FOR. The placement spans ~400 ms of
        // yields; tapping an attachment inside it cancels the task with the
        // anchor already decided and the scroll not yet run — an anchor
        // present, nothing settled. Both threads used to key the whole
        // routine on the anchor alone and so did NOTHING here: on the phone
        // `hasSettled` then stayed false for as long as the chat was open,
        // and since `publishPresence` ANDs it, that conversation could never
        // be marked read again; on the Mac the settle step runs after the
        // placement, so the same re-appear settled a thread still at the
        // BOTTOM and read messages nobody had seen.
        #expect(ThreadFollow.openingStep(hasSettled: false, hasAnchor: true)
            == .placeOnly)
    }
}
