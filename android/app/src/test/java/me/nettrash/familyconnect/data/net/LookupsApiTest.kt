/*
 * LookupsApiTest.kt
 * Family Connect (Android)
 *
 * The two calls "Looking things up" adds (docs/protocol.md): the member's
 * `POST /me/assistant-lookup-consent {"granted": bool}` and the owner's
 * `PATCH /families/mine {"ai_lookups": bool}` — their exact shape on the
 * wire, and how each answer the server can give is read.
 *
 * On the real shared client the app is built with, with an interceptor
 * answering in place of the network: nothing here reaches any server.
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
import okio.Buffer
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

@RunWith(RobolectricTestRunner::class)
class LookupsApiTest {

    private data class Seen(val method: String, val path: String, val body: String)

    private val seen = mutableListOf<Seen>()

    private var reply: Pair<Int, String> = 200 to """{"assistant_lookup_consent_at":"2026-10-03T09:00:00Z"}"""

    private val server = Interceptor { chain ->
        val request = chain.request()
        val buffer = Buffer().also { request.body?.writeTo(it) }
        seen += Seen(request.method, request.url.encodedPath, buffer.readUtf8())
        val (code, text) = reply
        Response.Builder()
            .request(request)
            .protocol(Protocol.HTTP_1_1)
            .code(code)
            .message("x")
            .body(text.toResponseBody("application/json".toMediaType()))
            .build()
    }

    private fun client() = ApiClient(
        context = RuntimeEnvironment.getApplication(),
        client = AppModule.provideOkHttpClient().newBuilder().addInterceptor(server).build(),
        // The house encoder: defaults are NOT written, which is what keeps a
        // PATCH to the one key it names.
        json = Json {
            ignoreUnknownKeys = true
            encodeDefaults = false
        },
        tokenStore = FakeTokenStore("tok"),
        settings = FakeSettingsRepository(SettingsState(serverUrl = "https://chat.example.com")),
    )

    private fun error(code: String) = """{"error":{"code":"$code","message":"whatever"}}"""

    // -- The member's consent ------------------------------------------------------

    @Test
    fun `granting posts granted true to the lookup endpoint and reads the stamp`() = runTest {
        val result = DefaultAuthApi(client()).setAssistantLookupConsent(true)

        val request = seen.single()
        assertThat(request.method).isEqualTo("POST")
        assertThat(request.path).isEqualTo("/api/v1/me/assistant-lookup-consent")
        assertThat(request.body).isEqualTo("""{"granted":true}""")
        assertThat((result as ApiResult.Ok).value.assistantLookupConsentAt).isEqualTo("2026-10-03T09:00:00Z")
    }

    @Test
    fun `withdrawing posts granted false and reads the null`() = runTest {
        reply = 200 to """{"assistant_lookup_consent_at":null}"""

        val result = DefaultAuthApi(client()).setAssistantLookupConsent(false)

        assertThat(seen.single().body).isEqualTo("""{"granted":false}""")
        assertThat((result as ApiResult.Ok).value.assistantLookupConsentAt).isNull()
    }

    @Test
    fun `it is a different endpoint from the assistant consent`() = runTest {
        reply = 200 to """{"assistant_consent_at":"2026-09-19T19:34:43Z"}"""

        DefaultAuthApi(client()).setAssistantConsent(true)

        assertThat(seen.single().path).isEqualTo("/api/v1/me/assistant-consent")
    }

    @Test
    fun `granting without the assistant consent is the server's refusal, read as such`() = runTest {
        reply = 403 to error("assistant_consent_required")

        val result = DefaultAuthApi(client()).setAssistantLookupConsent(true)

        assertThat((result as ApiResult.HttpError).code).isEqualTo("assistant_consent_required")
        assertThat(result.status).isEqualTo(403)
    }

    @Test
    fun `a server with no lookup source answers not found`() = runTest {
        reply = 404 to error("not_found")

        val result = DefaultAuthApi(client()).setAssistantLookupConsent(true)

        assertThat((result as ApiResult.HttpError).status).isEqualTo(404)
    }

    // -- The owner's switch ----------------------------------------------------------

    @Test
    fun `the switch patches the family with exactly that key and reads the answer`() = runTest {
        reply = 200 to """{"family":{"id":3,"name":"The Smiths","join_policy":"open","ai_lookups":true}}"""

        val result = DefaultFamilyApi(client()).setAiLookups(true)

        val request = seen.single()
        assertThat(request.method).isEqualTo("PATCH")
        assertThat(request.path).isEqualTo("/api/v1/families/mine")
        assertThat(request.body).isEqualTo("""{"ai_lookups":true}""")
        assertThat((result as ApiResult.Ok).value.family.aiLookups).isTrue()
    }

    @Test
    fun `a non-owner is refused`() = runTest {
        reply = 403 to error("not_family_owner")

        val result = DefaultFamilyApi(client()).setAiLookups(false)

        assertThat(seen.single().body).isEqualTo("""{"ai_lookups":false}""")
        assertThat((result as ApiResult.HttpError).code).isEqualTo("not_family_owner")
    }
}
