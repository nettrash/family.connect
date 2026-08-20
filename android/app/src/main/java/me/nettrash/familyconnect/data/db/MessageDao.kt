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
