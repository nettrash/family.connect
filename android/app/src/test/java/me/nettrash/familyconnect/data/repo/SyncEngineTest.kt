/*
 * SyncEngineTest.kt
 * Family Connect (Android)
 *
 * The protocol's four-step resync: reconcile membership, refresh the
 * chat list (server unread wins), page after_id per chat until a short
 * page, then flush pending sends. Real Room, fakes on the wire.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.data.db.MessageStatus
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.ChatDto
import me.nettrash.familyconnect.data.net.dto.ChatListItemDto
import me.nettrash.familyconnect.data.net.dto.ChatsResponse
import me.nettrash.familyconnect.data.net.dto.FamilyDto
import me.nettrash.familyconnect.data.net.dto.FamilyMineResponse
import me.nettrash.familyconnect.data.net.dto.MeResponse
import me.nettrash.familyconnect.data.net.dto.MemberDto
import me.nettrash.familyconnect.data.net.dto.MessagesResponse
import me.nettrash.familyconnect.data.net.ws.ClientFrame
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.testutil.FakeAuthApi
import me.nettrash.familyconnect.testutil.FakeChatApi
import me.nettrash.familyconnect.testutil.FakeChatSocket
import me.nettrash.familyconnect.testutil.FakeFamilyApi
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.FakeTokenStore
import me.nettrash.familyconnect.testutil.RecordingWiper
import me.nettrash.familyconnect.testutil.createTestDb
import me.nettrash.familyconnect.testutil.messageDto
import me.nettrash.familyconnect.testutil.userDto
import me.nettrash.familyconnect.util.Clock
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class SyncEngineTest {

    private companion object {
        const val ME = 7L
        const val PEER = 9L
        const val FAMILY_CHAT = 1L
    }

    private lateinit var db: AppDatabase
    private val authApi = FakeAuthApi()
    private val chatApi = FakeChatApi()
    private val familyApi = FakeFamilyApi()
    private val socket = FakeChatSocket()
    private val tokenStore = FakeTokenStore("tok")
    private val wiper = RecordingWiper()
    private val settings = FakeSettingsRepository(
        SettingsState(
            serverUrl = "https://chat.example.com",
            familyStatus = FamilyStatus.MEMBER,
            myUserId = ME,
        ),
    )

    @Before
    fun setUp() {
        db = createTestDb()
        val family = FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open")
        authApi.meResult = ApiResult.Ok(
            MeResponse(user = userDto(ME, "anna"), family = family, role = "member"),
        )
        familyApi.mineResult = ApiResult.Ok(
            FamilyMineResponse(
                family = family,
                members = listOf(
                    MemberDto(ME, "anna", "Anna", "owner"),
                    MemberDto(PEER, "ben", "Ben", "member"),
                ),
            ),
        )
    }

    @After
    fun tearDown() {
        db.close()
    }

    private fun TestScope.newEngine(): SyncEngine {
        val sessionRepository = SessionRepository(
            authApi = authApi,
            tokenStore = tokenStore,
            settings = settings,
            wiper = wiper,
            unauthorizedEvents = MutableSharedFlow(),
            scope = backgroundScope,
        )
        val chatRepository = ChatRepository(chatApi, db.chatDao(), socket)
        val familyRepository = FamilyRepository(
            familyApi = familyApi,
            memberDao = db.memberDao(),
            settings = settings,
            sessionRepository = sessionRepository,
            socket = socket,
            scope = backgroundScope,
        )
        val messageRepository = MessageRepository(
            chatApi = chatApi,
            messageDao = db.messageDao(),
            chatDao = db.chatDao(),
            socket = socket,
            settings = settings,
            chatRepository = chatRepository,
            scope = backgroundScope,
            clock = Clock { 1_000_000L },
        )
        runCurrent()
        return SyncEngine(
            sessionRepository = sessionRepository,
            chatRepository = chatRepository,
            familyRepository = familyRepository,
            messageRepository = messageRepository,
            chatDao = db.chatDao(),
            messageDao = db.messageDao(),
        )
    }

    private fun scriptChats(unread: Int = 3) {
        chatApi.chatsResult = ApiResult.Ok(
            ChatsResponse(
                listOf(
                    ChatListItemDto(
                        chat = ChatDto(id = FAMILY_CHAT, kind = "family", title = "The Smiths"),
                        lastMessage = messageDto(id = 5, chatId = FAMILY_CHAT, senderId = PEER),
                        unreadCount = unread,
                    ),
                ),
            ),
        )
    }

    @Test
    fun resyncPullsChatsRosterAndCatchUpPages() = runTest {
        val engine = newEngine()
        scriptChats(unread = 3)
        // Local cursor: we already hold messages up to id 5.
        db.messageDao().insertIgnore(
            listOf(MessageEntity("s5", 5, FAMILY_CHAT, PEER, "old", 1L, MessageStatus.SENT)),
        )
        // First page full (200), second short — the loop must run twice.
        chatApi.messagesHandler = { _, _, afterId, limit ->
            when (afterId) {
                5L -> ApiResult.Ok(
                    MessagesResponse(
                        (6L until 6L + limit).map { messageDto(id = it, chatId = FAMILY_CHAT, senderId = PEER) },
                    ),
                )
                else -> ApiResult.Ok(
                    MessagesResponse(listOf(messageDto(id = 999, chatId = FAMILY_CHAT, senderId = PEER))),
                )
            }
        }

        engine.resync()

        // Membership + roster refreshed.
        assertThat(authApi.meCalls).isEqualTo(1)
        assertThat(db.memberDao().observeMembers().first()).hasSize(2)
        // Chat list upserted; server unread count won.
        val chat = db.chatDao().getById(FAMILY_CHAT)!!
        assertThat(chat.unreadCount).isEqualTo(3)
        // Catch-up looped: full page then short page.
        assertThat(chatApi.messagesCalls).isEqualTo(2)
        assertThat(db.messageDao().maxServerId(FAMILY_CHAT)).isEqualTo(999L)
    }

    @Test
    fun resyncFlushesPendingOutboundMessages() = runTest {
        val engine = newEngine()
        scriptChats()
        db.messageDao().insert(
            MessageEntity("pending-uuid", null, FAMILY_CHAT, ME, "unsent", 1L, MessageStatus.SENDING),
        )
        socket.setOpen(true)

        engine.resync()
        runCurrent()

        val resent = socket.sent.filterIsInstance<ClientFrame.Send>()
        assertThat(resent.map { it.clientMsgId }).containsExactly("pending-uuid")
    }

    @Test
    fun resyncStopsWhenMembershipIsGone() = runTest {
        val engine = newEngine()
        // Server says: no family any more.
        authApi.meResult = ApiResult.Ok(MeResponse(user = userDto(ME)))
        scriptChats()

        engine.resync()

        // No chat list fetch once canChat is false.
        assertThat(chatApi.messagesCalls).isEqualTo(0)
        assertThat(db.chatDao().getById(FAMILY_CHAT)).isNull()
    }

    @Test
    fun resyncPreservesLocalReadMarkersWhileTakingServerUnread() = runTest {
        val engine = newEngine()
        scriptChats(unread = 7)
        // Pre-existing chat row with local-only markers.
        engine.resync() // creates the chat
        db.chatDao().setMyLastRead(FAMILY_CHAT, 4)
        db.chatDao().setPeerLastRead(FAMILY_CHAT, 2)

        scriptChats(unread = 0)
        engine.resync()

        val chat = db.chatDao().getById(FAMILY_CHAT)!!
        assertThat(chat.unreadCount).isEqualTo(0) // server wins
        assertThat(chat.myLastReadId).isEqualTo(4L) // local survives
        assertThat(chat.peerLastReadId).isEqualTo(2L)
    }
}
