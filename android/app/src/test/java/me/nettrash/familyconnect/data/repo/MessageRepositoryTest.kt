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
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
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
import me.nettrash.familyconnect.data.net.dto.MessageReactionStateDto
import me.nettrash.familyconnect.data.net.dto.MessageResponse
import me.nettrash.familyconnect.data.net.dto.ReactionDto
import me.nettrash.familyconnect.data.net.dto.ReactionsCodec
import me.nettrash.familyconnect.data.net.dto.ReplyToDto
import me.nettrash.familyconnect.data.net.ws.ClientFrame
import me.nettrash.familyconnect.data.net.ws.ServerFrame
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.testutil.FakeChatApi
import me.nettrash.familyconnect.testutil.FakeChatSocket
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.createTestDb
import me.nettrash.familyconnect.testutil.messageDto
import me.nettrash.familyconnect.testutil.reactionState
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

    // -- Replies ------------------------------------------------------------------------------------

    /// The optimistic row has to carry the quote itself, so the bubble draws
    /// it the instant it appears rather than when the server answers — and
    /// so a retry after a process death still quotes the right message.
    @Test
    fun sendWithAQuoteStoresItOnTheOptimisticRowAndSendsTheTarget() = runTest(dispatcher) {
        val repository = newRepository()
        insertChat()
        socket.setOpen(true)

        repository.send(
            CHAT,
            "Six works",
            ReplyToDto(messageId = 1337, senderId = 9, excerpt = "See you at six"),
        )
        advanceUntilIdle()

        val frame = socket.sent.filterIsInstance<ClientFrame.Send>().single()
        assertThat(frame.replyToMessageId).isEqualTo(1337)

        val row = messageDao.findByClientMsgId(frame.clientMsgId)!!
        assertThat(row.replyToMessageId).isEqualTo(1337)
        assertThat(row.replySenderId).isEqualTo(9)
        assertThat(row.replyExcerpt).isEqualTo("See you at six")
    }

    @Test
    fun anOrdinaryMessageCarriesNoReplyTarget() = runTest(dispatcher) {
        val repository = newRepository()
        insertChat()
        socket.setOpen(true)

        repository.send(CHAT, "Just talking")
        advanceUntilIdle()

        assertThat(socket.sent.filterIsInstance<ClientFrame.Send>().single().replyToMessageId)
            .isNull()
        val row = messageDao.findByClientMsgId(sentClientMsgId())!!
        assertThat(row.replyToMessageId).isNull()
    }

    /// The REST leg must carry the same target as the socket leg, or a
    /// message that falls back stops being a reply — silently.
    @Test
    fun theRestFallbackCarriesTheQuoteToo() = runTest(dispatcher) {
        val repository = newRepository()
        insertChat()
        socket.setOpen(false)

        repository.send(
            CHAT,
            "Six works",
            ReplyToDto(messageId = 1337, senderId = 9, excerpt = "See you at six"),
        )
        advanceUntilIdle()

        assertThat(chatApi.postedMessages).hasSize(1)
        assertThat(chatApi.postedReplyTargets).containsExactly(1337L)
    }

    /// A retry re-reads the target off the stored row: the caller that had
    /// it in hand is long gone by then.
    @Test
    fun aRetryStillQuotesTheSameMessage() = runTest(dispatcher) {
        val repository = newRepository()
        insertChat()
        socket.setOpen(false)
        chatApi.postMessageHandler = { _, _, _ ->
            ApiResult.NetworkError(IllegalStateException("offline"))
        }

        repository.send(
            CHAT,
            "Six works",
            ReplyToDto(messageId = 1337, senderId = 9, excerpt = "See you at six"),
        )
        advanceUntilIdle()
        val clientMsgId = chatApi.postedMessages.single().second
        assertThat(messageDao.findByClientMsgId(clientMsgId)!!.status)
            .isEqualTo(MessageStatus.FAILED)

        socket.setOpen(true)
        repository.retry(clientMsgId)
        advanceUntilIdle()

        assertThat(socket.sent.filterIsInstance<ClientFrame.Send>().single().replyToMessageId)
            .isEqualTo(1337)
    }

    /// An inbound reply from someone else keeps its quote in the cache, so
    /// the bubble renders identically after a relaunch.
    @Test
    fun anInboundReplyStoresTheServersQuote() = runTest(dispatcher) {
        val repository = newRepository()
        insertChat()
        socket.setOpen(true)

        socket.emit(
            ServerFrame.Message(
                message = messageDto(
                    id = 4242,
                    chatId = CHAT,
                    senderId = 9,
                    clientMsgId = "someone-elses-uuid",
                    body = "Six works",
                    replyTo = ReplyToDto(
                        messageId = 1337,
                        senderId = 7,
                        excerpt = "See you at six",
                    ),
                ),
            ),
        )
        advanceUntilIdle()

        val row = messageDao.findByServerId(4242)!!
        assertThat(row.replyToMessageId).isEqualTo(1337)
        assertThat(row.replySenderId).isEqualTo(7)
        assertThat(row.replyExcerpt).isEqualTo("See you at six")
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

    // -- Reactions -----------------------------------------------------------------------

    private suspend fun insertServerMessage(serverId: Long, chatId: Long = CHAT) {
        messageDao.insertIgnore(
            listOf(
                MessageEntity("s$serverId", serverId, chatId, PEER, "msg", NOW, MessageStatus.SENT),
            ),
        )
    }

    @Test
    fun reactionFrameAppliesStateAndAdvancesTheChatCursor() = runTest(dispatcher) {
        val repository = newRepository()
        insertChat()
        insertServerMessage(100)

        socket.emit(
            ServerFrame.Reaction(
                chatId = CHAT,
                messageId = 100,
                reactionSeq = 5,
                reactions = listOf(ReactionDto(PEER, "❤️")),
            ),
        )
        advanceUntilIdle()

        val row = messageDao.findByServerId(100L)!!
        assertThat(ReactionsCodec.decode(row.reactionsJson)).containsExactly(ReactionDto(PEER, "❤️"))
        assertThat(row.reactionSeq).isEqualTo(5L)
        assertThat(chatDao.maxReactionSeq(CHAT)).isEqualTo(5L)
    }

    @Test
    fun staleReactionFrameIsIgnored() = runTest(dispatcher) {
        val repository = newRepository()
        insertChat()
        insertServerMessage(100)

        socket.emit(ServerFrame.Reaction(CHAT, 100, 5, listOf(ReactionDto(PEER, "❤️"))))
        advanceUntilIdle()
        // Out-of-order older state — full-state semantics + the seq
        // guard mean it must NOT overwrite the newer one.
        socket.emit(ServerFrame.Reaction(CHAT, 100, 3, listOf(ReactionDto(PEER, "👍"))))
        advanceUntilIdle()

        val row = messageDao.findByServerId(100L)!!
        assertThat(ReactionsCodec.decode(row.reactionsJson)).containsExactly(ReactionDto(PEER, "❤️"))
        assertThat(row.reactionSeq).isEqualTo(5L)
        assertThat(chatDao.maxReactionSeq(CHAT)).isEqualTo(5L) // MAX guard held too
    }

    @Test
    fun reactionFrameForUnknownMessageIsDroppedButStillAdvancesTheCursor() = runTest(dispatcher) {
        val repository = newRepository()
        insertChat()

        socket.emit(ServerFrame.Reaction(CHAT, 999, 7, listOf(ReactionDto(PEER, "❤️"))))
        advanceUntilIdle()

        // No row conjured up — the state comes back embedded on the
        // Message when history pages there.
        assertThat(messageDao.findByServerId(999L)).isNull()
        // But the cursor moved: a live socket delivers in order, so seq
        // 7 is proof nothing below it is missing.
        assertThat(chatDao.maxReactionSeq(CHAT)).isEqualTo(7L)
    }

    @Test
    fun redeliveredMessageAppliesNewerEmbeddedReactionsToTheExistingRow() = runTest(dispatcher) {
        val repository = newRepository()
        insertChat()
        repository.applyServerMessage(
            messageDto(id = 100, chatId = CHAT, senderId = PEER, reactions = listOf(ReactionDto(PEER, "❤️")), reactionSeq = 5),
            live = false,
        )
        assertThat(messageDao.findByServerId(100L)!!.reactionSeq).isEqualTo(5L)

        // The same message re-delivered (history page overlap) with a
        // NEWER embedded state — before the existsByServerId early
        // return learned about reactions, this update was discarded.
        repository.applyServerMessage(
            messageDto(id = 100, chatId = CHAT, senderId = PEER, reactions = emptyList(), reactionSeq = 6),
            live = false,
        )
        val row = messageDao.findByServerId(100L)!!
        assertThat(ReactionsCodec.decode(row.reactionsJson)).isEmpty()
        assertThat(row.reactionSeq).isEqualTo(6L)

        // A STALE embedded state on yet another re-delivery is dropped.
        repository.applyServerMessage(
            messageDto(id = 100, chatId = CHAT, senderId = PEER, reactions = listOf(ReactionDto(PEER, "👍")), reactionSeq = 4),
            live = false,
        )
        assertThat(messageDao.findByServerId(100L)!!.reactionSeq).isEqualTo(6L)

        // Embedded reactions never move the chat cursor (protocol:
        // never derive it from held messages).
        assertThat(chatDao.maxReactionSeq(CHAT)).isEqualTo(0L)
    }

    @Test
    fun toggleReactionAppliesOptimisticallyThenTheAuthoritativeState() = runTest(dispatcher) {
        val repository = newRepository()
        insertChat()
        insertServerMessage(100)
        val gate = CompletableDeferred<ApiResult<MessageReactionStateDto>>()
        chatApi.putReactionHandler = { _, _, _ -> gate.await() }

        val toggle = repoScope.launch { repository.toggleReaction(CHAT, 100L, "❤️") }
        runCurrent()

        // REST still in flight — the chip is already visible, and the
        // guard seq is untouched so the response can still land.
        val optimistic = messageDao.findByServerId(100L)!!
        assertThat(ReactionsCodec.decode(optimistic.reactionsJson)).containsExactly(ReactionDto(ME, "❤️"))
        assertThat(optimistic.reactionSeq).isEqualTo(0L)
        assertThat(chatApi.putReactions).containsExactly(Triple(CHAT, 100L, "❤️"))

        gate.complete(ApiResult.Ok(reactionState(100L, 7L, listOf(ReactionDto(ME, "❤️")))))
        advanceUntilIdle()

        val row = messageDao.findByServerId(100L)!!
        assertThat(ReactionsCodec.decode(row.reactionsJson)).containsExactly(ReactionDto(ME, "❤️"))
        assertThat(row.reactionSeq).isEqualTo(7L)
        assertThat(chatDao.maxReactionSeq(CHAT)).isEqualTo(7L)
        assertThat(toggle.isCompleted).isTrue()
    }

    @Test
    fun togglingMyCurrentEmojiDeletesInsteadOfPuts() = runTest(dispatcher) {
        val repository = newRepository()
        insertChat()
        insertServerMessage(100)
        messageDao.applyReactionState(
            100L,
            ReactionsCodec.encode(listOf(ReactionDto(ME, "❤️"), ReactionDto(PEER, "👍"))),
            5L,
        )
        chatApi.deleteReactionHandler = { _, _ ->
            ApiResult.Ok(reactionState(100L, 8L, listOf(ReactionDto(PEER, "👍"))))
        }

        repository.toggleReaction(CHAT, 100L, "❤️")

        assertThat(chatApi.deletedReactions).containsExactly(CHAT to 100L)
        assertThat(chatApi.putReactions).isEmpty()
        val row = messageDao.findByServerId(100L)!!
        assertThat(ReactionsCodec.decode(row.reactionsJson)).containsExactly(ReactionDto(PEER, "👍"))
        assertThat(row.reactionSeq).isEqualTo(8L)
        assertThat(chatDao.maxReactionSeq(CHAT)).isEqualTo(8L)
    }

    @Test
    fun toggleReactionRevertsTheOptimisticChangeOnApiFailure() = runTest(dispatcher) {
        val repository = newRepository()
        insertChat()
        insertServerMessage(100)
        val before = ReactionsCodec.encode(listOf(ReactionDto(PEER, "❤️")))
        messageDao.applyReactionState(100L, before, 5L)
        val gate = CompletableDeferred<ApiResult<MessageReactionStateDto>>()
        chatApi.putReactionHandler = { _, _, _ -> gate.await() }

        val toggle = repoScope.launch { repository.toggleReaction(CHAT, 100L, "👍") }
        runCurrent()

        // Optimistic state really was applied…
        assertThat(ReactionsCodec.decode(messageDao.findByServerId(100L)!!.reactionsJson))
            .containsExactly(ReactionDto(PEER, "❤️"), ReactionDto(ME, "👍"))

        gate.complete(ApiResult.HttpError(404, "message_not_found", "gone"))
        advanceUntilIdle()

        // …and rolled back wholesale; no retry (mirrors postRead).
        val row = messageDao.findByServerId(100L)!!
        assertThat(row.reactionsJson).isEqualTo(before)
        assertThat(row.reactionSeq).isEqualTo(5L)
        assertThat(chatDao.maxReactionSeq(CHAT)).isEqualTo(0L) // cursor untouched
        assertThat(toggle.isCompleted).isTrue()
        assertThat(chatApi.putReactions).hasSize(1)
    }

    @Test
    fun catchUpReactionsAdvancesTheCursorEvenWhenNoMessageIsHeld() = runTest(dispatcher) {
        val repository = newRepository()
        insertChat()
        chatApi.reactionsHandler = { _, afterSeq, _ ->
            assertThat(afterSeq).isEqualTo(0L)
            ApiResult.Ok(
                me.nettrash.familyconnect.data.net.dto.ReactionsCatchUpResponse(
                    listOf(reactionState(999L, 12L, listOf(ReactionDto(PEER, "❤️")))),
                ),
            )
        }

        repository.catchUpReactions(CHAT, 0L)

        assertThat(messageDao.findByServerId(999L)).isNull() // dropped silently
        assertThat(chatDao.maxReactionSeq(CHAT)).isEqualTo(12L) // cursor advanced anyway
        assertThat(chatApi.reactionsCalls).isEqualTo(1) // short page ended the loop
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
