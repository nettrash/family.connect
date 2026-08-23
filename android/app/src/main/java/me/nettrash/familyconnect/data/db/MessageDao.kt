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
            attachmentName = :name
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

    /** Resync cursor: message ids are globally monotonic (protocol). */
    @Query("SELECT MAX(serverId) FROM messages WHERE chatId = :chatId")
    suspend fun maxServerId(chatId: Long): Long?

    /** History-paging cursor. */
    @Query("SELECT MIN(serverId) FROM messages WHERE chatId = :chatId")
    suspend fun oldestServerId(chatId: Long): Long?
}
