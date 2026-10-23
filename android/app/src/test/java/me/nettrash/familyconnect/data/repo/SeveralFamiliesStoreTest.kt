/*
 * SeveralFamiliesStoreTest.kt
 * Family Connect (Android)
 *
 * What this device keeps per FAMILY (#88; docs/protocol.md, "Several
 * families"): the roster one row per (family, member), the board and the
 * pack filed by `family_id` and caught up on a cursor per family, chats
 * told their `family_ids`, rows from before claimed for the home family,
 * and leaving one family dropping only its chats, board, pack and roster —
 * a direct chat staying while another family still lists it.
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
import me.nettrash.familyconnect.data.db.ChatEntity
import me.nettrash.familyconnect.data.db.GroupMembersCodec
import me.nettrash.familyconnect.data.db.MemberEntity
import me.nettrash.familyconnect.data.db.NoteEntity
import me.nettrash.familyconnect.data.db.PackItemEntity
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.BoardResponse
import me.nettrash.familyconnect.data.net.dto.ChatDto
import me.nettrash.familyconnect.data.net.dto.ChatListItemDto
import me.nettrash.familyconnect.data.net.dto.ChatsResponse
import me.nettrash.familyconnect.data.net.dto.FamilyDto
import me.nettrash.familyconnect.data.net.dto.PackResponse
import me.nettrash.familyconnect.data.net.ws.ServerFrame
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.data.settings.boardCursor
import me.nettrash.familyconnect.data.settings.packCursor
import me.nettrash.familyconnect.testutil.FakeAttachmentApi
import me.nettrash.familyconnect.testutil.FakeBoardApi
import me.nettrash.familyconnect.testutil.FakeChatApi
import me.nettrash.familyconnect.testutil.FakeChatSocket
import me.nettrash.familyconnect.testutil.FakePackApi
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.createTestDb
import me.nettrash.familyconnect.testutil.noteDto
import me.nettrash.familyconnect.testutil.packItemDto
import me.nettrash.familyconnect.testutil.testChatRepository
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class SeveralFamiliesStoreTest {

    private companion object {
        const val HOME = 3L
        const val OTHER = 5L
    }

    private val dispatcher = StandardTestDispatcher()
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private lateinit var db: AppDatabase
    private val socket = FakeChatSocket()
    private val settings = FakeSettingsRepository(
        SettingsState(
            serverUrl = "https://chat.example.com",
            myUserId = 7,
            families = listOf(
                FamilyDto(id = HOME, name = "The Smiths", joinPolicy = "open", role = "member"),
                FamilyDto(id = OTHER, name = "The Joneses", joinPolicy = "open", role = "owner"),
            ),
            currentFamilyId = HOME,
            multiFamilyServer = true,
        ),
    )

    @Before
    fun setUp() {
        db = createTestDb(dispatcher)
    }

    @After
    fun tearDown() {
        repoScope.cancel()
        db.close()
    }

    private fun member(familyId: Long, userId: Long, name: String = "user$userId", hasLeft: Boolean = false) =
        MemberEntity(userId = userId, username = name, displayName = name, role = "member", hasLeft = hasLeft, familyId = familyId)

    private fun chat(id: Long, kind: String, familyIds: List<Long>?) = ChatEntity(
        id = id,
        kind = kind,
        peerUserId = if (kind == "direct") 9 else null,
        title = "chat $id",
        unreadCount = 0,
        myLastReadId = null,
        peerLastReadId = null,
        lastMessageBody = null,
        lastMessageAt = null,
        lastMessageSenderId = null,
        familyIdsJson = GroupMembersCodec.encode(familyIds),
    )

    private fun note(id: Long, familyId: Long) = NoteEntity(
        id = id, authorId = 7, text = "n$id", color = "yellow", x = 0.1, y = 0.1,
        createdAt = 0, updatedAt = 0, boardSeq = id, familyId = familyId,
    )

    // -- The roster -------------------------------------------------------------

    @Test
    fun `one person in two families has a row in each and one name`() = runTest(dispatcher) {
        val members = db.memberDao()
        members.upsertAll(listOf(member(HOME, 9, "Ben"), member(OTHER, 9, "Ben", hasLeft = true)))

        assertThat(members.observeActiveMembers(HOME).first().map { it.userId }).containsExactly(9L)
        assertThat(members.observeActiveMembers(OTHER).first()).isEmpty()
        // Name resolution: one row, from a family they are still in.
        val named = members.observeMembers().first().single()
        assertThat(named.familyId).isEqualTo(HOME)

        // The account's facts reach every row of theirs.
        members.setBirthday(9, 3, 14)
        assertThat(members.observeAllActiveMemberships().first().single().birthdayDay).isEqualTo(14)
        members.writeTombstone(familyId = OTHER, userId = 9, username = "", displayName = "Deleted account")
        assertThat(members.observeActiveMembers(HOME).first()).isEmpty()
    }

    @Test
    fun `rows from before several families are claimed for the home family, never twice`() = runTest(dispatcher) {
        val members = db.memberDao()
        members.upsertAll(listOf(member(0, 9), member(0, 11), member(HOME, 11)))
        db.noteDao().upsert(note(1, 0))
        db.packDao().upsert(PackItemEntity(id = 5, addedBy = 7, attachmentJson = "[]", createdAt = 0, packSeq = 1))

        members.claimLegacy(HOME)
        db.noteDao().claimLegacy(HOME)
        db.packDao().claimLegacy(HOME)

        assertThat(members.observeActiveMembers(HOME).first().map { it.userId }).containsExactly(9L, 11L)
        assertThat(members.observeActiveMembers(0).first()).isEmpty()
        assertThat(db.noteDao().observeNotes(HOME).first().map { it.id }).containsExactly(1L)
        assertThat(db.packDao().items(HOME).map { it.id }).containsExactly(5L)
    }

    // -- The board -------------------------------------------------------------------

    @Test
    fun `each family's board is read by its own path and caught up on its own cursor`() = runTest(dispatcher) {
        val boardApi = FakeBoardApi()
        val board = BoardRepository(boardApi, db.noteDao(), db.memberDao(), settings, socket, backgroundScope)
        boardApi.boards[HOME] = BoardResponse(listOf(noteDto(id = 1, boardSeq = 10)), maxBoardSeq = 10)
        boardApi.boards[OTHER] = BoardResponse(listOf(noteDto(id = 2, boardSeq = 30)), maxBoardSeq = 30)

        board.catchUpBoard(10, HOME)
        board.catchUpBoard(30, OTHER)

        assertThat(boardApi.familyCalls).containsExactly("getBoard" to HOME, "getBoard" to OTHER).inOrder()
        assertThat(settings.current.boardCursor(HOME)).isEqualTo(10)
        assertThat(settings.current.boardCursor(OTHER)).isEqualTo(30)
        // The screen shows the family shown.
        assertThat(board.observeNotes().first().map { it.id }).containsExactly(1L)
        settings.setCurrentFamilyId(OTHER)
        assertThat(board.observeNotes().first().map { it.id }).containsExactly(2L)
    }

    @Test
    fun `a board note frame is filed under the family it names`() = runTest(dispatcher) {
        val board = BoardRepository(FakeBoardApi(), db.noteDao(), db.memberDao(), settings, socket, backgroundScope)
        runCurrent()

        socket.emit(ServerFrame.BoardNote(noteDto(id = 9, boardSeq = 50).copy(familyId = OTHER)))
        runCurrent()

        assertThat(db.noteDao().observeNotes(OTHER).first().map { it.id }).containsExactly(9L)
        assertThat(board.observeNotes().first()).isEmpty()
    }

    // -- The pack ----------------------------------------------------------------------

    @Test
    fun `each family's pack has its own cursor, and leaving drops only that pack`() = runTest(dispatcher) {
        val packApi = FakePackApi()
        val pack = PackRepository(
            context = RuntimeEnvironment.getApplication(),
            packApi = packApi,
            attachmentApi = FakeAttachmentApi(),
            packDao = db.packDao(),
            settings = settings,
            socket = socket,
            scope = backgroundScope,
        )
        packApi.pack = PackResponse(listOf(packItemDto(id = 5, packSeq = 12).copy(familyId = OTHER)), maxPackSeq = 12)
        pack.catchUpPack(12, familyId = OTHER)
        pack.applyItem(packItemDto(id = 6, packSeq = 13).copy(familyId = HOME))

        assertThat(packApi.familyCalls).containsExactly("getPack" to OTHER)
        assertThat(settings.current.packCursor(OTHER)).isEqualTo(12)
        assertThat(settings.current.packCursor(HOME)).isEqualTo(0)
        assertThat(pack.observeItems().first().map { it.id }).containsExactly(6L)

        pack.dropFamily(OTHER)
        assertThat(db.packDao().items().map { it.id }).containsExactly(6L)
    }

    // -- Chats ----------------------------------------------------------------------------

    @Test
    fun `GET chats tells each chat its families`() = runTest(dispatcher) {
        val chatApi = FakeChatApi()
        val chats = testChatRepository(chatApi, db.chatDao(), db.messageDao(), socket, repoScope)
        chatApi.chatsResult = ApiResult.Ok(
            ChatsResponse(
                listOf(
                    ChatListItemDto(ChatDto(1, "family", "The Smiths", familyIds = listOf(HOME)), unreadCount = 0),
                    ChatListItemDto(ChatDto(2, "direct", "Ben", peerUserId = 9, familyIds = listOf(HOME, OTHER)), unreadCount = 0),
                    ChatListItemDto(ChatDto(3, "ai", "Assistant"), unreadCount = 0),
                ),
            ),
        )

        chats.refreshChats()

        assertThat(db.chatDao().getById(1)!!.familyIds).containsExactly(HOME)
        assertThat(db.chatDao().getById(2)!!.familyIds).containsExactly(HOME, OTHER).inOrder()
        // A server from before says nothing, and nothing is invented.
        assertThat(db.chatDao().getById(3)!!.familyIds).isNull()
    }

    @Test
    fun `leaving one family drops its chats, and a direct chat another family lists stays`() = runTest(dispatcher) {
        val chats = testChatRepository(FakeChatApi(), db.chatDao(), db.messageDao(), socket, repoScope)
        runCurrent()
        db.chatDao().upsertAll(
            listOf(
                chat(1, "family", listOf(HOME)),
                chat(2, "family", listOf(OTHER)),
                chat(3, "direct", listOf(HOME, OTHER)),
                chat(4, "direct", listOf(OTHER)),
                chat(5, "group", listOf(OTHER)),
                chat(6, "ai", listOf(OTHER)),
            ),
        )

        chats.dropFamily(OTHER, remainingFamilyIds = setOf(HOME))

        assertThat(db.chatDao().allChatIds()).containsExactly(1L, 3L)
        assertThat(db.chatDao().getById(3)!!.familyIds).containsExactly(HOME)
    }

    @Test
    fun `the per-family store drops one family's board and roster and keeps the rest`() = runTest(dispatcher) {
        val chats = testChatRepository(FakeChatApi(), db.chatDao(), db.messageDao(), socket, repoScope)
        val pack = PackRepository(
            RuntimeEnvironment.getApplication(), FakePackApi(), FakeAttachmentApi(), db.packDao(), settings, socket, backgroundScope,
        )
        val store = RoomFamilyLocalData(chats, pack, db.memberDao(), db.noteDao(), db.packDao())
        db.memberDao().upsertAll(listOf(member(HOME, 9), member(OTHER, 9), member(OTHER, 11)))
        db.noteDao().upsert(note(1, HOME))
        db.noteDao().upsert(note(2, OTHER))

        store.dropFamily(OTHER, setOf(HOME))

        assertThat(db.memberDao().observeAllActiveMemberships().first().map { it.familyId to it.userId })
            .containsExactly(HOME to 9L)
        assertThat(db.noteDao().observeNotes().first().map { it.id }).containsExactly(1L)
    }
}
