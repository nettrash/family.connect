/*
 * SyncEngineTest.kt
 * Family Connect (Android)
 *
 * The protocol's four-step resync: reconcile membership, refresh the
 * chat list (server unread wins), page after_id per chat until a short
 * page, then flush pending sends. Real Room, fakes on the wire.
 */

package me.nettrash.familyconnect.data.repo

import org.robolectric.RuntimeEnvironment
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import me.nettrash.familyconnect.data.net.ws.ServerFrame
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.data.db.MessageStatus
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.ChatDto
import me.nettrash.familyconnect.data.net.dto.ChatListItemDto
import me.nettrash.familyconnect.data.net.dto.BoardResponse
import me.nettrash.familyconnect.data.net.dto.PackResponse
import me.nettrash.familyconnect.data.net.dto.ChatsResponse
import me.nettrash.familyconnect.data.net.dto.FamilyDto
import me.nettrash.familyconnect.data.net.dto.FamilyMineResponse
import me.nettrash.familyconnect.data.net.dto.MeResponse
import me.nettrash.familyconnect.data.net.dto.MemberDto
import me.nettrash.familyconnect.data.net.dto.MessagesResponse
import me.nettrash.familyconnect.data.net.dto.ReactionDto
import me.nettrash.familyconnect.data.net.dto.ReactionsCatchUpResponse
import me.nettrash.familyconnect.data.net.dto.PollCodec
import me.nettrash.familyconnect.data.net.dto.PollsCatchUpResponse
import me.nettrash.familyconnect.data.net.dto.ReactionsCodec
import me.nettrash.familyconnect.data.net.ws.ClientFrame
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.testutil.FakeAuthApi
import me.nettrash.familyconnect.testutil.FakeAttachmentApi
import me.nettrash.familyconnect.testutil.FakeBoardApi
import me.nettrash.familyconnect.testutil.FakePackApi
import me.nettrash.familyconnect.testutil.packItemDto
import me.nettrash.familyconnect.testutil.packTombstone
import me.nettrash.familyconnect.testutil.FakeChatApi
import me.nettrash.familyconnect.testutil.FakeChatSocket
import me.nettrash.familyconnect.testutil.FakeFamilyApi
import me.nettrash.familyconnect.testutil.FakePosterCache
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.FakeTokenStore
import me.nettrash.familyconnect.testutil.RecordingWiper
import me.nettrash.familyconnect.testutil.testChatRepository
import me.nettrash.familyconnect.testutil.createTestDb
import me.nettrash.familyconnect.testutil.messageDto
import me.nettrash.familyconnect.testutil.noteDto
import me.nettrash.familyconnect.testutil.pollDto
import me.nettrash.familyconnect.testutil.pollState
import me.nettrash.familyconnect.testutil.reactionState
import me.nettrash.familyconnect.testutil.userDto
import me.nettrash.familyconnect.util.Clock
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class SyncEngineTest {

    private companion object {
        const val ME = 7L
        const val PEER = 9L
        const val FAMILY_CHAT = 1L
    }

    // One scheduler for the tests AND for Room — see TestDb.kt. Repos
    // live on a foreground scope, not backgroundScope, so runCurrent /
    // advanceUntilIdle drive their collectors (see MessageRepositoryTest).
    private val dispatcher = StandardTestDispatcher()
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private lateinit var db: AppDatabase
    private val authApi = FakeAuthApi()
    private val chatApi = FakeChatApi()
    private val attachmentApi = FakeAttachmentApi()
    private val familyApi = FakeFamilyApi()
    private val boardApi = FakeBoardApi()
    private val packApi = FakePackApi()
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
        db = createTestDb(dispatcher)
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
        repoScope.cancel()
        db.close()
    }

    private fun TestScope.newEngine(): SyncEngine {
        val sessionRepository = SessionRepository(
            authApi = authApi,
            tokenStore = tokenStore,
            settings = settings,
            wiper = wiper,
            unauthorizedEvents = MutableSharedFlow(),
            scope = repoScope,
        )
        val chatRepository = testChatRepository(chatApi, db.chatDao(), db.messageDao(), socket, repoScope)
        val familyRepository = FamilyRepository(
            familyApi = familyApi,
            authApi = authApi,
            memberDao = db.memberDao(),
            settings = settings,
            sessionRepository = sessionRepository,
            socket = socket,
            scope = repoScope,
        )
        val messageRepository = MessageRepository(
            appContext = RuntimeEnvironment.getApplication(),
            chatApi = chatApi,
            attachmentApi = attachmentApi,
            messageDao = db.messageDao(),
            chatDao = db.chatDao(),
            socket = socket,
            settings = settings,
            chatRepository = chatRepository,
            posterCache = FakePosterCache(),
            scope = repoScope,
            clock = Clock { 1_000_000L },
            pendingAttachmentDao = db.pendingAttachmentDao(),
            staging = MediaStaging(RuntimeEnvironment.getApplication()),
        )
        runCurrent()
        val boardRepository = BoardRepository(
            boardApi = boardApi,
            noteDao = db.noteDao(),
            memberDao = db.memberDao(),
            settings = settings,
            socket = socket,
            scope = repoScope,
        )
        val packRepository = PackRepository(
            context = RuntimeEnvironment.getApplication(),
            packApi = packApi,
            attachmentApi = attachmentApi,
            packDao = db.packDao(),
            settings = settings,
            socket = socket,
            scope = repoScope,
        )
        return SyncEngine(
            sessionRepository = sessionRepository,
            chatRepository = chatRepository,
            familyRepository = familyRepository,
            messageRepository = messageRepository,
            boardRepository = boardRepository,
            packRepository = packRepository,
            chatDao = db.chatDao(),
            messageDao = db.messageDao(),
        )
    }

    private fun scriptChats(
        unread: Int = 3,
        maxReactionSeq: Long? = null,
        maxPollSeq: Long? = null,
    ) {
        chatApi.chatsResult = ApiResult.Ok(
            ChatsResponse(
                listOf(
                    ChatListItemDto(
                        chat = ChatDto(id = FAMILY_CHAT, kind = "family", title = "The Smiths"),
                        lastMessage = messageDto(id = 5, chatId = FAMILY_CHAT, senderId = PEER),
                        unreadCount = unread,
                        maxReactionSeq = maxReactionSeq,
                        maxPollSeq = maxPollSeq,
                    ),
                ),
            ),
        )
    }

    @Test
    fun resyncPullsChatsRosterAndCatchUpPages() = runTest(dispatcher) {
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
    fun aLiveMessageArrivingMidCatchUpDoesNotSkipTheBacklog() = runTest(dispatcher) {
        // The loss this pins: the page loop used to re-read max(serverId)
        // from the store before every request. A live `message` frame
        // landing between two pages writes a far higher id, the next
        // request asks for everything after THAT, and the whole backlog in
        // between is skipped for good — `after_id` never looks back, and
        // loadOlder only pages older than the OLDEST row held.
        val engine = newEngine()
        scriptChats()
        db.messageDao().insertIgnore(
            listOf(MessageEntity("s5", 5, FAMILY_CHAT, PEER, "old", 1L, MessageStatus.SENT)),
        )
        val asked = mutableListOf<Long?>()
        chatApi.messagesHandler = { _, _, afterId, limit ->
            asked += afterId
            when (afterId) {
                5L -> {
                    // The family is chatting: a brand-new message arrives
                    // on the socket while this page is in flight.
                    db.messageDao().insertIgnore(
                        listOf(
                            MessageEntity(
                                "s5000", 5000, FAMILY_CHAT, PEER, "live", 2L, MessageStatus.SENT,
                            ),
                        ),
                    )
                    ApiResult.Ok(
                        MessagesResponse(
                            (6L until 6L + limit).map {
                                messageDto(id = it, chatId = FAMILY_CHAT, senderId = PEER)
                            },
                        ),
                    )
                }
                else -> ApiResult.Ok(MessagesResponse(emptyList()))
            }
        }

        engine.resync()

        // The second request continued from where the FIRST PAGE ended
        // (id 205), not from the 5000 the live frame put in the store.
        assertThat(asked).containsExactly(5L, 205L).inOrder()
    }

    @Test
    fun resyncFlushesPendingOutboundMessages() = runTest(dispatcher) {
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
    fun resyncStopsWhenMembershipIsGone() = runTest(dispatcher) {
        val engine = newEngine()
        // Server says: no family any more.
        authApi.meResult = ApiResult.Ok(MeResponse(user = userDto(ME)))
        scriptChats()

        engine.resync()

        // No chat list fetch once canChat is false.
        assertThat(chatApi.messagesCalls).isEqualTo(0)
        assertThat(db.chatDao().getById(FAMILY_CHAT)).isNull()
    }

    // -- Reactions ----------------------------------------------------------

    @Test
    fun resyncRepairsReactionsMissedWhileOffline() = runTest(dispatcher) {
        // The gap this step closes: someone reacted while this device
        // was offline — no frame ever arrived, and the message itself
        // is already held, so message catch-up won't re-deliver it.
        val engine = newEngine()
        scriptChats(maxReactionSeq = 10L)
        db.messageDao().insertIgnore(
            listOf(MessageEntity("s5", 5, FAMILY_CHAT, PEER, "old", 1L, MessageStatus.SENT)),
        )
        chatApi.reactionsHandler = { chatId, afterSeq, _ ->
            assertThat(chatId).isEqualTo(FAMILY_CHAT)
            assertThat(afterSeq).isEqualTo(0L) // the stored cursor, not the server's
            ApiResult.Ok(
                ReactionsCatchUpResponse(
                    listOf(reactionState(5L, 10L, listOf(ReactionDto(PEER, "❤️")))),
                ),
            )
        }

        engine.resync()

        val row = db.messageDao().findByServerId(5L)!!
        assertThat(ReactionsCodec.decode(row.reactionsJson)).containsExactly(ReactionDto(PEER, "❤️"))
        assertThat(row.reactionSeq).isEqualTo(10L)
        assertThat(db.chatDao().maxReactionSeq(FAMILY_CHAT)).isEqualTo(10L)
        assertThat(chatApi.reactionsCalls).isEqualTo(1) // short page — one fetch

        // A second resync with the server no further ahead skips the
        // reaction fetch entirely.
        engine.resync()
        assertThat(chatApi.reactionsCalls).isEqualTo(1)
    }

    @Test
    fun resyncAdvancesTheReactionCursorEvenForMessagesNotHeldLocally() = runTest(dispatcher) {
        val engine = newEngine()
        scriptChats(maxReactionSeq = 10L)
        // No local copy of message 999 — its state is dropped, but the
        // cursor must still advance or every future resync would refetch
        // (and re-drop) the same page forever.
        chatApi.reactionsHandler = { _, _, _ ->
            ApiResult.Ok(
                ReactionsCatchUpResponse(
                    listOf(reactionState(999L, 10L, listOf(ReactionDto(PEER, "❤️")))),
                ),
            )
        }

        engine.resync()

        assertThat(db.messageDao().findByServerId(999L)).isNull()
        assertThat(db.chatDao().maxReactionSeq(FAMILY_CHAT)).isEqualTo(10L)

        engine.resync()
        assertThat(chatApi.reactionsCalls).isEqualTo(1) // cursor caught up — no refetch
    }

    @Test
    fun resyncSkipsReactionCatchUpWhenTheServerReportsNone() = runTest(dispatcher) {
        val engine = newEngine()
        scriptChats() // max_reaction_seq absent — nothing ever reacted

        engine.resync()

        assertThat(chatApi.reactionsCalls).isEqualTo(0)
    }

    @Test
    fun resyncPreservesLocalReadMarkersWhileTakingServerUnread() = runTest(dispatcher) {
        val engine = newEngine()
        scriptChats(unread = 7)
        // Pre-existing chat row with local-only markers.
        engine.resync() // creates the chat
        db.chatDao().setMyLastRead(FAMILY_CHAT, 4)
        db.chatDao().setPeerLastRead(FAMILY_CHAT, 2)
        db.chatDao().advanceMaxReactionSeq(FAMILY_CHAT, 9)
        db.chatDao().advanceMaxPollSeq(FAMILY_CHAT, 11)

        scriptChats(unread = 0)
        engine.resync()

        val chat = db.chatDao().getById(FAMILY_CHAT)!!
        assertThat(chat.unreadCount).isEqualTo(0) // server wins
        assertThat(chat.myLastReadId).isEqualTo(4L) // local survives
        assertThat(chat.peerLastReadId).isEqualTo(2L)
        assertThat(chat.maxReactionSeq).isEqualTo(9L) // reaction cursor too
        // And the poll cursor — NEVER the server's value, which says
        // what exists rather than what this device has applied.
        assertThat(chat.maxPollSeq).isEqualTo(11L)
    }

    // -- Polls --------------------------------------------------------------

    @Test
    fun resyncRepairsVotesMissedWhileOffline() = runTest(dispatcher) {
        // The gap this step closes: somebody voted while this device was
        // offline — no frame ever arrived, and the message itself is
        // already held, so `after_id` message catch-up cannot re-deliver
        // it. A vote is nothing but a change to an older row.
        val engine = newEngine()
        scriptChats(maxPollSeq = 90L)
        db.messageDao().insertIgnore(
            listOf(
                MessageEntity(
                    "s5",
                    5,
                    FAMILY_CHAT,
                    PEER,
                    "Pizza or pasta?",
                    1L,
                    MessageStatus.SENT,
                    pollJson = PollCodec.encode(
                        pollDto(88, "Pizza" to emptyList(), "Pasta" to emptyList()),
                    ),
                    pollSeq = 88,
                ),
            ),
        )
        chatApi.pollsHandler = { chatId, afterSeq, _ ->
            assertThat(chatId).isEqualTo(FAMILY_CHAT)
            assertThat(afterSeq).isEqualTo(0L) // the stored cursor, not the server's
            ApiResult.Ok(
                PollsCatchUpResponse(
                    listOf(pollState(5L, pollDto(90, "Pizza" to listOf(PEER), "Pasta" to emptyList()))),
                ),
            )
        }

        engine.resync()

        val row = db.messageDao().findByServerId(5L)!!
        assertThat(PollCodec.decode(row.pollJson)!!.options[0].votes).containsExactly(PEER)
        assertThat(row.pollSeq).isEqualTo(90L)
        assertThat(db.chatDao().maxPollSeq(FAMILY_CHAT)).isEqualTo(90L)
        assertThat(chatApi.pollsCalls).isEqualTo(1) // short page — one fetch

        // A second resync with the server no further ahead skips the
        // poll fetch entirely.
        engine.resync()
        assertThat(chatApi.pollsCalls).isEqualTo(1)
    }

    @Test
    fun resyncSkipsPollCatchUpWhenTheChatHoldsNoPoll() = runTest(dispatcher) {
        val engine = newEngine()
        scriptChats() // max_poll_seq absent — the family has never held one

        engine.resync()

        assertThat(chatApi.pollsCalls).isEqualTo(0)
    }

    /**
     * The board is caught up by the RESYNC, not only by the board screen
     * (issue #53). A cache filled by socket frames alone is a cache full of
     * holes, and a note this device never held would first appear the
     * moment somebody DRAGGED it — which the badge then counted as news.
     */
    @Test
    fun resyncCatchesTheBoardUp() = runTest(dispatcher) {
        val engine = newEngine()
        scriptChats()
        familyApi.mineResult = ApiResult.Ok(
            FamilyMineResponse(
                family = FamilyDto(id = 1, name = "The Smiths", joinPolicy = "open"),
                members = listOf(MemberDto(ME, "anna", "Anna", "owner")),
                maxBoardSeq = 40,
            ),
        )
        boardApi.board = BoardResponse(
            notes = listOf(
                noteDto(id = 1, boardSeq = 38, contentSeq = 12),
                noteDto(id = 2, boardSeq = 40, contentSeq = 40),
            ),
            maxBoardSeq = 40,
        )

        engine.resync()

        val notes = db.noteDao().observeNotes().first()
        assertThat(notes.map { it.id }).containsExactly(1L, 2L)
        assertThat(notes.single { it.id == 1L }.contentSeq).isEqualTo(12)
        assertThat(settings.state.first().boardCursor).isEqualTo(40)
    }

    // -- The sticker pack (docs/protocol.md, "Sticker pack") ------------------

    private fun scriptFamily(
        maxPackSeq: Long? = null,
        maxPackItems: Int? = 200,
        maxPackItemBytes: Long? = 524_288,
        maxRoundVideoMs: Long? = null,
        maxRoundVideoBytes: Long? = null,
    ) {
        familyApi.mineResult = ApiResult.Ok(
            FamilyMineResponse(
                family = FamilyDto(id = 1, name = "The Smiths", joinPolicy = "open"),
                members = listOf(MemberDto(ME, "anna", "Anna", "owner")),
                maxPackSeq = maxPackSeq,
                maxPackItems = maxPackItems,
                maxPackItemBytes = maxPackItemBytes,
                maxRoundVideoMs = maxRoundVideoMs,
                maxRoundVideoBytes = maxRoundVideoBytes,
            ),
        )
    }

    /**
     * After `GET /families/mine`: a device that holds no pack reads the
     * whole of it, and learns the two ceilings the panel and the picker
     * are drawn from.
     */
    @Test
    fun resyncReadsTheWholePackOnADeviceThatHoldsNone() = runTest(dispatcher) {
        val engine = newEngine()
        scriptChats()
        scriptFamily(maxPackSeq = 14)
        packApi.pack = PackResponse(
            items = listOf(packItemDto(id = 5, packSeq = 12), packItemDto(id = 6, packSeq = 14)),
            maxPackSeq = 14,
        )

        engine.resync()

        assertThat(db.packDao().items().map { it.id }).containsExactly(5L, 6L).inOrder()
        assertThat(packApi.fullReads).isEqualTo(1)
        assertThat(settings.current.packCursor).isEqualTo(14)
        assertThat(settings.current.packLimits).isEqualTo(PackLimits(200, 524_288))
    }

    /**
     * The resync that a socket opening starts is what lets a `pack_item`
     * frame move the cursor from then on: "the frame only once this
     * connection has caught up".
     */
    @Test
    fun aResyncOnAnOpenSocketLetsPackFramesMoveTheCursor() = runTest(dispatcher) {
        val engine = newEngine()
        runCurrent()
        scriptChats()
        scriptFamily(maxPackSeq = 14)
        packApi.pack = PackResponse(listOf(packItemDto(id = 5, packSeq = 12)), maxPackSeq = 14)
        socket.setOpen(true)

        engine.resync()
        assertThat(settings.current.packCursor).isEqualTo(14)

        socket.emit(ServerFrame.PackItem(packItemDto(id = 7, packSeq = 15)))
        runCurrent()
        assertThat(settings.current.packCursor).isEqualTo(15)
    }

    /**
     * The mark a pass catches up to is the one `GET /families/mine`
     * reported. A reconnect DURING that read means the mark predates the
     * new connection, so the pass must not call it caught up — whatever
     * changed while the wire was down is below the first frame it delivers.
     */
    @Test
    fun aResyncOvertakenByAReconnectDoesNotVouchForTheNewConnection() = runTest(dispatcher) {
        val engine = newEngine()
        runCurrent()
        scriptChats()
        scriptFamily(maxPackSeq = 14)
        packApi.pack = PackResponse(listOf(packItemDto(id = 5, packSeq = 12)), maxPackSeq = 14)
        socket.setOpen(true)
        familyApi.onMine = {
            familyApi.onMine = null
            socket.setOpen(false)
            socket.setOpen(true)
        }

        engine.resync()
        assertThat(settings.current.packCursor).isEqualTo(14)

        socket.emit(ServerFrame.PackItem(packItemDto(id = 7, packSeq = 19)))
        runCurrent()

        assertThat(db.packDao().items().map { it.id }).containsExactly(5L, 7L)
        assertThat(settings.current.packCursor).isEqualTo(14)
    }

    /** One that holds a pack loops the change feed instead — tombstones and all. */
    @Test
    fun resyncCatchesAHeldPackUpFromItsCursor() = runTest(dispatcher) {
        val engine = newEngine()
        scriptChats()
        scriptFamily(maxPackSeq = 14)
        packApi.pack = PackResponse(listOf(packItemDto(id = 5, packSeq = 12)), maxPackSeq = 12)
        engine.resync()
        scriptFamily(maxPackSeq = 20)
        packApi.changePages = mutableListOf(
            listOf(packItemDto(id = 7, packSeq = 18), packTombstone(id = 5, packSeq = 20)),
        )

        engine.resync()

        // The cursor the first read left is where the feed is asked from.
        assertThat(packApi.changeRequests).containsExactly(12L)
        assertThat(packApi.fullReads).isEqualTo(1)
        assertThat(db.packDao().items().map { it.id }).containsExactly(7L)
        assertThat(settings.current.packCursor).isEqualTo(20)
    }

    /**
     * `max_pack_items` absent is how a client knows the server predates
     * packs: no request is made, and nothing about stickers is offered.
     */
    @Test
    fun resyncAsksNothingOfAServerThatPredatesPacks() = runTest(dispatcher) {
        val engine = newEngine()
        scriptChats()
        scriptFamily(maxPackSeq = null, maxPackItems = null, maxPackItemBytes = null)

        engine.resync()

        assertThat(packApi.fullReads).isEqualTo(0)
        assertThat(packApi.changeRequests).isEmpty()
        assertThat(settings.current.packLimits).isNull()
    }

    /** A pack with ceilings and no mark has never been written to: no request either. */
    @Test
    fun resyncAsksNothingOfAnUntouchedPack() = runTest(dispatcher) {
        val engine = newEngine()
        scriptChats()
        scriptFamily(maxPackSeq = null)

        engine.resync()

        assertThat(packApi.fullReads).isEqualTo(0)
        // …but the sticker button is there: the server HAS packs.
        assertThat(settings.current.packLimits).isNotNull()
    }

    /** A server rolled back to a build without packs takes the affordances away again. */
    @Test
    fun theCeilingsAreAStateSetAndNotADelta() = runTest(dispatcher) {
        val engine = newEngine()
        scriptChats()
        scriptFamily(maxPackSeq = null)
        engine.resync()
        assertThat(settings.current.packLimits).isNotNull()

        scriptFamily(maxPackSeq = null, maxPackItems = null, maxPackItemBytes = null)
        engine.resync()

        assertThat(settings.current.packLimits).isNull()
    }

    // -- Video messages' limits (#79, "Discovery and limits") -----------------------

    /** The two keys are how this device learns the server has video messages. */
    @Test
    fun resyncLearnsTheVideoMessageLimits() = runTest(dispatcher) {
        val engine = newEngine()
        scriptChats()
        scriptFamily(maxRoundVideoMs = 60_000, maxRoundVideoBytes = 12_582_912)

        engine.resync()

        assertThat(settings.current.roundVideoLimits).isEqualTo(RoundVideoLimits(60_000, 12_582_912))
    }

    /** Their absence is the server predating video messages: no video entry anywhere — and a state-set. */
    @Test
    fun aServerWithoutTheKeysOffersNoVideoMessages() = runTest(dispatcher) {
        val engine = newEngine()
        scriptChats()
        scriptFamily(maxRoundVideoMs = 60_000, maxRoundVideoBytes = 4_194_304)
        engine.resync()
        assertThat(settings.current.roundVideoLimits).isNotNull()

        scriptFamily()
        engine.resync()

        assertThat(settings.current.roundVideoLimits).isNull()
    }
}
