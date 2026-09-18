/*
 * MessageRepositoryThreadTest.kt
 * Family Connect (Android) — tests
 *
 * How the chain lives in the store (docs/protocol.md, "Threads").
 *
 * The live rule is the one that can go wrong in both directions: the
 * root's own frame is never re-sent, so a client holding the root counts
 * each reply as it ARRIVES — and only then. Counting a page's replies
 * double-counts (the root's recomputed count already includes them);
 * counting a re-delivery double-counts (the REST echo and the socket frame
 * are two copies of one reply); counting nothing leaves "1 reply" under a
 * message the family has answered five times.
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
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.db.ChatDao
import me.nettrash.familyconnect.data.db.ChatEntity
import me.nettrash.familyconnect.data.db.MessageDao
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.MessageDto
import me.nettrash.familyconnect.data.net.dto.MessagesResponse
import me.nettrash.familyconnect.data.net.dto.ReplyToDto
import me.nettrash.familyconnect.data.net.ws.ServerFrame
import me.nettrash.familyconnect.data.settings.SettingsState
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
class MessageRepositoryThreadTest {

    private companion object {
        const val ME = 7L
        const val PEER = 9L
        const val CHAT = 42L
        const val NOW = 1_000_000L
        const val ROOT = 100L
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
        val chatRepository = testChatRepository(chatApi, chatDao, messageDao, socket, repoScope)
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

    private fun root(replyCount: Long? = null): MessageDto =
        messageDto(ROOT, senderId = ME, body = "Dinner at 7?").copy(replyCount = replyCount)

    private fun reply(id: Long, to: Long = ROOT, senderId: Long = PEER): MessageDto =
        messageDto(
            id,
            senderId = senderId,
            body = "reply $id",
            replyTo = ReplyToDto(messageId = to, senderId = ME, excerpt = "Dinner at 7?"),
        ).copy(threadRootId = ROOT)

    private suspend fun replyCountOfRoot(): Long = messageDao.findByServerId(ROOT)!!.replyCount

    @Test
    fun aLiveReplyRaisesTheCachedRootOnce() = runTest(dispatcher) {
        val repository = newRepository()
        insertFamilyChat()
        repository.applyServerMessage(root(), live = false)

        repository.applyServerMessage(reply(101), live = true)
        assertThat(replyCountOfRoot()).isEqualTo(1)
        assertThat(messageDao.findByServerId(101)!!.threadRootId).isEqualTo(ROOT)

        // The same reply again, on either path, is the same reply.
        repository.applyServerMessage(reply(101), live = true)
        repository.applyServerMessage(reply(101), live = false)
        assertThat(replyCountOfRoot()).isEqualTo(1)
    }

    @Test
    fun aPageNeverBumpsAndAServerCopyOfTheRootOverwrites() = runTest(dispatcher) {
        val repository = newRepository()
        insertFamilyChat()
        repository.applyServerMessage(root(replyCount = 2), live = false)
        assertThat(replyCountOfRoot()).isEqualTo(2)

        // Two replies from a history page: first sight, but not live — the
        // root's own count already held them.
        repository.applyServerMessage(reply(101), live = false)
        repository.applyServerMessage(reply(102), live = false)
        assertThat(replyCountOfRoot()).isEqualTo(2)

        // A live one on top, then a fresh copy of the root with the truth.
        repository.applyServerMessage(reply(103), live = true)
        assertThat(replyCountOfRoot()).isEqualTo(3)
        repository.applyServerMessage(root(replyCount = 5), live = false)
        assertThat(replyCountOfRoot()).isEqualTo(5)
        // ABSENT on a later copy means nobody has answered: retention took
        // the replies, and the next copy of the root says so.
        repository.applyServerMessage(root(), live = false)
        assertThat(replyCountOfRoot()).isEqualTo(0)
    }

    @Test
    fun aReplyAheadOfItsRootIsHarmless() = runTest(dispatcher) {
        val repository = newRepository()
        insertFamilyChat()
        repository.applyServerMessage(reply(101), live = true)
        assertThat(messageDao.findByServerId(101)!!.threadRootId).isEqualTo(ROOT)
        assertThat(messageDao.findByServerId(ROOT)).isNull()
        repository.applyServerMessage(root(replyCount = 1), live = false)
        assertThat(replyCountOfRoot()).isEqualTo(1)
    }

    @Test
    fun anOwnReplyKnowsItsRootBeforeTheAckAndIsCountedOnItOnce() = runTest(dispatcher) {
        val repository = newRepository()
        insertFamilyChat()
        repository.applyServerMessage(root(), live = false)
        repository.applyServerMessage(reply(101), live = false)

        // Quoting the REPLY: the root is still the top of the chain, derived
        // from the cached row.
        repository.send(CHAT, "Pizza then?", ReplyToDto(messageId = 101, senderId = PEER, excerpt = "reply 101"))
        runCurrent()
        val pending = messageDao.observeThread(ROOT).first().single { it.serverId == null }
        assertThat(pending.threadRootId).isEqualTo(ROOT)
        assertThat(replyCountOfRoot()).isEqualTo(0)

        // The server's echo of the send: the ack, counted once.
        val echo = messageDto(
            102,
            senderId = ME,
            clientMsgId = pending.clientMsgId,
            body = "Pizza then?",
            replyTo = ReplyToDto(messageId = 101, senderId = PEER, excerpt = "reply 101"),
        ).copy(threadRootId = ROOT)
        repository.applyServerMessage(echo, live = true)
        assertThat(replyCountOfRoot()).isEqualTo(1)
        assertThat(messageDao.findByServerId(102)!!.threadRootId).isEqualTo(ROOT)
        // The socket's copy of the same send, on the sender's own connection.
        repository.applyServerMessage(echo, live = true)
        assertThat(replyCountOfRoot()).isEqualTo(1)
        // And the socket's ACK for it, arriving after the REST echo already
        // gave the row its id: the same send a third time, through the
        // one path that reaches ackMessage with the id already set.
        socket.emit(ServerFrame.Ack(clientMsgId = pending.clientMsgId, message = echo))
        runCurrent()
        assertThat(replyCountOfRoot()).isEqualTo(1)
    }

    @Test
    fun anOwnReplyToAnUnknownQuoteDerivesNoRoot() = runTest(dispatcher) {
        val repository = newRepository()
        insertFamilyChat()
        repository.send(CHAT, "Pizza then?", ReplyToDto(messageId = 555, senderId = PEER, excerpt = "?"))
        runCurrent()
        val pending = messageDao.observeMessages(CHAT, 10).first().single()
        assertThat(pending.threadRootId).isNull()
    }

    @Test
    fun loadThreadPagesUntilAShortPageAndFillsTheStore() = runTest(dispatcher) {
        val repository = newRepository()
        insertFamilyChat()
        chatApi.threadHandler = { _, afterId, _ ->
            when (afterId) {
                null -> ApiResult.Ok(MessagesResponse(listOf(root(replyCount = 2), reply(101))))
                101L -> ApiResult.Ok(MessagesResponse(listOf(reply(102, to = 101))))
                else -> ApiResult.Ok(MessagesResponse(emptyList()))
            }
        }

        val completed = repository.loadThread(CHAT, ROOT, limit = 2)

        assertThat(completed).isTrue()
        assertThat(chatApi.threadCalls).containsExactly(ROOT to null, ROOT to 101L).inOrder()
        val chain = messageDao.observeThread(ROOT).first()
        assertThat(chain.map { it.serverId }).containsExactly(ROOT, 101L, 102L).inOrder()
        // The page's count, not the page's replies.
        assertThat(replyCountOfRoot()).isEqualTo(2)
    }

    /**
     * The thread read reconciling THIS device's own pending reply: the page
     * is root-first and the root's copy already counts the reply, so the
     * fold must not count it again — it is not a live arrival.
     */
    @Test
    fun aThreadPageReconcilingAPendingOwnReplyDoesNotDoubleCount() = runTest(dispatcher) {
        val repository = newRepository()
        insertFamilyChat()
        repository.applyServerMessage(root(), live = false)
        repository.send(CHAT, "Pizza then?", ReplyToDto(messageId = ROOT, senderId = ME, excerpt = "Dinner at 7?"))
        runCurrent()
        val pending = messageDao.observeThread(ROOT).first().single { it.serverId == null }
        val echo = messageDto(
            102,
            senderId = ME,
            clientMsgId = pending.clientMsgId,
            body = "Pizza then?",
            replyTo = ReplyToDto(messageId = ROOT, senderId = ME, excerpt = "Dinner at 7?"),
        ).copy(threadRootId = ROOT)
        chatApi.threadHandler = { _, _, _ ->
            ApiResult.Ok(MessagesResponse(listOf(root(replyCount = 1), echo)))
        }

        assertThat(repository.loadThread(CHAT, ROOT)).isTrue()

        assertThat(replyCountOfRoot()).isEqualTo(1)
        assertThat(messageDao.findByServerId(102)!!.clientMsgId).isEqualTo(pending.clientMsgId)
    }

    /**
     * The `after_id` catch-up stands in for the frames missed while away:
     * by construction it delivers only what is newer than everything held,
     * so a cached root's count cannot yet include the reply, and it counts.
     */
    @Test
    fun aCatchUpReplyCountsAsLive() = runTest(dispatcher) {
        val repository = newRepository()
        insertFamilyChat()
        repository.applyServerMessage(root(), live = false)
        repository.applyServerMessage(reply(101), live = false, chainLive = true)
        assertThat(replyCountOfRoot()).isEqualTo(1)
        // And a history page, which is older than what is held, does not.
        repository.applyServerMessage(reply(99), live = false)
        assertThat(replyCountOfRoot()).isEqualTo(1)
    }

    /**
     * The thread read is no part of catch-up: rows it fetches outside the
     * window must not move the paging cursors, or the next history page
     * would skip everything between them and the window — for good.
     */
    @Test
    fun aThreadReadLeavesThePagingCursorsAlone() = runTest(dispatcher) {
        val repository = newRepository()
        insertFamilyChat()
        // The contiguous window: 200..210.
        for (id in 200L..210L) repository.applyServerMessage(messageDto(id), live = false)
        assertThat(messageDao.oldestServerId(CHAT)).isEqualTo(200L)
        assertThat(messageDao.maxServerId(CHAT)).isEqualTo(210L)
        // A chain that began long before the window and was answered after it.
        chatApi.threadHandler = { _, _, _ ->
            ApiResult.Ok(MessagesResponse(listOf(root(replyCount = 1), reply(300))))
        }

        assertThat(repository.loadThread(CHAT, ROOT)).isTrue()

        assertThat(messageDao.findByServerId(ROOT)).isNotNull()
        assertThat(messageDao.findByServerId(300)).isNotNull()
        assertThat(messageDao.oldestServerId(CHAT)).isEqualTo(200L)
        assertThat(messageDao.maxServerId(CHAT)).isEqualTo(210L)
        // Once a page reaches the root, it is inside the window and counts.
        repository.applyServerMessage(root(replyCount = 1), live = false)
        assertThat(messageDao.oldestServerId(CHAT)).isEqualTo(ROOT)
    }

    /** The catch-up path itself, not only the flag: a page fetched by [MessageRepository.catchUp] counts. */
    @Test
    fun theCatchUpPathCountsARepliedRoot() = runTest(dispatcher) {
        val repository = newRepository()
        insertFamilyChat()
        repository.applyServerMessage(root(), live = false)
        chatApi.messagesHandler = { _, _, afterId, _ ->
            if (afterId == ROOT) ApiResult.Ok(MessagesResponse(listOf(reply(101)))) else ApiResult.Ok(MessagesResponse(emptyList()))
        }

        val page = repository.catchUp(CHAT, afterId = ROOT, limit = 50)

        assertThat(page?.size).isEqualTo(1)
        assertThat(replyCountOfRoot()).isEqualTo(1)
    }

    /**
     * A poll answering a message in a chain: its optimistic row carries the
     * root like a text reply's, so the thread screen shows it at once.
     */
    @Test
    fun aPendingPollReplyIsInTheThread() = runTest(dispatcher) {
        val repository = newRepository()
        insertFamilyChat()
        repository.applyServerMessage(root(), live = false)
        repository.sendPoll(
            CHAT,
            "Pizza or pasta?",
            listOf("Pizza", "Pasta"),
            ReplyToDto(messageId = ROOT, senderId = ME, excerpt = "Dinner at 7?"),
        )
        runCurrent()
        val pending = messageDao.observeThread(ROOT).first().single { it.serverId == null }
        assertThat(pending.threadRootId).isEqualTo(ROOT)
        assertThat(pending.pollJson).isNotNull()
    }

    @Test
    fun aFailedThreadReadAnswersFalseAndKeepsTheCache() = runTest(dispatcher) {
        val repository = newRepository()
        insertFamilyChat()
        repository.applyServerMessage(root(replyCount = 1), live = false)
        chatApi.threadHandler = { _, _, _ -> ApiResult.NetworkError(IllegalStateException("offline")) }

        assertThat(repository.loadThread(CHAT, ROOT)).isFalse()
        assertThat(replyCountOfRoot()).isEqualTo(1)
    }
}
