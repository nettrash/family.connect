/*
 * RoundChatApiTest.kt
 * Family Connect (Android)
 *
 * A video message over REST, on the real shared client (#79, docs/protocol.md,
 * "Video messages"): `POST /chats/{id}/messages` carries `"round": true` —
 * and an ordinary send carries no such key — and the `201` answer comes back
 * with the flag on its attachment. An interceptor answers in place of the
 * network: nothing here reaches any server.
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
class RoundChatApiTest {

    private val json = Json {
        ignoreUnknownKeys = true
        encodeDefaults = false
    }

    private val sent = mutableListOf<Pair<String, String>>()

    private val server = Interceptor { chain ->
        val request = chain.request()
        val buffer = Buffer().also { request.body?.writeTo(it) }
        sent += request.url.encodedPath to buffer.readUtf8()
        Response.Builder()
            .request(request)
            .protocol(Protocol.HTTP_1_1)
            .code(201)
            .message("Created")
            .body(
                """{"message": {"id": 900, "chat_id": 42, "sender_id": 7, "client_msg_id": "u1", "body": "",
                    "created_at": "2026-10-05T10:00:00Z",
                    "attachments": [{"id": 91, "kind": "video", "mime": "video/mp4", "size": 1649700,
                      "width": 480, "height": 480, "duration_ms": 23400, "has_preview": true, "round": true}]}}"""
                    .toResponseBody("application/json".toMediaType()),
            )
            .build()
    }

    private fun api() = DefaultChatApi(
        ApiClient(
            context = RuntimeEnvironment.getApplication(),
            client = AppModule.provideOkHttpClient().newBuilder().addInterceptor(server).build(),
            json = json,
            tokenStore = FakeTokenStore("tok"),
            settings = FakeSettingsRepository(SettingsState(serverUrl = "https://chat.example.com")),
        ),
    )

    @Test
    fun `a video message posts the flag and reads it back`() = runTest {
        val result = api().postMessage(
            chatId = 42, clientMsgId = "u1", body = "", replyToMessageId = 41, attachmentIds = listOf(91),
            round = true,
        )

        val (path, body) = sent.single()
        assertThat(path).isEqualTo("/api/v1/chats/42/messages")
        assertThat(json.parseToJsonElement(body)).isEqualTo(
            json.parseToJsonElement(
                """{"client_msg_id": "u1", "body": "", "reply_to_message_id": 41,
                    "attachment_ids": [91], "round": true}""",
            ),
        )
        val message = (result as ApiResult.Ok).value.message
        assertThat(message.resolvedAttachments.single().isRound).isTrue()
    }

    @Test
    fun `an ordinary send carries no round key`() = runTest {
        api().postMessage(chatId = 42, clientMsgId = "u1", body = "", attachmentIds = listOf(91))

        assertThat(sent.single().second).doesNotContain("round")
    }
}
