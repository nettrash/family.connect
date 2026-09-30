/*
 * StickerSendTest.kt
 * Family Connect (Android)
 *
 * Sending a chat sticker (docs/protocol.md, "Sticker pack" → "Sending
 * one"): a message with ONE attachment and ONE flag, whose bytes are the
 * pack item's own — unprepared, with no preview — and which is otherwise
 * an ordinary message, so it is queued, retried and sent offline exactly as
 * a photo is.
 *
 * Everything here goes through the real MessageRepository and the real
 * outbox. The thing under test is that the FLAG survives every leg of that
 * road: the placeholder row, the upload, the socket frame, the REST
 * fallback, a retry from a second process, and an upload that expired.
 *
 * iOS counterpart: the sticker send tests in ios/FamilyConnectTests.
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
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.net.dto.AttachmentResponse
import me.nettrash.familyconnect.data.net.dto.MessageDto
import me.nettrash.familyconnect.data.net.dto.MessageResponse
import me.nettrash.familyconnect.data.net.dto.ReplyToDto
import me.nettrash.familyconnect.data.net.ws.ClientFrame
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.testutil.FakeAttachmentApi
import me.nettrash.familyconnect.testutil.FakeChatApi
import me.nettrash.familyconnect.testutil.FakeChatSocket
import me.nettrash.familyconnect.testutil.FakePackApi
import me.nettrash.familyconnect.testutil.FakePosterCache
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.createTestDb
import me.nettrash.familyconnect.testutil.packItemDto
import me.nettrash.familyconnect.testutil.packTombstone
import me.nettrash.familyconnect.testutil.testChatRepository
import me.nettrash.familyconnect.util.Clock
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import java.io.File

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class StickerSendTest {

    private companion object {
        const val ME = 7L
        const val CHAT = 42L
        const val NOW = 1_000_000L

        /** The pack item's bytes — an animated WebP as far as its header goes. */
        val STICKER: ByteArray = ByteArray(300) { 5 }.also { bytes ->
            "RIFF".toByteArray().copyInto(bytes, 0)
            "WEBP".toByteArray().copyInto(bytes, 8)
            "VP8X".toByteArray().copyInto(bytes, 12)
            bytes[20] = 0x12
        }
    }

    /** Remembers the ORIGINAL bytes the send path hands over, by id. */
    private class RecordingPosterCache : PosterCache {
        val originals = mutableMapOf<Long, ByteArray>()
        val posters = mutableListOf<Long>()

        /** How many rows the chat held each time the original bytes arrived. */
        val rowsWhenSeeded = mutableListOf<Int>()
        var countRows: suspend () -> Int = { 0 }

        override suspend fun seedPoster(attachmentId: Long, jpeg: ByteArray) {
            posters += attachmentId
        }

        override suspend fun notePosterUpload(attachmentId: Long, landed: Boolean) = Unit

        override suspend fun seedOriginal(attachmentId: Long, source: File) {
            rowsWhenSeeded += countRows()
            originals[attachmentId] = source.readBytes()
        }
    }

    private val dispatcher = StandardTestDispatcher()
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private lateinit var db: AppDatabase
    private lateinit var messageDao: MessageDao
    private lateinit var chatDao: ChatDao
    private lateinit var chatApi: FakeChatApi
    private lateinit var attachmentApi: FakeAttachmentApi
    private lateinit var socket: FakeChatSocket
    private lateinit var settings: FakeSettingsRepository
    private val posterCache = RecordingPosterCache()

    /** What each upload actually carried: (bytes, media type). */
    private val uploads = mutableListOf<Pair<ByteArray, String>>()

    @Before
    fun setUp() {
        db = createTestDb(dispatcher)
        messageDao = db.messageDao()
        chatDao = db.chatDao()
        chatApi = FakeChatApi()
        attachmentApi = FakeAttachmentApi()
        socket = FakeChatSocket()
        settings = FakeSettingsRepository(
            SettingsState(
                serverUrl = "https://chat.example.com",
                familyStatus = FamilyStatus.MEMBER,
                myUserId = ME,
                packMaxItems = 200,
                packMaxItemBytes = 524_288,
            ),
        )
        PackRepository.bytesDirectory(RuntimeEnvironment.getApplication()).deleteRecursively()
        var nextUpload = 90L
        attachmentApi.uploadHandler = { file, mime, _ ->
            uploads += file.readBytes() to mime
            ApiResult.Ok(
                AttachmentResponse(
                    AttachmentDto(
                        id = nextUpload++, kind = "photo", mime = mime, size = file.length(),
                        width = 512, height = 512,
                    ),
                ),
            )
        }
        // The pack's bytes, as the server would hand them out.
        attachmentApi.downloadHandler = { _, _, destination ->
            destination.parentFile?.mkdirs()
            destination.writeBytes(STICKER)
            ApiResult.Ok(Unit)
        }
        ackAsSticker()
    }

    @After
    fun tearDown() {
        repoScope.cancel()
        db.close()
    }

    private fun TestScope.newMessages(cache: PosterCache = posterCache): MessageRepository {
        val repository = MessageRepository(
            appContext = RuntimeEnvironment.getApplication(),
            chatApi = chatApi,
            attachmentApi = attachmentApi,
            messageDao = messageDao,
            chatDao = chatDao,
            socket = socket,
            settings = settings,
            chatRepository = testChatRepository(chatApi, chatDao, messageDao, socket, repoScope),
            posterCache = cache,
            scope = repoScope,
            clock = Clock { NOW },
            pendingAttachmentDao = db.pendingAttachmentDao(),
            staging = MediaStaging(RuntimeEnvironment.getApplication()),
        )
        runCurrent()
        return repository
    }

    private fun newPack() = PackRepository(
        context = RuntimeEnvironment.getApplication(),
        packApi = FakePackApi(),
        attachmentApi = attachmentApi,
        packDao = db.packDao(),
        settings = settings,
        socket = socket,
        scope = repoScope,
    )

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

    /** The server's answer to a sticker send: the attachment, flagged. */
    private fun ackAsSticker() {
        chatApi.postMessageHandler = { chatId, clientMsgId, body ->
            ApiResult.Ok(
                MessageResponse(
                    MessageDto(
                        id = 900,
                        chatId = chatId,
                        senderId = ME,
                        clientMsgId = clientMsgId,
                        body = body,
                        createdAt = "2026-09-13T10:00:00Z",
                        attachments = listOf(
                            AttachmentDto(
                                id = chatApi.postedAttachmentIds.last()!!.single(),
                                kind = "photo", mime = "image/webp", size = STICKER.size.toLong(),
                                width = 512, height = 512, sticker = true,
                            ),
                        ),
                    ),
                ),
            )
        }
    }

    /** A pack holding one sticker, and the thing that sends from it. */
    private suspend fun TestScope.senderWithOneSticker(): Pair<StickerSender, PackRepository> {
        insertChat()
        val pack = newPack()
        pack.applyItem(packItemDto(id = 5, packSeq = 12, attachmentId = 71, size = STICKER.size.toLong()))
        return StickerSender(pack, newMessages(), settings) to pack
    }

    private suspend fun theItem() = db.packDao().findById(5)!!

    // -- One tap ------------------------------------------------------------------

    @Test
    fun `one tap sends the pack item's own bytes as a sticker`() = runTest(dispatcher) {
        val (sender, _) = senderWithOneSticker()

        assertThat(sender.send(theItem(), CHAT)).isEqualTo(StickerSender.Result.QUEUED)
        advanceUntilIdle()

        // THE BYPASS: what went up is byte for byte what the pack holds —
        // an animated WebP, not a 2048-pixel JPEG — so the message's upload
        // hashes to the pack item's file.
        assertThat(uploads).hasSize(1)
        assertThat(uploads.single().first).isEqualTo(STICKER)
        assertThat(uploads.single().second).isEqualTo("image/webp")
        assertThat(attachmentApi.uploadedMetadata.single().first).isEqualTo("photo")
        // And NO preview went up: a preview is a JPEG.
        assertThat(attachmentApi.calls).doesNotContain("preview")

        // The send: no body, exactly one attachment, and the flag.
        assertThat(chatApi.postedMessages.single().third).isEmpty()
        assertThat(chatApi.postedAttachmentIds.single()).containsExactly(90L)
        assertThat(chatApi.postedStickerFlags.single()).isTrue()

        val row = messageDao.findByClientMsgId(chatApi.postedMessages.single().second)!!
        assertThat(row.status).isEqualTo(MessageStatus.SENT)
        assertThat(row.serverId).isEqualTo(900)
        assertThat(row.attachmentList.single().isSticker).isTrue()
        // The chat list says so in one word.
        assertThat(chatDao.getById(CHAT)?.lastMessageBody).isEqualTo("Sticker")
    }

    @Test
    fun `the bubble has its picture before a byte has gone up`() = runTest(dispatcher) {
        val (sender, _) = senderWithOneSticker()
        // A network that never answers, so the send stays queued.
        attachmentApi.uploadHandler = { _, _, _ -> ApiResult.NetworkError(IllegalStateException("offline")) }

        assertThat(sender.send(theItem(), CHAT)).isEqualTo(StickerSender.Result.QUEUED)
        runCurrent()

        val row = messageDao.pendingSending().single()
        val placeholder = row.attachmentList.single()
        // A provisional id the server has never heard of, already flagged,
        // so the row draws bare from its first frame.
        assertThat(placeholder.id).isLessThan(0)
        assertThat(placeholder.isSticker).isTrue()
        assertThat(placeholder.mime).isEqualTo("image/webp")
        // The ORIGINAL bytes under that id — a sticker has no preview to
        // draw from, by design.
        assertThat(posterCache.originals[placeholder.id]).isEqualTo(STICKER)
        assertThat(posterCache.posters).isEmpty()
    }

    @Test
    fun `the bytes are in place before the row that draws them exists`() = runTest(dispatcher) {
        val (sender, _) = senderWithOneSticker()
        attachmentApi.uploadHandler = { _, _, _ -> ApiResult.NetworkError(IllegalStateException("offline")) }
        posterCache.countRows = { messageDao.pendingSending().size }

        sender.send(theItem(), CHAT)
        runCurrent()

        // The bubble asks for its file ONCE, when it composes, and nothing
        // tells it to ask again: a row inserted ahead of its bytes is a
        // grey square until the upload lands — and offline it does not.
        assertThat(posterCache.rowsWhenSeeded).containsExactly(0)
        assertThat(messageDao.pendingSending()).hasSize(1)
    }

    @Test
    fun `a sticker is a reply when one is being written`() = runTest(dispatcher) {
        val (sender, _) = senderWithOneSticker()

        sender.send(theItem(), CHAT, ReplyToDto(messageId = 41, senderId = 9, excerpt = "See you at six"))
        advanceUntilIdle()

        assertThat(chatApi.postedReplyTargets.single()).isEqualTo(41)
        assertThat(chatApi.postedStickerFlags.single()).isTrue()
    }

    @Test
    fun `an open socket carries the flag in the send frame`() = runTest(dispatcher) {
        val (sender, _) = senderWithOneSticker()
        socket.setOpen(true)
        runCurrent()

        sender.send(theItem(), CHAT)
        runCurrent()

        val frame = socket.sent.filterIsInstance<ClientFrame.Send>().single()
        assertThat(frame.sticker).isTrue()
        assertThat(frame.body).isEmpty()
        assertThat(frame.attachmentIds).containsExactly(90L)
    }

    @Test
    fun `an ordinary photo still sends no flag`() = runTest(dispatcher) {
        insertChat()
        val messages = newMessages()
        val file = File.createTempFile(
            "fc-upload", ".jpg",
            File(RuntimeEnvironment.getApplication().cacheDir, "uploads").apply { mkdirs() },
        ).apply { writeBytes(byteArrayOf(1, 2, 3)) }

        messages.sendMedia(
            listOf(MediaPrep.Prepared(file, "image/jpeg", "photo", 1600, 1200, null, ByteArray(8))),
            caption = "",
            chatId = CHAT,
        )
        advanceUntilIdle()

        // Null, not false: the key is omitted, and the request is byte for
        // byte what it was before stickers existed.
        assertThat(chatApi.postedStickerFlags.single()).isNull()
        assertThat(attachmentApi.calls).contains("preview")
    }

    @Test
    fun `a sticker with a caption or a second picture is not sent at all`() = runTest(dispatcher) {
        val (_, pack) = senderWithOneSticker()
        val messages = newMessages()
        val one = pack.stagedCopy(theItem())!!
        val two = pack.stagedCopy(theItem())!!

        // The server refuses both shapes (`validation`, `invalid_attachment`);
        // neither gets as far as a row.
        assertThat(messages.sendMedia(listOf(one), caption = "look", chatId = CHAT, sticker = true)).isNull()
        assertThat(messages.sendMedia(listOf(one, two), caption = "", chatId = CHAT, sticker = true)).isNull()
        advanceUntilIdle()

        assertThat(messageDao.pendingSending()).isEmpty()
        assertThat(chatApi.postedMessages).isEmpty()
        one.file.delete()
        two.file.delete()
    }

    // -- Offline, and the outbox ------------------------------------------------------

    @Test
    fun `a sticker tapped with no network is queued and goes when it returns`() = runTest(dispatcher) {
        val (sender, _) = senderWithOneSticker()
        val messages = newMessages()
        // The bytes are already on this phone (the resync fetched them).
        newPack().fileFor(theItem().attachment!!)
        attachmentApi.downloadHandler = { _, _, _ -> ApiResult.NetworkError(IllegalStateException("offline")) }
        var online = false
        val answer = attachmentApi.uploadHandler
        attachmentApi.uploadHandler = { file, mime, kind ->
            if (online) answer(file, mime, kind) else ApiResult.NetworkError(IllegalStateException("offline"))
        }

        // Offline: it is QUEUED — a bubble, not an error.
        assertThat(sender.send(theItem(), CHAT)).isEqualTo(StickerSender.Result.QUEUED)
        runCurrent()
        val queued = messageDao.pendingSending().single()
        assertThat(queued.status).isEqualTo(MessageStatus.SENDING)
        assertThat(chatApi.postedMessages).isEmpty()

        // The network returns and the ordinary outbox finishes the job.
        online = true
        messageDao.resetSendBudget(queued.clientMsgId)
        messages.flushPending()
        advanceUntilIdle()

        assertThat(chatApi.postedStickerFlags.last()).isTrue()
        assertThat(uploads.last().first).isEqualTo(STICKER)
        assertThat(messageDao.findByClientMsgId(queued.clientMsgId)?.status).isEqualTo(MessageStatus.SENT)
    }

    @Test
    fun `a sticker whose item was removed before it left still goes`() = runTest(dispatcher) {
        val (sender, pack) = senderWithOneSticker()
        var online = false
        val answer = attachmentApi.uploadHandler
        attachmentApi.uploadHandler = { file, mime, kind ->
            if (online) answer(file, mime, kind) else ApiResult.NetworkError(IllegalStateException("offline"))
        }
        sender.send(theItem(), CHAT)
        runCurrent()
        val queued = messageDao.pendingSending().single()

        // Somebody removes the item while the send waits in the outbox.
        // The message is a COPY with its own bytes; the server does not
        // check the pack, and neither does the outbox.
        pack.applyItem(packTombstone(id = 5, packSeq = 14))
        assertThat(db.packDao().findById(5)).isNull()

        online = true
        messageDao.resetSendBudget(queued.clientMsgId)
        newMessages().flushPending()
        advanceUntilIdle()

        assertThat(uploads.last().first).isEqualTo(STICKER)
        assertThat(chatApi.postedStickerFlags.last()).isTrue()
    }

    @Test
    fun `a retry from a second process is still a sticker`() = runTest(dispatcher) {
        val (sender, _) = senderWithOneSticker()
        // The message request is refused transiently, so the row stays
        // SENDING with its real attachment id and its flag.
        chatApi.postMessageHandler = { _, _, _ -> ApiResult.HttpError(503, null, "busy") }
        sender.send(theItem(), CHAT)
        runCurrent()
        val clientMsgId = messageDao.pendingSending().single().clientMsgId

        // "The process died": a new repository, which knows nothing but
        // what the ROW says.
        ackAsSticker()
        val reborn = newMessages(FakePosterCache())
        reborn.retry(clientMsgId)
        advanceUntilIdle()

        assertThat(chatApi.postedStickerFlags.last()).isTrue()
        assertThat(messageDao.findByClientMsgId(clientMsgId)?.attachmentList?.single()?.isSticker).isTrue()
    }

    @Test
    fun `an upload that expired in the outbox goes up again as a sticker`() = runTest(dispatcher) {
        val (sender, _) = senderWithOneSticker()
        var posts = 0
        val ack = chatApi.postMessageHandler
        chatApi.postMessageHandler = { chatId, id, body ->
            posts++
            if (posts == 1) ApiResult.HttpError(404, "attachment_expired", "gone") else ack(chatId, id, body)
        }

        sender.send(theItem(), CHAT)
        advanceUntilIdle()

        // The dead id was dropped, the SAME bytes went up again, and the
        // second claim is still flagged — not a photo this time round.
        assertThat(uploads).hasSize(2)
        assertThat(uploads.last().first).isEqualTo(STICKER)
        assertThat(chatApi.postedStickerFlags).containsExactly(true, true)
        assertThat(attachmentApi.calls).doesNotContain("preview")
    }

    @Test
    fun `a sticker whose bytes are not on this phone cannot be sent offline`() = runTest(dispatcher) {
        val (sender, _) = senderWithOneSticker()
        attachmentApi.downloadHandler = { _, _, _ -> ApiResult.NetworkError(IllegalStateException("offline")) }

        // The one case there is nothing to queue: no bytes, no network to
        // fetch them. Said to the person rather than drawn as a bubble.
        assertThat(sender.send(theItem(), CHAT)).isEqualTo(StickerSender.Result.NO_BYTES)
        runCurrent()

        assertThat(messageDao.pendingSending()).isEmpty()
        assertThat(settings.current.packRecents).isEmpty()
    }

    @Test
    fun `a sticker over a lowered ceiling is said rather than sent`() = runTest(dispatcher) {
        val (sender, _) = senderWithOneSticker()
        // The operator lowered the per-item ceiling under an item the pack
        // already holds. The ceiling binds a sticker MESSAGE too.
        settings.setPackLimits(200, STICKER.size - 1L)

        assertThat(sender.send(theItem(), CHAT)).isEqualTo(StickerSender.Result.TOO_LARGE)
        runCurrent()

        assertThat(messageDao.pendingSending()).isEmpty()
        assertThat(uploads).isEmpty()
    }

    @Test
    fun `sending remembers the sticker on this device only`() = runTest(dispatcher) {
        val (sender, _) = senderWithOneSticker()

        sender.send(theItem(), CHAT)
        advanceUntilIdle()

        assertThat(settings.current.packRecents).containsExactly(5L)
        // Nothing about it on the wire: the send names no pack item.
        assertThat(chatApi.postedAttachmentIds.single()).containsExactly(90L)
    }

    // -- Receiving ----------------------------------------------------------------------

    @Test
    fun `an inbound sticker keeps its flag and previews as the word`() = runTest(dispatcher) {
        insertChat()
        val messages = newMessages()

        messages.applyServerMessage(
            MessageDto(
                id = 901, chatId = CHAT, senderId = 9, clientMsgId = "theirs", body = "",
                createdAt = "2026-09-13T10:05:00Z",
                attachments = listOf(
                    AttachmentDto(id = 77, kind = "photo", mime = "image/png", size = 900, sticker = true),
                ),
            ),
            live = true,
        )
        advanceUntilIdle()

        assertThat(messageDao.findByServerId(901)!!.attachmentList.single().isSticker).isTrue()
        assertThat(chatDao.getById(CHAT)?.lastMessageBody).isEqualTo("Sticker")
    }

    @Test
    fun `the preview says Sticker for a sticker and Photo for a photo`() {
        val sticker = AttachmentDto(id = 1, kind = "photo", mime = "image/webp", size = 1, sticker = true)
        val photo = AttachmentDto(id = 2, kind = "photo", mime = "image/webp", size = 1)

        assertThat(MessageRepository.previewText("", listOf(sticker))).isEqualTo("Sticker")
        assertThat(MessageRepository.previewText("", listOf(photo))).isEqualTo("Photo")
    }
}
