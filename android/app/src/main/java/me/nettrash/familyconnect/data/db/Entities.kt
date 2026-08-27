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
import me.nettrash.familyconnect.data.net.dto.AttachmentsCodec
import me.nettrash.familyconnect.data.net.dto.CallDto
import me.nettrash.familyconnect.data.net.dto.BirthdayDto

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
    /**
     * "family" | "direct" | "ai" — mirrors Chat.kind on the wire. The
     * share target is the first place this app COMPARES against "ai"
     * (see [me.nettrash.familyconnect.ui.share.shareTargets]): the
     * assistant's private thread takes no shared files, so it is
     * filtered out of the chat picker there.
     */
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
    /**
     * The poll resync cursor — the fourth of the same shape.
     *
     * Advanced by catch-up pages and live `poll` frames only. A poll
     * riding on a fetched Message must NOT move it: a history page
     * proves nothing about OTHER polls' lower values, and a cursor that
     * jumped ahead of them would skip changes this device never saw.
     */
    @ColumnInfo(defaultValue = "0") val maxPollSeq: Long = 0,
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
     * The SECOND level: what the quoted message was itself answering.
     * Null means there is nothing behind the quote, which is the normal
     * case rather than a half-set row.
     */
    val replyParentMessageId: Long? = null,
    val replyParentSenderId: Long? = null,
    val replyParentExcerpt: String? = null,
    /**
     * Set once the body has been edited. [editSeq] is the apply guard: a
     * stored body is overwritten only by a body at least as new, or a
     * history page fetched before an edit would restore the old text
     * (docs/protocol.md, "Editing"). 0 = never edited.
     */
    @ColumnInfo(defaultValue = "0") val editSeq: Long = 0,
    val editedAt: Long? = null,
    /**
     * The message's FIRST attachment, flattened into columns rather than
     * kept as a relation: an attachment belongs to exactly one message,
     * is written once with it, and is read on every row of the thread —
     * a join would buy nothing. `attachmentId == null` means no media,
     * which is what the flat-column fallback keys on.
     *
     * Since v16 the full set lives in [attachmentsJson]; these twelve
     * columns are kept for rows written before plurality, and are still
     * mirrored from the first element on every write so a downgrade
     * degrades to "the first attachment" rather than to nothing.
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
    /**
     * Locations only, and always both together (docs/protocol.md,
     * "Locations"). Stored on the row rather than fetched, because a
     * location has no bytes at all — these three columns ARE the
     * attachment, so a cached message draws its pin offline.
     */
    val attachmentLatitude: Double? = null,
    val attachmentLongitude: Double? = null,
    val attachmentAccuracyM: Int? = null,
    /**
     * The poll on this message, stored as the wire-shape JSON object
     * (see PollCodec). Null = not a poll, which is the only thing
     * absence ever means here: a poll dies with its message, so nothing
     * on the wire can take one off a message that has one.
     *
     * Verbatim rather than flattened into columns, exactly like
     * [reactionsJson] and for the same reasons: it is a nested list of
     * lists, it is replaced whole (never patched), and the shape is the
     * server's.
     */
    val pollJson: String? = null,
    /**
     * The poll_seq stamped on the state in [pollJson]. Guards
     * out-of-order applies: a poll only lands when its seq is greater
     * (see MessageDao.applyPollState). 0 = no server state applied yet,
     * which is also what an optimistic just-sent poll carries so the
     * ack's authoritative copy still passes the guard.
     */
    @ColumnInfo(defaultValue = "0") val pollSeq: Long = 0,
    /**
     * The record of a voice call, when this message is one
     * (docs/protocol.md, "Voice calls"): "completed" | "missed" |
     * "declined" | "failed", and the seconds the call lasted when it was
     * ever answered. Null = not a call, which is the only thing absence
     * ever means here — a record is written once and never changes.
     * The body is the server's English placeholder, which the bubble
     * never shows once it knows the outcome.
     */
    val callOutcome: String? = null,
    val callDurationSecs: Int? = null,
    /**
     * True when the call above was a VIDEO call (docs/protocol.md,
     * "Video") — meaningless while [callOutcome] is null. A real default
     * rather than nullable, because absent-on-the-wire IS "voice".
     */
    @ColumnInfo(defaultValue = "0") val callVideo: Boolean = false,
    /**
     * The message's attachments, stored as the wire-shape JSON array
     * (see AttachmentsCodec) — 1 to 10 of them, in the sender's order.
     * Null on a row that carries none, and on rows written before
     * plurality, whose single attachment still lives in the flat
     * columns; [attachmentList] reads through both.
     */
    val attachmentsJson: String? = null,
) {
    /** The wire shape back out of the call columns, or null for a message that is not a call. */
    val call: CallDto?
        get() = callOutcome?.let { CallDto(outcome = it, durationSecs = callDurationSecs, video = callVideo) }

    /**
     * Every attachment on this message, in the sender's order: the JSON
     * column when it is present and readable, else the flat columns'
     * single attachment (a pre-plurality row), else nothing.
     */
    val attachmentList: List<AttachmentDto>
        get() = AttachmentsCodec.decode(attachmentsJson)
            ?: legacyAttachment?.let(::listOf).orEmpty()

    /** The ids [attachmentList] carries, for a send that must re-claim them. */
    val attachmentIds: List<Long>?
        get() = attachmentList.map { it.id }.takeIf { it.isNotEmpty() }

    /** The FIRST attachment — the pre-plurality read, kept because every
     *  single-attachment consumer (share, save, the context menu) means
     *  exactly this. */
    val attachment: AttachmentDto?
        get() = attachmentList.firstOrNull()

    /** The wire shape back out of the flat columns, or null for no media. */
    private val legacyAttachment: AttachmentDto?
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
                latitude = attachmentLatitude,
                longitude = attachmentLongitude,
                accuracyM = attachmentAccuracyM,
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
    /**
     * The account itself is GONE (docs/protocol.md, "Deleting an
     * account") — not merely out of this family.
     *
     * A second flag rather than a state of [hasLeft], because the two
     * answer different questions and both have to keep their answer: a
     * deleted account has also left, but somebody who left can come back
     * and a deleted one never can. This row survives for exactly one
     * reason — their messages, notes and reactions are still in the
     * family's history and have to be given a name.
     *
     * What it costs every reader: the display name is the server's
     * ENGLISH placeholder, so a screen draws its own translation instead
     * (see ChatViewModel.memberNames); there is no picture, because
     * [avatarVersion] is 0 on a tombstone; and no roster, picker, admin
     * list or statistic may include them. NOT NULL with a DEFAULT because
     * every row already in the table has an answer: they are not deleted.
     */
    @ColumnInfo(defaultValue = "0") val deleted: Boolean = false,
    /**
     * A day and a month, flattened into two nullable columns — the same
     * treatment the reply quote and the attachment get, and for the same
     * reason: it belongs to exactly one row and is read on every render
     * of it.
     *
     * Nullable with no default, because absence IS the meaning: unset is
     * not month 0, and there is no year here to be missing. Both columns
     * move together or not at all — read them through [birthday], which
     * is the only thing that should be asked.
     */
    val birthdayMonth: Int? = null,
    val birthdayDay: Int? = null,
) {
    /** The wire shape back out of the flat columns, or null for unset. */
    val birthday: BirthdayDto?
        get() {
            val month = birthdayMonth ?: return null
            val day = birthdayDay ?: return null
            return BirthdayDto(month = month, day = day)
        }
}
