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
import kotlinx.serialization.json.Json

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
data class ReactionDto(
    @SerialName("user_id") val userId: Long,
    val emoji: String,
)

@Serializable
data class MessageDto(
    val id: Long,
    @SerialName("chat_id") val chatId: Long,
    @SerialName("sender_id") val senderId: Long,
    @SerialName("client_msg_id") val clientMsgId: String,
    val body: String,
    @SerialName("created_at") val createdAt: String,
    // Both absent when (and only when) the message was never reacted
    // to; after clearing, reactions is [] with the seq still present.
    val reactions: List<ReactionDto>? = null,
    @SerialName("reaction_seq") val reactionSeq: Long? = null,
)

/**
 * Local persistence codec: the messages table stores a message's
 * reactions verbatim as the wire-shape JSON array (`reactionsJson`
 * column — null = never reacted, "[]" = cleared). Private Json so a
 * house-config change can never silently re-shape stored rows.
 */
object ReactionsCodec {
    private val json = Json { ignoreUnknownKeys = true }

    fun encode(reactions: List<ReactionDto>): String = json.encodeToString(reactions)

    fun decode(raw: String?): List<ReactionDto> =
        raw?.let { runCatching { json.decodeFromString<List<ReactionDto>>(it) }.getOrNull() }
            .orEmpty()
}

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
data class ReactionRequest(val emoji: String)

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
    // Absent while no message in the chat has ever been reacted to.
    @SerialName("max_reaction_seq") val maxReactionSeq: Long? = null,
)

@Serializable
data class ChatsResponse(val chats: List<ChatListItemDto>)

@Serializable
data class ChatResponse(val chat: ChatDto)

@Serializable
data class MessagesResponse(val messages: List<MessageDto>)

@Serializable
data class MessageResponse(val message: MessageDto)

/**
 * One message's full reaction state — the PUT/DELETE reaction response
 * AND each entry of the GET /chats/{id}/reactions catch-up (protocol:
 * the same shape on purpose; frames carry it too, never a delta).
 */
@Serializable
data class MessageReactionStateDto(
    @SerialName("message_id") val messageId: Long,
    @SerialName("reaction_seq") val reactionSeq: Long,
    val reactions: List<ReactionDto>,
)

@Serializable
data class ReactionsCatchUpResponse(
    @SerialName("message_reactions") val messageReactions: List<MessageReactionStateDto>,
)

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
