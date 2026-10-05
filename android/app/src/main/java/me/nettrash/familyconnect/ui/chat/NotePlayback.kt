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
 * Both live with the chat screen, provided through composition locals so the
 * bubbles' audio rows (ui/components/Attachments.kt) take part without
 * threading parameters through every bubble. This is the smallest honest
 * version of S5.3's rule: Phase 2's now-playing owner, living outside the
 * activity, replaces the coordinator — and adds audio focus and "becoming
 * noisy" for received notes.
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

/** One thing of this chat's plays at a time (S5.3), and a recording pauses it (S1.7). */
@Stable
internal class PlaybackCoordinator {

    private var current: (() -> Unit)? = null

    /** [pause] has started playing: whatever played before it pauses. */
    fun started(pause: () -> Unit) {
        val previous = current
        current = pause
        if (previous != null && previous !== pause) previous()
    }

    /** [pause]'s player stopped on its own, or went away. */
    fun stopped(pause: () -> Unit) {
        if (current === pause) current = null
    }

    /** A recording starts: nothing the app plays runs over it. */
    fun pauseAll() {
        val playing = current
        current = null
        playing?.invoke()
    }
}

/** The chat's coordinator, or null outside a chat (a thread plays as it always has). */
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
