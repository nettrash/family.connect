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

import androidx.room.ColumnInfo
import androidx.room.Entity
import androidx.room.Index
import androidx.room.PrimaryKey
import androidx.room.TypeConverter
import me.nettrash.familyconnect.data.net.dto.AttachmentDto

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
    /**
     * Reaction catch-up cursor: the highest reaction_seq this client has
     * APPLIED (or deliberately skipped) for this chat — advanced by
     * catch-up pages and live frames, never derived from held messages.
     * defaultValue keeps a fresh install and a 1→2 migrated schema
     * byte-identical under Room's validation.
     */
    @ColumnInfo(defaultValue = "0") val maxReactionSeq: Long = 0,
    /** The edit resync cursor — the edit twin of [maxReactionSeq]. */
    @ColumnInfo(defaultValue = "0") val maxEditSeq: Long = 0,
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
    /**
     * The message's reactions, stored as the wire-shape JSON array
     * (see ReactionsCodec). Null = never reacted, "[]" = cleared —
     * mirroring the protocol's absent-vs-empty distinction.
     */
    val reactionsJson: String? = null,
    /**
     * The reaction_seq stamped on the state in [reactionsJson]. Guards
     * out-of-order applies: a state only lands when its seq is greater
     * (see MessageDao.applyReactionState). 0 = no state ever applied.
     */
    @ColumnInfo(defaultValue = "0") val reactionSeq: Long = 0,
    /**
     * The quoted message, when this one is a reply. Three flat columns
     * rather than a relation: the quote is a SNAPSHOT the server recomputes
     * on every read (docs/protocol.md, "Replies"), and the quoted row may
     * not be in this device's cache at all.
     */
    val replyToMessageId: Long? = null,
    val replySenderId: Long? = null,
    val replyExcerpt: String? = null,
    /**
     * Set once the body has been edited. [editSeq] is the apply guard: a
     * stored body is overwritten only by a body at least as new, or a
     * history page fetched before an edit would restore the old text
     * (docs/protocol.md, "Editing"). 0 = never edited.
     */
    @ColumnInfo(defaultValue = "0") val editSeq: Long = 0,
    val editedAt: Long? = null,
    /**
     * The message's photo or video, flattened into columns rather than
     * kept as a relation: an attachment belongs to exactly one message,
     * is written once with it, and is read on every row of the thread —
     * a join would buy nothing. `attachmentId == null` means no media,
     * which is what [attachment] keys on.
     */
    val attachmentId: Long? = null,
    val attachmentKind: String? = null,
    val attachmentMime: String? = null,
    @ColumnInfo(defaultValue = "0") val attachmentSize: Long = 0,
    val attachmentWidth: Int? = null,
    val attachmentHeight: Int? = null,
    val attachmentDurationMs: Int? = null,
    @ColumnInfo(defaultValue = "0") val attachmentHasPreview: Boolean = false,
    /** Files only: the name is the whole thing a row shows. */
    val attachmentName: String? = null,
) {
    /** The wire shape back out of the flat columns, or null for no media. */
    val attachment: AttachmentDto?
        get() = attachmentId?.let { id ->
            AttachmentDto(
                id = id,
                kind = attachmentKind.orEmpty(),
                mime = attachmentMime.orEmpty(),
                size = attachmentSize,
                width = attachmentWidth,
                height = attachmentHeight,
                durationMs = attachmentDurationMs,
                hasPreview = attachmentHasPreview,
                name = attachmentName,
            )
        }
}

/**
 * One sticker note on the family board.
 *
 * Tombstones are NOT stored: the server keeps one so its change feed can
 * say "gone", but a client that has been told simply deletes its row —
 * there is nothing left to remember, and a stored tombstone would only have
 * to be filtered out of every read.
 *
 * [boardSeq] is the apply guard, the same shape as [MessageEntity.reactionSeq]:
 * a note is written only when the incoming seq is greater than the one
 * held, so an out-of-order frame cannot undo a newer move.
 */
@Entity(tableName = "notes")
data class NoteEntity(
    @PrimaryKey val id: Long,
    val authorId: Long,
    val text: String,
    /** One of the protocol's six names; kept as text so an unknown one still renders. */
    val color: String,
    /** Fractions of the board, 0..1 from the top-left. */
    val x: Double,
    val y: Double,
    val createdAt: Long,
    val updatedAt: Long,
    val boardSeq: Long,
)

@Entity(tableName = "members")
data class MemberEntity(
    @PrimaryKey val userId: Long,
    val username: String,
    val displayName: String,
    /** "owner" | "member". */
    val role: String,
    /**
     * Profile-picture version from the roster; 0 = no picture. Cached
     * here so a chat row can name its avatar's cache key without waiting
     * on the network. defaultValue matches MIGRATION_2_3 so a migrated
     * and a fresh schema validate identically.
     */
    @ColumnInfo(defaultValue = "0") val avatarVersion: Long = 0,
    /**
     * A member who left (or was removed) is KEPT, flagged rather than
     * deleted.
     *
     * Their old messages still need a display name and a face, and the
     * protocol retains history across leave/rejoin — so dropping the row
     * orphaned every message they ever sent, and their bubbles fell back
     * to "Member 11" with initials. iOS has always done it this way; this
     * is Android catching up.
     *
     * Left members are filtered out of the pickers and the admin list, not
     * out of name resolution.
     */
    @ColumnInfo(defaultValue = "0") val hasLeft: Boolean = false,
)
