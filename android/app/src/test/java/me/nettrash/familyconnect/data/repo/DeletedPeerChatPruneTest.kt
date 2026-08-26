/*
 * DeletedPeerChatPruneTest.kt
 * Family Connect (Android)
 *
 * A deleted account takes its direct chats with it, BOTH halves
 * (docs/protocol.md, "Deleting an account") — so after a peer deletes
 * their account that chat is no longer returned by `GET /chats`, and a
 * client that keeps its row shows a conversation under the peer's OLD
 * name whose every call answers 404.
 *
 * Account deletion is the first thing in this protocol that can make a
 * chat genuinely vanish. Leaving a family cannot: that history is
 * retained and resurfaces on rejoin. So a chat is pruned on exactly two
 * signals, and these tests pin both and the line between them:
 *
 *   - the `member_deleted` frame, which is what a member watching the
 *     chat list sees the row go by;
 *   - a SUCCESSFUL `GET /chats` that does not list it, which is what
 *     heals a device that was offline when it happened.
 *
 * And what must NOT be pruned, which is the half that costs history if
 * it is wrong: nothing at all on a failed or errored list, because a
 * flaky connection would then wipe somebody's messages; and never the
 * family chat or the assistant chat on any response, because neither can
 * disappear server-side while this client can still ask.
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
import me.nettrash.familyconnect.data.db.MessageDao
import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.data.db.MessageStatus
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.ChatDto
import me.nettrash.familyconnect.data.net.dto.ChatListItemDto
import me.nettrash.familyconnect.data.net.dto.ChatsResponse
import me.nettrash.familyconnect.data.net.dto.MemberDto
import me.nettrash.familyconnect.data.net.ws.ServerFrame
import me.nettrash.familyconnect.testutil.FakeChatApi
import me.nettrash.familyconnect.testutil.FakeChatSocket
import me.nettrash.familyconnect.testutil.testChatRepository
import me.nettrash.familyconnect.testutil.createTestDb
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class DeletedPeerChatPruneTest {

    private companion object {
        const val FAMILY_CHAT = 1L
        const val AI_CHAT = 2L
        const val DIRECT_GONE = 42L
        const val DIRECT_KEPT = 43L
        const val PEER_GONE = 9L
        const val PEER_KEPT = 11L
    }

    private val dispatcher = StandardTestDispatcher()

    // A FOREGROUND scope, not backgroundScope: the repository's frame
    // collector lives on it, and advanceUntilIdle() skips work parked on
    // backgroundScope entirely.
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private lateinit var db: AppDatabase
    private lateinit var chatDao: ChatDao
    private lateinit var messageDao: MessageDao
    private lateinit var chatApi: FakeChatApi
    private lateinit var socket: FakeChatSocket

    @Before
    fun setUp() {
        db = createTestDb(dispatcher)
        chatDao = db.chatDao()
        messageDao = db.messageDao()
        chatApi = FakeChatApi()
        socket = FakeChatSocket()
    }

    @After
    fun tearDown() {
        repoScope.cancel()
        db.close()
    }

    private fun TestScope.repository(): ChatRepository {
        val repository = testChatRepository(chatApi, chatDao, messageDao, socket, repoScope)
        // Let the frame collector subscribe before anything is emitted.
        runCurrent()
        return repository
    }

    // -- Fixtures ----------------------------------------------------------------------

    private fun chatRow(id: Long, kind: String, peerUserId: Long? = null) = ChatEntity(
        id = id,
        kind = kind,
        peerUserId = peerUserId,
        title = if (kind == "direct") "Junior" else "The Smiths",
        unreadCount = 2,
        // Everything keyed by chat id that is not a message lives on this
        // row, so these are what "and any per-chat local state" means.
        myLastReadId = 100L,
        peerLastReadId = 99L,
        lastMessageBody = "See you at six",
        lastMessageAt = 1_700_000_000_000L,
        lastMessageSenderId = peerUserId,
        maxReactionSeq = 7L,
        maxEditSeq = 8L,
        maxPollSeq = 9L,
    )

    private suspend fun seedEveryKindOfChat() {
        chatDao.upsertAll(
            listOf(
                chatRow(FAMILY_CHAT, "family"),
                chatRow(AI_CHAT, "ai"),
                chatRow(DIRECT_GONE, "direct", PEER_GONE),
                chatRow(DIRECT_KEPT, "direct", PEER_KEPT),
            ),
        )
        for (chatId in listOf(FAMILY_CHAT, AI_CHAT, DIRECT_GONE, DIRECT_KEPT)) {
            messageDao.insertIgnore(
                listOf(
                    message(chatId, serverId = chatId * 100 + 1),
                    message(chatId, serverId = chatId * 100 + 2),
                ),
            )
        }
    }

    private fun message(chatId: Long, serverId: Long) = MessageEntity(
        clientMsgId = "c$serverId",
        serverId = serverId,
        chatId = chatId,
        senderId = 9L,
        body = "body $serverId",
        createdAt = serverId,
        status = MessageStatus.SENT,
    )

    private fun listing(vararg chats: Pair<Long, String>) = ApiResult.Ok(
        ChatsResponse(
            chats.map { (id, kind) ->
                ChatListItemDto(
                    chat = ChatDto(
                        id = id,
                        kind = kind,
                        title = "Chat $id",
                        peerUserId = when (id) {
                            DIRECT_GONE -> PEER_GONE
                            DIRECT_KEPT -> PEER_KEPT
                            else -> null
                        },
                    ),
                    unreadCount = 0,
                )
            },
        ),
    )

    private val everythingButTheDeletedChat = listOf(
        FAMILY_CHAT to "family",
        AI_CHAT to "ai",
        DIRECT_KEPT to "direct",
    )

    private suspend fun held(chatId: Long) = chatDao.getById(chatId) != null

    private suspend fun messageCount(chatId: Long) =
        messageDao.observeMessages(chatId, limit = 50).first().size

    // -- The general repair: a successful list that omits a direct chat ----------------

    @Test
    fun `a successful list that omits a direct chat prunes it and its messages`() =
        runTest(dispatcher) {
            val repository = repository()
            seedEveryKindOfChat()
            chatApi.chatsResult = listing(*everythingButTheDeletedChat.toTypedArray())

            repository.refreshChats()

            // The row is gone, and with it the unread badge, both read
            // markers and all three catch-up cursors — they live on it.
            assertThat(held(DIRECT_GONE)).isFalse()
            // …and its messages, which nothing would ever read again.
            assertThat(messageCount(DIRECT_GONE)).isEqualTo(0)
            // The chat list no longer offers it.
            assertThat(chatDao.observeChats().first().map { it.id })
                .doesNotContain(DIRECT_GONE)
        }

    @Test
    fun `a direct chat the server still lists is left alone`() = runTest(dispatcher) {
        val repository = repository()
        seedEveryKindOfChat()
        chatApi.chatsResult = listing(*everythingButTheDeletedChat.toTypedArray())

        repository.refreshChats()

        assertThat(held(DIRECT_KEPT)).isTrue()
        assertThat(messageCount(DIRECT_KEPT)).isEqualTo(2)
        // The local-only state on the surviving row survives the merge.
        val kept = chatDao.getById(DIRECT_KEPT)!!
        assertThat(kept.myLastReadId).isEqualTo(100L)
        assertThat(kept.maxReactionSeq).isEqualTo(7L)
    }

    /**
     * The half that costs history if it is wrong.
     *
     * "Not in the response" only means "gone" when there IS a response.
     * An error or a dropped connection says nothing about what the server
     * holds, and pruning on one would let a train tunnel delete a family's
     * messages.
     */
    @Test
    fun `a chat list that failed with an error prunes nothing`() = runTest(dispatcher) {
        val repository = repository()
        seedEveryKindOfChat()
        chatApi.chatsResult = ApiResult.HttpError(500, null, null)

        repository.refreshChats()

        assertThat(held(DIRECT_GONE)).isTrue()
        assertThat(messageCount(DIRECT_GONE)).isEqualTo(2)
        assertThat(held(DIRECT_KEPT)).isTrue()
    }

    @Test
    fun `a chat list that never reached the server prunes nothing`() = runTest(dispatcher) {
        val repository = repository()
        seedEveryKindOfChat()
        chatApi.chatsResult = ApiResult.NetworkError(IllegalStateException("offline"))

        repository.refreshChats()

        assertThat(held(DIRECT_GONE)).isTrue()
        assertThat(messageCount(DIRECT_GONE)).isEqualTo(2)
        assertThat(held(DIRECT_KEPT)).isTrue()
    }

    /**
     * The interleaving that would make the prune destroy a chat nobody
     * deleted: `POST /chats/direct` lands DURING an in-flight `GET
     * /chats`, so the response was computed before that chat existed and
     * cannot possibly list it. Pruning on that absence deletes the
     * conversation the user is at that moment typing into.
     *
     * The candidate set is therefore snapshotted before the await: only
     * chats that were already here when we asked can be pruned by the
     * answer.
     */
    @Test
    fun `a direct chat started during the request is not pruned by it`() = runTest(dispatcher) {
        val repository = repository()
        seedEveryKindOfChat()
        chatApi.chatsResult = listing(*everythingButTheDeletedChat.toTypedArray())
        val gate = CompletableDeferred<Unit>()
        chatApi.chatsGate = gate

        val refresh = launch { repository.refreshChats() }
        runCurrent()

        // A brand-new direct chat with somebody the response knows nothing
        // about, plus the optimistic first message in it.
        chatDao.upsertAll(listOf(chatRow(50L, "direct", peerUserId = 77L)))
        messageDao.insertIgnore(listOf(message(50L, serverId = 5001L)))
        runCurrent()

        gate.complete(Unit)
        refresh.join()

        assertThat(held(50L)).isTrue()
        assertThat(messageCount(50L)).isEqualTo(1)
        // …and the chat that really is gone still went.
        assertThat(held(DIRECT_GONE)).isFalse()
    }

    /**
     * The scope of the prune, and the reason it is scoped at all: the
     * family chat lives as long as the family and the assistant thread as
     * long as the account, so neither can be missing from a list this
     * client successfully made. If one ever is, the answer is a bug
     * somewhere else — never deleting the family's history.
     */
    @Test
    fun `the family chat and the assistant chat are never pruned`() = runTest(dispatcher) {
        val repository = repository()
        seedEveryKindOfChat()
        // A response that lists NEITHER of them (and no direct chat
        // either) — the worst thing a well-formed 200 could say.
        chatApi.chatsResult = ApiResult.Ok(ChatsResponse(emptyList()))

        repository.refreshChats()

        assertThat(held(FAMILY_CHAT)).isTrue()
        assertThat(messageCount(FAMILY_CHAT)).isEqualTo(2)
        assertThat(held(AI_CHAT)).isTrue()
        assertThat(messageCount(AI_CHAT)).isEqualTo(2)
        // Only the direct chats went.
        assertThat(held(DIRECT_GONE)).isFalse()
        assertThat(held(DIRECT_KEPT)).isFalse()
    }

    // -- The immediate half: the member_deleted frame ----------------------------------

    @Test
    fun `the member deleted frame drops the direct chat with that peer`() =
        runTest(dispatcher) {
            repository()
            seedEveryKindOfChat()

            socket.emit(ServerFrame.MemberDeleted(familyId = 3, member = tombstone(PEER_GONE)))
            runCurrent()

            assertThat(held(DIRECT_GONE)).isFalse()
            assertThat(messageCount(DIRECT_GONE)).isEqualTo(0)

            // The RIGHT chat: nothing else moved. The family chat in
            // particular keeps every message the deleted account left in
            // it — the person is erased, the words stay.
            assertThat(held(DIRECT_KEPT)).isTrue()
            assertThat(messageCount(DIRECT_KEPT)).isEqualTo(2)
            assertThat(held(FAMILY_CHAT)).isTrue()
            assertThat(messageCount(FAMILY_CHAT)).isEqualTo(2)
            assertThat(held(AI_CHAT)).isTrue()
        }

    /**
     * A peer this account only ever shared a direct chat with may be in
     * another family altogether, or in none — and then the frame carries
     * no `family_id` at all. It is keyed on the member, so it still lands.
     */
    @Test
    fun `a tombstone with no family still drops the direct chat`() = runTest(dispatcher) {
        repository()
        seedEveryKindOfChat()

        socket.emit(ServerFrame.MemberDeleted(familyId = null, member = tombstone(PEER_GONE)))
        runCurrent()

        assertThat(held(DIRECT_GONE)).isFalse()
        assertThat(held(DIRECT_KEPT)).isTrue()
    }

    @Test
    fun `a tombstone for somebody with no direct chat here changes nothing`() =
        runTest(dispatcher) {
            repository()
            seedEveryKindOfChat()

            socket.emit(ServerFrame.MemberDeleted(familyId = 3, member = tombstone(777L)))
            runCurrent()

            assertThat(chatDao.observeChats().first().map { it.id })
                .containsExactly(FAMILY_CHAT, AI_CHAT, DIRECT_GONE, DIRECT_KEPT)
        }

    private fun tombstone(id: Long) = MemberDto(
        id = id,
        username = "junior",
        // The server's ENGLISH placeholder, exactly as the frame carries it.
        displayName = "Deleted account",
        avatarVersion = 0,
        deleted = true,
    )
}
