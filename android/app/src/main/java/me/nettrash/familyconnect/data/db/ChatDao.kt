/*
 * ChatDao.kt
 * Family Connect (Android)
 *
 * Chat-list queries. Ordering: the family chat is pinned first, the rest
 * by most-recent activity with never-messaged chats last. (Spelled as
 * `(lastMessageAt IS NULL) ASC, lastMessageAt DESC` rather than
 * `NULLS LAST` because the SQLite bundled with minSdk-26 devices
 * predates that syntax.)
 *
 * iOS counterpart: ios/FamilyConnect/Data/Db/ChatStore.swift
 */

package me.nettrash.familyconnect.data.db

import androidx.room.Dao
import androidx.room.Query
import androidx.room.Upsert
import kotlinx.coroutines.flow.Flow

/**
 * One chat's unread count and nothing else — a query projection, not a
 * table.
 *
 * Narrow on purpose: the unread flow is collected for the whole life of
 * the process (UnreadNotifications) and must not carry chat titles,
 * previews and cursors past every typing indicator that touches the row.
 */
data class ChatUnread(val chatId: Long, val unreadCount: Int)

@Dao
interface ChatDao {

    @Query(
        """
        SELECT * FROM chats
        ORDER BY (kind = 'family') DESC, (lastMessageAt IS NULL) ASC, lastMessageAt DESC
        """,
    )
    fun observeChats(): Flow<List<ChatEntity>>

    @Upsert
    suspend fun upsertAll(chats: List<ChatEntity>)

    @Query("SELECT * FROM chats WHERE id = :id")
    suspend fun getById(id: Long): ChatEntity?

    @Query("SELECT * FROM chats WHERE id = :id")
    fun observeById(id: Long): Flow<ChatEntity?>

    @Query("SELECT id FROM chats")
    suspend fun allChatIds(): List<Long>

    /**
     * The direct chats this device holds.
     *
     * Scoped to `direct` because that is the only kind that can
     * DISAPPEAR: a deleted account takes its direct chats with it, both
     * halves, and its private assistant thread with them
     * (docs/protocol.md, "Deleting an account"). The family chat lives as
     * long as the family and MY assistant chat as long as my account, so
     * neither is ever missing from a `GET /chats` this client made — and
     * pruning on their absence would only ever be a bug destroying
     * history.
     */
    @Query("SELECT id FROM chats WHERE kind = 'direct'")
    suspend fun directChatIds(): List<Long>

    /**
     * The direct chat(s) with one peer. A list, not a single id: the
     * server is get-or-create so there should be exactly one, and if a
     * device ever ended up holding two, a deleted account takes both.
     */
    @Query("SELECT id FROM chats WHERE kind = 'direct' AND peerUserId = :userId")
    suspend fun directChatIdsWith(userId: Long): List<Long>

    /**
     * Drop one chat row, and with it everything keyed to a chat that
     * lives ON the row — the unread badge, both read markers and all
     * three catch-up cursors. Its messages go through
     * [MessageDao.deleteByChat]; see ChatRepository.deleteDirectChat,
     * which is the only caller of either.
     */
    @Query("DELETE FROM chats WHERE id = :chatId")
    suspend fun deleteById(chatId: Long)

    /**
     * Monotonic preview update — the guard keeps an out-of-order ack
     * (server timestamp older than a message that already landed) from
     * regressing the chat list preview.
     */
    @Query(
        """
        UPDATE chats SET lastMessageBody = :body, lastMessageAt = :at, lastMessageSenderId = :senderId
        WHERE id = :chatId AND (lastMessageAt IS NULL OR lastMessageAt <= :at)
        """,
    )
    suspend fun updateLastMessage(chatId: Long, body: String, at: Long, senderId: Long)

    @Query("UPDATE chats SET unreadCount = unreadCount + 1 WHERE id = :chatId")
    suspend fun bumpUnread(chatId: Long)

    @Query("UPDATE chats SET unreadCount = 0 WHERE id = :chatId")
    suspend fun clearUnread(chatId: Long)

    /**
     * Set a chat's count outright, for the one caller that has RECOUNTED
     * it rather than moved it — a read reported by another of this
     * person's devices leaves an arbitrary number behind, not zero
     * (ChatRepository.applyMyReadMarker).
     */
    @Query("UPDATE chats SET unreadCount = :count WHERE id = :chatId")
    suspend fun setUnreadCount(chatId: Long, count: Int)

    /**
     * Every chat's unread count, for the life of the process.
     *
     * The source the notification numbers and the clearing rule are
     * derived from (UnreadBadge, UnreadNotifications). Room re-runs it on
     * any write to `chats`, which is exactly the set of moments the
     * number can have changed.
     */
    @Query("SELECT id AS chatId, unreadCount FROM chats")
    fun observeUnread(): Flow<List<ChatUnread>>

    /** MAX() keeps both markers monotonic — mirrors the server's rule. */
    @Query(
        """
        UPDATE chats SET myLastReadId = MAX(COALESCE(myLastReadId, 0), :messageId)
        WHERE id = :chatId
        """,
    )
    suspend fun setMyLastRead(chatId: Long, messageId: Long)

    @Query(
        """
        UPDATE chats SET peerLastReadId = MAX(COALESCE(peerLastReadId, 0), :messageId)
        WHERE id = :chatId
        """,
    )
    suspend fun setPeerLastRead(chatId: Long, messageId: Long)

    /**
     * Advance the reaction catch-up cursor. Scalar MAX() keeps it
     * monotonic, so applying frames/pages in any order is safe.
     */
    @Query("UPDATE chats SET maxReactionSeq = MAX(maxReactionSeq, :seq) WHERE id = :chatId")
    suspend fun advanceMaxReactionSeq(chatId: Long, seq: Long)

    /** The stored reaction cursor (null when the chat row is absent). */
    @Query("SELECT maxReactionSeq FROM chats WHERE id = :chatId")
    suspend fun maxReactionSeq(chatId: Long): Long?

    /** The edit cursor's twin of the two above. */
    @Query("UPDATE chats SET maxEditSeq = MAX(maxEditSeq, :seq) WHERE id = :chatId")
    suspend fun advanceMaxEditSeq(chatId: Long, seq: Long)

    @Query("SELECT maxEditSeq FROM chats WHERE id = :chatId")
    suspend fun maxEditSeq(chatId: Long): Long?

    /** The poll cursor's twin of the four above. */
    @Query("UPDATE chats SET maxPollSeq = MAX(maxPollSeq, :seq) WHERE id = :chatId")
    suspend fun advanceMaxPollSeq(chatId: Long, seq: Long)

    @Query("SELECT maxPollSeq FROM chats WHERE id = :chatId")
    suspend fun maxPollSeq(chatId: Long): Long?

    @Query("DELETE FROM chats")
    suspend fun deleteAll()
}
