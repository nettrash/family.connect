/*
 * Entities.kt
 * Family Connect (Android)
 *
 * Room schema. The app renders exclusively from these tables — the
 * WebSocket and REST layers only ever *write* here, which is what makes
 * offline reads and the optimistic send pipeline trivial.
 *
 * MessageEntity is keyed by client_msg_id (a UUID we mint before the
 * server ever sees the message) so an optimistic row and its eventual
 * ack are the SAME row — no flicker, no re-keying in the LazyColumn.
 * `serverId` is nullable-unique: NULL while pending (SQLite permits any
 * number of NULLs in a unique index), set exactly once on ack, and the
 * uniqueness is what dedups resync/paging inserts.
 *
 * iOS counterpart: ios/FamilyConnect/Data/Db/Entities.swift (SwiftData
 * models there).
 */

package me.nettrash.familyconnect.data.db

import androidx.room.Entity
import androidx.room.Index
import androidx.room.PrimaryKey
import androidx.room.TypeConverter

/** Lifecycle of an outbound message; inbound rows are always SENT. */
enum class MessageStatus {
    SENDING,
    SENT,
    FAILED,
}

/** Stored as TEXT — readable in a db dump, stable across enum reorders. */
class Converters {
    @TypeConverter
    fun fromStatus(status: MessageStatus): String = status.name

    @TypeConverter
    fun toStatus(raw: String): MessageStatus = MessageStatus.valueOf(raw)
}

@Entity(tableName = "chats")
data class ChatEntity(
    @PrimaryKey val id: Long,
    /** "family" | "direct" — mirrors Chat.kind on the wire. */
    val kind: String,
    val peerUserId: Long?,
    val title: String,
    val unreadCount: Int,
    /** Highest message id *I* have reported read (local monotonic mirror). */
    val myLastReadId: Long?,
    /** Highest message id the direct-chat peer has read (drives ✓✓). */
    val peerLastReadId: Long?,
    // Denormalized preview for the chat list — avoids a JOIN per row.
    val lastMessageBody: String?,
    val lastMessageAt: Long?,
    val lastMessageSenderId: Long?,
)

@Entity(
    tableName = "messages",
    indices = [
        Index(value = ["serverId"], unique = true),
        Index(value = ["chatId", "serverId"]),
        Index(value = ["chatId", "status"]),
    ],
)
data class MessageEntity(
    @PrimaryKey val clientMsgId: String,
    /** Server-assigned id; null while the send is in flight. */
    val serverId: Long?,
    val chatId: Long,
    val senderId: Long,
    val body: String,
    /** Epoch millis. Local clock until acked; server clock (authoritative) after. */
    val createdAt: Long,
    val status: MessageStatus,
)

@Entity(tableName = "members")
data class MemberEntity(
    @PrimaryKey val userId: Long,
    val username: String,
    val displayName: String,
    /** "owner" | "member". */
    val role: String,
)
