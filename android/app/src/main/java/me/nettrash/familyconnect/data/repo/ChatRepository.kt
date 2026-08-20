/*
 * ChatRepository.kt
 * Family Connect (Android)
 *
 * Chat list + read reporting.
 *
 *   refreshChats — GET /chats merged into Room. The server's unread
 *                  count always wins (it is the authority); the local
 *                  read markers (my/peer last-read) survive the merge
 *                  because they only exist client-side.
 *   openChatId   — which chat the user is looking at right now; the
 *                  message pipeline consults it to decide whether an
 *                  inbound message bumps unread.
 *   postRead     — WS `read` frame when the socket is open, REST
 *                  fallback otherwise. Monotonic guard here; the
 *                  500 ms debounce lives in ChatViewModel (closest to
 *                  the scroll events that trigger it).
 *
 * iOS counterpart: ios/FamilyConnect/Data/Repo/ChatRepository.swift
 */

package me.nettrash.familyconnect.data.repo

import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import me.nettrash.familyconnect.data.db.ChatDao
import me.nettrash.familyconnect.data.db.ChatEntity
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.ChatApi
import me.nettrash.familyconnect.data.net.ws.ChatSocket
import me.nettrash.familyconnect.data.net.ws.ClientFrame
import me.nettrash.familyconnect.data.net.ws.SocketState
import me.nettrash.familyconnect.util.TimeFormat
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class ChatRepository @Inject constructor(
    private val chatApi: ChatApi,
    private val chatDao: ChatDao,
    private val socket: ChatSocket,
) {

    private val _openChatId = MutableStateFlow<Long?>(null)

    /** The chat currently on screen (null when none). */
    val openChatId: StateFlow<Long?> = _openChatId

    fun setOpenChat(chatId: Long?) {
        _openChatId.value = chatId
    }

    fun observeChats(): Flow<List<ChatEntity>> = chatDao.observeChats()

    fun observeChat(chatId: Long): Flow<ChatEntity?> = chatDao.observeById(chatId)

    /**
     * On success, returns the server's `max_reaction_seq` per chat id
     * (0 where the server omitted it) — SyncEngine compares it against
     * the locally stored cursor to decide whether reaction catch-up is
     * needed. The stored cursor itself is local-only and survives the
     * merge exactly like the read markers.
     */
    suspend fun refreshChats(): ApiResult<Map<Long, Long>> =
        when (val result = chatApi.chats()) {
            is ApiResult.Ok -> {
                val merged = result.value.chats.map { item ->
                    val existing = chatDao.getById(item.chat.id)
                    ChatEntity(
                        id = item.chat.id,
                        kind = item.chat.kind,
                        peerUserId = item.chat.peerUserId,
                        title = item.chat.title,
                        // Authoritative (protocol: resync step 2).
                        unreadCount = item.unreadCount,
                        // Local-only markers survive the merge.
                        myLastReadId = existing?.myLastReadId,
                        peerLastReadId = existing?.peerLastReadId,
                        lastMessageBody = item.lastMessage?.body ?: existing?.lastMessageBody,
                        lastMessageAt = item.lastMessage?.createdAt
                            ?.let(TimeFormat::parseTimestamp)
                            ?: existing?.lastMessageAt,
                        lastMessageSenderId = item.lastMessage?.senderId
                            ?: existing?.lastMessageSenderId,
                        // Local-only reaction cursor — NEVER the server's
                        // value: only applied states may advance it.
                        maxReactionSeq = existing?.maxReactionSeq ?: 0L,
                    )
                }
                chatDao.upsertAll(merged)
                ApiResult.Ok(
                    result.value.chats.associate { it.chat.id to (it.maxReactionSeq ?: 0L) },
                )
            }
            is ApiResult.HttpError -> result
            is ApiResult.NetworkError -> result
        }

    /** POST /chats/direct — get-or-create, idempotent server-side. */
    suspend fun createDirect(userId: Long): ApiResult<ChatEntity> =
        when (val result = chatApi.createDirect(userId)) {
            is ApiResult.Ok -> {
                val dto = result.value.chat
                val entity = chatDao.getById(dto.id) ?: ChatEntity(
                    id = dto.id,
                    kind = dto.kind,
                    peerUserId = dto.peerUserId,
                    title = dto.title,
                    unreadCount = 0,
                    myLastReadId = null,
                    peerLastReadId = null,
                    lastMessageBody = null,
                    lastMessageAt = null,
                    lastMessageSenderId = null,
                )
                chatDao.upsertAll(listOf(entity))
                ApiResult.Ok(entity)
            }
            is ApiResult.HttpError -> result
            is ApiResult.NetworkError -> result
        }

    /**
     * Report the highest message id read in this chat. Monotonic: a
     * report at or below what we already sent is dropped. WS preferred
     * (cheap, no response); REST fallback when the socket is down —
     * the server keeps the max either way.
     */
    suspend fun postRead(chatId: Long, lastReadMessageId: Long) {
        val chat = chatDao.getById(chatId)
        if (chat != null && (chat.myLastReadId ?: 0L) >= lastReadMessageId) return
        chatDao.setMyLastRead(chatId, lastReadMessageId)
        chatDao.clearUnread(chatId)
        val sentOverSocket = socket.state.value == SocketState.Open &&
            socket.trySend(ClientFrame.Read(chatId, lastReadMessageId))
        if (!sentOverSocket) {
            chatApi.postRead(chatId, lastReadMessageId)
        }
    }
}
