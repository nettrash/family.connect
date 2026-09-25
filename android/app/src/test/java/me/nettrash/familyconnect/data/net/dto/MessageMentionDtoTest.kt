/*
 * MessageMentionDtoTest.kt
 * Family Connect (Android) — tests
 *
 * The mention's wire shapes (docs/protocol.md, "Mentioning a member").
 */

package me.nettrash.familyconnect.data.net.dto

import com.google.common.truth.Truth.assertThat
import kotlinx.serialization.json.Json
import org.junit.Test

class MessageMentionDtoTest {

    private val json = Json { ignoreUnknownKeys = true; encodeDefaults = false }

    @Test
    fun `a message's mentions decode, and are null when absent`() {
        val named = json.decodeFromString<MessageDto>(
            """
            {"id": 1338, "chat_id": 42, "sender_id": 7,
             "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
             "body": "@Anna are you in?", "created_at": "2026-08-19T17:03:12Z",
             "mentions": [{"user_id": 9, "name": "Anna"}]}
            """.trimIndent(),
        )
        assertThat(named.mentions).containsExactly(MentionDto(9, "Anna"))
        val plain = json.decodeFromString<MessageDto>(
            """
            {"id": 1338, "chat_id": 42, "sender_id": 7,
             "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
             "body": "Dinner at 7?", "created_at": "2026-08-19T17:03:12Z"}
            """.trimIndent(),
        )
        assertThat(plain.mentions).isNull()
    }

    @Test
    fun `a chat-list entry's mentioned reads true, and null when absent`() {
        val marked = json.decodeFromString<ChatListItemDto>(
            """{"chat": {"id": 42, "kind": "family", "title": "The Smiths"}, "unread_count": 2, "mentioned": true}""",
        )
        assertThat(marked.mentioned).isTrue()
        val plain = json.decodeFromString<ChatListItemDto>(
            """{"chat": {"id": 42, "kind": "family", "title": "The Smiths"}, "unread_count": 2}""",
        )
        assertThat(plain.mentioned).isNull()
    }

    @Test
    fun `the send request carries the list and omits it when null`() {
        val named = json.encodeToString(
            SendMessageRequest.serializer(),
            SendMessageRequest("u1", "@Anna?", mentions = listOf(MentionDto(9, "Anna"))),
        )
        assertThat(named).isEqualTo("""{"client_msg_id":"u1","body":"@Anna?","mentions":[{"user_id":9,"name":"Anna"}]}""")
        val plain = json.encodeToString(SendMessageRequest.serializer(), SendMessageRequest("u1", "hi"))
        assertThat(plain).doesNotContain("mentions")
    }

    @Test
    fun `the codec round-trips the wire shape`() {
        val list = listOf(MentionDto(9, "Anna"), MentionDto(11, "Uncle Bob"))
        assertThat(MentionsCodec.decode(MentionsCodec.encode(list))).isEqualTo(list)
        assertThat(MentionsCodec.decode(null)).isEmpty()
        assertThat(MentionsCodec.decode("not json")).isEmpty()
    }
}
