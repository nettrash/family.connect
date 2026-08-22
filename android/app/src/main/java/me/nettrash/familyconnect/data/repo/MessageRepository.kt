/*
 * MessageRepository.kt
 * Family Connect (Android)
 *
 * The send/receive pipeline. Design invariants:
 *
 *   - Optimistic send: the row is inserted as SENDING (PK = a fresh
 *     UUID client_msg_id) before any network I/O. The ack UPDATEs that
 *     same row — the UI never re-keys a bubble.
 *   - WS first, REST rescue: socket open → `send` frame + a 15 s ack
 *     timer. Timer fires (or socket closed) → REST POST with the SAME
 *     client_msg_id. The server dedups on (chat, sender, client_msg_id),
 *     so the worst case is the original message coming back as a 200.
 *   - Inbound `message` frames dedup three ways: our own clientMsgId →
 *     treated as the ack; known serverId → dropped; otherwise inserted
 *     under the synthetic PK "s<serverId>".
 *   - Unread bumps only for live frames in chats the user is NOT
 *     looking at, from senders other than me.
 *   - retry(clientMsgId) re-enters the pipeline with the same UUID —
 *     never a duplicate, even if the first attempt actually landed.
 *
 * iOS counterpart: ios/FamilyConnect/Data/Repo/MessageRepository.swift
 */

package me.nettrash.familyconnect.data.repo

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.db.ChatDao
import me.nettrash.familyconnect.data.db.MessageDao
import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.data.db.MessageStatus
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.ChatApi
import me.nettrash.familyconnect.data.net.dto.MessageDto
import me.nettrash.familyconnect.data.net.dto.ReactionDto
import me.nettrash.familyconnect.data.net.dto.ReactionsCodec
import me.nettrash.familyconnect.data.net.dto.ReplyToDto
import me.nettrash.familyconnect.data.net.ws.ChatSocket
import me.nettrash.familyconnect.data.net.ws.ClientFrame
import me.nettrash.familyconnect.data.net.ws.ServerFrame
import me.nettrash.familyconnect.data.net.ws.SocketState
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.di.AppScope
import me.nettrash.familyconnect.util.Clock
import me.nettrash.familyconnect.util.TimeFormat
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class MessageRepository @Inject constructor(
    private val chatApi: ChatApi,
    private val messageDao: MessageDao,
    private val chatDao: ChatDao,
    private val socket: ChatSocket,
    private val settings: SettingsRepository,
    private val chatRepository: ChatRepository,
    @param:AppScope private val scope: CoroutineScope,
    private val clock: Clock,
) {

    /** Ack timers keyed by client_msg_id; the ack cancels its timer. */
    private val pendingAcks = ConcurrentHashMap<String, Job>()

    init {
        scope.launch {
            socket.frames.collect { frame ->
                when (frame) {
                    is ServerFrame.Ack -> ackMessage(frame.clientMsgId, frame.message)
                    is ServerFrame.Message -> applyServerMessage(frame.message, live = true)
                    is ServerFrame.Read -> onPeerRead(frame)
                    is ServerFrame.Reaction -> onReaction(frame)
                    is ServerFrame.Error -> onSendError(frame)
                    else -> Unit
                }
            }
        }
    }

    fun observeMessages(chatId: Long, limit: Int): Flow<List<MessageEntity>> =
        messageDao.observeMessages(chatId, limit)

    // -- Outbound -----------------------------------------------------------------

    suspend fun send(chatId: Long, body: String, replyTo: ReplyToDto? = null) {
        val trimmed = body.trim()
        if (trimmed.isEmpty()) return
        val me = settings.state.first().myUserId ?: return
        val clientMsgId = UUID.randomUUID().toString()
        val now = clock.now()
        messageDao.insert(
            MessageEntity(
                clientMsgId = clientMsgId,
                serverId = null,
                chatId = chatId,
                senderId = me,
                body = trimmed,
                createdAt = now,
                status = MessageStatus.SENDING,
                // Held on the optimistic row so the bubble draws its quote
                // the instant it appears — and so a retry after a process
                // death still quotes the right message.
                replyToMessageId = replyTo?.messageId,
                replySenderId = replyTo?.senderId,
                replyExcerpt = replyTo?.excerpt,
            ),
        )
        chatDao.updateLastMessage(chatId, trimmed, now, me)
        dispatch(clientMsgId, chatId, trimmed, replyTo?.messageId)
    }

    /** Re-enter the pipeline with the SAME UUID — the server dedups. */
    suspend fun retry(clientMsgId: String) {
        val row = messageDao.findByClientMsgId(clientMsgId) ?: return
        if (row.serverId != null) return // already landed
        messageDao.setStatus(clientMsgId, MessageStatus.SENDING)
        dispatch(clientMsgId, row.chatId, row.body, row.replyToMessageId)
    }

    /** Discard a FAILED draft the user gave up on. */
    suspend fun deleteFailed(clientMsgId: String) {
        val row = messageDao.findByClientMsgId(clientMsgId) ?: return
        if (row.serverId != null) return // acked rows are history, not drafts
        pendingAcks.remove(clientMsgId)?.cancel()
        messageDao.deleteByClientMsgId(clientMsgId)
    }

    /** Reconnect step 4: re-send everything still awaiting an ack. */
    suspend fun flushPending() {
        messageDao.pendingSending().forEach { row ->
            if (!pendingAcks.containsKey(row.clientMsgId)) {
                dispatch(row.clientMsgId, row.chatId, row.body, row.replyToMessageId)
            }
        }
    }

    private fun dispatch(
        clientMsgId: String,
        chatId: Long,
        body: String,
        replyToMessageId: Long?,
    ) {
        val overSocket = socket.state.value == SocketState.Open &&
            socket.trySend(ClientFrame.Send(chatId, clientMsgId, body, replyToMessageId))
        if (overSocket) {
            pendingAcks[clientMsgId] = scope.launch {
                delay(ACK_TIMEOUT_MS)
                pendingAcks.remove(clientMsgId)
                // No ack in time — the frame may or may not have landed.
                // REST with the same client_msg_id is safe either way.
                restFallback(clientMsgId, chatId, body, replyToMessageId)
            }
        } else {
            scope.launch { restFallback(clientMsgId, chatId, body, replyToMessageId) }
        }
    }

    private suspend fun restFallback(
        clientMsgId: String,
        chatId: Long,
        body: String,
        replyToMessageId: Long?,
    ) {
        val row = messageDao.findByClientMsgId(clientMsgId) ?: return
        if (row.serverId != null) return // ack won the race
        when (val result = chatApi.postMessage(chatId, clientMsgId, body, replyToMessageId)) {
            is ApiResult.Ok -> ackMessage(clientMsgId, result.value.message)
            else -> messageDao.setStatus(clientMsgId, MessageStatus.FAILED)
        }
    }

    // -- Inbound -------------------------------------------------------------------

    private suspend fun ackMessage(clientMsgId: String, message: MessageDto) {
        pendingAcks.remove(clientMsgId)?.cancel()
        // A resync may have inserted this message under its synthetic
        // "s<id>" key before the ack reached us — drop that copy first or
        // markAcked would violate the serverId unique index.
        messageDao.findByServerId(message.id)?.let { existing ->
            if (existing.clientMsgId != clientMsgId) {
                messageDao.deleteByClientMsgId(existing.clientMsgId)
            }
        }
        val createdAt = TimeFormat.parseTimestamp(message.createdAt) ?: clock.now()
        messageDao.markAcked(clientMsgId, message.id, createdAt)
        // The server's recomputed snippet replaces the one this device
        // guessed when it enqueued the row — same rule as iOS's applyReply.
        messageDao.setReply(
            clientMsgId,
            message.replyTo?.messageId,
            message.replyTo?.senderId,
            message.replyTo?.excerpt,
        )
        chatDao.updateLastMessage(message.chatId, message.body, createdAt, message.senderId)
    }

    /**
     * One server-authored message, from a live `message` frame
     * (live=true) or a resync/history page (live=false).
     */
    suspend fun applyServerMessage(message: MessageDto, live: Boolean) {
        if (messageDao.existsByServerId(message.id)) {
            // Already held — but a re-delivered message (history page,
            // resync overlap) may carry NEWER embedded reactions. The
            // seq guard makes the apply a no-op when it doesn't.
            applyEmbeddedReactions(message)
            return
        }
        // My own message echoing back (WS ack lost, other path delivered,
        // or another of my devices sent it and this one holds the
        // optimistic row from a previous session): fold into that row.
        val own = messageDao.findByClientMsgId(message.clientMsgId)
        if (own != null && own.chatId == message.chatId && own.senderId == message.senderId) {
            ackMessage(message.clientMsgId, message)
            applyEmbeddedReactions(message)
            return
        }
        val createdAt = TimeFormat.parseTimestamp(message.createdAt) ?: clock.now()
        messageDao.insertIgnore(
            listOf(
                MessageEntity(
                    // Synthetic PK — inbound rows never had a local UUID.
                    clientMsgId = "s${message.id}",
                    serverId = message.id,
                    chatId = message.chatId,
                    senderId = message.senderId,
                    body = message.body,
                    createdAt = createdAt,
                    status = MessageStatus.SENT,
                    reactionsJson = message.reactions?.let(ReactionsCodec::encode),
                    reactionSeq = message.reactionSeq ?: 0L,
                    replyToMessageId = message.replyTo?.messageId,
                    replySenderId = message.replyTo?.senderId,
                    replyExcerpt = message.replyTo?.excerpt,
                ),
            ),
        )
        chatDao.updateLastMessage(message.chatId, message.body, createdAt, message.senderId)
        if (live) {
            val me = settings.state.first().myUserId
            val openChat = chatRepository.openChatId.value
            if (message.chatId != openChat && message.senderId != me) {
                chatDao.bumpUnread(message.chatId)
            }
        }
    }

    private suspend fun onPeerRead(frame: ServerFrame.Read) {
        // peerLastReadId drives the ✓✓ glyph, which only direct chats
        // show — and there the only other reader is the peer.
        val chat = chatDao.getById(frame.chatId) ?: return
        if (chat.kind == "direct" && chat.peerUserId == frame.userId) {
            chatDao.setPeerLastRead(frame.chatId, frame.lastReadMessageId)
        }
    }

    private suspend fun onSendError(frame: ServerFrame.Error) {
        val clientMsgId = frame.clientMsgId ?: return
        pendingAcks.remove(clientMsgId)?.cancel()
        messageDao.setStatus(clientMsgId, MessageStatus.FAILED)
    }

    // -- Reactions ------------------------------------------------------------------

    /**
     * Live `reaction` frame: full state, applied under the seq guard
     * (an unknown message matches zero rows — dropped silently, history
     * paging re-delivers the state embedded on the Message). The chat
     * cursor advances regardless: frames arrive in order on a live
     * socket, so this seq is proof everything below it was delivered.
     */
    private suspend fun onReaction(frame: ServerFrame.Reaction) {
        messageDao.applyReactionState(
            serverId = frame.messageId,
            json = ReactionsCodec.encode(frame.reactions),
            seq = frame.reactionSeq,
        )
        chatDao.advanceMaxReactionSeq(frame.chatId, frame.reactionSeq)
    }

    /**
     * Reactions riding on a fetched/re-delivered Message. Deliberately
     * does NOT advance the chat cursor: a history page proves nothing
     * about OTHER messages' lower seqs — only catch-up pages and live
     * frames may move it (protocol: never derive it from held messages).
     */
    private suspend fun applyEmbeddedReactions(message: MessageDto) {
        val seq = message.reactionSeq ?: return
        messageDao.applyReactionState(
            serverId = message.id,
            json = ReactionsCodec.encode(message.reactions.orEmpty()),
            seq = seq,
        )
    }

    /**
     * A tap on an emoji (chip or quick-set): my current reaction equals
     * it → remove, else set. Optimistic rewrite of reactionsJson first
     * (never the seq — the authoritative state must still pass the
     * guard), then REST; failure reverts. No retry — mirrors postRead's
     * stance, but WITH revert since the chips are visible state.
     */
    suspend fun toggleReaction(chatId: Long, messageServerId: Long, emoji: String) {
        val me = settings.state.first().myUserId ?: return
        val row = messageDao.findByServerId(messageServerId) ?: return
        val current = ReactionsCodec.decode(row.reactionsJson)
        val removing = current.any { it.userId == me && it.emoji == emoji }
        val optimistic = current.filterNot { it.userId == me } +
            if (removing) emptyList() else listOf(ReactionDto(userId = me, emoji = emoji))
        messageDao.setReactionsJson(
            messageServerId,
            ReactionsCodec.encode(optimistic),
            expectedSeq = row.reactionSeq,
        )

        val result = if (removing) {
            chatApi.deleteReaction(chatId, messageServerId)
        } else {
            chatApi.putReaction(chatId, messageServerId, emoji)
        }
        when (result) {
            is ApiResult.Ok -> {
                val state = result.value
                // Authoritative — through the guarded path, so a newer
                // WS frame that raced us is never overwritten.
                messageDao.applyReactionState(
                    serverId = state.messageId,
                    json = ReactionsCodec.encode(state.reactions),
                    seq = state.reactionSeq,
                )
                chatDao.advanceMaxReactionSeq(chatId, state.reactionSeq)
            }
            else -> messageDao.setReactionsJson(
                messageServerId,
                row.reactionsJson,
                expectedSeq = row.reactionSeq,
            )
        }
    }

    /**
     * Reaction catch-up (resync step 3b): pages strictly after
     * [afterSeq] until a short page. Each state applies under the seq
     * guard; the stored chat cursor advances to every page's max EVEN
     * when the referenced message isn't held locally — dropped states
     * come back embedded on Message objects when history pages there.
     */
    suspend fun catchUpReactions(chatId: Long, afterSeq: Long) {
        var cursor = afterSeq
        while (true) {
            val page = chatApi.getReactions(chatId, cursor, REACTION_PAGE)
                .okOrNull()?.messageReactions ?: return
            page.forEach { state ->
                messageDao.applyReactionState(
                    serverId = state.messageId,
                    json = ReactionsCodec.encode(state.reactions),
                    seq = state.reactionSeq,
                )
            }
            val pageMax = page.maxOfOrNull { it.reactionSeq }
            if (pageMax != null) {
                chatDao.advanceMaxReactionSeq(chatId, pageMax)
                cursor = pageMax
            }
            if (page.size < REACTION_PAGE) return
        }
    }

    // -- History paging ---------------------------------------------------------------

    /**
     * One reconnect catch-up page: strictly newer than [afterId],
     * oldest-first. Returns the page size, or null on failure (caller
     * stops looping — the next reconnect resumes from the same cursor).
     */
    suspend fun catchUp(chatId: Long, afterId: Long, limit: Int): Int? {
        val result = chatApi.messages(chatId, afterId = afterId, limit = limit)
        val page = result.okOrNull()?.messages ?: return null
        page.forEach { applyServerMessage(it, live = false) }
        return page.size
    }

    /**
     * Fetch the page older than the oldest loaded message (or the newest
     * page when nothing is loaded yet). Returns true when the start of
     * history was reached (short page).
     */
    suspend fun loadOlder(chatId: Long): Boolean {
        val oldest = messageDao.oldestServerId(chatId)
        val result = chatApi.messages(chatId, beforeId = oldest, limit = HISTORY_PAGE)
        val page = result.okOrNull()?.messages ?: return false
        page.forEach { applyServerMessage(it, live = false) }
        return page.size < HISTORY_PAGE
    }

    private companion object {
        const val ACK_TIMEOUT_MS = 15_000L
        const val HISTORY_PAGE = 50
        const val REACTION_PAGE = 200
    }
}
