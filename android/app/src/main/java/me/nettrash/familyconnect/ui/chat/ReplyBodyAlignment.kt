/*
 * ReplyBodyAlignment.kt
 * Family Connect (Android)
 *
 * The one alignment exception in a balloon, as a pure rule: the body of
 * MY OWN REPLY sits at End under its left-aligned quote — but only while
 * it is short. A body that wraps to three or more lines reads as a
 * paragraph, and a right-ragged paragraph is hard to read, so from the
 * third line on the block goes back to Start like every other body.
 * Pinned on the plain JVM (ReplyBodyAlignmentTest); ChatScreen's
 * TextBlock asks it with the line count of the laid-out text.
 */

package me.nettrash.familyconnect.ui.chat

object ReplyBodyAlignment {

    /** The most lines an own-reply body may have and still rag End. */
    const val MAX_END_ALIGNED_LINES = 2

    /**
     * Whether a body of [lineCount] laid-out lines aligns End. False for
     * anything that is not my own reply; the product owner's rule
     * otherwise: End through two lines, Start from three.
     */
    fun alignsEnd(ownReply: Boolean, lineCount: Int): Boolean =
        ownReply && lineCount <= MAX_END_ALIGNED_LINES
}
