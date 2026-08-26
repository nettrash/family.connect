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
import me.nettrash.familyconnect.data.net.dto.MessagePollStateDto
import me.nettrash.familyconnect.data.net.dto.MessageReactionStateDto
import me.nettrash.familyconnect.data.net.dto.EditMessageRequest
import me.nettrash.familyconnect.data.net.dto.NewPollDto
import me.nettrash.familyconnect.data.net.dto.PollsCatchUpResponse
import me.nettrash.familyconnect.data.net.dto.MessageResponse
import me.nettrash.familyconnect.data.net.dto.MessagesResponse
import me.nettrash.familyconnect.data.net.dto.ReactionRequest
import me.nettrash.familyconnect.data.net.dto.ReactionsCatchUpResponse
import me.nettrash.familyconnect.data.net.dto.ReadRequest
import me.nettrash.familyconnect.data.net.dto.SendMessageRequest
import me.nettrash.familyconnect.data.net.dto.VoteRequest
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
        /** The options that make this message a poll; the body is the question. */
        poll: NewPollDto? = null,
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

    /**
     * Sets MY choice on a poll — an idempotent state-set, not a toggle
     * (whether tapping the option you already hold means "keep" or
     * "clear" is a decision each client makes locally). Re-PUTting the
     * same option is a server-side no-op that burns no seq.
     */
    suspend fun putVote(chatId: Long, messageId: Long, optionId: Long): ApiResult<MessagePollStateDto>

    /** Retracts MY vote; idempotent (returns the current state either way). */
    suspend fun deleteVote(chatId: Long, messageId: Long): ApiResult<MessagePollStateDto>

    /** Closes a poll. Author only, one-way; closing a closed poll is a no-op. */
    suspend fun closePoll(chatId: Long, messageId: Long): ApiResult<MessagePollStateDto>

    /** Poll catch-up page: strictly after [afterSeq], ascending. */
    suspend fun getPolls(chatId: Long, afterSeq: Long, limit: Int = 50): ApiResult<PollsCatchUpResponse>
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
        poll: NewPollDto?,
    ): ApiResult<MessageResponse> =
        // 201 on first delivery, 200 when the same client_msg_id retries —
        // both are 2xx, both decode to the same message. Never a duplicate.
        client.post(
            "/chats/$chatId/messages",
            SendMessageRequest(clientMsgId, body, replyToMessageId, attachmentId, poll),
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

    override suspend fun putVote(
        chatId: Long,
        messageId: Long,
        optionId: Long,
    ): ApiResult<MessagePollStateDto> =
        client.put("/chats/$chatId/messages/$messageId/vote", VoteRequest(optionId))

    override suspend fun deleteVote(
        chatId: Long,
        messageId: Long,
    ): ApiResult<MessagePollStateDto> =
        client.delete("/chats/$chatId/messages/$messageId/vote")

    override suspend fun closePoll(
        chatId: Long,
        messageId: Long,
    ): ApiResult<MessagePollStateDto> =
        // POST with no body — postEmpty, not post(Unit), which would
        // serialise "{}" into a request the server has no field for.
        client.postEmpty("/chats/$chatId/messages/$messageId/poll/close")

    override suspend fun getPolls(
        chatId: Long,
        afterSeq: Long,
        limit: Int,
    ): ApiResult<PollsCatchUpResponse> =
        client.get("/chats/$chatId/polls?after_seq=$afterSeq&limit=$limit")
}
