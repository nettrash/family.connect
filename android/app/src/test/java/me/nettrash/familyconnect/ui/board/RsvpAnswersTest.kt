/*
 * RsvpAnswersTest.kt
 * Family Connect (Android) — tests
 *
 * The three answers to an event (docs/protocol.md, "Board"), pinned against
 * the iOS counterpart RsvpAnswer.
 */

package me.nettrash.familyconnect.ui.board

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class RsvpAnswersTest {

    @Test
    fun `the vocabulary is the protocol's, in picker order`() {
        assertThat(RsvpAnswers.all).containsExactly("going", "maybe", "no").inOrder()
    }

    @Test
    fun `each answer has its own label`() {
        assertThat(RsvpAnswers.all.map { RsvpAnswers.label(it) }.toSet()).hasSize(3)
    }

    /**
     * An answer from a NEWER server must draw as SOMETHING rather than
     * crash — and "going" is the safest read of an unknown one only in the
     * sense that it is a label, not a claim: the caller highlights nothing,
     * because `answered == option` never matches.
     */
    @Test
    fun `an unknown answer still resolves to a label`() {
        assertThat(RsvpAnswers.label("perhaps")).isEqualTo(RsvpAnswers.label("going"))
        assertThat(RsvpAnswers.all).doesNotContain("perhaps")
    }
}
