/*
 * BoardWallTest.kt
 * Family Connect (Android)
 *
 * The wall is TALLER than the window and never shorter (docs/protocol.md,
 * "Board"), and the factor is the one every client uses — a note two
 * thirds of the way down is two thirds of the way down on the phone and on
 * the Mac.
 *
 * Web counterpart: `fc_text::board`'s
 * `the_wall_is_taller_than_the_window_and_never_shorter`.
 * Apple counterpart: `NoteSizeTests.wallHeight`.
 */

package me.nettrash.familyconnect.ui.board

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class BoardWallTest {

    @Test
    fun `the wall is taller than the window and never shorter`() {
        assertThat(BoardWall.SCREENS).isEqualTo(1.6f)
        assertThat(BoardWall.heightPx(500)).isEqualTo(800)
        assertThat(BoardWall.heightPx(1000)).isGreaterThan(1000)
        assertThat(BoardWall.heightPx(0)).isEqualTo(0)
        // A window nobody could pin anything in is still not a wall behind
        // its own edges.
        assertThat(BoardWall.heightPx(1)).isAtLeast(1)
    }
}
