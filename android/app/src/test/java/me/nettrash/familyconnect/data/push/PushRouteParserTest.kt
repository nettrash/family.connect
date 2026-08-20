/*
 * PushRouteParserTest.kt
 * Family Connect (Android)
 *
 * Golden tests against the exact data payloads docs/protocol.md ("Push
 * notifications") specifies for FCM: kind message / join_request /
 * joined, all values strings. Pure JVM — the parser has no Android or
 * Firebase types by design.
 */

package me.nettrash.familyconnect.data.push

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class PushRouteParserTest {

    // -- Protocol golden payloads ------------------------------------------------

    @Test
    fun messagePayloadOpensTheChat() {
        // Verbatim from the protocol's FCM example.
        val data = mapOf(
            "kind" to "message",
            "chat_id" to "42",
            "message_id" to "1338",
        )

        assertThat(PushRouteParser.parse(data)).isEqualTo(PendingRoute.Chat(42L))
    }

    @Test
    fun joinRequestPayloadOpensTheJoinRequestsScreen() {
        // Join events carry family_id instead of chat/message ids.
        val data = mapOf(
            "kind" to "join_request",
            "family_id" to "3",
        )

        assertThat(PushRouteParser.parse(data)).isEqualTo(PendingRoute.JoinRequests)
    }

    @Test
    fun joinedPayloadOpensTheChatList() {
        val data = mapOf(
            "kind" to "joined",
            "family_id" to "3",
        )

        assertThat(PushRouteParser.parse(data)).isEqualTo(PendingRoute.ChatList)
    }

    // -- Compatibility / malformed input -------------------------------------------

    @Test
    fun unknownKindIsIgnored() {
        // Protocol compatibility rule: unknown kinds must not break v1
        // clients — call_incoming or whatever comes later just opens the app.
        assertThat(PushRouteParser.parse(mapOf("kind" to "call_incoming"))).isNull()
    }

    @Test
    fun missingKindIsIgnored() {
        assertThat(PushRouteParser.parse(emptyMap())).isNull()
        assertThat(PushRouteParser.parse(mapOf("chat_id" to "42"))).isNull()
    }

    @Test
    fun messageWithoutAParsableChatIdIsIgnored() {
        assertThat(PushRouteParser.parse(mapOf("kind" to "message"))).isNull()
        assertThat(
            PushRouteParser.parse(mapOf("kind" to "message", "chat_id" to "not-a-number")),
        ).isNull()
    }

    @Test
    fun extraUnknownFieldsAreIgnored() {
        val data = mapOf(
            "kind" to "message",
            "chat_id" to "42",
            "message_id" to "1338",
            "some_future_field" to "whatever",
        )

        assertThat(PushRouteParser.parse(data)).isEqualTo(PendingRoute.Chat(42L))
    }
}
