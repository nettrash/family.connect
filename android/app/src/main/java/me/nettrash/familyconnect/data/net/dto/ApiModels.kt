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
    /**
     * Bumps on every profile-picture change; 0 = no picture. The default
     * is what lets a client talk to a server older than the avatars
     * release — and it doubles as the cache key, so a changed picture can
     * never be served from a stale entry.
     */
    @SerialName("avatar_version") val avatarVersion: Long = 0,
)

@Serializable
data class MemberDto(
    val id: Long,
    val username: String,
    @SerialName("display_name") val displayName: String,
    val role: String,
    /** See UserDto.avatarVersion. */
    @SerialName("avatar_version") val avatarVersion: Long = 0,
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

/**
 * The quoted message on a reply — as much of it as a bubble needs to draw
 * the quote without holding the original. The server recomputes this on
 * every read, so it follows the quoted message rather than freezing at send
 * time (docs/protocol.md, "Replies").
 */
@Serializable
data class ReplyToDto(
    @SerialName("message_id") val messageId: Long,
    @SerialName("sender_id") val senderId: Long,
    val excerpt: String,
    /**
     * What the quoted message was itself answering — one level, no more.
     * Null is normal: the quoted message was not a reply, or its own parent
     * has been swept by retention (protocol.md, "Replies").
     */
    val parent: QuotedParentDto? = null,
) {
    companion object {
        /**
         * Longest excerpt, in Unicode CODE POINTS — matching
         * `ReplyTo::MAX_EXCERPT_CHARS` on the server and
         * `ReplyToSnapshot.maxExcerptScalars` on iOS.
         */
        const val MAX_EXCERPT_CHARS = 120

        /**
         * Cut like the server does. `String.take` counts UTF-16 CODE
         * UNITS, so it both keeps the wrong amount of text for
         * astral characters and can slice a surrogate pair in half —
         * which renders as a replacement glyph, in a quote the user did
         * not write. Code points, therefore, like `chars().take(120)`
         * server-side.
         */
        fun excerpt(body: String): String {
            if (body.codePointCount(0, body.length) <= MAX_EXCERPT_CHARS) return body
            val end = body.offsetByCodePoints(0, MAX_EXCERPT_CHARS)
            return body.substring(0, end)
        }
    }
}

/**
 * The second and LAST level of a quote (protocol.md, "Replies").
 *
 * Deliberately not the same type as [ReplyToDto]: with no `parent` of its
 * own there is nothing for a third level to decode into, so the cap is
 * structural rather than a rule to remember.
 */
@Serializable
data class QuotedParentDto(
    @SerialName("message_id") val messageId: Long,
    @SerialName("sender_id") val senderId: Long,
    val excerpt: String,
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
    // Present when (and only when) this message is a reply.
    @SerialName("reply_to") val replyTo: ReplyToDto? = null,
    // Both present when (and only when) the body has been edited. The seq
    // is the apply guard — see MessageRepository.applyBody.
    @SerialName("edited_at") val editedAt: String? = null,
    @SerialName("edit_seq") val editSeq: Long? = null,
    // Present when (and only when) the message carries a photo or video.
    val attachment: AttachmentDto? = null,
)

/**
 * A photo or video on a message (docs/protocol.md, "Photos, videos and files").
 *
 * Immutable once sent, with one exception: [hasPreview] flips from false
 * to true when the sender's preview upload lands, which may be after the
 * message itself.
 *
 * Dimensions are optional — the uploader may not have been able to work
 * them out — so a bubble that needs a shape before the bytes arrive uses
 * [aspectRatio], which has an answer either way.
 */
@Serializable
data class AttachmentDto(
    val id: Long,
    /** "photo" | "video". */
    val kind: String,
    val mime: String,
    val size: Long,
    val width: Int? = null,
    val height: Int? = null,
    @SerialName("duration_ms") val durationMs: Int? = null,
    @SerialName("has_preview") val hasPreview: Boolean = false,
    /**
     * Files only, and their whole identity — a photo renders itself,
     * where "attachment 34" tells nobody anything.
     */
    val name: String? = null,
) {
    val isVideo: Boolean get() = kind == KIND_VIDEO
    val isFile: Boolean get() = kind == KIND_FILE
    val isAudio: Boolean get() = kind == KIND_AUDIO

    /**
     * A filename for something that carries no name of its own.
     *
     * The EXTENSION is the part that matters: it is what the gallery, the
     * share target and the receiving app all read to decide what they have
     * been handed. Mirrors iOS's ChatSyncCoordinator.fallbackName.
     */
    val fallbackFileName: String
        get() {
            val ext = when (mime) {
                "image/jpeg" -> "jpg"
                "image/png" -> "png"
                "image/heic" -> "heic"
                "image/heif" -> "heif"
                "video/mp4" -> "mp4"
                "video/quicktime" -> "mov"
                else -> if (isVideo) "mp4" else "jpg"
            }
            return "${if (isVideo) "video" else "photo"}-$id.$ext"
        }

    /** What a bubble calls it: the name for a file, a word for the rest. */
    val displayName: String
        get() = name?.takeIf { it.isNotEmpty() }
            ?: when {
                isVideo -> "Video"
                isFile -> "File"
                else -> "Photo"
            }

    /**
     * Width over height, or 4:3 when the uploader could not say. Reserving
     * the right shape before the image arrives is what stops a photo from
     * shoving the thread when it finishes loading.
     */
    val aspectRatio: Float
        get() {
            val w = width ?: return DEFAULT_ASPECT
            val h = height ?: return DEFAULT_ASPECT
            return if (w > 0 && h > 0) w.toFloat() / h.toFloat() else DEFAULT_ASPECT
        }

    companion object {
        const val KIND_PHOTO = "photo"
        const val KIND_VIDEO = "video"
        const val KIND_AUDIO = "audio"
        const val KIND_FILE = "file"
        const val DEFAULT_ASPECT = 4f / 3f
    }
}

@Serializable
data class AttachmentResponse(val attachment: AttachmentDto)

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
    // encodeDefaults=false in the house Json config, so a null is omitted
    // rather than sent as "reply_to_message_id": null — which is what the
    // protocol writes for an ordinary message.
    @SerialName("reply_to_message_id") val replyToMessageId: Long? = null,
    /** The uploaded attachment this message claims, if any. */
    @SerialName("attachment_id") val attachmentId: Long? = null,
)

@Serializable
data class EditMessageRequest(val body: String)

@Serializable
data class ChangePasswordRequest(
    @SerialName("current_password") val currentPassword: String,
    @SerialName("new_password") val newPassword: String,
)

@Serializable
data class ResetPasswordRequest(
    @SerialName("new_password") val newPassword: String,
)

/**
 * One sticker note on the family board.
 *
 * A TOMBSTONE is the same object with `deleted: true` and no content: the
 * change feed has to be able to say "this note is gone", and an absent row
 * cannot say anything (docs/protocol.md, "Board"). Every content field is
 * therefore nullable.
 */
@Serializable
data class NoteDto(
    val id: Long,
    @SerialName("author_id") val authorId: Long? = null,
    val text: String? = null,
    val color: String? = null,
    val x: Double? = null,
    val y: Double? = null,
    @SerialName("created_at") val createdAt: String? = null,
    @SerialName("updated_at") val updatedAt: String? = null,
    @SerialName("board_seq") val boardSeq: Long,
    val deleted: Boolean? = null,
) {
    val isTombstone: Boolean get() = deleted == true
}

@Serializable
data class BoardResponse(
    val notes: List<NoteDto>,
    @SerialName("max_board_seq") val maxBoardSeq: Long,
)

@Serializable
data class BoardChangesResponse(val notes: List<NoteDto>)

@Serializable
data class NoteResponse(val note: NoteDto)

@Serializable
data class CreateNoteRequest(
    val text: String,
    val color: String,
    val x: Double,
    val y: Double,
)

/**
 * Every field optional: a MOVE sends only x/y (any member may), an edit
 * sends text and/or color (author only). Which fields are present is what
 * decides the permission the server applies — so nulls must be OMITTED,
 * which the house Json does via encodeDefaults=false.
 */
@Serializable
data class PatchNoteRequest(
    val text: String? = null,
    val color: String? = null,
    val x: Double? = null,
    val y: Double? = null,
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

/** PUT /me/avatar — the caller's user with its bumped avatar_version. */
@Serializable
data class AvatarResponse(val user: UserDto)

@Serializable
data class FamilyResponse(val family: FamilyDto)

@Serializable
data class FamilyMineResponse(
    val family: FamilyDto,
    val members: List<MemberDto>,
    // The board cursor, omitted while the board has never been written to.
    @SerialName("max_board_seq") val maxBoardSeq: Long? = null,
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
    // Absent while nothing in the chat has been edited.
    @SerialName("max_edit_seq") val maxEditSeq: Long? = null,
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

// -- Family statistics --------------------------------------------------------

/**
 * What the family has actually sent (protocol.md, "Family statistics").
 * Every member sees the same numbers.
 */
@Serializable
data class FamilyStatsDto(
    val totals: StatsTotalsDto,
    val members: List<MemberStatsDto> = emptyList(),
)

@Serializable
data class StatsTotalsDto(
    val members: Int = 0,
    val messages: Int = 0,
    @SerialName("board_notes") val boardNotes: Int = 0,
    val attachments: AttachmentStatsDto = AttachmentStatsDto(),
    val ai: AiStatsDto = AiStatsDto(),
)

@Serializable
data class MemberStatsDto(
    @SerialName("user_id") val userId: Long,
    @SerialName("display_name") val displayName: String,
    val messages: Int = 0,
    val attachments: AttachmentStatsDto = AttachmentStatsDto(),
    val ai: AiStatsDto = AiStatsDto(),
)

@Serializable
data class AttachmentStatsDto(
    val count: Int = 0,
    val bytes: Long = 0,
    val photo: Int = 0,
    val video: Int = 0,
    val audio: Int = 0,
    val file: Int = 0,
    /**
     * Each distinct file counted once. Family totals ONLY — a file two
     * members both sent belongs to neither alone, so there is no per-member
     * share of it, and the field is absent on a member row.
     */
    @SerialName("stored_bytes") val storedBytes: Long? = null,
)

@Serializable
data class AiStatsDto(
    val questions: Int = 0,
    @SerialName("prompt_tokens") val promptTokens: Int = 0,
    @SerialName("completion_tokens") val completionTokens: Int = 0,
)
