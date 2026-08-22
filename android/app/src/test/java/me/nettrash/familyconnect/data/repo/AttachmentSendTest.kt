/*
 * AttachmentSendTest.kt
 * Family Connect (Android)
 *
 * Sending a photo or video (docs/protocol.md, "Photos and videos"):
 * the order of the three calls, what the row carries before and after the
 * ack, and what happens when the bytes are refused.
 *
 * The size ceiling itself is checked on the server
 * (server/tests/attachment_flow.rs); what matters here is that the client
 * never shows a bubble for bytes that did not land.
 *
 * iOS counterpart: ios/FamilyConnectTests/AttachmentTests.swift
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.db.ChatDao
import me.nettrash.familyconnect.data.db.ChatEntity
import me.nettrash.familyconnect.data.db.MessageDao
import me.nettrash.familyconnect.data.db.MessageStatus
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.AttachmentResponse
import me.nettrash.familyconnect.data.net.dto.MessageDto
import me.nettrash.familyconnect.data.net.dto.MessageResponse
import me.nettrash.familyconnect.data.net.ws.ClientFrame
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.testutil.FakeAttachmentApi
import me.nettrash.familyconnect.testutil.FakeChatApi
import me.nettrash.familyconnect.testutil.FakeChatSocket
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.createTestDb
import me.nettrash.familyconnect.util.Clock
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import java.io.File

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class AttachmentSendTest {

    private companion object {
        const val ME = 7L
        const val CHAT = 42L
        const val NOW = 1_000_000L
    }

    private val dispatcher = StandardTestDispatcher()
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private lateinit var db: AppDatabase
    private lateinit var messageDao: MessageDao
    private lateinit var chatDao: ChatDao
    private lateinit var chatApi: FakeChatApi
    private val attachmentApi = FakeAttachmentApi()
    private lateinit var socket: FakeChatSocket
    private lateinit var settings: FakeSettingsRepository

    @Before
    fun setUp() {
        db = createTestDb(dispatcher)
        messageDao = db.messageDao()
        chatDao = db.chatDao()
        chatApi = FakeChatApi()
        socket = FakeChatSocket()
        settings = FakeSettingsRepository(
            SettingsState(
                serverUrl = "https://chat.example.com",
                familyStatus = FamilyStatus.MEMBER,
                myUserId = ME,
            ),
        )
    }

    @After
    fun tearDown() {
        repoScope.cancel()
        db.close()
    }

    private fun TestScope.newRepository(): MessageRepository {
        val repository = MessageRepository(
            chatApi = chatApi,
            attachmentApi = attachmentApi,
            messageDao = messageDao,
            chatDao = chatDao,
            socket = socket,
            settings = settings,
            chatRepository = ChatRepository(chatApi, chatDao, socket),
            scope = repoScope,
            clock = Clock { NOW },
        )
        runCurrent()
        return repository
    }

    private suspend fun insertChat() {
        chatDao.upsertAll(
            listOf(
                ChatEntity(
                    id = CHAT,
                    kind = "family",
                    peerUserId = null,
                    title = "The Smiths",
                    unreadCount = 0,
                    myLastReadId = null,
                    peerLastReadId = null,
                    lastMessageBody = null,
                    lastMessageAt = null,
                    lastMessageSenderId = null,
                ),
            ),
        )
    }

    /** A prepared photo on disk, as MediaPrep would leave one. */
    private fun prepared(previewJpeg: ByteArray? = ByteArray(64) { 0x7 }): MediaPrep.Prepared {
        val file = File.createTempFile("fc-upload", ".jpg")
        file.writeBytes(byteArrayOf(0xFF.toByte(), 0xD8.toByte(), 0xFF.toByte(), 0xE0.toByte()))
        return MediaPrep.Prepared(
            file = file,
            mime = "image/jpeg",
            kind = "photo",
            width = 1600,
            height = 1200,
            durationMs = null,
            previewJpeg = previewJpeg,
        )
    }

    private fun ackWith(hasPreview: Boolean) {
        chatApi.postMessageHandler = { chatId, clientMsgId, body ->
            ApiResult.Ok(
                MessageResponse(
                    MessageDto(
                        id = 900,
                        chatId = chatId,
                        senderId = ME,
                        clientMsgId = clientMsgId,
                        body = body,
                        createdAt = "2026-08-22T09:00:00Z",
                        attachment = FakeAttachmentApi.attachment(hasPreview = hasPreview),
                    ),
                ),
            )
        }
    }

    @Test
    fun `bytes go up first, then the preview, then the message`() = runTest(dispatcher) {
        insertChat()
        val repository = newRepository()
        ackWith(hasPreview = true)
        val media = prepared()

        assertThat(repository.sendMedia(media, caption = "", chatId = CHAT)).isTrue()
        advanceUntilIdle()

        // A message pointing at an upload that never landed would be
        // worse than a composer that is visibly busy — so the order is
        // load-bearing, not incidental.
        assertThat(attachmentApi.calls).containsExactly("upload", "preview").inOrder()
        assertThat(chatApi.postedAttachmentIds).containsExactly(34L)

        val row = messageDao.findByClientMsgId(chatApi.postedMessages.single().second)
        assertThat(row).isNotNull()
        // A photo needs no caption.
        assertThat(row!!.body).isEmpty()
        assertThat(row.attachmentId).isEqualTo(34)
        assertThat(row.attachment?.kind).isEqualTo("photo")
        assertThat(row.attachment?.width).isEqualTo(1600)
        assertThat(row.status).isEqualTo(MessageStatus.SENT)

        // The prepared file is ours; it does not outlive the send.
        assertThat(media.file.exists()).isFalse()
    }

    @Test
    fun `a caption rides along with the photo`() = runTest(dispatcher) {
        insertChat()
        val repository = newRepository()
        ackWith(hasPreview = true)

        assertThat(repository.sendMedia(prepared(), "at the lake", CHAT)).isTrue()
        advanceUntilIdle()

        val row = messageDao.findByClientMsgId(chatApi.postedMessages.single().second)
        assertThat(row!!.body).isEqualTo("at the lake")
        assertThat(chatApi.postedMessages.single().third).isEqualTo("at the lake")
    }

    @Test
    fun `a refused upload sends no message at all`() = runTest(dispatcher) {
        insertChat()
        val repository = newRepository()
        attachmentApi.uploadHandler = { _, _, _ ->
            ApiResult.HttpError(413, "attachment_too_large", "too big")
        }
        val media = prepared()

        assertThat(repository.sendMedia(media, caption = "", chatId = CHAT)).isFalse()
        advanceUntilIdle()

        assertThat(chatApi.postedMessages).isEmpty()
        assertThat(attachmentApi.calls).containsExactly("upload")
        assertThat(media.file.exists()).isFalse()
    }

    /** The preview is best-effort: a bubble without one fetches the full photo. */
    @Test
    fun `a failed preview upload does not fail the send`() = runTest(dispatcher) {
        insertChat()
        val repository = newRepository()
        attachmentApi.previewHandler = { _, _ ->
            ApiResult.HttpError(500, "internal", "nope")
        }
        // As the real server answers: the message carries its attachment,
        // and has_preview is false because the preview never landed.
        ackWith(hasPreview = false)

        assertThat(repository.sendMedia(prepared(), caption = "", chatId = CHAT)).isTrue()
        advanceUntilIdle()

        val row = messageDao.findByClientMsgId(chatApi.postedMessages.single().second)
        assertThat(row!!.attachmentId).isEqualTo(34)
        // Server truth wins over what this device guessed at send time.
        assertThat(row.attachmentHasPreview).isFalse()
    }

    @Test
    fun `no preview to upload means no preview request`() = runTest(dispatcher) {
        insertChat()
        val repository = newRepository()
        ackWith(hasPreview = false)

        assertThat(repository.sendMedia(prepared(previewJpeg = null), "", CHAT)).isTrue()
        advanceUntilIdle()

        assertThat(attachmentApi.calls).containsExactly("upload")
    }

    /** The socket path carries the attachment id too, not just REST. */
    @Test
    fun `an open socket sends the attachment id in the frame`() = runTest(dispatcher) {
        insertChat()
        val repository = newRepository()
        socket.setOpen(true)
        runCurrent()

        assertThat(repository.sendMedia(prepared(), caption = "", chatId = CHAT)).isTrue()
        runCurrent()

        val frame = socket.sent.filterIsInstance<ClientFrame.Send>().single()
        assertThat(frame.attachmentId).isEqualTo(34)
        assertThat(frame.body).isEmpty()
    }

    @Test
    fun `an inbound message carries its attachment into the row`() = runTest(dispatcher) {
        insertChat()
        val repository = newRepository()

        repository.applyServerMessage(
            MessageDto(
                id = 901,
                chatId = CHAT,
                senderId = 9,
                clientMsgId = "theirs",
                body = "",
                createdAt = "2026-08-22T09:05:00Z",
                attachment = FakeAttachmentApi.attachment(
                    id = 77,
                    kind = "video",
                    hasPreview = true,
                ),
            ),
            live = true,
        )
        advanceUntilIdle()

        val row = messageDao.findByServerId(901)
        assertThat(row).isNotNull()
        val attachment = row!!.attachment
        assertThat(attachment).isNotNull()
        assertThat(attachment!!.id).isEqualTo(77)
        assertThat(attachment.isVideo).isTrue()
        assertThat(attachment.durationMs).isEqualTo(8400)
        assertThat(attachment.hasPreview).isTrue()
    }

    @Test
    fun `a message with no attachment has none on the row`() = runTest(dispatcher) {
        insertChat()
        val repository = newRepository()

        repository.applyServerMessage(
            MessageDto(
                id = 902,
                chatId = CHAT,
                senderId = 9,
                clientMsgId = "plain",
                body = "See you at six",
                createdAt = "2026-08-22T09:06:00Z",
            ),
            live = true,
        )
        advanceUntilIdle()

        assertThat(messageDao.findByServerId(902)!!.attachment).isNull()
    }

    // -- Files ---------------------------------------------------------------

    /** A prepared file on disk, as MediaPrep.prepareFile would leave one. */
    private fun preparedFile(name: String = "receipts.pdf"): MediaPrep.Prepared {
        val file = File.createTempFile("fc-upload", ".pdf")
        file.writeBytes("%PDF-1.7 and then some".toByteArray())
        return MediaPrep.Prepared(
            file = file,
            mime = "application/pdf",
            kind = "file",
            width = null,
            height = null,
            durationMs = null,
            previewJpeg = null,
            name = name,
        )
    }

    @Test
    fun `a file is uploaded with its name and no preview`() = runTest(dispatcher) {
        insertChat()
        val repository = newRepository()
        attachmentApi.uploadHandler = { _, _, _ ->
            ApiResult.Ok(AttachmentResponse(FakeAttachmentApi.attachment(kind = "file")))
        }
        chatApi.postMessageHandler = { chatId, clientMsgId, body ->
            ApiResult.Ok(
                MessageResponse(
                    MessageDto(
                        id = 910,
                        chatId = chatId,
                        senderId = ME,
                        clientMsgId = clientMsgId,
                        body = body,
                        createdAt = "2026-08-22T09:00:00Z",
                        attachment = FakeAttachmentApi.attachment(kind = "file"),
                    ),
                ),
            )
        }

        assertThat(repository.sendMedia(preparedFile(), caption = "", chatId = CHAT)).isTrue()
        advanceUntilIdle()

        // No preview call at all: a document has nothing to draw.
        assertThat(attachmentApi.calls).containsExactly("upload")
        assertThat(attachmentApi.uploadedNames).containsExactly("receipts.pdf")

        val row = messageDao.findByClientMsgId(chatApi.postedMessages.single().second)
        val attachment = row!!.attachment
        assertThat(attachment!!.isFile).isTrue()
        assertThat(attachment.name).isEqualTo("receipts.pdf")
        assertThat(attachment.hasPreview).isFalse()
        // A file has no shape, so the bubble draws a row rather than a box.
        assertThat(attachment.width).isNull()
    }

    @Test
    fun `an inbound file keeps its name through the row`() = runTest(dispatcher) {
        insertChat()
        val repository = newRepository()

        repository.applyServerMessage(
            MessageDto(
                id = 911,
                chatId = CHAT,
                senderId = 9,
                clientMsgId = "theirs-file",
                body = "",
                createdAt = "2026-08-22T09:07:00Z",
                attachment = FakeAttachmentApi.attachment(
                    id = 88,
                    kind = "file",
                    name = "Rechnung März.pdf",
                ),
            ),
            live = true,
        )
        advanceUntilIdle()

        val attachment = messageDao.findByServerId(911)!!.attachment
        assertThat(attachment!!.name).isEqualTo("Rechnung März.pdf")
        assertThat(attachment.displayName).isEqualTo("Rechnung März.pdf")
    }

    /** The bubble falls back to a word rather than showing "attachment 34". */
    @Test
    fun `a nameless attachment still has something to call itself`() {
        assertThat(FakeAttachmentApi.attachment(kind = "photo").displayName).isEqualTo("Photo")
        assertThat(FakeAttachmentApi.attachment(kind = "video").displayName).isEqualTo("Video")
    }

    /**
     * A caption-less photo wrote "" as the chat-list preview, and an empty
     * string is not null — so the row rendered blank instead of falling
     * back. Mirrors iOS's previewFallsBackToTheAttachment.
     */
    @Test
    fun `the chat-list preview says what arrived when there is no caption`() {
        fun attachment(kind: String, name: String? = null) =
            FakeAttachmentApi.attachment(kind = kind, name = name)

        assertThat(MessageRepository.previewText("", attachment("photo"))).isEqualTo("Photo")
        assertThat(MessageRepository.previewText("", attachment("video"))).isEqualTo("Video")
        assertThat(MessageRepository.previewText("", attachment("file", "taxes.zip")))
            .isEqualTo("taxes.zip")
        // A caption still wins, and a plain message is untouched.
        assertThat(MessageRepository.previewText("at the lake", attachment("photo")))
            .isEqualTo("at the lake")
        assertThat(MessageRepository.previewText("hello", null)).isEqualTo("hello")
    }

    /**
     * The row the send writes must carry the same text, or the chat list
     * is blank until the next resync repairs it.
     */
    @Test
    fun `sending a caption-less photo writes a usable chat preview`() = runTest(dispatcher) {
        insertChat()
        val repository = newRepository()
        ackWith(hasPreview = true)

        assertThat(repository.sendMedia(prepared(), caption = "", chatId = CHAT)).isTrue()
        advanceUntilIdle()

        assertThat(chatDao.getById(CHAT)!!.lastMessageBody).isEqualTo("Photo")
    }

    /** Unused here, but pins the response type the API decodes into. */
    @Test
    fun `the upload response decodes to an attachment`() {
        val response = AttachmentResponse(FakeAttachmentApi.attachment(kind = "video"))
        assertThat(response.attachment.isVideo).isTrue()
        // Portrait or landscape, a bubble always has a shape to reserve.
        assertThat(response.attachment.aspectRatio).isGreaterThan(0f)
    }
}
