/*
 * BackdropTimeoutTest.kt
 * Family Connect (Android)
 *
 * An event's backdrop waits on the model — a picture, or a picture, a
 * rewrite and a second picture one after another — so it gets a timeout of
 * its OWN, no shorter than 90 s, never the ordinary 20 s every other call
 * keeps (docs/protocol.md, "Board", amended 2026-09-30). Under the shared
 * ceiling the client gave up on backdrops the server was still drawing,
 * and a request whose connection closes first draws nothing.
 *
 * Pinned on the budget each call actually RUNS with, read by an
 * interceptor that answers in place of the network, on the real shared
 * client the app is built with.
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
import java.util.concurrent.TimeUnit

@RunWith(RobolectricTestRunner::class)
class BackdropTimeoutTest {

    /** What one request ran with. */
    private data class Budget(val path: String, val readMs: Int, val callNanos: Long)

    private val seen = mutableListOf<Budget>()

    private val recorder = Interceptor { chain ->
        seen += Budget(
            path = chain.request().url.encodedPath,
            readMs = chain.readTimeoutMillis(),
            callNanos = chain.call().timeout().timeoutNanos(),
        )
        // Any answer will do: what is pinned is the budget, not the note.
        Response.Builder()
            .request(chain.request())
            .protocol(Protocol.HTTP_1_1)
            .code(500)
            .message("Internal Server Error")
            .body(
                """{"error":{"code":"internal","message":"internal error"}}"""
                    .toResponseBody("application/json".toMediaType()),
            )
            .build()
    }

    private fun boardApi(): BoardApi = DefaultBoardApi(
        ApiClient(
            context = RuntimeEnvironment.getApplication(),
            client = AppModule.provideOkHttpClient().newBuilder().addInterceptor(recorder).build(),
            json = Json { ignoreUnknownKeys = true },
            tokenStore = FakeTokenStore("tok"),
            settings = FakeSettingsRepository(SettingsState(serverUrl = "https://chat.example.com")),
        ),
    )

    @Test
    fun `a backdrop waits at least 90 seconds for its answer`() = runTest {
        boardApi().drawBackdrop(5L)

        val budget = seen.single()
        assertThat(budget.path).endsWith("/families/mine/board/notes/5/backdrop")
        // The answer arrives all at once when the drawing is done, so the
        // read is the wait: never shorter than the protocol's floor.
        assertThat(budget.readMs.toLong()).isAtLeast(TimeUnit.SECONDS.toMillis(90))
        // And no shorter wall clock cuts it off first: either none, or one
        // at least as long.
        assertThat(budget.callNanos == 0L || budget.callNanos >= TimeUnit.SECONDS.toNanos(90))
            .isTrue()
    }

    @Test
    fun `every other call keeps the ordinary budget`() = runTest {
        val api = boardApi()
        api.tickTask(5L, 11L, done = true)
        api.deleteNote(5L)

        assertThat(seen).hasSize(2)
        for (budget in seen) {
            assertThat(budget.readMs.toLong()).isEqualTo(TimeUnit.SECONDS.toMillis(30))
            assertThat(budget.callNanos).isEqualTo(TimeUnit.SECONDS.toNanos(20))
        }
    }

    @Test
    fun `the backdrop budget honours the protocol's floor`() {
        assertThat(ApiClient.BACKDROP_TIMEOUT.seconds).isAtLeast(90L)
    }
}
