/*
 * ChatApi.kt
 * Family Connect (Android)
 *
 * Suspend wrappers over the Chats & messages endpoint table of
 * docs/protocol.md. Pagination is cursor-based: `before_id` pages
 * history (strictly older, newest-first), `after_id` catches up after a
 * reconnect (strictly newer, oldest-first) — never both.
 *
 * Interface + impl split for testability (scripted fakes drive the send
 * pipeline's REST-fallback tests).
 *
 * iOS counterpart: ios/FamilyConnect/Data/Net/ChatApi.swift
 */

package me.nettrash.familyconnect.data.net

import me.nettrash.familyconnect.data.net.dto.ChatResponse
import me.nettrash.familyconnect.data.net.dto.ChatsResponse
import me.nettrash.familyconnect.data.net.dto.CreateDirectChatRequest
import me.nettrash.familyconnect.data.net.dto.MessageReactionStateDto
import me.nettrash.familyconnect.data.net.dto.EditMessageRequest
import me.nettrash.familyconnect.data.net.dto.MessageResponse
import me.nettrash.familyconnect.data.net.dto.MessagesResponse
import me.nettrash.familyconnect.data.net.dto.ReactionRequest
import me.nettrash.familyconnect.data.net.dto.ReactionsCatchUpResponse
import me.nettrash.familyconnect.data.net.dto.ReadRequest
import me.nettrash.familyconnect.data.net.dto.SendMessageRequest
import javax.inject.Inject
import javax.inject.Singleton

interface ChatApi {
    suspend fun chats(): ApiResult<ChatsResponse>
    suspend fun createDirect(userId: Long): ApiResult<ChatResponse>
    suspend fun messages(
        chatId: Long,
        beforeId: Long? = null,
        afterId: Long? = null,
        limit: Int = 50,
    ): ApiResult<MessagesResponse>

    suspend fun postMessage(
        chatId: Long,
        clientMsgId: String,
        body: String,
        replyToMessageId: Long? = null,
        /** An uploaded photo or video this message claims. */
        attachmentId: Long? = null,
    ): ApiResult<MessageResponse>
    suspend fun postRead(chatId: Long, lastReadMessageId: Long): ApiResult<Unit>

    /** PATCH a message's body. Author only (403 not_message_author otherwise). */
    suspend fun editMessage(chatId: Long, messageId: Long, body: String): ApiResult<MessageResponse>

    /** The edit catch-up: whole messages, ordered by edit_seq ascending. */
    suspend fun getEdits(chatId: Long, afterSeq: Long, limit: Int): ApiResult<MessagesResponse>

    /** Sets/replaces MY reaction — an idempotent state-set, not a toggle. */
    suspend fun putReaction(chatId: Long, messageId: Long, emoji: String): ApiResult<MessageReactionStateDto>

    /** Removes MY reaction; idempotent (returns the current state either way). */
    suspend fun deleteReaction(chatId: Long, messageId: Long): ApiResult<MessageReactionStateDto>

    /** Reaction catch-up page: strictly after [afterSeq], ascending. */
    suspend fun getReactions(chatId: Long, afterSeq: Long, limit: Int = 50): ApiResult<ReactionsCatchUpResponse>
}

@Singleton
class DefaultChatApi @Inject constructor(
    private val client: ApiClient,
) : ChatApi {

    override suspend fun chats(): ApiResult<ChatsResponse> =
        client.get("/chats")

    override suspend fun createDirect(userId: Long): ApiResult<ChatResponse> =
        client.post("/chats/direct", CreateDirectChatRequest(userId))

    override suspend fun messages(
        chatId: Long,
        beforeId: Long?,
        afterId: Long?,
        limit: Int,
    ): ApiResult<MessagesResponse> {
        require(beforeId == null || afterId == null) {
            "before_id and after_id are mutually exclusive (protocol: XOR)"
        }
        val query = buildList {
            beforeId?.let { add("before_id=$it") }
            afterId?.let { add("after_id=$it") }
            add("limit=$limit")
        }.joinToString("&")
        return client.get("/chats/$chatId/messages?$query")
    }

    override suspend fun postMessage(
        chatId: Long,
        clientMsgId: String,
        body: String,
        replyToMessageId: Long?,
        attachmentId: Long?,
    ): ApiResult<MessageResponse> =
        // 201 on first delivery, 200 when the same client_msg_id retries —
        // both are 2xx, both decode to the same message. Never a duplicate.
        client.post(
            "/chats/$chatId/messages",
            SendMessageRequest(clientMsgId, body, replyToMessageId, attachmentId),
        )

    override suspend fun postRead(chatId: Long, lastReadMessageId: Long): ApiResult<Unit> =
        client.post("/chats/$chatId/read", ReadRequest(lastReadMessageId))

    override suspend fun editMessage(
        chatId: Long,
        messageId: Long,
        body: String,
    ): ApiResult<MessageResponse> =
        client.patch("/chats/$chatId/messages/$messageId", EditMessageRequest(body))

    override suspend fun getEdits(
        chatId: Long,
        afterSeq: Long,
        limit: Int,
    ): ApiResult<MessagesResponse> =
        client.get("/chats/$chatId/edits?after_seq=$afterSeq&limit=$limit")

    override suspend fun putReaction(
        chatId: Long,
        messageId: Long,
        emoji: String,
    ): ApiResult<MessageReactionStateDto> =
        client.put("/chats/$chatId/messages/$messageId/reaction", ReactionRequest(emoji))

    override suspend fun deleteReaction(
        chatId: Long,
        messageId: Long,
    ): ApiResult<MessageReactionStateDto> =
        client.delete("/chats/$chatId/messages/$messageId/reaction")

    override suspend fun getReactions(
        chatId: Long,
        afterSeq: Long,
        limit: Int,
    ): ApiResult<ReactionsCatchUpResponse> =
        client.get("/chats/$chatId/reactions?after_seq=$afterSeq&limit=$limit")
}
