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
 *
 * …and, since the badge went onto the app icon, the two ways it is left
 * lit for messages that HAVE been read:
 *
 *   - the read marker `GET /chats` now reports being applied as an
 *     assignment rather than a maximum, so a response in flight while the
 *     reader reads walks it backwards.
 *   - a chat read on another of this person's devices leaving this one's
 *     tray notification — which on Android IS the badge — up, because no
 *     local count ever moved for anything to notice.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.db.ChatDao
import me.nettrash.familyconnect.data.db.ChatEntity
import me.nettrash.familyconnect.data.db.MessageDao
import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.data.db.MessageStatus
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.ChatDto
import me.nettrash.familyconnect.data.net.dto.ChatListItemDto
import me.nettrash.familyconnect.data.net.dto.ChatsResponse
import me.nettrash.familyconnect.data.net.ws.ClientFrame
import me.nettrash.familyconnect.data.push.PushNotifications
import me.nettrash.familyconnect.testutil.FakeChatApi
import me.nettrash.familyconnect.testutil.FakeChatSocket
import me.nettrash.familyconnect.testutil.testChatRepository
import me.nettrash.familyconnect.testutil.createTestDb
import me.nettrash.familyconnect.testutil.messageDto
import org.junit.After
import org.junit.Before
import org.junit.Test
import android.app.NotificationManager
import android.content.Context
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class ChatRepositoryTest {

    private companion object {
        const val CHAT = 42L
        const val PEER = 9L

        /** This device's owner — the person whose badge is at stake. */
        const val ME = 7L
    }

    // RuntimeEnvironment, not androidx.test ApplicationProvider — the
    // test classpath deliberately has no androidx.test (see TestDb.kt).
    private val context: Context = RuntimeEnvironment.getApplication()
    private val notificationManager: NotificationManager =
        context.getSystemService(NotificationManager::class.java)

    private val dispatcher = StandardTestDispatcher()
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private lateinit var db: AppDatabase
    private lateinit var chatDao: ChatDao
    private lateinit var messageDao: MessageDao
    private lateinit var chatApi: FakeChatApi
    private lateinit var socket: FakeChatSocket
    private lateinit var repository: ChatRepository

    @Before
    fun setUp() {
        db = createTestDb(dispatcher)
        chatDao = db.chatDao()
        messageDao = db.messageDao()
        chatApi = FakeChatApi()
        socket = FakeChatSocket()
        repository = testChatRepository(chatApi, chatDao, messageDao, socket, repoScope)
    }

    @After
    fun tearDown() {
        repoScope.cancel()
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
    private fun chatsResponse(
        unreadCount: Int,
        countedThrough: Long = 0L,
        lastReadMessageId: Long? = null,
    ) = ApiResult.Ok(
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
                    lastReadMessageId = lastReadMessageId,
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

    // -- refreshChats: MY read marker, and the badge it settles -------------------------
    //
    // `GET /chats` now reports `last_read_message_id` — the caller's OWN
    // marker, shared across every device they own (docs/protocol.md,
    // resync step 2). It is the only thing that can tell THIS device that
    // the chat was read on another one, so it is also what takes this
    // device's tray notification — which on Android IS the badge — down.

    @Test
    fun theChatListsReadMarkerIsAppliedToAChatThatHadNone() = runTest(dispatcher) {
        insertChat(unreadCount = 0)
        chatApi.chatsResult = chatsResponse(unreadCount = 0, lastReadMessageId = 1337L)

        repository.refreshChats()

        assertThat(chatDao.getById(CHAT)!!.myLastReadId).isEqualTo(1337L)
    }

    /**
     * The interleaving the monotonic rule exists for: the response was
     * computed BEFORE the reader read, and lands after. Writing it in
     * would walk the marker backwards, and postRead's guard would then
     * re-report reads the server already holds.
     */
    @Test
    fun aChatListResponseCannotWalkTheReadMarkerBackwards() = runTest(dispatcher) {
        insertChat(unreadCount = 0, myLastReadId = 1400L)
        chatApi.chatsResult = chatsResponse(unreadCount = 0, lastReadMessageId = 1337L)

        repository.refreshChats()

        assertThat(chatDao.getById(CHAT)!!.myLastReadId).isEqualTo(1400L)
    }

    /**
     * `0` is the protocol's "never reported anything here", and an absent
     * field is an older server binary. Both mean the same thing to a
     * monotonic apply: leave what is stored exactly as it is.
     */
    @Test
    fun aZeroOrAbsentMarkerLeavesTheStoredOneAlone() = runTest(dispatcher) {
        insertChat(unreadCount = 0, myLastReadId = 1400L)

        chatApi.chatsResult = chatsResponse(unreadCount = 0, lastReadMessageId = 0L)
        repository.refreshChats()
        assertThat(chatDao.getById(CHAT)!!.myLastReadId).isEqualTo(1400L)

        chatApi.chatsResult = chatsResponse(unreadCount = 0, lastReadMessageId = null)
        repository.refreshChats()
        assertThat(chatDao.getById(CHAT)!!.myLastReadId).isEqualTo(1400L)
    }

    /** The pure rule, without a chat list to carry it. */
    @Test
    fun mergedReadMarkerIsTheMaximumAndNeverInventsOne() {
        assertThat(ChatRepository.mergedReadMarker(stored = null, received = 1337L))
            .isEqualTo(1337L)
        assertThat(ChatRepository.mergedReadMarker(stored = 1400L, received = 1337L))
            .isEqualTo(1400L)
        assertThat(ChatRepository.mergedReadMarker(stored = 1337L, received = 1400L))
            .isEqualTo(1400L)
        // Nobody has read anything here: still nobody, not "message 0".
        assertThat(ChatRepository.mergedReadMarker(stored = null, received = 0L)).isNull()
        assertThat(ChatRepository.mergedReadMarker(stored = null, received = null)).isNull()
    }

    /**
     * READ ON THE TABLET, badge still lit on the phone — the whole point
     * of the marker landing here. No local count moved (this device never
     * saw the message), so nothing transitions and only the SERVER's
     * answer can settle it.
     */
    @Test
    fun aResyncFindingNothingUnreadTakesTheChatsNotificationDown() = runTest(dispatcher) {
        insertChat(unreadCount = 0)
        postTrayNotification()
        chatApi.chatsResult = chatsResponse(unreadCount = 0, lastReadMessageId = 1337L)

        repository.refreshChats()
        runCurrent()

        assertThat(activeTags()).isEmpty()
    }

    /** The other half: a chat that really does have unread keeps its badge. */
    @Test
    fun aResyncStillReportingUnreadLeavesTheNotificationUp() = runTest(dispatcher) {
        insertChat(unreadCount = 0)
        postTrayNotification()
        chatApi.chatsResult = chatsResponse(unreadCount = 2, countedThrough = 20L)

        repository.refreshChats()
        runCurrent()

        assertThat(activeTags()).containsExactly(PushNotifications.chatTag(CHAT))
    }

    /**
     * A chat read on another device that has ALSO had a message arrive
     * since, live, on this one. The server's count is 0 because it
     * answered before the message existed; the notification is stale all
     * the same, because the server never pushes to a device holding a
     * live socket — so the message that made the merged count 1 has no
     * notification of its own to protect.
     */
    @Test
    fun aLiveArrivalDoesNotKeepAStaleNotificationAlive() = runTest(dispatcher) {
        insertChat(unreadCount = 0)
        postTrayNotification()
        chatApi.chatsResult = chatsResponse(unreadCount = 0, lastReadMessageId = 1337L)
        val gate = CompletableDeferred<Unit>()
        chatApi.chatsGate = gate

        val refresh = launch { repository.refreshChats() }
        runCurrent()
        repository.bumpUnread(CHAT, messageId = 1400L)
        runCurrent()
        gate.complete(Unit)
        refresh.join()
        runCurrent()

        assertThat(chatDao.getById(CHAT)!!.unreadCount).isEqualTo(1)
        assertThat(activeTags()).isEmpty()
    }

    // -- applyMyReadMarker: the same thing arriving as a frame --------------------------

    /**
     * A marker from another device is a THRESHOLD, not an ending: the two
     * messages that arrived after it are still unread, and clearing the
     * count would drop them from the badge for good — `GET /chats`
     * computes from the same marker and would agree with the wrong
     * answer.
     */
    @Test
    fun aMarkerFromAnotherDeviceRecountsRatherThanClears() = runTest(dispatcher) {
        insertChat(unreadCount = 3)
        insertInbound(id = 10L)
        insertInbound(id = 11L)
        insertInbound(id = 12L)
        postTrayNotification()

        repository.applyMyReadMarker(CHAT, lastReadMessageId = 10L, myUserId = ME)
        runCurrent()

        assertThat(chatDao.getById(CHAT)!!.myLastReadId).isEqualTo(10L)
        assertThat(chatDao.getById(CHAT)!!.unreadCount).isEqualTo(2)
        // Still something to look at, so the badge stays.
        assertThat(activeTags()).containsExactly(PushNotifications.chatTag(CHAT))
    }

    @Test
    fun aMarkerPastEverythingClearsTheCountAndTheNotification() = runTest(dispatcher) {
        insertChat(unreadCount = 3)
        insertInbound(id = 10L)
        insertInbound(id = 11L)
        postTrayNotification()

        repository.applyMyReadMarker(CHAT, lastReadMessageId = 11L, myUserId = ME)
        runCurrent()

        assertThat(chatDao.getById(CHAT)!!.unreadCount).isEqualTo(0)
        assertThat(activeTags()).isEmpty()
    }

    /**
     * My OWN messages were never unread, so a marker that only clears
     * them changes nothing — the server's `unread_count` excludes them
     * (push_payload.rs, `build_message_unread_query`) and so must this.
     */
    @Test
    fun myOwnMessagesAreNotCountedAgainstMe() = runTest(dispatcher) {
        insertChat(unreadCount = 1)
        insertInbound(id = 10L)
        insertInbound(id = 11L, senderId = ME)

        repository.applyMyReadMarker(CHAT, lastReadMessageId = 10L, myUserId = ME)

        assertThat(chatDao.getById(CHAT)!!.unreadCount).isEqualTo(0)
    }

    /**
     * A frame that crosses a read happening HERE must not undo it: MAX()
     * keeps the higher marker, and the recount is measured against what
     * is stored afterwards rather than against what arrived.
     */
    @Test
    fun aLowerMarkerFromAnotherDeviceCannotResurrectReadMessages() = runTest(dispatcher) {
        insertChat(unreadCount = 0, myLastReadId = 12L)
        insertInbound(id = 10L)
        insertInbound(id = 11L)
        insertInbound(id = 12L)

        repository.applyMyReadMarker(CHAT, lastReadMessageId = 10L, myUserId = ME)

        assertThat(chatDao.getById(CHAT)!!.myLastReadId).isEqualTo(12L)
        assertThat(chatDao.getById(CHAT)!!.unreadCount).isEqualTo(0)
    }

    // -- tray helpers ------------------------------------------------------------------

    /**
     * Straight at the manager rather than through PushNotifications.show,
     * which is gated on a runtime permission this test process does not
     * hold — the (tag, id) slot is the same one show() posts into, and it
     * is the tag the sweep matches on.
     */
    private fun postTrayNotification(chatId: Long = CHAT) {
        PushNotifications.ensureChannel(context)
        notificationManager.notify(
            PushNotifications.chatTag(chatId),
            PushNotifications.NOTIFICATION_ID,
            PushNotifications.build(
                context,
                title = "Ben",
                body = "Dinner at 7?",
                kind = "message",
                chatId = chatId,
            ),
        )
    }

    private fun activeTags(): List<String?> =
        notificationManager.activeNotifications.map { it.tag }

    private suspend fun insertInbound(id: Long, senderId: Long = PEER) {
        messageDao.insert(
            MessageEntity(
                clientMsgId = "s$id",
                serverId = id,
                chatId = CHAT,
                senderId = senderId,
                body = "hi",
                createdAt = id,
                status = MessageStatus.SENT,
            ),
        )
    }
}
