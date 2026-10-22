/*
 * GroupChatSyncTest.kt
 * Family Connect (Android)
 *
 * How a GROUP reaches this device and leaves it (#107; docs/protocol.md,
 * "Groups"):
 *
 *   - `GET /chats` lists every group the reader is in, with its member list
 *     and admin, and a group it no longer lists is dropped — the repair for a
 *     device that slept through being removed or the group being deleted;
 *   - `chat_updated` {chat} creates or updates the row at once, keeping
 *     everything on it that is this device's (the preview, the count, the
 *     read markers, the cursors);
 *   - `chat_removed` {chat_id} drops the chat, its messages and what was
 *     being drafted there.
 *
 * And GroupRepository, which writes each endpoint's answer into the same
 * rows — the group as it now is, or gone.
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
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.db.ChatDao
import me.nettrash.familyconnect.data.db.ChatEntity
import me.nettrash.familyconnect.data.db.GroupMembersCodec
import me.nettrash.familyconnect.data.db.MessageDao
import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.data.db.MessageStatus
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.ChatDto
import me.nettrash.familyconnect.data.net.dto.ChatListItemDto
import me.nettrash.familyconnect.data.net.dto.ChatResponse
import me.nettrash.familyconnect.data.net.dto.ChatsResponse
import me.nettrash.familyconnect.data.net.ws.ServerFrame
import me.nettrash.familyconnect.testutil.FakeChatApi
import me.nettrash.familyconnect.testutil.FakeChatSocket
import me.nettrash.familyconnect.testutil.FakeGroupApi
import me.nettrash.familyconnect.testutil.createTestDb
import me.nettrash.familyconnect.testutil.testChatRepository
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class GroupChatSyncTest {

    private companion object {
        const val ME = 7L
        const val FAMILY_CHAT = 1L
        const val GROUP = 77L
        const val OTHER_GROUP = 78L
    }

    private val dispatcher = StandardTestDispatcher()
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private lateinit var db: AppDatabase
    private lateinit var chatDao: ChatDao
    private lateinit var messageDao: MessageDao
    private val chatApi = FakeChatApi()
    private val groupApi = FakeGroupApi()
    private val socket = FakeChatSocket()

    /** Every chat whose drafts were forgotten, in order. */
    private val forgotten = mutableListOf<Long>()

    @Before
    fun setUp() {
        db = createTestDb(dispatcher)
        chatDao = db.chatDao()
        messageDao = db.messageDao()
    }

    @After
    fun tearDown() {
        repoScope.cancel()
        db.close()
    }

    private fun TestScope.repository(): ChatRepository {
        val repository = testChatRepository(
            chatApi, chatDao, messageDao, socket, repoScope,
            chatDrafts = { forgotten += it },
        )
        runCurrent()
        return repository
    }

    // -- Fixtures ----------------------------------------------------------------------

    private fun groupDto(
        id: Long = GROUP,
        title: String = "Cousins",
        members: List<Long> = listOf(ME, 9, 11),
        admin: Long = ME,
    ) = ChatDto(id = id, kind = "group", title = title, memberIds = members, adminUserId = admin)

    private fun groupRow(id: Long = GROUP) = ChatEntity(
        id = id,
        kind = "group",
        peerUserId = null,
        title = "Cousins",
        unreadCount = 3,
        myLastReadId = 100L,
        peerLastReadId = null,
        lastMessageBody = "See you at six",
        lastMessageAt = 1_700_000_000_000L,
        lastMessageSenderId = 9L,
        maxReactionSeq = 7L,
        maxEditSeq = 8L,
        maxPollSeq = 9L,
        memberIdsJson = GroupMembersCodec.encode(listOf(ME, 9, 11)),
        adminUserId = ME,
    )

    private fun familyRow() = groupRow(FAMILY_CHAT).copy(kind = "family", memberIdsJson = null, adminUserId = null)

    private fun message(chatId: Long, serverId: Long) = MessageEntity(
        clientMsgId = "c$serverId",
        serverId = serverId,
        chatId = chatId,
        senderId = 9L,
        body = "body $serverId",
        createdAt = serverId,
        status = MessageStatus.SENT,
    )

    private suspend fun seed(vararg rows: ChatEntity) {
        chatDao.upsertAll(rows.toList())
        for (row in rows) {
            messageDao.insertIgnore(listOf(message(row.id, row.id * 100 + 1), message(row.id, row.id * 100 + 2)))
        }
    }

    private fun listing(vararg chats: ChatDto) = ApiResult.Ok(
        ChatsResponse(chats.map { ChatListItemDto(chat = it, unreadCount = 0) }),
    )

    private suspend fun held(chatId: Long) = chatDao.getById(chatId) != null

    private suspend fun messageCount(chatId: Long) =
        messageDao.observeMessages(chatId, limit = 50).first().size

    // -- GET /chats --------------------------------------------------------------------

    @Test
    fun `the chat list stores a group's members and admin`() = runTest(dispatcher) {
        val repository = repository()
        chatApi.chatsResult = listing(
            ChatDto(id = FAMILY_CHAT, kind = "family", title = "The Smiths"),
            groupDto(members = listOf(9, ME), admin = 9),
        )

        repository.refreshChats()

        val group = chatDao.getById(GROUP)!!
        assertThat(group.kind).isEqualTo("group")
        assertThat(group.title).isEqualTo("Cousins")
        assertThat(group.memberIds).containsExactly(9L, ME).inOrder()
        assertThat(group.adminUserId).isEqualTo(9L)
        // Nothing of the sort on the family chat.
        assertThat(chatDao.getById(FAMILY_CHAT)!!.memberIds).isEmpty()
        assertThat(chatDao.getById(FAMILY_CHAT)!!.adminUserId).isNull()
    }

    /** Left, removed from or deleted while this device slept. */
    @Test
    fun `a group the chat list no longer names is dropped with its messages`() = runTest(dispatcher) {
        val repository = repository()
        seed(familyRow(), groupRow(GROUP), groupRow(OTHER_GROUP))
        chatApi.chatsResult = listing(
            ChatDto(id = FAMILY_CHAT, kind = "family", title = "The Smiths"),
            groupDto(id = OTHER_GROUP),
        )

        repository.refreshChats()

        assertThat(held(GROUP)).isFalse()
        assertThat(messageCount(GROUP)).isEqualTo(0)
        assertThat(forgotten).containsExactly(GROUP)
        assertThat(held(OTHER_GROUP)).isTrue()
        assertThat(messageCount(OTHER_GROUP)).isEqualTo(2)
        assertThat(held(FAMILY_CHAT)).isTrue()
    }

    @Test
    fun `a failed chat list drops no group`() = runTest(dispatcher) {
        val repository = repository()
        seed(groupRow())
        chatApi.chatsResult = ApiResult.NetworkError(IllegalStateException("offline"))

        repository.refreshChats()

        assertThat(held(GROUP)).isTrue()
        assertThat(forgotten).isEmpty()
    }

    /**
     * A group created here — or arriving by `chat_updated` — while `GET
     * /chats` is in flight is not in a response computed before it existed.
     */
    @Test
    fun `a group that arrives during the request is not pruned by it`() = runTest(dispatcher) {
        val repository = repository()
        chatApi.chatsResult = listing(ChatDto(id = FAMILY_CHAT, kind = "family", title = "The Smiths"))
        val gate = CompletableDeferred<Unit>()
        chatApi.chatsGate = gate

        val refresh = launch { repository.refreshChats() }
        runCurrent()
        socket.emit(ServerFrame.ChatUpdated(groupDto()))
        runCurrent()
        gate.complete(Unit)
        refresh.join()

        assertThat(held(GROUP)).isTrue()
    }

    // -- chat_updated ------------------------------------------------------------------

    /** How a member just added sees the group appear at once. */
    @Test
    fun `chat updated for a group not held here creates it with nothing unread`() = runTest(dispatcher) {
        repository()

        socket.emit(ServerFrame.ChatUpdated(groupDto()))
        runCurrent()

        val group = chatDao.getById(GROUP)!!
        assertThat(group.title).isEqualTo("Cousins")
        assertThat(group.memberIds).containsExactly(ME, 9L, 11L).inOrder()
        assertThat(group.adminUserId).isEqualTo(ME)
        assertThat(group.unreadCount).isEqualTo(0)
        assertThat(chatDao.observeChats().first().map { it.id }).contains(GROUP)
    }

    @Test
    fun `chat updated renames and re-members a held group and keeps what is this device's`() =
        runTest(dispatcher) {
            repository()
            seed(groupRow())

            socket.emit(ServerFrame.ChatUpdated(groupDto(title = "Cousins 2026", members = listOf(9, 11, 12), admin = 9)))
            runCurrent()

            val group = chatDao.getById(GROUP)!!
            assertThat(group.title).isEqualTo("Cousins 2026")
            assertThat(group.memberIds).containsExactly(9L, 11L, 12L).inOrder()
            assertThat(group.adminUserId).isEqualTo(9L)
            // Local state survives the state-set.
            assertThat(group.unreadCount).isEqualTo(3)
            assertThat(group.myLastReadId).isEqualTo(100L)
            assertThat(group.lastMessageBody).isEqualTo("See you at six")
            assertThat(group.maxReactionSeq).isEqualTo(7L)
            assertThat(group.maxPollSeq).isEqualTo(9L)
            assertThat(messageCount(GROUP)).isEqualTo(2)
        }

    /** The frame is defined for groups; any other row is `GET /chats`'s to write. */
    @Test
    fun `chat updated naming a chat that is not a group changes nothing`() = runTest(dispatcher) {
        repository()
        seed(familyRow())

        socket.emit(ServerFrame.ChatUpdated(ChatDto(id = FAMILY_CHAT, kind = "family", title = "Renamed?")))
        runCurrent()

        assertThat(chatDao.getById(FAMILY_CHAT)!!.title).isEqualTo("Cousins")
    }

    // -- chat_removed ------------------------------------------------------------------

    @Test
    fun `chat removed drops the group, its messages and its drafts, and nothing else`() =
        runTest(dispatcher) {
            repository()
            seed(familyRow(), groupRow(GROUP), groupRow(OTHER_GROUP))

            socket.emit(ServerFrame.ChatRemoved(GROUP))
            runCurrent()

            assertThat(held(GROUP)).isFalse()
            assertThat(messageCount(GROUP)).isEqualTo(0)
            assertThat(forgotten).containsExactly(GROUP)
            assertThat(held(OTHER_GROUP)).isTrue()
            assertThat(held(FAMILY_CHAT)).isTrue()
            assertThat(messageCount(FAMILY_CHAT)).isEqualTo(2)
        }

    @Test
    fun `chat removed for a chat not held here is harmless`() = runTest(dispatcher) {
        repository()
        seed(groupRow())

        socket.emit(ServerFrame.ChatRemoved(999))
        runCurrent()

        assertThat(held(GROUP)).isTrue()
    }

    // -- GroupRepository ---------------------------------------------------------------

    private fun TestScope.groups(): GroupRepository = GroupRepository(groupApi, repository())

    @Test
    fun `a rename stores the group the server answered with`() = runTest(dispatcher) {
        val groups = groups()
        seed(groupRow())
        groupApi.renameResult = ApiResult.Ok(ChatResponse(groupDto(title = "Cousins 2026")))

        val result = groups.rename(GROUP, "Cousins 2026")

        assertThat(result).isInstanceOf(ApiResult.Ok::class.java)
        assertThat(groupApi.renamed).containsExactly(GROUP to "Cousins 2026")
        assertThat(chatDao.getById(GROUP)!!.title).isEqualTo("Cousins 2026")
    }

    @Test
    fun `adding and removing members store the new member list`() = runTest(dispatcher) {
        val groups = groups()
        seed(groupRow())
        groupApi.addMembersResult = ApiResult.Ok(ChatResponse(groupDto(members = listOf(ME, 9, 11, 12))))
        groupApi.removeMemberResult = ApiResult.Ok(groupDto(members = listOf(ME, 11, 12)))

        groups.addMembers(GROUP, listOf(12))
        assertThat(chatDao.getById(GROUP)!!.memberIds).containsExactly(ME, 9L, 11L, 12L).inOrder()

        groups.removeMember(GROUP, 9)
        assertThat(groupApi.removed).containsExactly(GROUP to 9L)
        assertThat(chatDao.getById(GROUP)!!.memberIds).containsExactly(ME, 11L, 12L).inOrder()
    }

    /** `DELETE …/members/{me}` → 204: the group goes from this device at once. */
    @Test
    fun `leaving drops the group from this device`() = runTest(dispatcher) {
        val groups = groups()
        seed(groupRow())
        groupApi.removeMemberResult = ApiResult.Ok(null)

        val result = groups.leave(GROUP, ME)

        assertThat(result).isEqualTo(ApiResult.Ok(Unit))
        assertThat(groupApi.removed).containsExactly(GROUP to ME)
        assertThat(held(GROUP)).isFalse()
        assertThat(messageCount(GROUP)).isEqualTo(0)
    }

    @Test
    fun `deleting drops the group from this device`() = runTest(dispatcher) {
        val groups = groups()
        seed(groupRow())
        groupApi.deleteResult = ApiResult.Ok(Unit)

        groups.delete(GROUP)

        assertThat(groupApi.deleted).containsExactly(GROUP)
        assertThat(held(GROUP)).isFalse()
    }

    /** Gone elsewhere while the reader looked at it: `chat_not_found`. */
    @Test
    fun `a group the server no longer shows this person is dropped`() = runTest(dispatcher) {
        val groups = groups()
        seed(groupRow())
        groupApi.renameResult = ApiResult.HttpError(404, "chat_not_found", "no such chat")

        groups.rename(GROUP, "Cousins 2026")

        assertThat(held(GROUP)).isFalse()
    }

    /** A refusal or a dead network leaves the group exactly as it was. */
    @Test
    fun `a refused or unanswered change keeps the group`() = runTest(dispatcher) {
        val groups = groups()
        seed(groupRow())
        groupApi.deleteResult = ApiResult.HttpError(403, "not_group_admin", "admin only")
        groupApi.removeMemberResult = ApiResult.NetworkError(IllegalStateException("offline"))

        groups.delete(GROUP)
        groups.leave(GROUP, ME)

        assertThat(held(GROUP)).isTrue()
        assertThat(chatDao.getById(GROUP)!!.title).isEqualTo("Cousins")
    }
}
