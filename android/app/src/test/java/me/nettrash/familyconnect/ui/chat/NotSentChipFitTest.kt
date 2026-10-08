/*
 * NotSentChipFitTest.kt
 * Family Connect (Android)
 *
 * The not-sent chip (#79, S2.8, the approved design) still fits on a narrow
 * phone in a longer language and at large text: "Not sent", ▶, the waveform,
 * the length, Send and ✕ in at most 360 units. Only the middle stretches, so
 * the fixed pieces are measured first — and with the words long enough, the
 * waveform was squeezed to nothing and the length cut off.
 */

package me.nettrash.familyconnect.ui.chat

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import com.google.common.truth.Truth.assertThat
import com.google.common.truth.Truth.assertWithMessage
import me.nettrash.familyconnect.data.repo.ParkedRecording
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
// A common narrow phone: 360 units across.
@Config(qualifiers = "w360dp-h800dp")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class NotSentChipFitTest {

    @get:Rule
    val compose = createComposeRule()

    private val entry = ParkedRecording(id = "a", chatId = 42, file = "a.m4a", durationMs = 42_000)

    private fun show(fontScale: Float, playing: Boolean = false) {
        compose.setContent {
            val base = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(base.density, fontScale)) {
                Box(Modifier.width(360.dp)) {
                    NotSentVoiceMessageRow(
                        entry = entry,
                        replyAuthorName = null,
                        enabled = true,
                        sending = false,
                        onSend = {},
                        onDelete = {},
                        playing = playing,
                        positionMs = if (playing) 12_000 else 0,
                        onTogglePlay = {},
                    )
                }
            }
        }
        compose.waitForIdle()
    }

    private fun assertFits(what: String) {
        val density = compose.density.density
        val wave = compose.onNodeWithTag("voice-chip-wave", useUnmergedTree = true).fetchSemanticsNode()
        val waveDp = wave.size.width / density
        val time = compose.onNodeWithTag("voice-chip-time", useUnmergedTree = true).fetchSemanticsNode()
        val layouts = mutableListOf<TextLayoutResult>()
        time.config.getOrNull(SemanticsActions.GetTextLayoutResult)?.action?.invoke(layouts)
        assertWithMessage("$what: waveform width (dp)").that(waveDp).isAtLeast(MIN_WAVE_DP)
        // Whole: as wide as the words need, on one line.
        val layout = layouts.single()
        assertWithMessage("$what: the length is cut off")
            .that(time.size.width.toFloat())
            .isAtLeast(kotlin.math.floor(layout.multiParagraph.intrinsics.maxIntrinsicWidth))
        assertWithMessage("$what: the length wraps").that(layout.lineCount).isEqualTo(1)
    }

    @Test
    @Config(qualifiers = "+ru")
    fun russianAtNormalText() {
        show(1f)
        assertFits("ru 1.0")
    }

    @Test
    @Config(qualifiers = "+de")
    fun germanAtLargeText() {
        show(1.3f)
        assertFits("de 1.3")
    }

    @Test
    @Config(qualifiers = "+de")
    fun germanAtLargestTextWhilePlaying() {
        show(2f, playing = true)
        assertFits("de 2.0 playing")
    }

    @Test
    @Config(qualifiers = "+en")
    fun englishAtNormalText() {
        show(1f)
        assertFits("en 1.0")
        // Where it fits, it is the design's one line: "Not sent" beside the waveform.
        val label = compose.onNodeWithText("Not sent").fetchSemanticsNode().boundsInRoot
        val wave = compose.onNodeWithTag("voice-chip-wave", useUnmergedTree = true).fetchSemanticsNode().boundsInRoot
        assertThat(label.right).isLessThan(wave.left)
        assertThat(label.center.y).isWithin(2f * compose.density.density).of(wave.center.y)
    }

    @Test
    @Config(qualifiers = "+ru")
    fun whereItDoesNotFitNotSentGoesAbove() {
        show(1f)
        val label = compose.onNodeWithText("Не отправлено").fetchSemanticsNode().boundsInRoot
        val wave = compose.onNodeWithTag("voice-chip-wave", useUnmergedTree = true).fetchSemanticsNode().boundsInRoot
        assertThat(label.bottom).isAtMost(wave.top)
        // The actions are all still there and whole.
        compose.onNodeWithText("Отправить").assertIsDisplayed()
    }

    private companion object {
        const val MIN_WAVE_DP = 40f
    }
}
