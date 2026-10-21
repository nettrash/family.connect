/*
 * ConversationColumnTest.kt
 * Family Connect (Android) — tests
 *
 * The wide rule of #106 (docs/composer-and-wide-layout-2026-10-20.md, 2.2)
 * and the two-pane threshold (section 3, fix 3), by value: the doc's own
 * table where Android appears in it, and the phone left exactly as it was.
 */

package me.nettrash.familyconnect.ui.components

import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class ConversationColumnTest {

    private fun Dp.rounded(): Float = Math.round(value * 10f) / 10f

    /** A 412dp phone: the 12dp gutter the thread always had, so a 388dp row and a 310dp bubble. */
    @Test
    fun aPhoneIsExactlyAsItWas() {
        val pane = 412.dp
        assertThat(ConversationColumn.listPadding(pane)).isEqualTo(12.dp)
        assertThat(ConversationColumn.width(pane)).isEqualTo(388.dp)
        // Today's bubble: four fifths of the row — not the row less 48 (340),
        // which would have widened every phone's balloons.
        assertThat(ConversationColumn.bubbleMaxWidth(388.dp).rounded()).isEqualTo(310.4f)
        // The composer is not held in at all: the bar is the phone's.
        assertThat(ConversationColumn.composerInset(pane)).isEqualTo(0.dp)
        // A 360dp phone too.
        assertThat(ConversationColumn.listPadding(360.dp)).isEqualTo(12.dp)
        assertThat(ConversationColumn.composerInset(360.dp)).isEqualTo(0.dp)
    }

    /** The doc's Android tablet row: a 939dp conversation pane is a 907dp column, 3 % empty. */
    @Test
    fun aTabletPaneUsesItsWidth() {
        val pane = 939.dp
        assertThat(ConversationColumn.width(pane)).isEqualTo(907.dp)
        assertThat(ConversationColumn.listPadding(pane)).isEqualTo(16.dp)
        assertThat(ConversationColumn.composerInset(pane)).isEqualTo(0.dp)
        // Balloons stop at 560 however wide the column is (#44's worry, kept solved).
        assertThat(ConversationColumn.bubbleMaxWidth(907.dp)).isEqualTo(560.dp)
    }

    /** Past 1024 + the gutters the column stops growing and is centred; so is the composer. */
    @Test
    fun aVeryWidePaneCentresA1024Column() {
        val pane = 1600.dp
        assertThat(ConversationColumn.width(pane)).isEqualTo(1024.dp)
        assertThat(ConversationColumn.listPadding(pane)).isEqualTo(288.dp)
        // The composer's row (8dp padding inside) starts 8dp outside the
        // column, as it does on every wide pane below the cap.
        assertThat(ConversationColumn.composerInset(pane)).isEqualTo(272.dp)
        assertThat(ConversationColumn.composerInset(pane) + ConversationColumn.COMPOSER_ROW_PADDING)
            .isEqualTo(ConversationColumn.listPadding(pane) - 8.dp)
        assertThat(ConversationColumn.bubbleMaxWidth(1024.dp)).isEqualTo(560.dp)
    }

    /** No jump where the cap starts to bind: 1056dp is 16 + 1024 + 16 either way. */
    @Test
    fun theCapJoinsWithoutAStep() {
        assertThat(ConversationColumn.width(1056.dp)).isEqualTo(1024.dp)
        assertThat(ConversationColumn.listPadding(1056.dp)).isEqualTo(16.dp)
        assertThat(ConversationColumn.composerInset(1056.dp)).isEqualTo(0.dp)
        assertThat(ConversationColumn.composerInset(1058.dp)).isEqualTo(1.dp)
    }

    /** The gutter steps 12 → 16 at 600dp, where the media cap already steps 240 → 320. */
    @Test
    fun theGutterStepsWithTheWideWindow() {
        assertThat(ConversationColumn.gutter(599.dp)).isEqualTo(12.dp)
        assertThat(ConversationColumn.gutter(600.dp)).isEqualTo(16.dp)
    }

    /** The narrowest a bubble's bound gets is never negative, and on a narrow row the 48dp spacer binds. */
    @Test
    fun theBubbleBoundOnNarrowRows() {
        assertThat(ConversationColumn.bubbleMaxWidth(0.dp)).isEqualTo(0.dp)
        assertThat(ConversationColumn.bubbleMaxWidth(100.dp)).isEqualTo(52.dp)
        assertThat(ConversationColumn.bubbleMaxWidth(700.dp)).isEqualTo(560.dp)
    }

    /**
     * Two panes only when the conversation keeps 400dp beside the 340dp list
     * (and its 1dp divider): a 600dp window used to split and leave 259dp.
     */
    @Test
    fun twoPanesOnlyWhenTheConversationKeeps400() {
        assertThat(isTwoPane(widthDp = 600, heightDp = 960)).isFalse()
        assertThat(isTwoPane(widthDp = 740, heightDp = 960)).isFalse()
        assertThat(isTwoPane(widthDp = 741, heightDp = 960)).isTrue()
        assertThat(isTwoPane(widthDp = 1280, heightDp = 800)).isTrue()
        // A phone lying down stays one pane, however wide.
        assertThat(isTwoPane(widthDp = 915, heightDp = 412)).isFalse()
        assertThat(isTwoPane(widthDp = 1280, heightDp = 479)).isFalse()
        assertThat(741 - LIST_PANE_WIDTH_DP - 1).isEqualTo(MIN_CONVERSATION_PANE_DP)
    }
}
