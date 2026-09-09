/*
 * NoteSizesFittingTest.kt
 * Family Connect (Android) — tests
 *
 * The fitting contract (docs/protocol.md, "Board"): a sticker's type scales
 * down from the size's own until the whole note is inside it, and stops at
 * a floor. Pinned against the iOS counterpart in NoteSizeTests.swift, which
 * asserts the same three things about NoteSize.
 */

package me.nettrash.familyconnect.ui.board

import androidx.compose.material3.Typography
import androidx.compose.ui.unit.sp
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class NoteSizesFittingTest {

    private val typography = Typography()

    @Test
    fun `the fitting range runs from the floor up to the step's own type`() {
        for (name in NoteSizes.steps) {
            val ceiling = NoteSizes.textStyle(name, typography).fontSize
            val range = NoteSizes.autoSize(name, typography)
            assertThat(range).isEqualTo(
                androidx.compose.foundation.text.TextAutoSize.StepBased(
                    minFontSize = ceiling * NoteSizes.MIN_TEXT_SCALE,
                    maxFontSize = ceiling,
                ),
            )
        }
    }

    /**
     * The floor is a FRACTION of the step's own type, and the same fraction
     * at every step. An absolute sp floor would sit ABOVE the ceiling on a
     * large font-scale setting, and a floor above the ceiling never fits
     * anything; three DIFFERENT fractions would make one note fit a small
     * sticker and not the size up, which is the opposite of what a step is.
     */
    @Test
    fun `the floor is one proportion, below the type and above nothing`() {
        assertThat(NoteSizes.MIN_TEXT_SCALE).isGreaterThan(0f)
        assertThat(NoteSizes.MIN_TEXT_SCALE).isLessThan(1f)
        val floors = NoteSizes.steps.map {
            NoteSizes.textStyle(it, typography).fontSize * NoteSizes.MIN_TEXT_SCALE
        }
        val ceilings = NoteSizes.steps.map { NoteSizes.textStyle(it, typography).fontSize }
        floors.zip(ceilings).forEach { (floor, ceiling) ->
            assertThat(floor.value).isLessThan(ceiling.value)
        }
    }

    /**
     * The line count stopped being the layout rule when fitting arrived: it
     * is a backstop for one unbroken word. A per-size count would stop the
     * shrink long before the sticker was full.
     */
    @Test
    fun `the line limit is a generous backstop, not a per-size layout rule`() {
        assertThat(NoteSizes.FITTED_MAX_LINES).isAtLeast(20)
        // The steps still differ in the two things that ARE the step.
        assertThat(NoteSizes.side("small").value).isLessThan(NoteSizes.side("medium").value)
        assertThat(NoteSizes.side("medium").value).isLessThan(NoteSizes.side("large").value)
        assertThat(NoteSizes.textStyle("small", typography).fontSize.value)
            .isLessThan(NoteSizes.textStyle("large", typography).fontSize.value)
    }
}
