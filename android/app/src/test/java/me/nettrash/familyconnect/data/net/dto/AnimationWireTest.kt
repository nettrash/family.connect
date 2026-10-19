/*
 * AnimationWireTest.kt
 * Family Connect (Android)
 *
 * An ANIMATION on the wire (#96, docs/protocol.md, "Animations" → "Sending
 * one"): one flag, the round flag's pattern. `animation: true` on the
 * Attachment of every read, absent — never `false` — otherwise; the same key
 * on the send, over REST and over the socket, and omitted from every other
 * send so those stay byte for byte what they were. The protocol's own
 * example is decoded as written.
 */

package me.nettrash.familyconnect.data.net.dto

import com.google.common.truth.Truth.assertThat
import com.google.common.truth.Truth.assertWithMessage
import kotlinx.serialization.json.Json
import me.nettrash.familyconnect.data.net.ws.ClientFrame
import org.junit.Test

class AnimationWireTest {

    // Same configuration as AppModule.provideJson.
    private val json = Json {
        ignoreUnknownKeys = true
        classDiscriminator = "type"
        encodeDefaults = false
    }

    /** docs/protocol.md, "Animations" → "Sending one". */
    private val animation = """{"id": 93, "kind": "video", "mime": "video/mp4", "size": 182330,
        "width": 480, "height": 270, "duration_ms": 2100, "has_preview": true, "animation": true}"""

    @Test
    fun `the protocol's attachment decodes as an animation`() {
        val attachment = json.decodeFromString<AttachmentDto>(animation)

        assertThat(attachment.animation).isTrue()
        assertThat(attachment.isAnimation).isTrue()
        assertThat(attachment.isVideo).isTrue()
        assertThat(attachment.isRound).isFalse()
        assertThat(attachment.durationMs).isEqualTo(2_100)
    }

    @Test
    fun `an attachment without the key is an ordinary video, and the flag on anything else is ignored`() {
        val video = json.decodeFromString<AttachmentDto>(
            """{"id": 92, "kind": "video", "mime": "video/mp4", "size": 9, "width": 480, "height": 270}""",
        )
        assertThat(video.animation).isNull()
        assertThat(video.isAnimation).isFalse()
        for (kind in listOf("photo", "audio", "file", "location")) {
            val stray = json.decodeFromString<AttachmentDto>(
                """{"id": 94, "kind": "$kind", "mime": "x/y", "size": 9, "animation": true}""",
            )
            assertWithMessage(kind).that(stray.isAnimation).isFalse()
        }
    }

    @Test
    fun `a stored row keeps exactly what the wire said`() {
        val flagged = json.decodeFromString<AttachmentDto>(animation)
        val plain = flagged.copy(animation = null)
        val stored = AttachmentsCodec.encode(listOf(flagged, plain))
        assertThat(AttachmentsCodec.decode(stored)).containsExactly(flagged, plain).inOrder()
        assertThat(AttachmentsCodec.encode(listOf(plain))).doesNotContain("animation")
    }

    @Test
    fun `a message carries the flag on its attachment`() {
        val message = json.decodeFromString<MessageDto>(
            """{"id": 900, "chat_id": 42, "sender_id": 9, "client_msg_id": "c", "body": "lol",
                "created_at": "2026-10-19T10:00:00Z", "attachments": [$animation]}""",
        )
        assertThat(message.resolvedAttachments.single().isAnimation).isTrue()
        assertThat(message.body).isEqualTo("lol")
    }

    @Test
    fun `an animation send is the protocol's request and frame`() {
        val rest = json.encodeToString(
            SendMessageRequest.serializer(),
            SendMessageRequest("u1", "", attachmentIds = listOf(93), animation = true),
        )
        assertThat(json.parseToJsonElement(rest)).isEqualTo(
            json.parseToJsonElement("""{"client_msg_id": "u1", "body": "", "attachment_ids": [93], "animation": true}"""),
        )

        val frame = json.encodeToString<ClientFrame>(
            ClientFrame.Send(chatId = 42, clientMsgId = "u1", body = "", attachmentIds = listOf(93), animation = true),
        )
        assertThat(json.parseToJsonElement(frame)).isEqualTo(
            json.parseToJsonElement(
                """{"type": "send", "chat_id": 42, "client_msg_id": "u1", "body": "",
                    "attachment_ids": [93], "animation": true}""",
            ),
        )
    }

    @Test
    fun `every other send carries no animation key at all`() {
        val rest = json.encodeToString(SendMessageRequest.serializer(), SendMessageRequest("u1", "hi", attachmentIds = listOf(34)))
        assertThat(rest).isEqualTo("""{"client_msg_id":"u1","body":"hi","attachment_ids":[34]}""")
        val frame = json.encodeToString<ClientFrame>(ClientFrame.Send(chatId = 42, clientMsgId = "u1", body = "hi"))
        assertThat(frame).doesNotContain("animation")
        val round = json.encodeToString(
            SendMessageRequest.serializer(),
            SendMessageRequest("u1", "", attachmentIds = listOf(91), round = true),
        )
        assertThat(round).doesNotContain("animation")
    }
}
