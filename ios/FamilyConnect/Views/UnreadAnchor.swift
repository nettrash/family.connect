//
//  UnreadAnchor.swift
//  FamilyConnect
//
//  WHERE A CHAT OPENS when there is something in it the reader has not
//  seen: at the OLDEST unread message, under an "N new messages" divider,
//  rather than at the newest — which is where every other open lands.
//
//  It lives out here beside ThreadFollow, and for the same reason: every
//  rule below decides where a scroll view is LOOKING, which is the one
//  thing no test can ask a scroll view afterwards. As a clause inside a
//  `.task` inside a `ScrollViewReader` it would be reachable only by
//  running the app and squinting at it. As arithmetic it is a table.
//
//  THE RESULT IS CAPTURED ONCE, AT OPEN, INTO VIEW STATE, AND NEVER
//  RECOMPUTED. Both of its inputs move while the reader reads — the count
//  is zeroed by the read path the moment they reach the bottom, and the
//  marker advances behind it — so an anchor (or a divider) derived on
//  every pass would vanish out from under the person it was drawn for, at
//  exactly the moment they arrived at the thing it was pointing at.
//
//  TWO BRANCHES, AND THEY ARE NOT RECONCILED:
//
//  - The MARKER branch is preferred and is the ordinary case. The server's
//    `last_read_message_id` (protocol.md, GET /chats) is this user's own
//    read marker, monotonic and shared across their devices, so the oldest
//    cached inbound message ABOVE it is precisely the oldest thing they
//    have not seen — no counting, and no dependence on the count agreeing.
//    It is an id THRESHOLD and not a reference: retention may long since
//    have swept the message it names, so it is only ever compared against.
//
//  - The COUNT-BACK branch is the fallback for a marker of `0`, which is a
//    real answer and not a missing one: a fresh install, or a re-login.
//    It walks newest-first counting only what the SERVER counts — skipping
//    rows this user sent, and rows with no server id yet (a locally
//    pending outbound message the server has never seen) — so the row it
//    lands on is the row the server's own predicate would land on.
//
//  Where the two would disagree the right answer is to GIVE UP to
//  `.newest`, never to guess and never to reconcile them with a min or a
//  max. A divider one row out of place is a lie about what somebody has
//  read, told in a chat where the read marker is permanent.
//
//  THE CAP IS A HANG FIX AND NOT A PREFERENCE. Both threads render a
//  bounded, NON-lazy suffix of the cache (see MacConversationView's
//  header: two captured hang reports, 36 of 38 main-thread samples inside
//  `LazyStack.measureEstimates`, ending in a Force Quit). Reaching an
//  unread message a thousand rows back would mean laying out everything
//  after it in one pass. Past the cap this rule gives up and the chat
//  opens at its newest message — the same way the quote jump gives up, and
//  the same way Android's bounded page loop gives up.
//

import Foundation

nonisolated enum UnreadAnchor {

    /// One cached row, reduced to the only two facts the rule needs.
    ///
    /// `serverID` is nil for a message this device has sent and the server
    /// has not acknowledged. Those rows are invisible to this rule in both
    /// branches: they are mine, they are not counted by the server, and
    /// they have no id to compare against a marker.
    struct Row: Equatable, Sendable {
        let serverID: Int64?
        let senderID: Int64

        init(serverID: Int64?, senderID: Int64) {
            self.serverID = serverID
            self.senderID = senderID
        }
    }

    /// Where the thread should open.
    ///
    /// `.message` carries a SERVER id rather than a local one because that
    /// is what both branches are arithmetic about; the view maps it to the
    /// row's `localID` at scroll time, exactly as the quote jump does.
    enum Target: Equatable, Sendable {
        case newest
        case message(Int64)
    }

    /// Rows kept ABOVE the anchored row.
    ///
    /// Not cosmetic: landing the target against the top sentinel fires a
    /// history page whose own scroll-restore fights the anchoring scroll,
    /// and the two of them together leave the reader somewhere neither
    /// intended. It is the phone's quote-jump margin, which exists for
    /// precisely this and is the same number for the same reason.
    static let margin = 15

    /// How wide the render window has to be to show a row `distance` back
    /// from the newest one, WITH the margin above it. Compared against the
    /// cap by the rule below, and used by the view to widen before it
    /// scrolls.
    static func rowsToRender(distanceFromNewest: Int) -> Int {
        distanceFromNewest + 1 + margin
    }

    /// - Parameters:
    ///   - unreadCount: the chat's unread count as the store holds it —
    ///     server-authoritative, plus any live messages counted since.
    ///   - myLastReadID: this user's own read marker, `0` when they have
    ///     never reported reading anything here.
    ///   - cachedNewestFirst: the cached rows, NEWEST FIRST. May be a
    ///     suffix: anything further back than `cap` is refused anyway, so
    ///     the caller is free to hand over only what could possibly win.
    ///   - myUserID: the reader.
    ///   - cap: the most rows the thread may render at once.
    static func openAnchor(
        unreadCount: Int,
        myLastReadID: Int64,
        cachedNewestFirst: [Row],
        myUserID: Int64,
        cap: Int
    ) -> Target {
        // 1. Nothing unread: the ordinary open, at the newest message.
        guard unreadCount > 0 else { return .newest }

        // Only a row the SERVER knows about and somebody ELSE sent can be
        // unread — which is the server's own predicate, spelled here so
        // the two cannot drift. (An assistant reply is somebody else: it
        // comes from a reserved account, it counts as unread, and its row
        // can legitimately still be empty while it streams.)
        var inbound: [(distance: Int, serverID: Int64)] = []
        for (distance, row) in cachedNewestFirst.enumerated() {
            guard let serverID = row.serverID, row.senderID != myUserID else { continue }
            inbound.append((distance, serverID))
        }

        let hit: (distance: Int, serverID: Int64)?
        if myLastReadID > 0 {
            // 2. MARKER branch. The oldest cached row above the marker is
            // the oldest unread one, whatever the count says.
            let above = inbound.filter { $0.serverID > myLastReadID }
            // 4. …unless the cache does not actually HOLD them all, which
            // is the one disagreement between the count and the rows that
            // is worth detecting: it means the oldest unread message is
            // older than anything downloaded, so any row picked here would
            // be the wrong one. Give up rather than point at it.
            guard above.count >= unreadCount else { return .newest }
            hit = above.last
        } else {
            // 3. COUNT-BACK branch, for a marker of 0. Walk newest-first
            // over exactly what the server counts and stop on the Nth.
            // 4. Fewer cached inbound rows than the count means a hole, a
            // short cache, or an empty one — all of which are the same
            // answer.
            guard inbound.count >= unreadCount else { return .newest }
            hit = inbound[unreadCount - 1]
        }

        guard let hit else { return .newest }
        // 5. Beyond the render cap: give up. MANDATORY — see the header.
        guard rowsToRender(distanceFromNewest: hit.distance) <= cap else { return .newest }
        // 6. Otherwise, open there.
        return .message(hit.serverID)
    }
}
