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

    @Query("DELETE FROM chats")
    suspend fun deleteAll()
}
