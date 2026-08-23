/*
 * ChatViewModelTest.kt
 * Family Connect (Android)
 *
 * Three behaviors: the pure grouping rules (sender names, timestamps,
 * date separators — buildChatItems directly), the loadOlder guard (one
 * in-flight fetch, none past the start of history), and the read-marker
 * debounce (many inbound messages → one `read`).
 */

package me.nettrash.familyconnect.ui.chat

import androidx.lifecycle.SavedStateHandle
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.StandardTestDispatcher
import me.nettrash.familyconnect.data.net.LinkPreviewRepository
import okhttp3.OkHttpClient
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import java.io.File
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.flow.first
import androidx.compose.foundation.text.input.setTextAndPlaceCursorAtEnd
import kotlinx.coroutines.test.setMain
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.db.ChatEntity
import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.data.db.MessageStatus
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.MessagesResponse
import me.nettrash.familyconnect.data.net.dto.ReactionDto
import me.nettrash.familyconnect.data.net.dto.ReactionsCodec
import me.nettrash.familyconnect.data.net.ws.ClientFrame
import me.nettrash.familyconnect.data.repo.ChatRepository
import me.nettrash.familyconnect.data.repo.FamilyStatus
import me.nettrash.familyconnect.data.repo.AttachmentRepository
import me.nettrash.familyconnect.data.repo.GallerySaver
import me.nettrash.familyconnect.data.repo.MediaPrep
import me.nettrash.familyconnect.data.repo.VoiceRecorder
import me.nettrash.familyconnect.data.repo.MessageRepository
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.testutil.FakeAttachmentApi
import me.nettrash.familyconnect.testutil.FakeChatApi
import me.nettrash.familyconnect.testutil.FakeChatSocket
import me.nettrash.familyconnect.testutil.FakeConnectivityObserver
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.createTestDb
import me.nettrash.familyconnect.testutil.messageDto
import me.nettrash.familyconnect.util.Clock
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import java.time.ZoneOffset

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class ChatViewModelTest {

    private companion object {
        const val ME = 7L
        const val PEER = 9L
        const val CHAT = 42L
        val ZONE: ZoneOffset = ZoneOffset.UTC

        // 2026-08-19 12:00:00 UTC
        const val NOON = 1_786_795_200_000L
        const val MINUTE = 60_000L
        const val DAY = 86_400_000L
    }

    private val dispatcher = StandardTestDispatcher()

    // Foreground scope for repositories + item subscriptions — see
    // MessageRepositoryTest for why backgroundScope won't do.
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private lateinit var db: AppDatabase
    private lateinit var chatApi: FakeChatApi
    private val attachmentApi = FakeAttachmentApi()
    private lateinit var socket: FakeChatSocket
    private lateinit var settings: FakeSettingsRepository
    private lateinit var chatRepository: ChatRepository
    private lateinit var messageRepository: MessageRepository

    @Before
    fun setUp() {
        Dispatchers.setMain(dispatcher)
        db = createTestDb(dispatcher)
        chatApi = FakeChatApi()
        socket = FakeChatSocket()
        settings = FakeSettingsRepository(
            SettingsState(
                serverUrl = "https://chat.example.com",
                familyStatus = FamilyStatus.MEMBER,
                myUserId = ME,
            ),
        )
    }

    @After
    fun tearDown() {
        repoScope.cancel()
        Dispatchers.resetMain()
        db.close()
    }

    private fun TestScope.newViewModel(kind: String = "direct"): ChatViewModel {
        chatRepository = ChatRepository(chatApi, db.chatDao(), socket)
        messageRepository = MessageRepository(
            chatApi = chatApi,
            attachmentApi = attachmentApi,
            messageDao = db.messageDao(),
            chatDao = db.chatDao(),
            socket = socket,
            settings = settings,
            chatRepository = chatRepository,
            scope = repoScope,
            clock = Clock { NOON },
        )
        runCurrent()
        // Chat row must exist for read reporting / unread rules.
        launch {
            db.chatDao().upsertAll(
                listOf(
                    ChatEntity(
                        id = CHAT,
                        kind = kind,
                        peerUserId = if (kind == "direct") PEER else null,
                        title = "Chat",
                        unreadCount = 0,
                        myLastReadId = null,
                        peerLastReadId = null,
                        lastMessageBody = null,
                        lastMessageAt = null,
                        lastMessageSenderId = null,
                    ),
                ),
            )
        }
        runCurrent()
        return ChatViewModel(
            appContext = RuntimeEnvironment.getApplication(),
            savedStateHandle = SavedStateHandle(mapOf("chatId" to CHAT)),
            messageRepository = messageRepository,
            chatRepository = chatRepository,
            settings = settings,
            socket = socket,
            clock = Clock { NOON },
            memberDao = db.memberDao(),
            connectivity = FakeConnectivityObserver(),
            // Real instance over a client that is never called: these
            // tests never render a bubble, so nothing requests a preview.
            linkPreviewRepository = LinkPreviewRepository(
                okHttp = OkHttpClient(),
                scope = CoroutineScope(StandardTestDispatcher(testScheduler)),
            ),
            // Real MediaPrep over Robolectric's ContentResolver: these
            // tests never pick media, so nothing here is exercised.
            mediaPrep = MediaPrep(
                context = RuntimeEnvironment.getApplication(),
                contentResolver = RuntimeEnvironment.getApplication().contentResolver,
            ),
            // Real recorder, never started: these tests do not record.
            voiceRecorder = VoiceRecorder(RuntimeEnvironment.getApplication()),
            attachmentApi = attachmentApi,
            gallerySaver = GallerySaver(),
            // The repo scope stands in for the app scope: a media send
            // must outlive the ViewModel, which is the whole point of it.
            appScope = repoScope,
            attachments = AttachmentRepository(
                context = RuntimeEnvironment.getApplication(),
                attachmentApi = attachmentApi,
                settings = settings,
                connectivity = FakeConnectivityObserver(),
                scope = repoScope,
            ),
        )
    }

    private fun entity(
        clientMsgId: String,
        serverId: Long?,
        senderId: Long,
        createdAt: Long,
        reactionsJson: String? = null,
    ) = MessageEntity(
        clientMsgId = clientMsgId,
        serverId = serverId,
        chatId = CHAT,
        senderId = senderId,
        body = "b",
        createdAt = createdAt,
        status = MessageStatus.SENT,
        reactionsJson = reactionsJson,
    )

    // -- Grouping (pure) ------------------------------------------------------

    @Test
    fun senderNameShowsOnlyInFamilyChatOnRunStarts() {
        // Newest-first: two consecutive PEER messages then one of mine.
        val messages = listOf(
            entity("s3", 3, PEER, NOON + 2 * MINUTE),
            entity("s2", 2, PEER, NOON + 1 * MINUTE),
            entity("s1", 1, ME, NOON),
        )
        val names = mapOf(PEER to "Ben", ME to "Anna")

        val familyItems = buildChatItems(messages, isFamilyChat = true, myUserId = ME, memberNames = names, nowMillis = NOON, zone = ZONE)
            .filterIsInstance<ChatListItem.MessageItem>()
        // s2 starts the PEER run (older neighbor is mine) → name; s3
        // continues it → no name; my own s1 → never a name.
        assertThat(familyItems.map { it.entity.clientMsgId to it.showSenderName })
            .containsExactly("s3" to false, "s2" to true, "s1" to false)
            .inOrder()
        assertThat(familyItems[1].senderName).isEqualTo("Ben")

        val directItems = buildChatItems(messages, isFamilyChat = false, myUserId = ME, memberNames = names, nowMillis = NOON, zone = ZONE)
            .filterIsInstance<ChatListItem.MessageItem>()
        assertThat(directItems.none { it.showSenderName }).isTrue()
    }

    @Test
    fun timestampShowsOnTheLastMessageOfASameMinuteRun() {
        // Newest-first: s3 (12:01), s2 (12:00), s1 (12:00) — all PEER.
        val messages = listOf(
            entity("s3", 3, PEER, NOON + MINUTE),
            entity("s2", 2, PEER, NOON + 10_000),
            entity("s1", 1, PEER, NOON),
        )
        val items = buildChatItems(messages, isFamilyChat = false, myUserId = ME, memberNames = emptyMap(), nowMillis = NOON, zone = ZONE)
            .filterIsInstance<ChatListItem.MessageItem>()

        // s3 is newest (nothing newer) → timestamp; s2 ends the 12:00
        // run visually (newer s3 is a different minute) → timestamp;
        // s1 has s2 in the same minute above it → none.
        assertThat(items.map { it.entity.clientMsgId to it.showTimestamp })
            .containsExactly("s3" to true, "s2" to true, "s1" to false)
            .inOrder()
    }

    @Test
    fun dateSeparatorsAppearAtDayBoundariesAndForTheOldestDay() {
        val messages = listOf(
            entity("s3", 3, PEER, NOON), // today
            entity("s2", 2, PEER, NOON - DAY), // yesterday
            entity("s1", 1, PEER, NOON - DAY - MINUTE), // yesterday too
        )
        val items = buildChatItems(messages, isFamilyChat = false, myUserId = ME, memberNames = emptyMap(), nowMillis = NOON, zone = ZONE)

        val kinds = items.map {
            when (it) {
                is ChatListItem.MessageItem -> it.entity.clientMsgId
                is ChatListItem.DateSeparator -> "sep:${it.label}"
            }
        }
        // reverseLayout renders list order bottom-up, so each day's pill
        // trails its messages in list order (= appears above on screen).
        assertThat(kinds).containsExactly(
            "s3", "sep:Today", "s2", "s1", "sep:Yesterday",
        ).inOrder()
    }

    @Test
    fun pendingMessagesKeepTheirClientKeyInTheItemList() {
        val messages = listOf(
            entity("local-uuid", null, ME, NOON),
            entity("s1", 1, ME, NOON - MINUTE),
        )
        val items = buildChatItems(messages, isFamilyChat = false, myUserId = ME, memberNames = emptyMap(), nowMillis = NOON, zone = ZONE)
            .filterIsInstance<ChatListItem.MessageItem>()
        assertThat(items.first().key).isEqualTo("local-uuid")
    }

    // -- Reaction chips (pure) ------------------------------------------------

    @Test
    fun reactionChipsAggregateInFirstSeenOrderWithCountsAndIncludesMe() {
        val chips = buildReactionChips(
            reactions = listOf(
                ReactionDto(11L, "❤️"),
                ReactionDto(12L, "👍"),
                ReactionDto(13L, "❤️"),
                ReactionDto(ME, "👍"),
            ),
            myUserId = ME,
        )

        // One chip per emoji, ordered by FIRST appearance — piling onto
        // an existing emoji must not reorder the row.
        assertThat(chips).containsExactly(
            ReactionChip(emoji = "❤️", count = 2, includesMe = false),
            ReactionChip(emoji = "👍", count = 2, includesMe = true),
        ).inOrder()
    }

    @Test
    fun reactionChipsAreEmptyForNoReactions() {
        assertThat(buildReactionChips(emptyList(), ME)).isEmpty()
    }

    @Test
    fun buildChatItemsThreadsChipsAndMyReactionFromTheRowJson() {
        val messages = listOf(
            entity(
                "s2",
                2,
                PEER,
                NOON,
                reactionsJson = ReactionsCodec.encode(
                    listOf(ReactionDto(PEER, "😂"), ReactionDto(ME, "❤️")),
                ),
            ),
            entity("s1", 1, PEER, NOON - MINUTE), // never reacted
        )
        val items = buildChatItems(messages, isFamilyChat = false, myUserId = ME, memberNames = emptyMap(), nowMillis = NOON, zone = ZONE)
            .filterIsInstance<ChatListItem.MessageItem>()

        assertThat(items[0].reactionChips).containsExactly(
            ReactionChip(emoji = "😂", count = 1, includesMe = false),
            ReactionChip(emoji = "❤️", count = 1, includesMe = true),
        ).inOrder()
        assertThat(items[0].myReaction).isEqualTo("❤️")
        assertThat(items[1].reactionChips).isEmpty()
        assertThat(items[1].myReaction).isNull()
    }

    @Test
    fun malformedReactionsJsonYieldsNoChipsInsteadOfCrashing() {
        val items = buildChatItems(
            listOf(entity("s1", 1, PEER, NOON, reactionsJson = "{not json")),
            isFamilyChat = false,
            myUserId = ME,
            memberNames = emptyMap(),
            nowMillis = NOON,
            zone = ZONE,
        ).filterIsInstance<ChatListItem.MessageItem>()

        assertThat(items.single().reactionChips).isEmpty()
    }

    // -- Reaction details (pure) ----------------------------------------------

    @Test
    fun reactionDetailsGroupPerEmojiInFirstSeenOrderWithNamesInReactionOrder() {
        val details = buildReactionDetails(
            reactions = listOf(
                ReactionDto(11L, "❤️"),
                ReactionDto(12L, "👍"),
                ReactionDto(13L, "❤️"),
            ),
            names = mapOf(11L to "Anna", 12L to "Ben", 13L to "Cleo"),
            myUserId = ME,
        )

        // Same emoji order as the chips (first seen), names in reaction
        // order within each emoji.
        assertThat(details).containsExactly(
            ReactionDetail(emoji = "❤️", names = listOf("Anna", "Cleo")),
            ReactionDetail(emoji = "👍", names = listOf("Ben")),
        ).inOrder()
    }

    @Test
    fun reactionDetailsShowMeAsYouListedFirstWithinMyEmoji() {
        val details = buildReactionDetails(
            reactions = listOf(
                ReactionDto(11L, "😂"),
                ReactionDto(12L, "😂"),
                ReactionDto(ME, "😂"), // I reacted LAST — still listed first
            ),
            names = mapOf(11L to "Anna", 12L to "Ben", ME to "Me Myself"),
            myUserId = ME,
        )

        // "You" replaces my roster name and leads its group; the others
        // keep their order.
        assertThat(details).containsExactly(
            ReactionDetail(emoji = "😂", names = listOf("You", "Anna", "Ben")),
        )
    }

    @Test
    fun reactionDetailsAcrossSeveralEmojisKeepYouOnlyInMyGroup() {
        val details = buildReactionDetails(
            reactions = listOf(
                ReactionDto(11L, "❤️"),
                ReactionDto(ME, "👍"),
                ReactionDto(12L, "❤️"),
                ReactionDto(13L, "👍"),
                ReactionDto(14L, "😮"),
            ),
            names = mapOf(11L to "Anna", 12L to "Ben", 13L to "Cleo", 14L to "Dan"),
            myUserId = ME,
        )

        assertThat(details).containsExactly(
            ReactionDetail(emoji = "❤️", names = listOf("Anna", "Ben")),
            ReactionDetail(emoji = "👍", names = listOf("You", "Cleo")),
            ReactionDetail(emoji = "😮", names = listOf("Dan")),
        ).inOrder()
    }

    @Test
    fun reactionDetailsFallBackToMemberIdForUnknownReactors() {
        val details = buildReactionDetails(
            reactions = listOf(ReactionDto(99L, "❤️")),
            names = emptyMap(), // roster does not know user 99
            myUserId = ME,
        )

        // Same fallback the sender-name label uses.
        assertThat(details).containsExactly(
            ReactionDetail(emoji = "❤️", names = listOf("Member 99")),
        )
    }

    @Test
    fun reactionDetailsAreEmptyForNoReactions() {
        assertThat(buildReactionDetails(emptyList(), emptyMap(), ME)).isEmpty()
    }

    // -- loadOlder guard ------------------------------------------------------------

    /**
     * The composer stages an attachment and Send commits it WITH whatever
     * was typed.
     *
     * This shipped broken: staging worked and the chip appeared, but
     * `send()` had no staged branch at all, so pressing Send posted the
     * caption as an ordinary text message and silently dropped the
     * attachment. Nothing caught it because nothing exercised the two
     * together.
     */
    @Test
    fun sendCommitsStagedMediaWithTheTypedCaption() = runTest(dispatcher) {
        val viewModel = newViewModel()
        val file = File.createTempFile("staged", ".jpg").apply { writeBytes(ByteArray(16) { 1 }) }
        viewModel.stagePrepared(
            MediaPrep.Prepared(
                file = file,
                mime = "image/jpeg",
                kind = AttachmentDto.KIND_PHOTO,
                width = 100,
                height = 80,
                durationMs = null,
                previewJpeg = null,
            ),
        )
        runCurrent()
        assertThat(viewModel.staged.value).isNotNull()

        viewModel.inputState.setTextAndPlaceCursorAtEnd("look at this")
        viewModel.send()
        advanceUntilIdle()

        assertThat(attachmentApi.calls).contains("upload")
        val stored = db.messageDao().observeMessages(CHAT, 50).first()
        val row = stored.firstOrNull { it.attachmentId != null }
        assertThat(row).isNotNull()
        assertThat(row!!.body).isEqualTo("look at this")
        // Composer emptied, staging consumed.
        assertThat(viewModel.staged.value).isNull()
        assertThat(viewModel.inputState.text.toString()).isEmpty()
    }

    /** A photo needs no caption — Send must be live on the attachment alone. */
    @Test
    fun sendCommitsStagedMediaWithNoCaption() = runTest(dispatcher) {
        val viewModel = newViewModel()
        val file = File.createTempFile("staged", ".jpg").apply { writeBytes(ByteArray(16) { 2 }) }
        viewModel.stagePrepared(
            MediaPrep.Prepared(
                file = file,
                mime = "image/jpeg",
                kind = AttachmentDto.KIND_PHOTO,
                width = 100,
                height = 80,
                durationMs = null,
                previewJpeg = null,
            ),
        )
        runCurrent()

        viewModel.send()
        advanceUntilIdle()

        assertThat(attachmentApi.calls).contains("upload")
        assertThat(viewModel.staged.value).isNull()
    }

    @Test
    fun loadOlderRunsOneFetchAtATime() = runTest(dispatcher) {
        val viewModel = newViewModel()
        chatApi.messagesHandler = { _, _, _, _ ->
            ApiResult.Ok(MessagesResponse((1L..50L).map { messageDto(id = it, chatId = CHAT, senderId = PEER) }))
        }

        viewModel.loadOlder()
        viewModel.loadOlder() // burst — must be swallowed by the guard
        viewModel.loadOlder()
        runCurrent()

        assertThat(chatApi.messagesCalls).isEqualTo(1)
    }

    @Test
    fun loadOlderStopsForGoodAfterReachingTheStart() = runTest(dispatcher) {
        val viewModel = newViewModel()
        chatApi.messagesHandler = { _, _, _, _ ->
            // Short page = start of history.
            ApiResult.Ok(MessagesResponse(listOf(messageDto(id = 1, chatId = CHAT, senderId = PEER))))
        }

        viewModel.loadOlder()
        runCurrent()
        viewModel.loadOlder()
        viewModel.loadOlder()
        runCurrent()

        assertThat(chatApi.messagesCalls).isEqualTo(1)
    }

    // -- Read-marker debounce -----------------------------------------------------------

    @Test
    fun rapidInboundMessagesProduceOneDebouncedReadReport() = runTest(dispatcher) {
        val viewModel = newViewModel()
        val itemsSubscription = repoScope.launch { viewModel.items.collect {} }
        viewModel.setResumed(true)
        runCurrent()

        messageRepository.applyServerMessage(messageDto(id = 10, chatId = CHAT, senderId = PEER), live = false)
        runCurrent()
        advanceTimeBy(300) // inside the debounce window
        messageRepository.applyServerMessage(messageDto(id = 11, chatId = CHAT, senderId = PEER), live = false)
        runCurrent()

        assertThat(chatApi.postedReads).isEmpty() // still debouncing

        advanceTimeBy(600)
        runCurrent()

        // One report, carrying the NEWEST id.
        assertThat(chatApi.postedReads).containsExactly(CHAT to 11L)
        assertThat(db.chatDao().getById(CHAT)!!.myLastReadId).isEqualTo(11L)
        itemsSubscription.cancel()
    }

    @Test
    fun noReadReportWhileNotResumed() = runTest(dispatcher) {
        val viewModel = newViewModel()
        val itemsSubscription = repoScope.launch { viewModel.items.collect {} }
        viewModel.setResumed(false)
        runCurrent()

        messageRepository.applyServerMessage(messageDto(id = 10, chatId = CHAT, senderId = PEER), live = false)
        runCurrent()
        advanceTimeBy(2_000)
        runCurrent()

        assertThat(chatApi.postedReads).isEmpty()
        itemsSubscription.cancel()
    }

    @Test
    fun readGoesOverTheSocketWhenOpen() = runTest(dispatcher) {
        val viewModel = newViewModel()
        val itemsSubscription = repoScope.launch { viewModel.items.collect {} }
        socket.setOpen(true)
        viewModel.setResumed(true)
        runCurrent()

        messageRepository.applyServerMessage(messageDto(id = 20, chatId = CHAT, senderId = PEER), live = false)
        runCurrent()
        advanceTimeBy(600)
        runCurrent()

        assertThat(socket.sent.filterIsInstance<ClientFrame.Read>())
            .containsExactly(ClientFrame.Read(chatId = CHAT, lastReadMessageId = 20L))
        assertThat(chatApi.postedReads).isEmpty() // no REST fallback needed
        itemsSubscription.cancel()
    }
}
