/*
 * OpenAnchorTest.kt
 * Family Connect (Android)
 *
 * The decision table for WHERE A CHAT OPENS, pinned without a view.
 * Pure JUnit on purpose — no Robolectric, no coroutines, no Compose:
 * every one of these cases is a way the divider can land above the
 * wrong message, and none of them needs a screen to be wrong.
 */

package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.db.AnchorRow
import org.junit.Test

class OpenAnchorTest {

    private companion object {
        const val ME = 7L
        const val PEER = 9L

        /** The real one, so a "beyond the cap" case is the real distance. */
        const val CAP = 250
    }

    /** Newest-first, the way MessageDao hands rows over. */
    private fun rows(vararg ids: Pair<Long?, Long>) =
        ids.map { (serverId, senderId) -> AnchorRow(serverId, senderId) }

    private fun inbound(vararg serverIds: Long) = serverIds.map { AnchorRow(it, PEER) }

    // -- Nothing to anchor at -------------------------------------------------

    @Test
    fun nothingUnreadOpensAtTheNewestMessage() {
        assertThat(
            openAnchor(
                unreadCount = 0,
                myLastReadId = 100,
                cachedNewestFirst = inbound(103, 102, 101),
                myUserId = ME,
                cap = CAP,
            ),
        ).isEqualTo(OpenAnchor.Newest)
    }

    @Test
    fun anEmptyCacheOpensAtTheNewestMessage() {
        assertThat(
            openAnchor(
                unreadCount = 3,
                myLastReadId = 100,
                cachedNewestFirst = emptyList(),
                myUserId = ME,
                cap = CAP,
            ),
        ).isEqualTo(OpenAnchor.Newest)
    }

    // -- The marker branch ----------------------------------------------------

    @Test
    fun theMarkerBranchAnchorsAtTheOldestMessageAboveTheMarker() {
        val anchor = openAnchor(
            unreadCount = 3,
            myLastReadId = 100,
            // Newest-first; 100 and below are already read.
            cachedNewestFirst = inbound(103, 102, 101, 100, 99),
            myUserId = ME,
            cap = CAP,
        )
        assertThat(anchor).isEqualTo(OpenAnchor.Message(serverId = 101, newCount = 3))
    }

    @Test
    fun theMarkerBranchIgnoresMyOwnMessagesAboveTheMarker() {
        // I answered in the middle of their run: 102 is mine, so the
        // unread ones are 103 and 101 and the divider belongs above 101.
        val anchor = openAnchor(
            unreadCount = 2,
            myLastReadId = 100,
            cachedNewestFirst = rows(103L to PEER, 102L to ME, 101L to PEER, 100L to PEER),
            myUserId = ME,
            cap = CAP,
        )
        assertThat(anchor).isEqualTo(OpenAnchor.Message(serverId = 101, newCount = 2))
    }

    @Test
    fun theMarkerBranchIgnoresLocallyPendingOutboundRows() {
        // A message of mine still in flight sits at the very top of the
        // list with no server id at all. It is neither unread nor
        // anchorable.
        val anchor = openAnchor(
            unreadCount = 2,
            myLastReadId = 100,
            cachedNewestFirst = rows(null to ME, 102L to PEER, 101L to PEER, 100L to PEER),
            myUserId = ME,
            cap = CAP,
        )
        assertThat(anchor).isEqualTo(OpenAnchor.Message(serverId = 101, newCount = 2))
    }

    /**
     * The cache does not reach back far enough — the server counts three
     * unread and this device holds one of them. Anchoring at the one it
     * holds would draw the divider under two messages it belongs above.
     */
    @Test
    fun fewerCachedRowsAboveTheMarkerThanTheCountGivesUp() {
        assertThat(
            openAnchor(
                unreadCount = 3,
                myLastReadId = 100,
                cachedNewestFirst = inbound(101, 100, 99),
                myUserId = ME,
                cap = CAP,
            ),
        ).isEqualTo(OpenAnchor.Newest)
    }

    /**
     * The other direction, and the same answer: the count and the rows
     * are two different instants, and when they disagree one of them is
     * stale. Reading on another device deflates the count without
     * touching this device's marker, and the divider drawn from the
     * stale half sits above messages that have already been read.
     */
    @Test
    fun moreCachedRowsAboveTheMarkerThanTheCountGivesUp() {
        assertThat(
            openAnchor(
                unreadCount = 1,
                myLastReadId = 100,
                cachedNewestFirst = inbound(103, 102, 101, 100),
                myUserId = ME,
                cap = CAP,
            ),
        ).isEqualTo(OpenAnchor.Newest)
    }

    /**
     * A marker at all means the marker branch, even where counting back
     * would have produced an answer. The two are never reconciled: where
     * they disagree the right move is to give up, not to guess which one
     * is telling the truth.
     */
    @Test
    fun aMarkerIsPreferredToCountingBackEvenWhenItGivesUp() {
        // Marker says one message is above it; the count says three.
        // Counting back would happily land on 4.
        assertThat(
            openAnchor(
                unreadCount = 3,
                myLastReadId = 5,
                cachedNewestFirst = inbound(6, 5, 4, 3),
                myUserId = ME,
                cap = CAP,
            ),
        ).isEqualTo(OpenAnchor.Newest)
    }

    // -- The count-back branch (a fresh install, or a re-login) ---------------

    @Test
    fun withNoMarkerTheCountWalksBackFromTheNewest() {
        val anchor = openAnchor(
            unreadCount = 3,
            myLastReadId = 0,
            cachedNewestFirst = inbound(60, 59, 58, 57, 56),
            myUserId = ME,
            cap = CAP,
        )
        assertThat(anchor).isEqualTo(OpenAnchor.Message(serverId = 58, newCount = 3))
    }

    @Test
    fun countingBackSkipsMyOwnMessagesAndPendingRows() {
        // The server counts neither, so neither may be counted here or
        // the walk stops one or two rows short of where it should.
        val anchor = openAnchor(
            unreadCount = 3,
            myLastReadId = 0,
            cachedNewestFirst = rows(
                null to ME,
                60L to PEER,
                59L to ME,
                58L to PEER,
                57L to ME,
                56L to PEER,
                55L to PEER,
            ),
            myUserId = ME,
            cap = CAP,
        )
        assertThat(anchor).isEqualTo(OpenAnchor.Message(serverId = 56, newCount = 3))
    }

    @Test
    fun countingBackGivesUpWhenThisDeviceHoldsFewerRowsThanTheCount() {
        assertThat(
            openAnchor(
                unreadCount = 5,
                myLastReadId = 0,
                cachedNewestFirst = inbound(60, 59, 58),
                myUserId = ME,
                cap = CAP,
            ),
        ).isEqualTo(OpenAnchor.Newest)
    }

    @Test
    fun countingBackGivesUpWhenEveryCachedRowIsMine() {
        assertThat(
            openAnchor(
                unreadCount = 1,
                myLastReadId = 0,
                cachedNewestFirst = rows(60L to ME, 59L to ME),
                myUserId = ME,
                cap = CAP,
            ),
        ).isEqualTo(OpenAnchor.Newest)
    }

    // -- The cap --------------------------------------------------------------

    /**
     * MANDATORY, and not a product threshold: the screen reaches an
     * anchor by paging its render window older, and that loop is
     * bounded. A target past the end of it is a scroll that would
     * silently never happen — so the chat opens at the newest message
     * instead, exactly as the quote jump gives up.
     */
    @Test
    fun aTargetBeyondTheCapGivesUp() {
        // 300 unread, all of them held: the oldest sits at index 299.
        val ids = (300L downTo 1L).toList().toLongArray()
        assertThat(
            openAnchor(
                unreadCount = 300,
                myLastReadId = 0,
                cachedNewestFirst = inbound(*ids),
                myUserId = ME,
                cap = CAP,
            ),
        ).isEqualTo(OpenAnchor.Newest)
    }

    @Test
    fun theLastRowInsideTheCapStillAnchors() {
        val ids = (CAP.toLong() downTo 1L).toList().toLongArray()
        assertThat(
            openAnchor(
                unreadCount = CAP,
                myLastReadId = 0,
                cachedNewestFirst = inbound(*ids),
                myUserId = ME,
                cap = CAP,
            ),
        ).isEqualTo(OpenAnchor.Message(serverId = 1, newCount = CAP))
    }
}
