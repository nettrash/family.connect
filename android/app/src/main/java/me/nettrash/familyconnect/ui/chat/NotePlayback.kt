/*
 * NotePlayback.kt
 * Family Connect (Android)
 *
 * Playing a voice note that has not been sent yet — the staged chip's ▶ and
 * the "not sent" row's ▶ (#79, docs/audio-video-messages-2026-10-04.md,
 * S2.7, S2.8) — and the two rules every play control in the chat keeps from
 * Phase 1 on (S1.7):
 *
 *  - ONE THING PLAYS AT A TIME: a note starting pauses whatever else of the
 *    chat's plays ([PlaybackCoordinator]).
 *  - NOTHING PLAYS OVER A RECORDING: starting one pauses what plays, and
 *    while it runs every play control is dimmed and says "You can play this
 *    after recording." ([RecordingGate]).
 *
 * Both are provided through composition locals so the bubbles' audio rows
 * (ui/components/Attachments.kt) take part without threading parameters
 * through every bubble. From Phase 2 the coordinator the app provides is the
 * NOW-PLAYING OWNER ([NowPlaying]), which lives outside the activity, asks
 * for transient audio focus while anything plays and pauses when it goes or
 * the headphones come out (S4), and keeps the round videos' players
 * (RoundVideoPlayback.kt).
 *
 * The player plays the LOCAL file — what was recorded is what is heard,
 * before anything has left the device.
 */

package me.nettrash.familyconnect.ui.chat

import android.media.MediaPlayer
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * One thing of the app's plays at a time (S5.3), a recording pauses it
 * (S1.7), and so does anything S4 says pauses playback — another app taking
 * the audio focus, the headphones coming out ([interruptions]).
 *
 * Also the keeper of the round videos' players ([round]): a circle's player
 * belongs to its attachment, not to the composable drawing it, so a rotation
 * or a fold that rebuilds the activity leaves it playing (S4's last column).
 * Open so the app's own [NowPlaying] can be one, built with the system's
 * focus and the device's played-videos store; a test builds this directly.
 */
@Stable
open class PlaybackCoordinator(
    private val interruptions: AudioInterruptions = AudioInterruptions.NONE,
    /** What THIS DEVICE has played (S5.2's dot) — in memory unless the app hands in its store. */
    val played: PlayedRoundVideos = PlayedRoundVideos.InMemory(),
    /** Which VOICE messages this device has played (#79): their own dot, the circles' pattern. */
    val playedVoice: PlayedRoundVideos = PlayedRoundVideos.InMemory(),
    /** How fast voice messages play on this device (#79). */
    val voiceSpeed: VoiceSpeed = VoiceSpeed.InMemory(),
) {

    private var current: (() -> Unit)? = null

    /** [pause] has started playing: whatever played before it pauses. */
    fun started(pause: () -> Unit) {
        val previous = current
        current = pause
        if (previous != null && previous !== pause) previous()
        // Held for as long as anything plays; asked again for nothing.
        interruptions.begin(onLost = ::pauseAll)
    }

    /** [pause]'s player stopped on its own, or went away. */
    fun stopped(pause: () -> Unit) {
        if (current === pause) {
            current = null
            interruptions.end()
        }
    }

    /**
     * A recording starts, a call takes the audio, the headphones come out,
     * the app goes to the background: nothing the app plays runs on.
     */
    fun pauseAll() {
        val playing = current
        current = null
        interruptions.end()
        playing?.invoke()
    }

    // -- Round videos --------------------------------------------------------

    private val rounds = HashMap<Long, RoundVideoPlayback>()

    /**
     * The one player of [attachmentId]'s circle, made by [factory] when this
     * is its first sight — or the one a rebuilt activity left behind, still
     * playing.
     */
    fun round(attachmentId: Long, factory: VideoPlayerFactory): RoundVideoPlayback =
        rounds.getOrPut(attachmentId) { RoundVideoPlayback(attachmentId, this, factory) }

    /** The circle went for good — scrolled away, its chat closed: its player goes too (S5.3). */
    fun release(playback: RoundVideoPlayback) {
        if (rounds[playback.attachmentId] === playback) rounds.remove(playback.attachmentId)
        playback.release()
    }

    /**
     * Leaving the chat (S4): everything stops, and every circle's player that
     * nothing draws any more is let go.
     *
     * A circle still DRAWN is not the closing chat's: the next chat's circles
     * compose — and register here — before the last chat's screen leaves and
     * calls this, so they keep their players (and a rotation after a chat
     * switch still finds the one playing). The closing chat's own circles
     * let theirs go as their bubbles leave (RoundVideoPlayback.unbind, not a
     * rebuild); what this sweeps up is one a rebuild left unbound.
     */
    fun stopAll() {
        pauseAll()
        rounds.values.filterNot(RoundVideoPlayback::bound).forEach(::release)
    }

    /** The circles holding a player right now — for the tests. */
    internal val roundCount: Int get() = rounds.size
}

/**
 * The app's coordinator — the now-playing owner from Phase 2 on, provided at
 * the root by MainActivity, so threads and chats share it — or null in a
 * preview or a test that provides none (each chat then makes its own).
 */
internal val LocalPlaybackCoordinator = staticCompositionLocalOf<PlaybackCoordinator?> { null }

/**
 * Whether this chat's composer is recording — every play control is then
 * dimmed, and activating one says [explain]'s sentence instead (S1.7).
 */
@Stable
internal class RecordingGate(
    val recording: Boolean,
    val explain: () -> Unit,
)

internal val LocalRecordingGate = compositionLocalOf { RecordingGate(recording = false, explain = {}) }

/**
 * A local voice note's playback: ▶ and ❚❚, where it is, and the ONE player
 * behind it, released with the composable that remembered it.
 */
@Stable
internal class NotePlayer(
    private val file: File,
    private val scope: CoroutineScope,
    private val coordinator: PlaybackCoordinator?,
) {
    var playing by mutableStateOf(false)
        private set

    /** Where playback is, for "0:12 / 0:42". */
    var positionMs by mutableLongStateOf(0L)
        private set

    private var player: MediaPlayer? = null

    /** The same lambda every time, so the coordinator can tell this player from another. */
    val pause: () -> Unit = {
        player?.runCatching { if (isPlaying) pause() }
        playing = false
    }

    fun toggle() {
        if (playing) {
            pause()
            coordinator?.stopped(pause)
        } else {
            play()
        }
    }

    private fun play() {
        val ready = player
        if (ready != null) {
            start(ready)
            return
        }
        scope.launch {
            val created = withContext(Dispatchers.IO) {
                runCatching {
                    MediaPlayer().apply {
                        setDataSource(file.absolutePath)
                        prepare()
                    }
                }.getOrNull()
            } ?: return@launch
            created.setOnCompletionListener {
                playing = false
                positionMs = 0L
                coordinator?.stopped(pause)
            }
            player = created
            start(created)
        }
    }

    private fun start(ready: MediaPlayer) {
        runCatching { ready.start() }.onFailure { return }
        playing = true
        coordinator?.started(pause)
    }

    /** While playing: where it is, five times a second. */
    suspend fun track() {
        while (playing) {
            positionMs = player?.runCatching { currentPosition.toLong() }?.getOrNull() ?: positionMs
            delay(200)
        }
    }

    fun release() {
        coordinator?.stopped(pause)
        player?.runCatching { release() }
        player = null
        playing = false
    }
}

/**
 * A player for [file], remembered per file and released when it leaves —
 * and paused the moment a recording starts.
 */
@Composable
internal fun rememberNotePlayer(file: File): NotePlayer {
    val scope = rememberCoroutineScope()
    val coordinator = LocalPlaybackCoordinator.current
    val player = remember(file) { NotePlayer(file, scope, coordinator) }
    DisposableEffect(player) { onDispose { player.release() } }
    LaunchedEffect(player, player.playing) { if (player.playing) player.track() }
    val gate = LocalRecordingGate.current
    LaunchedEffect(player, gate.recording) { if (gate.recording && player.playing) player.pause() }
    return player
}
