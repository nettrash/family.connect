/*
 * MessageRepositoryTest.kt
 * Family Connect (Android)
 *
 * The send pipeline, end to end against real in-memory Room and a fake
 * socket: optimistic insert → ack updates the SAME row; ack timeout →
 * REST fallback with the SAME client_msg_id; failures → FAILED; retry
 * reuses the UUID; inbound dedup in both frame orders; unread bump
 * rules. Virtual time drives the 15 s ack timer.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
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
import me.nettrash.familyconnect.data.net.dto.MessageResponse
import me.nettrash.familyconnect.data.net.ws.ClientFrame
import me.nettrash.familyconnect.data.net.ws.ServerFrame
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.testutil.FakeChatApi
import me.nettrash.familyconnect.testutil.FakeChatSocket
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.createTestDb
import me.nettrash.familyconnect.testutil.messageDto
import me.nettrash.familyconnect.util.Clock
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class MessageRepositoryTest {

    private companion object {
        const val ME = 7L
        const val PEER = 9L
        const val CHAT = 42L
        const val NOW = 1_000_000L
    }

    // One scheduler for the tests AND for Room — see TestDb.kt.
    private val dispatcher = StandardTestDispatcher()

    // The repository's "app scope" — deliberately NOT runTest's
    // backgroundScope: background tasks are skipped by advanceUntilIdle
    // (it stops once no *foreground* work remains), which would make
    // every frame-collector assertion flaky. A plain scope on the same
    // scheduler keeps all repository work foreground and deterministic.
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private lateinit var db: AppDatabase
    private lateinit var messageDao: MessageDao
    private lateinit var chatDao: ChatDao
    private lateinit var chatApi: FakeChatApi
    private lateinit var socket: FakeChatSocket
    private lateinit var settings: FakeSettingsRepository
    private lateinit var chatRepository: ChatRepository

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

    /** Builds the repository on the foreground repoScope and lets its frame collector subscribe. */
    private fun TestScope.newRepository(): MessageRepository {
        chatRepository = ChatRepository(chatApi, chatDao, socket)
        val repository = MessageRepository(
            chatApi = chatApi,
            messageDao = messageDao,
            chatDao = chatDao,
            socket = socket,
            settings = settings,
            chatRepository = chatRepository,
            scope = repoScope,
            clock = Clock { NOW },
        )
        runCurrent() // frame collector must be subscribed before any emit
        return repository
    }

    private suspend fun insertChat(
        id: Long = CHAT,
        kind: String = "direct",
        peerUserId: Long? = PEER,
    ) {
        chatDao.upsertAll(
            listOf(
                ChatEntity(
                    id = id,
                    kind = kind,
                    peerUserId = peerUserId,
                    title = "Chat $id",
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

    private fun sentClientMsgId(): String =
        socket.sent.filterIsInstance<ClientFrame.Send>().single().clientMsgId

    // -- Optimistic send + ack --------------------------------------------------

    @Test
    fun optimisticInsertThenAckUpdatesTheSameRow() = runTest(dispatcher) {
        val repository = newRepository()
        insertChat()
        socket.setOpen(true)

        repository.send(CHAT, "hello")

        val clientMsgId = sentClientMsgId()
        val pending = messageDao.findByClientMsgId(clientMsgId)!!
        assertThat(pending.status).isEqualTo(MessageStatus.SENDING)
        assertThat(pending.serverId).isNull()
        assertThat(pending.senderId).isEqualTo(ME)

        socket.emit(
            ServerFrame.Ack(
                clientMsgId = clientMsgId,
                message = messageDto(id = 100, chatId = CHAT, senderId = ME, clientMsgId = clientMsgId, body = "hello"),
            ),
        )
        advanceUntilIdle()

        val acked = messageDao.findByClientMsgId(clientMsgId)!!
        assertThat(acked.serverId).isEqualTo(100L)
        assertThat(acked.status).isEqualTo(MessageStatus.SENT)
        // Same row — never a second one.
        assertThat(messageDao.observeMessages(CHAT, 50).first()).hasSize(1)
        // No REST fallback happened.
        assertThat(chatApi.postedMessages).isEmpty()
    }

    @Test
    fun ackTimeoutFallsBackToRestWithTheSameClientMsgId() = runTest(dispatcher) {
        val repository = newRepository()
        insertChat()
        socket.setOpen(true)
        chatApi.postMessageHandler = { chatId, clientMsgId, body ->
            ApiResult.Ok(
                MessageResponse(
                    messageDto(id = 101, chatId = chatId, senderId = ME, clientMsgId = clientMsgId, body = body),
                ),
            )
        }

        repository.send(CHAT, "slow ack")
        val clientMsgId = sentClientMsgId()

        advanceTimeBy(15_001)
        advanceUntilIdle()

        assertThat(chatApi.postedMessages).hasSize(1)
        assertThat(chatApi.postedMessages.single().second).isEqualTo(clientMsgId)
        val row = messageDao.findByClientMsgId(clientMsgId)!!
        assertThat(row.serverId).isEqualTo(101L)
        assertThat(row.status).isEqualTo(MessageStatus.SENT)
    }

    @Test
    fun closedSocketGoesStraightToRest() = runTest(dispatcher) {
        val repository = newRepository()
        insertChat()
        socket.setOpen(false)
        chatApi.postMessageHandler = { chatId, clientMsgId, body ->
            ApiResult.Ok(
                MessageResponse(
                    messageDto(id = 102, chatId = chatId, senderId = ME, clientMsgId = clientMsgId, body = body),
                ),
            )
        }

        repository.send(CHAT, "offline path")
        advanceUntilIdle()

        assertThat(socket.sent).isEmpty()
        assertThat(chatApi.postedMessages).hasSize(1)
        val row = messageDao.findByClientMsgId(chatApi.postedMessages.single().second)!!
        assertThat(row.status).isEqualTo(MessageStatus.SENT)
    }

    @Test
    fun restFailureMarksTheRowFailed() = runTest(dispatcher) {
        val repository = newRepository()
        insertChat()
        socket.setOpen(false)
        chatApi.postMessageHandler = { _, _, _ ->
            ApiResult.HttpError(400, "message_too_long", "too long")
        }

        repository.send(CHAT, "doomed")
        advanceUntilIdle()

        val row = messageDao.findByClientMsgId(chatApi.postedMessages.single().second)!!
        assertThat(row.status).isEqualTo(MessageStatus.FAILED)
    }

    @Test
    fun retryReentersThePipelineWithTheSameUuid() = runTest(dispatcher) {
        val repository = newRepository()
        insertChat()
        socket.setOpen(false)
        chatApi.postMessageHandler = { _, _, _ ->
            ApiResult.NetworkError(RuntimeException("wifi died"))
        }

        repository.send(CHAT, "retry me")
        advanceUntilIdle()
        val clientMsgId = chatApi.postedMessages.single().second
        assertThat(messageDao.findByClientMsgId(clientMsgId)!!.status)
            .isEqualTo(MessageStatus.FAILED)

        // Second attempt succeeds — with the SAME UUID, so the server
        // dedups even if the first POST secretly landed.
        chatApi.postMessageHandler = { chatId, id, body ->
            ApiResult.Ok(
                MessageResponse(
                    messageDto(id = 103, chatId = chatId, senderId = ME, clientMsgId = id, body = body),
                ),
            )
        }
        repository.retry(clientMsgId)
        advanceUntilIdle()

        assertThat(chatApi.postedMessages).hasSize(2)
        assertThat(chatApi.postedMessages.map { it.second }.distinct()).hasSize(1)
        assertThat(messageDao.findByClientMsgId(clientMsgId)!!.status)
            .isEqualTo(MessageStatus.SENT)
    }

    // -- Inbound dedup ----------------------------------------------------------------

    @Test
    fun ackThenMessageFrameDoesNotDuplicate() = runTest(dispatcher) {
        val repository = newRepository()
        insertChat()
        socket.setOpen(true)

        repository.send(CHAT, "once")
        val clientMsgId = sentClientMsgId()
        val dto = messageDto(id = 104, chatId = CHAT, senderId = ME, clientMsgId = clientMsgId, body = "once")

        socket.emit(ServerFrame.Ack(clientMsgId, dto))
        advanceUntilIdle()
        socket.emit(ServerFrame.Message(dto)) // echo from the fan-out
        advanceUntilIdle()

        val rows = messageDao.observeMessages(CHAT, 50).first()
        assertThat(rows).hasSize(1)
        assertThat(rows.single().clientMsgId).isEqualTo(clientMsgId)
    }

    @Test
    fun messageThenAckFrameDoesNotDuplicate() = runTest(dispatcher) {
        val repository = newRepository()
        insertChat()
        socket.setOpen(true)

        repository.send(CHAT, "swapped order")
        val clientMsgId = sentClientMsgId()
        val dto = messageDto(id = 105, chatId = CHAT, senderId = ME, clientMsgId = clientMsgId, body = "swapped order")

        socket.emit(ServerFrame.Message(dto)) // fan-out echo arrives first
        advanceUntilIdle()
        socket.emit(ServerFrame.Ack(clientMsgId, dto))
        advanceUntilIdle()

        val rows = messageDao.observeMessages(CHAT, 50).first()
        assertThat(rows).hasSize(1)
        val row = rows.single()
        assertThat(row.clientMsgId).isEqualTo(clientMsgId) // folded into MY row
        assertThat(row.serverId).isEqualTo(105L)
        assertThat(row.status).isEqualTo(MessageStatus.SENT)
    }

    @Test
    fun resyncRowIsReplacedWhenTheLateAckArrives() = runTest(dispatcher) {
        val repository = newRepository()
        insertChat()
        // A pending send exists locally…
        messageDao.insert(
            MessageEntity(
                clientMsgId = "local-uuid",
                serverId = null,
                chatId = CHAT,
                senderId = ME,
                body = "raced",
                createdAt = NOW,
                status = MessageStatus.SENDING,
            ),
        )
        // …and a resync already stored the same message under "s106"
        // (different sender field can't match: same sender, same chat).
        val dto = messageDto(id = 106, chatId = CHAT, senderId = ME, clientMsgId = "local-uuid", body = "raced")
        messageDao.insertIgnore(
            listOf(
                MessageEntity(
                    clientMsgId = "s106",
                    serverId = 106,
                    chatId = CHAT,
                    senderId = ME,
                    body = "raced",
                    createdAt = NOW,
                    status = MessageStatus.SENT,
                ),
            ),
        )

        socket.setOpen(true)
        socket.emit(ServerFrame.Ack("local-uuid", dto))
        advanceUntilIdle()

        val rows = messageDao.observeMessages(CHAT, 50).first()
        assertThat(rows).hasSize(1)
        assertThat(rows.single().clientMsgId).isEqualTo("local-uuid")
        assertThat(rows.single().serverId).isEqualTo(106L)
    }

    // -- Unread bump rules -----------------------------------------------------------------

    @Test
    fun liveInboundMessageBumpsUnreadWhenChatIsNotOpen() = runTest(dispatcher) {
        val repository = newRepository()
        insertChat()
        chatRepository.setOpenChat(null)

        socket.emit(ServerFrame.Message(messageDto(id = 200, chatId = CHAT, senderId = PEER)))
        advanceUntilIdle()

        val chat = chatDao.getById(CHAT)!!
        assertThat(chat.unreadCount).isEqualTo(1)
        assertThat(messageDao.findByClientMsgId("s200")).isNotNull()
        assertThat(chat.lastMessageBody).isEqualTo("msg 200")
    }

    @Test
    fun liveInboundMessageForTheOpenChatDoesNotBumpUnread() = runTest(dispatcher) {
        val repository = newRepository()
        insertChat()
        chatRepository.setOpenChat(CHAT)

        socket.emit(ServerFrame.Message(messageDto(id = 201, chatId = CHAT, senderId = PEER)))
        advanceUntilIdle()

        assertThat(chatDao.getById(CHAT)!!.unreadCount).isEqualTo(0)
        assertThat(messageDao.findByClientMsgId("s201")).isNotNull() // still stored
    }

    @Test
    fun myOwnEchoFromAnotherDeviceDoesNotBumpUnread() = runTest(dispatcher) {
        val repository = newRepository()
        insertChat()
        chatRepository.setOpenChat(null)

        socket.emit(
            ServerFrame.Message(
                messageDto(id = 202, chatId = CHAT, senderId = ME, clientMsgId = "other-device-uuid"),
            ),
        )
        advanceUntilIdle()

        assertThat(chatDao.getById(CHAT)!!.unreadCount).isEqualTo(0)
        assertThat(messageDao.findByClientMsgId("s202")).isNotNull()
    }

    // -- Peer read + error frames --------------------------------------------------------------

    @Test
    fun peerReadFrameMovesPeerMarkerInDirectChatsOnly() = runTest(dispatcher) {
        val repository = newRepository()
        insertChat(id = CHAT, kind = "direct", peerUserId = PEER)
        insertChat(id = 1L, kind = "family", peerUserId = null)

        socket.emit(ServerFrame.Read(chatId = CHAT, userId = PEER, lastReadMessageId = 300))
        socket.emit(ServerFrame.Read(chatId = 1L, userId = PEER, lastReadMessageId = 10))
        advanceUntilIdle()

        assertThat(chatDao.getById(CHAT)!!.peerLastReadId).isEqualTo(300L)
        assertThat(chatDao.getById(1L)!!.peerLastReadId).isNull()
    }

    @Test
    fun sendErrorFrameMarksTheRowFailed() = runTest(dispatcher) {
        val repository = newRepository()
        insertChat()
        socket.setOpen(true)

        repository.send(CHAT, "rejected")
        val clientMsgId = sentClientMsgId()

        socket.emit(
            ServerFrame.Error(code = "not_chat_member", message = "nope", clientMsgId = clientMsgId),
        )
        advanceUntilIdle()

        assertThat(messageDao.findByClientMsgId(clientMsgId)!!.status)
            .isEqualTo(MessageStatus.FAILED)
        // The ack timer was cancelled — no REST fallback fires later.
        advanceTimeBy(20_000)
        advanceUntilIdle()
        assertThat(chatApi.postedMessages).isEmpty()
    }

    // -- History paging + flush ---------------------------------------------------------------------

    @Test
    fun loadOlderReportsReachedStartOnShortPage() = runTest(dispatcher) {
        val repository = newRepository()
        insertChat()
        messageDao.insertIgnore(
            listOf(
                MessageEntity("s50", 50, CHAT, PEER, "old", NOW, MessageStatus.SENT),
            ),
        )
        chatApi.messagesHandler = { _, beforeId, _, _ ->
            assertThat(beforeId).isEqualTo(50L)
            ApiResult.Ok(
                me.nettrash.familyconnect.data.net.dto.MessagesResponse(
                    listOf(
                        messageDto(id = 49, chatId = CHAT, senderId = PEER),
                        messageDto(id = 48, chatId = CHAT, senderId = PEER),
                    ),
                ),
            )
        }

        val reachedStart = repository.loadOlder(CHAT)

        assertThat(reachedStart).isTrue() // 2 < page size 50
        assertThat(messageDao.oldestServerId(CHAT)).isEqualTo(48L)
        // History inserts never bump unread.
        assertThat(chatDao.getById(CHAT)!!.unreadCount).isEqualTo(0)
    }

    @Test
    fun flushPendingResendsSendingRowsOverTheSocket() = runTest(dispatcher) {
        val repository = newRepository()
        insertChat()
        messageDao.insert(
            MessageEntity("stuck-uuid", null, CHAT, ME, "stuck", NOW, MessageStatus.SENDING),
        )
        socket.setOpen(true)

        repository.flushPending()
        runCurrent()

        val frame = socket.sent.filterIsInstance<ClientFrame.Send>().single()
        assertThat(frame.clientMsgId).isEqualTo("stuck-uuid")
        assertThat(frame.body).isEqualTo("stuck")
    }
}
