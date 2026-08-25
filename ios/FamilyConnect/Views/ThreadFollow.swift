//
//  ThreadFollow.swift
//  FamilyConnect
//
//  The arithmetic a bottom-anchored thread runs to decide two things: whether
//  the newest message is on screen, and how many rows to render.
//
//  It lives outside the view for the same reason `ComposerPriming` does. Every
//  rule here has been got wrong at least once, each time in a way no test
//  could see, because the rule was a clause inside a `guard` inside a closure
//  inside a `ScrollViewReader` — reachable only by running the app and looking
//  at it. Pulled out, it is arithmetic, and arithmetic can be pinned.
//
//  Used by the Mac's thread (MacConversationView). The phone's
//  (ConversationView) still spells the same rules inline; it has far more
//  re-pin hooks around them, so its versions have never been the thing that
//  broke. Moving it over is a separate change, and one that should be made by
//  someone with a phone in their hand.
//

import Foundation

enum ThreadFollow {

    /// How far ABOVE the viewport's top edge the bottom sentinel may sit and
    /// still count as on screen. Nothing legitimate puts it there — a bottom
    /// sentinel is the last thing in the content — so this is only slack for
    /// a layout pass caught mid-flight.
    static let aboveSlack: CGFloat = 32

    /// How far BELOW the viewport's bottom edge it may sit. Larger than the
    /// phone's 32 because the Mac's content stack carries 12pt of bottom
    /// padding, so a thread scrolled fully to the end parks the sentinel that
    /// much further down than the phone's does.
    static let belowSlack: CGFloat = 40

    /// Is the newest message on screen?
    ///
    /// - Parameters:
    ///   - sentinelMinY: the bottom sentinel's `minY` in `.scrollView` space.
    ///   - viewportHeight: the scroll view's visible height, or nil when there
    ///     is no scroll geometry to read yet.
    ///
    /// A nil viewport answers NO, and that direction is not arbitrary. This
    /// value decides whether a conversation is READ; the server's read marker
    /// is monotonic, so a read reported by mistake is permanent and reaches
    /// every device the person owns (see ChatPresence). A badge that lingers
    /// costs a glance. A badge cleared too early costs the message.
    static func isAtNewest(sentinelMinY: CGFloat, viewportHeight: CGFloat?) -> Bool {
        guard let viewportHeight else { return false }
        return sentinelMinY >= -aboveSlack && sentinelMinY <= viewportHeight + belowSlack
    }

    /// Should the thread scroll to its newest message now that one has
    /// arrived?
    ///
    /// Two reasons, and they are not the same reason. A reader already at the
    /// bottom stays there — following a conversation is what being at the
    /// bottom MEANS. And a member who pressed Send goes there from wherever
    /// they were, because pressing Send and watching nothing happen is worse
    /// than losing your place in history.
    ///
    /// `didSend` is DECLARED by the send, never inferred from which row sorts
    /// last: the optimistic row carries this device's clock while acked rows
    /// carry the server's, the assistant answers a mention with a row of its
    /// own, and an upload's row lands seconds after the click. All three make
    /// "the newest row is mine" false at moments when "I just sent" is true.
    static func followsArrival(isAtNewest: Bool, didSend: Bool) -> Bool {
        isAtNewest || didSend
    }

    /// How many rows to render after `arrived` new messages landed.
    ///
    /// Only widens for a reader who is NOT at the newest message: for them
    /// the window must grow, or an arriving message would slide the suffix
    /// out from under the history they are reading. For a reader AT the
    /// newest message widening buys nothing — the suffix already ends at the
    /// new row — and it costs a second content change one update later, which
    /// moves the thread twice for one message.
    ///
    /// Never shrinks, never exceeds `cap`, and never exceeds what is cached.
    /// The cap is the whole point of having a window: the stack that renders
    /// it is NON-lazy, so this is a real bound on main-thread layout work and
    /// not a hint. Past it the window SLIDES, keeping the newest rows.
    static func windowAfterArrival(
        current: Int, cached: Int, arrived: Int, isAtNewest: Bool, cap: Int
    ) -> Int {
        guard arrived > 0, !isAtNewest else { return current }
        return min(min(cached, current + arrived), cap)
    }

    /// How many rows to render after the reader reached the top sentinel and
    /// asked for more.
    ///
    /// Returns `current` unchanged when the cap already holds it there. The
    /// caller must treat that as "nothing to show" and do nothing else:
    /// widening by zero and then restoring the reading position anyway put
    /// the oldest rendered row back at the top of a window that had not
    /// grown, which simply threw the reader up there — and the top sentinel,
    /// still on screen, asked again.
    static func windowAfterPagingBack(current: Int, cached: Int, step: Int, cap: Int) -> Int {
        min(min(cached, current + step), cap)
    }
}
