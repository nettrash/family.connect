/*
 * MessageDao.kt
 * Family Connect (Android)
 *
 * Message queries. Ordering contract for the chat screen (which renders
 * a reverseLayout LazyColumn, index 0 at the bottom):
 *
 *     (serverId IS NULL) DESC, serverId DESC, createdAt DESC
 *
 * i.e. pending sends first (= the newest side visually), then acked
 * messages by server id — the globally monotonic order the server
 * assigns, immune to clock skew between devices. createdAt only breaks
 * ties among pending rows.
 *
 * iOS counterpart: ios/FamilyConnect/Data/Db/MessageStore.swift
 */

package me.nettrash.familyconnect.data.db

import androidx.room.Dao
import androidx.room.Insert
import androidx.room.OnConflictStrategy
import androidx.room.Query
import kotlinx.coroutines.flow.Flow

@Dao
interface MessageDao {

    @Query(
        """
        SELECT * FROM messages WHERE chatId = :chatId
        ORDER BY (serverId IS NULL) DESC, serverId DESC, createdAt DESC
        LIMIT :limit
        """,
    )
    fun observeMessages(chatId: Long, limit: Int): Flow<List<MessageEntity>>

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun insert(message: MessageEntity)

    /**
     * Bulk insert for resync/paging. IGNORE covers both conflict paths:
     * a duplicate clientMsgId (PK) and a duplicate serverId (unique
     * index) — either means we already have the message.
     */
    @Insert(onConflict = OnConflictStrategy.IGNORE)
    suspend fun insertIgnore(messages: List<MessageEntity>)

    @Query("SELECT * FROM messages WHERE clientMsgId = :clientMsgId")
    suspend fun findByClientMsgId(clientMsgId: String): MessageEntity?

    /**
     * Locations stored WITHOUT their coordinates — rows a build that
     * dropped them on the inbound path left behind.
     *
     * A location has no bytes to fall back on, so such a row is a bubble
     * with nothing in it. Catch-up is `after_id`-only and can never see an
     * older row again, so these never heal on their own; see
     * `MessageRepository.repairLocationsMissingCoordinates`.
     */
    @Query(
        """
        SELECT * FROM messages
        WHERE attachmentKind = 'location'
          AND attachmentLatitude IS NULL
          AND serverId IS NOT NULL
        LIMIT :limit
        """,
    )
    suspend fun locationsMissingCoordinates(limit: Int): List<MessageEntity>

    @Query("SELECT * FROM messages WHERE serverId = :serverId")
    suspend fun findByServerId(serverId: Long): MessageEntity?

    @Query("SELECT EXISTS(SELECT 1 FROM messages WHERE serverId = :serverId)")
    suspend fun existsByServerId(serverId: Long): Boolean

    /**
     * The ack path: same row (PK untouched) gains its server id, the
     * authoritative server timestamp, and flips to SENT.
     */
    @Query(
        """
        UPDATE messages SET serverId = :serverId, createdAt = :createdAt, status = 'SENT'
        WHERE clientMsgId = :clientMsgId
        """,
    )
    suspend fun markAcked(clientMsgId: String, serverId: Long, createdAt: Long)

    /**
     * Overwrite the quote from the server's copy.
     *
     * The server RECOMPUTES the snippet on every read (protocol.md,
     * "Replies"), so its version is always the authority: it reflects the
     * quoted message as it stands now, and it is cut the way the server
     * cuts. A row that kept the excerpt its own device guessed at send
     * time would drift the moment the quoted message was edited.
     */
    @Query(
        """
        UPDATE messages
        SET replyToMessageId = :replyToMessageId,
            replySenderId = :replySenderId,
            replyExcerpt = :replyExcerpt,
            replyParentMessageId = :replyParentMessageId,
            replyParentSenderId = :replyParentSenderId,
            replyParentExcerpt = :replyParentExcerpt
        WHERE clientMsgId = :clientMsgId
        """,
    )
    suspend fun setReply(
        clientMsgId: String,
        replyToMessageId: Long?,
        replySenderId: Long?,
        replyExcerpt: String?,
        replyParentMessageId: Long?,
        replyParentSenderId: Long?,
        replyParentExcerpt: String?,
    )

    /**
     * Replace a row's attachment with the server's copy.
     *
     * An attachment is fixed at send time with one exception — has_preview
     * flips from false to true when the sender's preview upload lands —
     * so the ack, not the optimistic row, is the authority. Written as one
     * statement for the same reason the reply snapshot is.
     */
    @Query(
        """
        UPDATE messages
        SET attachmentId = :attachmentId,
            attachmentKind = :kind,
            attachmentMime = :mime,
            attachmentSize = :size,
            attachmentWidth = :width,
            attachmentHeight = :height,
            attachmentDurationMs = :durationMs,
            attachmentHasPreview = :hasPreview,
            attachmentName = :name,
            attachmentLatitude = :latitude,
            attachmentLongitude = :longitude,
            attachmentAccuracyM = :accuracyM
        WHERE clientMsgId = :clientMsgId
        """,
    )
    suspend fun setAttachment(
        clientMsgId: String,
        attachmentId: Long?,
        kind: String?,
        mime: String?,
        size: Long,
        width: Int?,
        height: Int?,
        durationMs: Int?,
        hasPreview: Boolean,
        name: String?,
        latitude: Double?,
        longitude: Double?,
        accuracyM: Int?,
    )

    /**
     * Overwrite the body ONLY when the incoming copy is at least as new.
     *
     * The guard the protocol calls load-bearing. Deliveries are not
     * ordered: a history page fetched BEFORE an edit can arrive after the
     * frame carrying it, and an unguarded write would quietly restore the
     * old text — on one device and not another. Expressed in SQL so the
     * check and the write are one statement and cannot race each other.
     */
    @Query(
        """
        UPDATE messages
        SET body = :body, editSeq = :editSeq, editedAt = :editedAt
        WHERE serverId = :serverId AND :editSeq >= editSeq
        """,
    )
    suspend fun applyEdit(serverId: Long, body: String, editSeq: Long, editedAt: Long?): Int

    /** Refresh the quote on every reply that points at this message. */
    @Query(
        """
        UPDATE messages
        SET replyExcerpt = :excerpt
        WHERE replyToMessageId = :quotedMessageId
        """,
    )
    suspend fun refreshQuotesOf(quotedMessageId: Long, excerpt: String)

    /**
     * The same, one level down: an edited message may also be the SECOND
     * level of somebody else's quote. Without this the outer excerpt goes
     * stale and shows text its author has already changed — exactly what
     * recomputing-on-read exists to prevent.
     */
    @Query(
        """
        UPDATE messages
        SET replyParentExcerpt = :excerpt
        WHERE replyParentMessageId = :quotedMessageId
        """,
    )
    suspend fun refreshParentQuotesOf(quotedMessageId: Long, excerpt: String)

    /**
     * Append one streamed fragment to a message's body.
     *
     * Done in SQL rather than read-modify-write in Kotlin: fragments arrive
     * faster than a round trip through the flow, and two of them racing
     * would lose text. Returns 0 when this device does not have the row,
     * which is not an error — the final body arrives as an edit regardless.
     */
    @Query("UPDATE messages SET body = body || :fragment WHERE serverId = :serverId")
    suspend fun appendToBody(serverId: Long, fragment: String): Int

    @Query("UPDATE messages SET status = :status WHERE clientMsgId = :clientMsgId")
    suspend fun setStatus(clientMsgId: String, status: MessageStatus)

    @Query("DELETE FROM messages WHERE clientMsgId = :clientMsgId")
    suspend fun deleteByClientMsgId(clientMsgId: String)

    /**
     * Every message in one chat, for a chat that is going.
     *
     * The only reason a chat is ever deleted is that the server no longer
     * has it — a peer deleted their account, which takes the direct chat
     * with them, both halves (docs/protocol.md, "Deleting an account").
     * Nothing else in this protocol can make a chat vanish, and leaving a
     * family explicitly does not: that history is retained and resurfaces
     * on rejoin.
     *
     * Paired with [ChatDao.deleteById] and never called on its own — a
     * chat row whose messages are gone is a chat that opens empty, and
     * messages whose chat row is gone are rows nothing will ever read
     * again. See ChatRepository.deleteDirectChat, which owns the order.
     */
    @Query("DELETE FROM messages WHERE chatId = :chatId")
    suspend fun deleteByChat(chatId: Long)

    /** Outbound rows awaiting an ack — re-sent on every reconnect. */
    @Query("SELECT * FROM messages WHERE status = 'SENDING' ORDER BY createdAt ASC")
    suspend fun pendingSending(): List<MessageEntity>

    /**
     * The one write path for server-authored reaction state (WS frame,
     * catch-up page, or reactions embedded on a fetched Message). The
     * `reactionSeq < :seq` guard makes the apply atomic AND idempotent:
     * a stale or re-delivered state simply matches zero rows. Returns
     * the number of rows updated (0 = stale or message not held).
     */
    @Query(
        """
        UPDATE messages SET reactionsJson = :json, reactionSeq = :seq
        WHERE serverId = :serverId AND reactionSeq < :seq
        """,
    )
    suspend fun applyReactionState(serverId: Long, json: String, seq: Long): Int

    /**
     * Optimistic local rewrite for the toggle path — deliberately does
     * NOT touch reactionSeq, so the authoritative response (or any WS
     * frame) still passes the seq guard afterwards. Also the revert.
     * Compare-and-set on the seq observed at read time: a WS frame that
     * lands mid-toggle bumps the seq, and the stale optimistic write (or
     * the revert after a failed call) then matches zero rows instead of
     * clobbering the newer state — which nothing would re-deliver.
     */
    @Query(
        """
        UPDATE messages SET reactionsJson = :json
        WHERE serverId = :serverId AND reactionSeq = :expectedSeq
        """,
    )
    suspend fun setReactionsJson(serverId: Long, json: String?, expectedSeq: Long)

    /**
     * The one write path for server-authored poll state (WS frame,
     * catch-up page, or a poll embedded on a fetched Message). The
     * `pollSeq < :seq` guard makes the apply atomic AND idempotent: a
     * stale or re-delivered state simply matches zero rows. Returns the
     * number of rows updated (0 = stale, or the message is not held).
     *
     * Note what it CANNOT express: taking a poll off a message. Nothing
     * on the wire ever does — a poll dies with its message — so an
     * absent poll on an incoming Message is silence, never an erasure.
     */
    @Query(
        """
        UPDATE messages SET pollJson = :json, pollSeq = :seq
        WHERE serverId = :serverId AND pollSeq < :seq
        """,
    )
    suspend fun applyPollState(serverId: Long, json: String, seq: Long): Int

    /**
     * Optimistic local rewrite for the voting path — deliberately does
     * NOT touch pollSeq, so the authoritative response (or any WS frame)
     * still passes the seq guard afterwards. Also the revert.
     * Compare-and-set on the seq observed at read time, exactly like
     * [setReactionsJson]: a frame that lands mid-vote bumps the seq, and
     * the stale optimistic write (or the revert after a failed call)
     * then matches zero rows instead of clobbering the newer state —
     * which nothing would re-deliver.
     */
    @Query(
        """
        UPDATE messages SET pollJson = :json
        WHERE serverId = :serverId AND pollSeq = :expectedSeq
        """,
    )
    suspend fun setPollJson(serverId: Long, json: String?, expectedSeq: Long)

    /** Resync cursor: message ids are globally monotonic (protocol). */
    @Query("SELECT MAX(serverId) FROM messages WHERE chatId = :chatId")
    suspend fun maxServerId(chatId: Long): Long?

    /** History-paging cursor. */
    @Query("SELECT MIN(serverId) FROM messages WHERE chatId = :chatId")
    suspend fun oldestServerId(chatId: Long): Long?

    /**
     * How many messages this device holds that are newer than a read
     * marker and were not sent by me — the local recount of one chat's
     * unread.
     *
     * The predicate is the server's own, word for word (push_payload.rs,
     * `build_message_unread_query`: newer than my read marker, not sent
     * by me), so the two can never disagree about what "unread" MEANS.
     * They can still disagree about the ANSWER, because this device only
     * counts what it holds: a phone that was offline while three messages
     * arrived recounts zero. That is why only the cross-device read frame
     * uses it — that frame arrives on a live socket, which is also what
     * has been delivering the messages — and why `GET /chats` stays
     * authoritative for everything else.
     */
    @Query(
        """
        SELECT COUNT(*) FROM messages
        WHERE chatId = :chatId AND serverId > :afterId AND senderId <> :myUserId
        """,
    )
    suspend fun countInboundAfter(chatId: Long, afterId: Long, myUserId: Long): Int

    /**
     * The two columns the opening anchor reasons over, newest-first.
     *
     * A NARROW projection rather than [observeMessages] because it reads
     * further back than the render window does: the window opens at
     * ChatViewModel.INITIAL_LIMIT rows and the anchor may sit behind
     * them, and inflating the window to look would drag every body,
     * attachment and poll JSON in the chat through the flow to answer a
     * question about two Longs.
     *
     * The ORDER BY is [observeMessages]'s, character for character, and
     * has to stay that way: the anchor is resolved as a POSITION in this
     * list and then found again by id in the rendered one, and two
     * orderings that disagree would put the divider above a different
     * message than the one the arithmetic chose.
     */
    @Query(
        """
        SELECT serverId, senderId FROM messages WHERE chatId = :chatId
        ORDER BY (serverId IS NULL) DESC, serverId DESC, createdAt DESC
        LIMIT :limit
        """,
    )
    suspend fun anchorRows(chatId: Long, limit: Int): List<AnchorRow>
}

/**
 * One row as the opening-anchor arithmetic sees it (see
 * ui/chat/OpenAnchor.kt). A query result, NOT an @Entity — nothing here
 * changes the schema.
 *
 * `serverId == null` is a locally-pending outbound row: it has no id the
 * server ever counted, which is exactly why the arithmetic skips it.
 */
data class AnchorRow(
    val serverId: Long?,
    val senderId: Long,
)
