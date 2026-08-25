/*
 * ChatRepository.kt
 * Family Connect (Android)
 *
 * Chat list + read reporting.
 *
 *   refreshChats — GET /chats merged into Room. The server's unread
 *                  count always wins (it is the authority); the local
 *                  read markers (my/peer last-read) survive the merge
 *                  because they only exist client-side. Messages the
 *                  socket delivered WHILE the request was in flight are
 *                  added back on top, but only the ones newer than the
 *                  `last_message` the response carries — the rest the
 *                  server had already counted.
 *   openChatId   — which chat the user is looking at right now, and
 *                  whether its newest message is actually on screen; the
 *                  message pipeline consults both to decide whether an
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
import java.util.concurrent.ConcurrentHashMap
import javax.inject.Inject
import javax.inject.Singleton

/** What the server reports a chat has, against which the local cursors are compared. */
data class ServerCursors(val reactions: Long, val edits: Long)

@Singleton
class ChatRepository @Inject constructor(
    private val chatApi: ChatApi,
    private val chatDao: ChatDao,
    private val socket: ChatSocket,
) {

    private val _openChatId = MutableStateFlow<Long?>(null)

    /** The chat currently on screen (null when none). */
    val openChatId: StateFlow<Long?> = _openChatId

    private val _openChatAtNewest = MutableStateFlow(false)

    /**
     * Whether the open chat is parked at its newest message.
     *
     * "Open" alone is not "read": a reader thirty messages up the thread
     * is looking at the chat and at none of what is arriving — the list
     * deliberately does not scroll down to a new message for them
     * (ChatScreen's follow rule), so it never reaches the screen. Without
     * this second signal that message would be swallowed by the
     * suppression below and never counted, and the count would never come
     * back: the server's read marker is monotonic.
     */
    val openChatAtNewest: StateFlow<Boolean> = _openChatAtNewest

    /**
     * Publish what the conversation screen is showing. Both halves move
     * together so a stale "at newest" from the chat just left can never
     * be read against the chat just entered.
     */
    fun setOpenChat(chatId: Long?, atNewest: Boolean) {
        _openChatId.value = chatId
        _openChatAtNewest.value = chatId != null && atNewest
    }

    /**
     * Count one inbound message against a chat's badge.
     *
     * Routed through here rather than straight at the DAO so the bump is
     * also recorded in [liveUnreadIds] — a bump the DAO learns about and
     * this ledger does not is exactly the message [refreshChats] would
     * overwrite away.
     */
    suspend fun bumpUnread(chatId: Long, messageId: Long) {
        chatDao.bumpUnread(chatId)
        liveUnreadIds.computeIfAbsent(chatId) { ConcurrentHashMap.newKeySet() }.add(messageId)
    }

    /**
     * The message IDS this process has counted live, per chat — not a
     * count, and the difference matters.
     *
     * The server COMMITS a message and only then broadcasts it, and it
     * serves a concurrent `GET /chats` from the same database: a message
     * that races the request may perfectly well already be inside the
     * `unread_count` that comes back. A bare counter cannot tell that
     * apart from a message the server had not seen yet, so it added the
     * same message twice and the badge overstated by one until the next
     * refresh that nothing raced.
     *
     * With ids there is something to compare against: the response's own
     * `last_message` is the newest message the server knew about when it
     * answered, so anything at or below it was counted and anything above
     * it was not. [refreshChats] drops the confirmed ids afterwards,
     * which is also what stops this growing with the conversation.
     *
     * Concurrent because the bumps come off the socket's scope while the
     * refresh runs on the caller's.
     */
    private val liveUnreadIds = ConcurrentHashMap<Long, MutableSet<Long>>()

    fun observeChats(): Flow<List<ChatEntity>> = chatDao.observeChats()

    fun observeChat(chatId: Long): Flow<ChatEntity?> = chatDao.observeById(chatId)

    /**
     * On success, returns the server's `max_reaction_seq` and
     * `max_edit_seq` per chat id (0 where the server omitted either) —
     * SyncEngine compares each against the locally stored cursor to
     * decide whether that catch-up is needed. The stored cursors are
     * local-only and survive the merge exactly like the read markers.
     */
    suspend fun refreshChats(): ApiResult<Map<Long, ServerCursors>> {
        // Snapshot BEFORE the await. Anything the socket delivers while
        // the request is in flight may or may not be in the count that
        // comes back, and the whole-row rebuild below would drop what is
        // not — permanently, because the catch-up path never bumps and
        // the server marker never goes backwards.
        val idsAtRequest = liveUnreadIds.mapValues { it.value.toSet() }
        return when (val result = chatApi.chats()) {
            is ApiResult.Ok -> {
                val merged = result.value.chats.map { item ->
                    val existing = chatDao.getById(item.chat.id)
                    // Everything the server had already counted stops at
                    // `last_message`: that is the newest message it knew
                    // about when it answered. A message delivered live
                    // during the await counts on top only if it is NEWER
                    // than that — otherwise it is already inside
                    // `unread_count` and adding it again is the phantom
                    // unread this reconciliation exists to avoid. No
                    // `last_message` at all means an empty chat, so
                    // nothing was counted and everything is new.
                    val countedThrough = item.lastMessage?.id ?: 0L
                    val live = liveUnreadIds[item.chat.id]
                    val before = idsAtRequest[item.chat.id].orEmpty()
                    val arrivedDuringRequest = live.orEmpty()
                        .count { it !in before && it > countedThrough }
                    // The server has confirmed everything up to here, so
                    // those ids have no further say — dropping them is
                    // what keeps the ledger the size of a race rather
                    // than the size of the conversation.
                    live?.removeAll { it <= countedThrough }
                    ChatEntity(
                        id = item.chat.id,
                        kind = item.chat.kind,
                        peerUserId = item.chat.peerUserId,
                        title = item.chat.title,
                        // Authoritative (protocol: resync step 2), plus
                        // what arrived after the server counted.
                        unreadCount = item.unreadCount + arrivedDuringRequest,
                        // Local-only markers survive the merge.
                        myLastReadId = existing?.myLastReadId,
                        peerLastReadId = existing?.peerLastReadId,
                        // What arrived rather than the raw body: a
                        // caption-less photo has an EMPTY body, which is
                        // not null, so the row rendered blank.
                        lastMessageBody = item.lastMessage
                            ?.let { MessageRepository.previewText(it.body, it.attachment) }
                            ?: existing?.lastMessageBody,
                        lastMessageAt = item.lastMessage?.createdAt
                            ?.let(TimeFormat::parseTimestamp)
                            ?: existing?.lastMessageAt,
                        lastMessageSenderId = item.lastMessage?.senderId
                            ?: existing?.lastMessageSenderId,
                        // Local-only cursors — NEVER the server's values:
                        // only applied states may advance them.
                        maxReactionSeq = existing?.maxReactionSeq ?: 0L,
                        maxEditSeq = existing?.maxEditSeq ?: 0L,
                    )
                }
                chatDao.upsertAll(merged)
                ApiResult.Ok(
                    result.value.chats.associate {
                        it.chat.id to ServerCursors(
                            reactions = it.maxReactionSeq ?: 0L,
                            edits = it.maxEditSeq ?: 0L,
                        )
                    },
                )
            }
            is ApiResult.HttpError -> result
            is ApiResult.NetworkError -> result
        }
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
        // First, and deliberately NOT behind the monotonic guard: the
        // badge is a local DISPLAY of "you have something to look at",
        // and the user is looking at it. Guarding this is how a chat ends
        // up with a count that opening it can never clear — the marker
        // already at the newest id (a post that reached the server, or one
        // that did not) makes the guard return before the badge is
        // touched, and `GET /chats` keeps re-inflating it.
        chatDao.clearUnread(chatId)
        val chat = chatDao.getById(chatId)
        if (chat != null && (chat.myLastReadId ?: 0L) >= lastReadMessageId) return
        val sentOverSocket = socket.state.value == SocketState.Open &&
            socket.trySend(ClientFrame.Read(chatId, lastReadMessageId))
        val reported = sentOverSocket || chatApi.postRead(chatId, lastReadMessageId) is ApiResult.Ok
        // myLastReadId mirrors what the SERVER holds, so it may only move
        // once the server has actually been told. Advancing it first — on
        // a dropped socket or a 500 — silences every later attempt through
        // the guard above while the server stays behind, and the read is
        // then lost for good.
        if (reported) {
            chatDao.setMyLastRead(chatId, lastReadMessageId)
        }
    }
}
