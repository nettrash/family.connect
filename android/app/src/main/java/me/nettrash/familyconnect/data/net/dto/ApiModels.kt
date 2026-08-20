/*
 * ApiModels.kt
 * Family Connect (Android)
 *
 * REST wire shapes, transcribed 1:1 from docs/protocol.md ("Objects" +
 * per-endpoint tables). Every field is @SerialName'd to the protocol's
 * snake_case so Kotlin property names stay idiomatic without a global
 * naming strategy. Optional fields default to null — combined with
 * `ignoreUnknownKeys` this is what makes v1 clients forward-compatible
 * with fields the server adds later.
 *
 * iOS counterpart: ios/FamilyConnect/Data/Net/ApiModels.swift
 */

package me.nettrash.familyconnect.data.net.dto

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

// -- Objects ---------------------------------------------------------------

@Serializable
data class UserDto(
    val id: Long,
    val username: String,
    @SerialName("display_name") val displayName: String,
    // Absent on the embedded user of `member_joined` frames.
    @SerialName("created_at") val createdAt: String? = null,
)

@Serializable
data class MemberDto(
    val id: Long,
    val username: String,
    @SerialName("display_name") val displayName: String,
    val role: String,
)

@Serializable
data class FamilyDto(
    val id: Long,
    val name: String,
    @SerialName("join_policy") val joinPolicy: String,
    @SerialName("created_at") val createdAt: String? = null,
    // Present when (and only when) the caller is the owner.
    @SerialName("invite_code") val inviteCode: String? = null,
)

@Serializable
data class PendingJoinRequestDto(
    @SerialName("family_id") val familyId: Long,
    @SerialName("family_name") val familyName: String,
    @SerialName("created_at") val createdAt: String,
)

@Serializable
data class JoinRequestDto(
    val id: Long,
    val user: UserDto,
    @SerialName("created_at") val createdAt: String,
)

@Serializable
data class ChatDto(
    val id: Long,
    val kind: String,
    val title: String,
    @SerialName("peer_user_id") val peerUserId: Long? = null,
)

@Serializable
data class MessageDto(
    val id: Long,
    @SerialName("chat_id") val chatId: Long,
    @SerialName("sender_id") val senderId: Long,
    @SerialName("client_msg_id") val clientMsgId: String,
    val body: String,
    @SerialName("created_at") val createdAt: String,
)

// -- Request bodies ----------------------------------------------------------

@Serializable
data class RegisterRequest(
    val username: String,
    @SerialName("display_name") val displayName: String,
    val password: String,
)

@Serializable
data class LoginRequest(
    val username: String,
    val password: String,
)

@Serializable
data class CreateFamilyRequest(val name: String)

@Serializable
data class JoinFamilyRequest(@SerialName("invite_code") val inviteCode: String)

@Serializable
data class PatchFamilyRequest(@SerialName("join_policy") val joinPolicy: String)

@Serializable
data class CreateDirectChatRequest(@SerialName("user_id") val userId: Long)

@Serializable
data class SendMessageRequest(
    @SerialName("client_msg_id") val clientMsgId: String,
    val body: String,
)

@Serializable
data class ReadRequest(@SerialName("last_read_message_id") val lastReadMessageId: Long)

@Serializable
data class DeviceRequest(
    val platform: String,
    // No default so `push_token: null` is serialized explicitly, exactly
    // as the protocol table writes it. Null when the build has no FCM
    // config (no google-services.json) — the device row still registers,
    // it just can't be pushed to.
    @SerialName("push_token") val pushToken: String?,
)

// -- Response envelopes -------------------------------------------------------

@Serializable
data class AuthResponse(
    val token: String,
    val user: UserDto,
)

@Serializable
data class MeResponse(
    val user: UserDto,
    val family: FamilyDto? = null,
    val role: String? = null,
    @SerialName("pending_join_request") val pendingJoinRequest: PendingJoinRequestDto? = null,
)

@Serializable
data class JoinResponse(val status: String)

@Serializable
data class FamilyResponse(val family: FamilyDto)

@Serializable
data class FamilyMineResponse(
    val family: FamilyDto,
    val members: List<MemberDto>,
)

@Serializable
data class RotateInviteCodeResponse(@SerialName("invite_code") val inviteCode: String)

@Serializable
data class JoinRequestsResponse(val requests: List<JoinRequestDto>)

@Serializable
data class ApproveResponse(val member: MemberDto)

@Serializable
data class ChatListItemDto(
    val chat: ChatDto,
    @SerialName("last_message") val lastMessage: MessageDto? = null,
    @SerialName("unread_count") val unreadCount: Int,
)

@Serializable
data class ChatsResponse(val chats: List<ChatListItemDto>)

@Serializable
data class ChatResponse(val chat: ChatDto)

@Serializable
data class MessagesResponse(val messages: List<MessageDto>)

@Serializable
data class MessageResponse(val message: MessageDto)

@Serializable
data class DeviceResponse(@SerialName("device_id") val deviceId: Long)

// -- Error shape ---------------------------------------------------------------

@Serializable
data class ErrorBody(val error: ErrorPayload) {
    @Serializable
    data class ErrorPayload(
        val code: String,
        val message: String,
    )
}
