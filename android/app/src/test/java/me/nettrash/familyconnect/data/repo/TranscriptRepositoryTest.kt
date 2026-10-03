/*
 * TranscriptRepositoryTest.kt
 * Family Connect (Android)
 *
 * The device keeps the text it was given (docs/protocol.md, "Transcripts
 * on request"): asked once, then shown again from the database — after
 * "Hide text", after the chat is reopened, after the repository itself is
 * rebuilt — with no second request.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.db.LocalDataWiper
import me.nettrash.familyconnect.data.db.TranscriptEntity
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.TranscriptApi
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.net.dto.TranscriptDto
import me.nettrash.familyconnect.data.net.dto.TranscriptResponse
import me.nettrash.familyconnect.testutil.FakeAuthApi
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.FakeTokenStore
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
class TranscriptRepositoryTest {

    private val dispatcher = StandardTestDispatcher()
    private lateinit var db: AppDatabase
    private val api = FakeTranscriptApi()

    @Before
    fun setUp() {
        db = createTestDb(dispatcher)
    }

    @After
    fun tearDown() {
        db.close()
    }

    private val sound = FakeTranscriptSound()

    private fun repository() = TranscriptRepository(api, db.transcriptDao(), sound)

    private fun answer(text: String, language: String? = null) =
        ApiResult.Ok(TranscriptResponse(TranscriptDto(text, language)))

    @Test
    fun `the first ask goes to the server and the answer is kept, unfolded`() = runTest(dispatcher) {
        api.answers += answer("See you at six", "en")

        val outcome = repository().fetchStored(chatId = 3, messageId = 500, attachmentId = 40)

        assertThat(outcome).isEqualTo(TranscriptOutcome.Text("See you at six", "en"))
        assertThat(api.calls).containsExactly(FakeTranscriptApi.Call(3, 500, 40))
        assertThat(db.transcriptDao().find(40)).isEqualTo(
            TranscriptEntity(
                attachmentId = 40,
                text = "See you at six",
                language = "en",
                source = TranscriptEntity.SOURCE_STORED,
                hidden = false,
            ),
        )
    }

    @Test
    fun `asking again is answered from the device, with no second request`() = runTest(dispatcher) {
        api.answers += answer("See you at six")
        repository().fetchStored(3, 500, 40)

        // A NEW repository over the same database: the chat reopened.
        val again = repository().fetchStored(3, 500, 40)

        assertThat(again).isEqualTo(TranscriptOutcome.Text("See you at six", null))
        assertThat(api.calls).hasSize(1)
    }

    @Test
    fun `hide folds the text away and keeps it, and show unfolds it without asking`() = runTest(dispatcher) {
        api.answers += answer("See you at six")
        val repository = repository()
        repository.fetchStored(3, 500, 40)

        repository.setHidden(40, true)
        assertThat(repository.observe(40).first()?.hidden).isTrue()
        assertThat(repository.observe(40).first()?.text).isEqualTo("See you at six")

        // "Show text" through the request path still asks nothing.
        repository.fetchStored(3, 500, 40)
        assertThat(repository.observe(40).first()?.hidden).isFalse()
        assertThat(api.calls).hasSize(1)
    }

    @Test
    fun `silence is kept as an answer`() = runTest(dispatcher) {
        api.answers += answer("")

        val outcome = repository().fetchStored(3, 500, 40)

        assertThat(outcome).isEqualTo(TranscriptOutcome.Text("", null))
        assertThat(db.transcriptDao().find(40)?.text).isEmpty()
    }

    @Test
    fun `a failure keeps nothing, so the next ask goes to the server again`() = runTest(dispatcher) {
        api.answers += ApiResult.HttpError(500, "internal", "internal error")
        api.answers += ApiResult.HttpError(400, "transcript_refused", "refused")
        api.answers += answer("Third time")
        val repository = repository()

        assertThat(repository.fetchStored(3, 500, 40)).isEqualTo(TranscriptOutcome.Failed)
        assertThat(db.transcriptDao().find(40)).isNull()
        assertThat(repository.fetchStored(3, 500, 40)).isEqualTo(TranscriptOutcome.Refused)
        assertThat(db.transcriptDao().find(40)).isNull()
        assertThat(repository.fetchStored(3, 500, 40)).isEqualTo(TranscriptOutcome.Text("Third time", null))
        assertThat(api.calls).hasSize(3)
    }

    @Test
    fun `texts are kept per attachment`() = runTest(dispatcher) {
        api.answers += answer("first")
        api.answers += answer("second")
        val repository = repository()

        repository.fetchStored(3, 500, 40)
        repository.fetchStored(3, 500, 41)

        assertThat(db.transcriptDao().find(40)?.text).isEqualTo("first")
        assertThat(db.transcriptDao().find(41)?.text).isEqualTo("second")
    }

    @Test
    fun `a logout wipes them with everything else`() = runTest(dispatcher) {
        api.answers += answer("private words")
        repository().fetchStored(3, 500, 40)

        db.wipeAll()

        assertThat(db.transcriptDao().find(40)).isNull()
    }
    // -- Sound this device supplies: a video, an Ogg file, a recording over the ceiling --

    private val video = AttachmentDto(id = 70, kind = "video", mime = "video/mp4", size = 40_000_000, durationMs = 90_000)

    private fun notTranscribable() = ApiResult.HttpError(400, TranscriptOutcome.NOT_TRANSCRIBABLE, "no")

    private fun soundFile(bytes: Int = 4_000): File =
        File.createTempFile("sound", ".m4a").apply { writeBytes(ByteArray(bytes)) }

    @Test
    fun `allowed by the server, the sound is taken out, sent, and kept on this device only`() = runTest(dispatcher) {
        api.answers += notTranscribable()
        api.suppliedAnswers += answer("Happy birthday!", "en")
        val made = soundFile()
        sound.next = TranscriptSoundPlan.Result.Ready(made, TranscriptSoundPlan.Way.PASSTHROUGH)

        val outcome = repository().fetchSupplied(3, 500, video, MAX)

        assertThat(outcome).isEqualTo(TranscriptOutcome.Text("Happy birthday!", "en"))
        // First the empty request, which runs every check; then the sound.
        assertThat(api.calls).containsExactly(FakeTranscriptApi.Call(3, 500, 70))
        assertThat(api.supplied).containsExactly(FakeTranscriptApi.Supplied(3, 500, 70, 4_000))
        assertThat(sound.asked).containsExactly(70L to MAX)
        assertThat(db.transcriptDao().find(70)?.source).isEqualTo(TranscriptEntity.SOURCE_SUPPLIED)
        // Made for this one request.
        assertThat(made.exists()).isFalse()
    }

    @Test
    fun `a refusal from the server's checks arrives before anything is downloaded`() = runTest(dispatcher) {
        val refusals = mapOf(
            TranscriptOutcome.TRANSCRIPT_NOT_ALLOWED to TranscriptOutcome.Unavailable,
            TranscriptOutcome.TRANSCRIPTS_UNAVAILABLE to TranscriptOutcome.Unavailable,
            TranscriptOutcome.ASSISTANT_CONSENT_REQUIRED to TranscriptOutcome.ConsentRequired,
        )
        for ((code, expected) in refusals) {
            api.answers += ApiResult.HttpError(403, code, "no")
            assertThat(repository().fetchSupplied(3, 500, video, MAX)).isEqualTo(expected)
        }
        assertThat(sound.asked).isEmpty()
        assertThat(api.supplied).isEmpty()
        assertThat(db.transcriptDao().find(70)).isNull()
    }

    @Test
    fun `no answer to the first request is try again, and nothing is downloaded`() = runTest(dispatcher) {
        api.answers += ApiResult.NetworkError(java.io.IOException("offline"))

        assertThat(repository().fetchSupplied(3, 500, video, MAX)).isEqualTo(TranscriptOutcome.Failed)
        assertThat(sound.asked).isEmpty()
    }

    @Test
    fun `an answer the server already kept is taken as stored, and no sound is made`() = runTest(dispatcher) {
        api.answers += answer("Kept")

        assertThat(repository().fetchSupplied(3, 500, video, MAX)).isEqualTo(TranscriptOutcome.Text("Kept", null))
        assertThat(sound.asked).isEmpty()
        assertThat(db.transcriptDao().find(70)?.source).isEqualTo(TranscriptEntity.SOURCE_STORED)
    }

    @Test
    fun `a device that cannot read the sound says so, and sends nothing`() = runTest(dispatcher) {
        api.answers += notTranscribable()
        sound.next = TranscriptSoundPlan.Result.Unreadable

        assertThat(repository().fetchSupplied(3, 500, video, MAX)).isEqualTo(TranscriptOutcome.Unreadable)
        assertThat(api.supplied).isEmpty()
        assertThat(db.transcriptDao().find(70)).isNull()
    }

    @Test
    fun `sound that came out too long is said as too long, and sends nothing`() = runTest(dispatcher) {
        api.answers += notTranscribable()
        sound.next = TranscriptSoundPlan.Result.TooLong

        assertThat(repository().fetchSupplied(3, 500, video, MAX)).isEqualTo(TranscriptOutcome.TooLong)
        assertThat(api.supplied).isEmpty()
    }

    @Test
    fun `a recording too long by its stated length is told so without downloading it`() = runTest(dispatcher) {
        api.answers += notTranscribable()
        val twoHours = video.copy(durationMs = 2 * 60 * 60 * 1000)

        assertThat(repository().fetchSupplied(3, 500, twoHours, MAX)).isEqualTo(TranscriptOutcome.TooLong)
        // The server's checks ran first; the file was never fetched.
        assertThat(api.calls).hasSize(1)
        assertThat(sound.asked).isEmpty()
        assertThat(api.supplied).isEmpty()
    }

    @Test
    fun `a download that did not finish is try again`() = runTest(dispatcher) {
        api.answers += notTranscribable()
        sound.next = TranscriptSoundPlan.Result.NotFetched

        assertThat(repository().fetchSupplied(3, 500, video, MAX)).isEqualTo(TranscriptOutcome.Failed)
        assertThat(api.supplied).isEmpty()
    }

    @Test
    fun `a failed supplied request keeps nothing and still deletes the sound`() = runTest(dispatcher) {
        api.answers += notTranscribable()
        api.suppliedAnswers += ApiResult.HttpError(500, "internal", "provider")
        val made = soundFile()
        sound.next = TranscriptSoundPlan.Result.Ready(made, TranscriptSoundPlan.Way.REENCODE)

        assertThat(repository().fetchSupplied(3, 500, video, MAX)).isEqualTo(TranscriptOutcome.Failed)
        assertThat(db.transcriptDao().find(70)).isNull()
        assertThat(made.exists()).isFalse()
    }

    @Test
    fun `a supplied answer is shown again from the device with no request at all`() = runTest(dispatcher) {
        api.answers += notTranscribable()
        api.suppliedAnswers += answer("")
        sound.next = TranscriptSoundPlan.Result.Ready(soundFile(), TranscriptSoundPlan.Way.REENCODE)
        repository().fetchSupplied(3, 500, video, MAX)
        repository().setHidden(70, true)

        val again = repository().fetchSupplied(3, 500, video, MAX)

        // Silence is an answer, kept like any other.
        assertThat(again).isEqualTo(TranscriptOutcome.Text("", null))
        assertThat(api.calls).hasSize(1)
        assertThat(api.supplied).hasSize(1)
        assertThat(sound.asked).hasSize(1)
        assertThat(db.transcriptDao().find(70)?.hidden).isFalse()
    }

    // -- An answer belongs to the account that asked ------------------------------

    /**
     * The transcript API, answering only when [release] is completed — the
     * empty first request too, unless [holdStored] is off.
     */
    private class HeldApi(
        private val inner: FakeTranscriptApi,
        private val holdStored: Boolean = true,
    ) : TranscriptApi by inner {
        val release = CompletableDeferred<Unit>()
        override suspend fun transcribeStored(chatId: Long, messageId: Long, attachmentId: Long) =
            inner.transcribeStored(chatId, messageId, attachmentId).also { if (holdStored) release.await() }
        override suspend fun transcribeSupplied(chatId: Long, messageId: Long, attachmentId: Long, sound: File) =
            inner.transcribeSupplied(chatId, messageId, attachmentId, sound).also { release.await() }
    }

    /** The production session over this test's database, wiping it for real. */
    private fun TestScope.session(epoch: SessionEpoch): SessionRepository {
        val settings = FakeSettingsRepository()
        val tokens = FakeTokenStore()
        tokens.save("tok")
        return SessionRepository(
            authApi = FakeAuthApi(),
            tokenStore = tokens,
            settings = settings,
            wiper = LocalDataWiper { db.wipeAll() },
            unauthorizedEvents = MutableSharedFlow(),
            scope = backgroundScope,
            epoch = epoch,
        )
    }

    @Test
    fun `an answer that arrives after sign-out is neither kept nor returned`() = runTest(dispatcher) {
        val epoch = SessionEpoch()
        val session = session(epoch)
        val held = HeldApi(api)
        api.answers += answer("the previous account's words", "en")
        val repository = TranscriptRepository(held, db.transcriptDao(), sound, epoch)

        val asking = async { repository.fetchStored(3, 500, 40) }
        runCurrent()
        assertThat(api.calls).hasSize(1) // the request is out

        session.clearSession()
        held.release.complete(Unit)

        assertThat(asking.await()).isEqualTo(TranscriptOutcome.Dropped)
        assertThat(db.transcriptDao().find(40)).isNull()
        // And so nothing for the next account's line to draw.
        assertThat(repository.observe(40).first()).isNull()
    }

    @Test
    fun `a supplied answer that arrives after sign-out is neither kept nor returned`() = runTest(dispatcher) {
        val epoch = SessionEpoch()
        val session = session(epoch)
        val held = HeldApi(api, holdStored = false)
        api.answers += notTranscribable()
        api.suppliedAnswers += answer("words only the asker may read")
        val made = soundFile()
        sound.next = TranscriptSoundPlan.Result.Ready(made, TranscriptSoundPlan.Way.PASSTHROUGH)
        val repository = TranscriptRepository(held, db.transcriptDao(), sound, epoch)

        val asking = async { repository.fetchSupplied(3, 500, video, MAX) }
        runCurrent()
        assertThat(api.supplied).hasSize(1) // the sound is out

        session.clearSession()
        held.release.complete(Unit)

        assertThat(asking.await()).isEqualTo(TranscriptOutcome.Dropped)
        assertThat(db.transcriptDao().find(70)).isNull()
        assertThat(made.exists()).isFalse()
        assertThat(repository.observe(70).first()).isNull()
    }

    @Test
    fun `a request made after sign-out, by the next account, is kept as usual`() = runTest(dispatcher) {
        val epoch = SessionEpoch()
        val session = session(epoch)
        session.clearSession()
        api.answers += answer("the next account's own words")

        val outcome = TranscriptRepository(api, db.transcriptDao(), sound, epoch).fetchStored(3, 500, 40)

        assertThat(outcome).isEqualTo(TranscriptOutcome.Text("the next account's own words", null))
        assertThat(db.transcriptDao().find(40)?.text).isEqualTo("the next account's own words")
    }

    private companion object {
        const val MAX = 26_214_400L
    }
}
