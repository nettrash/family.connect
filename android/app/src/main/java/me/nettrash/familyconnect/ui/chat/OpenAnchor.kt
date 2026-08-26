/*
 * OpenAnchor.kt
 * Family Connect (Android)
 *
 * WHERE A CHAT OPENS. Pure arithmetic, no Compose, no Android, no
 * coroutines — so the whole decision table is unit-testable without a
 * view, which matters more here than anywhere else on this screen: get
 * it wrong and the divider lands above the wrong message, or a chat the
 * reader never looked at is marked read on every device they own.
 *
 * The rule (nettrash's choice: ALWAYS jump whenever there is anything
 * unread — no product threshold; the cap below is a technical floor,
 * not a preference):
 *
 *   1. nothing unread                    -> Newest
 *   2. MARKER branch (myLastReadId > 0)  -> the OLDEST cached row with
 *      a server id above the marker that somebody else sent.
 *   3. COUNT-BACK branch (marker == 0, a fresh install or a re-login)
 *      -> walk newest-first, SKIPPING my own rows and rows with no
 *      server id yet, counting to unreadCount. That mirrors the
 *      server's own predicate (push_payload.rs,
 *      `build_message_unread_query`: newer than the marker, not sent by
 *      me) — the same predicate MessageDao.countInboundAfter carries.
 *   4. nothing resolves (an empty cache, fewer cached inbound rows than
 *      the count, a hole) -> Newest
 *   5. the target sits further back than [cap] -> Newest. MANDATORY:
 *      the screen reaches an anchor by paging the render window older,
 *      and that loop is bounded. Above the cap there is nothing to
 *      scroll to, exactly as the quote jump gives up.
 *
 * The marker branch is PREFERRED and the count-back branch is only the
 * fallback for a marker of 0. The two are never reconciled by min/max:
 * where they would disagree the right answer is to give up, never to
 * guess.
 *
 * The count and the rows are TWO DIFFERENT INSTANTS (docs/protocol.md,
 * `GET /chats`: `unread_count` and `last_read_message_id` come from one
 * row, but the messages come from another request and from live
 * frames). The marker branch therefore CROSS-CHECKS: it anchors only
 * when the number of cached rows above the marker is exactly the count.
 * A disagreement in either direction means one of the two instants is
 * stale, and a divider drawn from a stale instant sits above the wrong
 * message — which is rule 4's give-up, not something to compensate for.
 *
 * The result is captured ONCE at open and never recomputed: the count
 * is zeroed and the marker advances the moment the reader reaches the
 * bottom, so anything derived every pass would delete the divider out
 * from under them.
 *
 * iOS counterpart: the same table beside the follow rules in
 * ios/FamilyConnect/Views/ConversationView.swift.
 */

package me.nettrash.familyconnect.ui.chat

import me.nettrash.familyconnect.data.db.AnchorRow

/** Where the thread should be parked when the screen finishes opening. */
sealed interface OpenAnchor {

    /** At the bottom, the way every chat opened before this feature. */
    data object Newest : OpenAnchor

    /**
     * At [serverId] — the oldest message the reader has not seen — with
     * a "[newCount] new messages" divider directly above it.
     */
    data class Message(val serverId: Long, val newCount: Int) : OpenAnchor
}

/**
 * Decide where a chat opens. See the file header for the table; the
 * parameters are all snapshots taken at open.
 *
 * @param unreadCount the chat row's count, as `GET /chats` last wrote it
 * @param myLastReadId my own read marker, 0 when nothing was ever read
 * @param cachedNewestFirst what this device holds, in MessageDao's order
 * @param myUserId me — my own messages are never unread
 * @param cap how far back the screen can actually scroll
 */
fun openAnchor(
    unreadCount: Int,
    myLastReadId: Long,
    cachedNewestFirst: List<AnchorRow>,
    myUserId: Long,
    cap: Int,
): OpenAnchor {
    if (unreadCount <= 0) return OpenAnchor.Newest

    val targetIndex = if (myLastReadId > 0L) {
        markerTarget(unreadCount, myLastReadId, cachedNewestFirst, myUserId)
    } else {
        countBackTarget(unreadCount, cachedNewestFirst, myUserId)
    }
    if (targetIndex < 0) return OpenAnchor.Newest
    // The give-up rule 5. Never widen to reach a target instead: the
    // page loop that gets there is bounded on its own count, and a
    // window that grows to meet an anchor grows without limit.
    if (targetIndex >= cap) return OpenAnchor.Newest
    val serverId = cachedNewestFirst[targetIndex].serverId ?: return OpenAnchor.Newest
    return OpenAnchor.Message(serverId = serverId, newCount = unreadCount)
}

/**
 * The oldest cached row above the marker, or -1.
 *
 * Returns -1 as well when the rows and the count disagree — see the
 * file header on the two instants.
 */
private fun markerTarget(
    unreadCount: Int,
    myLastReadId: Long,
    cachedNewestFirst: List<AnchorRow>,
    myUserId: Long,
): Int {
    var oldest = -1
    var above = 0
    cachedNewestFirst.forEachIndexed { index, row ->
        val id = row.serverId
        if (id != null && id > myLastReadId && row.senderId != myUserId) {
            above++
            // Newest-first, so the LAST match is the oldest one.
            oldest = index
        }
    }
    if (above != unreadCount) return -1
    return oldest
}

/**
 * The [unreadCount]-th inbound acked row walking back from the newest,
 * or -1 when this device does not hold that many.
 */
private fun countBackTarget(
    unreadCount: Int,
    cachedNewestFirst: List<AnchorRow>,
    myUserId: Long,
): Int {
    var counted = 0
    cachedNewestFirst.forEachIndexed { index, row ->
        // A pending outbound row has no id the server ever counted, and
        // my own messages are never unread to me. Skipping both is what
        // makes this walk the server's predicate rather than a guess.
        if (row.serverId == null || row.senderId == myUserId) return@forEachIndexed
        counted++
        if (counted == unreadCount) return index
    }
    return -1
}
