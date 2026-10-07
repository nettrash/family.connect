/*
 * RoundWireTest.kt
 * Family Connect (Android)
 *
 * A VIDEO MESSAGE on the wire (#79, docs/protocol.md, "Video messages"; the
 * plan's "On the wire"): one flag, the sticker's pattern. `round: true` on
 * the Attachment of every read, absent — never `false` — otherwise; the same
 * key on the send, over REST and over the socket, and omitted from every
 * ordinary send so those stay byte for byte what they were.
 *
 * The Apple counterpart is RoundWireTests, made from StickerWireTests.
 */

package me.nettrash.familyconnect.data.net.dto

import com.google.common.truth.Truth.assertThat
import com.google.common.truth.Truth.assertWithMessage
import kotlinx.serialization.json.Json
import me.nettrash.familyconnect.data.net.ws.ClientFrame
import org.junit.Test

class RoundWireTest {

    // Same configuration as AppModule.provideJson.
    private val json = Json {
        ignoreUnknownKeys = true
        classDiscriminator = "type"
        encodeDefaults = false
    }

    private val circle = """{"id": 91, "kind": "video", "mime": "video/mp4", "size": 1649700,
        "width": 480, "height": 480, "duration_ms": 23400, "has_preview": true, "round": true}"""

    // -- The Attachment ---------------------------------------------------------------

    @Test
    fun `the protocol's attachment decodes as a video message`() {
        val attachment = json.decodeFromString<AttachmentDto>(circle)

        assertThat(attachment.round).isTrue()
        assertThat(attachment.isRound).isTrue()
        // Still a video in every other respect.
        assertThat(attachment.isVideo).isTrue()
        assertThat(attachment.durationMs).isEqualTo(23_400)
        assertThat(attachment.hasPreview).isTrue()
    }

    @Test
    fun `an attachment without the key is an ordinary video`() {
        val video = json.decodeFromString<AttachmentDto>(
            """{"id": 92, "kind": "video", "mime": "video/mp4", "size": 9, "width": 480, "height": 480}""",
        )

        assertThat(video.round).isNull()
        assertThat(video.isRound).isFalse()
    }

    @Test
    fun `the flag on anything but a video draws as what it is`() {
        for (kind in listOf("photo", "audio", "file", "location")) {
            val stray = json.decodeFromString<AttachmentDto>(
                """{"id": 93, "kind": "$kind", "mime": "x/y", "size": 9, "round": true}""",
            )
            assertWithMessage(kind).that(stray.isRound).isFalse()
        }
        assertThat(AttachmentDto(id = 1, kind = "video", mime = "video/mp4", size = 1, round = false).isRound)
            .isFalse()
    }

    @Test
    fun `a stored row keeps exactly what the wire said - the flag, or no key at all`() {
        val flagged = json.decodeFromString<AttachmentDto>(circle)
        val plain = flagged.copy(round = null)

        // Inside the attachments JSON a row already stores: no migration.
        val stored = AttachmentsCodec.encode(listOf(flagged, plain))
        assertThat(AttachmentsCodec.decode(stored)).containsExactly(flagged, plain).inOrder()
        assertThat(AttachmentsCodec.encode(listOf(plain))).doesNotContain("round")
    }

    @Test
    fun `a message frame and the legacy attachment carry the flag`() {
        val message = json.decodeFromString<MessageDto>(
            """{"id": 900, "chat_id": 42, "sender_id": 9, "client_msg_id": "c", "body": "",
                "created_at": "2026-10-05T10:00:00Z", "attachments": [$circle]}""",
        )
        assertThat(message.resolvedAttachments.single().isRound).isTrue()

        val legacy = json.decodeFromString<MessageDto>(
            """{"id": 901, "chat_id": 42, "sender_id": 9, "client_msg_id": "d", "body": "",
                "created_at": "2026-10-05T10:00:00Z", "attachment": $circle}""",
        )
        assertThat(legacy.resolvedAttachments.single().isRound).isTrue()
    }

    // -- Sending -----------------------------------------------------------------------

    @Test
    fun `a video message send is the protocol's request`() {
        val encoded = json.encodeToString(
            SendMessageRequest.serializer(),
            SendMessageRequest("u1", "", replyToMessageId = 41, attachmentIds = listOf(91), round = true),
        )

        assertThat(json.parseToJsonElement(encoded)).isEqualTo(
            json.parseToJsonElement(
                """{"client_msg_id": "u1", "body": "", "reply_to_message_id": 41,
                    "attachment_ids": [91], "round": true}""",
            ),
        )
    }

    @Test
    fun `the send frame carries the flag`() {
        val encoded = json.encodeToString<ClientFrame>(
            ClientFrame.Send(chatId = 42, clientMsgId = "4f9e21c0", body = "", attachmentIds = listOf(91), round = true),
        )

        assertThat(json.parseToJsonElement(encoded)).isEqualTo(
            json.parseToJsonElement(
                """{"type": "send", "chat_id": 42, "client_msg_id": "4f9e21c0", "body": "",
                    "attachment_ids": [91], "round": true}""",
            ),
        )
    }

    @Test
    fun `an ordinary send carries no round key at all`() {
        val rest = json.encodeToString(
            SendMessageRequest.serializer(),
            SendMessageRequest("u1", "hi", attachmentIds = listOf(34)),
        )
        assertThat(rest).isEqualTo("""{"client_msg_id":"u1","body":"hi","attachment_ids":[34]}""")

        val frame = json.encodeToString<ClientFrame>(ClientFrame.Send(chatId = 42, clientMsgId = "u1", body = "hi"))
        assertThat(frame).doesNotContain("round")

        // And a sticker is a sticker: the one flag, not both.
        val sticker = json.encodeToString(
            SendMessageRequest.serializer(),
            SendMessageRequest("u1", "", attachmentIds = listOf(90), sticker = true),
        )
        assertThat(sticker).doesNotContain("round")
    }
}
