/*
 * MediaOnlyTest.kt
 * Family Connect (Android)
 *
 * Pins which messages the bubble draws BARE for their attachments — no
 * balloon, the tile as the message — and which keep the balloon. A
 * cross-platform contract (MediaOnlyTests.swift mirrors these vectors):
 * photos and videos alone, one or several, draw bare; a caption, a quote,
 * a poll, a call, a body still arriving, or any file, audio or location
 * row keeps the balloon, because those are words and controls and need
 * the surface.
 */

package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.data.db.MessageStatus
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.net.dto.AttachmentsCodec
import me.nettrash.familyconnect.data.net.dto.PollCodec
import me.nettrash.familyconnect.data.net.dto.PollDto
import me.nettrash.familyconnect.data.net.dto.PollOptionDto
import me.nettrash.familyconnect.testutil.FakeAttachmentApi
import org.junit.Test

class MediaOnlyTest {

    private val photo = FakeAttachmentApi.attachment(id = 1, kind = "photo")
    private val video = FakeAttachmentApi.attachment(id = 2, kind = "video")
    private val file = FakeAttachmentApi.attachment(id = 3, kind = "file")
    private val audio = FakeAttachmentApi.attachment(id = 4, kind = "audio")
    private val place = FakeAttachmentApi.attachment(id = 5, kind = "location")

    private fun message(
        body: String = "",
        attachments: List<AttachmentDto>,
        replyToMessageId: Long? = null,
        pollJson: String? = null,
        callOutcome: String? = null,
    ) = MessageEntity(
        clientMsgId = "m1",
        serverId = 1,
        chatId = 42,
        senderId = 7,
        body = body,
        createdAt = 1_700_000_000_000,
        status = MessageStatus.SENT,
        replyToMessageId = replyToMessageId,
        replySenderId = replyToMessageId?.let { 3 },
        replyExcerpt = replyToMessageId?.let { "which one?" },
        pollJson = pollJson,
        callOutcome = callOutcome,
        attachmentsJson = AttachmentsCodec.encode(attachments).takeIf { attachments.isNotEmpty() },
    )

    private val poll = PollCodec.encode(
        PollDto(
            pollSeq = 1,
            closed = false,
            options = listOf(PollOptionDto(id = 1, text = "Pizza", votes = emptyList()), PollOptionDto(id = 2, text = "Pasta", votes = emptyList())),
        ),
    )

    @Test
    fun photosAndVideosAloneDrawBare() {
        assertThat(isMediaOnly(message(attachments = listOf(photo)))).isTrue()
        assertThat(isMediaOnly(message(attachments = listOf(video)))).isTrue()
        assertThat(isMediaOnly(message(attachments = listOf(photo, video, photo)))).isTrue()
    }

    @Test
    fun aFileAudioOrLocationRowKeepsTheBalloonAloneOrBesideAPhoto() {
        assertThat(isMediaOnly(message(attachments = listOf(file)))).isFalse()
        assertThat(isMediaOnly(message(attachments = listOf(audio)))).isFalse()
        assertThat(isMediaOnly(message(attachments = listOf(place)))).isFalse()
        assertThat(isMediaOnly(message(attachments = listOf(photo, file)))).isFalse()
        assertThat(isMediaOnly(message(attachments = listOf(photo, photo, audio)))).isFalse()
    }

    @Test
    fun anythingWithWordsKeepsTheBalloon() {
        assertThat(isMediaOnly(message(body = "look", attachments = listOf(photo)))).isFalse()
        assertThat(isMediaOnly(message(attachments = listOf(photo), replyToMessageId = 9))).isFalse()
        assertThat(isMediaOnly(message(attachments = listOf(photo), pollJson = poll))).isFalse()
        assertThat(isMediaOnly(message(attachments = listOf(photo), callOutcome = "missed"))).isFalse()
        assertThat(isMediaOnly(message(attachments = listOf(photo)), isStreaming = true)).isFalse()
    }

    @Test
    fun aTextMessageAndAnEmptyOneAreNotMediaOnly() {
        assertThat(isMediaOnly(message(body = "hi", attachments = emptyList()))).isFalse()
        assertThat(isMediaOnly(message(attachments = emptyList()))).isFalse()
    }

    @Test
    fun aPrePluralityRowWithItsSinglePhotoInTheFlatColumnsDrawsBare() {
        // A row written before attachments became a list keeps its one
        // attachment in the flat columns; the rule reads through both.
        val legacy = MessageEntity(
            clientMsgId = "m0",
            serverId = 1,
            chatId = 42,
            senderId = 7,
            body = "",
            createdAt = 1_700_000_000_000,
            status = MessageStatus.SENT,
            attachmentId = photo.id,
            attachmentKind = photo.kind,
            attachmentMime = photo.mime,
            attachmentSize = photo.size,
            attachmentWidth = photo.width,
            attachmentHeight = photo.height,
        )
        assertThat(isMediaOnly(legacy)).isTrue()
    }
}
