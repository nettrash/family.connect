/*
 * RoundPresentationTest.kt
 * Family Connect (Android)
 *
 * Which messages are drawn as a CIRCLE, and what that changes around it
 * (#79, docs/audio-video-messages-2026-10-04.md, S5.1, S5.2, S5.4, S5.7): the
 * one test every client shares — exactly one attachment, `kind=video`,
 * `round: true` — no Edit, the quote that names it, its size, its dot and
 * its ring. StickerPresentationTest's twin.
 */

package me.nettrash.familyconnect.ui.chat

import androidx.compose.ui.unit.dp
import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.data.db.MessageStatus
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.net.dto.AttachmentsCodec
import org.junit.Test

class RoundPresentationTest {

    private val circle = AttachmentDto(
        id = 91, kind = "video", mime = "video/mp4", size = 1_649_700, width = 480, height = 480,
        durationMs = 23_400, hasPreview = true, round = true,
    )
    private val video = circle.copy(id = 92, round = null)
    private val photo = AttachmentDto(id = 93, kind = "photo", mime = "image/jpeg", size = 9)

    private fun message(
        attachments: List<AttachmentDto>,
        body: String = "",
        serverId: Long? = 1,
        senderId: Long = 9,
        replyToMessageId: Long? = null,
        attachmentsJson: String? = AttachmentsCodec.encode(attachments).takeIf { attachments.isNotEmpty() },
    ) = MessageEntity(
        clientMsgId = "m$serverId-$replyToMessageId",
        serverId = serverId,
        chatId = 42,
        senderId = senderId,
        body = body,
        createdAt = 1_700_000_000_000,
        status = MessageStatus.SENT,
        replyToMessageId = replyToMessageId,
        replySenderId = replyToMessageId?.let { 3 },
        replyExcerpt = replyToMessageId?.let { "" },
        attachmentsJson = attachmentsJson,
    )

    // -- Which messages are circles ------------------------------------------------------

    @Test
    fun `one flagged video is a circle`() {
        assertThat(roundOf(message(listOf(circle)))).isEqualTo(circle)
        // A reply is still one, with its quote above it.
        assertThat(roundOf(message(listOf(circle), replyToMessageId = 41))).isEqualTo(circle)
    }

    @Test
    fun `the test is exactly one attachment, a video, flagged - and nothing else`() {
        // ONE attachment:
        assertThat(roundOf(message(listOf(circle, video)))).isNull()
        assertThat(roundOf(message(listOf(circle, circle.copy(id = 94))))).isNull()
        assertThat(roundOf(message(emptyList()))).isNull()
        // of kind VIDEO:
        for (kind in listOf("photo", "audio", "file", "location")) {
            assertThat(roundOf(message(listOf(circle.copy(kind = kind))))).isNull()
        }
        // carrying `round: true`:
        assertThat(roundOf(message(listOf(video)))).isNull()
        assertThat(roundOf(message(listOf(circle.copy(round = false))))).isNull()
        // and NO BODY, compared exactly as fc_text::record::is_round does
        // ("a body beside it", "a body of one space" in record-vectors.json):
        // words beside it — which the server refuses anyway — draw it as the
        // ordinary video it then is, as on the web, iOS and Windows.
        assertThat(roundOf(message(listOf(circle), body = "hi"))).isNull()
        assertThat(roundOf(message(listOf(circle), body = " "))).isNull()
        assertThat(roundOf(message(listOf(circle), body = ""))).isEqualTo(circle)
        // And a sticker is not a circle, nor a circle a sticker.
        assertThat(stickerOf(message(listOf(circle)))).isNull()
    }

    @Test
    fun `a video cached before the flag existed draws as the square video it was`() {
        val older = message(
            attachments = emptyList(),
            attachmentsJson = """[{"id":92,"kind":"video","mime":"video/mp4","size":9,"width":480,"height":480}]""",
        )
        assertThat(roundOf(older)).isNull()
        assertThat(isMediaOnly(older)).isTrue()
    }

    // -- Edit -----------------------------------------------------------------------------

    @Test
    fun `edit is never offered on a video message`() {
        val mine = 9L
        assertThat(canEditMessage(message(listOf(circle), senderId = mine), myUserId = mine)).isFalse()
        assertThat(canEditMessage(message(listOf(circle), senderId = mine, replyToMessageId = 41), mine)).isFalse()
        // An ordinary video of mine with a caption still is.
        assertThat(canEditMessage(message(listOf(video), body = "look", senderId = mine), mine)).isTrue()
    }

    // -- The quote ----------------------------------------------------------------------------

    @Test
    fun `a reply to a circle the list holds says so, and a reply to anything else does not`() {
        val items = buildChatItems(
            messagesNewestFirst = listOf(
                message(listOf(photo), serverId = 3, replyToMessageId = 2),
                message(emptyList(), body = "nice", serverId = 4, replyToMessageId = 1),
                message(listOf(photo), serverId = 2),
                message(listOf(circle), serverId = 1),
            ),
            isFamilyChat = true,
            myUserId = 7,
            memberNames = emptyMap(),
            nowMillis = 1_700_000_000_000,
        ).filterIsInstance<ChatListItem.MessageItem>().associateBy { it.entity.serverId }

        assertThat(items.getValue(4).quotesRound).isTrue()
        assertThat(items.getValue(3).quotesRound).isFalse()
        assertThat(items.getValue(1).quotesRound).isFalse()
    }

    @Test
    fun `a reply to a circle that is not loaded says what it always said`() {
        val items = buildChatItems(
            messagesNewestFirst = listOf(message(emptyList(), body = "nice", serverId = 4, replyToMessageId = 1)),
            isFamilyChat = true,
            myUserId = 7,
            memberNames = emptyMap(),
            nowMillis = 1_700_000_000_000,
        )

        assertThat(items.filterIsInstance<ChatListItem.MessageItem>().single().quotesRound).isFalse()
        assertThat(quotesRound(items, 1)).isFalse()
    }

    @Test
    fun `the reply banner knows a circle by its id`() {
        val items = buildChatItems(
            messagesNewestFirst = listOf(message(listOf(video), serverId = 2), message(listOf(circle), serverId = 1)),
            isFamilyChat = true,
            myUserId = 7,
            memberNames = emptyMap(),
            nowMillis = 1_700_000_000_000,
        )

        assertThat(quotesRound(items, 1)).isTrue()
        assertThat(quotesRound(items, 2)).isFalse()
        assertThat(quotesRound(items, 99)).isFalse()
    }

    // -- How it is drawn ---------------------------------------------------------------------

    @Test
    fun `200 under 600 dp, 240 from 600 dp`() {
        assertThat(RoundVideoDrawing.diameter(360)).isEqualTo(200.dp)
        assertThat(RoundVideoDrawing.diameter(599)).isEqualTo(200.dp)
        assertThat(RoundVideoDrawing.diameter(600)).isEqualTo(240.dp)
        assertThat(RoundVideoDrawing.diameter(1280)).isEqualTo(240.dp)
        // Larger than a sticker (160), and the play disc and dot are S5.2's.
        assertThat(RoundVideoDrawing.COMPACT).isGreaterThan(160.dp)
        assertThat(RoundVideoDrawing.PLAY_DISC).isEqualTo(44.dp)
        assertThat(RoundVideoDrawing.UNPLAYED_DOT).isEqualTo(8.dp)
        assertThat(RoundVideoDrawing.PROGRESS_RING).isEqualTo(3.dp)
    }

    @Test
    fun `the dot is somebody else's circle this device has not played`() {
        assertThat(RoundVideoRules.showsUnplayedDot(isMine = false, acked = true, played = false)).isTrue()
        assertThat(RoundVideoRules.showsUnplayedDot(isMine = false, acked = true, played = true)).isFalse()
        assertThat(RoundVideoRules.showsUnplayedDot(isMine = true, acked = true, played = false)).isFalse()
        assertThat(RoundVideoRules.showsUnplayedDot(isMine = false, acked = false, played = false)).isFalse()
    }

    @Test
    fun `the ring sweeps, and steps once a second under reduced motion`() {
        assertThat(RoundVideoRules.progress(positionMs = 11_700, durationMs = 23_400, stepped = false))
            .isWithin(1e-6f).of(0.5f)
        assertThat(RoundVideoRules.progress(positionMs = 11_700, durationMs = 23_400, stepped = true))
            .isWithin(1e-6f).of(11_000f / 23_400f)
        assertThat(RoundVideoRules.progress(positionMs = 30_000, durationMs = 23_400, stepped = false)).isEqualTo(1f)
        assertThat(RoundVideoRules.progress(positionMs = 5_000, durationMs = 0, stepped = false)).isEqualTo(0f)
    }
}
