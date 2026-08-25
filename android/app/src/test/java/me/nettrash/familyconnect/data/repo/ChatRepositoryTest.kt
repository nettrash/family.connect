/*
 * ChatRepositoryTest.kt
 * Family Connect (Android)
 *
 * The two ways an unread badge is destroyed by the plumbing rather than
 * by anybody reading anything:
 *
 *   - postRead advancing myLastReadId for a report the server never
 *     took. The marker mirrors what the SERVER holds, so moving it on a
 *     dropped frame or a 500 silences every later attempt through the
 *     monotonic guard while the server stays behind — and `GET /chats`
 *     keeps re-inflating a count nothing can now clear.
 *   - refreshChats writing back a count the server computed BEFORE a
 *     live message existed. The catch-up path never bumps, so that
 *     message is dropped from the badge for good.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.db.ChatDao
import me.nettrash.familyconnect.data.db.ChatEntity
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.ChatDto
import me.nettrash.familyconnect.data.net.dto.ChatListItemDto
import me.nettrash.familyconnect.data.net.dto.ChatsResponse
import me.nettrash.familyconnect.data.net.ws.ClientFrame
import me.nettrash.familyconnect.testutil.FakeChatApi
import me.nettrash.familyconnect.testutil.FakeChatSocket
import me.nettrash.familyconnect.testutil.createTestDb
import me.nettrash.familyconnect.testutil.messageDto
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class ChatRepositoryTest {

    private companion object {
        const val CHAT = 42L
        const val PEER = 9L
    }

    private val dispatcher = StandardTestDispatcher()
    private lateinit var db: AppDatabase
    private lateinit var chatDao: ChatDao
    private lateinit var chatApi: FakeChatApi
    private lateinit var socket: FakeChatSocket
    private lateinit var repository: ChatRepository

    @Before
    fun setUp() {
        db = createTestDb(dispatcher)
        chatDao = db.chatDao()
        chatApi = FakeChatApi()
        socket = FakeChatSocket()
        repository = ChatRepository(chatApi, chatDao, socket)
    }

    @After
    fun tearDown() {
        db.close()
    }

    private suspend fun insertChat(unreadCount: Int = 0, myLastReadId: Long? = null) {
        chatDao.upsertAll(
            listOf(
                ChatEntity(
                    id = CHAT,
                    kind = "direct",
                    peerUserId = PEER,
                    title = "Chat",
                    unreadCount = unreadCount,
                    myLastReadId = myLastReadId,
                    peerLastReadId = null,
                    lastMessageBody = null,
                    lastMessageAt = null,
                    lastMessageSenderId = null,
                ),
            ),
        )
    }

    /**
     * [countedThrough] is the id of the newest message the server knew
     * about when it answered — the `last_message` every real response
     * carries. It is what says which live arrivals the `unreadCount`
     * already includes, so the correction below is only as good as this
     * being set the way the server would set it.
     */
    private fun chatsResponse(unreadCount: Int, countedThrough: Long = 0L) = ApiResult.Ok(
        ChatsResponse(
            listOf(
                ChatListItemDto(
                    chat = ChatDto(id = CHAT, kind = "direct", title = "Chat", peerUserId = PEER),
                    lastMessage = if (countedThrough == 0L) {
                        null
                    } else {
                        messageDto(id = countedThrough, chatId = CHAT, senderId = PEER)
                    },
                    unreadCount = unreadCount,
                ),
            ),
        ),
    )

    // -- postRead: the marker follows the SERVER ---------------------------------------

    @Test
    fun aReadTheServerRefusedLeavesTheMarkerUnadvanced() = runTest(dispatcher) {
        insertChat(unreadCount = 3)
        chatApi.postReadResult = ApiResult.HttpError(500, null, null)

        repository.postRead(CHAT, 11L)

        assertThat(chatApi.postedReads).containsExactly(CHAT to 11L)
        // Unadvanced: the server does not have it, so neither do we.
        assertThat(chatDao.getById(CHAT)!!.myLastReadId).isNull()
        // …but the badge still goes, because the user IS looking at it.
        assertThat(chatDao.getById(CHAT)!!.unreadCount).isEqualTo(0)
    }

    @Test
    fun aReadTheSocketDroppedFallsBackToRestAndIsRetriedLater() = runTest(dispatcher) {
        insertChat(unreadCount = 1)
        socket.setOpen(true)
        socket.sendSucceeds = false // frame never leaves the client
        chatApi.postReadResult = ApiResult.NetworkError(IllegalStateException("offline"))

        repository.postRead(CHAT, 11L)
        assertThat(chatDao.getById(CHAT)!!.myLastReadId).isNull()

        // The monotonic guard must not have latched on the failed attempt.
        socket.sendSucceeds = true
        repository.postRead(CHAT, 11L)

        assertThat(socket.sent).containsExactly(ClientFrame.Read(CHAT, 11L))
        assertThat(chatDao.getById(CHAT)!!.myLastReadId).isEqualTo(11L)
    }

    /**
     * The state the old order could strand a chat in: the marker already
     * at the newest id (so the guard returns immediately) while the badge
     * says 3, because `GET /chats` re-inflated it from a server that
     * never got the read. Clearing has to happen before the guard.
     */
    @Test
    fun aReadBelowTheMarkerStillClearsTheBadge() = runTest(dispatcher) {
        insertChat(unreadCount = 3, myLastReadId = 11L)

        repository.postRead(CHAT, 11L)

        assertThat(chatDao.getById(CHAT)!!.unreadCount).isEqualTo(0)
        // Nothing re-sent — that part of the guard still holds.
        assertThat(chatApi.postedReads).isEmpty()
        assertThat(socket.sent).isEmpty()
    }

    // -- refreshChats: what arrives mid-flight survives ---------------------------------

    @Test
    fun aLiveBumpDuringAChatListRefreshIsAddedBackToTheServerCount() = runTest(dispatcher) {
        insertChat(unreadCount = 0)
        // The server counted 2, through message 20.
        chatApi.chatsResult = chatsResponse(unreadCount = 2, countedThrough = 20L)
        val gate = CompletableDeferred<Unit>()
        chatApi.chatsGate = gate

        val refresh = launch { repository.refreshChats() }
        runCurrent()

        // The socket delivers 21 while GET /chats is still in flight —
        // newer than what the response counted, so it is genuinely on top.
        repository.bumpUnread(CHAT, messageId = 21L)
        runCurrent()

        gate.complete(Unit)
        refresh.join()

        assertThat(chatDao.getById(CHAT)!!.unreadCount).isEqualTo(3)
    }

    @Test
    fun aLiveBumpBeforeTheRefreshIsNotDoubleCounted() = runTest(dispatcher) {
        insertChat(unreadCount = 0)
        chatApi.chatsResult = chatsResponse(unreadCount = 2, countedThrough = 20L)

        // Already counted by the server: nothing arrived across the await,
        // so nothing is added on top.
        repository.bumpUnread(CHAT, messageId = 19L)
        repository.bumpUnread(CHAT, messageId = 20L)
        repository.refreshChats()

        assertThat(chatDao.getById(CHAT)!!.unreadCount).isEqualTo(2)
    }

    /**
     * The interleaving a bare counter cannot see, and the one that made
     * the badge lie.
     *
     * The server COMMITS a message and only then broadcasts it, and it
     * serves the in-flight `GET /chats` from the same database — so the
     * order can perfectly well be: request sent, message 21 committed,
     * the chat-list query runs and counts it, THEN the socket frame
     * arrives here. Every one of those bumps happened "during the
     * request", so a counter diff adds 21 a second time and the chat
     * shows 3 unread for 2 unread messages, permanently, until some
     * later refresh nothing races.
     *
     * The response's own `last_message` is what settles it: the server
     * had already reached 21, so 21 has been counted.
     */
    @Test
    fun aMessageTheServerCountedMidRequestIsNotAddedTwice() = runTest(dispatcher) {
        insertChat(unreadCount = 0)
        // Counted 2, and the newest it saw was the very message that is
        // about to arrive on the socket.
        chatApi.chatsResult = chatsResponse(unreadCount = 2, countedThrough = 21L)
        val gate = CompletableDeferred<Unit>()
        chatApi.chatsGate = gate

        val refresh = launch { repository.refreshChats() }
        runCurrent()

        repository.bumpUnread(CHAT, messageId = 21L)
        runCurrent()

        gate.complete(Unit)
        refresh.join()

        assertThat(chatDao.getById(CHAT)!!.unreadCount).isEqualTo(2)
    }

    /**
     * Both at once, which is the case that says the correction is by
     * IDENTITY and not by arithmetic: one arrival the server had counted
     * and one it had not, across the same await.
     */
    @Test
    fun onlyTheArrivalsNewerThanTheServersLastMessageAreAddedBack() = runTest(dispatcher) {
        insertChat(unreadCount = 0)
        chatApi.chatsResult = chatsResponse(unreadCount = 2, countedThrough = 21L)
        val gate = CompletableDeferred<Unit>()
        chatApi.chatsGate = gate

        val refresh = launch { repository.refreshChats() }
        runCurrent()

        repository.bumpUnread(CHAT, messageId = 21L)
        repository.bumpUnread(CHAT, messageId = 22L)
        runCurrent()

        gate.complete(Unit)
        refresh.join()

        assertThat(chatDao.getById(CHAT)!!.unreadCount).isEqualTo(3)
    }

    /**
     * A chat with no messages at all counted nothing, so everything that
     * arrived across the await is new. Without this the `last_message`
     * rule would silently swallow the first message a chat ever gets.
     */
    @Test
    fun aChatWithNoLastMessageCountsEverythingThatArrived() = runTest(dispatcher) {
        insertChat(unreadCount = 0)
        chatApi.chatsResult = chatsResponse(unreadCount = 0)
        val gate = CompletableDeferred<Unit>()
        chatApi.chatsGate = gate

        val refresh = launch { repository.refreshChats() }
        runCurrent()

        repository.bumpUnread(CHAT, messageId = 1L)
        runCurrent()

        gate.complete(Unit)
        refresh.join()

        assertThat(chatDao.getById(CHAT)!!.unreadCount).isEqualTo(1)
    }
}
