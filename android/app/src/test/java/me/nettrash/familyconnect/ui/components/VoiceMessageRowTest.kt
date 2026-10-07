/*
 * VoiceMessageRowTest.kt
 * Family Connect (Android)
 *
 * The voice bubble as the approved design draws it (#79; docs/protocol.md,
 * "A voice note's waveform"), on Robolectric:
 *
 *  - TalkBack: the waveform is "Voice message, 0:42", an adjustable value
 *    (where it is, of how long) whose set-progress seeks, with "Not played"
 *    or "Played" as its state; the play disc is Play;
 *  - the unplayed dot: somebody else's message this device has not played,
 *    never the reader's own, gone once the played store has it;
 *  - the speed chip: "1×" → "1.5×" → "2×" → "1×", said as "Playback speed, …",
 *    one setting for the device;
 *  - and in PIXELS (native graphics): the bars ARE the waveform — a loud
 *    slice is drawn taller than a quiet one — the played ones are the accent
 *    and the rest the quiet tone, and an attachment with no waveform draws
 *    the flat placeholder.
 */

package me.nettrash.familyconnect.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.activity.ComponentActivity
import me.nettrash.familyconnect.testutil.pixelsOf
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.unit.dp
import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.repo.Waveform
import me.nettrash.familyconnect.ui.chat.LocalPlaybackCoordinator
import me.nettrash.familyconnect.ui.chat.PlaybackCoordinator
import me.nettrash.familyconnect.ui.chat.VoiceSpeed
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@Config(qualifiers = "en")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class VoiceMessageRowTest {

    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    private val owner = PlaybackCoordinator()

    private val note = AttachmentDto(
        id = 77, kind = AttachmentDto.KIND_AUDIO, mime = "audio/mp4", size = 4096, durationMs = 42_000,
        waveform = "0124689abcddeeedcba987654321001245678aabbba98642",
    )

    private fun show(attachment: AttachmentDto = note, isMine: Boolean = false, acked: Boolean = true) {
        compose.setContent {
            CompositionLocalProvider(LocalPlaybackCoordinator provides owner) {
                VoiceMessageRow(
                    attachment = attachment,
                    streamUrl = { null },
                    onLongPress = {},
                    onDoubleTap = {},
                    isMine = isMine,
                    acked = acked,
                )
            }
        }
    }

    private fun wave() = compose.onNodeWithContentDescription("Voice message, 0:42")

    @Test
    fun `talkback hears what it is, that it is new, and can move it`() {
        show()

        wave()
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Not played"))
            .assert(
                SemanticsMatcher.expectValue(
                    SemanticsProperties.ProgressBarRangeInfo,
                    ProgressBarRangeInfo(42_000f, 0f..42_000f),
                ),
            )
        compose.onNodeWithContentDescription("Play").assertExists()
        compose.onNodeWithTag("voice-unplayed-77", useUnmergedTree = true).assertExists()

        // TalkBack's adjust: it seeks, and the clock says where.
        compose.runOnIdle {
            wave().fetchSemanticsNode().config[SemanticsActions.SetProgress].action!!.invoke(12_000f)
        }
        wave()
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "0:12 / 0:42"))
            .assert(
                SemanticsMatcher.expectValue(
                    SemanticsProperties.ProgressBarRangeInfo,
                    ProgressBarRangeInfo(12_000f, 0f..42_000f),
                ),
            )
        compose.onNodeWithText("0:12").assertExists()
    }

    /**
     * TalkBack's double tap lands on the bubble, where the waveform's label
     * merges: it is the ▶'s play — here held back by a running recording,
     * which says why — never a click that does nothing.
     */
    @Test
    fun `talkback's double tap on the bubble plays it`() {
        var explained = 0
        compose.setContent {
            CompositionLocalProvider(
                LocalPlaybackCoordinator provides owner,
                me.nettrash.familyconnect.ui.chat.LocalRecordingGate provides
                    me.nettrash.familyconnect.ui.chat.RecordingGate(recording = true, explain = { explained++ }),
            ) {
                VoiceMessageRow(
                    attachment = note,
                    streamUrl = { null },
                    onLongPress = {},
                    onDoubleTap = {},
                    isMine = false,
                    acked = true,
                )
            }
        }
        compose.runOnIdle {
            val click = wave().fetchSemanticsNode().config[SemanticsActions.OnClick]
            assertThat(click.label).isEqualTo("Play")
            click.action!!.invoke()
        }
        compose.runOnIdle { assertThat(explained).isEqualTo(1) }
    }

    @Test
    fun `the reader's own message has no dot, and a played one loses it`() {
        show(isMine = true)
        compose.onNodeWithTag("voice-unplayed-77", useUnmergedTree = true).assertDoesNotExist()
        wave().assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "0:42"))
    }

    @Test
    fun `once played on this device the dot goes and it says Played`() {
        show()
        compose.runOnIdle { owner.playedVoice.markPlayed(77) }

        compose.onNodeWithTag("voice-unplayed-77", useUnmergedTree = true).assertDoesNotExist()
        wave().assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Played"))
    }

    /**
     * The dot goes the moment playback STARTS — not at the end — as it does
     * on iOS, Windows and the web, and as the approved design draws it: the
     * dot is "you haven't played it yet". A real (shadowed) MediaPlayer that
     * never reaches its end, so a dot cleared only at completion stays.
     */
    @Test
    fun `the dot goes the moment it starts playing`() {
        val context = androidx.test.core.app.ApplicationProvider.getApplicationContext<android.content.Context>()
        val url = "https://family.example/attachments/77/stream"
        org.robolectric.shadows.ShadowMediaPlayer.addMediaInfo(
            org.robolectric.shadows.util.DataSource.toDataSource(context, android.net.Uri.parse(url), emptyMap()),
            org.robolectric.shadows.ShadowMediaPlayer.MediaInfo(42_000, 0),
        )
        compose.setContent {
            CompositionLocalProvider(LocalPlaybackCoordinator provides owner) {
                VoiceMessageRow(
                    attachment = note,
                    streamUrl = { url to emptyMap() },
                    onLongPress = {},
                    onDoubleTap = {},
                    isMine = false,
                    acked = true,
                )
            }
        }
        compose.onNodeWithTag("voice-unplayed-77", useUnmergedTree = true).assertExists()

        compose.onNodeWithContentDescription("Play").performClick()
        compose.waitUntil(5_000) { compose.onAllNodesWithContentDescription("Pause").fetchSemanticsNodes().isNotEmpty() }

        compose.onNodeWithTag("voice-unplayed-77", useUnmergedTree = true).assertDoesNotExist()
        compose.runOnIdle { assertThat(77L in owner.playedVoice.ids.value).isTrue() }
    }

    @Test
    fun `the sender's own on its way has no dot either`() {
        show(acked = false)
        compose.onNodeWithTag("voice-unplayed-77", useUnmergedTree = true).assertDoesNotExist()
    }

    @Test
    fun `the speed chip steps the device's speed and says so`() {
        val speed = VoiceSpeed.InMemory()
        compose.setContent {
            val now by speed.speed.collectAsState()
            VoiceSpeedChip(speed = now, onStep = speed::step)
        }
        compose.onNodeWithContentDescription("Playback speed, 1×").performClick()
        compose.onNodeWithContentDescription("Playback speed, 1.5×").performClick()
        compose.onNodeWithContentDescription("Playback speed, 2×").performClick()
        compose.onNodeWithContentDescription("Playback speed, 1×").assertExists()
        assertThat(speed.speed.value).isEqualTo(1f)
    }

    @Test
    fun `the rules - dot, bar count, seek, speed words`() {
        assertThat(VoiceBubbleRules.showsUnplayedDot(isMine = false, acked = true, played = false)).isTrue()
        assertThat(VoiceBubbleRules.showsUnplayedDot(isMine = true, acked = true, played = false)).isFalse()
        assertThat(VoiceBubbleRules.showsUnplayedDot(isMine = false, acked = false, played = false)).isFalse()
        assertThat(VoiceBubbleRules.showsUnplayedDot(isMine = false, acked = true, played = true)).isFalse()
        // 48 bars of 3 with 47 gaps of 2 is 238: every level gets a bar; never more than 48.
        assertThat(VoiceBubbleRules.barCount(238f, 3f, 2f)).isEqualTo(48)
        assertThat(VoiceBubbleRules.barCount(2_000f, 3f, 2f)).isEqualTo(48)
        assertThat(VoiceBubbleRules.barCount(100f, 3f, 2f)).isEqualTo(20)
        assertThat(VoiceBubbleRules.barCount(1f, 3f, 2f)).isEqualTo(1)
        // A touch a quarter of the way in seeks a quarter in — from the right in a right-to-left layout.
        assertThat(VoiceBubbleRules.seekTo(50f, 200f, 40_000, rtl = false)).isEqualTo(10_000)
        assertThat(VoiceBubbleRules.seekTo(50f, 200f, 40_000, rtl = true)).isEqualTo(30_000)
        assertThat(VoiceBubbleRules.seekTo(-5f, 200f, 40_000, rtl = false)).isEqualTo(0)
        assertThat(VoiceBubbleRules.seekTo(500f, 200f, 40_000, rtl = false)).isEqualTo(40_000)
        assertThat(VoiceSpeed.next(1f)).isEqualTo(1.5f)
        assertThat(VoiceSpeed.next(1.5f)).isEqualTo(2f)
        assertThat(VoiceSpeed.next(2f)).isEqualTo(1f)
        assertThat(VoiceSpeed.normalised(3f)).isEqualTo(1f)
    }

    // -- Pixels ---------------------------------------------------------------

    private val played = Color(0xFFD00000)
    private val rest = Color(0xFF0000D0)

    /** 48 bars exactly (238 dp), 34 dp tall on white; [lit] of them played. */
    private fun drawBars(levels: List<Int>, lit: Int) {
        compose.setContent {
            Box(Modifier.background(Color.White).testTag("bars-ground")) {
                WaveformBars(
                    levels = levels,
                    played = { lit },
                    active = played,
                    inactive = rest,
                    modifier = Modifier.width(238.dp).height(34.dp),
                )
            }
        }
    }

    /** The centre column of bar [index] of 48 across [width] pixels. */
    private fun column(index: Int, width: Int): Int {
        val bar = width / 238f * 3f
        val step = (width - bar) / 47f
        return (index * step + bar / 2f).toInt()
    }

    /** How many pixels of bar [index]'s centre column are inked. */
    private fun inkedHeight(index: Int): Int {
        val pixels = compose.pixelsOf("bars-ground")
        val x = column(index, pixels.width)
        return (0 until pixels.height).count { y -> pixels[x, y] != Color.White.toArgb() }
    }

    private fun colourOf(index: Int): Int {
        val pixels = compose.pixelsOf("bars-ground")
        return pixels[column(index, pixels.width), pixels.height / 2]
    }

    private fun groundHeight(): Int = compose.pixelsOf("bars-ground").height

    @Test
    fun `a loud slice is a tall bar and a quiet one a short bar, at (2 + level) over 17`() {
        val levels = List(24) { 0 } + List(24) { 15 }
        drawBars(levels, lit = 0)

        val quiet = inkedHeight(3)
        val loud = inkedHeight(40)
        assertThat(loud).isGreaterThan(quiet * 4)
        // Level 15 is the full height; level 0 is 2/17 of it.
        val full = groundHeight()
        assertThat(loud.toDouble()).isWithin(2.0).of(full.toDouble())
        assertThat(quiet.toDouble()).isWithin(2.0).of(full * Waveform.barFraction(0))
    }

    @Test
    fun `the played bars are the accent, the rest the quiet tone`() {
        drawBars(List(48) { 10 }, lit = 12)

        assertThat(colourOf(5)).isEqualTo(played.toArgb())
        assertThat(colourOf(11)).isEqualTo(played.toArgb())
        assertThat(colourOf(12)).isEqualTo(rest.toArgb())
        assertThat(colourOf(47)).isEqualTo(rest.toArgb())
    }

    @Test
    fun `no waveform draws the flat placeholder`() {
        drawBars(Waveform.levelsOrPlaceholder(null), lit = 0)

        val heights = listOf(0, 10, 23, 47).map(::inkedHeight)
        assertThat(heights.toSet()).hasSize(1)
        val full = groundHeight()
        assertThat(heights.first().toDouble()).isWithin(2.0).of(full * Waveform.barFraction(Waveform.PLACEHOLDER_LEVEL))
    }
}
