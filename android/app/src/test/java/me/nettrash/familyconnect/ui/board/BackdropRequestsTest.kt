package me.nettrash.familyconnect.ui.board

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.repo.BackdropOutcome
import me.nettrash.familyconnect.testutil.FakeAttachmentApi
import me.nettrash.familyconnect.ui.chat.AssistantConsent.BackdropGate
import org.junit.Test

/**
 * An event's backdrop sends the author's title to the model, so it asks the
 * consent a `/draw` asks — before, when this device knows the author has
 * not agreed, and on the server's `assistant_consent_required` when it
 * did not know (docs/protocol.md, "Consenting to the assistant", amended
 * 2026-09-30). Agreeing finishes the backdrop already asked for.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class BackdropRequestsTest {

    private val picture = FakeAttachmentApi.attachment(id = 905L)

    /** A scripted server and consent record, and what each was asked. */
    private class World {
        var gate = BackdropGate.DRAW
        val answers = ArrayDeque<BackdropOutcome>()
        val drawn = mutableListOf<Long>()
        var agreeWorks = true
        var agreements = 0
        val settled = mutableListOf<BackdropOutcome>()
    }

    private fun TestScope.requests(world: World) = BackdropRequests(
        scope = this,
        gate = { world.gate },
        draw = { noteId ->
            world.drawn += noteId
            world.answers.removeFirstOrNull() ?: BackdropOutcome.Failed
        },
        agree = {
            world.agreements += 1
            if (world.agreeWorks) world.gate = BackdropGate.DRAW
            world.agreeWorks
        },
    )

    @Test
    fun `an author who has agreed is drawn for at once`() = runTest {
        val world = World().apply { answers += BackdropOutcome.Drawn(picture) }
        val requests = requests(world)

        requests.request(5L) { world.settled += it }
        runCurrent()

        assertThat(world.drawn).containsExactly(5L)
        assertThat(world.settled).containsExactly(BackdropOutcome.Drawn(picture))
        assertThat(requests.asking.first()).isFalse()
    }

    @Test
    fun `an author who has not agreed is asked first, and nothing is sent`() = runTest {
        val world = World().apply { gate = BackdropGate.ASK }
        val requests = requests(world)

        requests.request(5L) { world.settled += it }
        runCurrent()

        assertThat(requests.asking.first()).isTrue()
        assertThat(world.drawn).isEmpty()
        // Still waiting on the answer: the button keeps saying so.
        assertThat(world.settled).isEmpty()
    }

    @Test
    fun `agreeing records it and draws the backdrop that was waiting`() = runTest {
        val world = World().apply {
            gate = BackdropGate.ASK
            answers += BackdropOutcome.Drawn(picture)
        }
        val requests = requests(world)
        requests.request(5L) { world.settled += it }
        runCurrent()

        requests.agreed()
        // A second tap on "I Agree" finds nothing left to agree to.
        requests.agreed()
        runCurrent()

        assertThat(world.agreements).isEqualTo(1)
        assertThat(world.drawn).containsExactly(5L)
        assertThat(world.settled).containsExactly(BackdropOutcome.Drawn(picture))
        assertThat(requests.asking.first()).isFalse()
    }

    @Test
    fun `not now sends nothing and settles quietly`() = runTest {
        val world = World().apply { gate = BackdropGate.ASK }
        val requests = requests(world)
        requests.request(5L) { world.settled += it }
        runCurrent()

        requests.dismissed()
        runCurrent()

        assertThat(world.agreements).isEqualTo(0)
        assertThat(world.drawn).isEmpty()
        assertThat(world.settled).containsExactly(BackdropOutcome.Declined)
        assertThat(requests.asking.first()).isFalse()
    }

    /**
     * This device thought the author had agreed; the server says not (a
     * consent withdrawn on another device). The server's answer wins: the
     * consent screen, then the backdrop again on a yes.
     */
    @Test
    fun `the server's consent refusal asks, then draws again on a yes`() = runTest {
        val world = World().apply {
            answers += BackdropOutcome.ConsentRequired
            answers += BackdropOutcome.Drawn(picture)
        }
        val requests = requests(world)

        requests.request(5L) { world.settled += it }
        runCurrent()

        assertThat(requests.asking.first()).isTrue()
        assertThat(world.settled).isEmpty()

        requests.agreed()
        runCurrent()

        assertThat(world.drawn).containsExactly(5L, 5L).inOrder()
        assertThat(world.settled).containsExactly(BackdropOutcome.Drawn(picture))
    }

    @Test
    fun `a consent that could not be recorded draws nothing and says it failed`() = runTest {
        val world = World().apply {
            gate = BackdropGate.ASK
            agreeWorks = false
        }
        val requests = requests(world)
        requests.request(5L) { world.settled += it }
        runCurrent()

        requests.agreed()
        runCurrent()

        assertThat(world.drawn).isEmpty()
        assertThat(world.settled).containsExactly(BackdropOutcome.Failed)
    }

    /** No named assistant, no screen to ask on: settled, never left hanging. */
    @Test
    fun `an assistant nobody can name is never asked and never drawn for`() = runTest {
        val world = World().apply { gate = BackdropGate.WITHHELD }
        val requests = requests(world)

        requests.request(5L) { world.settled += it }
        runCurrent()

        assertThat(world.drawn).isEmpty()
        assertThat(requests.asking.first()).isFalse()
        assertThat(world.settled).containsExactly(BackdropOutcome.Failed)
    }

    /** Every other answer is handed straight back, the refusal included. */
    @Test
    fun `a refusal or a failure is handed back as it came`() = runTest {
        val world = World().apply {
            answers += BackdropOutcome.Refused
            answers += BackdropOutcome.Failed
        }
        val requests = requests(world)

        requests.request(5L) { world.settled += it }
        requests.request(6L) { world.settled += it }
        runCurrent()

        assertThat(world.settled)
            .containsExactly(BackdropOutcome.Refused, BackdropOutcome.Failed).inOrder()
        assertThat(requests.asking.first()).isFalse()
    }
}
