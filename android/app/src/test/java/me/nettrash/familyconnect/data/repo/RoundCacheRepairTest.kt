/*
 * RoundCacheRepairTest.kt
 * Family Connect (Android)
 *
 * A circle cached by a build from before #79 (docs/audio-video-messages-2026-10-04.md,
 * S5.8). Such a build wrote every received attachment set back with only the
 * fields it knew, so a video message that arrived before the upgrade is
 * stored as a plain video — and the catch-up only ever ADDS (`after_id`), so
 * nothing would read it again: it drew square for good on the very phone
 * that had been upgraded to draw it round.
 *
 * The resync now reads such rows once more through the one-message fetch the
 * location repair uses. Everything here goes through `SyncEngine.resync()`,
 * and the pre-#79 rows are written with raw SQL exactly as that build left
 * them (no flag in the JSON, the new column at its migration default), so the
 * test compiles — and fails — against the code before the repair.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.data.db.MessageStatus
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.UndecodableResponseException
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.net.dto.AttachmentsCodec
import me.nettrash.familyconnect.data.net.dto.ChatDto
import me.nettrash.familyconnect.data.net.dto.ChatListItemDto
import me.nettrash.familyconnect.data.net.dto.ChatsResponse
import me.nettrash.familyconnect.data.net.dto.FamilyDto
import me.nettrash.familyconnect.data.net.dto.FamilyMineResponse
import me.nettrash.familyconnect.data.net.dto.MeResponse
import me.nettrash.familyconnect.data.net.dto.MemberDto
import me.nettrash.familyconnect.data.net.dto.MessagesResponse
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.testutil.FakeAttachmentApi
import me.nettrash.familyconnect.testutil.FakeAuthApi
import me.nettrash.familyconnect.testutil.FakeBoardApi
import me.nettrash.familyconnect.testutil.FakeChatApi
import me.nettrash.familyconnect.testutil.FakeChatSocket
import me.nettrash.familyconnect.testutil.FakeFamilyApi
import me.nettrash.familyconnect.testutil.FakePackApi
import me.nettrash.familyconnect.testutil.FakePosterCache
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.FakeTokenStore
import me.nettrash.familyconnect.testutil.RecordingWiper
import me.nettrash.familyconnect.testutil.createTestDb
import me.nettrash.familyconnect.testutil.messageDto
import me.nettrash.familyconnect.testutil.testChatRepository
import me.nettrash.familyconnect.testutil.userDto
import me.nettrash.familyconnect.util.Clock
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class RoundCacheRepairTest {

    private companion object {
        const val ME = 7L
        const val PEER = 9L
        const val CHAT = 1L
    }

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
    private val settings = FakeSettingsRepository(
        SettingsState(
            serverUrl = "https://chat.example.com",
            familyStatus = FamilyStatus.MEMBER,
            myUserId = ME,
        ),
    )

    /** The one-message reads the repair made, in order — server ids. */
    private val asked = mutableListOf<Long>()

    /** What the server answers for one message, by id; absent = it has no such message. */
    private val server = mutableMapOf<Long, ApiResult<MessagesResponse>>()

    @Before
    fun setUp() {
        db = createTestDb(dispatcher)
        val family = FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open")
        authApi.meResult = ApiResult.Ok(MeResponse(user = userDto(ME, "anna"), family = family, role = "member"))
        familyApi.mineResult = ApiResult.Ok(
            FamilyMineResponse(
                family = family,
                members = listOf(MemberDto(ME, "anna", "Anna", "owner"), MemberDto(PEER, "ben", "Ben", "member")),
            ),
        )
        chatApi.chatsResult = ApiResult.Ok(
            ChatsResponse(
                listOf(
                    ChatListItemDto(
                        chat = ChatDto(id = CHAT, kind = "family", title = "The Smiths"),
                        lastMessage = null,
                        unreadCount = 0,
                    ),
                ),
            ),
        )
        chatApi.messagesHandler = { _, beforeId, _, limit ->
            if (beforeId != null && limit == 1) {
                val id = beforeId - 1
                asked += id
                server[id] ?: ApiResult.Ok(MessagesResponse(emptyList()))
            } else {
                // The catch-up and any history read: nothing new.
                ApiResult.Ok(MessagesResponse(emptyList()))
            }
        }
    }

    @After
    fun tearDown() {
        repoScope.cancel()
        db.close()
    }

    private fun TestScope.newEngine(): SyncEngine {
        val sessionRepository = SessionRepository(
            authApi = authApi,
            tokenStore = FakeTokenStore("tok"),
            settings = settings,
            wiper = RecordingWiper(),
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
        return SyncEngine(
            sessionRepository = sessionRepository,
            chatRepository = chatRepository,
            familyRepository = familyRepository,
            messageRepository = messageRepository,
            boardRepository = BoardRepository(
                boardApi = boardApi,
                noteDao = db.noteDao(),
                memberDao = db.memberDao(),
                settings = settings,
                socket = socket,
                scope = repoScope,
            ),
            packRepository = PackRepository(
                context = RuntimeEnvironment.getApplication(),
                packApi = packApi,
                attachmentApi = attachmentApi,
                packDao = db.packDao(),
                settings = settings,
                socket = socket,
                scope = repoScope,
            ),
            chatDao = db.chatDao(),
            messageDao = db.messageDao(),
        )
    }

    private fun video(id: Long, round: Boolean? = null) = AttachmentDto(
        id = id, kind = "video", mime = "video/mp4", size = 4_000, width = 480, height = 480,
        durationMs = 23_400, hasPreview = true, round = round,
    )

    private fun photo(id: Long) = AttachmentDto(
        id = id, kind = "photo", mime = "image/jpeg", size = 2_000, width = 800, height = 600, hasPreview = true,
    )

    /**
     * A row exactly as a pre-#79 build left it: raw SQL, so it is the shape
     * that build wrote (no `round` in the JSON) with every column it never
     * knew at its default.
     */
    private fun cachedByOldBuild(
        serverId: Long?,
        attachments: List<AttachmentDto>,
        body: String = "",
        clientMsgId: String = "s$serverId",
    ) {
        val first = attachments.first()
        // The old encoder knew no `round`, so the stored set never carries one.
        val json = AttachmentsCodec.encode(attachments.map { it.copy(round = null) }).replace("'", "''")
        db.openHelper.writableDatabase.execSQL(
            """
            INSERT INTO messages (clientMsgId, serverId, chatId, senderId, body, createdAt, status,
                attachmentId, attachmentKind, attachmentMime, attachmentSize, attachmentWidth,
                attachmentHeight, attachmentDurationMs, attachmentHasPreview, attachmentsJson)
            VALUES ('$clientMsgId', ${serverId ?: "NULL"}, $CHAT, $PEER, '$body', ${serverId ?: 0} * 1000,
                'SENT', ${first.id}, '${first.kind}', '${first.mime}', ${first.size}, ${first.width},
                ${first.height}, ${first.durationMs ?: "NULL"}, ${if (first.hasPreview) 1 else 0}, '$json')
            """.trimIndent(),
        )
    }

    /** The server's copy of a circle: the same video, with the flag. */
    private fun serverHasCircle(id: Long) {
        server[id] = ApiResult.Ok(
            MessagesResponse(
                listOf(messageDto(id = id, chatId = CHAT, senderId = PEER, body = "", attachments = listOf(video(id, round = true)))),
            ),
        )
    }

    private suspend fun stored(serverId: Long) = db.messageDao().findByServerId(serverId)!!.attachmentList

    private suspend fun isRound(serverId: Long) = stored(serverId).single().isRound

    @Test
    fun aCircleCachedBeforeTheUpgradeIsRoundAfterTheNextResync() = runTest(dispatcher) {
        cachedByOldBuild(50, listOf(video(50)))
        serverHasCircle(50)
        assertThat(isRound(50)).isFalse()

        newEngine().resync()

        assertThat(asked).containsExactly(50L)
        assertThat(isRound(50)).isTrue()
        // The flat columns still mirror the first attachment.
        val row = db.messageDao().findByServerId(50)!!
        assertThat(row.attachmentKind).isEqualTo("video")
        assertThat(row.attachmentId).isEqualTo(50L)
        // And the chat list says what it is.
        assertThat(db.chatDao().getById(CHAT)!!.lastMessageBody).isEqualTo("Video message")
    }

    @Test
    fun theRepairMovesNoCursor() = runTest(dispatcher) {
        // A row the thread read fetched sits outside the contiguous window
        // (`detached`); one copy of it says nothing about the gap around it,
        // so the repair must not attach it the way a history page would.
        db.messageDao().insertIgnore(
            listOf(MessageEntity("s10", 10, CHAT, PEER, "hello", 10_000, MessageStatus.SENT)),
        )
        cachedByOldBuild(50, listOf(video(50)))
        db.openHelper.writableDatabase.execSQL("UPDATE messages SET detached = 1 WHERE serverId = 50")
        serverHasCircle(50)

        newEngine().resync()

        assertThat(asked).containsExactly(50L)
        assertThat(isRound(50)).isTrue()
        assertThat(db.messageDao().findByServerId(50)!!.detached).isTrue()
        // The window's edges — what the catch-up and the history read page
        // from — are where they were.
        assertThat(db.messageDao().maxServerId(CHAT)).isEqualTo(10L)
        assertThat(db.messageDao().oldestServerId(CHAT)).isEqualTo(10L)
    }

    @Test
    fun onlyPossibleCirclesAreAskedForOnceAndNewestFirst() = runTest(dispatcher) {
        // Candidates: one video, no body, on the server, not yet known.
        cachedByOldBuild(10, listOf(video(10)))
        cachedByOldBuild(30, listOf(video(30)))
        cachedByOldBuild(20, listOf(video(20)))
        serverHasCircle(30)
        // 20 is an ordinary video after all; 10 the server no longer has.
        server[20] = ApiResult.Ok(
            MessagesResponse(listOf(messageDto(id = 20, chatId = CHAT, senderId = PEER, body = "", attachments = listOf(video(20))))),
        )
        // Not candidates: a captioned video, a photo, two videos, a send
        // that never reached the server.
        cachedByOldBuild(40, listOf(video(40)), body = "look")
        cachedByOldBuild(45, listOf(photo(45)))
        cachedByOldBuild(35, listOf(video(35), video(36)))
        cachedByOldBuild(null, listOf(video(60)), clientMsgId = "local-unsent")
        // Written by THIS build: it already knows the flag (a square video).
        db.messageDao().insertIgnore(
            listOf(
                MessageEntity(
                    clientMsgId = "s70", serverId = 70, chatId = CHAT, senderId = PEER, body = "",
                    createdAt = 70_000, status = MessageStatus.SENT, attachmentId = 70,
                    attachmentKind = "video", attachmentMime = "video/mp4",
                    attachmentsJson = AttachmentsCodec.encode(listOf(video(70))),
                ),
            ),
        )
        val engine = newEngine()

        engine.resync()

        assertThat(asked).containsExactly(30L, 20L, 10L).inOrder()
        assertThat(isRound(30)).isTrue()
        assertThat(isRound(20)).isFalse()
        assertThat(isRound(10)).isFalse()
        assertThat(stored(35)).hasSize(2)

        // Every one of them is settled: the next resync asks for nothing.
        asked.clear()
        engine.resync()
        assertThat(asked).isEmpty()
    }

    @Test
    fun atMostTwentyFiveAreAskedPerResync() = runTest(dispatcher) {
        (1L..30L).forEach { id ->
            cachedByOldBuild(id, listOf(video(id)))
            serverHasCircle(id)
        }
        val engine = newEngine()

        engine.resync()
        assertThat(asked).containsExactlyElementsIn((30L downTo 6L).toList()).inOrder()
        assertThat(isRound(6)).isTrue()
        assertThat(isRound(5)).isFalse()

        asked.clear()
        engine.resync()
        assertThat(asked).containsExactly(5L, 4L, 3L, 2L, 1L).inOrder()
        assertThat((1L..30L).all { isRound(it) }).isTrue()

        asked.clear()
        engine.resync()
        assertThat(asked).isEmpty()
    }

    @Test
    fun aFailureSkipsThatMessageAndThePassGoesOn() = runTest(dispatcher) {
        (1L..3L).forEach { id ->
            cachedByOldBuild(id, listOf(video(id)))
            serverHasCircle(id)
        }
        server[2] = ApiResult.HttpError(500, "internal", "boom")
        val engine = newEngine()

        engine.resync()

        assertThat(asked).containsExactly(3L, 2L, 1L).inOrder()
        assertThat(isRound(3)).isTrue()
        assertThat(isRound(2)).isFalse()
        assertThat(isRound(1)).isTrue()

        // The skipped one is still unknown, so the next resync asks again.
        serverHasCircle(2)
        asked.clear()
        engine.resync()
        assertThat(asked).containsExactly(2L)
        assertThat(isRound(2)).isTrue()
    }

    @Test
    fun aRefusalOrAnUnreadableAnswerSettlesThatMessageAndThePassGoesOn() = runTest(dispatcher) {
        // The server read the request and refused it, or answered with a
        // body this build cannot read: asking again would get the same
        // answer. Left unknown, the newest such message was asked FIRST on
        // every resync for good. iOS: `roundRepairOutcome` -> `.settled`.
        val refusals = listOf<ApiResult<MessagesResponse>>(
            ApiResult.HttpError(403, "forbidden", "not a member"),
            ApiResult.HttpError(404, "not_found", null),
            ApiResult.HttpError(409, "conflict", null),
            ApiResult.HttpError(400, "bad_request", null),
            ApiResult.HttpError(410, null, null),
            ApiResult.HttpError(413, null, null),
            ApiResult.HttpError(422, "invalid", null),
            ApiResult.NetworkError(UndecodableResponseException(IllegalArgumentException("not json"))),
        )
        // Every refused message sits between two readable circles, so a
        // pass that stopped at any of them would leave the next one square.
        val ids = (1L..(2L * refusals.size + 1))
        ids.forEach { id ->
            cachedByOldBuild(id, listOf(video(id)))
            serverHasCircle(id)
        }
        val refused = refusals.mapIndexed { i, answer -> (2L * i + 2) to answer }.toMap()
        refused.forEach { (id, answer) -> server[id] = answer }
        val engine = newEngine()

        engine.resync()

        assertThat(asked).containsExactlyElementsIn(ids.reversed()).inOrder()
        ids.forEach { id -> assertThat(isRound(id)).isEqualTo(id !in refused) }

        // Settled: even with the server answering now, none is asked again.
        refused.keys.forEach { serverHasCircle(it) }
        asked.clear()
        engine.resync()
        assertThat(asked).isEmpty()
    }

    @Test
    fun anyServerErrorSkipsThatMessageAndAsksAgainNextTime() = runTest(dispatcher) {
        (1L..5L).forEach { id ->
            cachedByOldBuild(id, listOf(video(id)))
            serverHasCircle(id)
        }
        server[4] = ApiResult.HttpError(502, null, null)
        server[2] = ApiResult.HttpError(503, null, null, retryAfterSeconds = 5)
        val engine = newEngine()

        engine.resync()

        assertThat(asked).containsExactly(5L, 4L, 3L, 2L, 1L).inOrder()
        assertThat(isRound(4)).isFalse()
        assertThat(isRound(2)).isFalse()
        assertThat(isRound(1)).isTrue()

        serverHasCircle(4)
        serverHasCircle(2)
        asked.clear()
        engine.resync()
        assertThat(asked).containsExactly(4L, 2L).inOrder()
        assertThat(isRound(4)).isTrue()
        assertThat(isRound(2)).isTrue()
    }

    @Test
    fun aLostSessionStopsThePass() = runTest(dispatcher) {
        // A 401 is about the session, not the message: every further read
        // would be refused the same way, and settling the rows would lose
        // their repair for good. iOS: `.unauthorized` -> `.endPass`.
        (1L..3L).forEach { id ->
            cachedByOldBuild(id, listOf(video(id)))
            serverHasCircle(id)
        }
        server[2] = ApiResult.HttpError(401, "unauthorized", null)
        val engine = newEngine()

        engine.resync()

        assertThat(asked).containsExactly(3L, 2L).inOrder()
        assertThat(isRound(3)).isTrue()
        assertThat(isRound(1)).isFalse()

        // Signed in again: both are read on the next resync.
        serverHasCircle(2)
        asked.clear()
        engine.resync()
        assertThat(asked).containsExactly(2L, 1L).inOrder()
        assertThat(isRound(2)).isTrue()
        assertThat(isRound(1)).isTrue()
    }

    @Test
    fun aRateLimitStopsThePassToo() = runTest(dispatcher) {
        // A 429 is not about THIS message: every further request in the
        // pass would only collect another refusal. (iOS: `.throttled` ->
        // `.endPass`.)
        (1L..3L).forEach { id ->
            cachedByOldBuild(id, listOf(video(id)))
            serverHasCircle(id)
        }
        server[2] = ApiResult.HttpError(429, null, null, retryAfterSeconds = 30)
        val engine = newEngine()

        engine.resync()

        assertThat(asked).containsExactly(3L, 2L).inOrder()
        assertThat(isRound(1)).isFalse()

        serverHasCircle(2)
        asked.clear()
        engine.resync()
        assertThat(asked).containsExactly(2L, 1L).inOrder()
    }

    @Test
    fun aNetworkDownStopsThePass() = runTest(dispatcher) {
        (1L..3L).forEach { id ->
            cachedByOldBuild(id, listOf(video(id)))
            serverHasCircle(id)
        }
        server[2] = ApiResult.NetworkError(java.io.IOException("offline"))
        val engine = newEngine()

        engine.resync()

        assertThat(asked).containsExactly(3L, 2L).inOrder()
        assertThat(isRound(3)).isTrue()
        assertThat(isRound(1)).isFalse()

        // Back online: the rest are read on the next resync.
        serverHasCircle(2)
        asked.clear()
        engine.resync()
        assertThat(asked).containsExactly(2L, 1L).inOrder()
        assertThat(isRound(2)).isTrue()
        assertThat(isRound(1)).isTrue()
    }
}
