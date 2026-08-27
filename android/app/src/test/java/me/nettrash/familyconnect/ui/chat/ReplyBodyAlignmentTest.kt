/*
 * ReplyBodyAlignmentTest.kt
 * Family Connect (Android)
 *
 * The own-reply body alignment rule: End through two lines, Start from
 * three, never End for anything that is not my own reply.
 */

package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class ReplyBodyAlignmentTest {

    @Test
    fun ownReplyAlignsEndThroughTwoLines() {
        assertThat(ReplyBodyAlignment.alignsEnd(ownReply = true, lineCount = 1)).isTrue()
        assertThat(ReplyBodyAlignment.alignsEnd(ownReply = true, lineCount = 2)).isTrue()
    }

    @Test
    fun ownReplyFallsBackToStartFromThreeLines() {
        assertThat(ReplyBodyAlignment.alignsEnd(ownReply = true, lineCount = 3)).isFalse()
        assertThat(ReplyBodyAlignment.alignsEnd(ownReply = true, lineCount = 12)).isFalse()
    }

    @Test
    fun theThresholdIsTheConstant() {
        assertThat(ReplyBodyAlignment.MAX_END_ALIGNED_LINES).isEqualTo(2)
        assertThat(ReplyBodyAlignment.alignsEnd(ownReply = true, lineCount = ReplyBodyAlignment.MAX_END_ALIGNED_LINES)).isTrue()
        assertThat(ReplyBodyAlignment.alignsEnd(ownReply = true, lineCount = ReplyBodyAlignment.MAX_END_ALIGNED_LINES + 1)).isFalse()
    }

    @Test
    fun anythingButMyOwnReplyNeverAlignsEnd() {
        for (lines in 1..4) {
            assertThat(ReplyBodyAlignment.alignsEnd(ownReply = false, lineCount = lines)).isFalse()
        }
    }
}
