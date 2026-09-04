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

import org.robolectric.RuntimeEnvironment
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
import me.nettrash.familyconnect.testutil.FakePosterCache
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.testChatRepository
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
    private val posterCache = FakePosterCache()
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
            appContext = RuntimeEnvironment.getApplication(),
            chatApi = chatApi,
            attachmentApi = attachmentApi,
            messageDao = messageDao,
            chatDao = chatDao,
            socket = socket,
            settings = settings,
            chatRepository = testChatRepository(chatApi, chatDao, messageDao, socket, repoScope),
            posterCache = posterCache,
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

        assertThat(repository.sendMedia(listOf(media), caption = "", chatId = CHAT)).isTrue()
        advanceUntilIdle()

        // A message pointing at an upload that never landed would be
        // worse than a composer that is visibly busy — so the order is
        // load-bearing, not incidental.
        assertThat(attachmentApi.calls).containsExactly("upload", "preview").inOrder()
        assertThat(chatApi.postedAttachmentIds).containsExactly(listOf(34L))

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

        assertThat(repository.sendMedia(listOf(prepared()), "at the lake", CHAT)).isTrue()
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

        assertThat(repository.sendMedia(listOf(media), caption = "", chatId = CHAT)).isFalse()
        advanceUntilIdle()

        assertThat(chatApi.postedMessages).isEmpty()
        assertThat(attachmentApi.calls).containsExactly("upload")
        // The prepared file SURVIVES a refused upload — nothing landed,
        // so the composer can re-stage it for a retry (iOS parity).
        assertThat(media.file.exists()).isTrue()
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

        assertThat(repository.sendMedia(listOf(prepared()), caption = "", chatId = CHAT)).isTrue()
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

        assertThat(repository.sendMedia(listOf(prepared(previewJpeg = null)), "", CHAT)).isTrue()
        advanceUntilIdle()

        assertThat(attachmentApi.calls).containsExactly("upload")
    }

    /**
     * ISSUE #54. The poster upload is best-effort, and it used to be
     * best-effort ONCE: a refused `uploadPreview` left `has_preview` false
     * on the server with the pixels only on this device and nothing that
     * would ever offer them again. The send still succeeds — that part
     * never changed — but the frame is now KEPT and the debt recorded, so
     * the next time the network is there the poster goes up.
     */
    @Test
    fun `a video whose poster upload failed keeps the frame and records the debt`() =
        runTest(dispatcher) {
            insertChat()
            val repository = newRepository()
            ackWith(hasPreview = false)
            attachmentApi.uploadHandler = { _, _, _ ->
                ApiResult.Ok(AttachmentResponse(FakeAttachmentApi.attachment(id = 34, kind = "video")))
            }
            attachmentApi.previewHandler = { _, _ -> ApiResult.HttpError(500, "internal", "no") }

            assertThat(repository.sendMedia(listOf(preparedVideo()), "", CHAT)).isTrue()
            advanceUntilIdle()

            assertThat(posterCache.seeded).containsExactly(34L to 64)
            assertThat(posterCache.noted).containsExactly(34L to false)
        }

    /** The same send when the poster lands: kept all the same (the sender
     *  draws its own bubble from it), and nothing is owed. */
    @Test
    fun `a video whose poster landed owes nothing`() = runTest(dispatcher) {
        insertChat()
        val repository = newRepository()
        ackWith(hasPreview = true)
        attachmentApi.uploadHandler = { _, _, _ ->
            ApiResult.Ok(AttachmentResponse(FakeAttachmentApi.attachment(id = 34, kind = "video")))
        }

        assertThat(repository.sendMedia(listOf(preparedVideo()), "", CHAT)).isTrue()
        advanceUntilIdle()

        assertThat(posterCache.seeded).containsExactly(34L to 64)
        assertThat(posterCache.noted).containsExactly(34L to true)
    }

    /**
     * A photo needs none of it. Its preview failing costs bandwidth, not
     * the picture — the bubble falls back to the full bytes — so nothing
     * is kept and nothing is owed.
     */
    @Test
    fun `a photo whose preview upload failed owes nothing`() = runTest(dispatcher) {
        insertChat()
        val repository = newRepository()
        ackWith(hasPreview = false)
        attachmentApi.previewHandler = { _, _ -> ApiResult.HttpError(500, "internal", "no") }

        assertThat(repository.sendMedia(listOf(prepared()), "", CHAT)).isTrue()
        advanceUntilIdle()

        assertThat(posterCache.seeded).isEmpty()
        assertThat(posterCache.noted).isEmpty()
    }

    /** A prepared video on disk, as MediaPrep would leave one. */
    private fun preparedVideo(
        previewJpeg: ByteArray? = ByteArray(64) { 0x7 },
    ): MediaPrep.Prepared {
        val file = File.createTempFile("fc-upload", ".mp4")
        file.writeBytes(byteArrayOf(0, 0, 0, 0x18, 0x66, 0x74, 0x79, 0x70))
        return MediaPrep.Prepared(
            file = file,
            mime = "video/mp4",
            kind = "video",
            width = 1080,
            height = 1920,
            durationMs = 8400,
            previewJpeg = previewJpeg,
        )
    }

    /** The socket path carries the attachment id too, not just REST. */
    @Test
    fun `an open socket sends the attachment id in the frame`() = runTest(dispatcher) {
        insertChat()
        val repository = newRepository()
        socket.setOpen(true)
        runCurrent()

        assertThat(repository.sendMedia(listOf(prepared()), caption = "", chatId = CHAT)).isTrue()
        runCurrent()

        val frame = socket.sent.filterIsInstance<ClientFrame.Send>().single()
        assertThat(frame.attachmentIds).containsExactly(34L)
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

        assertThat(repository.sendMedia(listOf(preparedFile()), caption = "", chatId = CHAT)).isTrue()
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

        assertThat(repository.sendMedia(listOf(prepared()), caption = "", chatId = CHAT)).isTrue()
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

    // -- Plurality (docs/protocol.md, "Photos, videos, audio, files and locations") --

    @Test
    fun `an album uploads each item in the sender's order and sends one message`() =
        runTest(dispatcher) {
            insertChat()
            val repository = newRepository()
            var nextId = 100L
            attachmentApi.uploadHandler = { _, _, kind ->
                ApiResult.Ok(
                    AttachmentResponse(FakeAttachmentApi.attachment(id = nextId++, kind = kind)),
                )
            }
            chatApi.postMessageHandler = { chatId, clientMsgId, body ->
                ApiResult.Ok(
                    MessageResponse(
                        MessageDto(
                            id = 920,
                            chatId = chatId,
                            senderId = ME,
                            clientMsgId = clientMsgId,
                            body = body,
                            createdAt = "2026-08-26T09:00:00Z",
                            attachments = listOf(
                                FakeAttachmentApi.attachment(id = 100, hasPreview = true),
                                FakeAttachmentApi.attachment(id = 101, hasPreview = true),
                                FakeAttachmentApi.attachment(id = 102, hasPreview = true),
                            ),
                            // The legacy spelling rides beside it: the FIRST
                            // element, which a plural-aware reader ignores.
                            attachment = FakeAttachmentApi.attachment(id = 100, hasPreview = true),
                        ),
                    ),
                )
            }
            val media = listOf(prepared(), prepared(), prepared())

            assertThat(repository.sendMedia(media, caption = "", chatId = CHAT)).isTrue()
            advanceUntilIdle()

            // Bytes then preview, per item, in the sender's order — then
            // ONE message claiming all three ids in that same order.
            assertThat(attachmentApi.calls)
                .containsExactly("upload", "preview", "upload", "preview", "upload", "preview")
                .inOrder()
            assertThat(chatApi.postedAttachmentIds).containsExactly(listOf(100L, 101L, 102L))
            assertThat(chatApi.postedMessages).hasSize(1)

            val row = messageDao.findByClientMsgId(chatApi.postedMessages.single().second)!!
            assertThat(row.attachmentList.map { it.id })
                .containsExactly(100L, 101L, 102L)
                .inOrder()
            // The legacy read stays the first element, so single-attachment
            // consumers (share, save, the context menu) keep their meaning.
            assertThat(row.attachment!!.id).isEqualTo(100L)
            media.forEach { assertThat(it.file.exists()).isFalse() }
        }

    @Test
    fun `a mid-way refused upload sends no message at all for the album`() =
        runTest(dispatcher) {
            insertChat()
            val repository = newRepository()
            var uploads = 0
            attachmentApi.uploadHandler = { _, _, _ ->
                uploads += 1
                if (uploads == 2) {
                    ApiResult.HttpError(413, "attachment_too_large", "too big")
                } else {
                    ApiResult.Ok(
                        AttachmentResponse(FakeAttachmentApi.attachment(id = uploads.toLong())),
                    )
                }
            }
            val media = listOf(prepared(), prepared(), prepared())

            assertThat(repository.sendMedia(media, caption = "", chatId = CHAT)).isFalse()
            advanceUntilIdle()

            // Nothing was written and nothing was claimed: the first
            // upload's bytes are the server's 24-hour sweep's business.
            assertThat(chatApi.postedMessages).isEmpty()
            // The third item was never attempted — the failure stops the walk.
            assertThat(attachmentApi.calls.count { it == "upload" }).isEqualTo(2)
            // The first item's bytes are on the server, so its file was
            // consumed; the failed item and the never-attempted tail KEEP
            // theirs — that is what the composer re-stages for retry.
            assertThat(media[0].file.exists()).isFalse()
            assertThat(media[1].file.exists()).isTrue()
            assertThat(media[2].file.exists()).isTrue()
        }

    /** The read rule on the arrival path: prefer `attachments`, ignore the legacy copy. */
    @Test
    fun `an inbound message prefers attachments over the legacy attachment`() =
        runTest(dispatcher) {
            insertChat()
            val repository = newRepository()

            repository.applyServerMessage(
                MessageDto(
                    id = 921,
                    chatId = CHAT,
                    senderId = 9,
                    clientMsgId = "theirs-album",
                    body = "",
                    createdAt = "2026-08-26T09:05:00Z",
                    attachments = listOf(
                        FakeAttachmentApi.attachment(id = 41),
                        FakeAttachmentApi.attachment(id = 42, kind = "video"),
                    ),
                    attachment = FakeAttachmentApi.attachment(id = 41),
                ),
                live = false,
            )
            advanceUntilIdle()

            val row = messageDao.findByServerId(921)!!
            assertThat(row.attachmentList.map { it.id }).containsExactly(41L, 42L).inOrder()
            // And the chat list counted the set rather than naming one item.
            assertThat(chatDao.getById(CHAT)!!.lastMessageBody).isEqualTo("2 attachments")
        }

    /** A legacy-only server (`attachment`, no `attachments`) still lands its one. */
    @Test
    fun `a legacy single attachment still reads through the fallback`() = runTest(dispatcher) {
        insertChat()
        val repository = newRepository()

        repository.applyServerMessage(
            MessageDto(
                id = 922,
                chatId = CHAT,
                senderId = 9,
                clientMsgId = "theirs-legacy",
                body = "",
                createdAt = "2026-08-26T09:06:00Z",
                attachment = FakeAttachmentApi.attachment(id = 55),
            ),
            live = false,
        )
        advanceUntilIdle()

        assertThat(messageDao.findByServerId(922)!!.attachmentList.map { it.id })
            .containsExactly(55L)
    }

    /**
     * The plural previews, per the push-summary convention: several of
     * one kind become a count, a mixed set the bare "N attachments", and
     * a caption still wins over everything.
     */
    @Test
    fun `the chat-list preview counts a set`() {
        fun a(kind: String, id: Long) = FakeAttachmentApi.attachment(id = id, kind = kind)

        assertThat(
            MessageRepository.previewText("", listOf(a("photo", 1), a("photo", 2), a("photo", 3))),
        ).isEqualTo("3 Photos")
        assertThat(MessageRepository.previewText("", listOf(a("video", 1), a("video", 2))))
            .isEqualTo("2 Videos")
        assertThat(MessageRepository.previewText("", listOf(a("audio", 1), a("audio", 2))))
            .isEqualTo("2 Audio")
        assertThat(
            MessageRepository.previewText(
                "",
                listOf(a("file", 1), a("file", 2), a("file", 3), a("file", 4)),
            ),
        ).isEqualTo("4 Files")
        // A mixed set gives its names up for the plain count.
        assertThat(MessageRepository.previewText("", listOf(a("photo", 1), a("video", 2))))
            .isEqualTo("2 attachments")
        // A caption still wins over any count.
        assertThat(
            MessageRepository.previewText("beach day", listOf(a("photo", 1), a("photo", 2))),
        ).isEqualTo("beach day")
    }
}
