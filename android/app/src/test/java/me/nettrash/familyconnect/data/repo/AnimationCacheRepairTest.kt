/*
 * AnimationCacheRepairTest.kt
 * Family Connect (Android)
 *
 * An ANIMATION cached by a build from before #96 (docs/protocol.md,
 * "Animations"). Such a build wrote every received attachment set back with
 * only the fields it knew, so an animation another client sent is stored as a
 * plain video — and the catch-up only ever adds, so it would play with a
 * play button for good on the phone that was upgraded to draw it moving.
 * RoundCacheRepairTest's harness and rules: the resync reads such rows once
 * more, one message each, newest first; the rows are written with raw SQL as
 * a post-#79, pre-#96 build left them (round known, animation not).
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
class AnimationCacheRepairTest {

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

    private fun video(id: Long, animation: Boolean? = null) = AttachmentDto(
        id = id, kind = "video", mime = "video/mp4", size = 4_000, width = 480, height = 270,
        durationMs = 2_100, hasPreview = true, animation = animation,
    )

    private fun photo(id: Long) = AttachmentDto(
        id = id, kind = "photo", mime = "image/jpeg", size = 2_000, width = 800, height = 600, hasPreview = true,
    )

    /**
     * A row exactly as a post-#79, pre-#96 build left it: raw SQL, no
     * `animation` in the JSON, the round question known and the animation
     * one at its migration default.
     */
    private fun cachedByOldBuild(
        serverId: Long?,
        attachments: List<AttachmentDto>,
        body: String = "",
        clientMsgId: String = "s$serverId",
    ) {
        val first = attachments.first()
        // The old encoder knew no `animation`, so the stored set never carries one.
        val json = AttachmentsCodec.encode(attachments.map { it.copy(animation = null) }).replace("'", "''")
        db.openHelper.writableDatabase.execSQL(
            """
            INSERT INTO messages (clientMsgId, serverId, chatId, senderId, body, createdAt, status,
                attachmentId, attachmentKind, attachmentMime, attachmentSize, attachmentWidth,
                attachmentHeight, attachmentDurationMs, attachmentHasPreview, attachmentsJson,
                attachmentsKnowRound)
            VALUES ('$clientMsgId', ${serverId ?: "NULL"}, $CHAT, $PEER, '$body', ${serverId ?: 0} * 1000,
                'SENT', ${first.id}, '${first.kind}', '${first.mime}', ${first.size}, ${first.width},
                ${first.height}, ${first.durationMs ?: "NULL"}, ${if (first.hasPreview) 1 else 0}, '$json', 1)
            """.trimIndent(),
        )
    }

    /** The server's copy of an animation: the same video, with the flag and its caption. */
    private fun serverHasAnimation(id: Long, body: String = "") {
        server[id] = ApiResult.Ok(
            MessagesResponse(
                listOf(messageDto(id = id, chatId = CHAT, senderId = PEER, body = body, attachments = listOf(video(id, animation = true)))),
            ),
        )
    }

    private suspend fun isAnimation(serverId: Long) = db.messageDao().findByServerId(serverId)!!.attachmentList.single().isAnimation

    @Test
    fun anAnimationCachedBeforeTheUpgradeMovesAfterTheNextResync() = runTest(dispatcher) {
        cachedByOldBuild(50, listOf(video(50)))
        serverHasAnimation(50)
        assertThat(isAnimation(50)).isFalse()

        newEngine().resync()

        assertThat(asked).containsExactly(50L)
        assertThat(isAnimation(50)).isTrue()
        assertThat(db.chatDao().getById(CHAT)!!.lastMessageBody).isEqualTo("GIF")
    }

    @Test
    fun aCaptionedOneIsACandidateTooAndEachRowIsAskedOnce() = runTest(dispatcher) {
        // An animation MAY carry a caption, unlike a circle: the body is not asked.
        cachedByOldBuild(40, listOf(video(40)), body = "lol")
        serverHasAnimation(40, body = "lol")
        // An ordinary video after all, a photo, two videos, an unsent row.
        cachedByOldBuild(30, listOf(video(30)))
        server[30] = ApiResult.Ok(
            MessagesResponse(listOf(messageDto(id = 30, chatId = CHAT, senderId = PEER, body = "", attachments = listOf(video(30))))),
        )
        cachedByOldBuild(45, listOf(photo(45)))
        cachedByOldBuild(35, listOf(video(35), video(36)))
        cachedByOldBuild(null, listOf(video(60)), clientMsgId = "local-unsent")
        val engine = newEngine()

        engine.resync()

        assertThat(asked).containsExactly(40L, 30L).inOrder()
        assertThat(isAnimation(40)).isTrue()
        assertThat(isAnimation(30)).isFalse()

        // Settled: a second resync asks nothing again.
        asked.clear()
        engine.resync()
        assertThat(asked).isEmpty()
    }
}
