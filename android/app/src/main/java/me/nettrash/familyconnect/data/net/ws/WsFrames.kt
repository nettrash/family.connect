/*
 * WsFrames.kt
 * Family Connect (Android)
 *
 * WebSocket frame vocabulary, transcribed 1:1 from docs/protocol.md
 * ("WebSocket protocol"). Two sealed hierarchies discriminated by the
 * "type" field:
 *
 *   ClientFrame — send / read / typing / ping
 *   ServerFrame — ack / message / read / typing / member_joined /
 *                 member_left / pong / error
 *
 * Compatibility rule: unknown `type` values must be *dropped*, not crash
 * the socket — that is how call_offer / call_answer signaling arrives for
 * v2 clients without breaking v1. `parseServerFrame` below encodes that
 * rule (and is what WsFrameSerdeTest pins down).
 *
 * iOS counterpart: ios/FamilyConnect/Data/Net/WsFrames.swift
 */

package me.nettrash.familyconnect.data.net.ws

import kotlinx.serialization.ExperimentalSerializationApi
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonClassDiscriminator
import me.nettrash.familyconnect.data.net.dto.MessageDto
import me.nettrash.familyconnect.data.net.dto.UserDto

// -- Client → server ---------------------------------------------------------

@OptIn(ExperimentalSerializationApi::class)
@Serializable
@JsonClassDiscriminator("type")
sealed interface ClientFrame {

    @Serializable
    @SerialName("send")
    data class Send(
        @SerialName("chat_id") val chatId: Long,
        @SerialName("client_msg_id") val clientMsgId: String,
        val body: String,
    ) : ClientFrame

    @Serializable
    @SerialName("read")
    data class Read(
        @SerialName("chat_id") val chatId: Long,
        @SerialName("last_read_message_id") val lastReadMessageId: Long,
    ) : ClientFrame

    @Serializable
    @SerialName("typing")
    data class Typing(
        @SerialName("chat_id") val chatId: Long,
    ) : ClientFrame

    @Serializable
    @SerialName("ping")
    data object Ping : ClientFrame
}

// -- Server → client -----------------------------------------------------------

@OptIn(ExperimentalSerializationApi::class)
@Serializable
@JsonClassDiscriminator("type")
sealed interface ServerFrame {

    @Serializable
    @SerialName("ack")
    data class Ack(
        @SerialName("client_msg_id") val clientMsgId: String,
        val message: MessageDto,
    ) : ServerFrame

    @Serializable
    @SerialName("message")
    data class Message(
        val message: MessageDto,
    ) : ServerFrame

    @Serializable
    @SerialName("read")
    data class Read(
        @SerialName("chat_id") val chatId: Long,
        @SerialName("user_id") val userId: Long,
        @SerialName("last_read_message_id") val lastReadMessageId: Long,
    ) : ServerFrame

    @Serializable
    @SerialName("typing")
    data class Typing(
        @SerialName("chat_id") val chatId: Long,
        @SerialName("user_id") val userId: Long,
    ) : ServerFrame

    @Serializable
    @SerialName("member_joined")
    data class MemberJoined(
        @SerialName("family_id") val familyId: Long,
        val user: UserDto,
    ) : ServerFrame

    @Serializable
    @SerialName("member_left")
    data class MemberLeft(
        @SerialName("family_id") val familyId: Long,
        @SerialName("user_id") val userId: Long,
    ) : ServerFrame

    @Serializable
    @SerialName("pong")
    data object Pong : ServerFrame

    @Serializable
    @SerialName("error")
    data class Error(
        val code: String,
        val message: String,
        // Present when the error answers a `send`.
        @SerialName("client_msg_id") val clientMsgId: String? = null,
    ) : ServerFrame
}

/**
 * Parse one inbound text frame. Returns null for anything that doesn't
 * decode as a known [ServerFrame] — most importantly frames with an
 * unknown "type" (future protocol additions), which the compatibility
 * rules require us to silently drop rather than kill the socket over.
 */
fun parseServerFrame(json: Json, text: String): ServerFrame? =
    runCatching { json.decodeFromString<ServerFrame>(text) }.getOrNull()
