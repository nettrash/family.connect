/*
 * RoundVideoPlayback.kt
 * Family Connect (Android)
 *
 * One round video's player, and the rules a tap on its circle follows (#79,
 * docs/audio-video-messages-2026-10-04.md, S5.3):
 *
 *  - A tap PLAYS IT IN PLACE, with sound; another tap pauses; at the end it
 *    returns to the poster.
 *  - A tapped circle LOADS at once (a ring over its poster), and a second tap
 *    while it loads GIVES UP — the stream is not committed to by one touch.
 *  - A failure leaves the poster with "Couldn't load the video. Tap to try
 *    again.", and the tap tries again.
 *  - ONE THING PLAYS AT A TIME: starting reports to the now-playing owner
 *    ([PlaybackCoordinator]), which pauses whatever played before and pauses
 *    this when a recording, a call, another app's sound or pulled headphones
 *    say so (S4).
 *  - Playing marks it played ON THIS DEVICE, which takes its dot away (S5.2).
 *
 * Owned by the coordinator, keyed by attachment, NOT by the composable that
 * draws it: a rotation, a fold or a resize rebuilds MainActivity (it declares
 * no `configChanges`), and S4 says a circle plays on through that. The
 * bubble binds and unbinds; a player nobody binds again soon after an
 * unbind that said "the activity is only being rebuilt" is let go
 * ([KEEP_UNBOUND_MS]), and any other unbind lets it go at once — a circle
 * scrolled out of view or a chat closed stops (S5.3).
 *
 * The player itself is behind [VideoPlayer] — `MediaPlayer` in the app,
 * because `setDataSource(context, uri, headers)` carries the Authorization
 * header the stream needs (the reason the audio rows and the viewer use the
 * platform players too), and a fake in the tests.
 */

package me.nettrash.familyconnect.ui.chat

import android.content.Context
import android.media.AudioAttributes
import android.media.MediaPlayer
import android.view.Surface
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.core.net.toUri
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** What a round video plays through. Main thread only. */
interface VideoPlayer {
    fun setSurface(surface: Surface?)
    fun start()
    fun pause()
    fun seekTo(positionMs: Int)
    val positionMs: Int
    fun release()

    /** What a player says later, on the main thread. */
    interface Events {
        /** Prepared: it can start. */
        fun onReady()

        /** Played to the end. */
        fun onEnded()

        /** The stream could not be had, or broke. */
        fun onFailed()
    }
}

/** Opens a round video's stream. */
fun interface VideoPlayerFactory {
    /**
     * Start opening [attachmentId]: a player whose [VideoPlayer.Events.onReady]
     * or [VideoPlayer.Events.onFailed] follows — or null when it cannot even
     * begin (no address to stream from).
     */
    suspend fun open(attachmentId: Long, events: VideoPlayer.Events): VideoPlayer?
}

/** A test's factory, provided in place of the app's MediaPlayer one. */
internal val LocalVideoPlayerFactory = staticCompositionLocalOf<VideoPlayerFactory?> { null }

/** The app's [VideoPlayerFactory]: a MediaPlayer on the authenticated stream. */
class MediaPlayerVideoFactory(
    context: Context,
    private val streamUrl: suspend (Long) -> Pair<String, Map<String, String>>?,
) : VideoPlayerFactory {

    private val appContext = context.applicationContext

    override suspend fun open(attachmentId: Long, events: VideoPlayer.Events): VideoPlayer? {
        val (url, headers) = streamUrl(attachmentId) ?: return null
        return runCatching {
            // Made on the main thread, so its events arrive there too.
            val player = MediaPlayer()
            player.setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_MEDIA)
                    .setContentType(AudioAttributes.CONTENT_TYPE_MOVIE)
                    .build(),
            )
            player.setDataSource(appContext, url.toUri(), headers)
            player.setOnPreparedListener { events.onReady() }
            player.setOnCompletionListener { events.onEnded() }
            player.setOnErrorListener { _, _, _ ->
                events.onFailed()
                true
            }
            // Never `prepare()`: over a network that is a main-thread stall
            // for as long as the first bytes take.
            player.prepareAsync()
            MediaPlayerVideo(player)
        }.getOrNull()
    }

    private class MediaPlayerVideo(private val player: MediaPlayer) : VideoPlayer {
        override fun setSurface(surface: Surface?) {
            runCatching { player.setSurface(surface) }
        }

        override fun start() = player.start()
        override fun pause() {
            runCatching { if (player.isPlaying) player.pause() }
        }

        override fun seekTo(positionMs: Int) {
            runCatching { player.seekTo(positionMs) }
        }

        override val positionMs: Int get() = runCatching { player.currentPosition }.getOrDefault(0)
        override fun release() {
            runCatching { player.release() }
        }
    }
}

/** One circle's playback — see the file header. */
@Stable
class RoundVideoPlayback internal constructor(
    val attachmentId: Long,
    private val owner: PlaybackCoordinator,
    private val factory: VideoPlayerFactory,
    private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate),
) {
    enum class Phase { IDLE, LOADING, PLAYING, PAUSED, FAILED }

    var phase by mutableStateOf(Phase.IDLE)
        private set

    /** Where it is, for the ring and the capsule; 0 when idle. */
    var positionMs by mutableIntStateOf(0)
        private set

    /** A player exists (loading, or ready): the circle draws its picture surface. */
    val hasPlayer: Boolean get() = phase == Phase.LOADING || phase == Phase.PLAYING || phase == Phase.PAUSED

    private var player: VideoPlayer? = null
    private var ready = false
    private var surface: Surface? = null
    private var opening: Job? = null

    /** Bumped by every load and give-up, so a stale player's late word is ignored. */
    private var generation = 0
    private var bindings = 0

    /** The coordinator's handle — the same lambda every time, so it tells this circle from another. */
    val pause: () -> Unit = {
        if (phase == Phase.PLAYING) {
            player?.let { positionMs = it.positionMs; it.pause() }
            phase = Phase.PAUSED
        }
    }

    /** A tap on the circle (S5.3). */
    fun tap() {
        when (phase) {
            Phase.IDLE, Phase.FAILED -> if (ready && player != null) play() else load()
            Phase.LOADING -> giveUp()
            Phase.PLAYING -> {
                pause()
                owner.stopped(pause)
            }
            Phase.PAUSED -> play()
        }
    }

    /** Read the player's position — the circle polls it while playing. */
    fun track() {
        if (phase == Phase.PLAYING) player?.let { positionMs = it.positionMs }
    }

    /** The circle's picture surface came (or came back, after a rebuild). */
    fun attachSurface(attached: Surface) {
        surface = attached
        player?.setSurface(attached)
    }

    /** The surface [detached] went — only if it is still the one in use. */
    fun detachSurface(detached: Surface?) {
        if (surface !== detached) return
        surface = null
        player?.setSurface(null)
    }

    /** A composable draws this circle. */
    fun bind() {
        bindings++
    }

    /** Some composable draws it right now — a screen on show, not one that has left. */
    internal val bound: Boolean get() = bindings > 0

    /**
     * The composable went. [rebuilding] — the activity is only being rebuilt
     * around it — keeps the player for a moment so the new one can take it
     * over still playing; otherwise it stops now.
     */
    fun unbind(rebuilding: Boolean) {
        bindings = (bindings - 1).coerceAtLeast(0)
        if (bindings > 0) return
        if (!rebuilding) {
            owner.release(this)
            return
        }
        scope.launch {
            delay(KEEP_UNBOUND_MS)
            if (bindings == 0) owner.release(this@RoundVideoPlayback)
        }
    }

    /** Stop and let the player go; the circle is back at its poster. */
    internal fun release() {
        generation++
        opening?.cancel()
        opening = null
        owner.stopped(pause)
        discard()
        positionMs = 0
        phase = Phase.IDLE
    }

    private fun load() {
        discard()
        phase = Phase.LOADING
        positionMs = 0
        val mine = ++generation
        val events = object : VideoPlayer.Events {
            override fun onReady() {
                if (mine != generation) return
                ready = true
                if (phase == Phase.LOADING && player != null) play()
            }

            override fun onEnded() {
                if (mine != generation) return
                owner.stopped(pause)
                // "At the end it returns to the poster and loses its dot"
                // (S5.3): played to the END, not merely started — a second
                // heard and paused is still not played.
                owner.played.markPlayed(attachmentId)
                // Back to the poster, ready to play again from the start
                // without fetching the stream a second time.
                player?.seekTo(0)
                positionMs = 0
                phase = Phase.IDLE
            }

            override fun onFailed() {
                if (mine != generation) return
                owner.stopped(pause)
                discard()
                phase = Phase.FAILED
            }
        }
        opening = scope.launch {
            val opened = runCatching { factory.open(attachmentId, events) }.getOrNull()
            if (mine != generation) {
                opened?.release()
                return@launch
            }
            if (opened == null) {
                phase = Phase.FAILED
                return@launch
            }
            player = opened
            surface?.let(opened::setSurface)
            // Ready before it was handed over (a player that prepared at once).
            if (ready && phase == Phase.LOADING) play()
        }
    }

    private fun play() {
        val active = player ?: return
        if (runCatching { active.start() }.isFailure) {
            discard()
            phase = Phase.FAILED
            return
        }
        phase = Phase.PLAYING
        owner.started(pause)
    }

    /** The second tap while it loads: nothing plays, nothing is left open. */
    private fun giveUp() {
        generation++
        opening?.cancel()
        opening = null
        discard()
        phase = Phase.IDLE
    }

    private fun discard() {
        player?.release()
        player = null
        ready = false
    }

    companion object {
        /** How long an unbound player waits for a rebuilt activity to take it back. */
        const val KEEP_UNBOUND_MS = 3_000L
    }
}
