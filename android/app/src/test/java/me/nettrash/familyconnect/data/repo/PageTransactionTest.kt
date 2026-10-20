/*
 * PageTransactionTest.kt
 * Family Connect (Android)
 *
 * A page of catch-up or history is committed as ONE transaction
 * (MessageRepository.inPage, data/db/DbTransactions.kt): the chat sees it
 * land as one change rather than a row at a time, the rows are the rows they
 * always were, and the staged media files a page frees are removed only once
 * the rows that named them are committed gone.
 */

package me.nettrash.familyconnect.data.repo

import androidx.room.withTransaction
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.db.DbTransactions
import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.data.db.MessageStatus
import me.nettrash.familyconnect.data.db.PendingAttachmentEntity
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.MessagesResponse
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
import java.io.File

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class PageTransactionTest {

    private companion object {
        const val ME = 7L
        const val PEER = 9L
        const val CHAT = 42L
        const val NOW = 1_000_000L
        const val OWN = "c-own-1"
    }

    private val dispatcher = StandardTestDispatcher()
    // Foreground, for the reason MessageRepositoryTest gives.
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private lateinit var db: AppDatabase
    private lateinit var chatApi: FakeChatApi
    private lateinit var settings: FakeSettingsRepository

    @Before
    fun setUp() {
        db = createTestDb(dispatcher)
        chatApi = FakeChatApi()
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

    private fun TestScope.newRepository(
        transactions: DbTransactions,
        database: AppDatabase = db,
    ): MessageRepository {
        val socket = FakeChatSocket()
        val repository = MessageRepository(
            appContext = RuntimeEnvironment.getApplication(),
            chatApi = chatApi,
            attachmentApi = FakeAttachmentApi(),
            messageDao = database.messageDao(),
            chatDao = database.chatDao(),
            socket = socket,
            settings = settings,
            chatRepository = testChatRepository(chatApi, database.chatDao(), database.messageDao(), socket, repoScope),
            posterCache = FakePosterCache(),
            scope = repoScope,
            clock = Clock { NOW },
            pendingAttachmentDao = database.pendingAttachmentDao(),
            staging = MediaStaging(RuntimeEnvironment.getApplication()),
            transactions = transactions,
        )
        runCurrent()
        return repository
    }

    private fun servePage(vararg ids: Long) {
        chatApi.messagesHandler = { _, _, _, _ ->
            ApiResult.Ok(MessagesResponse(ids.map { messageDto(id = it, chatId = CHAT, senderId = PEER) }))
        }
    }

    @Test
    fun aCatchUpPageLandsAsOneChangeOfTheChatsWindow() = runTest(dispatcher) {
        val repository = newRepository(DbTransactions.of(db))
        val windows = mutableListOf<List<MessageEntity>>()
        val watcher = repoScope.launch { db.messageDao().observeMessages(CHAT, 100).collect { windows += it } }
        advanceUntilIdle()
        assertThat(windows).hasSize(1) // the empty chat, before anything lands
        servePage(*(101L..140L).toList().toLongArray())

        val page = repository.catchUp(CHAT, afterId = 100, limit = 50)
        advanceUntilIdle()

        assertThat(page?.size).isEqualTo(40)
        assertThat(page?.maxServerId).isEqualTo(140L)
        // ONE more window, holding the whole page — not forty, one row each.
        assertThat(windows).hasSize(2)
        assertThat(windows.last().map { it.serverId }).isEqualTo((140L downTo 101L).toList())
        watcher.cancel()
    }

    /** What the chat's window holds in [database] right now. */
    private fun TestScope.window(database: AppDatabase): List<MessageEntity> {
        var rows: List<MessageEntity> = emptyList()
        val job = repoScope.launch { database.messageDao().observeMessages(CHAT, 100).collect { rows = it } }
        advanceUntilIdle()
        job.cancel()
        return rows
    }

    /**
     * The contrast the test above is about: the same page with no
     * transaction is a new window per row — which is what the open chat
     * used to rebuild itself for, forty times over, mid-scroll.
     */
    @Test
    fun withoutTheTransactionEveryRowWasAWindowOfItsOwn() = runTest(dispatcher) {
        val repository = newRepository(DbTransactions.Direct)
        val windows = mutableListOf<List<MessageEntity>>()
        val watcher = repoScope.launch { db.messageDao().observeMessages(CHAT, 100).collect { windows += it } }
        advanceUntilIdle()
        servePage(*(101L..140L).toList().toLongArray())

        repository.catchUp(CHAT, afterId = 100, limit = 50)
        advanceUntilIdle()

        assertThat(windows.size).isGreaterThan(2)
        assertThat(windows.last().map { it.serverId }).isEqualTo((140L downTo 101L).toList())
        watcher.cancel()
    }

    @Test
    fun theRowsAPageWritesAreTheSameWithAndWithoutTheTransaction() = runTest(dispatcher) {
        servePage(201, 202, 203)
        newRepository(DbTransactions.Direct).catchUp(CHAT, afterId = 200, limit = 50)
        advanceUntilIdle()
        val direct = window(db)

        val second = createTestDb(dispatcher)
        try {
            newRepository(DbTransactions.of(second), second).catchUp(CHAT, afterId = 200, limit = 50)
            advanceUntilIdle()
            val batched = window(second)

            assertThat(batched).hasSize(3)
            assertThat(batched).isEqualTo(direct)
        } finally {
            second.close()
        }
    }

    /** My own optimistic send, its media still staged, as a lost-ack echo will find it. */
    private suspend fun stageOwnSend(): File {
        db.messageDao().insertIgnore(
            listOf(
                MessageEntity(
                    clientMsgId = OWN,
                    serverId = null,
                    chatId = CHAT,
                    senderId = ME,
                    body = "",
                    createdAt = NOW,
                    status = MessageStatus.SENDING,
                ),
            ),
        )
        val directory = File(RuntimeEnvironment.getApplication().filesDir, "outbox/page-test").apply { mkdirs() }
        val staged = File(directory, "photo.jpg").apply { writeBytes(byteArrayOf(1, 2, 3)) }
        db.pendingAttachmentDao().insertAll(
            listOf(
                PendingAttachmentEntity(
                    clientMsgId = OWN,
                    position = 0,
                    localPath = staged.absolutePath,
                    previewPath = null,
                    mime = "image/jpeg",
                    kind = "photo",
                    sizeBytes = 3,
                    width = 1,
                    height = 1,
                    durationMs = null,
                    name = null,
                    latitude = null,
                    longitude = null,
                    accuracyM = null,
                ),
            ),
        )
        return staged
    }

    private fun serveOwnEcho() {
        chatApi.messagesHandler = { _, _, _, _ ->
            ApiResult.Ok(MessagesResponse(listOf(messageDto(id = 300, chatId = CHAT, senderId = ME, clientMsgId = OWN, body = ""))))
        }
    }

    @Test
    fun aCommittedPageRemovesTheStagedFilesItsAckFreed() = runTest(dispatcher) {
        val repository = newRepository(DbTransactions.of(db))
        val staged = stageOwnSend()
        serveOwnEcho()

        repository.catchUp(CHAT, afterId = 299, limit = 50)
        advanceUntilIdle()

        assertThat(db.messageDao().findByClientMsgId(OWN)?.serverId).isEqualTo(300L)
        assertThat(db.pendingAttachmentDao().itemsFor(OWN)).isEmpty()
        assertThat(staged.exists()).isFalse()
    }

    @Test
    fun aPageThatRollsBackLeavesTheStagedFilesItsRowsStillName() = runTest(dispatcher) {
        // The page applies, and then its transaction fails: everything it
        // wrote is undone — so the pending rows are back, and the files
        // they name must still be there for them.
        val failing = object : DbTransactions {
            override suspend fun <R> run(block: suspend () -> R): R = db.withTransaction {
                block()
                throw IllegalStateException("the commit failed")
            }
        }
        val repository = newRepository(failing)
        val staged = stageOwnSend()
        serveOwnEcho()

        val thrown = runCatching { repository.catchUp(CHAT, afterId = 299, limit = 50) }.exceptionOrNull()
        advanceUntilIdle()

        assertThat(thrown).isInstanceOf(IllegalStateException::class.java)
        assertThat(db.messageDao().findByClientMsgId(OWN)?.serverId).isNull()
        assertThat(db.pendingAttachmentDao().itemsFor(OWN)).hasSize(1)
        assertThat(staged.exists()).isTrue()
    }
}
