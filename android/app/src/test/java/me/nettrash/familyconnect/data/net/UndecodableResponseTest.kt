/*
 * UndecodableResponseTest.kt
 * Family Connect (Android)
 *
 * `ApiClient.decode` reports a 2xx it cannot read as a NetworkError — the
 * same type as a dead network — so a caller that must tell the two apart
 * (`MessageRepository.repairUnknownRoundFlags`: an unreadable answer is about
 * the ONE message asked for, a dead network is about every further read) reads
 * `isUndecodable`. Pinned here on the real shared client; an interceptor
 * answers in place of the network, so nothing reaches any server.
 */

package me.nettrash.familyconnect.data.net

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.Json
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.di.AppModule
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.FakeTokenStore
import okhttp3.Interceptor
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.Protocol
import okhttp3.Response
import okhttp3.ResponseBody.Companion.toResponseBody
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import java.io.IOException

@RunWith(RobolectricTestRunner::class)
class UndecodableResponseTest {

    private val json = Json {
        ignoreUnknownKeys = true
        encodeDefaults = false
    }

    private fun answering(body: String) = Interceptor { chain ->
        Response.Builder()
            .request(chain.request())
            .protocol(Protocol.HTTP_1_1)
            .code(200)
            .message("OK")
            .body(body.toResponseBody("application/json".toMediaType()))
            .build()
    }

    private fun api(interceptor: Interceptor, serverUrl: String? = "https://chat.example.com") = DefaultChatApi(
        ApiClient(
            context = RuntimeEnvironment.getApplication(),
            client = AppModule.provideOkHttpClient().newBuilder().addInterceptor(interceptor).build(),
            json = json,
            tokenStore = FakeTokenStore("tok"),
            settings = FakeSettingsRepository(SettingsState(serverUrl = serverUrl)),
        ),
    )

    private suspend fun read(api: ChatApi) = api.messages(42, beforeId = 51, limit = 1)

    @Test
    fun `a 2xx that is not json is undecodable`() = runTest {
        val result = read(api(answering("<html>gateway says hi</html>")))

        assertThat((result as ApiResult.NetworkError).isUndecodable).isTrue()
    }

    @Test
    fun `a 2xx of the wrong shape is undecodable`() = runTest {
        val result = read(api(answering("""{"messages": [{"id": "fifty"}]}""")))

        assertThat((result as ApiResult.NetworkError).isUndecodable).isTrue()
    }

    @Test
    fun `a readable 2xx still decodes`() = runTest {
        val result = read(api(answering("""{"messages": []}""")))

        assertThat((result as ApiResult.Ok).value.messages).isEmpty()
    }

    @Test
    fun `a dead network is not undecodable`() = runTest {
        val result = read(api(Interceptor { throw IOException("offline") }))

        assertThat((result as ApiResult.NetworkError).isUndecodable).isFalse()
    }

    @Test
    fun `no server configured is not undecodable`() = runTest {
        val result = read(api(answering("""{"messages": []}"""), serverUrl = null))

        assertThat((result as ApiResult.NetworkError).isUndecodable).isFalse()
    }
}
