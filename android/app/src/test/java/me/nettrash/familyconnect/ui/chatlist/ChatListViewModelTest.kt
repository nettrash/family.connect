/*
 * ChatListViewModelTest.kt
 * Family Connect (Android)
 *
 * Chat-list surface: family chat pinned first regardless of recency,
 * the member picker excludes me, createDirect navigates with the chat
 * id, and refresh merges server unread counts over local rows.
 */

package me.nettrash.familyconnect.ui.chatlist

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.db.ChatEntity
import me.nettrash.familyconnect.data.db.MemberEntity
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.ChatDto
import me.nettrash.familyconnect.data.net.dto.ChatListItemDto
import me.nettrash.familyconnect.data.net.dto.ChatResponse
import me.nettrash.familyconnect.data.net.dto.ChatsResponse
import me.nettrash.familyconnect.data.repo.ChatRepository
import me.nettrash.familyconnect.data.repo.FamilyRepository
import me.nettrash.familyconnect.data.repo.FamilyStatus
import me.nettrash.familyconnect.data.repo.SessionRepository
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.testutil.FakeAuthApi
import me.nettrash.familyconnect.testutil.FakeChatApi
import me.nettrash.familyconnect.testutil.FakeChatSocket
import me.nettrash.familyconnect.testutil.FakeConnectivityObserver
import me.nettrash.familyconnect.testutil.FakeFamilyApi
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.FakeTokenStore
import me.nettrash.familyconnect.testutil.RecordingWiper
import me.nettrash.familyconnect.testutil.createTestDb
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class ChatListViewModelTest {

    private companion object {
        const val ME = 7L
        const val PEER = 9L
    }

    private val dispatcher = StandardTestDispatcher()

    // Foreground scope for repositories + StateFlow subscriptions — see
    // MessageRepositoryTest for why backgroundScope won't do.
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private lateinit var db: AppDatabase
    private lateinit var chatApi: FakeChatApi
    private lateinit var chatRepository: ChatRepository
    private val socket = FakeChatSocket()
    private val settings = FakeSettingsRepository(
        SettingsState(
            serverUrl = "https://chat.example.com",
            familyStatus = FamilyStatus.MEMBER,
            myUserId = ME,
        ),
    )

    @Before
    fun setUp() {
        Dispatchers.setMain(dispatcher)
        db = createTestDb(dispatcher)
        chatApi = FakeChatApi()
    }

    @After
    fun tearDown() {
        repoScope.cancel()
        Dispatchers.resetMain()
        db.close()
    }

    private fun TestScope.newViewModel(): ChatListViewModel {
        chatRepository = ChatRepository(chatApi, db.chatDao(), socket)
        val sessionRepository = SessionRepository(
            authApi = FakeAuthApi(),
            tokenStore = FakeTokenStore("tok"),
            settings = settings,
            wiper = RecordingWiper(),
            unauthorizedEvents = MutableSharedFlow(),
            scope = repoScope,
        )
        val familyRepository = FamilyRepository(
            familyApi = FakeFamilyApi(),
            memberDao = db.memberDao(),
            settings = settings,
            sessionRepository = sessionRepository,
            socket = socket,
            scope = repoScope,
        )
        runCurrent()
        return ChatListViewModel(
            appContext = RuntimeEnvironment.getApplication(),
            chatRepository = chatRepository,
            familyRepository = familyRepository,
            settings = settings,
            connectivity = FakeConnectivityObserver(),
            socket = socket,
        )
    }

    private fun chat(
        id: Long,
        kind: String,
        lastMessageAt: Long?,
        unread: Int = 0,
    ) = ChatEntity(
        id = id,
        kind = kind,
        peerUserId = if (kind == "direct") PEER else null,
        title = if (kind == "family") "The Smiths" else "Ben",
        unreadCount = unread,
        myLastReadId = null,
        peerLastReadId = null,
        lastMessageBody = lastMessageAt?.let { "hi" },
        lastMessageAt = lastMessageAt,
        lastMessageSenderId = lastMessageAt?.let { PEER },
    )

    @Test
    fun familyChatIsPinnedFirstEvenWhenDirectChatsAreNewer() = runTest(dispatcher) {
        db.chatDao().upsertAll(
            listOf(
                chat(2, "direct", lastMessageAt = 5_000L),
                chat(1, "family", lastMessageAt = 1_000L),
                chat(3, "direct", lastMessageAt = null), // never messaged → last
            ),
        )
        val viewModel = newViewModel()
        val subscription = repoScope.launch { viewModel.chats.collect {} }
        runCurrent()

        assertThat(viewModel.chats.value.orEmpty().map { it.id }).containsExactly(1L, 2L, 3L).inOrder()
        subscription.cancel()
    }

    @Test
    fun memberPickerExcludesMyself() = runTest(dispatcher) {
        db.memberDao().upsertAll(
            listOf(
                MemberEntity(ME, "anna", "Anna", "owner"),
                MemberEntity(PEER, "ben", "Ben", "member"),
            ),
        )
        val viewModel = newViewModel()
        val subscription = repoScope.launch { viewModel.pickableMembers.collect {} }
        runCurrent()

        assertThat(viewModel.pickableMembers.value.map { it.userId }).containsExactly(PEER)
        subscription.cancel()
    }

    @Test
    fun openDirectChatEmitsTheChatIdToNavigateTo() = runTest(dispatcher) {
        chatApi.createDirectResult = ApiResult.Ok(
            ChatResponse(ChatDto(id = 77, kind = "direct", title = "Ben", peerUserId = PEER)),
        )
        val viewModel = newViewModel()
        val navigated = async { viewModel.navigateToChat.first() }
        runCurrent()

        viewModel.openDirectChat(PEER)
        runCurrent()

        assertThat(navigated.await()).isEqualTo(77L)
        assertThat(db.chatDao().getById(77L)).isNotNull() // stored for the chat screen
    }

    @Test
    fun openDirectChatFailureSurfacesAnError() = runTest(dispatcher) {
        chatApi.createDirectResult = ApiResult.HttpError(409, "not_same_family", "not in your family")
        val viewModel = newViewModel()
        runCurrent()

        viewModel.openDirectChat(PEER)
        runCurrent()

        assertThat(viewModel.error.value).isNotNull()
    }

    @Test
    fun refreshMergesServerChatsIntoRoom() = runTest(dispatcher) {
        chatApi.chatsResult = ApiResult.Ok(
            ChatsResponse(
                listOf(
                    ChatListItemDto(
                        chat = ChatDto(id = 1, kind = "family", title = "The Smiths"),
                        lastMessage = null,
                        unreadCount = 4,
                    ),
                ),
            ),
        )
        val viewModel = newViewModel()
        runCurrent()

        viewModel.refresh()
        runCurrent()

        val stored = db.chatDao().getById(1L)!!
        assertThat(stored.unreadCount).isEqualTo(4)
        assertThat(stored.title).isEqualTo("The Smiths")
    }
}
