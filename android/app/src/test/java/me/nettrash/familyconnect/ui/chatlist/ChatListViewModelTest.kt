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
import me.nettrash.familyconnect.R
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
import me.nettrash.familyconnect.data.repo.GroupRepository
import me.nettrash.familyconnect.data.repo.SessionRepository
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.testutil.FakeAuthApi
import me.nettrash.familyconnect.testutil.FakeChatApi
import me.nettrash.familyconnect.testutil.FakeChatSocket
import me.nettrash.familyconnect.testutil.FakeConnectivityObserver
import me.nettrash.familyconnect.testutil.FakeFamilyApi
import me.nettrash.familyconnect.testutil.FakeGroupApi
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.FakeTokenStore
import me.nettrash.familyconnect.testutil.RecordingWiper
import me.nettrash.familyconnect.testutil.testChatRepository
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
    private val groupApi = FakeGroupApi()
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
        chatRepository = testChatRepository(chatApi, db.chatDao(), db.messageDao(), socket, repoScope)
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
            authApi = FakeAuthApi(),
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
            noteDao = db.noteDao(),
            settingsRepository = settings,
            settings = settings,
            connectivity = FakeConnectivityObserver(),
            socket = socket,
            groupRepository = GroupRepository(groupApi, chatRepository),
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

    // -- New Group (#107; docs/protocol.md, "Groups") ------------------------------

    /**
     * Everyone still in the family but me — and, unlike the direct-chat
     * picker, the blocked too: a block is never a reason to refuse an add.
     */
    @Test
    fun groupCandidatesAreTheActiveFamilyButMeBlockedIncluded() = runTest(dispatcher) {
        settings.setBlockedUserIds(setOf(PEER))
        db.memberDao().upsertAll(
            listOf(
                MemberEntity(ME, "anna", "Anna", "owner"),
                MemberEntity(PEER, "ben", "Ben", "member"),
                MemberEntity(11, "cleo", "Cleo", "member"),
                MemberEntity(12, "dan", "Dan", "member", hasLeft = true),
            ),
        )
        val viewModel = newViewModel()
        val subscriptions = listOf(
            repoScope.launch { viewModel.groupCandidates.collect {} },
            repoScope.launch { viewModel.pickableMembers.collect {} },
        )
        runCurrent()

        assertThat(viewModel.groupCandidates.value.map { it.userId }).containsExactly(PEER, 11L)
        assertThat(viewModel.pickableMembers.value.map { it.userId }).containsExactly(11L)
        subscriptions.forEach { it.cancel() }
    }

    @Test
    fun aNameThatIsBlankOrTooLongIsNeverSent() = runTest(dispatcher) {
        val viewModel = newViewModel()
        runCurrent()

        viewModel.createGroup("   ", setOf(PEER))
        runCurrent()
        assertThat(viewModel.groupError.value).isNotNull()

        viewModel.createGroup("x".repeat(65), setOf(PEER))
        runCurrent()
        assertThat(viewModel.groupError.value).isNotNull()
        assertThat(groupApi.created).isEmpty()
    }

    @Test
    fun aGroupWithNobodyElseIsNeverSent() = runTest(dispatcher) {
        val viewModel = newViewModel()
        runCurrent()

        viewModel.createGroup("Cousins", emptySet())
        runCurrent()

        assertThat(viewModel.groupError.value).isNotNull()
        assertThat(groupApi.created).isEmpty()
    }

    @Test
    fun aCreatedGroupIsStoredAndOpened() = runTest(dispatcher) {
        groupApi.createResult = ApiResult.Ok(
            ChatResponse(
                ChatDto(id = 90, kind = "group", title = "Cousins", memberIds = listOf(ME, PEER), adminUserId = ME),
            ),
        )
        val viewModel = newViewModel()
        val navigated = async { viewModel.navigateToChat.first() }
        runCurrent()

        viewModel.createGroup("  Cousins  ", setOf(PEER))
        runCurrent()

        // Sent trimmed, with the others named (the server adds the creator).
        assertThat(groupApi.created).containsExactly("Cousins" to listOf(PEER))
        assertThat(navigated.await()).isEqualTo(90L)
        val stored = db.chatDao().getById(90L)!!
        assertThat(stored.kind).isEqualTo("group")
        assertThat(stored.memberIds).containsExactly(ME, PEER).inOrder()
        assertThat(stored.adminUserId).isEqualTo(ME)
        assertThat(viewModel.groupError.value).isNull()
    }

    @Test
    fun theFamilysFiftiethGroupIsSaidSo() = runTest(dispatcher) {
        groupApi.createResult = ApiResult.HttpError(409, "too_many_groups", "50")
        val viewModel = newViewModel()
        runCurrent()

        viewModel.createGroup("Cousins", setOf(PEER))
        runCurrent()

        assertThat(viewModel.groupError.value)
            .isEqualTo(RuntimeEnvironment.getApplication().getString(R.string.e_too_many_groups))
        assertThat(viewModel.creatingGroup.value).isFalse()
    }

    /** A failure that says nothing about the request is not a refusal. */
    @Test
    fun aTransientFailureIsNotSaidAsARefusal() = runTest(dispatcher) {
        groupApi.createResult = ApiResult.HttpError(503, null, null)
        val viewModel = newViewModel()
        runCurrent()

        viewModel.createGroup("Cousins", setOf(PEER))
        runCurrent()

        assertThat(viewModel.groupError.value)
            .isEqualTo(RuntimeEnvironment.getApplication().getString(R.string.e_unreachable))
    }
}
