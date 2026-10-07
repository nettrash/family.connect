/*
 * NowPlayingTest.kt
 * Family Connect (Android)
 *
 * The now-playing owner's two system halves (#79,
 * docs/audio-video-messages-2026-10-04.md, S4, S5.2):
 *
 *  - SystemAudioInterruptions asks for AUDIOFOCUS_GAIN_TRANSIENT while
 *    something plays and gives it back after; a loss of focus — a call,
 *    another app's sound — and ACTION_AUDIO_BECOMING_NOISY pause; nothing is
 *    heard once it has stopped listening.
 *  - The played-videos store keeps the newest 5 000 ids on this device,
 *    survives a restart, and is wiped by the sign-out reset.
 */

package me.nettrash.familyconnect.ui.chat

import android.content.Context
import android.content.Intent
import android.media.AudioManager
import android.os.Looper
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import me.nettrash.familyconnect.data.settings.DataStoreSettingsRepository
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import java.io.File

@RunWith(RobolectricTestRunner::class)
class NowPlayingTest {

    @get:Rule
    val folder = TemporaryFolder()

    private val context: Context = RuntimeEnvironment.getApplication()
    private val audio = context.getSystemService(AudioManager::class.java)

    // -- Audio focus and becoming noisy --------------------------------------------------

    @Test
    fun `playing asks for transient focus, for speech, and pauses on every loss`() {
        val interruptions = SystemAudioInterruptions(context)
        var lost = 0
        interruptions.begin { lost++ }

        val asked = shadowOf(audio).lastAudioFocusRequest
        assertThat(asked.durationHint).isEqualTo(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT)
        assertThat(asked.audioFocusRequest.willPauseWhenDucked()).isTrue()

        for (change in listOf(
            AudioManager.AUDIOFOCUS_LOSS,
            AudioManager.AUDIOFOCUS_LOSS_TRANSIENT,
            AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK,
        )) {
            asked.listener.onAudioFocusChange(change)
        }
        assertThat(lost).isEqualTo(3)
        // Getting it back starts nothing by itself.
        asked.listener.onAudioFocusChange(AudioManager.AUDIOFOCUS_GAIN)
        assertThat(lost).isEqualTo(3)
        interruptions.end()
    }

    @Test
    fun `asked again while held, it asks for nothing`() {
        val interruptions = SystemAudioInterruptions(context)
        interruptions.begin {}
        val first = shadowOf(audio).lastAudioFocusRequest
        interruptions.begin {}

        assertThat(shadowOf(audio).lastAudioFocusRequest).isSameInstanceAs(first)
        interruptions.end()
    }

    @Test
    fun `the headphones coming out pause it, and only while it plays`() {
        val interruptions = SystemAudioInterruptions(context)
        var lost = 0
        interruptions.begin { lost++ }

        context.sendBroadcast(Intent(AudioManager.ACTION_AUDIO_BECOMING_NOISY))
        shadowOf(Looper.getMainLooper()).idle()
        assertThat(lost).isEqualTo(1)

        interruptions.end()
        context.sendBroadcast(Intent(AudioManager.ACTION_AUDIO_BECOMING_NOISY))
        shadowOf(Looper.getMainLooper()).idle()
        assertThat(lost).isEqualTo(1)
    }

    @Test
    fun `stopping gives the focus back`() {
        val interruptions = SystemAudioInterruptions(context)
        interruptions.begin {}
        val asked = shadowOf(audio).lastAudioFocusRequest.audioFocusRequest

        interruptions.end()

        assertThat(shadowOf(audio).lastAbandonedAudioFocusRequest).isSameInstanceAs(asked)
    }

    @Test
    fun `refused - a call holds the audio - it pauses rather than plays over it`() {
        val interruptions = SystemAudioInterruptions(context)
        shadowOf(audio).setNextFocusRequestResponse(AudioManager.AUDIOFOCUS_REQUEST_FAILED)
        var lost = 0

        interruptions.begin { lost++ }
        shadowOf(Looper.getMainLooper()).idle()

        assertThat(lost).isEqualTo(1)
        interruptions.end()
    }

    // -- What this device has played -----------------------------------------------------------

    @Test
    fun `the newest five thousand are remembered`() {
        val many = (1L..5_001L).toSet()
        val kept = PlayedRoundVideos.trim(many)

        assertThat(kept).hasSize(PlayedRoundVideos.REMEMBERED)
        assertThat(kept).doesNotContain(1L)
        assertThat(kept).contains(5_001L)
        assertThat(PlayedRoundVideos.decode(PlayedRoundVideos.encode(setOf(3L, 1L, 2L)))).containsExactly(1L, 2L, 3L)
        assertThat(PlayedRoundVideos.decode(null)).isEmpty()
        assertThat(PlayedRoundVideos.decode("4,x,5")).containsExactly(4L, 5L)
    }

    /** One "process" over [file], shut down completely after [block]. */
    private fun <T> launch(file: File, block: suspend (StoredPlayedRoundVideos, DataStoreSettingsRepository) -> T): T =
        runBlocking {
            val job = Job()
            val scope = CoroutineScope(Dispatchers.IO + job)
            val store = PreferenceDataStoreFactory.create(scope = scope) { file }
            try {
                block(StoredPlayedRoundVideos(store, scope), DataStoreSettingsRepository(store))
            } finally {
                job.cancelAndJoin()
            }
        }

    @Test
    fun `played survives a restart and goes with the sign-out`() {
        val file = File(folder.root, "settings.preferences_pb")
        launch(file) { played, _ ->
            played.markPlayed(91)
            played.markPlayed(92)
            withTimeout(5_000) { played.ids.first { it.containsAll(listOf(91L, 92L)) } }
        }
        launch(file) { played, settings ->
            assertThat(withTimeout(5_000) { played.ids.first { it.isNotEmpty() } }).containsExactly(91L, 92L)

            // The sign-out reset keeps only what is the DEVICE's; this is
            // the account's.
            settings.resetKeepingServerUrl()
            assertThat(withTimeout(5_000) { played.ids.first { it.isEmpty() } }).isEmpty()
        }
    }
}
