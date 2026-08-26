/*
 * ScrollToNewestTest.kt
 * Family Connect (Android)
 *
 * The decision table for WHEN THE SCROLL-TO-NEWEST BUTTON SHOWS, pinned
 * without a view. Pure JUnit on purpose — no Robolectric, no coroutines,
 * no Compose: every one of these cases is a way the button can lie
 * (flashing mid-open, hiding behind screens of tall bubbles), and none
 * of them needs a screen to be wrong.
 */

package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class ScrollToNewestTest {

    // -- The settled gate -----------------------------------------------------

    @Test
    fun hiddenBeforeTheScreenSettlesEvenFarUpTheThread() {
        // An anchored open parks the list well off the bottom while the
        // opening scroll is still in flight — the button must not blink.
        assertThat(showScrollToNewest(settled = false, firstVisibleItemIndex = 30)).isFalse()
    }

    @Test
    fun hiddenBeforeTheScreenSettlesAtTheBottomToo() {
        // The empty-list first frame: index 0 on EVERY chat's first frame.
        assertThat(showScrollToNewest(settled = false, firstVisibleItemIndex = 0)).isFalse()
    }

    // -- At the newest message ------------------------------------------------

    @Test
    fun hiddenAtTheNewestMessage() {
        assertThat(showScrollToNewest(settled = true, firstVisibleItemIndex = 0)).isFalse()
    }

    @Test
    fun hiddenOneMessageOffTheBottom() {
        // index <= 1 is the read gate's "at the newest message" on this
        // reverseLayout thread — the button uses the very same line.
        assertThat(showScrollToNewest(settled = true, firstVisibleItemIndex = 1)).isFalse()
    }

    // -- Meaningfully off the bottom ------------------------------------------

    @Test
    fun shownAsSoonAsThePositionStopsCountingAsAtNewest() {
        assertThat(showScrollToNewest(settled = true, firstVisibleItemIndex = 2)).isTrue()
    }

    @Test
    fun shownFarUpTheThread() {
        assertThat(showScrollToNewest(settled = true, firstVisibleItemIndex = 250)).isTrue()
    }

    @Test
    fun shownAtAnAnchoredOpenOnceSettled() {
        // The whole point of the anchored open: the reader IS off the
        // bottom, so the way back down is on offer from the first
        // settled frame.
        assertThat(showScrollToNewest(settled = true, firstVisibleItemIndex = 30)).isTrue()
    }
}
