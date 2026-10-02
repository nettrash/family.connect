/*
 * TranscriptRequestsTest.kt
 * Family Connect (Android)
 *
 * "Show text" sends a recording's sound to the provider, so it asks the
 * consent a `/draw` asks — before, when this device knows the asker has not
 * agreed, and on the server's `assistant_consent_required` when it did not
 * know — and agreeing asks for the text again, as a backdrop does
 * (docs/protocol.md, "Transcripts on request"). And the line under the
 * player says what each answer means.
 */

package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.repo.TranscriptOutcome
import me.nettrash.familyconnect.ui.chat.AssistantConsent.TranscriptGate
import me.nettrash.familyconnect.ui.chat.TranscriptRequests.Ask
import me.nettrash.familyconnect.ui.chat.TranscriptRequests.Status
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class TranscriptRequestsTest {

    private val voiceNote = Ask(chatId = 3, messageId = 500, attachmentId = 40)

    /** A scripted server and consent record, and what each was asked. */
    private class World {
        var gate = TranscriptGate.ASK_FOR_TEXT
        val answers = ArrayDeque<TranscriptOutcome>()
        val fetched = mutableListOf<Ask>()
        var hold: CompletableDeferred<Unit>? = null
        var agreeWorks = true
        var agreements = 0
    }

    private fun TestScope.requests(world: World) = TranscriptRequests(
        scope = this,
        gate = { world.gate },
        fetch = { ask ->
            world.fetched += ask
            world.hold?.await()
            world.answers.removeFirstOrNull() ?: TranscriptOutcome.Failed
        },
        agree = {
            world.agreements += 1
            if (world.agreeWorks) world.gate = TranscriptGate.ASK_FOR_TEXT
            world.agreeWorks
        },
    )

    @Test
    fun `an asker who has agreed is answered, and the line has nothing more to say`() = runTest {
        val world = World().apply { answers += TranscriptOutcome.Text("hello", "en") }
        val requests = requests(world)

        requests.request(voiceNote)
        runCurrent()

        assertThat(world.fetched).containsExactly(voiceNote)
        // The text is the database's to draw now.
        assertThat(requests.status.value).isEmpty()
        assertThat(requests.asking.first()).isFalse()
    }

    @Test
    fun `while it runs the line says it is getting the text, and a second tap asks nothing`() = runTest {
        val world = World().apply {
            hold = CompletableDeferred()
            answers += TranscriptOutcome.Text("hello", null)
        }
        val requests = requests(world)

        requests.request(voiceNote)
        runCurrent()
        assertThat(requests.status.value[40]).isEqualTo(Status.LOADING)

        requests.request(voiceNote)
        runCurrent()
        assertThat(world.fetched).hasSize(1)

        world.hold!!.complete(Unit)
        runCurrent()
        assertThat(requests.status.value).isEmpty()
    }

    @Test
    fun `an asker who has not agreed is asked first, and nothing is sent`() = runTest {
        val world = World().apply { gate = TranscriptGate.ASK_CONSENT }
        val requests = requests(world)

        requests.request(voiceNote)
        runCurrent()

        assertThat(requests.asking.first()).isTrue()
        assertThat(world.fetched).isEmpty()
    }

    @Test
    fun `agreeing records it and asks for the text that was waiting`() = runTest {
        val world = World().apply {
            gate = TranscriptGate.ASK_CONSENT
            answers += TranscriptOutcome.Text("hello", null)
        }
        val requests = requests(world)
        requests.request(voiceNote)
        runCurrent()

        requests.agreed()
        runCurrent()

        assertThat(world.agreements).isEqualTo(1)
        assertThat(world.fetched).containsExactly(voiceNote)
        assertThat(requests.asking.first()).isFalse()
        assertThat(requests.status.value).isEmpty()
    }

    @Test
    fun `a consent that is not recorded sends nothing and says it failed`() = runTest {
        val world = World().apply {
            gate = TranscriptGate.ASK_CONSENT
            agreeWorks = false
        }
        val requests = requests(world)
        requests.request(voiceNote)
        runCurrent()

        requests.agreed()
        runCurrent()

        assertThat(world.fetched).isEmpty()
        assertThat(requests.status.value[40]).isEqualTo(Status.FAILED)
    }

    @Test
    fun `not now sends nothing and says nothing`() = runTest {
        val world = World().apply { gate = TranscriptGate.ASK_CONSENT }
        val requests = requests(world)
        requests.request(voiceNote)
        runCurrent()

        requests.dismissed()
        runCurrent()

        assertThat(requests.asking.first()).isFalse()
        assertThat(world.fetched).isEmpty()
        assertThat(requests.status.value).isEmpty()
        // A late "I Agree" finds nothing waiting.
        requests.agreed()
        runCurrent()
        assertThat(world.agreements).isEqualTo(0)
    }

    @Test
    fun `the server's consent question is asked, then the same recording again`() = runTest {
        // This device thought the asker had agreed; the server says not
        // (a consent withdrawn on another device).
        val world = World().apply {
            answers += TranscriptOutcome.ConsentRequired
            answers += TranscriptOutcome.Text("hello", null)
        }
        val requests = requests(world)

        requests.request(voiceNote)
        runCurrent()
        assertThat(requests.asking.first()).isTrue()
        assertThat(requests.status.value).isEmpty()

        requests.agreed()
        runCurrent()

        assertThat(world.fetched).containsExactly(voiceNote, voiceNote).inOrder()
        assertThat(requests.asking.first()).isFalse()
    }

    @Test
    fun `a consent question with nobody to name is a failure, not a screen`() = runTest {
        val world = World().apply { answers += TranscriptOutcome.ConsentRequired }
        val requests = TranscriptRequests(
            scope = this,
            gate = { if (world.fetched.isEmpty()) TranscriptGate.ASK_FOR_TEXT else TranscriptGate.WITHHELD },
            fetch = { ask ->
                world.fetched += ask
                world.answers.removeFirst()
            },
            agree = { true },
        )

        requests.request(voiceNote)
        runCurrent()

        assertThat(requests.asking.first()).isFalse()
        assertThat(requests.status.value[40]).isEqualTo(Status.FAILED)
    }

    @Test
    fun `each answer says what it means`() = runTest {
        val world = World().apply {
            answers += TranscriptOutcome.Refused
            answers += TranscriptOutcome.Unavailable
            answers += TranscriptOutcome.Failed
            answers += TranscriptOutcome.TooLong
            answers += TranscriptOutcome.Unreadable
        }
        val requests = requests(world)

        requests.request(Ask(3, 500, 40))
        requests.request(Ask(3, 501, 41))
        requests.request(Ask(3, 502, 42))
        requests.request(Ask(3, 503, 43))
        requests.request(Ask(3, 504, 44))
        runCurrent()

        assertThat(requests.status.value).containsExactly(
            40L, Status.REFUSED,
            41L, Status.UNAVAILABLE,
            42L, Status.FAILED,
            43L, Status.TOO_LONG,
            44L, Status.UNREADABLE,
        )
    }

    @Test
    fun `a failure may be tried again, and the text then replaces the failure`() = runTest {
        val world = World().apply {
            answers += TranscriptOutcome.Failed
            answers += TranscriptOutcome.Text("hello", null)
        }
        val requests = requests(world)

        requests.request(voiceNote)
        runCurrent()
        assertThat(requests.status.value[40]).isEqualTo(Status.FAILED)

        requests.request(voiceNote)
        runCurrent()
        assertThat(requests.status.value).isEmpty()
        assertThat(world.fetched).hasSize(2)
    }

    @Test
    fun `the gate follows the consent record and the processor`() {
        assertThat(AssistantConsent.transcriptGate("Azure", "2026-10-01T00:00:00Z"))
            .isEqualTo(TranscriptGate.ASK_FOR_TEXT)
        assertThat(AssistantConsent.transcriptGate("Azure", null)).isEqualTo(TranscriptGate.ASK_CONSENT)
        assertThat(AssistantConsent.transcriptGate(null, "2026-10-01T00:00:00Z")).isEqualTo(TranscriptGate.WITHHELD)
        assertThat(AssistantConsent.transcriptGate(" ", null)).isEqualTo(TranscriptGate.WITHHELD)
    }
}
