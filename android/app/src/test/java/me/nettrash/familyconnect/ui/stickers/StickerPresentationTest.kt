/*
 * StickerPresentationTest.kt
 * Family Connect (Android)
 *
 * How a chat STICKER is told apart and drawn (docs/protocol.md, "Sticker
 * pack" → "How it is drawn"): which messages are one, that an old message
 * without the flag is still the photo it was, that a blocked member's
 * sticker is hidden like any message of theirs, and the numbers of the one
 * fixed box.
 *
 * The chat kind of sticker. The board's notes are StickyNoteTest's.
 */

package me.nettrash.familyconnect.ui.stickers

import androidx.compose.ui.unit.dp
import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.data.db.MessageStatus
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.net.dto.AttachmentsCodec
import me.nettrash.familyconnect.ui.chat.ChatListItem
import me.nettrash.familyconnect.ui.chat.buildChatItems
import me.nettrash.familyconnect.ui.chat.canEditMessage
import me.nettrash.familyconnect.ui.chat.isMediaOnly
import me.nettrash.familyconnect.ui.chat.stickerOf
import org.junit.Test

class StickerPresentationTest {

    private val sticker = AttachmentDto(
        id = 90, kind = "photo", mime = "image/webp", size = 40_211, width = 512, height = 512, sticker = true,
    )
    private val photo = sticker.copy(id = 91, sticker = null)

    private fun message(
        attachments: List<AttachmentDto>,
        body: String = "",
        senderId: Long = 9,
        replyToMessageId: Long? = null,
        attachmentsJson: String? = AttachmentsCodec.encode(attachments).takeIf { attachments.isNotEmpty() },
    ) = MessageEntity(
        clientMsgId = "m1",
        serverId = 1,
        chatId = 42,
        senderId = senderId,
        body = body,
        createdAt = 1_700_000_000_000,
        status = MessageStatus.SENT,
        replyToMessageId = replyToMessageId,
        replySenderId = replyToMessageId?.let { 3 },
        replyExcerpt = replyToMessageId?.let { "which one?" },
        attachmentsJson = attachmentsJson,
    )

    // -- Which messages are stickers ---------------------------------------------

    @Test
    fun `a message whose one attachment is flagged is a sticker`() {
        assertThat(stickerOf(message(listOf(sticker)))).isEqualTo(sticker)
    }

    @Test
    fun `a sticker that is a reply is still a sticker`() {
        // Replying with one is how a sticker answers something — and,
        // unlike a bare photo, it stays bare under its quote.
        val reply = message(listOf(sticker), replyToMessageId = 41)

        assertThat(stickerOf(reply)).isEqualTo(sticker)
        assertThat(isMediaOnly(reply)).isFalse()
    }

    @Test
    fun `a photo without the flag is a photo`() {
        val plain = message(listOf(photo))

        assertThat(stickerOf(plain)).isNull()
        // …and keeps the treatment it always had.
        assertThat(isMediaOnly(plain)).isTrue()
    }

    @Test
    fun `a message cached before stickers existed draws as the photo it was`() {
        // The flat columns only — a row from before plurality, let alone
        // before the flag. Nothing about it changed when stickers arrived.
        val old = MessageEntity(
            clientMsgId = "old",
            serverId = 1,
            chatId = 42,
            senderId = 9,
            body = "",
            createdAt = 1_600_000_000_000,
            status = MessageStatus.SENT,
            attachmentId = 34,
            attachmentKind = "photo",
            attachmentMime = "image/webp",
            attachmentSize = 1234,
        )
        // And a JSON row written by a build that did not know the key.
        val olderJson = message(
            attachments = emptyList(),
            attachmentsJson = """[{"id":34,"kind":"photo","mime":"image/webp","size":1234}]""",
        )

        assertThat(stickerOf(old)).isNull()
        assertThat(isMediaOnly(old)).isTrue()
        assertThat(stickerOf(olderJson)).isNull()
        assertThat(isMediaOnly(olderJson)).isTrue()
    }

    @Test
    fun `the test is exactly one attachment, a photo, flagged - and nothing else`() {
        // The same three conditions on every client.
        // ONE attachment:
        assertThat(stickerOf(message(listOf(sticker, photo)))).isNull()
        assertThat(stickerOf(message(listOf(sticker, sticker.copy(id = 92))))).isNull()
        assertThat(stickerOf(message(emptyList()))).isNull()
        // of kind PHOTO:
        assertThat(stickerOf(message(listOf(sticker.copy(kind = "file"))))).isNull()
        assertThat(stickerOf(message(listOf(sticker.copy(kind = "video"))))).isNull()
        // carrying `sticker: true`:
        assertThat(stickerOf(message(listOf(sticker.copy(sticker = false))))).isNull()
        assertThat(stickerOf(message(listOf(photo)))).isNull()
        // The body is NOT part of the test. No conforming server sends one
        // beside the flag; a client that looked at it could only disagree
        // with one that did not.
        assertThat(stickerOf(message(listOf(sticker), body = "look"))).isEqualTo(sticker)
    }

    // -- Edit ------------------------------------------------------------------------

    @Test
    fun `edit is never offered on a sticker`() {
        val mine = 9L
        // The reader's own sticker, delivered: everything "Edit" otherwise
        // asks for — and still no.
        assertThat(canEditMessage(message(listOf(sticker), senderId = mine), myUserId = mine)).isFalse()
        assertThat(
            canEditMessage(message(listOf(sticker), senderId = mine, replyToMessageId = 41), myUserId = mine),
        ).isFalse()
        // Stated, not implied by an empty body: a sticker row that somehow
        // carried words is no more editable.
        assertThat(
            canEditMessage(message(listOf(sticker), body = "look", senderId = mine), myUserId = mine),
        ).isFalse()
    }

    @Test
    fun `edit is offered on the reader's own ordinary message and nobody else's`() {
        val mine = 9L
        assertThat(canEditMessage(message(emptyList(), body = "hello", senderId = mine), myUserId = mine)).isTrue()
        // A WebP photograph is a photo: what cannot be edited is the flag.
        assertThat(canEditMessage(message(listOf(photo), body = "look", senderId = mine), myUserId = mine)).isTrue()
        assertThat(canEditMessage(message(emptyList(), body = "hello", senderId = 3), myUserId = mine)).isFalse()
        assertThat(canEditMessage(message(emptyList(), body = "hello", senderId = mine), myUserId = null)).isFalse()
    }

    // -- Blocking ------------------------------------------------------------------

    @Test
    fun `a blocked member's sticker is hidden like any message of theirs`() {
        val items = buildChatItems(
            messagesNewestFirst = listOf(message(listOf(sticker), senderId = 9)),
            isFamilyChat = true,
            myUserId = 7,
            memberNames = mapOf(9L to "Ben"),
            nowMillis = 1_700_000_000_000,
            blockedUserIds = setOf(9L),
        )

        val row = items.filterIsInstance<ChatListItem.MessageItem>().single()
        // The row STAYS (history paging and the read marker depend on it)
        // and draws the placeholder; the bubble never reaches the sticker.
        assertThat(row.isHiddenByBlock).isTrue()
        assertThat(row.showSenderName).isFalse()
    }

    @Test
    fun `an unblocked member's sticker is not hidden`() {
        val items = buildChatItems(
            messagesNewestFirst = listOf(message(listOf(sticker), senderId = 9)),
            isFamilyChat = true,
            myUserId = 7,
            memberNames = mapOf(9L to "Ben"),
            nowMillis = 1_700_000_000_000,
            blockedUserIds = setOf(11L),
        )

        assertThat(items.filterIsInstance<ChatListItem.MessageItem>().single().isHiddenByBlock).isFalse()
    }

    // -- The box ---------------------------------------------------------------------

    @Test
    fun `the box is larger than an emoji and smaller than a photograph`() {
        // The emoji-only ladder tops out at 96 (EmojiOnly), and a photo
        // tile is up to 240dp wide on a phone (attachmentMaxWidth).
        // 160, the number every client draws a sticker's box at.
        assertThat(StickerDrawing.CHAT_BOX).isEqualTo(160.dp)
        assertThat(StickerDrawing.CHAT_BOX).isGreaterThan(96.dp)
        assertThat(StickerDrawing.CHAT_BOX).isLessThan(240.dp)
        assertThat(StickerDrawing.ENLARGED_BOX).isGreaterThan(StickerDrawing.CHAT_BOX)
    }

    @Test
    fun `a sticker is decoded for the box it is drawn in and never larger`() {
        // Within the box: left exactly as it is, and never enlarged.
        assertThat(StickerDrawing.targetSize(512, 512)).isNull()
        assertThat(StickerDrawing.targetSize(1024, 1024)).isNull()
        assertThat(StickerDrawing.targetSize(96, 96, edge = 256)).isNull()
        // The case a power-of-two sample cannot reach: 2000 pixels is
        // under twice the ceiling, so sampling leaves it at 2000 — 16 MB
        // a cell. An exact size does not.
        assertThat(StickerDrawing.targetSize(2000, 2000)).isEqualTo(1024 to 1024)
        assertThat(StickerDrawing.targetSize(2000, 2000, edge = 300)).isEqualTo(300 to 300)
        // Whole, proportions kept.
        assertThat(StickerDrawing.targetSize(2000, 1000, edge = 300)).isEqualTo(300 to 150)
        assertThat(StickerDrawing.targetSize(512, 512, edge = 200)).isEqualTo(200 to 200)
        assertThat(StickerDrawing.targetSize(0, 0)).isNull()
        // Nothing decoded is ever over the edge it was decoded for.
        for (side in listOf(257, 511, 1025, 2000, 2047, 2048, 4000)) {
            val (width, height) = StickerDrawing.targetSize(side, side / 2 + 1, edge = 256)!!
            assertThat(maxOf(width, height)).isEqualTo(256)
        }
    }

    @Test
    fun `a box asks for its own pixels up to the ceiling`() {
        // A panel cell on a dense phone, the chat box, the enlarged view.
        assertThat(StickerDrawing.decodeEdge(342)).isEqualTo(342)
        assertThat(StickerDrawing.decodeEdge(504)).isEqualTo(504)
        assertThat(StickerDrawing.decodeEdge(1152)).isEqualTo(StickerDrawing.DECODE_EDGE)
        // Not measured yet: the ceiling, not nothing.
        assertThat(StickerDrawing.decodeEdge(0)).isEqualTo(StickerDrawing.DECODE_EDGE)
    }

    @Test
    fun `the legacy sample never undershoots the box`() {
        assertThat(StickerDrawing.sampleSize(512, 512)).isEqualTo(1)
        assertThat(StickerDrawing.sampleSize(96, 96)).isEqualTo(1)
        assertThat(StickerDrawing.sampleSize(1024, 1024)).isEqualTo(1)
        // Never below the target edge: the layout only ever scales DOWN.
        assertThat(StickerDrawing.sampleSize(2048, 1024)).isEqualTo(2)
        assertThat(StickerDrawing.sampleSize(4096, 4096)).isEqualTo(4)
        assertThat(StickerDrawing.sampleSize(0, 0)).isEqualTo(1)
    }
}
