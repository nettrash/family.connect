/*
 * RoundSendTest.kt
 * Family Connect (Android)
 *
 * Sending a VIDEO MESSAGE (#79, docs/protocol.md, "Video messages"; the
 * plan's "Where it plugs in → Android → Phase 2"): ONE square MP4, NO body,
 * and `round: true` on the send — otherwise an ordinary video, so it is
 * queued, retried and sent offline as one is, and it KEEPS its poster (a
 * sticker uploads none; a circle's square poster is what every reader draws
 * until it plays).
 *
 * Through the real MessageRepository and the real outbox, as StickerSendTest
 * goes: the thing under test is that the flag survives every leg — the
 * placeholder row, the upload, the frame, the REST fallback, a retry from a
 * second process and an upload that expired — and that a shape the server
 * would refuse never becomes a row. Nothing in Phase 2 sends one yet (the
 * recorder is Phase 3); this is the path it will use.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import com.google.common.truth.Truth.assertWithMessage
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
import me.nettrash.familyconnect.testutil.FakePosterCache
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.createTestDb
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
class RoundSendTest {

    private companion object {
        const val ME = 7L
        const val CHAT = 42L
        const val NOW = 1_000_000L
        val POSTER = ByteArray(64) { 9 }
    }

    private val dispatcher = StandardTestDispatcher()
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private lateinit var db: AppDatabase
    private lateinit var messageDao: MessageDao
    private lateinit var chatDao: ChatDao
    private lateinit var chatApi: FakeChatApi
    private lateinit var attachmentApi: FakeAttachmentApi
    private lateinit var socket: FakeChatSocket
    private lateinit var posters: FakePosterCache

    @Before
    fun setUp() {
        db = createTestDb(dispatcher)
        messageDao = db.messageDao()
        chatDao = db.chatDao()
        chatApi = FakeChatApi()
        attachmentApi = FakeAttachmentApi()
        socket = FakeChatSocket()
        posters = FakePosterCache()
        var nextUpload = 91L
        attachmentApi.uploadHandler = { file, mime, kind ->
            ApiResult.Ok(
                AttachmentResponse(
                    AttachmentDto(
                        id = nextUpload++, kind = kind, mime = mime, size = file.length(),
                        width = 480, height = 480, durationMs = 23_400,
                    ),
                ),
            )
        }
        ackAsRound()
    }

    @After
    fun tearDown() {
        repoScope.cancel()
        db.close()
    }

    /** A device that has heard a server with video messages (its discovery keys). */
    private val withRound = SettingsState(myUserId = ME, roundVideoMaxMs = 60_000, roundVideoMaxBytes = 12_582_912)

    private fun TestScope.newMessages(state: SettingsState = withRound): MessageRepository {
        val repository = MessageRepository(
            appContext = RuntimeEnvironment.getApplication(),
            chatApi = chatApi,
            attachmentApi = attachmentApi,
            messageDao = messageDao,
            chatDao = chatDao,
            socket = socket,
            settings = FakeSettingsRepository(state),
            chatRepository = testChatRepository(chatApi, chatDao, messageDao, socket, repoScope),
            posterCache = posters,
            scope = repoScope,
            clock = Clock { NOW },
            pendingAttachmentDao = db.pendingAttachmentDao(),
            staging = MediaStaging(RuntimeEnvironment.getApplication()),
        )
        runCurrent()
        return repository
    }

    private suspend fun insertChat() {
        chatDao.upsertAll(
            listOf(
                ChatEntity(
                    id = CHAT, kind = "family", peerUserId = null, title = "The Smiths", unreadCount = 0,
                    myLastReadId = null, peerLastReadId = null, lastMessageBody = null, lastMessageAt = null,
                    lastMessageSenderId = null,
                ),
            ),
        )
    }

    /** The server's answer to a round send: the video, flagged. */
    private fun ackAsRound() {
        chatApi.postMessageHandler = { chatId, clientMsgId, body ->
            ApiResult.Ok(
                MessageResponse(
                    MessageDto(
                        id = 900, chatId = chatId, senderId = ME, clientMsgId = clientMsgId, body = body,
                        createdAt = "2026-10-05T10:00:00Z",
                        attachments = listOf(
                            AttachmentDto(
                                id = chatApi.postedAttachmentIds.last()!!.single(),
                                kind = "video", mime = "video/mp4", size = 3, width = 480, height = 480,
                                durationMs = 23_400, hasPreview = true, round = true,
                            ),
                        ),
                    ),
                ),
            )
        }
    }

    /** What the recorder will hand over: a square MP4 with its poster. */
    private fun recorded(
        mime: String = "video/mp4",
        kind: String = AttachmentDto.KIND_VIDEO,
        width: Int? = 480,
        height: Int? = 480,
        durationMs: Int? = 23_400,
    ): MediaPrep.Prepared {
        val file = File.createTempFile(
            "fc-round", ".mp4",
            File(RuntimeEnvironment.getApplication().cacheDir, "uploads").apply { mkdirs() },
        ).apply { writeBytes(byteArrayOf(1, 2, 3)) }
        return MediaPrep.Prepared(file, mime, kind, width, height, durationMs, POSTER)
    }

    // -- One send ------------------------------------------------------------------

    @Test
    fun `a video message goes up with its poster and comes back flagged`() = runTest(dispatcher) {
        insertChat()
        val messages = newMessages()

        val clientMsgId = messages.sendMedia(listOf(recorded()), caption = "", chatId = CHAT, round = true)
        advanceUntilIdle()

        assertThat(clientMsgId).isNotNull()
        assertThat(attachmentApi.uploadedMetadata.single().first).isEqualTo("video")
        // KEPT, unlike a sticker's: the square poster is what a circle draws.
        assertThat(attachmentApi.calls).containsExactly("upload", "preview").inOrder()
        assertThat(attachmentApi.uploadedPreviews.single()).isEqualTo(91L to POSTER.size)
        // The send: no body, the one video, the flag — and not the sticker's.
        assertThat(chatApi.postedMessages.single().third).isEmpty()
        assertThat(chatApi.postedAttachmentIds.single()).containsExactly(91L)
        assertThat(chatApi.postedRoundFlags.single()).isTrue()
        assertThat(chatApi.postedStickerFlags.single()).isNull()

        val row = messageDao.findByClientMsgId(clientMsgId!!)!!
        assertThat(row.status).isEqualTo(MessageStatus.SENT)
        assertThat(row.attachmentList.single().isRound).isTrue()
        // The chat list says so (S5.7).
        assertThat(chatDao.getById(CHAT)?.lastMessageBody).isEqualTo("Video message")
    }

    @Test
    fun `the circle draws round before a byte has gone up`() = runTest(dispatcher) {
        insertChat()
        val messages = newMessages()
        attachmentApi.uploadHandler = { _, _, _ -> ApiResult.NetworkError(IllegalStateException("offline")) }

        messages.sendMedia(listOf(recorded()), caption = "", chatId = CHAT, round = true)
        runCurrent()

        val placeholder = messageDao.pendingSending().single().attachmentList.single()
        assertThat(placeholder.id).isLessThan(0)
        assertThat(placeholder.isRound).isTrue()
        // Its poster under the provisional id, so the sender's own circle
        // draws from the local file at once (S5.6).
        assertThat(posters.seeded).contains(placeholder.id to POSTER.size)
    }

    @Test
    fun `a video message is a reply when one is being written`() = runTest(dispatcher) {
        insertChat()
        val messages = newMessages()

        messages.sendMedia(
            listOf(recorded()), caption = "", chatId = CHAT,
            replyTo = ReplyToDto(messageId = 41, senderId = 9, excerpt = "Where are you?"), round = true,
        )
        advanceUntilIdle()

        assertThat(chatApi.postedReplyTargets.single()).isEqualTo(41)
        assertThat(chatApi.postedRoundFlags.single()).isTrue()
    }

    @Test
    fun `an open socket carries the flag in the send frame`() = runTest(dispatcher) {
        insertChat()
        val messages = newMessages()
        socket.setOpen(true)
        runCurrent()

        messages.sendMedia(listOf(recorded()), caption = "", chatId = CHAT, round = true)
        runCurrent()

        val frame = socket.sent.filterIsInstance<ClientFrame.Send>().single()
        assertThat(frame.round).isTrue()
        assertThat(frame.sticker).isNull()
        assertThat(frame.body).isEmpty()
        assertThat(frame.attachmentIds).containsExactly(91L)
    }

    @Test
    fun `an ordinary video still sends no flag`() = runTest(dispatcher) {
        insertChat()
        val messages = newMessages()
        socket.setOpen(true)
        runCurrent()

        messages.sendMedia(listOf(recorded()), caption = "", chatId = CHAT)
        runCurrent()

        // Null, not false: the key is omitted on the frame…
        assertThat(socket.sent.filterIsInstance<ClientFrame.Send>().single().round).isNull()
        // …and on the REST request the ack timeout falls back to.
        advanceUntilIdle()
        assertThat(chatApi.postedRoundFlags).containsExactly(null)
        assertThat(messageDao.pendingSending()).isEmpty()
    }

    // -- What the server would refuse never becomes a row -------------------------------

    @Test
    fun `a shape the server refuses is not sent at all`() = runTest(dispatcher) {
        insertChat()
        val messages = newMessages()
        val refused = mapOf(
            "words" to (listOf(recorded()) to "look"),
            "two videos" to (listOf(recorded(), recorded()) to ""),
            "a photo" to (listOf(recorded(mime = "image/jpeg", kind = AttachmentDto.KIND_PHOTO)) to ""),
            "an audio" to (listOf(recorded(mime = "audio/mp4", kind = AttachmentDto.KIND_AUDIO)) to ""),
            "QuickTime" to (listOf(recorded(mime = "video/quicktime")) to ""),
            "not square" to (listOf(recorded(width = 480, height = 640)) to ""),
            "over 720" to (listOf(recorded(width = 721, height = 721)) to ""),
            "no size" to (listOf(recorded(width = null)) to ""),
            "no length" to (listOf(recorded(durationMs = null)) to ""),
            "zero length" to (listOf(recorded(durationMs = 0)) to ""),
            "over a minute" to (listOf(recorded(durationMs = 60_001)) to ""),
        )
        for ((shape, send) in refused) {
            val (items, caption) = send
            assertWithMessage(shape)
                .that(messages.sendMedia(items, caption = caption, chatId = CHAT, round = true))
                .isNull()
        }
        // Never beside the sticker flag (`validation`).
        assertWithMessage("with sticker")
            .that(messages.sendMedia(listOf(recorded()), caption = "", chatId = CHAT, sticker = true, round = true))
            .isNull()
        advanceUntilIdle()

        assertThat(messageDao.pendingSending()).isEmpty()
        assertThat(chatApi.postedMessages).isEmpty()
        assertThat(attachmentApi.calls).isEmpty()
    }

    @Test
    fun `the limits themselves are accepted`() {
        // A minute exactly and a 720-pixel square are inside (the server's
        // `> 60000` and `> 720`); whitespace is no words, judged as it is sent.
        val edge = MediaPrep.Prepared(File("x"), "video/mp4", AttachmentDto.KIND_VIDEO, 720, 720, 60_000, null)
        val any = Long.MAX_VALUE
        assertThat(RoundSend.accepts(listOf(edge), caption = "  \n", sticker = false, maxBytes = any)).isTrue()
        assertThat(RoundSend.accepts(listOf(edge.copy(width = 1, height = 1, durationMs = 1)), "", false, any)).isTrue()
        assertThat(RoundSend.accepts(listOf(edge.copy(width = 0, height = 0)), "", false, any)).isFalse()
        assertThat(RoundSend.accepts(emptyList(), "", false, any)).isFalse()
    }

    /**
     * `max_round_video_bytes` (#79, Phase 3): a clip over the ceiling this
     * device has heard is never sent as a video message — the recorder sends
     * it as a regular video after saying so (S3.6); this is the backstop.
     */
    @Test
    fun `over the ceiling the server announced is not sent as a video message`() = runTest(dispatcher) {
        insertChat()
        // recorded() writes three bytes.
        val messages = newMessages(SettingsState(myUserId = ME, roundVideoMaxMs = 60_000, roundVideoMaxBytes = 2))
        assertThat(messages.sendMedia(listOf(recorded()), caption = "", chatId = CHAT, round = true)).isNull()
        advanceUntilIdle()
        assertThat(messageDao.pendingSending()).isEmpty()
        assertThat(chatApi.postedMessages).isEmpty()

        // As a regular video it goes.
        assertThat(messages.sendMedia(listOf(recorded()), caption = "", chatId = CHAT)).isNotNull()
    }

    @Test
    fun `exactly the ceiling is a video message`() = runTest(dispatcher) {
        insertChat()
        val messages = newMessages(SettingsState(myUserId = ME, roundVideoMaxMs = 60_000, roundVideoMaxBytes = 3))
        assertThat(messages.sendMedia(listOf(recorded()), caption = "", chatId = CHAT, round = true)).isNotNull()
        advanceUntilIdle()
        assertThat(chatApi.postedRoundFlags).containsExactly(true)
    }

    /**
     * No ceiling heard is no video message at all: the keys' absence is a
     * server without video messages, which would IGNORE `round` and deliver
     * a square video (docs/protocol.md, "Video messages") — "a client must
     * not send the flag without the discovery keys".
     */
    @Test
    fun `the size check needs a ceiling and only then applies`() {
        val clip = File.createTempFile("fc-round", ".mp4").apply {
            writeBytes(ByteArray(10))
            deleteOnExit()
        }
        val prepared = MediaPrep.Prepared(clip, "video/mp4", AttachmentDto.KIND_VIDEO, 480, 480, 23_400, null)
        assertThat(RoundSend.accepts(listOf(prepared), "", false, maxBytes = null)).isFalse()
        assertThat(RoundSend.accepts(listOf(prepared), "", false, maxBytes = 10)).isTrue()
        assertThat(RoundSend.accepts(listOf(prepared), "", false, maxBytes = 9)).isFalse()
    }

    @Test
    fun `without the discovery keys no video message is sent`() = runTest(dispatcher) {
        insertChat()
        val messages = newMessages(SettingsState(myUserId = ME))

        assertThat(messages.sendMedia(listOf(recorded()), caption = "", chatId = CHAT, round = true)).isNull()
        advanceUntilIdle()
        assertThat(messageDao.pendingSending()).isEmpty()
        assertThat(chatApi.postedMessages).isEmpty()
        assertThat(attachmentApi.calls).isEmpty()
    }

    /**
     * Queued with the keys, retried after they went (a server rolled back,
     * a family moved): the row is already there, so it goes — WITHOUT the
     * flag, the ordinary video such a server would have made of it anyway.
     */
    @Test
    fun `a retry after the keys went sends no flag`() = runTest(dispatcher) {
        insertChat()
        chatApi.postMessageHandler = { _, _, _ -> ApiResult.HttpError(503, null, "busy") }
        newMessages().sendMedia(listOf(recorded()), caption = "", chatId = CHAT, round = true)
        advanceUntilIdle()
        assertThat(chatApi.postedRoundFlags.toSet()).containsExactly(true)
        val clientMsgId = messageDao.pendingSending().single().clientMsgId

        newMessages(SettingsState(myUserId = ME)).retry(clientMsgId)
        advanceUntilIdle()

        assertThat(chatApi.postedRoundFlags.last()).isNull()
    }

    /** The same through the upload leg: queued offline with the keys, its bytes go up after they went. */
    @Test
    fun `an upload that finishes after the keys went sends no flag`() = runTest(dispatcher) {
        insertChat()
        val ok = attachmentApi.uploadHandler
        attachmentApi.uploadHandler = { _, _, _ -> ApiResult.NetworkError(IllegalStateException("offline")) }
        newMessages().sendMedia(listOf(recorded()), caption = "", chatId = CHAT, round = true)
        advanceUntilIdle()
        val clientMsgId = messageDao.pendingSending().single().clientMsgId
        assertThat(chatApi.postedRoundFlags).isEmpty()

        attachmentApi.uploadHandler = ok
        newMessages(SettingsState(myUserId = ME)).uploadPending(clientMsgId)
        advanceUntilIdle()

        assertThat(chatApi.postedRoundFlags).containsExactly(null)
    }

    @Test
    fun `an outbox flush after the keys went sends no flag`() = runTest(dispatcher) {
        insertChat()
        chatApi.postMessageHandler = { _, _, _ -> ApiResult.HttpError(503, null, "busy") }
        newMessages().sendMedia(listOf(recorded()), caption = "", chatId = CHAT, round = true)
        advanceUntilIdle()
        val queued = messageDao.pendingSending().single()
        val before = chatApi.postedRoundFlags.size

        val later = newMessages(SettingsState(myUserId = ME))
        messageDao.resetSendBudget(queued.clientMsgId)
        later.flushPending()
        advanceUntilIdle()

        assertThat(chatApi.postedRoundFlags.size).isGreaterThan(before)
        assertThat(chatApi.postedRoundFlags.drop(before).toSet()).containsExactly(null)
    }

    // -- Retries --------------------------------------------------------------------------

    @Test
    fun `a retry from a second process is still a video message`() = runTest(dispatcher) {
        insertChat()
        chatApi.postMessageHandler = { _, _, _ -> ApiResult.HttpError(503, null, "busy") }
        newMessages().sendMedia(listOf(recorded()), caption = "", chatId = CHAT, round = true)
        advanceUntilIdle()
        val clientMsgId = messageDao.pendingSending().single().clientMsgId

        // "The process died": a new repository, which knows only the ROW.
        ackAsRound()
        newMessages().retry(clientMsgId)
        advanceUntilIdle()

        assertThat(chatApi.postedRoundFlags.last()).isTrue()
        assertThat(messageDao.findByClientMsgId(clientMsgId)?.attachmentList?.single()?.isRound).isTrue()
    }

    @Test
    fun `the outbox flush carries the flag too`() = runTest(dispatcher) {
        insertChat()
        chatApi.postMessageHandler = { _, _, _ -> ApiResult.HttpError(503, null, "busy") }
        val messages = newMessages()
        messages.sendMedia(listOf(recorded()), caption = "", chatId = CHAT, round = true)
        advanceUntilIdle()
        val queued = messageDao.pendingSending().single()

        ackAsRound()
        messageDao.resetSendBudget(queued.clientMsgId)
        messages.flushPending()
        advanceUntilIdle()

        assertThat(chatApi.postedRoundFlags).isNotEmpty()
        assertThat(chatApi.postedRoundFlags.toSet()).containsExactly(true)
        assertThat(messageDao.findByClientMsgId(queued.clientMsgId)?.status).isEqualTo(MessageStatus.SENT)
    }

    @Test
    fun `an upload that expired in the outbox goes up again as a video message, poster and all`() =
        runTest(dispatcher) {
            insertChat()
            var posts = 0
            val ack = chatApi.postMessageHandler
            chatApi.postMessageHandler = { chatId, id, body ->
                posts++
                if (posts == 1) ApiResult.HttpError(404, "attachment_expired", "gone") else ack(chatId, id, body)
            }

            newMessages().sendMedia(listOf(recorded()), caption = "", chatId = CHAT, round = true)
            advanceUntilIdle()

            assertThat(attachmentApi.calls).containsExactly("upload", "preview", "upload", "preview").inOrder()
            assertThat(chatApi.postedRoundFlags).containsExactly(true, true)
        }

    // -- Receiving -------------------------------------------------------------------------

    @Test
    fun `an inbound video message keeps its flag and previews as the words`() = runTest(dispatcher) {
        insertChat()
        val messages = newMessages()

        messages.applyServerMessage(
            MessageDto(
                id = 901, chatId = CHAT, senderId = 9, clientMsgId = "theirs", body = "",
                createdAt = "2026-10-05T10:05:00Z",
                attachments = listOf(
                    AttachmentDto(
                        id = 77, kind = "video", mime = "video/mp4", size = 900, width = 480, height = 480,
                        durationMs = 5_000, hasPreview = true, round = true,
                    ),
                ),
            ),
            live = true,
        )
        advanceUntilIdle()

        assertThat(messageDao.findByServerId(901)!!.attachmentList.single().isRound).isTrue()
        assertThat(chatDao.getById(CHAT)?.lastMessageBody).isEqualTo("Video message")
    }

    @Test
    fun `the preview checks a video message before a video`() {
        val round = AttachmentDto(id = 1, kind = "video", mime = "video/mp4", size = 1, round = true)
        val video = round.copy(id = 2, round = null)
        val strayOnAPhoto = AttachmentDto(id = 3, kind = "photo", mime = "image/jpeg", size = 1, round = true)

        assertThat(MessageRepository.previewText("", listOf(round))).isEqualTo("Video message")
        assertThat(MessageRepository.previewText("", listOf(video))).isEqualTo("Video")
        assertThat(MessageRepository.previewText("", listOf(strayOnAPhoto))).isEqualTo("Photo")
        // Two of them are two videos — the count, as before.
        assertThat(MessageRepository.previewText("", listOf(round, video))).isEqualTo("2 Videos")
    }
}
