/*
 * MessageRepositoryMentionTest.kt
 * Family Connect (Android) — tests
 *
 * Member mentions in the store (docs/protocol.md, "Mentioning a member"):
 * the list on the optimistic row and on both send legs, the server's copy
 * overwriting it on the ack, and the "@" mark on the chat row — set by a
 * live frame naming this reader, never by a page, cleared by reading.
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
import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.data.db.MessageStatus
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.MessageDto
import me.nettrash.familyconnect.data.net.dto.MentionDto
import me.nettrash.familyconnect.data.net.dto.MentionsCodec
import me.nettrash.familyconnect.data.net.dto.MessageResponse
import me.nettrash.familyconnect.data.net.dto.ReplyToDto
import me.nettrash.familyconnect.data.net.ws.ClientFrame
import me.nettrash.familyconnect.data.net.ws.ServerFrame
import me.nettrash.familyconnect.data.settings.SettingsState
import java.io.File
import me.nettrash.familyconnect.testutil.FakeAttachmentApi
import me.nettrash.familyconnect.testutil.FakeChatApi
import me.nettrash.familyconnect.testutil.FakeChatSocket
import me.nettrash.familyconnect.testutil.FakePosterCache
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.createTestDb
import me.nettrash.familyconnect.testutil.messageDto
import me.nettrash.familyconnect.testutil.testChatRepository
import me.nettrash.familyconnect.util.Clock
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class MessageRepositoryMentionTest {

    private companion object {
        const val ME = 7L
        const val PEER = 9L
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

    private lateinit var chatRepository: ChatRepository

    private fun TestScope.newRepository(): MessageRepository {
        val chatRepository = testChatRepository(chatApi, chatDao, messageDao, socket, repoScope)
        this@MessageRepositoryMentionTest.chatRepository = chatRepository
        val repository = MessageRepository(
            appContext = RuntimeEnvironment.getApplication(),
            chatApi = chatApi,
            attachmentApi = attachmentApi,
            messageDao = messageDao,
            chatDao = chatDao,
            socket = socket,
            settings = settings,
            chatRepository = chatRepository,
            posterCache = FakePosterCache(),
            scope = repoScope,
            clock = Clock { NOW },
            pendingAttachmentDao = db.pendingAttachmentDao(),
            staging = MediaStaging(RuntimeEnvironment.getApplication()),
        )
        runCurrent()
        return repository
    }

    private suspend fun insertFamilyChat() {
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

    private val junior = MentionDto(userId = PEER, name = "Junior")
    private val meNamed = MentionDto(userId = ME, name = "Olive")

    private fun named(id: Long, senderId: Long = PEER, mentions: List<MentionDto>?): MessageDto =
        messageDto(id, senderId = senderId, body = "@Olive dinner?").copy(mentions = mentions)

    private fun sentFrames() = socket.sent.filterIsInstance<ClientFrame.Send>()

    @Test
    fun sendStoresTheListOnTheRowAndTheSocketLegCarriesIt() = runTest(dispatcher) {
        val repository = newRepository()
        insertFamilyChat()
        socket.setOpen(true)

        repository.send(CHAT, "@Junior dinner?", mentions = listOf(junior))
        advanceUntilIdle()

        val frame = sentFrames().single()
        assertThat(frame.mentions).containsExactly(junior)
        val row = messageDao.findByClientMsgId(frame.clientMsgId)!!
        assertThat(MentionsCodec.decode(row.mentionsJson)).containsExactly(junior)
        // And an ordinary message carries neither.
        repository.send(CHAT, "just talking")
        advanceUntilIdle()
        assertThat(sentFrames().last().mentions).isNull()
        assertThat(messageDao.findByClientMsgId(sentFrames().last().clientMsgId)!!.mentionsJson).isNull()
    }

    @Test
    fun theRestLegCarriesTheListAndTheAckOverwritesTheRow() = runTest(dispatcher) {
        val repository = newRepository()
        insertFamilyChat()
        socket.setOpen(false)
        // The server's copy is what stays on the row — the same authority
        // rule as the quote's excerpt, pinned with a copy that differs from
        // what this device resolved.
        val serverCopy = MentionDto(PEER, "Junior")
        chatApi.postMessageHandler = { _, clientMsgId, body ->
            ApiResult.Ok(
                MessageResponse(
                    messageDto(300, senderId = ME, clientMsgId = clientMsgId, body = body)
                        .copy(mentions = listOf(serverCopy)),
                ),
            )
        }

        repository.send(CHAT, "@Junior dinner?", mentions = listOf(junior, MentionDto(11, "Nobody")))
        advanceUntilIdle()

        assertThat(chatApi.postedMentions.single()).containsExactly(junior, MentionDto(11, "Nobody")).inOrder()
        assertThat(MentionsCodec.decode(messageDao.findByServerId(300)!!.mentionsJson)).containsExactly(serverCopy)
    }

    @Test
    fun aResendOffTheRowStillNamesWhomTheSenderNamed() = runTest(dispatcher) {
        val repository = newRepository()
        insertFamilyChat()
        // A row the process died on: nothing in memory, only the row.
        messageDao.insert(
            MessageEntity(
                "stuck-uuid", null, CHAT, ME, "@Junior dinner?", NOW, MessageStatus.SENDING,
                mentionsJson = MentionsCodec.encode(listOf(junior)),
            ),
        )
        socket.setOpen(true)

        // The outbox flush, over the socket.
        repository.flushPending()
        runCurrent()
        assertThat(sentFrames().single().mentions).containsExactly(junior)

        // Tap-to-retry, over REST.
        socket.setOpen(false)
        messageDao.markSendFailed("stuck-uuid", 1)
        chatApi.postMessageHandler = { _, _, _ -> ApiResult.HttpError(500, "internal", "boom") }
        repository.retry("stuck-uuid")
        advanceUntilIdle()
        // Every REST attempt the backoff made carried the list.
        assertThat(chatApi.postedMentions).isNotEmpty()
        assertThat(chatApi.postedMentions.toSet()).containsExactly(listOf(junior))
    }

    @Test
    fun aLiveFrameNamingMeMarksTheRowAndNamingSomebodyElseDoesNot() = runTest(dispatcher) {
        val repository = newRepository()
        insertFamilyChat()
        repository.applyServerMessage(named(100, mentions = listOf(junior)), live = true)
        assertThat(chatDao.getById(CHAT)!!.mentionedUnread).isFalse()
        assertThat(chatDao.getById(CHAT)!!.unreadCount).isEqualTo(1)
        repository.applyServerMessage(named(101, mentions = listOf(meNamed)), live = true)
        assertThat(chatDao.getById(CHAT)!!.mentionedUnread).isTrue()
        assertThat(chatDao.getById(CHAT)!!.unreadCount).isEqualTo(2)
        assertThat(MentionsCodec.decode(messageDao.findByServerId(101)!!.mentionsJson)).containsExactly(meNamed)
    }

    @Test
    fun myOwnMessageNamingMeMarksNothingAndNeitherDoesAPage() = runTest(dispatcher) {
        val repository = newRepository()
        insertFamilyChat()
        repository.applyServerMessage(named(100, senderId = ME, mentions = listOf(meNamed)), live = true)
        assertThat(chatDao.getById(CHAT)!!.mentionedUnread).isFalse()
        // A history page holding one that names me: the count came from
        // `GET /chats`, and so did the mark.
        repository.applyServerMessage(named(101, mentions = listOf(meNamed)), live = false)
        assertThat(chatDao.getById(CHAT)!!.mentionedUnread).isFalse()
        assertThat(MentionsCodec.decode(messageDao.findByServerId(101)!!.mentionsJson)).containsExactly(meNamed)
        // The same live frame twice is one message.
        repository.applyServerMessage(named(102, mentions = listOf(meNamed)), live = true)
        repository.applyServerMessage(named(102, mentions = listOf(meNamed)), live = true)
        assertThat(chatDao.getById(CHAT)!!.unreadCount).isEqualTo(1)
        assertThat(chatDao.getById(CHAT)!!.mentionedUnread).isTrue()
    }

    @Test
    fun aFrameFromABlockedMemberNeverMarks() = runTest(dispatcher) {
        settings.setBlockedUserIds(setOf(PEER))
        val repository = newRepository()
        insertFamilyChat()

        // The frame ARRIVES — a blocked member's family-chat message is not
        // suppressed, it is drawn as a hidden row — and it counts.
        repository.applyServerMessage(named(100, mentions = listOf(meNamed)), live = true)
        assertThat(chatDao.getById(CHAT)!!.unreadCount).isEqualTo(1)
        assertThat(chatDao.getById(CHAT)!!.mentionedUnread).isFalse()

        // Somebody they have not blocked, naming them, still marks it.
        repository.applyServerMessage(
            named(101, senderId = 12L, mentions = listOf(meNamed)),
            live = true,
        )
        assertThat(chatDao.getById(CHAT)!!.mentionedUnread).isTrue()
    }

    @Test
    fun aMentionThatLandsWhileImLookingMarksNothing() = runTest(dispatcher) {
        val repository = newRepository()
        insertFamilyChat()
        chatRepository.setOpenChat(CHAT, atNewest = true)

        repository.applyServerMessage(named(100, mentions = listOf(meNamed)), live = true)
        assertThat(chatDao.getById(CHAT)!!.unreadCount).isEqualTo(0)
        assertThat(chatDao.getById(CHAT)!!.mentionedUnread).isFalse()

        // Away from the newest message, the same frame is genuinely unread.
        chatRepository.setOpenChat(CHAT, atNewest = false)
        repository.applyServerMessage(named(101, mentions = listOf(meNamed)), live = true)
        assertThat(chatDao.getById(CHAT)!!.mentionedUnread).isTrue()
    }

    @Test
    fun aCaptionAndAQuestionNameMembersLikeEveryOtherBody() = runTest(dispatcher) {
        val repository = newRepository()
        insertFamilyChat()
        socket.setOpen(true)

        // A poll's question.
        repository.sendPoll(CHAT, "@Junior pizza or pasta?", listOf("Pizza", "Pasta"), null, listOf(junior))
        advanceUntilIdle()
        assertThat(sentFrames().last().mentions).containsExactly(junior)

        // A location's caption.
        repository.sendLocation(
            latitude = 44.8, longitude = 20.4, accuracyM = 12, label = null,
            caption = "@Junior we are here", chatId = CHAT, replyTo = null,
            mentions = listOf(junior),
        )
        advanceUntilIdle()
        assertThat(sentFrames().last().mentions).containsExactly(junior)

        // And every row holds its own list, for the retry.
        val rows = messageDao.pendingSending()
        assertThat(rows).hasSize(2)
        rows.forEach {
            assertThat(MentionsCodec.decode(it.mentionsJson)).containsExactly(junior)
        }
    }

    /**
     * A photo's caption goes out through the UPLOAD leg, which dispatches
     * from the row rather than from the caller's arguments — so the list
     * has to be on the row and read back off it, the same way the poll's
     * options are.
     */
    @Test
    fun aPhotoCaptionNamesMembersThroughTheUploadLeg() = runTest(dispatcher) {
        val repository = newRepository()
        insertFamilyChat()
        socket.setOpen(true)
        val file = File.createTempFile(
            "fc-upload", ".jpg",
            File(RuntimeEnvironment.getApplication().cacheDir, "uploads").apply { mkdirs() },
        )
        file.writeBytes(byteArrayOf(0xFF.toByte(), 0xD8.toByte(), 0xFF.toByte(), 0xE0.toByte()))
        val media = MediaPrep.Prepared(
            file = file, mime = "image/jpeg", kind = "photo",
            width = 1600, height = 1200, durationMs = null, previewJpeg = ByteArray(64) { 0x7 },
        )

        val clientMsgId = repository.sendMedia(
            listOf(media), caption = "@Junior look at this", chatId = CHAT,
            replyTo = null, mentions = listOf(junior),
        )
        advanceUntilIdle()

        assertThat(clientMsgId).isNotNull()
        assertThat(MentionsCodec.decode(messageDao.findByClientMsgId(clientMsgId!!)!!.mentionsJson))
            .containsExactly(junior)
        assertThat(sentFrames().single().mentions).containsExactly(junior)
    }

    @Test
    fun readingClearsTheMarkWithTheCount() = runTest(dispatcher) {
        val repository = newRepository()
        insertFamilyChat()
        repository.applyServerMessage(named(100, mentions = listOf(meNamed)), live = true)
        assertThat(chatDao.getById(CHAT)!!.mentionedUnread).isTrue()
        chatDao.clearUnread(CHAT)
        assertThat(chatDao.getById(CHAT)!!.mentionedUnread).isFalse()
        assertThat(chatDao.getById(CHAT)!!.unreadCount).isEqualTo(0)
        // A recount that lands above zero keeps the mark; one at zero clears it.
        repository.applyServerMessage(named(101, mentions = listOf(meNamed)), live = true)
        chatDao.setUnreadCount(CHAT, 1)
        assertThat(chatDao.getById(CHAT)!!.mentionedUnread).isTrue()
        chatDao.setUnreadCount(CHAT, 0)
        assertThat(chatDao.getById(CHAT)!!.mentionedUnread).isFalse()
    }
}
