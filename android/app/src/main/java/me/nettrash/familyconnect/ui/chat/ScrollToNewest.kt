/*
 * ScrollToNewest.kt
 * Family Connect (Android)
 *
 * The one rule for WHEN THE SCROLL-TO-NEWEST BUTTON SHOWS, pinned
 * without a view — same shape as OpenAnchor.
 */

package me.nettrash.familyconnect.ui.chat

/**
 * Whether the scroll-to-newest button should be visible.
 *
 * Two gates, both load-bearing:
 *
 * - [settled] — the ViewModel's opening gate ([ChatViewModel.settled]).
 *   Until the opening scroll is done the list's position means nothing
 *   (an empty LazyColumn reports index 0 on the first frame of EVERY
 *   chat, and an anchored open parks the list well up the thread while
 *   still moving), so an unsettled screen never shows the button — it
 *   must not blink during the open.
 *
 * - [firstVisibleItemIndex] `> 1` — "meaningfully off the bottom", and
 *   deliberately the SAME threshold the read gate reports through
 *   [ChatViewModel.setAtNewest] (`index <= 1` is "at the newest message"
 *   on this reverseLayout thread). The button appears exactly when the
 *   reader's position stops counting as reading the newest message —
 *   within one bubble of scrolling, where the old `index > 5` rule was
 *   several SCREENS of tall bubbles away.
 */
fun showScrollToNewest(settled: Boolean, firstVisibleItemIndex: Int): Boolean =
    settled && firstVisibleItemIndex > 1
