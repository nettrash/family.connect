/*
 * TranscriptRoutingTest.kt
 * Family Connect (Android)
 *
 * "Show text" reaches the right request (docs/protocol.md, "Transcripts on
 * request"): a voice note the server can send from its own copy is asked
 * for with no body, and nothing is taken out on the device; a video, an Ogg
 * file or a recording over the ceiling goes the supplied way — against the
 * ceiling the server states at the moment of asking.
 *
 * The whole screen-side object, over a real database, with the network and
 * the device's media code scripted.
 */

package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.db.TranscriptEntity
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.net.dto.TranscriptDto
import me.nettrash.familyconnect.data.net.dto.TranscriptResponse
import me.nettrash.familyconnect.data.repo.TranscriptOutcome
import me.nettrash.familyconnect.data.repo.TranscriptRepository
import me.nettrash.familyconnect.data.repo.TranscriptSoundPlan
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.FakeTranscriptApi
import me.nettrash.familyconnect.testutil.FakeTranscriptSound
import me.nettrash.familyconnect.testutil.createTestDb
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import java.io.File

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class TranscriptRoutingTest {

    private val dispatcher = StandardTestDispatcher()
    private lateinit var db: AppDatabase
    private val api = FakeTranscriptApi()
    private val sound = FakeTranscriptSound()
    private val settings = FakeSettingsRepository(
        SettingsState(
            myUserId = 7,
            assistantProcessor = "Microsoft — Azure OpenAI",
            assistantTranscribe = true,
            assistantTranscribeMaxBytes = MAX,
            assistantConsentAt = "2026-10-01T10:00:00Z",
        ),
    )

    @Before
    fun setUp() {
        db = createTestDb(dispatcher)
    }

    @After
    fun tearDown() {
        db.close()
    }

    private fun TestScope.transcripts() = Transcripts(
        scope = backgroundScope,
        settings = settings,
        repository = TranscriptRepository(api, db.transcriptDao(), sound),
        agree = { true },
    )

    private fun answer(text: String) = ApiResult.Ok(TranscriptResponse(TranscriptDto(text)))

    private val voiceNote = AttachmentDto(id = 40, kind = "audio", mime = "audio/mp4", size = 48_000, durationMs = 6_000)
    private val clip = AttachmentDto(id = 70, kind = "video", mime = "video/mp4", size = 40_000_000, durationMs = 90_000)

    @Test
    fun `a voice note the server can send is asked for with no body, and nothing is extracted`() = runTest(dispatcher) {
        api.answers += answer("See you at six")
        val transcripts = transcripts()

        transcripts.request(3, 500, voiceNote)
        runCurrent()

        assertThat(api.calls).containsExactly(FakeTranscriptApi.Call(3, 500, 40))
        assertThat(api.supplied).isEmpty()
        assertThat(sound.asked).isEmpty()
        assertThat(db.transcriptDao().find(40)?.source).isEqualTo(TranscriptEntity.SOURCE_STORED)
    }

    @Test
    fun `a video goes the supplied way, under the ceiling the server states`() = runTest(dispatcher) {
        api.answers += ApiResult.HttpError(400, TranscriptOutcome.NOT_TRANSCRIBABLE, "no")
        api.suppliedAnswers += answer("Look at the cake")
        sound.next = TranscriptSoundPlan.Result.Ready(
            File.createTempFile("sound", ".m4a").apply { writeBytes(ByteArray(100)) },
            TranscriptSoundPlan.Way.PASSTHROUGH,
        )
        val transcripts = transcripts()

        transcripts.request(3, 500, clip)
        runCurrent()

        assertThat(sound.asked).containsExactly(70L to MAX)
        assertThat(api.supplied.single().attachmentId).isEqualTo(70)
        assertThat(db.transcriptDao().find(70)?.text).isEqualTo("Look at the cake")
        assertThat(transcripts.status.value).isEmpty()
    }

    @Test
    fun `an ogg voice note and one over the ceiling go the supplied way too`() = runTest(dispatcher) {
        val transcripts = transcripts()
        for (attachment in listOf(voiceNote.copy(id = 41, mime = "audio/ogg"), voiceNote.copy(id = 42, size = MAX + 1))) {
            api.answers += ApiResult.HttpError(400, TranscriptOutcome.NOT_TRANSCRIBABLE, "no")
            transcripts.request(3, 500, attachment)
            runCurrent()
        }

        assertThat(sound.asked.map { it.first }).containsExactly(41L, 42L).inOrder()
    }

    @Test
    fun `a device that cannot take the sound out says it could not read it, with no retry`() = runTest(dispatcher) {
        api.answers += ApiResult.HttpError(400, TranscriptOutcome.NOT_TRANSCRIBABLE, "no")
        sound.next = TranscriptSoundPlan.Result.Unreadable
        val transcripts = transcripts()

        transcripts.request(3, 500, clip)
        runCurrent()

        assertThat(transcripts.status.value[70]).isEqualTo(TranscriptRequests.Status.UNREADABLE)
        assertThat(api.supplied).isEmpty()
    }

    private companion object {
        const val MAX = 26_214_400L
    }
}
