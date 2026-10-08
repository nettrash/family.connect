/*
 * RoundVideoPlaybackTest.kt
 * Family Connect (Android)
 *
 * A circle's tap rules and the now-playing owner's (#79,
 * docs/audio-video-messages-2026-10-04.md, S4, S5.3), against a fake player:
 *
 *  - a tap loads and plays in place; a second tap while it loads gives up; a
 *    failure says so and the next tap tries again; at the end it is back at
 *    its poster and plays again without a second fetch;
 *  - playing marks it played on this device, which takes its dot away;
 *  - one thing plays at a time, and a recording, a lost audio focus or
 *    pulled headphones pause it — the owner holds the focus exactly while
 *    something plays;
 *  - the player belongs to the attachment: a rebuilt activity takes it back
 *    still playing, a circle scrolled away or a chat closed lets it go.
 */

package me.nettrash.familyconnect.ui.chat

import android.os.Looper
import android.view.Surface
import com.google.common.truth.Truth.assertThat
import java.time.Duration
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf

@RunWith(RobolectricTestRunner::class)
class RoundVideoPlaybackTest {

    private class FakePlayer : VideoPlayer {
        var starts = 0
        var pauses = 0
        var released = false
        var shownOn: Surface? = null
        val seeks = mutableListOf<Int>()
        override var positionMs: Int = 0
        override fun setSurface(surface: Surface?) {
            shownOn = surface
        }

        override fun start() {
            starts++
        }

        override fun pause() {
            pauses++
        }

        override fun seekTo(positionMs: Int) {
            seeks += positionMs
        }

        override fun release() {
            released = true
        }
    }

    /** Hands out players and keeps their event sinks, so a test says when they prepare. */
    private class FakeFactory(private val opens: Boolean = true) : VideoPlayerFactory {
        val players = mutableListOf<FakePlayer>()
        val events = mutableListOf<VideoPlayer.Events>()
        override suspend fun open(attachmentId: Long, events: VideoPlayer.Events): VideoPlayer? {
            if (!opens) return null
            this.events += events
            return FakePlayer().also { players += it }
        }
    }

    /** Records the focus being held and gives the test the "lost it" switch. */
    private class FakeInterruptions : AudioInterruptions {
        var held = false
        var begins = 0
        var lose: (() -> Unit)? = null
        override fun begin(onLost: () -> Unit) {
            if (!held) begins++
            held = true
            lose = onLost
        }

        override fun end() {
            held = false
        }
    }

    private val interruptions = FakeInterruptions()
    private val owner = PlaybackCoordinator(interruptions)
    private val factory = FakeFactory()

    private fun playing(id: Long = 91): RoundVideoPlayback {
        val playback = owner.round(id, factory)
        playback.bind()
        playback.tap()
        factory.events.last().onReady()
        return playback
    }

    // -- A tap ---------------------------------------------------------------------------

    @Test
    fun `a tap loads at once and plays when the stream is ready`() {
        val playback = owner.round(91, factory)
        playback.tap()

        assertThat(playback.phase).isEqualTo(RoundVideoPlayback.Phase.LOADING)
        assertThat(factory.players).hasSize(1)
        assertThat(factory.players.single().starts).isEqualTo(0)

        factory.events.single().onReady()

        assertThat(playback.phase).isEqualTo(RoundVideoPlayback.Phase.PLAYING)
        assertThat(factory.players.single().starts).isEqualTo(1)
        assertThat(interruptions.held).isTrue()
    }

    @Test
    fun `another tap pauses and the next one goes on`() {
        val playback = playing()
        val player = factory.players.single()

        playback.tap()
        assertThat(playback.phase).isEqualTo(RoundVideoPlayback.Phase.PAUSED)
        assertThat(player.pauses).isEqualTo(1)
        // Nothing plays: the focus goes back.
        assertThat(interruptions.held).isFalse()

        playback.tap()
        assertThat(playback.phase).isEqualTo(RoundVideoPlayback.Phase.PLAYING)
        assertThat(player.starts).isEqualTo(2)
        assertThat(factory.players).hasSize(1)
    }

    @Test
    fun `a second tap while it loads gives up and leaves nothing open`() {
        val playback = owner.round(91, factory)
        playback.tap()
        playback.tap()

        assertThat(playback.phase).isEqualTo(RoundVideoPlayback.Phase.IDLE)
        assertThat(factory.players.single().released).isTrue()
        // The stream answering late changes nothing.
        factory.events.single().onReady()
        assertThat(playback.phase).isEqualTo(RoundVideoPlayback.Phase.IDLE)
        assertThat(factory.players.single().starts).isEqualTo(0)
        assertThat(interruptions.held).isFalse()
    }

    @Test
    fun `a failure says so and the next tap tries again`() {
        val playback = owner.round(91, factory)
        playback.tap()
        factory.events.single().onFailed()

        assertThat(playback.phase).isEqualTo(RoundVideoPlayback.Phase.FAILED)
        assertThat(factory.players.single().released).isTrue()

        playback.tap()
        assertThat(playback.phase).isEqualTo(RoundVideoPlayback.Phase.LOADING)
        assertThat(factory.players).hasSize(2)
        factory.events.last().onReady()
        assertThat(playback.phase).isEqualTo(RoundVideoPlayback.Phase.PLAYING)
    }

    @Test
    fun `no address to stream from is a failure, not a spinner`() {
        val playback = owner.round(91, FakeFactory(opens = false))
        playback.tap()

        assertThat(playback.phase).isEqualTo(RoundVideoPlayback.Phase.FAILED)
    }

    @Test
    fun `at the end it is back at its poster and plays again without a second fetch`() {
        val playback = playing()
        val player = factory.players.single()
        player.positionMs = 23_400
        playback.track()
        assertThat(playback.positionMs).isEqualTo(23_400)

        factory.events.single().onEnded()

        assertThat(playback.phase).isEqualTo(RoundVideoPlayback.Phase.IDLE)
        assertThat(playback.positionMs).isEqualTo(0)
        assertThat(player.seeks).containsExactly(0)
        assertThat(interruptions.held).isFalse()

        playback.tap()
        assertThat(playback.phase).isEqualTo(RoundVideoPlayback.Phase.PLAYING)
        assertThat(factory.players).hasSize(1)
    }

    /** "At the end it returns to the poster and loses its dot" (S5.3) — not at the start. */
    @Test
    fun `playing to the end marks it played on this device`() {
        assertThat(owner.played.ids.value).doesNotContain(91L)
        val circle = playing(91)
        // Started, paused a second in: the dot stays.
        assertThat(owner.played.ids.value).doesNotContain(91L)
        circle.tap()
        assertThat(owner.played.ids.value).doesNotContain(91L)
        circle.tap()
        factory.events.single().onEnded()
        assertThat(owner.played.ids.value).contains(91L)
        // Loading alone does not, nor does a failure.
        owner.round(92, factory).tap()
        assertThat(owner.played.ids.value).doesNotContain(92L)
        factory.events.last().onFailed()
        assertThat(owner.played.ids.value).doesNotContain(92L)
    }

    @Test
    fun `the picture surface is handed to the player whenever both exist`() {
        val playback = owner.round(91, factory)
        val surface = Surface(android.graphics.SurfaceTexture(0))
        playback.attachSurface(surface)
        playback.tap()
        assertThat(factory.players.single().shownOn).isSameInstanceAs(surface)

        // A stale surface going does not take the current one away.
        playback.detachSurface(Surface(android.graphics.SurfaceTexture(1)))
        assertThat(factory.players.single().shownOn).isSameInstanceAs(surface)
        playback.detachSurface(surface)
        assertThat(factory.players.single().shownOn).isNull()
    }

    // -- One thing at a time, and what pauses it --------------------------------------------

    @Test
    fun `a second circle starting pauses the first`() {
        val first = playing(91)
        val second = playing(92)

        assertThat(first.phase).isEqualTo(RoundVideoPlayback.Phase.PAUSED)
        assertThat(second.phase).isEqualTo(RoundVideoPlayback.Phase.PLAYING)
        assertThat(interruptions.held).isTrue()
    }

    @Test
    fun `a voice note starting pauses a circle, and a circle a voice note`() {
        val circle = playing(91)
        var notePaused = 0
        val note: () -> Unit = { notePaused++ }

        owner.started(note)
        assertThat(circle.phase).isEqualTo(RoundVideoPlayback.Phase.PAUSED)

        circle.tap()
        assertThat(notePaused).isEqualTo(1)
    }

    @Test
    fun `losing the audio focus or the headphones pauses what plays`() {
        val circle = playing(91)

        interruptions.lose!!.invoke()

        assertThat(circle.phase).isEqualTo(RoundVideoPlayback.Phase.PAUSED)
        assertThat(interruptions.held).isFalse()
    }

    @Test
    fun `a recording starting pauses it`() {
        val circle = playing(91)
        owner.pauseAll()
        assertThat(circle.phase).isEqualTo(RoundVideoPlayback.Phase.PAUSED)
    }

    @Test
    fun `the focus is held exactly while something plays`() {
        val note: () -> Unit = {}
        assertThat(interruptions.held).isFalse()

        owner.started(note)
        owner.started(note)
        assertThat(interruptions.held).isTrue()
        assertThat(interruptions.begins).isEqualTo(1)

        // Something that is not playing stopping gives nothing back.
        owner.stopped {}
        assertThat(interruptions.held).isTrue()

        owner.stopped(note)
        assertThat(interruptions.held).isFalse()
    }

    // -- Whose player it is --------------------------------------------------------------

    @Test
    fun `a rebuilt activity takes the player back still playing`() {
        val circle = playing(91)

        circle.unbind(rebuilding = true)
        // The new activity's bubble asks for the same circle.
        val again = owner.round(91, factory)
        again.bind()
        shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(RoundVideoPlayback.KEEP_UNBOUND_MS + 1))

        assertThat(again).isSameInstanceAs(circle)
        assertThat(again.phase).isEqualTo(RoundVideoPlayback.Phase.PLAYING)
        assertThat(factory.players.single().released).isFalse()
    }

    @Test
    fun `a rebuild nobody takes back lets the player go`() {
        val circle = playing(91)

        circle.unbind(rebuilding = true)
        shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(RoundVideoPlayback.KEEP_UNBOUND_MS + 1))

        assertThat(factory.players.single().released).isTrue()
        assertThat(circle.phase).isEqualTo(RoundVideoPlayback.Phase.IDLE)
        assertThat(owner.roundCount).isEqualTo(0)
        assertThat(interruptions.held).isFalse()
    }

    @Test
    fun `scrolled out of view, it stops`() {
        val circle = playing(91)

        circle.unbind(rebuilding = false)

        assertThat(factory.players.single().released).isTrue()
        assertThat(circle.phase).isEqualTo(RoundVideoPlayback.Phase.IDLE)
        assertThat(owner.roundCount).isEqualTo(0)
        // And a later sight of it starts afresh.
        assertThat(owner.round(91, factory)).isNotSameInstanceAs(circle)
    }

    @Test
    fun `closing the chat stops everything`() {
        val first = playing(91)
        val second = playing(92)
        var notePaused = 0
        owner.started { notePaused++ }
        // The closing chat's circles leave with it — not a rebuild.
        first.unbind(rebuilding = false)
        second.unbind(rebuilding = false)
        // One left over from a rebuild nobody took back yet.
        val leftOver = playing(93)
        leftOver.unbind(rebuilding = true)

        owner.stopAll()

        assertThat(factory.players.map { it.released }).containsExactly(true, true, true)
        assertThat(first.phase).isEqualTo(RoundVideoPlayback.Phase.IDLE)
        assertThat(leftOver.phase).isEqualTo(RoundVideoPlayback.Phase.IDLE)
        assertThat(notePaused).isEqualTo(1)
        assertThat(owner.roundCount).isEqualTo(0)
        assertThat(interruptions.held).isFalse()
    }

    /**
     * Another chat: its circles are drawn — and register — BEFORE the last
     * chat's screen leaves and calls [PlaybackCoordinator.stopAll]. They are
     * not the closing chat's, so they keep their players, and one tapped
     * afterwards still plays on through a rebuild (S4's last column).
     */
    @Test
    fun `the next chat's circles survive the last chat closing`() {
        val leaving = playing(91)
        val next = owner.round(94, factory)
        next.bind()

        owner.stopAll()
        // The pause reached what played; the leaving chat's own circle then goes with its bubble.
        assertThat(leaving.phase).isEqualTo(RoundVideoPlayback.Phase.PAUSED)
        leaving.unbind(rebuilding = false)

        assertThat(owner.round(94, factory)).isSameInstanceAs(next)
        next.tap()
        factory.events.last().onReady()
        assertThat(next.phase).isEqualTo(RoundVideoPlayback.Phase.PLAYING)

        // A rotation: the rebuilt activity's bubble finds the SAME player, still playing.
        next.unbind(rebuilding = true)
        val again = owner.round(94, factory)
        again.bind()
        shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(RoundVideoPlayback.KEEP_UNBOUND_MS + 1))
        assertThat(again).isSameInstanceAs(next)
        assertThat(again.phase).isEqualTo(RoundVideoPlayback.Phase.PLAYING)
    }
}
