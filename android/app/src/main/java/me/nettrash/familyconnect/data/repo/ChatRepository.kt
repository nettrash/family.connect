/*
 * ChatRepository.kt
 * Family Connect (Android)
 *
 * Chat list + read reporting.
 *
 *   refreshChats — GET /chats merged into Room. The server's unread
 *                  count always wins (it is the authority); MY read
 *                  marker is merged monotonically with the one the
 *                  response now carries (`last_read_message_id` —
 *                  protocol resync step 2), and the PEER's survives the
 *                  merge untouched because nothing on the response
 *                  describes it. Messages the socket delivered WHILE
 *                  the request was in flight are
 *                  added back on top, but only the ones newer than the
 *                  `last_message` the response carries — the rest the
 *                  server had already counted. A direct chat the
 *                  response does NOT list is pruned — see below.
 *   deleteDirectChat
 *                — the one way a chat leaves this device, reached from
 *                  the two paths a vanished chat is learned about: the
 *                  `member_deleted` frame (immediate) and the prune in
 *                  refreshChats (the repair for a device that was
 *                  offline when it happened).
 *   openChatId   — which chat the user is looking at right now, and
 *                  whether its newest message is actually on screen; the
 *                  message pipeline consults both to decide whether an
 *                  inbound message bumps unread.
 *   postRead     — WS `read` frame when the socket is open, REST
 *                  fallback otherwise. Monotonic guard here; the
 *                  500 ms debounce lives in ChatViewModel (closest to
 *                  the scroll events that trigger it).
 *   applyMyReadMarker
 *                — the same marker arriving from ANOTHER of this
 *                  person's devices, off a `read` frame. Monotonic, and
 *                  it recounts rather than clears.
 *
 * THE BADGE. A read marker belongs to the person and not the device, so
 * both of the paths above end at UnreadNotifications: a chat read
 * anywhere has to take its notification — which on Android IS the badge
 * — down here too. `refreshChats` is the one that fires on today's
 * server, because `deliver_read` fans the frame out to OTHER members and
 * the reader's own devices never see it (server/src/events.rs).
 *
 * iOS counterpart: ios/FamilyConnect/Data/Repo/ChatRepository.swift
 */

package me.nettrash.familyconnect.data.repo

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.db.ChatDao
import me.nettrash.familyconnect.data.db.ChatEntity
import me.nettrash.familyconnect.data.db.MessageDao
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.ChatApi
import me.nettrash.familyconnect.data.net.ws.ChatSocket
import me.nettrash.familyconnect.data.net.ws.ClientFrame
import me.nettrash.familyconnect.data.net.ws.ServerFrame
import me.nettrash.familyconnect.data.net.ws.SocketState
import me.nettrash.familyconnect.data.push.UnreadNotifications
import me.nettrash.familyconnect.di.AppScope
import me.nettrash.familyconnect.util.TimeFormat
import java.util.concurrent.ConcurrentHashMap
import javax.inject.Inject
import javax.inject.Singleton

/** What the server reports a chat has, against which the local cursors are compared. */
data class ServerCursors(val reactions: Long, val edits: Long, val polls: Long)

@Singleton
class ChatRepository @Inject constructor(
    private val chatApi: ChatApi,
    private val chatDao: ChatDao,
    private val messageDao: MessageDao,
    private val socket: ChatSocket,
    /**
     * Injected rather than reached for statically because constructing it
     * is what STARTS the unread sweep — this repository is in the graph
     * Hilt builds before the first frame can arrive (FamilyConnectApp),
     * and the badge has to be watching from then on.
     */
    private val unreadNotifications: UnreadNotifications,
    @param:AppScope private val scope: CoroutineScope,
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

    // Declared AFTER the state it touches: the collector body may run on
    // the very first dispatch after construction, and a lambda reading a
    // property whose initializer has not run yet sees null.
    init {
        scope.launch {
            socket.frames.collect { frame ->
                // A peer deleted their account, so the direct chat with
                // them no longer exists on the server — the immediate
                // half of the repair, so a member watching the chat list
                // sees the row go rather than finding out by tapping it
                // and getting 404s out of every call. The ROSTER half of
                // this same frame is FamilyRepository's: their tombstone
                // row STAYS, because the family chat still holds messages
                // that have to be given a name (docs/protocol.md,
                // "Deleting an account").
                //
                // Keyed on the member, never on the frame's family: a
                // peer this account only ever shared a direct chat with
                // may be in another family altogether, or in none.
                if (frame is ServerFrame.MemberDeleted) {
                    for (chatId in chatDao.directChatIdsWith(frame.member.id)) {
                        deleteDirectChat(chatId)
                    }
                }
            }
        }
    }

    fun observeChats(): Flow<List<ChatEntity>> = chatDao.observeChats()

    fun observeChat(chatId: Long): Flow<ChatEntity?> = chatDao.observeById(chatId)

    /**
     * The chat row as it stands right now, once.
     *
     * The opening anchor needs `unreadCount` and `myLastReadId` as they
     * were the moment the screen opened, and it needs them TOGETHER: the
     * flow's first emission can be a null while Room answers, and its
     * later ones already have the read path's zero in them.
     */
    suspend fun chatSnapshot(chatId: Long): ChatEntity? = chatDao.getById(chatId)

    /**
     * On success, returns the server's `max_reaction_seq`, `max_edit_seq`
     * and `max_poll_seq` per chat id (0 where the server omitted any of
     * them) — SyncEngine compares each against the locally stored cursor
     * to decide whether that catch-up is needed. The stored cursors are
     * local-only and survive the merge exactly like the read markers.
     */
    suspend fun refreshChats(): ApiResult<Map<Long, ServerCursors>> {
        // Snapshot BEFORE the await. Anything the socket delivers while
        // the request is in flight may or may not be in the count that
        // comes back, and the whole-row rebuild below would drop what is
        // not — permanently, because the catch-up path never bumps and
        // the server marker never goes backwards.
        val idsAtRequest = liveUnreadIds.mapValues { it.value.toSet() }
        // Snapshotted before the await for the same reason, and it is the
        // prune's whole safety margin: a direct chat this device started
        // WHILE the request was in flight cannot be in a response the
        // server computed before it existed, and pruning it would delete
        // the conversation the user is at that moment typing into.
        // Only chats that were already here when we asked are candidates.
        val directAtRequest = chatDao.directChatIds()
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
                        // MY marker is no longer local-only: `GET /chats`
                        // reports it (protocol: resync step 2), and it is
                        // the only thing that can tell this device the
                        // chat was read on another one. MONOTONIC, never
                        // assigned — see [mergedReadMarker].
                        myLastReadId = mergedReadMarker(
                            stored = existing?.myLastReadId,
                            received = item.lastReadMessageId,
                        ),
                        // The PEER's marker still is: nothing on this
                        // response describes it.
                        peerLastReadId = existing?.peerLastReadId,
                        // What arrived rather than the raw body: a
                        // caption-less photo has an EMPTY body, which is
                        // not null, so the row rendered blank.
                        lastMessageBody = item.lastMessage
                            ?.let {
                                MessageRepository.previewText(
                                    it.body,
                                    it.resolvedAttachments,
                                    it.call,
                                )
                            }
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
                        maxPollSeq = existing?.maxPollSeq ?: 0L,
                    )
                }
                chatDao.upsertAll(merged)
                // Anything DIRECT the server no longer lists is gone, so
                // it goes here too — the repair that heals a device which
                // was offline when the peer deleted their account and
                // never saw the frame. Without it the row sits in the
                // list under the peer's OLD name and answers 404 to
                // everything.
                //
                // Only on a response that actually came back, and `GET
                // /chats` has no cursor and no pages: one 200 is the
                // whole list, which is what makes "not in the response"
                // mean "gone" rather than "not on this page". A network
                // failure or an error takes the two branches below and
                // deletes nothing — a flaky connection must never wipe
                // somebody's history.
                val listed = result.value.chats.mapTo(HashSet()) { it.chat.id }
                for (chatId in directAtRequest) {
                    if (chatId !in listed) deleteDirectChat(chatId)
                }
                // The cross-device half of the badge. A chat the SERVER
                // says has nothing unread has been read by this person —
                // on this device or on another one, the marker does not
                // distinguish — so anything this device is still showing
                // about it in the tray is stale. Read from the RESPONSE
                // and not from the merged rows, which may carry
                // `arrivedDuringRequest` on top: a message that arrived
                // during the await came over a LIVE socket, and the
                // server never pushes to a device holding one — so it has
                // no notification of its own, and letting it veto the
                // sweep would leave the stale one lit.
                unreadNotifications.onServerSaysRead(
                    result.value.chats
                        .filter { it.unreadCount <= 0 }
                        .mapTo(HashSet()) { it.chat.id },
                )
                ApiResult.Ok(
                    result.value.chats.associate {
                        it.chat.id to ServerCursors(
                            reactions = it.maxReactionSeq ?: 0L,
                            edits = it.maxEditSeq ?: 0L,
                            polls = it.maxPollSeq ?: 0L,
                        )
                    },
                )
            }
            is ApiResult.HttpError -> result
            is ApiResult.NetworkError -> result
        }
    }

    /**
     * Drop one direct chat and everything this device keeps about it.
     *
     * The ONE way a chat leaves this device, shared by both halves of the
     * repair — the `member_deleted` frame above and the prune in
     * [refreshChats] — so the two can never disagree about what "gone"
     * means. Deliberately takes a chat id that has already been proved
     * direct: the family chat and the assistant chat cannot disappear
     * server-side, so nothing may ever delete them here.
     *
     * Messages first, then the row: the reverse order would leave, for an
     * instant, a chat list with no row to hang the messages off and a
     * screen that could re-open an empty chat. Everything else keyed by
     * chat id lives ON the row — the unread badge, both read markers and
     * the three catch-up cursors — so it goes with it. The live-unread
     * ledger is the only piece held in memory rather than in Room, and it
     * is dropped here too; drafts are in the ViewModel and die with the
     * screen, and an attachment is cached under its own id, never a
     * chat's.
     */
    private suspend fun deleteDirectChat(chatId: Long) {
        messageDao.deleteByChat(chatId)
        chatDao.deleteById(chatId)
        liveUnreadIds.remove(chatId)
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

    /**
     * Apply a read marker this person set on ANOTHER of their devices.
     *
     * The marker belongs to the person, not the device (docs/protocol.md,
     * `GET /chats` → `last_read_message_id`): reading on a tablet must
     * put the phone's badge down too. Applied monotonically by the DAO's
     * MAX(), so a frame that crosses a read happening here cannot walk
     * this device's marker backwards.
     *
     * The count is RECOUNTED rather than cleared, because a marker is a
     * threshold and not an ending: another device reading up to 1337 says
     * nothing about the two messages that arrived after it, and clearing
     * would drop them from the badge for good — `GET /chats` computes
     * from the same marker and would agree with the wrong answer.
     * [MessageDao.countInboundAfter] carries the caveat: it counts what
     * this device HOLDS, which is everything only because this path is
     * reached from a live socket.
     *
     * [myUserId] is passed in because the caller (MessageRepository, off
     * the `read` frame) already had to know who it is to decide the frame
     * was about this user at all.
     */
    suspend fun applyMyReadMarker(chatId: Long, lastReadMessageId: Long, myUserId: Long) {
        chatDao.setMyLastRead(chatId, lastReadMessageId)
        // Re-read rather than reused: MAX() may have kept a HIGHER marker
        // this device already had, and recounting against the incoming
        // one would then resurrect messages it has already read.
        val marker = chatDao.getById(chatId)?.myLastReadId ?: lastReadMessageId
        val remaining = messageDao.countInboundAfter(chatId, marker, myUserId)
        chatDao.setUnreadCount(chatId, remaining)
        if (remaining == 0) {
            // Directly as well as through the sweep: the sweep only fires
            // on a count this process watched go DOWN, and a device that
            // never saw the pushed message was already sitting at zero
            // with the notification still up.
            unreadNotifications.onServerSaysRead(setOf(chatId))
        }
    }

    companion object {
        /**
         * `max(stored, received)` — the rule protocol.md states for
         * `last_read_message_id`, and the reason it is a function rather
         * than an assignment.
         *
         * A `GET /chats` still in flight while the reader is reading
         * carries a marker from before they read: writing it in would
         * walk the marker backwards, and this client's monotonic guard in
         * [postRead] would then re-report reads the server already has —
         * or, worse, the chat list would re-inflate a count the reader
         * just cleared.
         *
         * Absent (an older server) and `0` ("never reported anything
         * here") land in the same place on purpose: both leave whatever
         * is stored exactly as it was, including a stored NULL, which is
         * this schema's way of saying the same thing as the protocol's 0.
         */
        fun mergedReadMarker(stored: Long?, received: Long?): Long? {
            val merged = maxOf(stored ?: 0L, received ?: 0L)
            return if (merged > 0L) merged else stored
        }
    }
}
