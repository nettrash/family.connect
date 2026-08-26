/*
 * UnreadBadgeTest.kt
 * Family Connect (Android)
 *
 * The badge arithmetic, asserted without a launcher to read it back off —
 * which is the whole reason it is a pure object. Two rules, and both of
 * them exist because of a way the badge lies:
 *
 *   - the total clamps PER CHAT, so one broken count cannot cancel out
 *     real unread messages sitting in another chat;
 *   - the sweep is a TRANSITION and never the plain state, so a process
 *     that has just started — and whose store honestly says zero for a
 *     message it has never seen — does not delete the one notification
 *     the user has not read.
 */

package me.nettrash.familyconnect.data.push

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class UnreadBadgeTest {

    // -- the total ---------------------------------------------------------------------

    @Test
    fun theTotalIsTheSumOfTheChats() {
        assertThat(UnreadBadge.total(listOf(3, 1, 7))).isEqualTo(11)
    }

    @Test
    fun nothingUnreadIsZeroAndSoIsAnEmptyStore() {
        assertThat(UnreadBadge.total(listOf(0, 0))).isEqualTo(0)
        assertThat(UnreadBadge.total(emptyList())).isEqualTo(0)
    }

    /**
     * The clamp is per chat, not on the sum. A count that has somehow
     * gone negative is a broken row, and a broken row must not be able to
     * hide messages waiting in a chat that is perfectly fine.
     */
    @Test
    fun aNegativeCountCannotCancelOutAnotherChatsMessages() {
        assertThat(UnreadBadge.total(listOf(-5, 3))).isEqualTo(3)
        assertThat(UnreadBadge.total(listOf(-5, -1))).isEqualTo(0)
    }

    // -- the sweep ---------------------------------------------------------------------

    @Test
    fun aChatGoingToZeroIsSwept() {
        assertThat(UnreadBadge.nowRead(before = mapOf(42L to 3), after = mapOf(42L to 0)))
            .containsExactly(42L)
    }

    @Test
    fun aChatStillHoldingUnreadIsNotSwept() {
        assertThat(UnreadBadge.nowRead(before = mapOf(42L to 3), after = mapOf(42L to 1)))
            .isEmpty()
    }

    /**
     * THE COLD-LAUNCH TRAP. The first emission has nothing before it, and
     * a store that says zero for every chat is exactly what a process
     * that has just started looks like — including one started by a push
     * whose message the SYSTEM tray rendered and this process never saw.
     * Acting on that zero would delete the notification the launch is
     * about.
     */
    @Test
    fun theFirstEmissionSweepsNothing() {
        assertThat(UnreadBadge.nowRead(before = emptyMap(), after = mapOf(42L to 0)))
            .isEmpty()
    }

    /** A chat that was already read stays read; there is nothing to do. */
    @Test
    fun aChatThatWasAlreadyAtZeroIsNotSweptAgain() {
        assertThat(UnreadBadge.nowRead(before = mapOf(42L to 0), after = mapOf(42L to 0)))
            .isEmpty()
    }

    /**
     * A peer deleting their account takes the direct chat with it, and
     * the row the count lived on. The notification would otherwise
     * outlive the conversation it points at — a badge for a chat that
     * cannot be opened.
     */
    @Test
    fun aChatThatDisappearsWithUnreadIsSwept() {
        assertThat(UnreadBadge.nowRead(before = mapOf(42L to 2), after = emptyMap()))
            .containsExactly(42L)
    }

    @Test
    fun onlyTheChatsThatChangedAreSwept() {
        assertThat(
            UnreadBadge.nowRead(
                before = mapOf(1L to 3, 2L to 1, 3L to 0),
                after = mapOf(1L to 0, 2L to 1, 3L to 0),
            ),
        ).containsExactly(1L)
    }
}
