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
 *                 member_left / member_deleted / family_owner /
 *                 reaction / poll / pong / error
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
import me.nettrash.familyconnect.data.net.dto.MemberDto
import me.nettrash.familyconnect.data.net.dto.MessageDto
import me.nettrash.familyconnect.data.net.dto.NewPollDto
import me.nettrash.familyconnect.data.net.dto.NoteDto
import me.nettrash.familyconnect.data.net.dto.PollDto
import me.nettrash.familyconnect.data.net.dto.ReactionDto
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
        /**
         * Optional: the message being answered (protocol.md, "Replies").
         * Omitted rather than serialized as null — the house Json sets
         * encodeDefaults=false, which is what keeps an ordinary send frame
         * byte-identical to what it was before replies existed.
         */
        @SerialName("reply_to_message_id") val replyToMessageId: Long? = null,
        /**
         * Optional: an uploaded photo or video this message claims
         * (protocol.md, "Photos, videos and files"). Omitted the same way, for
         * the same reason.
         */
        @SerialName("attachment_id") val attachmentId: Long? = null,
        /**
         * Optional: the options that make this message a poll
         * (protocol.md, "Polls"). The body is then the QUESTION. Omitted
         * the same way, for the same reason — and mutually exclusive
         * with [attachmentId], which the server enforces.
         */
        val poll: NewPollDto? = null,
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

    /**
     * A member DELETED their account (docs/protocol.md, "Deleting an
     * account"). It carries the whole tombstone `Member` — `deleted:
     * true`, the placeholder display name, `avatar_version: 0`, no
     * birthday — because that is exactly what has to be overwritten.
     *
     * The one frame in this protocol whose job is to WIPE stored fields,
     * so it is applied by writing the tombstone deliberately
     * ([me.nettrash.familyconnect.data.db.MemberDao.writeTombstone])
     * rather than through the ordinary member upsert, which everywhere
     * else must never let an absent field clear a stored one. It never
     * notifies and never counts as unread: it is a correction to what
     * this client already holds, not news.
     */
    @Serializable
    @SerialName("member_deleted")
    data class MemberDeleted(
        /**
         * ABSENT when the account belonged to no family, so nullable with
         * a default — a required field here dropped the whole frame as
         * unparseable for exactly the peer who needs it most: somebody
         * this account only ever shared a direct chat with, whose chat is
         * about to vanish and for whom nothing else says why. A client
         * keys this frame on the `member` and never on the family: a peer
         * outside the family receives a tombstone tagged with a family
         * they are not in, and in the sole-owner case with one that no
         * longer exists at all.
         */
        @SerialName("family_id") val familyId: Long? = null,
        val member: MemberDto,
    ) : ServerFrame

    /**
     * The family has a new owner — sent when an owner deletes their
     * account and ownership passes to the longest-standing remaining
     * member. It reaches every member of the family; the one it NAMES
     * gains the owner-only screens immediately rather than at its next
     * `GET /me`.
     */
    @Serializable
    @SerialName("family_owner")
    data class FamilyOwner(
        @SerialName("family_id") val familyId: Long,
        @SerialName("user_id") val userId: Long,
    ) : ServerFrame

    /**
     * One board note in whatever state it now has — created, edited, moved,
     * or a tombstone. Never notifies and never counts as unread.
     */
    @Serializable
    @SerialName("board_note")
    data class BoardNote(val note: NoteDto) : ServerFrame

    /**
     * One fragment of the assistant's reply, as it is generated.
     *
     * COSMETIC: the row named by [messageId] is the truth, and its final
     * body arrives as [MessageEdited] whether or not any of these were seen
     * (protocol.md, "The assistant"). Missing them costs a live-typing
     * effect, never the answer.
     */
    @Serializable
    @SerialName("ai_delta")
    data class AiDelta(
        @SerialName("chat_id") val chatId: Long,
        @SerialName("message_id") val messageId: Long,
        val text: String,
    ) : ServerFrame

    /** The reply stopped early; whatever arrived is already on the row. */
    @Serializable
    @SerialName("ai_error")
    data class AiError(
        @SerialName("chat_id") val chatId: Long,
        @SerialName("message_id") val messageId: Long,
    ) : ServerFrame

    /**
     * An edit of an existing message. A SEPARATE frame from [Message]
     * because that one bumps unread counts and raises notifications, and
     * an edit must do neither (protocol.md, "Editing").
     */
    @Serializable
    @SerialName("message_edited")
    data class MessageEdited(val message: MessageDto) : ServerFrame

    /**
     * A message's FULL current reaction state (protocol: never a delta) —
     * idempotent; applied only when reactionSeq beats the stored one.
     */
    @Serializable
    @SerialName("reaction")
    data class Reaction(
        @SerialName("chat_id") val chatId: Long,
        @SerialName("message_id") val messageId: Long,
        @SerialName("reaction_seq") val reactionSeq: Long,
        val reactions: List<ReactionDto>,
    ) : ServerFrame

    /**
     * A poll's FULL current state (protocol: never a delta), to every
     * member of the chat — the voter's own connections included, since
     * their own request is answered by its HTTP response.
     *
     * Applied only when [PollDto.pollSeq] beats the one stored for that
     * message, so an out-of-order frame cannot undo a newer vote. It
     * never notifies and never touches an unread count: a vote is not a
     * message.
     */
    @Serializable
    @SerialName("poll")
    data class Poll(
        @SerialName("chat_id") val chatId: Long,
        @SerialName("message_id") val messageId: Long,
        val poll: PollDto,
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
