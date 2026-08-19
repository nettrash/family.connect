/*
 * WsFrameSerdeTest.kt
 * Family Connect (Android)
 *
 * Round-trips every frame against the EXACT JSON in docs/protocol.md's
 * "WebSocket protocol" section — the doc is authoritative, so these
 * literals are transcribed, not derived. Comparison is by JsonElement:
 * exact content, order-insensitive.
 */

package me.nettrash.familyconnect.data.net.ws

import com.google.common.truth.Truth.assertThat
import kotlinx.serialization.json.Json
import org.junit.Test

class WsFrameSerdeTest {

    // Same configuration as AppModule.provideJson.
    private val json = Json {
        ignoreUnknownKeys = true
        classDiscriminator = "type"
        encodeDefaults = false
    }

    private fun assertEncodesTo(frame: ClientFrame, expected: String) {
        val encoded = json.encodeToString<ClientFrame>(frame)
        assertThat(json.parseToJsonElement(encoded))
            .isEqualTo(json.parseToJsonElement(expected))
    }

    private fun assertRoundTrips(literal: String, expected: ServerFrame) {
        val decoded = parseServerFrame(json, literal)
        assertThat(decoded).isEqualTo(expected)
        // And back: re-encoding produces the same JSON content.
        val encoded = json.encodeToString<ServerFrame>(decoded!!)
        assertThat(json.parseToJsonElement(encoded))
            .isEqualTo(json.parseToJsonElement(literal))
    }

    // -- Client → server (protocol.md lines, verbatim values) ---------------

    @Test
    fun sendFrameEncodesExactlyAsProtocol() {
        assertEncodesTo(
            ClientFrame.Send(
                chatId = 42,
                clientMsgId = "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
                body = "Dinner at 7?",
            ),
            """{"type": "send", "chat_id": 42, "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01", "body": "Dinner at 7?"}""",
        )
    }

    @Test
    fun readFrameEncodesExactlyAsProtocol() {
        assertEncodesTo(
            ClientFrame.Read(chatId = 42, lastReadMessageId = 1337),
            """{"type": "read", "chat_id": 42, "last_read_message_id": 1337}""",
        )
    }

    @Test
    fun typingFrameEncodesExactlyAsProtocol() {
        assertEncodesTo(
            ClientFrame.Typing(chatId = 42),
            """{"type": "typing", "chat_id": 42}""",
        )
    }

    @Test
    fun pingFrameEncodesExactlyAsProtocol() {
        assertEncodesTo(ClientFrame.Ping, """{"type": "ping"}""")
    }

    // -- Server → client -------------------------------------------------------

    private val messageJson =
        """{"id": 1338, "chat_id": 42, "sender_id": 7,
           "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
           "body": "Dinner at 7?", "created_at": "2026-08-19T17:03:12Z"}"""

    private val messageDto = me.nettrash.familyconnect.data.net.dto.MessageDto(
        id = 1338,
        chatId = 42,
        senderId = 7,
        clientMsgId = "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
        body = "Dinner at 7?",
        createdAt = "2026-08-19T17:03:12Z",
    )

    @Test
    fun ackFrameRoundTrips() {
        assertRoundTrips(
            """{"type": "ack", "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01", "message": $messageJson}""",
            ServerFrame.Ack(
                clientMsgId = "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
                message = messageDto,
            ),
        )
    }

    @Test
    fun messageFrameRoundTrips() {
        assertRoundTrips(
            """{"type": "message", "message": $messageJson}""",
            ServerFrame.Message(message = messageDto),
        )
    }

    @Test
    fun readFrameRoundTrips() {
        assertRoundTrips(
            """{"type": "read", "chat_id": 42, "user_id": 9, "last_read_message_id": 1338}""",
            ServerFrame.Read(chatId = 42, userId = 9, lastReadMessageId = 1338),
        )
    }

    @Test
    fun typingFrameRoundTrips() {
        assertRoundTrips(
            """{"type": "typing", "chat_id": 42, "user_id": 9}""",
            ServerFrame.Typing(chatId = 42, userId = 9),
        )
    }

    @Test
    fun memberJoinedFrameRoundTrips() {
        assertRoundTrips(
            """{"type": "member_joined", "family_id": 3, "user": {"id": 11, "username": "junior", "display_name": "Junior"}}""",
            ServerFrame.MemberJoined(
                familyId = 3,
                user = me.nettrash.familyconnect.data.net.dto.UserDto(
                    id = 11,
                    username = "junior",
                    displayName = "Junior",
                ),
            ),
        )
    }

    @Test
    fun memberLeftFrameRoundTrips() {
        assertRoundTrips(
            """{"type": "member_left", "family_id": 3, "user_id": 11}""",
            ServerFrame.MemberLeft(familyId = 3, userId = 11),
        )
    }

    @Test
    fun pongFrameRoundTrips() {
        assertRoundTrips("""{"type": "pong"}""", ServerFrame.Pong)
    }

    @Test
    fun errorFrameRoundTrips() {
        assertRoundTrips(
            """{"type": "error", "code": "not_chat_member", "message": "you are not a member of this chat", "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01"}""",
            ServerFrame.Error(
                code = "not_chat_member",
                message = "you are not a member of this chat",
                clientMsgId = "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
            ),
        )
    }

    @Test
    fun errorFrameWithoutClientMsgIdDecodes() {
        val decoded = parseServerFrame(json, """{"type": "error", "code": "internal", "message": "boom"}""")
        assertThat(decoded).isEqualTo(ServerFrame.Error(code = "internal", message = "boom"))
    }

    // -- Compatibility rules --------------------------------------------------------

    @Test
    fun unknownFrameTypeIsDroppedNotThrown() {
        // A future voice/video signaling frame must not kill the socket.
        val decoded = parseServerFrame(
            json,
            """{"type": "call_offer", "chat_id": 42, "sdp": "v=0..."}""",
        )
        assertThat(decoded).isNull()
    }

    @Test
    fun malformedJsonIsDroppedNotThrown() {
        assertThat(parseServerFrame(json, "{not json")).isNull()
    }

    @Test
    fun unknownKeysInKnownFramesAreIgnored() {
        val decoded = parseServerFrame(
            json,
            """{"type": "pong", "server_time": "2026-08-19T17:03:12Z", "shard": 4}""",
        )
        assertThat(decoded).isEqualTo(ServerFrame.Pong)

        val read = parseServerFrame(
            json,
            """{"type": "read", "chat_id": 42, "user_id": 9, "last_read_message_id": 1338, "extra": true}""",
        )
        assertThat(read).isEqualTo(ServerFrame.Read(chatId = 42, userId = 9, lastReadMessageId = 1338))
    }
}
