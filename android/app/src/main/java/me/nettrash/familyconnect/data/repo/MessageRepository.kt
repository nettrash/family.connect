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
 *   - Unread bumps only for live frames from senders other than me,
 *     and only where the message did not land in front of the user —
 *     the chat open AND parked at its newest message.
 *   - retry(clientMsgId) re-enters the pipeline with the same UUID —
 *     never a duplicate, even if the first attempt actually landed.
 *
 * iOS counterpart: ios/FamilyConnect/Data/Repo/MessageRepository.swift
 */

package me.nettrash.familyconnect.data.repo

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
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
import me.nettrash.familyconnect.data.net.AttachmentApi
import me.nettrash.familyconnect.data.net.ChatApi
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
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
    private val attachmentApi: AttachmentApi,
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
                    is ServerFrame.MessageEdited -> {
                        // The authoritative body: whatever was accumulated
                        // from deltas is replaced, and the row stops being
                        // "still being written".
                        _streamingMessageIds.update { it - frame.message.id }
                        applyEdit(frame.message)
                    }
                    is ServerFrame.AiDelta -> appendAssistantDelta(frame)
                    is ServerFrame.AiError ->
                        // Whatever arrived is already on the row and stays
                        // there — a partial answer beats a bubble that never
                        // resolves.
                        _streamingMessageIds.update { it - frame.messageId }
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
        dispatch(clientMsgId, chatId, trimmed, replyTo?.messageId, attachmentId = null)
    }

    /**
     * Send a photo or video.
     *
     * The upload happens FIRST and the row is inserted only once the
     * bytes have an id — a bubble pointing at an upload that never landed
     * would be worse than a composer that is visibly busy. Which is why,
     * unlike [send], this one is not optimistic.
     *
     * The preview upload is best-effort and comes BEFORE the message so
     * the first render of the bubble already has a thumbnail; failing it
     * is survivable (the bubble fetches the full photo instead), so it
     * never fails the send.
     *
     * Returns false when the bytes did not make it — the caller keeps the
     * picked media and says so.
     */
    /**
     * Share a place.
     *
     * The same shape as [sendMedia] — upload first, insert the row only
     * once the attachment has an id — and for the same reason: a bubble
     * pointing at an upload that failed is worse than a composer that is
     * visibly busy. What differs is that there is nothing to upload and no
     * file to delete afterwards (docs/protocol.md, "Locations").
     */
    suspend fun sendLocation(
        latitude: Double,
        longitude: Double,
        accuracyM: Int?,
        label: String?,
        caption: String,
        chatId: Long,
        replyTo: ReplyToDto? = null,
    ): Boolean {
        val me = settings.state.first().myUserId ?: return false
        val uploaded = attachmentApi.uploadLocation(latitude, longitude, accuracyM, label)
        val attachment = (uploaded as? ApiResult.Ok)?.value?.attachment ?: return false

        val body = caption.trim()
        val clientMsgId = UUID.randomUUID().toString()
        val now = clock.now()
        messageDao.insert(
            MessageEntity(
                clientMsgId = clientMsgId,
                serverId = null,
                chatId = chatId,
                senderId = me,
                body = body,
                createdAt = now,
                status = MessageStatus.SENDING,
                attachmentId = attachment.id,
                attachmentKind = attachment.kind,
                attachmentMime = attachment.mime,
                attachmentSize = attachment.size,
                attachmentName = attachment.name,
                attachmentLatitude = attachment.latitude,
                attachmentLongitude = attachment.longitude,
                attachmentAccuracyM = attachment.accuracyM,
                replyToMessageId = replyTo?.messageId,
                replySenderId = replyTo?.senderId,
                replyExcerpt = replyTo?.excerpt,
            ),
        )
        chatDao.updateLastMessage(chatId, previewText(body, attachment), now, me)
        dispatch(clientMsgId, chatId, body, replyTo?.messageId, attachment.id)
        return true
    }

    suspend fun sendMedia(
        prepared: MediaPrep.Prepared,
        caption: String,
        chatId: Long,
        replyTo: ReplyToDto? = null,
    ): Boolean {
        try {
            val me = settings.state.first().myUserId ?: return false
            val uploaded = attachmentApi.upload(
                file = prepared.file,
                mime = prepared.mime,
                kind = prepared.kind,
                width = prepared.width,
                height = prepared.height,
                durationMs = prepared.durationMs,
                name = prepared.name,
            )
            val attachment = (uploaded as? ApiResult.Ok)?.value?.attachment ?: return false

            var hasPreview = false
            prepared.previewJpeg?.let { jpeg ->
                hasPreview = attachmentApi.uploadPreview(attachment.id, jpeg) is ApiResult.Ok
            }

            val body = caption.trim()
            val clientMsgId = UUID.randomUUID().toString()
            val now = clock.now()
            messageDao.insert(
                MessageEntity(
                    clientMsgId = clientMsgId,
                    serverId = null,
                    chatId = chatId,
                    senderId = me,
                    // A photo needs no caption: an empty body is allowed
                    // WITH an attachment (protocol.md), and only then.
                    body = body,
                    createdAt = now,
                    status = MessageStatus.SENDING,
                    attachmentId = attachment.id,
                    attachmentKind = attachment.kind,
                    attachmentMime = attachment.mime,
                    attachmentSize = attachment.size,
                    attachmentWidth = attachment.width,
                    attachmentHeight = attachment.height,
                    attachmentDurationMs = attachment.durationMs,
                    attachmentHasPreview = hasPreview,
                    attachmentName = attachment.name,
                    attachmentLatitude = attachment.latitude,
                    attachmentLongitude = attachment.longitude,
                    attachmentAccuracyM = attachment.accuracyM,
                    // A photo can answer a message like any other reply;
                    // this used to be dropped on the floor, so the quote
                    // vanished and its banner stayed armed.
                    replyToMessageId = replyTo?.messageId,
                    replySenderId = replyTo?.senderId,
                    replyExcerpt = replyTo?.excerpt,
                ),
            )
            // What arrived, not an empty string: a caption-less photo left
            // the chat-list row blank because "" is not null and the
            // fallback never fired.
            chatDao.updateLastMessage(chatId, previewText(body, attachment), now, me)
            dispatch(clientMsgId, chatId, body, replyTo?.messageId, attachment.id)
            return true
        } finally {
            // The prepared file is ours whatever happened to it.
            prepared.file.delete()
        }
    }

    /**
     * What the chat list shows on its second line.
     *
     * A photo is normally sent with no caption, and an empty string is not
     * null — so the row rendered blank rather than falling back. Mirrors
     * iOS's ChatSyncCoordinator.preview.
     */
    fun previewText(body: String, attachment: AttachmentDto?): String =
        Companion.previewText(body, attachment)

    /**
     * Assistant replies still being written, by server id. Drives the
     * bubble's cursor and nothing else; it is deliberately in memory only,
     * so a row that was mid-stream when the app was killed is not stuck
     * looking live after a relaunch.
     */
    private val _streamingMessageIds = MutableStateFlow<Set<Long>>(emptySet())
    val streamingMessageIds: StateFlow<Set<Long>> = _streamingMessageIds

    /**
     * Append one fragment to the assistant's row.
     *
     * Deltas carry no `editSeq`, so this never fights the edit guard: the
     * final body arrives as an edit with a real seq and overwrites whatever
     * was accumulated — which is also how a client that missed every
     * fragment ends up correct.
     */
    private suspend fun appendAssistantDelta(frame: ServerFrame.AiDelta) {
        val appended = messageDao.appendToBody(frame.messageId, frame.text)
        if (appended > 0) {
            _streamingMessageIds.update { it + frame.messageId }
        }
    }

    /** Re-enter the pipeline with the SAME UUID — the server dedups. */
    suspend fun retry(clientMsgId: String) {
        val row = messageDao.findByClientMsgId(clientMsgId) ?: return
        if (row.serverId != null) return // already landed
        messageDao.setStatus(clientMsgId, MessageStatus.SENDING)
        dispatch(clientMsgId, row.chatId, row.body, row.replyToMessageId, row.attachmentId)
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
                dispatch(row.clientMsgId, row.chatId, row.body, row.replyToMessageId, row.attachmentId)
            }
        }
    }

    private fun dispatch(
        clientMsgId: String,
        chatId: Long,
        body: String,
        replyToMessageId: Long?,
        attachmentId: Long?,
    ) {
        val overSocket = socket.state.value == SocketState.Open &&
            socket.trySend(
                ClientFrame.Send(chatId, clientMsgId, body, replyToMessageId, attachmentId),
            )
        if (overSocket) {
            pendingAcks[clientMsgId] = scope.launch {
                delay(ACK_TIMEOUT_MS)
                pendingAcks.remove(clientMsgId)
                // No ack in time — the frame may or may not have landed.
                // REST with the same client_msg_id is safe either way.
                restFallback(clientMsgId, chatId, body, replyToMessageId, attachmentId)
            }
        } else {
            scope.launch { restFallback(clientMsgId, chatId, body, replyToMessageId, attachmentId) }
        }
    }

    private suspend fun restFallback(
        clientMsgId: String,
        chatId: Long,
        body: String,
        replyToMessageId: Long?,
        attachmentId: Long?,
    ) {
        val row = messageDao.findByClientMsgId(clientMsgId) ?: return
        if (row.serverId != null) return // ack won the race
        val result = chatApi.postMessage(chatId, clientMsgId, body, replyToMessageId, attachmentId)
        when (result) {
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
            // Blanket-overwritten, INCLUDING to null: unlike the first
            // level, the second legitimately becomes absent when retention
            // sweeps a grandparent, and a stale copy would draw a quote of
            // a message that no longer exists.
            message.replyTo?.parent?.messageId,
            message.replyTo?.parent?.senderId,
            message.replyTo?.parent?.excerpt,
        )
        // An attachment is fixed at send time — except has_preview, which
        // flips once the preview upload lands. The server's copy is the
        // one that counts, so it replaces what this device guessed.
        messageDao.setAttachment(
            clientMsgId,
            message.attachment?.id,
            message.attachment?.kind,
            message.attachment?.mime,
            message.attachment?.size ?: 0L,
            message.attachment?.width,
            message.attachment?.height,
            message.attachment?.durationMs,
            message.attachment?.hasPreview ?: false,
            message.attachment?.name,
            message.attachment?.latitude,
            message.attachment?.longitude,
            message.attachment?.accuracyM,
        )
        chatDao.updateLastMessage(
            message.chatId,
            previewText(message.body, message.attachment),
            createdAt,
            message.senderId,
        )
    }

    /**
     * Apply an edited body under the seq guard, and advance the chat's
     * edit cursor so a later catch-up does not replay it.
     *
     * Never bumps unread and never notifies: an edit is not new mail.
     */
    suspend fun applyEdit(message: MessageDto) {
        val seq = message.editSeq ?: return
        val editedAt = message.editedAt?.let(TimeFormat::parseTimestamp)
        val updated = messageDao.applyEdit(message.id, message.body, seq, editedAt)
        if (updated > 0) {
            // A quote is a snapshot of the body, so every local reply
            // pointing at this message is now stale.
            val excerpt = ReplyToDto.excerpt(message.body)
            messageDao.refreshQuotesOf(message.id, excerpt)
            messageDao.refreshParentQuotesOf(message.id, excerpt)
        }
        chatDao.advanceMaxEditSeq(message.chatId, seq)
    }

    /**
     * One server-authored message, from a live `message` frame
     * (live=true) or a resync/history page (live=false).
     */
    suspend fun applyServerMessage(message: MessageDto, live: Boolean) {
        if (messageDao.existsByServerId(message.id)) {
            // Already held — but a re-delivered message (history page,
            // resync overlap) may carry NEWER embedded reactions, or a
            // newer BODY. Both applies are seq-guarded, so an older copy
            // is a no-op rather than a revert.
            applyEmbeddedReactions(message)
            if (message.editSeq != null) applyEdit(message)
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
                    replyParentMessageId = message.replyTo?.parent?.messageId,
                    replyParentSenderId = message.replyTo?.parent?.senderId,
                    replyParentExcerpt = message.replyTo?.parent?.excerpt,
                    editSeq = message.editSeq ?: 0L,
                    editedAt = message.editedAt?.let(TimeFormat::parseTimestamp),
                    attachmentId = message.attachment?.id,
                    attachmentKind = message.attachment?.kind,
                    attachmentMime = message.attachment?.mime,
                    attachmentSize = message.attachment?.size ?: 0L,
                    attachmentWidth = message.attachment?.width,
                    attachmentHeight = message.attachment?.height,
                    attachmentDurationMs = message.attachment?.durationMs,
                    attachmentHasPreview = message.attachment?.hasPreview ?: false,
                    attachmentName = message.attachment?.name,
                    // A location IS these three columns — it has no bytes
                    // to fall back on, so dropping them here does not
                    // degrade the bubble, it EMPTIES it. This is the path a
                    // message ARRIVES on, which is why the bug they caused
                    // was invisible to whoever sent the location and total
                    // for everybody else.
                    attachmentLatitude = message.attachment?.latitude,
                    attachmentLongitude = message.attachment?.longitude,
                    attachmentAccuracyM = message.attachment?.accuracyM,
                ),
            ),
        )
        chatDao.updateLastMessage(
            message.chatId,
            previewText(message.body, message.attachment),
            createdAt,
            message.senderId,
        )
        if (live) {
            val me = settings.state.first().myUserId
            val openChat = chatRepository.openChatId.value
            // Open is not enough: the chat must also be parked at its
            // newest message, because that is the only case where this
            // one lands on screen. ChatScreen refuses to scroll a reader
            // who is up the thread down to it, so suppressing the bump
            // for them would hide a message they never saw — and nothing
            // adds it back afterwards.
            val seen = message.chatId == openChat && chatRepository.openChatAtNewest.value
            if (!seen && message.senderId != me) {
                chatRepository.bumpUnread(message.chatId, message.id)
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

    /**
     * One edit catch-up loop: after_seq pages until a short page, each
     * message applied through the same seq-guarded path a live frame
     * takes. The cursor advances to every page's max seq, whether or not
     * we held the messages it named.
     */
    suspend fun catchUpEdits(chatId: Long, afterSeq: Long) {
        var cursor = afterSeq
        while (true) {
            val page = chatApi.getEdits(chatId, cursor, EDIT_PAGE).okOrNull()?.messages ?: return
            page.forEach { applyEdit(it) }
            val pageMax = page.mapNotNull { it.editSeq }.maxOrNull()
            if (pageMax != null) {
                chatDao.advanceMaxEditSeq(chatId, pageMax)
                cursor = pageMax
            }
            if (page.size < EDIT_PAGE) return
        }
    }

    /** PATCH the body. Author-only server-side; returns false on refusal. */
    suspend fun edit(chatId: Long, messageId: Long, body: String): Boolean {
        val trimmed = body.trim()
        if (trimmed.isEmpty()) return false
        return when (val result = chatApi.editMessage(chatId, messageId, trimmed)) {
            is ApiResult.Ok -> {
                applyEdit(result.value.message)
                true
            }
            else -> false
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
    /**
     * Re-fetch locations this device stored without their coordinates.
     *
     * **Catch-up only ever ADDS.** `after_id` asks for messages newer than
     * the newest one held, so a message already in the cache is never read
     * again — which means a row written by a build that dropped the
     * coordinates stays broken FOREVER on a device that has otherwise been
     * fixed, as a bubble with nothing in it.
     *
     * `before_id = serverId + 1, limit = 1` asks for exactly that one
     * message through an endpoint that already exists, and `applyServer`
     * puts it back through the same path a live delivery takes. Bounded, so
     * a cache full of them cannot turn a reconnect into a storm.
     */
    suspend fun repairLocationsMissingCoordinates() {
        val broken = messageDao.locationsMissingCoordinates(REPAIR_BATCH)
        if (broken.isEmpty()) return
        for (row in broken) {
            val serverId = row.serverId ?: continue
            val page = chatApi.messages(row.chatId, beforeId = serverId + 1, limit = 1)
            val dto = (page as? ApiResult.Ok)?.value?.messages?.firstOrNull { it.id == serverId }
                ?: continue
            applyServerMessage(dto, live = false)
        }
    }

    suspend fun loadOlder(chatId: Long): Boolean {
        val oldest = messageDao.oldestServerId(chatId)
        val result = chatApi.messages(chatId, beforeId = oldest, limit = HISTORY_PAGE)
        val page = result.okOrNull()?.messages ?: return false
        page.forEach { applyServerMessage(it, live = false) }
        return page.size < HISTORY_PAGE
    }

    companion object {
        /**
         * What the chat list shows on its second line.
         *
         * A photo is normally sent with no caption, and an empty string is
         * not null — so the row rendered blank rather than falling back.
         * Mirrors iOS's ChatSyncCoordinator.preview.
         */
        fun previewText(body: String, attachment: AttachmentDto?): String {
            if (body.isNotEmpty()) return body
            if (attachment == null) return body
            return when {
                attachment.isVideo -> "Video"
                attachment.isAudio -> attachment.name?.takeIf { it.isNotEmpty() } ?: "Audio"
                attachment.isLocation ->
                    attachment.name?.takeIf { it.isNotEmpty() } ?: "Location"
                attachment.isFile -> attachment.name?.takeIf { it.isNotEmpty() } ?: "File"
                else -> "Photo"
            }
        }

        /** Most broken locations repaired per resync — see the note above. */
        private const val REPAIR_BATCH = 25

        private const val ACK_TIMEOUT_MS = 15_000L
        private const val HISTORY_PAGE = 50
        private const val EDIT_PAGE = 200
        private const val REACTION_PAGE = 200
    }
}
