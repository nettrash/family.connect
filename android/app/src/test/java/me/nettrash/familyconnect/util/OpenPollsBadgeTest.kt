/*
 * OpenPollsBadgeTest.kt
 * Family Connect (Android)
 *
 * The open-polls badge rule (docs/protocol.md, "Finding the open ones").
 *
 * The rule is "open polls this reader has not voted in", and every one of the
 * three obvious alternatives is wrong in a way a test can state:
 *
 * - counting ALL open polls never clears by anything the reader can do, so it
 *   stays lit until somebody else closes them and stops meaning anything;
 * - counting CLOSED ones asks for a decision that has already been made;
 * - counting somebody ELSE's outstanding polls is a badge about a person who
 *   is not holding the phone.
 *
 * The vectors here are mirrored by value in
 * ios/FamilyConnectTests/OpenPollsBadgeTests.swift: same polls, same reader,
 * same numbers. Two ports that disagree about what a badge counts would show
 * a family two different numbers for the same chat.
 */

package me.nettrash.familyconnect.util

import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.net.dto.PollDto
import me.nettrash.familyconnect.data.net.dto.PollOptionDto
import org.junit.Test

class OpenPollsBadgeTest {

    private val me = 7L
    private val someoneElse = 9L

    private fun poll(closed: Boolean = false, votes: List<Long> = emptyList()) = PollDto(
        pollSeq = 1,
        closed = closed,
        options = listOf(
            PollOptionDto(id = 1, text = "Pizza", votes = votes),
            PollOptionDto(id = 2, text = "Pasta", votes = emptyList()),
        ),
    )

    @Test
    fun `an open poll nobody has answered counts`() {
        assertThat(OpenPollsBadge.count(listOf(poll()), me)).isEqualTo(1)
    }

    @Test
    fun `voting clears it, which is the whole point of the rule`() {
        val answered = poll(votes = listOf(me))
        assertThat(OpenPollsBadge.count(listOf(answered), me)).isEqualTo(0)
        assertThat(OpenPollsBadge.hasVoted(answered, me)).isTrue()
    }

    /** The badge is about the person holding the phone. */
    @Test
    fun `somebody else's vote does not answer it for me`() {
        val theirs = poll(votes = listOf(someoneElse))
        assertThat(OpenPollsBadge.count(listOf(theirs), me)).isEqualTo(1)
        assertThat(OpenPollsBadge.hasVoted(theirs, me)).isFalse()
    }

    /**
     * A closed poll is a RESULT. There is nothing left to ask of anybody,
     * whether or not this reader ever answered it.
     */
    @Test
    fun `a closed poll never counts, answered or not`() {
        assertThat(OpenPollsBadge.count(listOf(poll(closed = true)), me)).isEqualTo(0)
        assertThat(
            OpenPollsBadge.count(listOf(poll(closed = true, votes = listOf(me))), me),
        ).isEqualTo(0)
    }

    @Test
    fun `the count is over the whole set, and only the unanswered open ones`() {
        val polls = listOf(
            poll(),                                        // counts
            poll(votes = listOf(me)),                      // answered
            poll(votes = listOf(someoneElse)),             // counts
            poll(closed = true),                           // closed
            poll(closed = true, votes = listOf(me)),       // closed and answered
            poll(),                                        // counts
        )
        assertThat(OpenPollsBadge.count(polls, me)).isEqualTo(3)
    }

    @Test
    fun `no polls is no badge`() {
        assertThat(OpenPollsBadge.count(emptyList(), me)).isEqualTo(0)
    }

    /**
     * The same rule reads a wire poll and the view model's own — the chat's
     * badge is computed over `PollView` and the surface's list over `PollDto`,
     * and both implement the interface rather than counting by hand.
     */
    @Test
    fun `the same rule reads either type of poll`() {
        val dto = poll(votes = listOf(someoneElse))
        assertThat(OpenPollsBadge.count(listOf(dto), me)).isEqualTo(1)
        assertThat(OpenPollsBadge.count(listOf(dto), someoneElse)).isEqualTo(0)
    }
}
