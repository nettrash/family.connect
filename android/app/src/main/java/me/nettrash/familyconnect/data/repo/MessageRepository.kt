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
import me.nettrash.familyconnect.data.db.AnchorRow
import me.nettrash.familyconnect.data.db.ChatDao
import me.nettrash.familyconnect.data.db.MessageDao
import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.data.db.MessageStatus
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.AttachmentApi
import me.nettrash.familyconnect.data.net.ChatApi
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.net.dto.AttachmentsCodec
import me.nettrash.familyconnect.data.net.dto.CallDto
import me.nettrash.familyconnect.data.net.dto.MessageDto
import me.nettrash.familyconnect.data.net.dto.MessagePollStateDto
import me.nettrash.familyconnect.data.net.dto.NewPollDto
import me.nettrash.familyconnect.data.net.dto.PollCodec
import me.nettrash.familyconnect.data.net.dto.PollDto
import me.nettrash.familyconnect.data.net.dto.PollOptionDto
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
                    is ServerFrame.Read -> onRead(frame)
                    is ServerFrame.Reaction -> onReaction(frame)
                    is ServerFrame.Poll -> onPoll(frame)
                    is ServerFrame.Error -> onSendError(frame)
                    else -> Unit
                }
            }
        }
    }

    fun observeMessages(chatId: Long, limit: Int): Flow<List<MessageEntity>> =
        messageDao.observeMessages(chatId, limit)

    /**
     * (serverId, senderId) for the newest [limit] rows of a chat, in
     * [observeMessages]'s order — what the opening anchor reasons over.
     * See MessageDao.anchorRows for why it is its own narrow query.
     */
    suspend fun anchorRows(chatId: Long, limit: Int): List<AnchorRow> =
        messageDao.anchorRows(chatId, limit)

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
        dispatch(clientMsgId, chatId, trimmed, replyTo?.messageId, attachmentIds = null)
    }

    /**
     * Start a poll: an ordinary message whose BODY is the question, with
     * the options riding beside it (docs/protocol.md, "Polls").
     *
     * Optimistic like [send] and unlike [sendMedia], because there is
     * nothing to upload first — and the optimistic row carries a poll of
     * its own so the bubble draws as a poll immediately rather than as a
     * bare question that turns into one. That local copy uses NEGATIVE
     * option ids (the server's are positive) and `pollSeq = 0`: nothing
     * may be voted on before the message has a server id anyway, and a
     * zero seq is what lets the ack's authoritative poll — real ids and
     * a real seq — pass the guard rather than be dropped as stale.
     *
     * The question is the body, so the chat-list preview, the push and a
     * reply excerpt all need no new case.
     */
    suspend fun sendPoll(
        chatId: Long,
        question: String,
        options: List<String>,
        replyTo: ReplyToDto? = null,
    ) {
        val trimmedQuestion = question.trim()
        val trimmedOptions = options.map { it.trim() }.filter { it.isNotEmpty() }
        // A poll's body may NOT be empty, unlike a message carrying an
        // attachment: `message_empty` applies to a poll with no question.
        if (trimmedQuestion.isEmpty() || trimmedOptions.size < MIN_POLL_OPTIONS) return
        val me = settings.state.first().myUserId ?: return
        val clientMsgId = UUID.randomUUID().toString()
        val now = clock.now()
        val optimistic = PollDto(
            pollSeq = 0L,
            closed = false,
            options = trimmedOptions.mapIndexed { index, text ->
                PollOptionDto(id = -(index + 1).toLong(), text = text, votes = emptyList())
            },
        )
        messageDao.insert(
            MessageEntity(
                clientMsgId = clientMsgId,
                serverId = null,
                chatId = chatId,
                senderId = me,
                body = trimmedQuestion,
                createdAt = now,
                status = MessageStatus.SENDING,
                pollJson = PollCodec.encode(optimistic),
                replyToMessageId = replyTo?.messageId,
                replySenderId = replyTo?.senderId,
                replyExcerpt = replyTo?.excerpt,
            ),
        )
        chatDao.updateLastMessage(chatId, trimmedQuestion, now, me)
        dispatch(
            clientMsgId,
            chatId,
            trimmedQuestion,
            replyTo?.messageId,
            attachmentIds = null,
            poll = NewPollDto(trimmedOptions),
        )
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
                // The JSON column too, like every other write site: a
                // location is a one-element set (protocol: always alone).
                attachmentsJson = AttachmentsCodec.encode(listOf(attachment)),
                replyToMessageId = replyTo?.messageId,
                replySenderId = replyTo?.senderId,
                replyExcerpt = replyTo?.excerpt,
            ),
        )
        chatDao.updateLastMessage(chatId, previewText(body, listOf(attachment)), now, me)
        dispatch(clientMsgId, chatId, body, replyTo?.messageId, listOf(attachment.id))
        return true
    }

    suspend fun sendMedia(
        prepared: List<MediaPrep.Prepared>,
        caption: String,
        chatId: Long,
        replyTo: ReplyToDto? = null,
        /** Told before each item's upload starts: (1-based index, total). */
        onProgress: (index: Int, total: Int) -> Unit = { _, _ -> },
    ): Boolean {
        if (prepared.isEmpty()) return false
        val me = settings.state.first().myUserId ?: return false
        // Each item's bytes go up in the SENDER'S order, because that
        // order is what `attachment_ids` preserves and what every read
        // gives back (protocol.md, "Photos, videos, audio, files and
        // locations"). A mid-way failure returns false with earlier
        // uploads already landed: nothing has claimed them, so the
        // server's 24-hour sweep of unclaimed attachments is what
        // cleans them up — re-uploading on retry is the cheaper
        // mistake, not a leak.
        //
        // Each item's prepared FILE is deleted only once its own upload
        // has landed (mirroring iOS ChatSyncCoordinator.sendMedia): on a
        // mid-way failure the files of the failed item and everything
        // after it are LEFT ON DISK, which is what lets the composer
        // re-stage exactly the unsent tail for a one-tap retry.
        val uploadedAttachments = ArrayList<AttachmentDto>(prepared.size)
        prepared.forEachIndexed { index, item ->
            onProgress(index + 1, prepared.size)
            val uploaded = attachmentApi.upload(
                file = item.file,
                mime = item.mime,
                kind = item.kind,
                width = item.width,
                height = item.height,
                durationMs = item.durationMs,
                name = item.name,
            )
            val attachment = (uploaded as? ApiResult.Ok)?.value?.attachment ?: return false

            // Per-item and best-effort, exactly as it was for one: a
            // failed preview costs a thumbnail, never the send.
            var hasPreview = false
            item.previewJpeg?.let { jpeg ->
                hasPreview = attachmentApi.uploadPreview(attachment.id, jpeg) is ApiResult.Ok
            }
            uploadedAttachments += attachment.copy(hasPreview = hasPreview)
            // This item's bytes are on the server; its prepared file is
            // consumed. Items after this one keep theirs until their own
            // upload lands, so a mid-way failure leaves them restorable.
            item.file.delete()
        }

        val first = uploadedAttachments.first()
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
                // The flat columns mirror the FIRST element; the full
                // set rides in attachmentsJson.
                attachmentId = first.id,
                attachmentKind = first.kind,
                attachmentMime = first.mime,
                attachmentSize = first.size,
                attachmentWidth = first.width,
                attachmentHeight = first.height,
                attachmentDurationMs = first.durationMs,
                attachmentHasPreview = first.hasPreview,
                attachmentName = first.name,
                attachmentLatitude = first.latitude,
                attachmentLongitude = first.longitude,
                attachmentAccuracyM = first.accuracyM,
                attachmentsJson = AttachmentsCodec.encode(uploadedAttachments),
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
        chatDao.updateLastMessage(chatId, previewText(body, uploadedAttachments), now, me)
        dispatch(
            clientMsgId,
            chatId,
            body,
            replyTo?.messageId,
            uploadedAttachments.map { it.id },
        )
        return true
    }

    /**
     * What the chat list shows on its second line.
     *
     * A photo is normally sent with no caption, and an empty string is not
     * null — so the row rendered blank rather than falling back. Mirrors
     * iOS's ChatSyncCoordinator.preview.
     */
    fun previewText(
        body: String,
        attachments: List<AttachmentDto>,
        call: CallDto? = null,
    ): String = Companion.previewText(body, attachments, call)

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
        dispatch(
            clientMsgId,
            row.chatId,
            row.body,
            row.replyToMessageId,
            row.attachmentIds,
            pendingPollOf(row),
        )
    }

    /**
     * The options a not-yet-acked poll must be re-sent with.
     *
     * Read back off the row rather than held in memory, so a retry after
     * the process died still carries them — without this a re-sent poll
     * would land as a plain question with no options at all, and the
     * server would have no way to know one was meant.
     */
    private fun pendingPollOf(row: MessageEntity): NewPollDto? {
        if (row.serverId != null) return null
        val options = PollCodec.decode(row.pollJson)?.options ?: return null
        return options.takeIf { it.isNotEmpty() }?.let { NewPollDto(it.map(PollOptionDto::text)) }
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
                dispatch(
                    row.clientMsgId,
                    row.chatId,
                    row.body,
                    row.replyToMessageId,
                    row.attachmentIds,
                    pendingPollOf(row),
                )
            }
        }
    }

    private fun dispatch(
        clientMsgId: String,
        chatId: Long,
        body: String,
        replyToMessageId: Long?,
        attachmentIds: List<Long>?,
        poll: NewPollDto? = null,
    ) {
        val overSocket = socket.state.value == SocketState.Open &&
            socket.trySend(
                ClientFrame.Send(chatId, clientMsgId, body, replyToMessageId, attachmentIds, poll),
            )
        if (overSocket) {
            pendingAcks[clientMsgId] = scope.launch {
                delay(ACK_TIMEOUT_MS)
                pendingAcks.remove(clientMsgId)
                // No ack in time — the frame may or may not have landed.
                // REST with the same client_msg_id is safe either way.
                restFallback(clientMsgId, chatId, body, replyToMessageId, attachmentIds, poll)
            }
        } else {
            scope.launch {
                restFallback(clientMsgId, chatId, body, replyToMessageId, attachmentIds, poll)
            }
        }
    }

    private suspend fun restFallback(
        clientMsgId: String,
        chatId: Long,
        body: String,
        replyToMessageId: Long?,
        attachmentIds: List<Long>?,
        poll: NewPollDto? = null,
    ) {
        val row = messageDao.findByClientMsgId(clientMsgId) ?: return
        if (row.serverId != null) return // ack won the race
        val result =
            chatApi.postMessage(chatId, clientMsgId, body, replyToMessageId, attachmentIds, poll)
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
        // Read through the plural-first rule: prefer `attachments`, fall
        // back to the legacy `attachment`. Per the protocol the two are
        // never present without each other, so an "absent" set here means
        // a message that genuinely carries none — never a wipe of stored
        // state by an older wire shape.
        val acked = message.resolvedAttachments
        val ackedFirst = acked.firstOrNull()
        messageDao.setAttachment(
            clientMsgId,
            ackedFirst?.id,
            ackedFirst?.kind,
            ackedFirst?.mime,
            ackedFirst?.size ?: 0L,
            ackedFirst?.width,
            ackedFirst?.height,
            ackedFirst?.durationMs,
            ackedFirst?.hasPreview ?: false,
            ackedFirst?.name,
            ackedFirst?.latitude,
            ackedFirst?.longitude,
            ackedFirst?.accuracyM,
            acked.takeIf { it.isNotEmpty() }?.let(AttachmentsCodec::encode),
        )
        // The server's poll — real option ids and a real seq — replacing
        // the local copy the optimistic row drew with. Guarded like every
        // other apply, and a no-op for a message that is not a poll.
        applyEmbeddedPoll(message)
        chatDao.updateLastMessage(
            message.chatId,
            previewText(message.body, acked, message.call),
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
            applyEmbeddedPoll(message)
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
        // The plural-first read, on the path a message ARRIVES on. This is
        // one of the write sites the "third-time rule" names: history
        // pages and live frames both land here, and insertIgnore never
        // overwrites, so a re-delivery without the field cannot wipe what
        // an earlier delivery stored.
        val arrived = message.resolvedAttachments
        val arrivedFirst = arrived.firstOrNull()
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
                    attachmentId = arrivedFirst?.id,
                    attachmentKind = arrivedFirst?.kind,
                    attachmentMime = arrivedFirst?.mime,
                    attachmentSize = arrivedFirst?.size ?: 0L,
                    attachmentWidth = arrivedFirst?.width,
                    attachmentHeight = arrivedFirst?.height,
                    attachmentDurationMs = arrivedFirst?.durationMs,
                    attachmentHasPreview = arrivedFirst?.hasPreview ?: false,
                    attachmentName = arrivedFirst?.name,
                    // A location IS these three columns — it has no bytes
                    // to fall back on, so dropping them here does not
                    // degrade the bubble, it EMPTIES it. This is the path a
                    // message ARRIVES on, which is why the bug they caused
                    // was invisible to whoever sent the location and total
                    // for everybody else.
                    attachmentLatitude = arrivedFirst?.latitude,
                    attachmentLongitude = arrivedFirst?.longitude,
                    attachmentAccuracyM = arrivedFirst?.accuracyM,
                    attachmentsJson = arrived.takeIf { it.isNotEmpty() }
                        ?.let(AttachmentsCodec::encode),
                    // The THIRD write site a new field on a message needs,
                    // after the send path and the frame-apply path below.
                    // Missing it drops every poll this device sees for the
                    // first time in a history page — silently, since the
                    // bubble still draws the question.
                    pollJson = message.poll?.let(PollCodec::encode),
                    pollSeq = message.poll?.pollSeq ?: 0L,
                    // A call record arrives ONLY on this path: the server
                    // writes it, nobody sends one, so there is no optimistic
                    // row to fold into. insertIgnore never overwrites, so a
                    // history page re-delivering the message without the
                    // object cannot wipe it either.
                    callOutcome = message.call?.outcome,
                    callDurationSecs = message.call?.durationSecs,
                    callVideo = message.call?.video == true,
                ),
            ),
        )
        chatDao.updateLastMessage(
            message.chatId,
            previewText(message.body, arrived, message.call),
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

    /**
     * A `read` frame, which is two different facts wearing one shape.
     *
     * From SOMEBODY ELSE it is a receipt: peerLastReadId drives the ✓✓
     * glyph, which only direct chats show — and there the only other
     * reader is the peer.
     *
     * From THIS user it is the same person reading on another of their
     * devices. The marker is theirs and not the device's
     * (docs/protocol.md, `GET /chats` → `last_read_message_id`), so this
     * device's badge is stale the moment the frame lands.
     *
     * That second branch does not fire against today's server:
     * `deliver_read` fans the frame out to `others(...)`, so a reader's
     * own connections are excluded and this frame is always somebody
     * else's (server/src/events.rs). It is written anyway, because the
     * marker's meaning is what makes it right — a client that treats its
     * own user's read as somebody else's receipt is wrong on its own
     * terms — and because the resync path that DOES fire today
     * (ChatRepository.refreshChats) is the same operation arriving a
     * reconnect later. The wire was not changed for it; see the report.
     */
    private suspend fun onRead(frame: ServerFrame.Read) {
        val chat = chatDao.getById(frame.chatId) ?: return
        if (frame.userId == settings.state.first().myUserId) {
            chatRepository.applyMyReadMarker(
                chatId = frame.chatId,
                lastReadMessageId = frame.lastReadMessageId,
                myUserId = frame.userId,
            )
            return
        }
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
                // WS frame that raced us is never overwritten. The ROW
                // only: an HTTP reply is not a live frame and not a
                // catch-up page, so it may not advance the chat-wide
                // cursor (protocol.md, "Best-effort delivery"). See
                // applyPollState for the failure that rule prevents; it
                // is the same one here, with reaction_seq in place of
                // poll_seq.
                messageDao.applyReactionState(
                    serverId = state.messageId,
                    json = ReactionsCodec.encode(state.reactions),
                    seq = state.reactionSeq,
                )
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

    // -- Polls --------------------------------------------------------------------

    /**
     * Live `poll` frame: the poll's FULL current state, applied under the
     * seq guard (an unknown message matches zero rows — dropped
     * silently, and history paging re-delivers the state embedded on the
     * Message). The chat cursor advances REGARDLESS: frames arrive in
     * order on a live socket, so this seq is proof everything below it
     * was delivered, and a cursor that stayed put would make the next
     * resync re-read a page it already has.
     */
    private suspend fun onPoll(frame: ServerFrame.Poll) {
        messageDao.applyPollState(
            serverId = frame.messageId,
            json = PollCodec.encode(frame.poll),
            seq = frame.poll.pollSeq,
        )
        chatDao.advanceMaxPollSeq(frame.chatId, frame.poll.pollSeq)
    }

    /**
     * A poll riding on a fetched/re-delivered Message.
     *
     * Two rules, both load-bearing. It does NOT advance the chat cursor:
     * a history page proves nothing about OTHER polls' lower seqs, and
     * only catch-up pages and live frames may move it. And an ABSENT
     * poll returns early rather than clearing what is stored: a poll
     * dies only with its message, so silence is silence — a Message
     * fetched by a path that omits it must never wipe a poll this device
     * already holds.
     */
    private suspend fun applyEmbeddedPoll(message: MessageDto) {
        val poll = message.poll ?: return
        messageDao.applyPollState(
            serverId = message.id,
            json = PollCodec.encode(poll),
            seq = poll.pollSeq,
        )
    }

    /**
     * A tap on an option: the one I already hold → retract, else set.
     *
     * The protocol's vote is an idempotent state-set rather than a
     * toggle, and says outright that whether tapping your current choice
     * means "keep it" or "clear it" is each client's decision — this one
     * clears, which is the only way to un-vote from the bubble.
     *
     * Optimistic rewrite of pollJson first and NEVER of pollSeq (the
     * authoritative state must still pass the guard), then REST; failure
     * reverts. Same shape as [toggleReaction], including the
     * compare-and-set on the seq read a moment ago, so a frame that
     * lands mid-vote is never clobbered by a stale local write.
     */
    suspend fun toggleVote(chatId: Long, messageServerId: Long, optionId: Long) {
        val me = settings.state.first().myUserId ?: return
        val row = messageDao.findByServerId(messageServerId) ?: return
        val current = PollCodec.decode(row.pollJson) ?: return
        // A closed poll refuses votes server-side (`poll_closed`), so
        // there is nothing to be optimistic about.
        if (current.closed) return
        if (current.options.none { it.id == optionId }) return
        val retracting = current.optionHeldBy(me)?.id == optionId
        val optimistic = current.copy(
            options = current.options.map { option ->
                val without = option.votes.filterNot { it == me }
                when {
                    retracting || option.id != optionId -> option.copy(votes = without)
                    // Appended, not inserted: the server orders votes by
                    // when they were cast, and mine has just been.
                    else -> option.copy(votes = without + me)
                }
            },
        )
        messageDao.setPollJson(
            messageServerId,
            PollCodec.encode(optimistic),
            expectedSeq = row.pollSeq,
        )

        val result = if (retracting) {
            chatApi.deleteVote(chatId, messageServerId)
        } else {
            chatApi.putVote(chatId, messageServerId, optionId)
        }
        when (result) {
            is ApiResult.Ok -> applyPollState(result.value)
            // Wholesale rollback, no retry — mirrors toggleReaction: the
            // bars are visible state, and a vote nobody took must not
            // stay on screen.
            else -> messageDao.setPollJson(
                messageServerId,
                row.pollJson,
                expectedSeq = row.pollSeq,
            )
        }
    }

    /**
     * Close a poll. The author's, and one-way — the family owner does not
     * outrank them here, exactly as with editing. Returns false when the
     * server refuses (403 `not_message_author`), which the caller says
     * rather than pretending it worked.
     */
    suspend fun closePoll(chatId: Long, messageServerId: Long): Boolean =
        when (val result = chatApi.closePoll(chatId, messageServerId)) {
            is ApiResult.Ok -> {
                applyPollState(result.value)
                true
            }
            else -> false
        }

    /**
     * The authoritative state from a vote / retract / close response,
     * through the guarded path so a newer frame that raced us is never
     * overwritten.
     *
     * The ROW moves and the CHAT CURSOR deliberately does not — the rule
     * protocol.md states for every route that is neither a live frame nor
     * a catch-up page ("Best-effort delivery"), and the one the embedded
     * poll above already followed. A cursor is a chat-wide watermark and
     * one poll's seq is no evidence about another's: REST answers while
     * the socket is down, so a vote answered with seq 100 would push the
     * cursor past somebody else's seq 99 whose frame was never delivered,
     * and the next resync — comparing max_poll_seq against a cursor
     * already at 100 — would ask for nothing. That state would then be
     * lost until the poll holding it next changed. One redundant catch-up
     * page is the cheaper mistake. iOS spells the same rule in
     * ChatSyncCoordinator.vote.
     */
    private suspend fun applyPollState(state: MessagePollStateDto) {
        messageDao.applyPollState(
            serverId = state.messageId,
            json = PollCodec.encode(state.poll),
            seq = state.poll.pollSeq,
        )
    }

    /**
     * Poll catch-up (resync step 3d): pages strictly after [afterSeq]
     * until a short page, byte for byte the shape the edit and reaction
     * loops have. Each state applies under the seq guard; the stored
     * cursor advances to every page's max EVEN when the message it names
     * is not held locally — dropped states come back embedded on Message
     * objects when history pages there, and a cursor that refused to move
     * would re-read the same page on every reconnect for ever.
     */
    suspend fun catchUpPolls(chatId: Long, afterSeq: Long) {
        var cursor = afterSeq
        while (true) {
            val page = chatApi.getPolls(chatId, cursor, POLL_PAGE).okOrNull()?.polls ?: return
            page.forEach { state ->
                messageDao.applyPollState(
                    serverId = state.messageId,
                    json = PollCodec.encode(state.poll),
                    seq = state.poll.pollSeq,
                )
            }
            val pageMax = page.maxOfOrNull { it.poll.pollSeq }
            if (pageMax != null) {
                chatDao.advanceMaxPollSeq(chatId, pageMax)
                cursor = pageMax
            }
            if (page.size < POLL_PAGE) return
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
     * oldest-first. Returns what the page held, or null on failure (caller
     * stops looping — the next reconnect resumes from the same cursor).
     *
     * [CatchUpPage.maxServerId] is what the loop advances its cursor with.
     * The alternative — re-reading `max(serverId)` from the store between
     * pages — is what protocol.md forbids: a live `message` frame landing
     * mid-loop writes a much higher id, the next request asks for
     * everything after THAT, and the whole backlog in between is skipped
     * for good (`after_id` never looks back, and `loadOlder` only pages
     * older than the OLDEST row held). iOS's runCatchUp carries the
     * cursor in a local for the same reason.
     */
    suspend fun catchUp(chatId: Long, afterId: Long, limit: Int): CatchUpPage? {
        val result = chatApi.messages(chatId, afterId = afterId, limit = limit)
        val page = result.okOrNull()?.messages ?: return null
        page.forEach { applyServerMessage(it, live = false) }
        return CatchUpPage(size = page.size, maxServerId = page.maxOfOrNull { it.id })
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
        fun previewText(
            body: String,
            attachments: List<AttachmentDto>,
            call: CallDto? = null,
        ): String {
            // A call record's body is the server's English placeholder,
            // and the preview says the same thing in the same words — the
            // one line this list has for it. Checked FIRST, because the
            // body is never empty on a record.
            if (call != null) {
                // The video wording mirrors the server's own placeholder
                // (protocol.md, "Video") — same English-in-the-DB
                // convention as the attachment summaries below.
                return when {
                    call.video && call.outcome == CallDto.MISSED -> "Missed video call"
                    call.video -> "Video call"
                    call.outcome == CallDto.MISSED -> "Missed voice call"
                    else -> "Voice call"
                }
            }
            if (body.isNotEmpty()) return body
            val attachment = attachments.firstOrNull() ?: return body
            if (attachments.size > 1) {
                // Several of one kind get a count — "3 Photos" — and a
                // mixed set the plain "N attachments": the same ENGLISH
                // literals the push summary uses, per this function's own
                // convention (protocol.md, "Push notifications"; names
                // give way to the count).
                val n = attachments.size
                val kinds = attachments.mapTo(HashSet()) { it.kind }
                return if (kinds.size == 1) {
                    when {
                        attachment.isVideo -> "$n Videos"
                        attachment.isAudio -> "$n Audio"
                        attachment.isFile -> "$n Files"
                        // A location is always alone (protocol), so a
                        // uniform multi-set here can only be photos — or
                        // a kind added later, which falls through.
                        attachment.kind == AttachmentDto.KIND_PHOTO -> "$n Photos"
                        else -> "$n attachments"
                    }
                } else {
                    "$n attachments"
                }
            }
            return when {
                attachment.isVideo -> "Video"
                attachment.isAudio -> attachment.name?.takeIf { it.isNotEmpty() } ?: "Audio"
                attachment.isLocation ->
                    attachment.name?.takeIf { it.isNotEmpty() } ?: "Location"
                attachment.isFile -> attachment.name?.takeIf { it.isNotEmpty() } ?: "File"
                else -> "Photo"
            }
        }

        /** The pre-plurality spelling, kept for single-attachment callers. */
        fun previewText(body: String, attachment: AttachmentDto?, call: CallDto? = null): String =
            previewText(body, attachment?.let(::listOf).orEmpty(), call)

        /** Most broken locations repaired per resync — see the note above. */
        private const val REPAIR_BATCH = 25

        private const val ACK_TIMEOUT_MS = 15_000L
        private const val HISTORY_PAGE = 50
        private const val EDIT_PAGE = 200
        private const val REACTION_PAGE = 200
        private const val POLL_PAGE = 200

        /**
         * Fewest options a poll may be created with, mirroring the
         * server's `Poll::MIN_OPTIONS`. One option is not a question.
         */
        const val MIN_POLL_OPTIONS = 2

        /** Most options a poll may be created with (protocol "Limits"). */
        const val MAX_POLL_OPTIONS = 10

        /** Longest one option's text may be (protocol "Limits"). */
        const val MAX_POLL_OPTION_CHARS = 100
    }
}

/**
 * One page of [MessageRepository.catchUp]: how many messages came back,
 * and the largest server id among them.
 *
 * The id is the half that matters. It is what the resync loop advances
 * its cursor with, so that a live message arriving while the loop runs
 * cannot jump the cursor past the backlog still being paged.
 */
data class CatchUpPage(
    val size: Int,
    /** Null on an empty page — the cursor then stays where it is. */
    val maxServerId: Long?,
)
