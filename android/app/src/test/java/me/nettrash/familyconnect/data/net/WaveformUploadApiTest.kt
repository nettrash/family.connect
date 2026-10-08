/*
 * WaveformUploadApiTest.kt
 * Family Connect (Android)
 *
 * A voice note's waveform on the wire, on the real shared client (#79;
 * docs/protocol.md, "A voice note's waveform"): `POST /attachments` carries
 * `waveform=<48 lowercase hex digits>` on an audio upload — and on nothing
 * else, and never a malformed one, which the server would refuse with
 * `validation` and so lose the note — and the answer's echo is read back
 * onto the attachment. An answer WITHOUT the key (a server from before
 * waveforms, which ignores the unknown parameter) is an ordinary success.
 * An interceptor answers in place of the network: nothing reaches a server.
 */

package me.nettrash.familyconnect.data.net

import com.google.common.truth.Truth.assertThat
import java.io.File
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.Json
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.di.AppModule
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.FakeTokenStore
import okhttp3.Interceptor
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.Protocol
import okhttp3.Response
import okhttp3.ResponseBody.Companion.toResponseBody
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

@RunWith(RobolectricTestRunner::class)
class WaveformUploadApiTest {

    @get:Rule
    val folder = TemporaryFolder()

    private val json = Json {
        ignoreUnknownKeys = true
        encodeDefaults = false
    }

    private val queries = mutableListOf<String?>()

    /** What the server echoes, or nothing at all (an older server). */
    private var echo: String? = WAVE

    private val server = Interceptor { chain ->
        val request = chain.request()
        queries += request.url.encodedQuery
        val waveformKey = echo?.let { """, "waveform": "$it"""" } ?: ""
        Response.Builder()
            .request(request)
            .protocol(Protocol.HTTP_1_1)
            .code(201)
            .message("Created")
            .body(
                """{"attachment": {"id": 77, "kind": "audio", "mime": "audio/mp4", "size": 4096,
                    "duration_ms": 12000, "has_preview": false$waveformKey}}"""
                    .toResponseBody("application/json".toMediaType()),
            )
            .build()
    }

    private fun api() = DefaultAttachmentApi(
        ApiClient(
            context = RuntimeEnvironment.getApplication(),
            client = AppModule.provideOkHttpClient().newBuilder().addInterceptor(server).build(),
            json = json,
            tokenStore = FakeTokenStore("tok"),
            settings = FakeSettingsRepository(SettingsState(serverUrl = "https://chat.example.com")),
        ),
    )

    private var made = 0

    private fun note(): File = folder.newFile("voice-${made++}.m4a").apply { writeBytes(ByteArray(4096) { 1 }) }

    @Test
    fun `a voice note's upload carries its waveform and reads the echo back`() = runTest {
        val result = api().upload(
            file = note(), mime = "audio/mp4", kind = AttachmentDto.KIND_AUDIO,
            width = null, height = null, durationMs = 12_000, waveform = WAVE,
        )

        assertThat(queries.single()).isEqualTo("kind=audio&duration_ms=12000&waveform=$WAVE")
        assertThat((result as ApiResult.Ok).value.attachment.waveform).isEqualTo(WAVE)
    }

    @Test
    fun `a server from before waveforms answers without one, and that is a success`() = runTest {
        echo = null

        val result = api().upload(
            file = note(), mime = "audio/mp4", kind = AttachmentDto.KIND_AUDIO,
            width = null, height = null, durationMs = 12_000, waveform = WAVE,
        )

        val attachment = (result as ApiResult.Ok).value.attachment
        assertThat(attachment.id).isEqualTo(77)
        assertThat(attachment.waveform).isNull()
    }

    @Test
    fun `no waveform, a malformed one, or one on anything but audio is never sent`() = runTest {
        val upload: suspend (String, String?) -> Unit = { kind, waveform ->
            api().upload(
                file = note(),
                mime = "audio/mp4", kind = kind, width = null, height = null, durationMs = 12_000,
                waveform = waveform,
            )
        }
        upload(AttachmentDto.KIND_AUDIO, null)
        upload(AttachmentDto.KIND_AUDIO, WAVE.uppercase())
        upload(AttachmentDto.KIND_AUDIO, WAVE.dropLast(1))
        upload(AttachmentDto.KIND_VIDEO, WAVE)
        upload(AttachmentDto.KIND_FILE, WAVE)

        assertThat(queries).hasSize(5)
        queries.forEach { assertThat(it).doesNotContain("waveform") }
    }

    private companion object {
        const val WAVE = "0124689abcddeeedcba987654321001245678aabbba98642"
    }
}
