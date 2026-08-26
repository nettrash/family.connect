/*
 * UnreadNotificationsTest.kt
 * Family Connect (Android)
 *
 * The badge's source and its clearing, against a real Room store and a
 * real NotificationManager (Robolectric). What is under test is not the
 * arithmetic — UnreadBadgeTest owns that — but the wiring that has to be
 * RUNNING: that the total follows the store across writes, and that a
 * chat going to nothing takes its tray entry with it while a cold launch
 * takes nothing with it at all.
 */

package me.nettrash.familyconnect.data.push

import android.app.NotificationManager
import android.content.Context
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.db.ChatDao
import me.nettrash.familyconnect.data.db.ChatEntity
import me.nettrash.familyconnect.testutil.createTestDb
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class UnreadNotificationsTest {

    private companion object {
        const val FAMILY = 1L
        const val DIRECT = 42L
    }

    private val dispatcher = StandardTestDispatcher()

    // A FOREGROUND scope, like the app's: the collector is meant to run
    // for the life of the process, and backgroundScope work is skipped by
    // advanceUntilIdle (which is why every wait below is runCurrent).
    private val appScope = CoroutineScope(dispatcher + SupervisorJob())
    private val context: Context = RuntimeEnvironment.getApplication()
    private val manager: NotificationManager =
        context.getSystemService(NotificationManager::class.java)
    private lateinit var db: AppDatabase
    private lateinit var chatDao: ChatDao

    @Before
    fun setUp() {
        db = createTestDb(dispatcher)
        chatDao = db.chatDao()
        PushNotifications.ensureChannel(context)
    }

    @After
    fun tearDown() {
        appScope.cancel()
        db.close()
    }

    private fun start() = UnreadNotifications(context, chatDao, appScope)

    private suspend fun insertChat(id: Long, unreadCount: Int) {
        chatDao.upsertAll(
            listOf(
                ChatEntity(
                    id = id,
                    kind = if (id == FAMILY) "family" else "direct",
                    peerUserId = if (id == FAMILY) null else 9L,
                    title = "Chat $id",
                    unreadCount = unreadCount,
                    myLastReadId = null,
                    peerLastReadId = null,
                    lastMessageBody = null,
                    lastMessageAt = null,
                    lastMessageSenderId = null,
                ),
            ),
        )
    }

    /**
     * Straight at the manager rather than through PushNotifications.show,
     * which is gated on a runtime permission this process does not hold.
     * The (tag, id) slot is the same one show() posts into — and the tag
     * is what the sweep matches, because a notification the SYSTEM tray
     * built carries an id of its own choosing and only the tag is ours.
     */
    private fun postTrayNotification(chatId: Long) {
        manager.notify(
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

    private fun activeTags(): List<String?> = manager.activeNotifications.map { it.tag }

    // -- the source --------------------------------------------------------------------

    @Test
    fun theTotalFollowsTheStore() = runTest(dispatcher) {
        insertChat(FAMILY, unreadCount = 3)
        insertChat(DIRECT, unreadCount = 1)
        val unread = start()
        runCurrent()

        assertThat(unread.total.value).isEqualTo(4)

        chatDao.clearUnread(DIRECT)
        runCurrent()
        assertThat(unread.total.value).isEqualTo(3)

        chatDao.bumpUnread(FAMILY)
        runCurrent()
        assertThat(unread.total.value).isEqualTo(4)
    }

    /**
     * The resync case, from the source's side: `GET /chats` writes the
     * authoritative counts into the same rows, so the total is right
     * again without anything having to tell it so.
     */
    @Test
    fun aResyncOverwritingTheCountsMovesTheTotal() = runTest(dispatcher) {
        insertChat(FAMILY, unreadCount = 3)
        val unread = start()
        runCurrent()
        assertThat(unread.total.value).isEqualTo(3)

        insertChat(FAMILY, unreadCount = 9) // upsert = what refreshChats does
        runCurrent()

        assertThat(unread.total.value).isEqualTo(9)
    }

    /**
     * Across process death there is no "previous" to compare against, and
     * the store is the only thing that survived — so the number a fresh
     * process reports is whatever Room held, immediately, with no network.
     */
    @Test
    fun aFreshProcessReportsWhatTheStoreHeld() = runTest(dispatcher) {
        insertChat(FAMILY, unreadCount = 5)

        val unread = start()
        runCurrent()

        assertThat(unread.total.value).isEqualTo(5)
    }

    // -- the clearing ------------------------------------------------------------------

    @Test
    fun aChatReadHereTakesItsNotificationDown() = runTest(dispatcher) {
        insertChat(DIRECT, unreadCount = 2)
        postTrayNotification(DIRECT)
        start()
        runCurrent()
        assertThat(activeTags()).containsExactly(PushNotifications.chatTag(DIRECT))

        chatDao.clearUnread(DIRECT)
        runCurrent()

        assertThat(activeTags()).isEmpty()
    }

    /** Only that chat's: another chat's messages are still waiting. */
    @Test
    fun readingOneChatLeavesTheOthersAlone() = runTest(dispatcher) {
        insertChat(FAMILY, unreadCount = 1)
        insertChat(DIRECT, unreadCount = 2)
        postTrayNotification(FAMILY)
        postTrayNotification(DIRECT)
        start()
        runCurrent()

        chatDao.clearUnread(DIRECT)
        runCurrent()

        assertThat(activeTags()).containsExactly(PushNotifications.chatTag(FAMILY))
    }

    /**
     * THE COLD-LAUNCH TRAP, end to end. A push arrived while the app was
     * dead: the SYSTEM tray rendered it and this process never saw the
     * message, so the store honestly says zero. Sweeping on that zero
     * would delete the notification the user has not read yet — which is
     * a badge feature deleting the very thing it exists to show.
     */
    @Test
    fun aColdStartDoesNotSweepANotificationTheStoreNeverHeardAbout() = runTest(dispatcher) {
        insertChat(DIRECT, unreadCount = 0)
        postTrayNotification(DIRECT)

        start()
        runCurrent()

        assertThat(activeTags()).containsExactly(PushNotifications.chatTag(DIRECT))
    }

    /**
     * A peer deleted their account, so the direct chat is gone from this
     * device (ChatRepository.deleteDirectChat). The notification pointed
     * at a conversation that no longer exists and cannot be opened.
     */
    @Test
    fun aChatDeletedWithUnreadTakesItsNotificationDown() = runTest(dispatcher) {
        insertChat(DIRECT, unreadCount = 2)
        postTrayNotification(DIRECT)
        start()
        runCurrent()

        chatDao.deleteById(DIRECT)
        runCurrent()

        assertThat(activeTags()).isEmpty()
    }

    /**
     * The cross-device trigger, in isolation: no local count moved, and
     * the only thing that knows is the server's answer.
     */
    @Test
    fun theServerSayingReadTakesTheNotificationDownWithNoLocalChange() = runTest(dispatcher) {
        insertChat(DIRECT, unreadCount = 0)
        postTrayNotification(DIRECT)
        val unread = start()
        runCurrent()

        unread.onServerSaysRead(setOf(DIRECT))

        assertThat(activeTags()).isEmpty()
    }

    @Test
    fun anEmptySetOfChatsSweepsNothing() = runTest(dispatcher) {
        insertChat(DIRECT, unreadCount = 0)
        postTrayNotification(DIRECT)
        val unread = start()
        runCurrent()

        unread.onServerSaysRead(emptySet())

        assertThat(activeTags()).containsExactly(PushNotifications.chatTag(DIRECT))
    }
}
