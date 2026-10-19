/*
 * AnimationSendTest.kt
 * Family Connect (Android)
 *
 * Sending an ANIMATION (#96, docs/protocol.md, "Animations" → "Sending
 * one"): ONE `video/mp4` with its size and length, `animation: true` on the
 * send — over the socket and over REST — and otherwise an ordinary video: its
 * poster goes up, it may carry a caption, and it is queued, retried and sent
 * offline as one is. Through the real MessageRepository and outbox, as
 * RoundSendTest goes: the flag has to survive the placeholder row, the
 * upload, the frame, the REST fallback, a retry from a second process and an
 * upload that expired, and a shape the server would refuse never becomes a
 * row. And the composer's split ([AnimationSend.groups]): an animation is a
 * message of its own.
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
class AnimationSendTest {

    private companion object {
        const val ME = 7L
        const val CHAT = 42L
        const val NOW = 1_000_000L
        val POSTER = ByteArray(48) { 5 }
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
        var nextUpload = 93L
        attachmentApi.uploadHandler = { file, mime, kind ->
            ApiResult.Ok(
                AttachmentResponse(
                    AttachmentDto(
                        id = nextUpload++, kind = kind, mime = mime, size = file.length(),
                        width = 480, height = 270, durationMs = 2_100,
                    ),
                ),
            )
        }
        chatApi.postMessageHandler = { chatId, clientMsgId, body ->
            ApiResult.Ok(
                MessageResponse(
                    MessageDto(
                        id = 900, chatId = chatId, senderId = ME, clientMsgId = clientMsgId, body = body,
                        createdAt = "2026-10-19T10:00:00Z",
                        attachments = listOf(
                            AttachmentDto(
                                id = chatApi.postedAttachmentIds.last()!!.single(),
                                kind = "video", mime = "video/mp4", size = 3, width = 480, height = 270,
                                durationMs = 2_100, hasPreview = true, animation = true,
                            ),
                        ),
                    ),
                ),
            )
        }
    }

    @After
    fun tearDown() {
        repoScope.cancel()
        db.close()
    }

    private fun TestScope.newMessages(): MessageRepository {
        val repository = MessageRepository(
            appContext = RuntimeEnvironment.getApplication(),
            chatApi = chatApi,
            attachmentApi = attachmentApi,
            messageDao = messageDao,
            chatDao = chatDao,
            socket = socket,
            settings = FakeSettingsRepository(SettingsState(myUserId = ME)),
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

    /** What MediaPrep hands over for an animated GIF: the MP4, its size, its length and its first frame. */
    private fun made(
        mime: String = "video/mp4",
        kind: String = AttachmentDto.KIND_VIDEO,
        width: Int? = 480,
        height: Int? = 270,
        durationMs: Int? = 2_100,
        animation: Boolean = true,
    ): MediaPrep.Prepared {
        val file = File.createTempFile(
            "fc-gif", ".mp4",
            File(RuntimeEnvironment.getApplication().cacheDir, "uploads").apply { mkdirs() },
        ).apply { writeBytes(byteArrayOf(1, 2, 3)) }
        return MediaPrep.Prepared(file, mime, kind, width, height, durationMs, POSTER, animation = animation)
    }

    // -- One send ----------------------------------------------------------------------

    @Test
    fun `an animation goes up with its poster and its caption and comes back flagged`() = runTest(dispatcher) {
        insertChat()
        val messages = newMessages()

        val clientMsgId = messages.sendMedia(listOf(made()), caption = "lol", chatId = CHAT, animation = true)
        advanceUntilIdle()

        assertThat(clientMsgId).isNotNull()
        assertThat(attachmentApi.uploadedMetadata.single().first).isEqualTo("video")
        assertThat(attachmentApi.calls).containsExactly("upload", "preview").inOrder()
        assertThat(chatApi.postedMessages.single().third).isEqualTo("lol")
        assertThat(chatApi.postedAnimationFlags.single()).isTrue()
        assertThat(chatApi.postedRoundFlags.single()).isNull()
        assertThat(chatApi.postedStickerFlags.single()).isNull()
        val row = messageDao.findByClientMsgId(clientMsgId!!)!!
        assertThat(row.status).isEqualTo(MessageStatus.SENT)
        assertThat(row.attachmentList.single().isAnimation).isTrue()
    }

    @Test
    fun `the chat list says GIF and the bytes are kept for the tile`() = runTest(dispatcher) {
        insertChat()
        val messages = newMessages()
        attachmentApi.uploadHandler = { _, _, _ -> ApiResult.NetworkError(IllegalStateException("offline")) }

        messages.sendMedia(listOf(made()), caption = "", chatId = CHAT, animation = true)
        runCurrent()

        val placeholder = messageDao.pendingSending().single().attachmentList.single()
        assertThat(placeholder.id).isLessThan(0)
        assertThat(placeholder.isAnimation).isTrue()
        assertThat(chatDao.getById(CHAT)?.lastMessageBody).isEqualTo("GIF")
        // The poster AND the MP4 under the provisional id: the sender's own tile
        // plays from the local file, never asking the server for an id it does not know.
        assertThat(posters.seeded).contains(placeholder.id to POSTER.size)
        assertThat(posters.originals).contains(placeholder.id)
    }

    @Test
    fun `an open socket carries the flag in the send frame`() = runTest(dispatcher) {
        insertChat()
        val messages = newMessages()
        socket.setOpen(true)
        runCurrent()

        messages.sendMedia(listOf(made()), caption = "", chatId = CHAT, animation = true)
        runCurrent()

        val frame = socket.sent.filterIsInstance<ClientFrame.Send>().single()
        assertThat(frame.animation).isTrue()
        assertThat(frame.round).isNull()
        assertThat(frame.sticker).isNull()
        assertThat(frame.attachmentIds).containsExactly(93L)
        // Kept under the server's id too.
        assertThat(posters.originals).contains(93L)
    }

    @Test
    fun `an ordinary video still sends no flag`() = runTest(dispatcher) {
        insertChat()
        val messages = newMessages()

        messages.sendMedia(listOf(made(animation = false)), caption = "", chatId = CHAT)
        advanceUntilIdle()

        assertThat(chatApi.postedAnimationFlags).containsExactly(null)
    }

    @Test
    fun `a retry from a fresh process still sends an animation`() = runTest(dispatcher) {
        insertChat()
        val ok = chatApi.postMessageHandler
        chatApi.postMessageHandler = { _, _, _ -> ApiResult.HttpError(503, null, "busy") }
        newMessages().sendMedia(listOf(made()), caption = "", chatId = CHAT, animation = true)
        advanceUntilIdle()
        val clientMsgId = messageDao.pendingSending().single().clientMsgId

        chatApi.postMessageHandler = ok
        newMessages().retry(clientMsgId)
        advanceUntilIdle()

        assertThat(chatApi.postedAnimationFlags.toSet()).containsExactly(true)
    }

    @Test
    fun `an upload that finishes in a later process sends the flag`() = runTest(dispatcher) {
        insertChat()
        val ok = attachmentApi.uploadHandler
        attachmentApi.uploadHandler = { _, _, _ -> ApiResult.NetworkError(IllegalStateException("offline")) }
        newMessages().sendMedia(listOf(made()), caption = "", chatId = CHAT, animation = true)
        advanceUntilIdle()
        val clientMsgId = messageDao.pendingSending().single().clientMsgId
        assertThat(chatApi.postedAnimationFlags).isEmpty()

        attachmentApi.uploadHandler = ok
        newMessages().uploadPending(clientMsgId)
        advanceUntilIdle()

        assertThat(chatApi.postedAnimationFlags).containsExactly(true)
    }

    @Test
    fun `an upload the server swept goes up again still an animation`() = runTest(dispatcher) {
        insertChat()
        val ok = chatApi.postMessageHandler
        var first = true
        chatApi.postMessageHandler = { chatId, clientMsgId, body ->
            if (first) {
                first = false
                ApiResult.HttpError(400, "attachment_expired", "gone")
            } else {
                ok(chatId, clientMsgId, body)
            }
        }
        newMessages().sendMedia(listOf(made()), caption = "", chatId = CHAT, animation = true)
        advanceUntilIdle()

        assertThat(attachmentApi.calls.count { it == "upload" }).isEqualTo(2)
        assertThat(chatApi.postedAnimationFlags).containsExactly(true, true).inOrder()
    }

    // -- What the server would refuse never becomes a row --------------------------------

    @Test
    fun `a shape the server refuses is not sent at all`() = runTest(dispatcher) {
        insertChat()
        val messages = newMessages()
        val refused = mapOf(
            "two videos" to listOf(made(), made()),
            "a photo" to listOf(made(mime = "image/jpeg", kind = AttachmentDto.KIND_PHOTO)),
            "QuickTime" to listOf(made(mime = "video/quicktime")),
            "short side over 720" to listOf(made(width = 1280, height = 721)),
            "no size" to listOf(made(height = null)),
            "no length" to listOf(made(durationMs = null)),
            "zero length" to listOf(made(durationMs = 0)),
        )
        for ((shape, items) in refused) {
            assertWithMessage(shape)
                .that(messages.sendMedia(items, caption = "", chatId = CHAT, animation = true))
                .isNull()
        }
        assertWithMessage("with sticker")
            .that(messages.sendMedia(listOf(made()), caption = "", chatId = CHAT, sticker = true, animation = true))
            .isNull()
        advanceUntilIdle()

        assertThat(messageDao.pendingSending()).isEmpty()
        assertThat(chatApi.postedMessages).isEmpty()
        assertThat(attachmentApi.calls).isEmpty()
    }

    @Test
    fun `the limits themselves are accepted`() {
        val edge = MediaPrep.Prepared(File("x"), "video/mp4", AttachmentDto.KIND_VIDEO, 1280, 720, 1, null, animation = true)
        assertThat(AnimationSend.accepts(listOf(edge), sticker = false, round = false)).isTrue()
        assertThat(AnimationSend.accepts(listOf(edge.copy(width = 2, height = 2)), false, false)).isTrue()
        assertThat(AnimationSend.accepts(listOf(edge.copy(width = 0)), false, false)).isFalse()
        assertThat(AnimationSend.accepts(listOf(edge), sticker = false, round = true)).isFalse()
        assertThat(AnimationSend.accepts(emptyList(), false, false)).isFalse()
    }

    // -- A composer holding more than the animation ------------------------------------------

    @Test
    fun `an animation goes flagged only alone - beside other items it is an ordinary video in the set`() {
        val photo = MediaPrep.Prepared(File("p"), "image/jpeg", AttachmentDto.KIND_PHOTO, 4, 3, null, null)
        val gif = MediaPrep.Prepared(File("g"), "video/mp4", AttachmentDto.KIND_VIDEO, 4, 4, 300, null, animation = true)

        assertThat(AnimationSend.sendsAsAnimation(listOf(gif))).isTrue()
        assertThat(AnimationSend.sendsAsAnimation(listOf(photo, gif))).isFalse()
        assertThat(AnimationSend.sendsAsAnimation(listOf(gif, gif.copy(file = File("g2"))))).isFalse()
        assertThat(AnimationSend.sendsAsAnimation(listOf(photo))).isFalse()
        assertThat(AnimationSend.sendsAsAnimation(emptyList())).isFalse()
    }

    @Test
    fun `a set with an animation in it is one message with no flag`() = runTest(dispatcher) {
        insertChat()
        val messages = newMessages()
        chatApi.postMessageHandler = { _, _, _ -> ApiResult.HttpError(503, null, "busy") }
        val set = listOf(made(), made(mime = "image/jpeg", kind = AttachmentDto.KIND_PHOTO, animation = false))

        messages.sendMedia(set, caption = "both", chatId = CHAT, animation = AnimationSend.sendsAsAnimation(set))
        advanceUntilIdle()

        assertThat(chatApi.postedAttachmentIds.first()).hasSize(2)
        assertThat(chatApi.postedAnimationFlags.toSet()).containsExactly(null)
    }
}
