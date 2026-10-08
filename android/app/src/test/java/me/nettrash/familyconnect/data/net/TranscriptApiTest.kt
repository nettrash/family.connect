/*
 * TranscriptApiTest.kt
 * Family Connect (Android)
 *
 * The transcript call (docs/protocol.md, "Transcripts on request"): its
 * SHAPE — a POST with no body, which asks the server for its own stored
 * copy; its BUDGET — a timeout of its own, at least 90 s, never the
 * ordinary 20 s; and how every answer the server can give is READ.
 *
 * On the real shared client the app is built with, with an interceptor
 * answering in place of the network.
 */

package me.nettrash.familyconnect.data.net

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.Json
import me.nettrash.familyconnect.data.repo.TranscriptOutcome
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.di.AppModule
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.FakeTokenStore
import okhttp3.Interceptor
import okhttp3.MultipartReader
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.Protocol
import okhttp3.Response
import okhttp3.ResponseBody.Companion.toResponseBody
import okio.Buffer
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import java.io.File
import java.io.IOException
import java.util.concurrent.TimeUnit

@RunWith(RobolectricTestRunner::class)
class TranscriptApiTest {

    /** What one request looked like, and the budget it ran with. */
    private data class Seen(
        val method: String,
        val path: String,
        val contentType: String?,
        val bodyBytes: Long,
        val readMs: Int,
        val callNanos: Long,
    )

    private val seen = mutableListOf<Seen>()

    /** The bytes of each request body exactly as they went on the wire. */
    private val wire = mutableListOf<ByteArray>()

    /** The next answer: a status and a body, or null for a dropped connection. */
    private var reply: Pair<Int, String>? = 200 to """{"transcript":{"text":"hello"}}"""

    private val server = Interceptor { chain ->
        val request = chain.request()
        val body = request.body
        val buffer = Buffer().also { body?.writeTo(it) }
        wire += buffer.copy().readByteArray()
        seen += Seen(
            method = request.method,
            path = request.url.encodedPath,
            contentType = body?.contentType()?.toString() ?: request.header("Content-Type"),
            bodyBytes = buffer.size,
            readMs = chain.readTimeoutMillis(),
            callNanos = chain.call().timeout().timeoutNanos(),
        )
        val (code, text) = reply ?: throw IOException("connection reset")
        Response.Builder()
            .request(request)
            .protocol(Protocol.HTTP_1_1)
            .code(code)
            .message("x")
            .body(text.toResponseBody("application/json".toMediaType()))
            .build()
    }

    private fun api(): TranscriptApi = DefaultTranscriptApi(
        ApiClient(
            context = RuntimeEnvironment.getApplication(),
            client = AppModule.provideOkHttpClient().newBuilder().addInterceptor(server).build(),
            json = Json { ignoreUnknownKeys = true },
            tokenStore = FakeTokenStore("tok"),
            settings = FakeSettingsRepository(SettingsState(serverUrl = "https://chat.example.com")),
        ),
    )

    private fun error(code: String) = """{"error":{"code":"$code","message":"whatever"}}"""

    // -- Shape ---------------------------------------------------------------------

    @Test
    fun `it posts to the attachment's transcript with no body at all`() = runTest {
        api().transcribeStored(chatId = 3, messageId = 500, attachmentId = 40)

        val request = seen.single()
        assertThat(request.method).isEqualTo("POST")
        assertThat(request.path).isEqualTo("/api/v1/chats/3/messages/500/attachments/40/transcript")
        // Anything that is not multipart/form-data is the stored-copy shape;
        // sending nothing at all is the plainest way to say so.
        assertThat(request.bodyBytes).isEqualTo(0L)
        assertThat(request.contentType.orEmpty()).doesNotContain("multipart")
    }

    // -- Budget --------------------------------------------------------------------

    @Test
    fun `it waits at least 90 seconds for its answer`() = runTest {
        api().transcribeStored(3, 500, 40)

        val budget = seen.single()
        assertThat(budget.readMs.toLong()).isAtLeast(TimeUnit.SECONDS.toMillis(90))
        // No shorter wall clock cuts it off first: none, or one as long.
        assertThat(budget.callNanos == 0L || budget.callNanos >= TimeUnit.SECONDS.toNanos(90)).isTrue()
    }

    @Test
    fun `the transcript budget honours the protocol's floor and outlasts the proxy`() {
        assertThat(ApiClient.TRANSCRIPT_TIMEOUT.seconds).isAtLeast(90L)
        // The reference nginx waits 300 s on this route: waiting past it
        // lets the proxy's own answer arrive rather than racing it.
        assertThat(ApiClient.TRANSCRIPT_TIMEOUT.seconds).isAtLeast(300L)
    }

    // -- Reading the answer ----------------------------------------------------------

    @Test
    fun `an answer with a language decodes whole`() = runTest {
        reply = 200 to """{"transcript":{"text":"Привет","language":"ru"}}"""

        val result = api().transcribeStored(3, 500, 40)

        val transcript = (result as ApiResult.Ok).value.transcript
        assertThat(transcript.text).isEqualTo("Привет")
        assertThat(transcript.language).isEqualTo("ru")
    }

    @Test
    fun `silence is an answer with empty text and no language`() = runTest {
        reply = 200 to """{"transcript":{"text":""}}"""

        val transcript = (api().transcribeStored(3, 500, 40) as ApiResult.Ok).value.transcript

        assertThat(transcript.text).isEmpty()
        assertThat(transcript.language).isNull()
    }

    private suspend fun outcomeOf(status: Int, code: String): TranscriptOutcome {
        reply = status to error(code)
        return TranscriptOutcome.ofFailure(api().transcribeStored(3, 500, 40))
    }

    @Test
    fun `a refusal by the provider's filter is terminal`() = runTest {
        assertThat(outcomeOf(400, "transcript_refused")).isEqualTo(TranscriptOutcome.Refused)
    }

    @Test
    fun `a missing consent is asked, not reported`() = runTest {
        assertThat(outcomeOf(403, "assistant_consent_required")).isEqualTo(TranscriptOutcome.ConsentRequired)
    }

    @Test
    fun `the three no-for-good answers read as not available`() = runTest {
        assertThat(outcomeOf(400, "not_transcribable")).isEqualTo(TranscriptOutcome.Unavailable)
        assertThat(outcomeOf(403, "transcript_not_allowed")).isEqualTo(TranscriptOutcome.Unavailable)
        assertThat(outcomeOf(403, "transcripts_unavailable")).isEqualTo(TranscriptOutcome.Unavailable)
    }

    @Test
    fun `a provider failure, an unknown code and a dropped connection may be tried again`() = runTest {
        assertThat(outcomeOf(500, "internal")).isEqualTo(TranscriptOutcome.Failed)
        assertThat(outcomeOf(404, "message_not_found")).isEqualTo(TranscriptOutcome.Failed)
        assertThat(outcomeOf(400, "validation")).isEqualTo(TranscriptOutcome.Failed)
        // The proxy's own 504, in a body the error shape does not cover.
        reply = 504 to "<html>gateway timeout</html>"
        assertThat(TranscriptOutcome.ofFailure(api().transcribeStored(3, 500, 40))).isEqualTo(TranscriptOutcome.Failed)
        reply = null
        val dropped = api().transcribeStored(3, 500, 40)
        assertThat(dropped).isInstanceOf(ApiResult.NetworkError::class.java)
        assertThat(TranscriptOutcome.ofFailure(dropped)).isEqualTo(TranscriptOutcome.Failed)
    }

    @Test
    fun `only the code decides, never the status alone`() {
        // Four of the codes are 403s; a 403 with no code is not a consent question.
        assertThat(TranscriptOutcome.ofFailure(ApiResult.HttpError(403, null, null)))
            .isEqualTo(TranscriptOutcome.Failed)
        assertThat(TranscriptOutcome.ofFailure(ApiResult.HttpError(403, "not_chat_member", null)))
            .isEqualTo(TranscriptOutcome.Failed)
    }
    // -- Sound this device supplies ---------------------------------------------------

    private fun soundOf(bytes: ByteArray): File =
        File.createTempFile("sound", ".m4a").apply { writeBytes(bytes); deleteOnExit() }

    @Test
    fun `supplied sound goes as multipart with one part named audio, typed audio mp4`() = runTest {
        // An MPEG-4 header the server checks for, then some sound.
        val bytes = byteArrayOf(0, 0, 0, 0x18, 'f'.code.toByte(), 't'.code.toByte(), 'y'.code.toByte(), 'p'.code.toByte()) +
            ByteArray(2_000) { (it % 251).toByte() }

        api().transcribeSupplied(chatId = 3, messageId = 500, attachmentId = 70, sound = soundOf(bytes))

        val request = seen.single()
        assertThat(request.method).isEqualTo("POST")
        assertThat(request.path).isEqualTo("/api/v1/chats/3/messages/500/attachments/70/transcript")
        assertThat(request.contentType).startsWith("multipart/form-data; boundary=")

        // Read back off the wire the way the server will.
        val boundary = request.contentType!!.substringAfter("boundary=")
        val parts = mutableListOf<Pair<okhttp3.Headers, ByteArray>>()
        MultipartReader(Buffer().write(wire.single()), boundary).use { reader ->
            while (true) {
                val part = reader.nextPart() ?: break
                parts += part.headers to part.body.readByteArray()
            }
        }
        val (headers, body) = parts.single()
        assertThat(headers["Content-Disposition"]).isEqualTo("""form-data; name="audio"; filename="sound.m4a"""")
        assertThat(headers["Content-Type"]).isEqualTo("audio/mp4")
        assertThat(body).isEqualTo(bytes)
    }

    @Test
    fun `supplied sound waits as long as the stored copy does`() = runTest {
        api().transcribeSupplied(3, 500, 70, soundOf(ByteArray(10)))

        val budget = seen.single()
        assertThat(budget.readMs.toLong()).isAtLeast(TimeUnit.SECONDS.toMillis(90))
        assertThat(budget.callNanos == 0L || budget.callNanos >= TimeUnit.SECONDS.toNanos(90)).isTrue()
    }

    @Test
    fun `a supplied answer and its refusals read like the stored copy's`() = runTest {
        reply = 200 to """{"transcript":{"text":"from the video"}}"""
        val ok = api().transcribeSupplied(3, 500, 70, soundOf(ByteArray(10)))
        assertThat((ok as ApiResult.Ok).value.transcript.text).isEqualTo("from the video")

        reply = 400 to error("not_transcribable")
        assertThat(TranscriptOutcome.ofFailure(api().transcribeSupplied(3, 500, 70, soundOf(ByteArray(10)))))
            .isEqualTo(TranscriptOutcome.Unavailable)
    }
}
