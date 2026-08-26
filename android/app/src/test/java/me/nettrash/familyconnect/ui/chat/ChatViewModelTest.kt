/*
 * ChatViewModelTest.kt
 * Family Connect (Android)
 *
 * Three behaviors: the pure grouping rules (sender names, timestamps,
 * date separators — buildChatItems directly), the loadOlder guard (one
 * in-flight fetch, none past the start of history), and the read-marker
 * rule — read means SEEN, so a report needs the screen RESUMED *and*
 * the list at the newest message, debounced (many inbound messages →
 * one `read`). The scroll half is the load-bearing one: the server's
 * marker is monotonic, so a read posted for a message nobody looked at
 * cannot be taken back on any of that person's devices.
 */

package me.nettrash.familyconnect.ui.chat

import android.app.NotificationManager
import android.content.ClipData
import android.net.Uri
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
import kotlinx.coroutines.flow.filterNotNull
import androidx.compose.foundation.text.input.setTextAndPlaceCursorAtEnd
import kotlinx.coroutines.test.setMain
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.db.ChatEntity
import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.data.db.MessageStatus
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.MessageResponse
import me.nettrash.familyconnect.data.net.dto.MessagesResponse
import me.nettrash.familyconnect.data.net.dto.PollCodec
import me.nettrash.familyconnect.data.net.dto.ReactionDto
import me.nettrash.familyconnect.data.net.dto.ReplyToDto
import me.nettrash.familyconnect.data.net.dto.ReactionsCodec
import me.nettrash.familyconnect.data.net.ws.ClientFrame
import me.nettrash.familyconnect.data.push.PushNotifications
import me.nettrash.familyconnect.data.repo.ChatRepository
import me.nettrash.familyconnect.data.repo.FamilyStatus
import me.nettrash.familyconnect.data.repo.AttachmentRepository
import me.nettrash.familyconnect.data.repo.GallerySaver
import me.nettrash.familyconnect.data.repo.LocationProvider
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
import me.nettrash.familyconnect.testutil.pollDto
import me.nettrash.familyconnect.testutil.pollState
import me.nettrash.familyconnect.util.Clock
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
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
        chatRepository = ChatRepository(chatApi, db.chatDao(), db.messageDao(), socket, repoScope)
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
            // Real provider, never asked: these tests hold no location
            // permission, so `hasPermission()` is false and nothing runs.
            locationProvider = LocationProvider(RuntimeEnvironment.getApplication()),
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

    // -- Read markers: resumed AND at the newest message ---------------------------------

    @Test
    fun rapidInboundMessagesProduceOneDebouncedReadReport() = runTest(dispatcher) {
        val viewModel = newViewModel()
        val itemsSubscription = repoScope.launch { viewModel.items.collect {} }
        viewModel.setResumed(true)
        viewModel.setAtNewest(true)
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
        viewModel.setAtNewest(true)
        runCurrent()

        messageRepository.applyServerMessage(messageDto(id = 10, chatId = CHAT, senderId = PEER), live = false)
        runCurrent()
        advanceTimeBy(2_000)
        runCurrent()

        assertThat(chatApi.postedReads).isEmpty()
        itemsSubscription.cancel()
    }

    /**
     * The one the user reported: the chat is open, the app is in front,
     * and the reader is thirty messages up the thread reading something
     * else. Nothing down at the bottom has been seen, and the server's
     * marker only ever moves forward — so nothing may be reported.
     */
    @Test
    fun noReadReportWhileScrolledAwayFromTheNewestMessage() = runTest(dispatcher) {
        val viewModel = newViewModel()
        val itemsSubscription = repoScope.launch { viewModel.items.collect {} }
        viewModel.setResumed(true)
        viewModel.setAtNewest(false)
        runCurrent()

        messageRepository.applyServerMessage(messageDto(id = 30, chatId = CHAT, senderId = PEER), live = false)
        runCurrent()
        advanceTimeBy(2_000)
        runCurrent()

        assertThat(chatApi.postedReads).isEmpty()
        assertThat(socket.sent.filterIsInstance<ClientFrame.Read>()).isEmpty()
        assertThat(db.chatDao().getById(CHAT)!!.myLastReadId).isNull()
        itemsSubscription.cancel()
    }

    @Test
    fun scrollingBackToTheNewestMessageReportsTheRead() = runTest(dispatcher) {
        val viewModel = newViewModel()
        val itemsSubscription = repoScope.launch { viewModel.items.collect {} }
        viewModel.setResumed(true)
        viewModel.setAtNewest(false)
        runCurrent()

        messageRepository.applyServerMessage(messageDto(id = 31, chatId = CHAT, senderId = PEER), live = false)
        runCurrent()
        advanceTimeBy(2_000)
        runCurrent()
        assertThat(chatApi.postedReads).isEmpty()

        // Arriving at the bottom is the moment it is seen.
        viewModel.setAtNewest(true)
        runCurrent()
        advanceTimeBy(600)
        runCurrent()

        assertThat(chatApi.postedReads).containsExactly(CHAT to 31L)
        itemsSubscription.cancel()
    }

    /**
     * Backgrounding revokes the authority to read, and coming back does
     * not hand it out again by itself: everything that arrived while the
     * app was away is unread until the reader is looking at it.
     */
    @Test
    fun backgroundingAndReturningScrolledAwayReadsNothing() = runTest(dispatcher) {
        val viewModel = newViewModel()
        val itemsSubscription = repoScope.launch { viewModel.items.collect {} }
        viewModel.setResumed(true)
        viewModel.setAtNewest(false)
        runCurrent()

        viewModel.setResumed(false)
        runCurrent()
        messageRepository.applyServerMessage(messageDto(id = 32, chatId = CHAT, senderId = PEER), live = true)
        runCurrent()

        viewModel.setResumed(true)
        runCurrent()
        advanceTimeBy(2_000)
        runCurrent()

        assertThat(chatApi.postedReads).isEmpty()
        // Arrived while backgrounded, so it counts.
        assertThat(db.chatDao().getById(CHAT)!!.unreadCount).isEqualTo(1)
        itemsSubscription.cancel()
    }

    @Test
    fun readGoesOverTheSocketWhenOpen() = runTest(dispatcher) {
        val viewModel = newViewModel()
        val itemsSubscription = repoScope.launch { viewModel.items.collect {} }
        socket.setOpen(true)
        viewModel.setResumed(true)
        viewModel.setAtNewest(true)
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

    /**
     * D10: on Android the tray entry IS the launcher dot, so reading the
     * chat has to take it down — otherwise the dot advertises messages
     * the user is looking at.
     */
    @Test
    fun readingTheChatDismissesItsTrayNotification() = runTest(dispatcher) {
        val context = RuntimeEnvironment.getApplication()
        val manager = context.getSystemService(NotificationManager::class.java)
        PushNotifications.ensureChannel(context)
        // Straight at the manager rather than through show(), which is
        // gated on a runtime permission this test process does not hold —
        // the (tag, id) slot is the same one show() posts into.
        manager.notify(
            PushNotifications.chatTag(CHAT),
            PushNotifications.NOTIFICATION_ID,
            PushNotifications.build(context, "Ben", "Dinner at 7?", kind = "message", chatId = CHAT),
        )
        assertThat(manager.activeNotifications).isNotEmpty()

        val viewModel = newViewModel()
        val itemsSubscription = repoScope.launch { viewModel.items.collect {} }
        viewModel.setResumed(true)
        viewModel.setAtNewest(true)
        runCurrent()

        messageRepository.applyServerMessage(messageDto(id = 40, chatId = CHAT, senderId = PEER), live = false)
        runCurrent()
        advanceTimeBy(600)
        runCurrent()

        assertThat(manager.activeNotifications).isEmpty()
        itemsSubscription.cancel()
    }

    // -- Pasting --------------------------------------------------------------
    //
    // A pasted item takes the same road a picked one does: prepare, stage,
    // and wait for Send. What is new is that nobody chose it from a picker,
    // so the kind, the media type and the NAME all have to be worked out
    // from what the clipboard says — and a clipboard also holds things that
    // are not attachments at all.

    /**
     * Wait for the staged item, rather than for the virtual clock.
     *
     * MediaPrep copies and decodes on `Dispatchers.IO` — a real thread the
     * test scheduler does not own — so `advanceUntilIdle()` returns while
     * the prepare is still running. runTest keeps pumping the scheduler
     * while the body is suspended, so awaiting the value is both correct
     * and deterministic.
     */
    private suspend fun ChatViewModel.awaitStaged(
        predicate: (MediaPrep.Prepared) -> Boolean = { true },
    ): MediaPrep.Prepared = staged.filterNotNull().first(predicate)

    /** The same wait, for the paths that end in the composer's error strip. */
    private suspend fun ChatViewModel.awaitFailure(): ChatViewModel.MediaSendState.Failed =
        mediaState.first { it is ChatViewModel.MediaSendState.Failed }
            as ChatViewModel.MediaSendState.Failed

    /** A real 1x1 PNG: the decoder here is the platform's, not a fake. */
    private val ONE_PIXEL_PNG: ByteArray = android.util.Base64.decode(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
        android.util.Base64.DEFAULT,
    )

    /** Put bytes behind a content Uri, the way a provider would. */
    private fun clipboardItem(name: String, bytes: ByteArray = ByteArray(32) { 7 }): Uri {
        val uri = Uri.parse("content://me.nettrash.test/$name")
        shadowOf(RuntimeEnvironment.getApplication().contentResolver)
            .registerInputStreamSupplier(uri) { bytes.inputStream() }
        return uri
    }

    /**
     * The rule an animated GIF depends on: re-encoding it as a photo would
     * turn it into one still frame, and the server refuses `image/gif` as a
     * photo anyway. It goes as a file, bytes untouched.
     */
    @Test
    fun pastedGifIsStagedAsAFileWithAName() = runTest(dispatcher) {
        val viewModel = newViewModel()
        val uri = clipboardItem("opaque-id-1000000042")

        val result = viewModel.pasteAttachment(uri, "image/gif")
        val staged = viewModel.awaitStaged()

        assertThat(result).isEqualTo(ChatViewModel.PasteResult.STAGING)
        assertThat(staged.kind).isEqualTo(AttachmentDto.KIND_FILE)
        assertThat(staged.mime).isEqualTo("image/gif")
        // NOT the Uri's opaque tail, and not the cache file's name either.
        assertThat(staged.name).isEqualTo("Pasted image.gif")
        assertThat(staged.file.name).doesNotContain("Pasted")
        assertThat(staged.file.readBytes()).hasLength(32)
        assertThat(viewModel.mediaState.value).isEqualTo(ChatViewModel.MediaSendState.Idle)
    }

    /**
     * The ordinary case: a copied photo. It takes the picker's own
     * preparation — re-encoded to the one type the server magic-checks,
     * with the thumbnail the bubble draws — and needs no name, because a
     * photo's is ignored on the wire (docs/protocol.md, "Files").
     */
    @Test
    fun aPastedPhotoIsPreparedLikeAPickedOne() = runTest(dispatcher) {
        val viewModel = newViewModel()

        viewModel.pasteAttachment(clipboardItem("blob", ONE_PIXEL_PNG), "image/png")

        val staged = viewModel.awaitStaged()
        assertThat(staged.kind).isEqualTo(AttachmentDto.KIND_PHOTO)
        assertThat(staged.mime).isEqualTo("image/jpeg")
        assertThat(staged.name).isNull()
        assertThat(staged.previewJpeg).isNotNull()
    }

    /** `kind=file` is refused outright without a name of 1–255 characters. */
    @Test
    fun aPastedDocumentIsNamedAfterItsType() = runTest(dispatcher) {
        val viewModel = newViewModel()

        viewModel.pasteAttachment(clipboardItem("blob"), "application/pdf")

        val staged = viewModel.awaitStaged()
        assertThat(staged.name).isEqualTo("Pasted file.pdf")
        assertThat(staged.mime).isEqualTo("application/pdf")
    }

    /** Audio the server can magic-check keeps its player, and its name. */
    @Test
    fun pastedAudioIsStagedAsAudio() = runTest(dispatcher) {
        val viewModel = newViewModel()

        viewModel.pasteAttachment(clipboardItem("blob"), "audio/mpeg")

        val staged = viewModel.awaitStaged()
        assertThat(staged.kind).isEqualTo(AttachmentDto.KIND_AUDIO)
        assertThat(staged.mime).isEqualTo("audio/mpeg")
        assertThat(staged.name).isEqualTo("Pasted sound.mp3")
    }

    /**
     * A copied link is a Uri too. Attaching one would mean downloading
     * somebody's web page — the address belongs in the composer.
     */
    @Test
    fun aCopiedLinkIsNotAnAttachment() = runTest(dispatcher) {
        val viewModel = newViewModel()

        val result = viewModel.pasteAttachment(
            Uri.parse("https://example.com/holiday.jpg"),
            "image/jpeg",
        )
        advanceUntilIdle()

        assertThat(result).isEqualTo(ChatViewModel.PasteResult.NOTHING)
        assertThat(viewModel.staged.value).isNull()
        assertThat(viewModel.mediaState.value).isEqualTo(ChatViewModel.MediaSendState.Idle)
    }

    /**
     * One attachment per message: a paste goes through the same staging
     * the picker does, so it REPLACES what was staged (and deletes the
     * file nothing else would clean up) rather than queueing behind it.
     */
    @Test
    fun aPasteReplacesWhateverWasAlreadyStaged() = runTest(dispatcher) {
        val viewModel = newViewModel()
        val first = File.createTempFile("staged", ".jpg").apply { writeBytes(ByteArray(16) { 3 }) }
        viewModel.stagePrepared(
            MediaPrep.Prepared(
                file = first,
                mime = "image/jpeg",
                kind = AttachmentDto.KIND_PHOTO,
                width = 10,
                height = 10,
                durationMs = null,
                previewJpeg = null,
            ),
        )
        runCurrent()

        viewModel.pasteAttachment(clipboardItem("blob"), "application/pdf")

        val staged = viewModel.awaitStaged { it.mime == "application/pdf" }
        assertThat(staged.kind).isEqualTo(AttachmentDto.KIND_FILE)
        assertThat(first.exists()).isFalse()
    }

    /**
     * The guard the attach menu carries, repeated for the door that is not
     * behind it: the composer is borrowed for an edit, which has no second
     * attachment to add.
     */
    @Test
    fun aPasteIsRefusedWhileEditingAMessage() = runTest(dispatcher) {
        val viewModel = newViewModel()
        viewModel.beginEdit(messageId = 5, body = "old text")
        runCurrent()

        val result = viewModel.pasteAttachment(clipboardItem("blob"), "application/pdf")
        advanceUntilIdle()

        assertThat(result).isEqualTo(ChatViewModel.PasteResult.BUSY)
        assertThat(viewModel.staged.value).isNull()
    }

    /** And the other half of that guard: one upload at a time. */
    @Test
    fun aPasteIsRefusedWhileAnUploadIsRunning() = runTest(dispatcher) {
        val viewModel = newViewModel()
        val file = File.createTempFile("staged", ".jpg").apply { writeBytes(ByteArray(16) { 4 }) }
        viewModel.stagePrepared(
            MediaPrep.Prepared(
                file = file,
                mime = "image/jpeg",
                kind = AttachmentDto.KIND_PHOTO,
                width = 10,
                height = 10,
                durationMs = null,
                previewJpeg = null,
            ),
        )
        runCurrent()
        // send() marks the composer Uploading before it launches anything,
        // so nothing has to be advanced to be in flight.
        viewModel.send()

        val result = viewModel.pasteAttachment(clipboardItem("blob"), "application/pdf")

        assertThat(result).isEqualTo(ChatViewModel.PasteResult.BUSY)
        advanceUntilIdle()
    }

    // -- The attach menu's Paste ----------------------------------------------

    @Test
    fun theMenuPasteStagesAnAttachableItem() = runTest(dispatcher) {
        val viewModel = newViewModel()
        val uri = clipboardItem("blob")
        val clip = ClipData("image", arrayOf("image/gif"), ClipData.Item(uri))

        val result = viewModel.pasteFromClipboard(clip)
        val staged = viewModel.awaitStaged()

        assertThat(result).isEqualTo(ChatViewModel.PasteResult.STAGING)
        assertThat(staged.kind).isEqualTo(AttachmentDto.KIND_FILE)
        // The words that came with it were not swallowed into the caption.
        assertThat(viewModel.inputState.text.toString()).isEmpty()
    }

    /** Words on the clipboard still land in the composer, as words. */
    @Test
    fun theMenuPastePutsTextInTheComposer() = runTest(dispatcher) {
        val viewModel = newViewModel()

        val result = viewModel.pasteFromClipboard(ClipData.newPlainText("l", "dinner at 7"))
        advanceUntilIdle()

        assertThat(result).isEqualTo(ChatViewModel.PasteResult.TEXT)
        assertThat(viewModel.inputState.text.toString()).isEqualTo("dinner at 7")
        assertThat(viewModel.staged.value).isNull()
    }

    /** Appended to what was being written, with a separator, never over it. */
    @Test
    fun pastedTextIsAppendedToWhatWasAlreadyTyped() = runTest(dispatcher) {
        val viewModel = newViewModel()
        viewModel.inputState.setTextAndPlaceCursorAtEnd("see you at")

        viewModel.pasteFromClipboard(ClipData.newPlainText("l", "7"))
        advanceUntilIdle()

        assertThat(viewModel.inputState.text.toString()).isEqualTo("see you at 7")
    }

    /**
     * A clip carrying both — the shape a browser copy has. The picture is
     * the attachment; the words stay words.
     */
    @Test
    fun theMenuPastePrefersTheAttachableItem() = runTest(dispatcher) {
        val viewModel = newViewModel()
        val clip = ClipData.newPlainText("l", "look at this")
        clip.addItem(ClipData.Item(clipboardItem("blob")))

        viewModel.pasteFromClipboard(clip)

        assertThat(viewModel.awaitStaged().kind).isEqualTo(AttachmentDto.KIND_FILE)
        assertThat(viewModel.inputState.text.toString()).isEmpty()
    }

    /** An empty clipboard says so rather than looking broken. */
    @Test
    fun theMenuPasteSaysWhenThereIsNothingToPaste() = runTest(dispatcher) {
        val viewModel = newViewModel()

        val result = viewModel.pasteFromClipboard(null)

        assertThat(result).isEqualTo(ChatViewModel.PasteResult.NOTHING)
        assertThat(viewModel.awaitFailure().reason)
            .isEqualTo(
                RuntimeEnvironment.getApplication().getString(R.string.e_nothing_to_paste),
            )
    }

    // -- Polls -----------------------------------------------------------------

    @Test
    fun aPollCanOnlyBeStartedInTheFamilyChat() = runTest(dispatcher) {
        // Anywhere else the server answers `invalid_poll`, so the menu
        // hides the item rather than offering an affordance that fails.
        val direct = newViewModel(kind = "direct")
        runCurrent()

        assertThat(direct.canCreatePoll.value).isFalse()
        direct.beginPoll()
        assertThat(direct.pollDraft.value).isNull()
    }

    @Test
    fun theFamilyChatOpensAPollSheetOnAFreshDraft() = runTest(dispatcher) {
        val viewModel = newViewModel(kind = "family")
        runCurrent()

        assertThat(viewModel.canCreatePoll.value).isTrue()
        viewModel.beginPoll()

        val draft = viewModel.pollDraft.value!!
        assertThat(draft.question).isEmpty()
        assertThat(draft.options).hasSize(2)
        assertThat(draft.isValid).isFalse()
    }

    @Test
    fun theSheetEditsTheDraftItHolds() = runTest(dispatcher) {
        val viewModel = newViewModel(kind = "family")
        runCurrent()
        viewModel.beginPoll()

        viewModel.setPollQuestion("Pizza or pasta?")
        viewModel.setPollOption(0, "Pizza")
        viewModel.setPollOption(1, "Pasta")
        viewModel.addPollOption()
        viewModel.setPollOption(2, "Sushi")
        viewModel.removePollOption(1)

        val draft = viewModel.pollDraft.value!!
        assertThat(draft.question).isEqualTo("Pizza or pasta?")
        assertThat(draft.options).containsExactly("Pizza", "Sushi").inOrder()
        assertThat(draft.isValid).isTrue()

        viewModel.cancelPoll()
        assertThat(viewModel.pollDraft.value).isNull()
    }

    @Test
    fun sendingAPollPostsTheQuestionAsTheBodyAndClosesTheSheet() = runTest(dispatcher) {
        val viewModel = newViewModel(kind = "family")
        runCurrent()
        chatApi.postMessageHandler = { _, clientMsgId, body ->
            ApiResult.Ok(
                MessageResponse(
                    messageDto(
                        id = 1340,
                        chatId = CHAT,
                        senderId = ME,
                        clientMsgId = clientMsgId,
                        body = body,
                        poll = pollDto(88, "Pizza" to emptyList(), "Pasta" to emptyList()),
                    ),
                ),
            )
        }
        viewModel.beginPoll()
        viewModel.setPollQuestion("Pizza or pasta?")
        viewModel.setPollOption(0, "Pizza")
        viewModel.setPollOption(1, "Pasta")

        viewModel.sendPoll()
        advanceUntilIdle()

        assertThat(viewModel.pollDraft.value).isNull()
        assertThat(chatApi.postedMessages.single().third).isEqualTo("Pizza or pasta?")
        assertThat(chatApi.postedPolls.single()?.options)
            .containsExactly("Pizza", "Pasta").inOrder()
        // And the bubble is a poll, drawn off the stored row.
        val row = db.messageDao().findByServerId(1340L)!!
        assertThat(row.pollSeq).isEqualTo(88L)
    }

    @Test
    fun aPollTakesThePrimedReplyWithIt() = runTest(dispatcher) {
        val viewModel = newViewModel(kind = "family")
        runCurrent()
        chatApi.postMessageHandler = { _, clientMsgId, body ->
            ApiResult.Ok(
                MessageResponse(
                    messageDto(id = 1341, chatId = CHAT, senderId = ME, clientMsgId = clientMsgId, body = body),
                ),
            )
        }
        viewModel.beginReply(
            ReplyToDto(messageId = 1337, senderId = PEER, excerpt = "What shall we eat?"),
        )
        viewModel.beginPoll()
        viewModel.setPollQuestion("Pizza or pasta?")
        viewModel.setPollOption(0, "Pizza")
        viewModel.setPollOption(1, "Pasta")

        viewModel.sendPoll()
        advanceUntilIdle()

        assertThat(chatApi.postedReplyTargets.single()).isEqualTo(1337L)
        // Cleared, so the next message does not quote it too.
        assertThat(viewModel.replyDraft.value).isNull()
    }

    @Test
    fun anInvalidDraftIsNotSent() = runTest(dispatcher) {
        val viewModel = newViewModel(kind = "family")
        runCurrent()
        viewModel.beginPoll()
        viewModel.setPollQuestion("Pizza or pasta?")
        viewModel.setPollOption(0, "Pizza")

        viewModel.sendPoll()
        advanceUntilIdle()

        assertThat(chatApi.postedMessages).isEmpty()
        // The sheet stays open on what was typed, rather than throwing it away.
        assertThat(viewModel.pollDraft.value).isNotNull()
    }

    @Test
    fun aTapOnAnOptionVotes() = runTest(dispatcher) {
        val viewModel = newViewModel(kind = "family")
        runCurrent()
        db.messageDao().insertIgnore(
            listOf(
                MessageEntity(
                    clientMsgId = "s100",
                    serverId = 100,
                    chatId = CHAT,
                    senderId = PEER,
                    body = "Pizza or pasta?",
                    createdAt = NOON,
                    status = MessageStatus.SENT,
                    pollJson = PollCodec.encode(
                        pollDto(88, "Pizza" to emptyList(), "Pasta" to emptyList()),
                    ),
                    pollSeq = 88,
                ),
            ),
        )
        chatApi.putVoteHandler = { _, _, _ ->
            ApiResult.Ok(pollState(100L, pollDto(89, "Pizza" to listOf(ME), "Pasta" to emptyList())))
        }

        viewModel.vote(messageServerId = 100L, optionId = 5L)
        advanceUntilIdle()

        assertThat(chatApi.putVotes).containsExactly(Triple(CHAT, 100L, 5L))
        val stored = PollCodec.decode(db.messageDao().findByServerId(100L)!!.pollJson)!!
        assertThat(stored.options[0].votes).containsExactly(ME)
    }

    @Test
    fun aRefusedCloseSaysSoInTheComposersStrip() = runTest(dispatcher) {
        val viewModel = newViewModel(kind = "family")
        runCurrent()
        chatApi.closePollHandler = { _, _ ->
            ApiResult.HttpError(403, "not_message_author", "not yours")
        }

        viewModel.closePoll(messageServerId = 100L)
        advanceUntilIdle()

        assertThat(viewModel.awaitFailure().reason)
            .isEqualTo(RuntimeEnvironment.getApplication().getString(R.string.e_close_poll_failed))
    }
}
