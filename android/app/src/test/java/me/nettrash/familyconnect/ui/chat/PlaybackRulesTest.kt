/*
 * PlaybackRulesTest.kt
 * Family Connect (Android)
 *
 * The two rules every play control in a chat keeps from Phase 1 on (#79,
 * docs/audio-video-messages-2026-10-04.md, S1.7, S5.3):
 *
 *  - one thing plays at a time: a note starting pauses whatever else plays,
 *    and a recording starting pauses it all (PlaybackCoordinator);
 *  - nothing plays over a recording: the bubbles' own audio rows — not only
 *    the staged and not-sent notes — are dimmed while one runs and say "You
 *    can play this after recording." instead of playing (RecordingGate).
 */

package me.nettrash.familyconnect.ui.chat

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.ui.components.AttachmentBlock
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class PlaybackRulesTest {

    @get:Rule
    val compose = createComposeRule()

    @Test
    fun aNoteStartingPausesTheOneThatWasPlaying() {
        val coordinator = PlaybackCoordinator()
        val paused = mutableListOf<String>()
        val first: () -> Unit = { paused += "first" }
        val second: () -> Unit = { paused += "second" }

        coordinator.started(first)
        coordinator.started(second)
        assertThat(paused).containsExactly("first")

        // Starting the same one again pauses nothing.
        coordinator.started(second)
        assertThat(paused).containsExactly("first")
    }

    @Test
    fun aRecordingStartingPausesWhatPlaysAndNothingAfterItStopped() {
        val coordinator = PlaybackCoordinator()
        val paused = mutableListOf<String>()
        val note: () -> Unit = { paused += "note" }

        coordinator.started(note)
        coordinator.pauseAll()
        assertThat(paused).containsExactly("note")

        // Stopped on its own: a later recording has nothing to pause.
        coordinator.started(note)
        coordinator.stopped(note)
        coordinator.pauseAll()
        assertThat(paused).containsExactly("note")
    }

    /** A received voice note's ▶ waits while the composer records, and says why (S1.7). */
    @Test
    fun aBubblesAudioRowSaysWhyItWaitsWhileRecording() {
        var explained = 0
        compose.setContent {
            CompositionLocalProvider(
                LocalRecordingGate provides RecordingGate(recording = true, explain = { explained++ }),
            ) {
                AttachmentBlock(
                    attachment = AttachmentDto(
                        id = 9, kind = AttachmentDto.KIND_AUDIO, mime = "audio/mp4", size = 4_096, durationMs = 42_000,
                    ),
                    onOpen = {},
                )
            }
        }

        compose.onNodeWithContentDescription("Play")
            .assert(
                SemanticsMatcher.expectValue(
                    SemanticsProperties.StateDescription,
                    "You can play this after recording.",
                ),
            )
            .performClick()

        assertThat(explained).isEqualTo(1)
    }

    /** Outside a recording the row is as it always was: no state, no sentence. */
    @Test
    fun aBubblesAudioRowIsUnchangedWhenNothingRecords() {
        var explained = 0
        compose.setContent {
            CompositionLocalProvider(
                LocalRecordingGate provides RecordingGate(recording = false, explain = { explained++ }),
            ) {
                AttachmentBlock(
                    attachment = AttachmentDto(
                        id = 9, kind = AttachmentDto.KIND_AUDIO, mime = "audio/mp4", size = 4_096, durationMs = 42_000,
                    ),
                    onOpen = {},
                )
            }
        }

        compose.onNodeWithContentDescription("Play")
            .assert(SemanticsMatcher.keyNotDefined(SemanticsProperties.StateDescription))
            .performClick()

        assertThat(explained).isEqualTo(0)
    }
}
