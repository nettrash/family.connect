/*
 * MessageThreadDtoTest.kt
 * Family Connect (Android) — tests
 *
 * The chain's two wire fields (docs/protocol.md, "Threads"): absent — never
 * null or 0 — on a message that is neither a reply nor a root somebody
 * answered, and a server that predates threads sends neither, which must
 * decode as "no chain" rather than fail.
 */

package me.nettrash.familyconnect.data.net.dto

import com.google.common.truth.Truth.assertThat
import kotlinx.serialization.json.Json
import org.junit.Test

class MessageThreadDtoTest {

    private val json = Json { ignoreUnknownKeys = true }

    @Test
    fun `a message with neither field decodes as no chain`() {
        val message = json.decodeFromString<MessageDto>(
            """
            {"id": 1338, "chat_id": 42, "sender_id": 7,
             "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
             "body": "Dinner at 7?", "created_at": "2026-08-19T17:03:12Z"}
            """.trimIndent(),
        )
        assertThat(message.threadRootId).isNull()
        assertThat(message.replyCount).isNull()
    }

    @Test
    fun `a reply names its root and a root carries its count`() {
        val reply = json.decodeFromString<MessageDto>(
            """
            {"id": 1339, "chat_id": 42, "sender_id": 9,
             "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a02",
             "body": "Works for me", "created_at": "2026-08-19T17:04:12Z",
             "reply_to": {"message_id": 1338, "sender_id": 7, "excerpt": "Dinner at 7?"},
             "thread_root_id": 1338}
            """.trimIndent(),
        )
        assertThat(reply.threadRootId).isEqualTo(1338L)
        assertThat(reply.replyCount).isNull()

        val root = json.decodeFromString<MessageDto>(
            """
            {"id": 1338, "chat_id": 42, "sender_id": 7,
             "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
             "body": "Dinner at 7?", "created_at": "2026-08-19T17:03:12Z",
             "reply_count": 3}
            """.trimIndent(),
        )
        assertThat(root.replyCount).isEqualTo(3L)
        assertThat(root.threadRootId).isNull()
    }
}
