/*
 * ChatViewModelTest.kt
 * Family Connect (Android)
 *
 * Four behaviors: the pure grouping rules (sender names, timestamps,
 * date separators, the unread divider — buildChatItems directly), the
 * loadOlder guard (one in-flight fetch, none past the start of
 * history), the read-marker rule — read means SEEN, so a report needs
 * the screen RESUMED, the list at the newest message *and* the screen
 * SETTLED, debounced (many inbound messages → one `read`) — and where
 * a chat opens. The scroll half is the load-bearing one: the server's
 * marker is monotonic, so a read posted for a message nobody looked at
 * cannot be taken back on any of that person's devices. The settled
 * half is the same defect from the other side: an empty list reports
 * "at the newest message" on its first frame, so without it a chat
 * that opens anchored marks itself wholly read before the reader has
 * seen anything.
 */

package me.nettrash.familyconnect.ui.chat

import me.nettrash.familyconnect.data.repo.TranscriptRepository
import me.nettrash.familyconnect.testutil.FakeTranscriptApi
import android.app.NotificationManager
import android.content.ClipData
import android.net.Uri
import androidx.lifecycle.SavedStateHandle
import me.nettrash.familyconnect.calls.CallEnding
import androidx.lifecycle.ViewModelStore
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModel
import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.net.dto.AttachmentsCodec
import androidx.compose.runtime.snapshots.Snapshot
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.joinAll
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.test.StandardTestDispatcher
import me.nettrash.familyconnect.data.net.LinkPreviewRepository
import okhttp3.OkHttpClient
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.TestResult
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
import me.nettrash.familyconnect.data.net.dto.AttachmentResponse
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
import me.nettrash.familyconnect.data.repo.MessageBody
import me.nettrash.familyconnect.data.repo.VoiceRecorder
import me.nettrash.familyconnect.data.repo.Waveform
import me.nettrash.familyconnect.data.repo.ParkedRecordings
import me.nettrash.familyconnect.data.repo.ParkedRecording
import me.nettrash.familyconnect.data.repo.SessionEpoch
import me.nettrash.familyconnect.calls.CallState
import me.nettrash.familyconnect.calls.CallStateSource
import me.nettrash.familyconnect.testutil.FakeVoiceRecorder
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import me.nettrash.familyconnect.data.repo.MediaStaging
import me.nettrash.familyconnect.data.repo.MessageRepository
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.testutil.FakeAttachmentApi
import me.nettrash.familyconnect.testutil.FakeChatApi
import me.nettrash.familyconnect.testutil.FakeChatSocket
import me.nettrash.familyconnect.testutil.FakeConnectivityObserver
import me.nettrash.familyconnect.testutil.FakePosterCache
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.testChatRepository
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
import me.nettrash.familyconnect.data.repo.FamilyRepository
import me.nettrash.familyconnect.data.repo.SessionRepository
import me.nettrash.familyconnect.testutil.FakeFamilyApi
import me.nettrash.familyconnect.testutil.FakeAuthApi
import me.nettrash.familyconnect.testutil.FakeTokenStore
import me.nettrash.familyconnect.testutil.RecordingWiper
import kotlinx.coroutines.flow.MutableSharedFlow

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class ChatViewModelTest {

    private companion object {
        const val ME = 7L
        const val PEER = 9L
        /** A third member, so a block can be about one person and not the set. */
        const val OTHER = 12L
        const val CHAT = 42L
        val ZONE: ZoneOffset = ZoneOffset.UTC

        // 2026-08-19 12:00:00 UTC
        const val NOON = 1_786_795_200_000L
        const val MINUTE = 60_000L
        const val DAY = 86_400_000L

        /** A voice note's waveform as the wire spells it (#79). */
        const val WAVE = "0124689abcddeeedcba987654321001245678aabbba98642"
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

    // #79, Phase 0: the microphone, the call and where a recording that
    // was not sent waits.
    private lateinit var recorder: FakeVoiceRecorder
    private lateinit var parked: ParkedRecordings
    private val epoch = SessionEpoch()
    private val callState = MutableStateFlow<CallState>(CallState.Idle)
    /** Every ViewModel a test built, so [recordingTest] can put away what they record. */
    private val viewModels = mutableListOf<ChatViewModel>()
    /**
     * Each test's parked recordings in a folder of its own: Robolectric
     * shares one filesDir between the tests of a JVM, and a store from an
     * earlier test, still sweeping on an IO thread, would take these files.
     */
    @get:org.junit.Rule
    val parkedRoot = org.junit.rules.TemporaryFolder()

    private val calls = object : CallStateSource {
        override val state: StateFlow<CallState> = callState
        override fun requestAnswer() = Unit
    }

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
        recorder = FakeVoiceRecorder(File(RuntimeEnvironment.getApplication().cacheDir, "recordings"))
        parked = ParkedRecordings(settings, epoch, repoScope, parkedRoot.newFolder("parked-recordings"))
    }

    @After
    fun tearDown() {
        repoScope.cancel()
        Dispatchers.resetMain()
        db.close()
    }

    private fun TestScope.newViewModel(
        kind: String = "direct",
        /** What `GET /chats` last said about this chat — the anchor's inputs. */
        unreadCount: Int = 0,
        myLastReadId: Long? = null,
    ): ChatViewModel {
        chatRepository = testChatRepository(chatApi, db.chatDao(), db.messageDao(), socket, repoScope, settings)
        // Only ever asked to block / unblock / report here; the fakes
        // under it are enough for that.
        val familyRepository = FamilyRepository(
            familyApi = FakeFamilyApi(),
            authApi = FakeAuthApi(),
            memberDao = db.memberDao(),
            settings = settings,
            sessionRepository = SessionRepository(
                authApi = FakeAuthApi(),
                tokenStore = FakeTokenStore("tok"),
                settings = settings,
                wiper = RecordingWiper(),
                unauthorizedEvents = MutableSharedFlow(),
                scope = repoScope,
            ),
            socket = socket,
            scope = repoScope,
        )
        messageRepository = MessageRepository(
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
            clock = Clock { NOON },
            pendingAttachmentDao = db.pendingAttachmentDao(),
            staging = MediaStaging(RuntimeEnvironment.getApplication()),
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
                        unreadCount = unreadCount,
                        myLastReadId = myLastReadId,
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
            familyRepository = familyRepository,
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
            // The microphone, scripted (#79): most tests never record, and
            // the ones that do drive its clock and its endings themselves.
            voiceRecorder = recorder,
            parked = parked,
            attachmentApi = attachmentApi,
            gallerySaver = GallerySaver(),
            // Real provider, never asked: these tests hold no location
            // permission, so `hasPermission()` is false and nothing runs.
            locationProvider = LocationProvider(RuntimeEnvironment.getApplication()),
            // Never asked: these tests draw no bubble, so no "Show text".
            transcriptRepository = TranscriptRepository(FakeTranscriptApi(), db.transcriptDao(), me.nettrash.familyconnect.testutil.FakeTranscriptSound()),
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
            calls = calls,
            // #79, Phase 1: the voice reducer's clock is the test's virtual time.
            uptime = me.nettrash.familyconnect.util.Uptime { testScheduler.currentTime },
        ).also { viewModels += it }
    }

    /**
     * Put rows in the table BEFORE the ViewModel exists.
     *
     * The opening anchor is decided once, in init, from what this device
     * already holds — so a test that seeds afterwards is testing a chat
     * that opened empty.
     */
    private fun TestScope.seed(vararg rows: MessageEntity) {
        launch { db.messageDao().insertIgnore(rows.toList()) }
        runCurrent()
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
                is ChatListItem.NewMessagesDivider -> "new:${it.count}"
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
        val file = File.createTempFile("staged", ".jpg", File(RuntimeEnvironment.getApplication().cacheDir, "uploads").apply { mkdirs() }).apply { writeBytes(ByteArray(16) { 1 }) }
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
        assertThat(viewModel.staged.value).isNotEmpty()

        viewModel.inputState.setTextAndPlaceCursorAtEnd("look at this")
        viewModel.send()
        advanceUntilIdle()

        assertThat(attachmentApi.calls).contains("upload")
        val stored = db.messageDao().observeMessages(CHAT, 50).first()
        val row = stored.firstOrNull { it.attachmentId != null }
        assertThat(row).isNotNull()
        assertThat(row!!.body).isEqualTo("look at this")
        // Composer emptied, staging consumed.
        assertThat(viewModel.staged.value).isEmpty()
        assertThat(viewModel.inputState.text.toString()).isEmpty()
    }

    /** A photo needs no caption — Send must be live on the attachment alone. */
    @Test
    fun sendCommitsStagedMediaWithNoCaption() = runTest(dispatcher) {
        val viewModel = newViewModel()
        val file = File.createTempFile("staged", ".jpg", File(RuntimeEnvironment.getApplication().cacheDir, "uploads").apply { mkdirs() }).apply { writeBytes(ByteArray(16) { 2 }) }
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
        assertThat(viewModel.staged.value).isEmpty()
    }

    /**
     * A FAILED album send restores the unsent tail to the composer.
     *
     * sendMedia deletes an item's prepared file only once its upload has
     * landed, so after a mid-way failure the files still on disk are
     * exactly the unsent items — and they must reappear STAGED, with the
     * caption back in the field, for a one-tap retry (iOS parity: both
     * Apple composers re-stage the survivors the same way).
     */
    @Test
    fun aFailedAlbumSendLeavesTheBubbleHoldingTheSet() = runTest(dispatcher) {
        // The composer used to catch a failed set and put it back, because
        // there was nowhere else for it to live. Now the send is a row from
        // the moment it is made: the bubble holds the photos, the caption
        // and the reply, and the composer is free the instant Send is
        // pressed. That is why the strip is empty here rather than full.
        val viewModel = newViewModel()
        attachmentApi.uploadHandler = { _, _, _ ->
            ApiResult.HttpError(413, "attachment_too_large", "no")
        }
        val items = List(3) { tempPrepared(tag = it.toByte()) }
        items.forEach { viewModel.stagePrepared(it) }
        runCurrent()
        viewModel.inputState.setTextAndPlaceCursorAtEnd("three of us")

        viewModel.send()
        advanceUntilIdle()

        assertThat(viewModel.staged.value).isEmpty()
        // And the message exists, with its caption and its photos, waiting
        // to be retried. A refused upload is terminal, so the row is FAILED
        // rather than queued — but it is a row, which is the whole point.
        val rows = db.messageDao().observeMessages(CHAT, 50).first()
        assertThat(rows).hasSize(1)
        assertThat(rows.single().body).isEqualTo("three of us")
        assertThat(rows.single().status).isEqualTo(MessageStatus.FAILED)
        assertThat(db.pendingAttachmentDao().itemsFor(rows.single().clientMsgId)).hasSize(3)
    }

    @Test
    fun concurrentStagingLosesNoItemAndHoldsTheCap() = runTest(dispatcher) {
        val viewModel = newViewModel()
        val items = List(24) { tempPrepared(tag = it.toByte()) }

        withContext(Dispatchers.Default) {
            items.map { item -> launch { viewModel.stagePrepared(item) } }.joinAll()
        }

        assertThat(viewModel.staged.value).hasSize(AttachmentDto.MAX_PER_MESSAGE)
        // Every item either sits staged with its file intact, or was
        // refused at the cap with its file deleted — none silently lost.
        val stagedNow = viewModel.staged.value.toSet()
        items.forEach { item ->
            if (item in stagedNow) {
                assertThat(item.file.exists()).isTrue()
            } else {
                assertThat(item.file.exists()).isFalse()
            }
        }
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

    // -- Read markers: resumed AND at the newest message AND settled ---------------------

    @Test
    fun rapidInboundMessagesProduceOneDebouncedReadReport() = runTest(dispatcher) {
        val viewModel = newViewModel()
        val itemsSubscription = repoScope.launch { viewModel.items.collect {} }
        viewModel.setResumed(true)
        viewModel.setAtNewest(true)
        viewModel.setSettled()
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
        viewModel.setSettled()
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
        viewModel.setSettled()
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
        viewModel.setSettled()
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
     * The read marker runs THROUGH a blocked member's messages.
     *
     * A marker parked below the hidden row would leap forward the moment a
     * third member posted, and that leap is a repeatable oracle for the
     * blocked person watching the other end — reason (c) in
     * docs/protocol.md's "It follows that the server still DELIVERS",
     * rebuilt by the client one layer above the server.
     *
     * The blocked member sends LAST, which is the only arrangement that
     * can catch this: with an ordinary message on top, the marker reaches
     * the right answer through it and a client that skipped hidden rows
     * would still look correct.
     */
    @Test
    fun theReadMarkerRunsThroughABlockedMembersMessages() = runTest(dispatcher) {
        settings.setBlockedUserIds(setOf(PEER))
        val viewModel = newViewModel()
        val itemsSubscription = repoScope.launch { viewModel.items.collect {} }
        viewModel.setResumed(true)
        viewModel.setAtNewest(true)
        viewModel.setSettled()
        runCurrent()

        messageRepository.applyServerMessage(messageDto(id = 40, chatId = CHAT, senderId = OTHER), live = false)
        messageRepository.applyServerMessage(messageDto(id = 41, chatId = CHAT, senderId = PEER), live = false)
        runCurrent()
        advanceTimeBy(600)
        runCurrent()

        // 41, the hidden one — not 40.
        assertThat(chatApi.postedReads).contains(CHAT to 41L)
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
        viewModel.setSettled()
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
        viewModel.setSettled()
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
        viewModel.setSettled()
        runCurrent()

        messageRepository.applyServerMessage(messageDto(id = 40, chatId = CHAT, senderId = PEER), live = false)
        runCurrent()
        advanceTimeBy(600)
        runCurrent()

        assertThat(manager.activeNotifications).isEmpty()
        itemsSubscription.cancel()
    }

    /**
     * THE DATA-LOSING RACE, from the inside.
     *
     * Everything else is true — the screen is resumed, and the list says
     * it is at the newest message, which is what an EMPTY LazyColumn
     * says on its first frame because firstVisibleItemIndex is 0. If the
     * read collector believed that, a chat about to be anchored thirty
     * messages up the thread would report the newest id and mark itself
     * wholly read, on every device this person owns and for good — the
     * server's marker only moves forward.
     *
     * runCurrent(), never advanceUntilIdle(): the point is what happens
     * while things are still in flight.
     */
    @Test
    fun noReadIsPostedBeforeTheScreenHasSettled() = runTest(dispatcher) {
        val viewModel = newViewModel()
        val itemsSubscription = repoScope.launch { viewModel.items.collect {} }
        viewModel.setResumed(true)
        viewModel.setAtNewest(true)
        runCurrent()

        messageRepository.applyServerMessage(messageDto(id = 50, chatId = CHAT, senderId = PEER), live = false)
        runCurrent()
        advanceTimeBy(2_000)
        runCurrent()

        assertThat(chatApi.postedReads).isEmpty()
        assertThat(socket.sent.filterIsInstance<ClientFrame.Read>()).isEmpty()
        assertThat(db.chatDao().getById(CHAT)!!.myLastReadId).isNull()

        // And the moment the screen says it has finished opening, the
        // same state means what it says.
        viewModel.setSettled()
        runCurrent()
        advanceTimeBy(600)
        runCurrent()
        assertThat(chatApi.postedReads).containsExactly(CHAT to 50L)
        itemsSubscription.cancel()
    }

    /**
     * The open chat is published as NOT at the newest message until the
     * screen settles — the safe direction. A message arriving during the
     * opening window is genuinely unseen, so it counts.
     */
    @Test
    fun aMessageArrivingBeforeTheScreenSettlesStillCounts() = runTest(dispatcher) {
        val viewModel = newViewModel()
        val itemsSubscription = repoScope.launch { viewModel.items.collect {} }
        viewModel.setResumed(true)
        viewModel.setAtNewest(true)
        runCurrent()

        messageRepository.applyServerMessage(messageDto(id = 51, chatId = CHAT, senderId = PEER), live = true)
        runCurrent()

        assertThat(db.chatDao().getById(CHAT)!!.unreadCount).isEqualTo(1)
        itemsSubscription.cancel()
    }

    // -- Where the chat opens ------------------------------------------------
    //
    // The arithmetic itself is pinned in OpenAnchorTest; these are about
    // the ViewModel wiring it to the chat row and the cache, once, at
    // open — and about what an anchored open must NOT do.

    @Test
    fun aChatWithUnreadMessagesOpensAtTheOldestOfThem() = runTest(dispatcher) {
        seed(
            entity("m12", 12, PEER, NOON + 2 * MINUTE),
            entity("m11", 11, PEER, NOON + MINUTE),
            entity("m10", 10, PEER, NOON),
            entity("m9", 9, PEER, NOON - MINUTE),
        )
        val viewModel = newViewModel(unreadCount = 3, myLastReadId = 9)
        val itemsSubscription = repoScope.launch { viewModel.items.collect {} }
        runCurrent()

        assertThat(viewModel.openAnchor.value)
            .isEqualTo(OpenAnchor.Message(serverId = 10, newCount = 3))
        itemsSubscription.cancel()
    }

    @Test
    fun aChatWithNothingUnreadOpensAtTheNewestMessage() = runTest(dispatcher) {
        seed(
            entity("m12", 12, PEER, NOON + MINUTE),
            entity("m11", 11, PEER, NOON),
        )
        val viewModel = newViewModel(unreadCount = 0, myLastReadId = 12)
        val itemsSubscription = repoScope.launch { viewModel.items.collect {} }
        runCurrent()

        assertThat(viewModel.openAnchor.value).isEqualTo(OpenAnchor.Newest)
        // ...and nothing draws a divider.
        assertThat(viewModel.items.value.filterIsInstance<ChatListItem.NewMessagesDivider>())
            .isEmpty()
        itemsSubscription.cancel()
    }

    /** A fresh install has no marker at all, so the count walks back. */
    @Test
    fun aFreshInstallCountsBackToTheOldestUnread() = runTest(dispatcher) {
        seed(
            entity("m12", 12, PEER, NOON + 2 * MINUTE),
            entity("m11", 11, ME, NOON + MINUTE),
            entity("m10", 10, PEER, NOON),
            entity("m9", 9, PEER, NOON - MINUTE),
        )
        val viewModel = newViewModel(unreadCount = 2, myLastReadId = null)
        val itemsSubscription = repoScope.launch { viewModel.items.collect {} }
        runCurrent()

        // 11 is mine and is skipped, exactly as the server skips it.
        assertThat(viewModel.openAnchor.value)
            .isEqualTo(OpenAnchor.Message(serverId = 10, newCount = 2))
        itemsSubscription.cancel()
    }

    /**
     * The whole product consequence in one test: a chat opened away from
     * the bottom reads NOTHING and keeps its count — and then reaching
     * the bottom does everything reading a chat ever did.
     */
    @Test
    fun anAnchoredOpenReadsNothingUntilTheReaderReachesTheBottom() = runTest(dispatcher) {
        val context = RuntimeEnvironment.getApplication()
        val manager = context.getSystemService(NotificationManager::class.java)
        PushNotifications.ensureChannel(context)
        manager.notify(
            PushNotifications.chatTag(CHAT),
            PushNotifications.NOTIFICATION_ID,
            PushNotifications.build(context, "Ben", "Dinner at 7?", kind = "message", chatId = CHAT),
        )

        seed(
            entity("m12", 12, PEER, NOON + 2 * MINUTE),
            entity("m11", 11, PEER, NOON + MINUTE),
            entity("m10", 10, PEER, NOON),
        )
        val viewModel = newViewModel(unreadCount = 3, myLastReadId = 9)
        val itemsSubscription = repoScope.launch { viewModel.items.collect {} }
        runCurrent()
        assertThat(viewModel.openAnchor.value)
            .isEqualTo(OpenAnchor.Message(serverId = 10, newCount = 3))

        // The screen anchors: the bottom sentinel is off screen, so it
        // reports NOT at the newest message, and settles there.
        viewModel.setResumed(true)
        viewModel.setAtNewest(false)
        viewModel.setSettled()
        runCurrent()
        advanceTimeBy(2_000)
        runCurrent()

        assertThat(chatApi.postedReads).isEmpty()
        assertThat(socket.sent.filterIsInstance<ClientFrame.Read>()).isEmpty()
        assertThat(db.chatDao().getById(CHAT)!!.unreadCount).isEqualTo(3)
        // The tray entry survives the open, deliberately: it is the
        // launcher dot, and there really is something still unread.
        assertThat(manager.activeNotifications).isNotEmpty()

        // Reading down to the bottom is the moment it is all seen.
        viewModel.setAtNewest(true)
        runCurrent()
        advanceTimeBy(600)
        runCurrent()

        assertThat(chatApi.postedReads).containsExactly(CHAT to 12L)
        assertThat(db.chatDao().getById(CHAT)!!.unreadCount).isEqualTo(0)
        assertThat(manager.activeNotifications).isEmpty()
        itemsSubscription.cancel()
    }

    /**
     * The anchor is captured ONCE. Reaching the bottom zeroes the count
     * and advances the marker, and the divider must not evaporate with
     * them — the reader is still looking at it.
     */
    @Test
    fun theDividerSurvivesTheChatBeingRead() = runTest(dispatcher) {
        seed(
            entity("m12", 12, PEER, NOON + MINUTE),
            entity("m11", 11, PEER, NOON),
        )
        val viewModel = newViewModel(unreadCount = 2, myLastReadId = 10)
        val itemsSubscription = repoScope.launch { viewModel.items.collect {} }
        runCurrent()
        assertThat(viewModel.items.value.filterIsInstance<ChatListItem.NewMessagesDivider>())
            .hasSize(1)

        viewModel.setResumed(true)
        viewModel.setAtNewest(true)
        viewModel.setSettled()
        runCurrent()
        advanceTimeBy(600)
        runCurrent()

        assertThat(db.chatDao().getById(CHAT)!!.unreadCount).isEqualTo(0)
        assertThat(viewModel.items.value.filterIsInstance<ChatListItem.NewMessagesDivider>())
            .hasSize(1)
        itemsSubscription.cancel()
    }

    /**
     * `reachedStart` bounds the FETCH, not the render window: a resync
     * page can leave rows in Room the window has never reached, and a
     * window that could no longer grow made them unreachable for good —
     * which is also what would strand an opening anchor behind it.
     */
    @Test
    fun theWindowKeepsWideningAfterTheServerRunsOutOfHistory() = runTest(dispatcher) {
        seed(
            *(1L..200L).map { entity("m$it", it, PEER, NOON + it * 1_000) }.toTypedArray(),
        )
        val viewModel = newViewModel()
        val itemsSubscription = repoScope.launch { viewModel.items.collect {} }
        // Empty page = the server has nothing older.
        chatApi.messagesHandler = { _, _, _, _ -> ApiResult.Ok(MessagesResponse(emptyList())) }
        runCurrent()

        fun shown() = viewModel.items.value.filterIsInstance<ChatListItem.MessageItem>().size
        assertThat(shown()).isEqualTo(ChatViewModel.INITIAL_LIMIT)

        viewModel.loadOlder()
        runCurrent()
        assertThat(shown()).isEqualTo(ChatViewModel.INITIAL_LIMIT + ChatViewModel.PAGE_SIZE)

        // The fetch is over, but the window is not.
        viewModel.loadOlder()
        runCurrent()
        assertThat(chatApi.messagesCalls).isEqualTo(1)
        assertThat(shown()).isEqualTo(ChatViewModel.INITIAL_LIMIT + 2 * ChatViewModel.PAGE_SIZE)
        itemsSubscription.cancel()
    }

    // -- The divider (pure) --------------------------------------------------

    @Test
    fun theDividerSitsDirectlyAboveTheOldestUnreadMessage() {
        val messages = listOf(
            entity("s3", 3, PEER, NOON + 2 * MINUTE),
            entity("s2", 2, PEER, NOON + MINUTE),
            entity("s1", 1, PEER, NOON),
        )
        val items = buildChatItems(
            messagesNewestFirst = messages,
            isFamilyChat = false,
            myUserId = ME,
            memberNames = emptyMap(),
            nowMillis = NOON,
            zone = ZONE,
            firstUnreadServerId = 2,
            newMessageCount = 2,
        )
        val kinds = items.map {
            when (it) {
                is ChatListItem.MessageItem -> it.entity.clientMsgId
                is ChatListItem.DateSeparator -> "sep:${it.label}"
                is ChatListItem.NewMessagesDivider -> "new:${it.count}"
            }
        }
        // List order is bottom-up, so the divider trailing s2 draws
        // directly ABOVE it — under the day pill, which trails the
        // oldest message of the day.
        assertThat(kinds).containsExactly("s3", "s2", "new:2", "s1", "sep:Today").inOrder()
    }

    @Test
    fun thereIsNoDividerWithoutAFirstUnreadMessage() {
        val messages = listOf(
            entity("s2", 2, PEER, NOON + MINUTE),
            entity("s1", 1, PEER, NOON),
        )
        val items = buildChatItems(
            messagesNewestFirst = messages,
            isFamilyChat = false,
            myUserId = ME,
            memberNames = emptyMap(),
            nowMillis = NOON,
            zone = ZONE,
        )
        assertThat(items.filterIsInstance<ChatListItem.NewMessagesDivider>()).isEmpty()
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
    ): MediaPrep.Prepared = staged.first { list -> list.any(predicate) }.last(predicate)

    /** The same wait, for the paths that end in the composer's error strip. */
    private suspend fun ChatViewModel.awaitFailure(): ChatViewModel.MediaSendState.Failed =
        mediaState.first { it is ChatViewModel.MediaSendState.Failed }
            as ChatViewModel.MediaSendState.Failed

    /** A sentence in the notice line that is not an error (#79: a dimmed microphone says why). */
    private suspend fun ChatViewModel.awaitNotice(): ChatViewModel.MediaSendState.Notice =
        mediaState.first { it is ChatViewModel.MediaSendState.Notice }
            as ChatViewModel.MediaSendState.Notice

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

    /**
     * Audio the server can magic-check keeps its player, and its name — and the bytes here are a
     * real ID3 header, because that claim is CHECKED against them now: an audio upload whose
     * bytes are not what it calls them goes as a FILE rather than as a refused audio upload
     * (MediaPrep.Magic).
     */
    @Test
    fun pastedAudioIsStagedAsAudio() = runTest(dispatcher) {
        val viewModel = newViewModel()

        viewModel.pasteAttachment(
            clipboardItem("blob", "ID3".toByteArray() + ByteArray(29) { 0 }),
            "audio/mpeg",
        )

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
        assertThat(viewModel.staged.value).isEmpty()
        assertThat(viewModel.mediaState.value).isEqualTo(ChatViewModel.MediaSendState.Idle)
    }

    /**
     * Plurality: a paste goes through the same staging the picker does,
     * so it APPENDS behind what was already staged — the old
     * replace-first rule died when a message learned to carry up to ten
     * attachments (docs/protocol.md).
     */
    @Test
    fun aPasteAppendsBehindWhateverWasAlreadyStaged() = runTest(dispatcher) {
        val viewModel = newViewModel()
        val first = File.createTempFile("staged", ".jpg", File(RuntimeEnvironment.getApplication().cacheDir, "uploads").apply { mkdirs() }).apply { writeBytes(ByteArray(16) { 3 }) }
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
        // The first item is still there, still first, its file untouched.
        assertThat(viewModel.staged.value).hasSize(2)
        assertThat(viewModel.staged.value.first().mime).isEqualTo("image/jpeg")
        assertThat(first.exists()).isTrue()
    }

    /** A message carries at most ten: the eleventh is refused with a notice. */
    @Test
    fun theEleventhStagedItemIsRefusedWithANotice() = runTest(dispatcher) {
        val viewModel = newViewModel()
        repeat(AttachmentDto.MAX_PER_MESSAGE) {
            viewModel.stagePrepared(tempPrepared(tag = it.toByte()))
        }
        runCurrent()
        assertThat(viewModel.staged.value).hasSize(AttachmentDto.MAX_PER_MESSAGE)

        val extra = tempPrepared(tag = 99)
        viewModel.stagePrepared(extra)
        runCurrent()

        assertThat(viewModel.staged.value).hasSize(AttachmentDto.MAX_PER_MESSAGE)
        // The refused file is cleaned up — nothing else ever would.
        assertThat(extra.file.exists()).isFalse()
        assertThat(viewModel.mediaState.value)
            .isInstanceOf(ChatViewModel.MediaSendState.Failed::class.java)
    }

    /** Each chip has its OWN remove: dropping one leaves the others in order. */
    @Test
    fun discardingOneStagedItemLeavesTheOthersInOrder() = runTest(dispatcher) {
        val viewModel = newViewModel()
        val a = tempPrepared(tag = 1)
        val b = tempPrepared(tag = 2)
        val c = tempPrepared(tag = 3)
        listOf(a, b, c).forEach(viewModel::stagePrepared)
        runCurrent()

        viewModel.discardStaged(1)
        runCurrent()

        assertThat(viewModel.staged.value).containsExactly(a, c).inOrder()
        assertThat(b.file.exists()).isFalse()
        assertThat(a.file.exists()).isTrue()
        assertThat(c.file.exists()).isTrue()
    }

    private fun tempPrepared(tag: Byte, bytes: Int = 16): MediaPrep.Prepared {
        val file = File.createTempFile("staged", ".jpg", File(RuntimeEnvironment.getApplication().cacheDir, "uploads").apply { mkdirs() }).apply { writeBytes(ByteArray(bytes) { tag }) }
        return MediaPrep.Prepared(
            file = file,
            mime = "image/jpeg",
            kind = AttachmentDto.KIND_PHOTO,
            width = 10,
            height = 10,
            durationMs = null,
            previewJpeg = null,
        )
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
        assertThat(viewModel.staged.value).isEmpty()
    }

    /** And the other half of that guard: one upload at a time. */
    @Test
    fun aPasteRightAfterSendStagesBecauseTheSendHasLeft() = runTest(dispatcher) {
        val viewModel = newViewModel()
        val file = File.createTempFile("staged", ".jpg", File(RuntimeEnvironment.getApplication().cacheDir, "uploads").apply { mkdirs() }).apply { writeBytes(ByteArray(16) { 4 }) }
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
        // A send no longer holds the composer: it becomes a row and its
        // uploads belong to the outbox, so the person can compose the next
        // message immediately. Pasting straight after Send therefore
        // STAGES rather than being refused as BUSY, which is the point of
        // making a media send durable.
        viewModel.send()

        val result = viewModel.pasteAttachment(clipboardItem("blob"), "application/pdf")

        assertThat(result).isEqualTo(ChatViewModel.PasteResult.STAGING)
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
        assertThat(viewModel.staged.value).isEmpty()
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

    // -- The text field's own paste -------------------------------------------
    //
    // The other door: the field's long-press menu, Ctrl+V from a hardware
    // keyboard, a keyboard that inserts pictures, a drop onto the composer.
    // It used to decide for itself what a clip was; these say that it now
    // gives the SAME answer the menu does, because both ask the same rule.

    /** A clip holding a picture stages it, whichever door it came through. */
    @Test
    fun theFieldPasteStagesAnAttachableItem() = runTest(dispatcher) {
        val viewModel = newViewModel()
        val clip = ClipData("image", arrayOf("image/gif"), ClipData.Item(clipboardItem("blob")))

        val result = viewModel.pasteIntoField(clip)

        assertThat(result).isEqualTo(ChatViewModel.PasteResult.STAGING)
        assertThat(viewModel.awaitStaged().kind).isEqualTo(AttachmentDto.KIND_FILE)
    }

    /**
     * Words are the one thing this door does NOT do itself: it reports
     * them and hands them back, so the field inserts them where the caret
     * is. Appending here would move somebody's cursor for no reason on the
     * one platform that never had to.
     */
    @Test
    fun theFieldPasteLeavesWordsToTheField() = runTest(dispatcher) {
        val viewModel = newViewModel()
        viewModel.inputState.setTextAndPlaceCursorAtEnd("see you at")

        val result = viewModel.pasteIntoField(ClipData.newPlainText("l", "7"))
        advanceUntilIdle()

        assertThat(result).isEqualTo(ChatViewModel.PasteResult.TEXT)
        // Untouched: the field has not pasted yet, and this door must not
        // paste for it.
        assertThat(viewModel.inputState.text.toString()).isEqualTo("see you at")
        assertThat(viewModel.staged.value).isEmpty()
    }

    /** A copied link is words at this door too — not a download. */
    @Test
    fun theFieldPasteTreatsALinkAsWords() = runTest(dispatcher) {
        val viewModel = newViewModel()
        val clip = ClipData(
            "uri",
            arrayOf("text/uri-list"),
            ClipData.Item(
                "https://example.com/holiday.jpg",
                null,
                Uri.parse("https://example.com/holiday.jpg"),
            ),
        )

        val result = viewModel.pasteIntoField(clip)
        advanceUntilIdle()

        assertThat(result).isEqualTo(ChatViewModel.PasteResult.TEXT)
        assertThat(viewModel.staged.value).isEmpty()
    }

    /**
     * The clip the two doors used to disagree about: a picture and the
     * words that came with it. Both take the picture, and neither types
     * the words — a caption is written, not inherited.
     */
    @Test
    fun bothDoorsAnswerAMixedClipTheSameWay() = runTest(dispatcher) {
        val menuClip = ClipData.newPlainText("l", "look at this")
        menuClip.addItem(ClipData.Item(clipboardItem("blob")))
        val fieldClip = ClipData.newPlainText("l", "look at this")
        fieldClip.addItem(ClipData.Item(clipboardItem("blob")))

        val throughTheMenu = newViewModel()
        val throughTheField = newViewModel()

        val menuResult = throughTheMenu.pasteFromClipboard(menuClip)
        val fieldResult = throughTheField.pasteIntoField(fieldClip)

        assertThat(menuResult).isEqualTo(fieldResult)
        assertThat(throughTheMenu.awaitStaged().kind).isEqualTo(AttachmentDto.KIND_FILE)
        assertThat(throughTheField.awaitStaged().kind).isEqualTo(AttachmentDto.KIND_FILE)
        assertThat(throughTheMenu.inputState.text.toString()).isEmpty()
        assertThat(throughTheField.inputState.text.toString()).isEmpty()
    }

    /** The field's paste never claims there was nothing to paste — the field knows. */
    @Test
    fun theFieldPasteStaysQuietAboutAClipItCannotPlace() = runTest(dispatcher) {
        val viewModel = newViewModel()

        val result = viewModel.pasteIntoField(ClipData.newPlainText("l", ""))
        advanceUntilIdle()

        assertThat(result).isEqualTo(ChatViewModel.PasteResult.NOTHING)
        assertThat(viewModel.mediaState.value).isEqualTo(ChatViewModel.MediaSendState.Idle)
    }

    /**
     * The ordering that used to be wrong: the busy guard ran BEFORE the
     * rule, so a copied LINK pasted mid-edit came back BUSY and the door
     * swallowed an address that was never an attachment.
     */
    @Test
    fun aLinkStillPastesAsWordsWhileEditing() = runTest(dispatcher) {
        val viewModel = newViewModel()
        viewModel.beginEdit(messageId = 5, body = "old text")
        runCurrent()
        val clip = ClipData(
            "uri",
            arrayOf("text/uri-list"),
            ClipData.Item("https://example.com/a", null, Uri.parse("https://example.com/a")),
        )

        val result = viewModel.pasteFromClipboard(clip)
        advanceUntilIdle()

        assertThat(result).isEqualTo(ChatViewModel.PasteResult.TEXT)
        assertThat(viewModel.inputState.text.toString())
            .isEqualTo("old text https://example.com/a")
    }

    /**
     * And when it really was an attachment: the edit banner explains the
     * MODE, not the refusal, so the refusal says itself.
     */
    @Test
    fun aPasteRefusedByAnEditSaysWhy() = runTest(dispatcher) {
        val viewModel = newViewModel()
        viewModel.beginEdit(messageId = 5, body = "old text")
        runCurrent()

        val result = viewModel.pasteAttachment(clipboardItem("blob"), "application/pdf")

        assertThat(result).isEqualTo(ChatViewModel.PasteResult.BUSY)
        assertThat(viewModel.awaitFailure().reason).isEqualTo(
            RuntimeEnvironment.getApplication().getString(R.string.e_finish_editing_first),
        )
    }

    /**
     * An error notice is not the composer being busy. It used to be —
     * both doors treated any non-Idle state as blocked — so the sentence
     * left behind by one failed paste blocked the next one until
     * something cleared it.
     */
    @Test
    fun aPasteWorksWhileAnErrorNoticeIsStillShowing() = runTest(dispatcher) {
        val viewModel = newViewModel()
        // The notice a paste of an empty clipboard leaves behind.
        viewModel.pasteFromClipboard(null)
        runCurrent()
        assertThat(viewModel.mediaState.value)
            .isInstanceOf(ChatViewModel.MediaSendState.Failed::class.java)

        val result = viewModel.pasteAttachment(clipboardItem("blob"), "application/pdf")

        assertThat(result).isEqualTo(ChatViewModel.PasteResult.STAGING)
        assertThat(viewModel.awaitStaged().kind).isEqualTo(AttachmentDto.KIND_FILE)
    }

    // -- The body limit -------------------------------------------------------
    //
    // Nothing enforced 4000 characters anywhere before this: a pasted wall
    // of text looked like it had worked, then failed at Send with
    // `message_too_long` — by which time the clipboard had often moved on.

    /**
     * What fits is kept and the sentence says the rest was left out — the
     * same choice the Apple clients make, so a family using both sees one
     * behaviour.
     */
    @Test
    fun theMenuPasteKeepsWhatFitsAndSaysTheRestWasNot() = runTest(dispatcher) {
        val viewModel = newViewModel()

        val result = viewModel.pasteFromClipboard(
            ClipData.newPlainText("l", "x".repeat(MessageBody.MAX_CHARS + 500)),
        )

        assertThat(result).isEqualTo(ChatViewModel.PasteResult.TRUNCATED)
        assertThat(viewModel.inputState.text.length).isEqualTo(MessageBody.MAX_CHARS)
        assertThat(viewModel.awaitFailure().reason).isEqualTo(
            RuntimeEnvironment.getApplication()
                .getString(R.string.e_paste_truncated, MessageBody.MAX_CHARS),
        )
    }

    @Test
    fun wordsThatExactlyFitAreTaken() = runTest(dispatcher) {
        val viewModel = newViewModel()

        val result = viewModel.pasteFromClipboard(
            ClipData.newPlainText("l", "x".repeat(MessageBody.MAX_CHARS)),
        )
        advanceUntilIdle()

        assertThat(result).isEqualTo(ChatViewModel.PasteResult.TEXT)
        assertThat(viewModel.inputState.text.length).isEqualTo(MessageBody.MAX_CHARS)
    }

    /** What is ALREADY in the draft counts towards the limit. */
    @Test
    fun aPasteIntoANearlyFullDraftKeepsOnlyWhatFits() = runTest(dispatcher) {
        val viewModel = newViewModel()
        viewModel.inputState.setTextAndPlaceCursorAtEnd("x".repeat(MessageBody.MAX_CHARS - 3))

        val result = viewModel.pasteFromClipboard(ClipData.newPlainText("l", "yyy"))

        // 3997 x's, a separator, then the two y's there was room for.
        assertThat(result).isEqualTo(ChatViewModel.PasteResult.TRUNCATED)
        assertThat(viewModel.inputState.text.toString())
            .isEqualTo("x".repeat(MessageBody.MAX_CHARS - 3) + " yy")
    }

    /**
     * A draft already at the ceiling takes nothing, is left exactly as it
     * was, and gets the OTHER sentence — "the rest wasn't pasted" is wrong
     * when none of it was.
     */
    @Test
    fun aPasteIntoAFullDraftChangesNothingAndSaysSo() = runTest(dispatcher) {
        val viewModel = newViewModel()
        val full = "x".repeat(MessageBody.MAX_CHARS)
        viewModel.inputState.setTextAndPlaceCursorAtEnd(full)

        val result = viewModel.pasteFromClipboard(ClipData.newPlainText("l", "more"))

        assertThat(result).isEqualTo(ChatViewModel.PasteResult.FULL)
        assertThat(viewModel.inputState.text.toString()).isEqualTo(full)
        assertThat(viewModel.awaitFailure().reason).isEqualTo(
            RuntimeEnvironment.getApplication()
                .getString(R.string.e_message_at_limit, MessageBody.MAX_CHARS),
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

    // -- Pictures -------------------------------------------------------------
    //
    // The composer's two picture affordances, each behind its own
    // capability check. What is asserted here is ABSENCE as much as
    // presence: a surface offered where the server will not act is the
    // failure mode the whole `assistant` object exists to prevent, and in
    // the vision case it would be a surface that says pixels are about to
    // leave when they are not — or, far worse, the reverse.

    /** Both locks open, in the member's own thread: the one place it is offered. */
    private fun TestScope.picturesConfigured(
        vision: Boolean = true,
        images: Boolean = true,
        familyAllows: Boolean = true,
        /** The owner's third switch — off by default, as on the wire. */
        historyPhotos: Boolean = false,
        familyHistory: Boolean = true,
    ) {
        launch {
            settings.setAssistant(userId = 1L, displayName = "Assistant", vision = vision, images = images)
            settings.setFamilyAiVision(familyAllows)
            settings.setFamilyAiHistory(familyHistory)
            settings.setFamilyAiHistoryPhotos(historyPhotos)
        }
        runCurrent()
    }

    @Test
    fun aPictureIsOfferedOnlyInTheMembersOwnThreadWithBothLocksOpen() = runTest(dispatcher) {
        for (kind in listOf("ai", "family", "direct")) {
            val viewModel = newViewModel(kind = kind)
            picturesConfigured()
            runCurrent()
            // The DEDICATED door is the assistant's own chat's. Not
            // because a picture never reaches the assistant from the
            // family chat — since #56 a photo on an `@ai` message, or on
            // the message it replies to, travels under these same two
            // locks (docs/protocol.md, "Showing the assistant a picture
            // from the family chat") — but because that path rides the
            // ordinary "Photo or video" door and the reply affordance the
            // family composer already has.
            assertThat(viewModel.canShowAssistantPicture.value).isEqualTo(kind == "ai")
        }
    }

    @Test
    fun neitherLockOnItsOwnOffersAPicture() = runTest(dispatcher) {
        val viewModel = newViewModel(kind = "ai")

        // The operator has no vision deployment: this cannot happen at
        // all here, whatever the owner switched on.
        picturesConfigured(vision = false, familyAllows = true)
        runCurrent()
        assertThat(viewModel.canShowAssistantPicture.value).isFalse()

        // The server can see, and this family has not said it may.
        picturesConfigured(vision = true, familyAllows = false)
        runCurrent()
        assertThat(viewModel.canShowAssistantPicture.value).isFalse()

        picturesConfigured(vision = true, familyAllows = true)
        runCurrent()
        assertThat(viewModel.canShowAssistantPicture.value).isTrue()
    }

    /**
     * Generation takes BOTH surfaces — the member's own thread and the
     * family chat, where the whole family watches the answer arrive. It
     * is allowed there precisely because what leaves is only the asking
     * member's own words, which a mention already sends today.
     */
    @Test
    fun drawIsOfferedInBothAssistantSurfacesAndNowhereElse() = runTest(dispatcher) {
        for (kind in listOf("ai", "family", "direct")) {
            val viewModel = newViewModel(kind = kind)
            picturesConfigured()
            runCurrent()
            assertThat(viewModel.canAskForPicture.value).isEqualTo(kind != "direct")
        }
    }

    /** A server that cannot make one must not offer to: `/draw` is just text there. */
    @Test
    fun drawIsNotOfferedWithoutAnImagesDeployment() = runTest(dispatcher) {
        val viewModel = newViewModel(kind = "ai")
        picturesConfigured(images = false)
        runCurrent()
        assertThat(viewModel.canAskForPicture.value).isFalse()
    }

    /**
     * The composer's output has to be a body the SERVER reads as a
     * request, so it is checked through the shared grammar rather than
     * against a literal — including the family chat's leading mention,
     * without which the server never looks for the token at all.
     */
    @Test
    fun theDrawButtonRewritesTheDraftIntoARequest() = runTest(dispatcher) {
        val aiChat = newViewModel(kind = "ai")
        picturesConfigured()
        runCurrent()
        aiChat.inputState.setTextAndPlaceCursorAtEnd("a cat in a hat")
        aiChat.insertDrawToken()
        assertThat(aiChat.inputState.text.toString()).isEqualTo("/draw a cat in a hat")
        assertThat(AssistantMention.drawPrompt(aiChat.inputState.text.toString()))
            .isEqualTo("a cat in a hat")

        val familyChat = newViewModel(kind = "family")
        picturesConfigured()
        runCurrent()
        familyChat.inputState.setTextAndPlaceCursorAtEnd("a cat in a hat")
        familyChat.insertDrawToken()
        assertThat(familyChat.inputState.text.toString()).isEqualTo("@ai /draw a cat in a hat")
        assertThat(AssistantMention.mentions(familyChat.inputState.text.toString())).isTrue()
        assertThat(AssistantMention.drawPrompt(familyChat.inputState.text.toString()))
            .isEqualTo("a cat in a hat")
    }

    /** No capability, no rewrite: the draft is left exactly as it was. */
    @Test
    fun theDrawButtonDoesNothingWhereItIsNotOffered() = runTest(dispatcher) {
        val viewModel = newViewModel(kind = "direct")
        picturesConfigured()
        runCurrent()
        viewModel.inputState.setTextAndPlaceCursorAtEnd("a cat")

        viewModel.insertDrawToken()

        assertThat(viewModel.inputState.text.toString()).isEqualTo("a cat")
    }

    /**
     * The disclosure is raised by what is STAGED, not by the door it came
     * through — and only in the assistant's own thread, where a picture
     * can actually be shown to it.
     */
    @Test
    fun theDisclosureIsRaisedByWhatIsStagedAndOnlyInTheAssistantsThread() = runTest(dispatcher) {
        val viewModel = newViewModel(kind = "ai")
        picturesConfigured()
        runCurrent()
        // Nothing staged: nothing to say.
        assertThat(viewModel.assistantPictureNotice.value).isNull()

        viewModel.stagePrepared(tempPrepared(tag = 1))
        runCurrent()
        assertThat(viewModel.assistantPictureNotice.value)
            .isEqualTo(
                AiPictureNotice.WillShow(
                    shown = 1, extraPhotos = 0, otherAttachments = 0, unreadablePhotos = 0,
                ),
            )

        // The same photo in the family chat raises no strip HERE. This
        // pins this strip's scope, not a rule about what travels: since
        // #56 a photo on an `@ai` message there DOES reach the model under
        // the same two locks (docs/protocol.md, "Showing the assistant a
        // picture from the family chat"), and the family composer has a
        // strip of its own for that moment — [ChatViewModel.mentionPictureNotice],
        // raised by the draft mentioning the assistant, pinned below.
        val familyChat = newViewModel(kind = "family")
        picturesConfigured()
        familyChat.stagePrepared(tempPrepared(tag = 1))
        runCurrent()
        assertThat(familyChat.assistantPictureNotice.value).isNull()
    }

    /**
     * THE BOUND THE STRIP CLAIMED AND DID NOT APPLY.
     *
     * A photograph over 5 MiB is one the SERVER leaves out and names to
     * the model — while the strip told the member it "leaves this server
     * for the model your server talks to". This goes through the real
     * staging path, so the size it is judged on is the one measured off
     * the file that will be uploaded.
     */
    @Test
    fun aStagedPhotoTheServerWillLeaveOutIsNotPromisedToTheModel() = runTest(dispatcher) {
        val viewModel = newViewModel(kind = "ai")
        picturesConfigured()
        runCurrent()

        viewModel.stagePrepared(tempPrepared(tag = 1, bytes = (AiPictureNotice.MAX_BYTES + 1).toInt()))
        runCurrent()

        assertThat(viewModel.assistantPictureNotice.value).isEqualTo(
            AiPictureNotice.WillShow(
                shown = 0, extraPhotos = 0, otherAttachments = 0, unreadablePhotos = 1,
            ),
        )
    }

    /** With a lock shut the sentence flips rather than disappearing. */
    @Test
    fun aPictureThatWillNotBeShownSaysSo() = runTest(dispatcher) {
        val viewModel = newViewModel(kind = "ai")
        picturesConfigured(familyAllows = false)
        runCurrent()
        viewModel.stagePrepared(tempPrepared(tag = 1))
        runCurrent()

        assertThat(viewModel.assistantPictureNotice.value).isEqualTo(AiPictureNotice.WillNotShow)
    }

    // -- The family composer's strip (#56) --------------------------------------

    /**
     * Type into the composer the way a member does, and let the
     * ViewModel's snapshot watch see it: a [TextFieldState] write lands in
     * the global snapshot, and outside a composition nobody sends the
     * apply notifications `snapshotFlow` listens for.
     */
    private fun TestScope.type(viewModel: ChatViewModel, draft: String) {
        viewModel.inputState.setTextAndPlaceCursorAtEnd(draft)
        Snapshot.sendApplyNotifications()
        runCurrent()
    }

    /** A photo somebody else sent, as its row is held here. */
    private fun photoMessage(serverId: Long, photos: Int): MessageEntity = MessageEntity(
        clientMsgId = "s$serverId",
        serverId = serverId,
        chatId = CHAT,
        senderId = PEER,
        body = "",
        createdAt = NOON,
        status = MessageStatus.SENT,
        attachmentsJson = AttachmentsCodec.encode(
            List(photos) { index ->
                AttachmentDto(
                    id = serverId * 10 + index, kind = AttachmentDto.KIND_PHOTO, mime = "image/jpeg",
                    size = 40_000L, width = 64, height = 64, hasPreview = true,
                )
            },
        ),
    )

    /**
     * With a lock shut nothing leaves, so nothing is announced — and the
     * family composer, unlike the assistant's own, says nothing at all
     * rather than naming a lock a non-owner cannot open.
     */
    @Test
    fun theFamilyStripIsAbsentWithALockShut() = runTest(dispatcher) {
        val familyAllowsNot = newViewModel(kind = "family")
        picturesConfigured(familyAllows = false)
        familyAllowsNot.stagePrepared(tempPrepared(tag = 1))
        type(familyAllowsNot, "@ai what is this?")
        assertThat(familyAllowsNot.mentionPictureNotice.value).isNull()

        val serverCannotSee = newViewModel(kind = "family")
        picturesConfigured(vision = false)
        serverCannotSee.stagePrepared(tempPrepared(tag = 1))
        type(serverCannotSee, "@ai what is this?")
        assertThat(serverCannotSee.mentionPictureNotice.value).isNull()
    }

    /** Without `@ai` the photo is an ordinary attachment on an ordinary message. */
    @Test
    fun theFamilyStripIsAbsentWithoutAMention() = runTest(dispatcher) {
        val viewModel = newViewModel(kind = "family")
        picturesConfigured()
        viewModel.stagePrepared(tempPrepared(tag = 1))
        type(viewModel, "what is this?")
        assertThat(viewModel.mentionPictureNotice.value).isNull()
        // And it appears the moment the mention is typed, off the same
        // staged photo — the strip follows the draft, not the door.
        type(viewModel, "@ai what is this?")
        assertThat(viewModel.mentionPictureNotice.value).isEqualTo(
            MentionPictureNotice(
                shownOnMention = 1, shownOnQuote = 0, extraPhotos = 0, otherAttachments = 0, unreadablePhotos = 0,
            ),
        )
    }

    /** The family chat's strip, and only the family chat's. */
    @Test
    fun theFamilyStripIsPresentWithAStagedPhotoAndOnlyInTheFamilyChat() = runTest(dispatcher) {
        val family = newViewModel(kind = "family")
        picturesConfigured()
        family.stagePrepared(tempPrepared(tag = 1))
        type(family, "@ai what is this?")
        assertThat(family.mentionPictureNotice.value).isEqualTo(
            MentionPictureNotice(
                shownOnMention = 1, shownOnQuote = 0, extraPhotos = 0, otherAttachments = 0, unreadablePhotos = 0,
            ),
        )
        // The assistant's own chat has its own strip (assistantPictureNotice)
        // and a direct chat has no assistant in it at all.
        for (kind in listOf("ai", "direct")) {
            val other = newViewModel(kind = kind)
            picturesConfigured()
            other.stagePrepared(tempPrepared(tag = 1))
            type(other, "@ai what is this?")
            assertThat(other.mentionPictureNotice.value).isNull()
        }
    }

    /**
     * Replying to a photo with `@ai` points the assistant at THAT photo,
     * and the strip says so — read off this device's own row for the
     * quoted message, because a ReplyToDto carries an excerpt and nothing
     * else.
     */
    @Test
    fun theFamilyStripIsPresentWithAQuotedPhoto() = runTest(dispatcher) {
        seed(photoMessage(serverId = 100, photos = 1))
        val viewModel = newViewModel(kind = "family")
        picturesConfigured()
        viewModel.beginReply(ReplyToDto(messageId = 100, senderId = PEER, excerpt = ""))
        type(viewModel, "@ai what is this?")
        assertThat(viewModel.mentionPictureNotice.value).isEqualTo(
            MentionPictureNotice(
                shownOnMention = 0, shownOnQuote = 1, extraPhotos = 0, otherAttachments = 0, unreadablePhotos = 0,
            ),
        )
        // Cancelling the reply takes the photo — and the strip — with it.
        viewModel.cancelReply()
        runCurrent()
        assertThat(viewModel.mentionPictureNotice.value).isNull()
    }

    /**
     * THE SHARED BUDGET, end to end: three staged and three on the quoted
     * message is four shown — the staged first — and two named, exactly
     * as the server will count them.
     */
    @Test
    fun theFamilyStripCountsAcrossMentionAndQuoteAsTheServerDoes() = runTest(dispatcher) {
        seed(photoMessage(serverId = 100, photos = 3))
        val viewModel = newViewModel(kind = "family")
        picturesConfigured()
        viewModel.beginReply(ReplyToDto(messageId = 100, senderId = PEER, excerpt = ""))
        viewModel.stagePrepared(tempPrepared(tag = 1))
        viewModel.stagePrepared(tempPrepared(tag = 2))
        viewModel.stagePrepared(tempPrepared(tag = 3))
        type(viewModel, "@ai which is best?")
        assertThat(viewModel.mentionPictureNotice.value).isEqualTo(
            MentionPictureNotice(
                shownOnMention = 3, shownOnQuote = 1, extraPhotos = 2, otherAttachments = 0, unreadablePhotos = 0,
            ),
        )
        assertThat(viewModel.mentionPictureNotice.value!!.shown).isEqualTo(AiPictureNotice.MAX_PHOTOS)
    }

    // -- Recent photos: the owner's third switch --------------------------------

    /**
     * Under `ai_history_photos` the strip shows for a bare `@ai` draft —
     * the case #56's strip was absent for — and says "up to four", because
     * every one of the four may be somebody else's recent picture. It
     * follows the switch off again, mirrored from the settings the
     * repository writes on every `GET /families/mine` and on the owner's
     * own PATCH (docs/protocol.md, "Recent photos from the family chat").
     */
    @Test
    fun theFamilyStripShowsForABareMentionUnderTheThirdSwitch() = runTest(dispatcher) {
        val viewModel = newViewModel(kind = "family")
        picturesConfigured(historyPhotos = true)
        type(viewModel, "@ai what time is the match?")
        assertThat(viewModel.mentionPictureNotice.value).isEqualTo(
            MentionPictureNotice(
                shownOnMention = 0, shownOnQuote = 0, extraPhotos = 0, otherAttachments = 0,
                unreadablePhotos = 0, recentUpTo = AiPictureNotice.MAX_PHOTOS,
            ),
        )
        // A staged photo takes its place first; history gets what is left.
        viewModel.stagePrepared(tempPrepared(tag = 1))
        runCurrent()
        assertThat(viewModel.mentionPictureNotice.value!!.recentUpTo).isEqualTo(3)
        // The switch off again — the owner's PATCH answer, or the next
        // resync — and the strip is #56's: a photo of the member's own,
        // nothing about the history.
        launch { settings.setFamilyAiHistoryPhotos(false) }
        runCurrent()
        assertThat(viewModel.mentionPictureNotice.value!!.recentUpTo).isNull()
        viewModel.discardStaged(0)
        runCurrent()
        assertThat(viewModel.mentionPictureNotice.value).isNull()
    }

    /**
     * The third switch is inert with either lock shut or without a
     * transcript, and the strip must not announce what will not happen:
     * with `ai_history` off there is no history for a photo to come from.
     */
    @Test
    fun theThirdSwitchIsInertWithoutItsPrerequisites() = runTest(dispatcher) {
        val noHistory = newViewModel(kind = "family")
        picturesConfigured(historyPhotos = true, familyHistory = false)
        type(noHistory, "@ai what is this?")
        assertThat(noHistory.mentionPictureNotice.value).isNull()

        val familyAllowsNot = newViewModel(kind = "family")
        picturesConfigured(historyPhotos = true, familyAllows = false)
        type(familyAllowsNot, "@ai what is this?")
        assertThat(familyAllowsNot.mentionPictureNotice.value).isNull()

        val serverCannotSee = newViewModel(kind = "family")
        picturesConfigured(historyPhotos = true, vision = false)
        type(serverCannotSee, "@ai what is this?")
        assertThat(serverCannotSee.mentionPictureNotice.value).isNull()

        // And never in the assistant's own chat, whose pictures are the
        // member's own and never an earlier turn's.
        val own = newViewModel(kind = "ai")
        picturesConfigured(historyPhotos = true)
        type(own, "@ai what is this?")
        assertThat(own.mentionPictureNotice.value).isNull()
    }

    // -- Today's recorder made safe (#79, Phase 0) ----------------------------
    //
    // docs/audio-video-messages-2026-10-04.md, S2.8 and S4: nothing records
    // during a call; an interruption stops and KEEPS — never sends, never
    // discards — and what it keeps waits in its own "Voice message not sent"
    // row, with the reply it was recorded under, until the person sends or
    // deletes it; the five-minute cap goes to review; and the recorder that
    // used to outlive the chat that started it is stopped when it goes.

    private val app get() = RuntimeEnvironment.getApplication()

    private val aQuote = ReplyToDto(messageId = 501, senderId = PEER, excerpt = "Are you coming?")
    private val anotherQuote = ReplyToDto(messageId = 502, senderId = PEER, excerpt = "Bring the cake")

    /** The chat's not-sent rows, once [count] of them have landed (the store writes off the main thread). */
    private suspend fun ChatViewModel.awaitNotSent(count: Int = 1) =
        notSent.first { it.size == count }

    /** A recording of [ms] running in this chat's composer. */
    private fun TestScope.recording(viewModel: ChatViewModel, ms: Long) {
        viewModel.startRecording()
        runCurrent()
        recorder.elapsed = ms
    }

    /** A not-sent message parked straight into the store, as an earlier launch would have left it. */
    private suspend fun parkedNote(ms: Long, replyTo: ReplyToDto? = null, caption: String = ""): String {
        val source = File.createTempFile("voice-", ".m4a", app.cacheDir)
            .apply { writeBytes(FakeVoiceRecorder.M4A_HEAD + ByteArray(4096) { 3 }) }
        return requireNotNull(parked.park(CHAT, source, ms, replyTo, caption, epoch.current())).id
    }

    /**
     * runTest for the tests that record. runTest drains the test clock when
     * the body ends — pass or fail — and a recording's 200 ms counter would
     * keep it busy for ever, so whatever a test leaves recording is put away
     * first, however the body ended.
     */
    private fun recordingTest(body: suspend TestScope.() -> Unit): TestResult = runTest(dispatcher) {
        try {
            body()
        } finally {
            viewModels.forEach { it.cancelRecording() }
        }
    }

    /**
     * What the store holds once every park already on its way has landed:
     * a park takes the store's lock before it touches the disk, and so does
     * the sweep — so once the parks due now have started, this waits for
     * them all. runCurrent, not advanceUntilIdle: a test may leave a
     * recording's counter ticking, and that clock never goes idle.
     */
    private suspend fun TestScope.settledParks(): List<ParkedRecording> {
        runCurrent()
        parked.sweep()
        return settings.current.parkedRecordings
    }

    /** Clear [viewModel] the way the system does when its screen is popped, which runs onCleared. */
    private fun clear(viewModel: ChatViewModel) {
        val store = ViewModelStore()
        ViewModelProvider(
            store,
            object : ViewModelProvider.Factory {
                @Suppress("UNCHECKED_CAST")
                override fun <T : ViewModel> create(modelClass: Class<T>): T = viewModel as T
            },
        )[ChatViewModel::class.java]
        store.clear()
    }

    private fun failure(viewModel: ChatViewModel): String? =
        (viewModel.mediaState.value as? ChatViewModel.MediaSendState.Failed)?.reason

    /** What the notice line says when it is not an error (Phase 1's [ChatViewModel.MediaSendState.Notice]). */
    private fun notice(viewModel: ChatViewModel): String? =
        (viewModel.mediaState.value as? ChatViewModel.MediaSendState.Notice)?.text

    /** No recording during a call, in any phase (S1.7): nothing opens, and the strip says why. */
    @Test
    fun recordingIsRefusedDuringACall() = recordingTest {
        callState.value = CallState.Incoming(callId = "c1", chatId = CHAT, peerUserId = PEER)
        val viewModel = newViewModel()
        runCurrent()

        viewModel.startRecording()
        runCurrent()

        assertThat(recorder.starts).isEqualTo(0)
        assertThat(viewModel.recordingMs.value).isNull()
        assertThat(viewModel.callLive.value).isTrue()
        // Phase 1: a dimmed microphone's reason is a notice, not an error (S1.3).
        assertThat(notice(viewModel)).isEqualTo(app.getString(R.string.e_record_after_the_call))
    }

    /** "A call in any phase but idle or ended" (S1.2): one that has ended is no longer one. */
    @Test
    fun anEndedCallNoLongerStopsAnybodyRecording() = recordingTest {
        callState.value = CallState.Ended(
            callId = "c1", chatId = CHAT, peerUserId = PEER, reason = CallEnding.HANGUP,
        )
        val viewModel = newViewModel()
        runCurrent()

        viewModel.startRecording()
        runCurrent()

        assertThat(recorder.starts).isEqualTo(1)
        assertThat(viewModel.callLive.value).isFalse()
    }

    /**
     * A call that rings stops the recording and KEEPS it (S4): a "not sent"
     * row with its length and the reply it was recorded under, which leaves
     * the composer with it. Nothing is sent.
     */
    @Test
    fun aCallThatRingsStopsTheRecordingAndKeepsItAsNotSent() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        viewModel.beginReply(aQuote)
        recording(viewModel, 4_200)

        callState.value = CallState.Incoming(callId = "c1", chatId = CHAT, peerUserId = PEER)
        runCurrent()

        assertThat(recorder.isRecording).isFalse()
        assertThat(viewModel.recordingMs.value).isNull()
        val entry = viewModel.awaitNotSent().single()
        assertThat(entry.chatId).isEqualTo(CHAT)
        assertThat(entry.durationMs).isEqualTo(4_200)
        assertThat(entry.replyTo).isEqualTo(aQuote)
        assertThat(entry.caption).isEmpty()
        assertThat(parked.file(entry).exists()).isTrue()
        assertThat(parked.file(entry).parentFile).isEqualTo(File(parkedRoot.root, "parked-recordings"))
        assertThat(viewModel.replyDraft.value).isNull()
        assertThat(attachmentApi.calls).doesNotContain("upload")
    }

    /** Under a second there is nothing worth keeping: deleted, and nothing parked. */
    @Test
    fun anInterruptionUnderASecondKeepsNothing() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        recording(viewModel, 999)

        callState.value = CallState.Outgoing(callId = "c1", chatId = CHAT, peerUserId = PEER)
        advanceUntilIdle()

        assertThat(recorder.isRecording).isFalse()
        assertThat(recorder.files.single().exists()).isFalse()
        assertThat(settledParks()).isEmpty()
        assertThat(viewModel.recordingMs.value).isNull()
    }

    /**
     * THE BUG: Back mid-recording left the one recorder in the process
     * recording, the microphone open, and every later chat unable to record.
     * Clearing the ViewModel stops it and keeps what it had.
     */
    @Test
    fun clearingTheChatClosesTheMicrophoneAndKeepsTheRecording() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        recording(viewModel, 6_000)

        clear(viewModel)
        advanceUntilIdle()

        assertThat(recorder.isRecording).isFalse()
        val entry = settings.state.first { it.parkedRecordings.isNotEmpty() }.parkedRecordings.single()
        assertThat(entry.durationMs).isEqualTo(6_000)
        assertThat(parked.file(entry).exists()).isTrue()
    }

    /**
     * Leaving the chat (S2.8): a voice message still in review becomes a "not
     * sent" one, taking the words in the field as its caption and leaving the
     * field empty. What is not a recording stays as it was — it can be
     * picked again.
     */
    @Test
    fun leavingTheChatTurnsAVoiceMessageInReviewIntoNotSent() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        recording(viewModel, 3_000)
        viewModel.stopRecording()
        val note = viewModel.awaitStaged { it.voiceNote }
        val photo = tempPrepared(tag = 9)
        viewModel.stagePrepared(photo)
        viewModel.beginReply(aQuote)
        viewModel.inputState.setTextAndPlaceCursorAtEnd("for grandma")
        runCurrent()

        viewModel.screenAttached()
        viewModel.screenDetached(changingConfigurations = false)
        runCurrent()

        val entry = viewModel.awaitNotSent().single()
        assertThat(entry.caption).isEqualTo("for grandma")
        assertThat(entry.replyTo).isEqualTo(aQuote)
        assertThat(entry.durationMs).isEqualTo(3_000)
        assertThat(viewModel.inputState.text.toString()).isEmpty()
        assertThat(viewModel.staged.value).containsExactly(photo)
        assertThat(note.file.exists()).isFalse()
        assertThat(parked.file(entry).exists()).isTrue()
    }

    /** The call screen coming over the chat is not leaving it: review is kept (S4's call column). */
    @Test
    fun aCallKeepsAVoiceMessageInReviewWhereItIs() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        recording(viewModel, 3_000)
        viewModel.stopRecording()
        viewModel.awaitStaged { it.voiceNote }
        callState.value = CallState.Incoming(callId = "c1", chatId = CHAT, peerUserId = PEER)
        runCurrent()

        viewModel.screenAttached()
        viewModel.screenStopped(changingConfigurations = false)
        viewModel.screenDetached(changingConfigurations = false)
        advanceUntilIdle()

        assertThat(viewModel.staged.value.single().voiceNote).isTrue()
        assertThat(settledParks()).isEmpty()
    }

    /** The app to the background, the screen locked (ON_STOP): stop and keep (S4). */
    @Test
    fun theScreenStoppingKeepsTheRecording() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        recording(viewModel, 2_500)

        viewModel.screenStopped(changingConfigurations = false)
        runCurrent()

        assertThat(recorder.isRecording).isFalse()
        assertThat(viewModel.awaitNotSent().single().durationMs).isEqualTo(2_500)
    }

    /**
     * A rotation, a fold, a theme change rebuild the activity around the same
     * ViewModel, and the recording carries on (S4) — so long as a screen
     * comes back to show it.
     */
    @Test
    fun aConfigurationChangeIsNotAnInterruption() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        viewModel.screenAttached()
        recording(viewModel, 2_500)

        viewModel.screenStopped(changingConfigurations = true)
        viewModel.screenDetached(changingConfigurations = true)
        viewModel.screenAttached()
        advanceTimeBy(ChatViewModel.ORPHAN_GRACE_MS * 2)
        runCurrent()

        assertThat(recorder.isRecording).isTrue()
        assertThat(recorder.stops).isEqualTo(0)
        assertThat(settledParks()).isEmpty()
    }

    /** A rebuilt activity that never shows this chat again must not leave it recording unseen. */
    @Test
    fun aScreenThatNeverComesBackLetsTheRecordingGo() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        viewModel.screenAttached()
        recording(viewModel, 2_500)

        viewModel.screenDetached(changingConfigurations = true)
        advanceTimeBy(ChatViewModel.ORPHAN_GRACE_MS - 1)
        runCurrent()
        assertThat(recorder.isRecording).isTrue()

        advanceTimeBy(2)
        runCurrent()
        assertThat(recorder.isRecording).isFalse()
        assertThat(viewModel.awaitNotSent().single().durationMs).isEqualTo(2_500)
    }

    /** Five minutes stops into review with the sentence — never a send (S2.5). */
    @Test
    fun theFiveMinuteCapStagesTheRecordingAndSendsNothing() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        recording(viewModel, VoiceNoteRules.VOICE_CAP_MS)

        recorder.end(VoiceRecorder.Ending.CAP)
        val note = viewModel.awaitStaged { it.voiceNote }
        val notice = viewModel.awaitNotice()

        assertThat(viewModel.recordingMs.value).isNull()
        assertThat(note.kind).isEqualTo(AttachmentDto.KIND_AUDIO)
        assertThat(note.durationMs).isEqualTo(VoiceNoteRules.VOICE_CAP_MS.toInt())
        assertThat(note.name).isNull()
        assertThat(notice.text).isEqualTo(app.getString(R.string.s_recording_stopped_at_five_minutes))
        advanceUntilIdle()
        assertThat(attachmentApi.calls).doesNotContain("upload")
        assertThat(db.messageDao().observeMessages(CHAT, 50).first()).isEmpty()
    }

    /** The recorder failing keeps what was readable as "not sent", with the sentence (S4). */
    @Test
    fun aRecorderThatFailsKeepsWhatWasReadable() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        recording(viewModel, 8_000)

        recorder.end(VoiceRecorder.Ending.FAILED)
        runCurrent()

        assertThat(viewModel.awaitNotSent().single().durationMs).isEqualTo(8_000)
        assertThat(failure(viewModel)).isEqualTo(app.getString(R.string.e_recording_stopped_unexpectedly))
    }

    /** …and says so when nothing could be read back at all. */
    @Test
    fun aRecorderThatFailsWithNothingReadableStillSaysSo() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        recording(viewModel, 8_000)

        recorder.end(VoiceRecorder.Ending.FAILED, keep = false)
        advanceUntilIdle()

        assertThat(viewModel.recordingMs.value).isNull()
        assertThat(settledParks()).isEmpty()
        assertThat(failure(viewModel)).isEqualTo(app.getString(R.string.e_recording_stopped_unexpectedly))
    }

    /** An alarm, a phone call, the assistant — something else has the audio: stop and keep. */
    @Test
    fun anotherAppTakingTheAudioKeepsTheRecording() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        recording(viewModel, 1_500)

        recorder.end(VoiceRecorder.Ending.INTERRUPTED)
        runCurrent()

        assertThat(viewModel.awaitNotSent().single().durationMs).isEqualTo(1_500)
        assertThat(viewModel.mediaState.value).isEqualTo(ChatViewModel.MediaSendState.Idle)
    }

    /** One recording at a time in the whole app (S1.7): another chat starting one parks this one. */
    @Test
    fun anotherChatStartingARecordingKeepsThisOne() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        recording(viewModel, 2_000)

        recorder.end(VoiceRecorder.Ending.SUPERSEDED)
        runCurrent()

        assertThat(viewModel.recordingMs.value).isNull()
        assertThat(viewModel.awaitNotSent().single().durationMs).isEqualTo(2_000)
    }

    /** A recording nobody has finished deciding about is sent or deleted before another starts. */
    @Test
    fun recordingIsRefusedWhileANotSentMessageWaits() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        parkedNote(4_000)
        viewModel.awaitNotSent()

        viewModel.startRecording()
        runCurrent()

        assertThat(recorder.starts).isEqualTo(0)
        assertThat(notice(viewModel)).isEqualTo(app.getString(R.string.e_send_or_delete_the_unsent_first))
    }

    /** The floor people see is a second (Decision 15): a Stop under it keeps nothing. */
    @Test
    fun aStopUnderASecondIsTooShort() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        recording(viewModel, 999)

        viewModel.stopRecording()
        advanceUntilIdle()

        assertThat(viewModel.staged.value).isEmpty()
        assertThat(recorder.files.single().exists()).isFalse()
        assertThat(failure(viewModel)).isEqualTo(app.getString(R.string.e_recording_too_short))
    }

    /**
     * Its Send sends it with THAT reply and caption, and nothing else (S2.8):
     * the composer's own words, its own primed reply and its staged photo
     * stay where they are. The waiting copy goes once the outbox has it.
     */
    @Test
    fun aNotSentMessageGoesWithItsOwnReplyAndCaptionAndNothingElse() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        val id = parkedNote(12_000, replyTo = aQuote, caption = "for grandma")
        val entry = viewModel.awaitNotSent().single()
        val photo = tempPrepared(tag = 4)
        viewModel.stagePrepared(photo)
        viewModel.beginReply(anotherQuote)
        viewModel.inputState.setTextAndPlaceCursorAtEnd("hello")
        runCurrent()

        viewModel.sendNotSent(id)
        val row = db.messageDao().observeMessages(CHAT, 50)
            .first { rows -> rows.any { it.attachmentKind == AttachmentDto.KIND_AUDIO } }
            .single()
        viewModel.notSent.first { it.isEmpty() }

        assertThat(row.body).isEqualTo("for grandma")
        assertThat(row.replyToMessageId).isEqualTo(aQuote.messageId)
        assertThat(row.attachmentName).isNull()
        assertThat(viewModel.inputState.text.toString()).isEqualTo("hello")
        assertThat(viewModel.replyDraft.value).isEqualTo(anotherQuote)
        assertThat(viewModel.staged.value).containsExactly(photo)
        assertThat(parked.file(entry).exists()).isFalse()
    }

    /**
     * While the composer is busy with another attachment its strip is that
     * one's live progress and its busy guard: a not-sent Send waits, and goes
     * once the strip is free.
     */
    @Test
    fun aNotSentSendWaitsWhileTheComposerIsBusy() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        val id = parkedNote(5_000)
        viewModel.awaitNotSent()
        viewModel.reportAttachmentBusy("Saving…")

        viewModel.sendNotSent(id)
        advanceUntilIdle()

        assertThat(db.messageDao().observeMessages(CHAT, 50).first()).isEmpty()
        assertThat(settledParks().map { it.id }).containsExactly(id)
        assertThat(viewModel.mediaState.value).isEqualTo(ChatViewModel.MediaSendState.Working("Saving…"))

        viewModel.clearMediaState()
        viewModel.sendNotSent(id)
        db.messageDao().observeMessages(CHAT, 50).first { it.isNotEmpty() }
        viewModel.notSent.first { it.isEmpty() }
    }

    /**
     * In the member's own `ai` chat a voice message reaches the model like
     * anything else, so its Send asks first — and agreeing sends IT, not the
     * draft sitting in the box.
     */
    @Test
    fun aNotSentMessageInTheAssistantsChatAsksFirstThenSendsOnlyItself() = recordingTest {
        launch {
            settings.setAssistant(userId = 1L, displayName = "Assistant", processor = "OpenAI")
        }
        runCurrent()
        val viewModel = newViewModel(kind = "ai")
        runCurrent()
        val id = parkedNote(3_000)
        viewModel.awaitNotSent()
        viewModel.inputState.setTextAndPlaceCursorAtEnd("a draft nobody sent")

        viewModel.sendNotSent(id)
        runCurrent()
        assertThat(viewModel.assistantConsentAsk.value).isNotNull()
        assertThat(db.messageDao().observeMessages(CHAT, 50).first()).isEmpty()

        viewModel.agreeToTheAssistant()
        val rows = db.messageDao().observeMessages(CHAT, 50).first { it.isNotEmpty() }
        viewModel.notSent.first { it.isEmpty() }

        assertThat(rows.single().attachmentKind).isEqualTo(AttachmentDto.KIND_AUDIO)
        assertThat(viewModel.inputState.text.toString()).isEqualTo("a draft nobody sent")
    }

    /** Its ✕ under ten seconds deletes at once (S2.8). */
    @Test
    fun deletingAShortNotSentMessageDeletesItAtOnce() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        val id = parkedNote(9_999)
        val entry = viewModel.awaitNotSent().single()

        viewModel.deleteNotSent(id)
        viewModel.notSent.first { it.isEmpty() }

        assertThat(viewModel.deleteAsk.value).isNull()
        assertThat(parked.file(entry).exists()).isFalse()
    }

    /** Ten seconds or more asks "Delete this recording?" — and Keep keeps it. */
    @Test
    fun deletingALongNotSentMessageAsksFirst() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        val id = parkedNote(10_000)
        viewModel.awaitNotSent()

        viewModel.deleteNotSent(id)
        assertThat(viewModel.deleteAsk.value).isEqualTo(id)
        viewModel.answerDeleteAsk(delete = false)
        advanceUntilIdle()
        assertThat(viewModel.deleteAsk.value).isNull()
        assertThat(settledParks().map { it.id }).containsExactly(id)

        viewModel.deleteNotSent(id)
        viewModel.answerDeleteAsk(delete = true)
        viewModel.notSent.first { it.isEmpty() }
        assertThat(settledParks()).isEmpty()
    }

    /** Deleting a recording never deletes words somebody typed: its caption comes back. */
    @Test
    fun deletingANotSentMessageGivesItsWordsBack() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        val id = parkedNote(2_000, replyTo = aQuote, caption = "for grandma")
        viewModel.awaitNotSent()

        viewModel.deleteNotSent(id)
        viewModel.notSent.first { it.isEmpty() }

        assertThat(viewModel.inputState.text.toString()).isEqualTo("for grandma")
        assertThat(viewModel.replyDraft.value).isEqualTo(aQuote)
    }

    // -- Voice in the Send slot (#79, Phase 1) ----------------------------------
    //
    // docs/audio-video-messages-2026-10-04.md, S1-S2, S6, S7, S9: the shared
    // reducer (RecordGesture, held to the reference's vectors elsewhere) as
    // this ViewModel feeds it and carries it out — a tap records hands-free
    // and the same slot sends; a hold is a walkie-talkie whose release opens
    // a five-second Undo window, written to the parked store marked "sending"
    // first; the first release on a device, Review Before Sending, TalkBack
    // and silence all go to review; the 600 ms guard; the one-second floor.
    // Every timer runs on the test scheduler's clock (runCurrent, never
    // advanceUntilIdle, while something records).

    /** Where the microphone is, in window coordinates. */
    private val micX = 340.0
    private val micY = 780.0

    /** A finger, a granted microphone, no screen reader, Android's default 400 ms long press. */
    private val finger = ChatViewModel.VoiceEnvironment(systemLongPressMs = 400)

    /** H for [finger]: max(500, 400). */
    private val hMs = ComposerSlot.holdThresholdMs(400)

    /** A finger taps the microphone: down, and up inside before H. */
    private fun TestScope.tap(
        viewModel: ChatViewModel,
        env: ChatViewModel.VoiceEnvironment = finger,
        canHold: Boolean = true,
    ) {
        viewModel.micDown(micX, micY, canHold, rtl = false, env = env)
        advanceTimeBy(120)
        runCurrent()
        viewModel.micUp(micX, micY, inside = true)
        runCurrent()
    }

    /** A finger held on the microphone until H: recording starts there. */
    private fun TestScope.hold(viewModel: ChatViewModel, env: ChatViewModel.VoiceEnvironment = finger) {
        viewModel.micDown(micX, micY, canHold = true, rtl = false, env = env)
        advanceTimeBy(hMs)
        runCurrent()
    }

    /** The held finger lets go after [recordedMs] of recording, where it is. */
    private fun TestScope.letGo(viewModel: ChatViewModel, recordedMs: Long, x: Double = micX, y: Double = micY) {
        recorder.elapsed = recordedMs
        viewModel.micUp(x, y, inside = true)
        runCurrent()
    }

    /** Past the slot's 600 ms activation guard. */
    private fun TestScope.pastTheGuard() {
        advanceTimeBy(ComposerSlot.ACTIVATION_GUARD_MS)
        runCurrent()
    }

    /** This device has had its first held release taught: the next one may open the Undo window. */
    private fun TestScope.taught() {
        launch { settings.setHeldReleaseTaught() }
        runCurrent()
    }

    /** Everything the ViewModel asks the screen to do, collected from now. */
    private fun TestScope.screenEffects(viewModel: ChatViewModel): List<ChatViewModel.VoiceEffect> {
        val seen = mutableListOf<ChatViewModel.VoiceEffect>()
        backgroundScope.launch { viewModel.voiceEffects.collect { seen += it } }
        runCurrent()
        return seen
    }

    private fun haptics(effects: List<ChatViewModel.VoiceEffect>) =
        effects.filterIsInstance<ChatViewModel.VoiceEffect.Haptic>().map { it.haptic }

    private suspend fun rows() = db.messageDao().observeMessages(CHAT, 50).first()

    /**
     * The store's entries once [count] have landed — waited for on the wall
     * clock, WITHOUT suspending: runTest moves virtual time to the next timer
     * whenever the test body suspends on real I/O, and a pending Undo window
     * would then run out under the very assertion that it has not.
     */
    private fun TestScope.parkedWithoutTime(count: Int): List<ParkedRecording> {
        runCurrent()
        val deadline = System.currentTimeMillis() + 5_000
        while (settings.current.parkedRecordings.size < count && System.currentTimeMillis() < deadline) {
            Thread.sleep(5)
        }
        runCurrent()
        return settings.current.parkedRecordings
    }

    private suspend fun awaitAudioRow() = db.messageDao().observeMessages(CHAT, 50)
        .first { rows -> rows.any { it.attachmentKind == AttachmentDto.KIND_AUDIO } }
        .single { it.attachmentKind == AttachmentDto.KIND_AUDIO }

    /** A tap records hands-free (S2.2) and the same slot, now an arrow, sends it (S2.5). */
    @Test
    fun aTapRecordsHandsFreeAndTheSameSlotSendsIt() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        val effects = screenEffects(viewModel)
        viewModel.beginReply(aQuote)

        tap(viewModel)

        assertThat(recorder.starts).isEqualTo(1)
        assertThat(viewModel.hold.value.recording).isEqualTo(ComposerSlot.Recording.HANDS_FREE)
        assertThat(viewModel.announcement.value?.text).isEqualTo(app.getString(R.string.s_announce_recording))
        assertThat(effects).contains(ChatViewModel.VoiceEffect.FocusSlot)
        assertThat(haptics(effects)).containsExactly(RecordGesture.Haptic.LIGHT)

        pastTheGuard()
        recorder.elapsed = 4_200
        viewModel.activateSlot(finger)
        runCurrent()

        val row = awaitAudioRow()
        assertThat(recorder.isRecording).isFalse()
        assertThat(row.body).isEmpty()
        assertThat(row.attachmentName).isNull()
        assertThat(row.replyToMessageId).isEqualTo(aQuote.messageId)
        assertThat(viewModel.replyDraft.value).isNull()
        assertThat(viewModel.hold.value.recording).isEqualTo(ComposerSlot.Recording.NONE)
        // Said once the outbox has it (S2.5), which is a pass after the row.
        realTimeUntil { viewModel.announcement.value?.text == app.getString(R.string.s_announce_voice_message_sent) }
        assertThat(viewModel.announcement.value?.text).isEqualTo(app.getString(R.string.s_announce_voice_message_sent))
        assertThat(haptics(effects)).containsExactly(RecordGesture.Haptic.LIGHT, RecordGesture.Haptic.SUCCESS).inOrder()
        assertThat(effects).contains(ChatViewModel.VoiceEffect.Ended)
    }

    /** A double tap on the microphone cannot send what it started (S1.1's guard). */
    @Test
    fun aSecondTapInsideTheGuardDoesNothing() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        tap(viewModel)
        recorder.elapsed = 1_500

        viewModel.activateSlot(finger)
        viewModel.micDown(micX, micY, canHold = true, rtl = false, env = finger)
        viewModel.micUp(micX, micY, inside = true)
        runCurrent()

        assertThat(recorder.isRecording).isTrue()
        assertThat(rows()).isEmpty()
    }

    /**
     * A double tap on Send cannot start a recording: the slot's own Send
     * guards the microphone it turns into — and a word typed and sent at once
     * is never slowed (S1.1).
     */
    @Test
    fun aDoubleTapOnSendCannotStartARecording() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        type(viewModel, "ok")

        viewModel.sendFromSlot()
        tap(viewModel)

        assertThat(rows().single().body).isEqualTo("ok")
        assertThat(recorder.starts).isEqualTo(0)

        pastTheGuard()
        tap(viewModel)
        assertThat(recorder.starts).isEqualTo(1)
    }

    /**
     * The guard holds back only the slot's own activation: words typed after
     * a Send and sent at once still go, however quickly (S1.1, Decision 15) —
     * while the same, emptied composer pressed again is still ignored.
     */
    @Test
    fun aWordTypedAndSentAtOnceAfterASendIsNeverHeldBack() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        type(viewModel, "ok")
        viewModel.sendFromSlot()
        runCurrent()
        assertThat(viewModel.slotPressIgnored(ComposerSlot.Slot.Send)).isTrue()

        type(viewModel, "hi")
        assertThat(viewModel.slotPressIgnored(ComposerSlot.Slot.Send)).isFalse()
        viewModel.sendFromSlot()
        runCurrent()

        assertThat(rows().map { it.body }).containsExactly("ok", "hi")
        assertThat(recorder.starts).isEqualTo(0)
    }

    /**
     * Something staged after a Send is the person's own change: never
     * guarded (S1.1) — the Send right after it sends it, though the field is
     * as empty as the Send left it.
     */
    @Test
    fun somethingStagedAfterASendIsNeverHeldBack() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        type(viewModel, "ok")
        viewModel.sendFromSlot()
        runCurrent()
        assertThat(viewModel.slotPressIgnored(ComposerSlot.Slot.Send)).isTrue()

        viewModel.stagePrepared(tempPrepared(tag = 1))
        runCurrent()
        assertThat(viewModel.slotPressIgnored(ComposerSlot.Slot.Send)).isFalse()
        viewModel.sendFromSlot()
        runCurrent()

        assertThat(viewModel.staged.value).isEmpty()
    }

    /**
     * The same through each way the app really stages — the photo picker,
     * the file picker, a paste: each is the person's own change (S1.1).
     */
    @Test
    fun aPickedPhotoAfterASendIsNeverHeldBack() = recordingTest {
        stagedAfterASendGoes { it.stageMedia(clipboardItem("photo", ONE_PIXEL_PNG), isVideo = false) }
    }

    @Test
    fun aPickedFileAfterASendIsNeverHeldBack() = recordingTest {
        stagedAfterASendGoes { it.stageFile(clipboardItem("file.pdf")) }
    }

    @Test
    fun aPastedItemAfterASendIsNeverHeldBack() = recordingTest {
        stagedAfterASendGoes { it.pasteAttachment(clipboardItem("blob"), "application/pdf") }
    }

    /** Send "ok", stage through [how] inside the guard, and find the Send not held back. */
    private suspend fun TestScope.stagedAfterASendGoes(how: (ChatViewModel) -> Unit) {
        val viewModel = newViewModel()
        runCurrent()
        type(viewModel, "ok")
        viewModel.sendFromSlot()
        runCurrent()
        val guardEnds = viewModel.hold.value.guardUntilMs
        assertThat(viewModel.slotPressIgnored(ComposerSlot.Slot.Send)).isTrue()

        how(viewModel)
        // The preparation runs on Dispatchers.IO and comes back to the test's
        // dispatcher; waiting in real time, each pass running what is due,
        // keeps virtual time where the Send left it (a suspending wait would
        // let it jump past the guard).
        val deadline = System.currentTimeMillis() + 10_000
        while (viewModel.staged.value.isEmpty() && System.currentTimeMillis() < deadline) {
            Thread.sleep(5)
            runCurrent()
        }
        runCurrent()
        assertThat(viewModel.staged.value).hasSize(1)
        // Still inside the 600 ms the Send set, had nothing lifted it.
        assertThat(testScheduler.currentTime).isLessThan(guardEnds)
        assertThat(viewModel.hold.value.guardUntilMs).isEqualTo(0)
        assertThat(viewModel.slotPressIgnored(ComposerSlot.Slot.Send)).isFalse()
    }

    /**
     * The field the slot's own Send emptied is not the person's change: when
     * that emptying reaches the field's watcher — a frame later, as the
     * Recomposer applies the snapshot — the guard stays, and a double tap on
     * Send still cannot start a recording (S1.1).
     */
    @Test
    fun theFieldTheSendEmptiedLiftsNothing() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        type(viewModel, "ok")
        viewModel.sendFromSlot()
        Snapshot.sendApplyNotifications()
        runCurrent()

        tap(viewModel)

        assertThat(rows().single().body).isEqualTo("ok")
        assertThat(recorder.starts).isEqualTo(0)
    }

    /**
     * Words typed and deleted after a Send lift the guard (S1.1): the
     * microphone the emptied composer shows records at once — a decision of
     * its own, not the second half of a double tap.
     */
    @Test
    fun wordsTypedAndDeletedAfterASendLetTheMicrophoneRecord() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        type(viewModel, "ok")
        viewModel.sendFromSlot()
        runCurrent()

        type(viewModel, "x")
        type(viewModel, "")
        tap(viewModel)

        assertThat(rows().single().body).isEqualTo("ok")
        assertThat(recorder.starts).isEqualTo(1)
    }

    /**
     * Something taken off the strip after a Stop staged a note is the
     * person's change too: the guard the Stop set lifts, and Send sends (S1.1).
     */
    @Test
    fun takingSomethingOffAfterAStopLiftsTheGuard() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        viewModel.stagePrepared(tempPrepared(tag = 1))
        runCurrent()
        viewModel.recordVoiceMessage(finger)
        runCurrent()
        assertThat(viewModel.hold.value.recording).isEqualTo(ComposerSlot.Recording.HANDS_FREE_BESIDE_DRAFT)
        pastTheGuard()
        recorder.elapsed = 3_000
        viewModel.activateSlot(finger)
        runCurrent()
        assertThat(viewModel.hold.value.phase).isEqualTo(RecordGesture.Phase.Idle)
        assertThat(viewModel.slotPressIgnored(ComposerSlot.Slot.Send)).isTrue()

        // Inside the Stop's 600 ms (virtual time has not moved).
        viewModel.discardStaged(viewModel.staged.value.indexOfFirst { !it.voiceNote })
        runCurrent()
        assertThat(viewModel.slotPressIgnored(ComposerSlot.Slot.Send)).isFalse()
        viewModel.awaitStaged { it.voiceNote }
        viewModel.sendFromSlot()
        awaitAudioRow()
    }

    /**
     * "Delete this recording?" answered Delete about a staged note of ten
     * seconds or more takes it off the strip: the person's change, which
     * lifts the guard the Stop that staged it set (S1.1).
     */
    @Test
    fun deletingALongStagedNoteAfterAStopLiftsTheGuard() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        viewModel.stagePrepared(tempPrepared(tag = 1))
        runCurrent()
        viewModel.recordVoiceMessage(finger)
        runCurrent()
        pastTheGuard()
        recorder.elapsed = 12_000
        viewModel.activateSlot(finger)
        runCurrent()
        val guardEnds = viewModel.hold.value.guardUntilMs
        // Real time, not a suspending wait: virtual time stays inside the guard.
        val deadline = System.currentTimeMillis() + 10_000
        while (viewModel.staged.value.none { it.voiceNote } && System.currentTimeMillis() < deadline) {
            Thread.sleep(5)
            runCurrent()
        }
        assertThat(viewModel.slotPressIgnored(ComposerSlot.Slot.Send)).isTrue()

        viewModel.discardStaged(viewModel.staged.value.indexOfFirst { it.voiceNote })
        runCurrent()
        assertThat(viewModel.slotPressIgnored(ComposerSlot.Slot.Send)).isTrue()
        viewModel.answerStagedDelete(delete = true)
        runCurrent()

        assertThat(testScheduler.currentTime).isLessThan(guardEnds)
        assertThat(viewModel.staged.value.none { it.voiceNote }).isTrue()
        assertThat(viewModel.slotPressIgnored(ComposerSlot.Slot.Send)).isFalse()
    }

    /** Something staged while a released note waits ends the window — by SENDING (S2.6). */
    @Test
    fun somethingStagedDuringTheUndoWindowSendsAtOnce() = recordingTest {
        taught()
        val viewModel = newViewModel()
        runCurrent()
        hold(viewModel)
        letGo(viewModel, recordedMs = 2_000)
        assertThat(viewModel.hold.value.undo).isNotNull()

        viewModel.stagePrepared(tempPrepared(tag = 1))
        runCurrent()
        // Ended by the staging itself — not by the five seconds running out.
        assertThat(viewModel.hold.value.undo).isNull()

        awaitAudioRow()
    }

    /** Recording never starts on touch-down: at H, and only there (S2.3, Decision 6). */
    @Test
    fun holdingRecordsAtTheHoldThresholdAndNotBefore() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        val effects = screenEffects(viewModel)

        viewModel.micDown(micX, micY, canHold = true, rtl = false, env = finger)
        advanceTimeBy(hMs - 1)
        runCurrent()
        assertThat(recorder.starts).isEqualTo(0)

        advanceTimeBy(1)
        runCurrent()
        assertThat(recorder.starts).isEqualTo(1)
        assertThat(viewModel.hold.value.recording).isEqualTo(ComposerSlot.Recording.HELD)
        assertThat(haptics(effects)).containsExactly(RecordGesture.Haptic.MEDIUM)
        // The keyboard, if it was up, stays up under the hold row (S2.3).
        assertThat(effects).doesNotContain(ChatViewModel.VoiceEffect.FocusSlot)
    }

    /** H follows the person's "Touch & hold delay" — never shorter than theirs (S1.1). */
    @Test
    fun aLongerTouchAndHoldDelayIsALongerHold() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        val slow = finger.copy(systemLongPressMs = 1_000)

        viewModel.micDown(micX, micY, canHold = true, rtl = false, env = slow)
        advanceTimeBy(999)
        runCurrent()
        assertThat(recorder.starts).isEqualTo(0)
        advanceTimeBy(1)
        runCurrent()
        assertThat(recorder.starts).isEqualTo(1)
    }

    /** A mouse clicks on release, whatever the length — never a hold (S8.4). */
    @Test
    fun aMousePressOfAnyLengthIsAClick() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()

        viewModel.micDown(micX, micY, canHold = false, rtl = false, env = finger)
        advanceTimeBy(3_000)
        runCurrent()
        assertThat(recorder.starts).isEqualTo(0)

        viewModel.micUp(micX, micY, inside = true)
        runCurrent()
        assertThat(viewModel.hold.value.recording).isEqualTo(ComposerSlot.Recording.HANDS_FREE)
    }

    /**
     * The first held release on a device goes to review, with the one line
     * that says what letting go does — and remembers it was taught (S2.3, S7.3).
     */
    @Test
    fun theFirstHeldReleaseOnTheDeviceGoesToReviewAndTeaches() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        hold(viewModel)

        letGo(viewModel, recordedMs = 2_400)

        val note = viewModel.awaitStaged { it.voiceNote }
        assertThat(note.durationMs).isEqualTo(2_400)
        assertThat(viewModel.awaitNotice().text).isEqualTo(app.getString(R.string.s_next_time_letting_go_sends))
        assertThat(settings.current.heldReleaseTaught).isTrue()
        assertThat(rows()).isEmpty()
        assertThat(viewModel.announcement.value?.text)
            .isEqualTo(app.getString(R.string.s_announce_ready_to_review, "0:02"))
    }

    /**
     * Every other release waits five seconds with an Undo before anything is
     * uploaded (S2.6): written to the parked store marked "sending" at the
     * release — not a not-sent row — and handed to the outbox when the
     * window runs out, the entry going with the hand-off.
     */
    @Test
    fun aHeldReleaseWaitsFiveSecondsThenSends() = recordingTest {
        taught()
        val viewModel = newViewModel()
        runCurrent()
        viewModel.beginReply(aQuote)
        hold(viewModel)

        letGo(viewModel, recordedMs = 3_000)

        val waiting = parkedWithoutTime(1).single()
        assertThat(waiting.sending).isTrue()
        assertThat(waiting.replyTo).isEqualTo(aQuote)
        assertThat(viewModel.notSent.value).isEmpty()
        assertThat(viewModel.hold.value.undo?.recordedMs).isEqualTo(3_000)
        assertThat(viewModel.replyDraft.value).isNull()

        advanceTimeBy(ComposerSlot.UNDO_WINDOW_MS - 1)
        runCurrent()
        assertThat(rows()).isEmpty()

        advanceTimeBy(1)
        runCurrent()
        // The window's own timer ended it at five seconds — asked before
        // anything suspends, since a suspended test body lets virtual time
        // run on to whatever timer comes next.
        assertThat(viewModel.hold.value.undo).isNull()
        val row = awaitAudioRow()
        // Said once the outbox has it — never while the hand-off could still fail.
        realTimeUntil { viewModel.announcement.value?.text == app.getString(R.string.s_announce_voice_message_sent) }
        assertThat(viewModel.announcement.value?.text).isEqualTo(app.getString(R.string.s_announce_voice_message_sent))
        assertThat(row.replyToMessageId).isEqualTo(aQuote.messageId)
        settings.state.first { it.parkedRecordings.isEmpty() }
        assertThat(viewModel.hold.value.undo).isNull()
    }

    /**
     * A note whose window ran out but which cannot be prepared is never
     * lost and never left hidden as "sending": it becomes "not sent" (S2.5,
     * S2.6), and nothing is sent.
     */
    @Test
    fun aReleasedNoteThatCannotBePreparedBecomesNotSent() = recordingTest {
        taught()
        val viewModel = newViewModel()
        runCurrent()
        hold(viewModel)
        letGo(viewModel, recordedMs = 3_000)
        val waiting = parkedWithoutTime(1).single()
        assertThat(waiting.sending).isTrue()
        // Its bytes become unreadable before the window runs out.
        parked.file(waiting).delete()

        advanceTimeBy(ComposerSlot.UNDO_WINDOW_MS)
        runCurrent()
        val deadline = System.currentTimeMillis() + 5_000
        // The preparation runs on Dispatchers.IO and comes back to the test's
        // dispatcher, so each pass also runs what is due — without moving
        // virtual time.
        while (settings.current.parkedRecordings.any { it.sending } && System.currentTimeMillis() < deadline) {
            Thread.sleep(5)
            runCurrent()
        }
        runCurrent()

        assertThat(settings.current.parkedRecordings.single().sending).isFalse()
        assertThat(viewModel.mediaState.value)
            .isEqualTo(ChatViewModel.MediaSendState.Failed(app.getString(R.string.e_prepare_failed)))
        assertThat(rows()).isEmpty()
    }

    /**
     * [dir] made unusable for [body] — a plain file where the directory goes,
     * so nothing can be written under it — and given back after.
     */
    private inline fun <T> blocking(dir: File, body: () -> T): T {
        dir.deleteRecursively()
        dir.writeText("blocked")
        try {
            return body()
        } finally {
            dir.delete()
            dir.mkdirs()
        }
    }

    /** MediaPrep's staging directory: blocked, every preparation fails. */
    private val uploadsDir: File get() = File(RuntimeEnvironment.getApplication().cacheDir, "uploads")

    /** The outbox's: blocked, the outbox refuses every hand-off. */
    private val outboxDir: File get() = File(RuntimeEnvironment.getApplication().filesDir, "outbox")

    /** Real time, each pass running what is due, until [done] — or five seconds. */
    private fun TestScope.realTimeUntil(done: () -> Boolean) {
        val deadline = System.currentTimeMillis() + 5_000
        while (!done() && System.currentTimeMillis() < deadline) {
            Thread.sleep(5)
            runCurrent()
        }
        runCurrent()
    }

    /** The Send arrow's note recorded hands-free, with a reply primed. */
    private fun TestScope.sendHandsFree(viewModel: ChatViewModel) {
        viewModel.beginReply(aQuote)
        tap(viewModel)
        pastTheGuard()
        recorder.elapsed = 4_200
        viewModel.activateSlot(finger)
    }

    /** S2.5: a note Send cannot prepare lands in review with the error — never lost. */
    @Test
    fun aSentNoteThatCannotBePreparedLandsInReviewWithTheError() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        val effects = screenEffects(viewModel)
        blocking(uploadsDir) {
            sendHandsFree(viewModel)
            realTimeUntil { viewModel.staged.value.any { it.voiceNote } }
        }
        realTimeUntil { viewModel.announcement.value?.text == app.getString(R.string.e_prepare_failed) }
        // Never "Voice message sent" about a note that went back to review (S2.5, S6).
        assertThat(viewModel.announcement.value?.text).isEqualTo(app.getString(R.string.e_prepare_failed))
        assertThat(haptics(effects)).doesNotContain(RecordGesture.Haptic.SUCCESS)
        assertThat(haptics(effects).last()).isEqualTo(RecordGesture.Haptic.WARNING)

        val note = viewModel.staged.value.single()
        assertThat(note.voiceNote).isTrue()
        assertThat(note.durationMs).isEqualTo(4_200)
        assertThat(note.file.exists()).isTrue()
        assertThat(viewModel.mediaState.value)
            .isEqualTo(ChatViewModel.MediaSendState.Failed(app.getString(R.string.e_prepare_failed)))
        assertThat(viewModel.replyDraft.value).isEqualTo(aQuote)
        assertThat(settings.current.parkedRecordings).isEmpty()
        assertThat(rows()).isEmpty()
    }

    /** S2.5: a note the outbox will not take lands in review with the error, too. */
    @Test
    fun aSentNoteTheOutboxRefusesLandsInReviewWithTheError() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        val effects = screenEffects(viewModel)
        blocking(outboxDir) {
            sendHandsFree(viewModel)
            realTimeUntil { viewModel.staged.value.any { it.voiceNote } }
        }
        realTimeUntil { viewModel.announcement.value?.text == app.getString(R.string.e_send_failed) }
        assertThat(viewModel.announcement.value?.text).isEqualTo(app.getString(R.string.e_send_failed))
        assertThat(haptics(effects)).doesNotContain(RecordGesture.Haptic.SUCCESS)

        val note = viewModel.staged.value.single()
        assertThat(note.voiceNote).isTrue()
        assertThat(note.file.exists()).isTrue()
        assertThat(viewModel.mediaState.value)
            .isEqualTo(ChatViewModel.MediaSendState.Failed(app.getString(R.string.e_send_failed)))
        assertThat(viewModel.replyDraft.value).isEqualTo(aQuote)
        assertThat(settings.current.parkedRecordings).isEmpty()
        assertThat(rows()).isEmpty()
    }

    /**
     * Left before the failure is known: the review it would land in is
     * already "not sent" (S4), so that is where it waits — with its reply.
     */
    @Test
    fun aSentNoteThatFailsAfterTheChatWasLeftIsNotSent() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        blocking(outboxDir) {
            sendHandsFree(viewModel)
            // Before the hand-off has even started (it runs on the next pass).
            viewModel.screenDetached(changingConfigurations = false)
            realTimeUntil { settings.current.parkedRecordings.isNotEmpty() }
        }

        assertThat(viewModel.staged.value).isEmpty()
        val waiting = settings.current.parkedRecordings.single()
        assertThat(waiting.sending).isFalse()
        assertThat(waiting.replyTo).isEqualTo(aQuote)
    }

    /** The Undo window's send is Send's (S2.6): one it cannot prepare lands in review. */
    @Test
    fun aReleasedNoteThatCannotBePreparedLandsInReview() = recordingTest {
        taught()
        val viewModel = newViewModel()
        runCurrent()
        viewModel.beginReply(aQuote)
        hold(viewModel)
        letGo(viewModel, recordedMs = 3_000)
        assertThat(parkedWithoutTime(1).single().sending).isTrue()

        blocking(uploadsDir) {
            advanceTimeBy(ComposerSlot.UNDO_WINDOW_MS)
            runCurrent()
            realTimeUntil {
                viewModel.staged.value.any { it.voiceNote } && settings.current.parkedRecordings.isEmpty()
            }
        }

        val note = viewModel.staged.value.single()
        assertThat(note.voiceNote).isTrue()
        assertThat(note.durationMs).isEqualTo(3_000)
        assertThat(note.file.exists()).isTrue()
        assertThat(viewModel.mediaState.value)
            .isEqualTo(ChatViewModel.MediaSendState.Failed(app.getString(R.string.e_prepare_failed)))
        assertThat(viewModel.replyDraft.value).isEqualTo(aQuote)
        assertThat(settings.current.parkedRecordings).isEmpty()
        assertThat(rows()).isEmpty()
    }

    /** ... and one the outbox will not take. */
    @Test
    fun aReleasedNoteTheOutboxRefusesLandsInReview() = recordingTest {
        taught()
        val viewModel = newViewModel()
        runCurrent()
        val effects = screenEffects(viewModel)
        hold(viewModel)
        letGo(viewModel, recordedMs = 3_000)
        assertThat(parkedWithoutTime(1).single().sending).isTrue()

        blocking(outboxDir) {
            advanceTimeBy(ComposerSlot.UNDO_WINDOW_MS)
            runCurrent()
            realTimeUntil {
                viewModel.staged.value.any { it.voiceNote } && settings.current.parkedRecordings.isEmpty()
            }
        }

        assertThat(viewModel.staged.value.single().voiceNote).isTrue()
        assertThat(viewModel.mediaState.value)
            .isEqualTo(ChatViewModel.MediaSendState.Failed(app.getString(R.string.e_send_failed)))
        assertThat(settings.current.parkedRecordings).isEmpty()
        assertThat(rows()).isEmpty()
        // The window's send is Send's (S2.6): what failed is said, never "sent".
        realTimeUntil { viewModel.announcement.value?.text == app.getString(R.string.e_send_failed) }
        assertThat(viewModel.announcement.value?.text).isEqualTo(app.getString(R.string.e_send_failed))
        assertThat(haptics(effects)).doesNotContain(RecordGesture.Haptic.SUCCESS)
    }

    /**
     * A Send whose recording turns out to have kept nothing (the recorder's
     * floor) is "too short" — shown, said and felt — never "Voice message
     * sent" about a note that does not exist; and a Stop's is never "Ready to
     * review" (S2.5, S6).
     */
    @Test
    fun aRecordingThatKeptNothingIsTooShortNeverSentOrReady() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        val effects = screenEffects(viewModel)
        recorder.keepsNothing = true
        tap(viewModel)
        pastTheGuard()
        recorder.elapsed = 4_200
        viewModel.activateSlot(finger)
        runCurrent()

        assertThat(viewModel.announcement.value?.text).isEqualTo(app.getString(R.string.e_recording_too_short))
        assertThat(viewModel.mediaState.value)
            .isEqualTo(ChatViewModel.MediaSendState.Failed(app.getString(R.string.e_recording_too_short)))
        assertThat(haptics(effects)).containsExactly(RecordGesture.Haptic.LIGHT, RecordGesture.Haptic.WARNING).inOrder()
        assertThat(viewModel.staged.value).isEmpty()
        assertThat(rows()).isEmpty()

        pastTheGuard()
        tap(viewModel)
        recorder.elapsed = 42_000
        viewModel.stopRecording()
        runCurrent()
        assertThat(viewModel.announcement.value?.text).isEqualTo(app.getString(R.string.e_recording_too_short))
        assertThat(viewModel.staged.value).isEmpty()
    }

    /**
     * Leaving the chat sends a note in its Undo window (S2.6); if the outbox
     * will not take it, it is "not sent" — never staged into the composer
     * just left.
     */
    @Test
    fun aReleasedNoteLeavingSendsThatTheOutboxRefusesIsNotSent() = recordingTest {
        taught()
        val viewModel = newViewModel()
        runCurrent()
        hold(viewModel)
        letGo(viewModel, recordedMs = 3_000)
        assertThat(parkedWithoutTime(1).single().sending).isTrue()

        blocking(outboxDir) {
            viewModel.screenDetached(changingConfigurations = false)
            runCurrent()
            realTimeUntil { settings.current.parkedRecordings.none { it.sending } }
        }

        assertThat(viewModel.staged.value).isEmpty()
        assertThat(settings.current.parkedRecordings.single().sending).isFalse()
        assertThat(rows()).isEmpty()
    }

    /** Undo: the note goes to review, its reply back in the composer, and nothing is sent (S2.6). */
    @Test
    fun undoTakesTheNoteToReviewAndSendsNothing() = recordingTest {
        taught()
        val viewModel = newViewModel()
        runCurrent()
        viewModel.beginReply(aQuote)
        hold(viewModel)
        letGo(viewModel, recordedMs = 3_000)

        viewModel.undoVoiceMessage()
        assertThat(viewModel.hold.value.undo).isNull()
        val note = viewModel.awaitStaged { it.voiceNote }

        assertThat(note.durationMs).isEqualTo(3_000)
        settings.state.first { it.parkedRecordings.isEmpty() }
        assertThat(viewModel.replyDraft.value).isEqualTo(aQuote)
        advanceTimeBy(ComposerSlot.UNDO_WINDOW_MS * 2)
        runCurrent()
        assertThat(rows()).isEmpty()
        assertThat(viewModel.announcement.value?.text)
            .isEqualTo(app.getString(R.string.s_announce_ready_to_review, "0:03"))
    }

    /** A character typed ends the window early — by SENDING: letting go had decided (S2.6). */
    @Test
    fun typingDuringTheUndoWindowSendsAtOnce() = recordingTest {
        taught()
        val viewModel = newViewModel()
        runCurrent()
        hold(viewModel)
        letGo(viewModel, recordedMs = 2_000)

        type(viewModel, "w")
        // Ended by the action itself — not by the five seconds running out.
        assertThat(viewModel.hold.value.undo).isNull()

        awaitAudioRow()
        assertThat(viewModel.hold.value.undo).isNull()
        assertThat(viewModel.inputState.text.toString()).isEqualTo("w")
    }

    /** The paperclip, a sticker, `@ai` — any other action sends the waiting note now (S2.6). */
    @Test
    fun anotherActionDuringTheUndoWindowSendsAtOnce() = recordingTest {
        taught()
        val viewModel = newViewModel()
        runCurrent()
        hold(viewModel)
        letGo(viewModel, recordedMs = 2_000)

        viewModel.otherAction()
        // Ended by the action itself — not by the five seconds running out.
        assertThat(viewModel.hold.value.undo).isNull()

        awaitAudioRow()
    }

    /** An interruption during the window — a call, leaving the chat — sends it now (S4). */
    @Test
    fun aCallDuringTheUndoWindowSendsIt() = recordingTest {
        taught()
        val viewModel = newViewModel()
        runCurrent()
        hold(viewModel)
        letGo(viewModel, recordedMs = 2_000)

        callState.value = CallState.Incoming(callId = "c1", chatId = CHAT, peerUserId = PEER)
        runCurrent()
        assertThat(viewModel.hold.value.undo).isNull()

        awaitAudioRow()
        settings.state.first { it.parkedRecordings.isEmpty() }
    }

    @Test
    fun leavingTheChatDuringTheUndoWindowSendsIt() = recordingTest {
        taught()
        val viewModel = newViewModel()
        runCurrent()
        viewModel.screenAttached()
        hold(viewModel)
        letGo(viewModel, recordedMs = 2_000)

        viewModel.screenDetached(changingConfigurations = false)
        runCurrent()
        assertThat(viewModel.hold.value.undo).isNull()

        awaitAudioRow()
    }

    /** The microphone itself during the window sends the waiting note, then records (S2.6). */
    @Test
    fun theMicrophoneDuringTheUndoWindowSendsTheWaitingNoteFirst() = recordingTest {
        taught()
        val viewModel = newViewModel()
        runCurrent()
        hold(viewModel)
        letGo(viewModel, recordedMs = 2_000)
        pastTheGuard()

        tap(viewModel)
        assertThat(viewModel.hold.value.undo).isNull()

        awaitAudioRow()
        assertThat(recorder.starts).isEqualTo(2)
        assertThat(viewModel.hold.value.recording).isEqualTo(ComposerSlot.Recording.HANDS_FREE)
    }

    /** Review Before Sending (S9) is the Undo window's "turn off": a held release reviews, untaught. */
    @Test
    fun reviewBeforeSendingKeepsAHeldReleaseForReview() = recordingTest {
        taught()
        launch { settings.setReviewBeforeSending(true) }
        runCurrent()
        val viewModel = newViewModel()
        runCurrent()
        hold(viewModel)

        letGo(viewModel, recordedMs = 2_000)

        viewModel.awaitStaged { it.voiceNote }
        assertThat(viewModel.hold.value.undo).isNull()
        assertThat(settledParks()).isEmpty()
        // "Next time, letting go will send it" would not be true here.
        assertThat(notice(viewModel)).isNull()
    }

    /**
     * Under TalkBack a held release always goes to review, and the
     * microphone opens only once "Recording" has been spoken — a fixed
     * second on Android — so the app's own voice stays out of the note (S6).
     */
    @Test
    fun underTalkBackTheMicrophoneWaitsForItsOwnWordAndReleasesReview() = recordingTest {
        taught()
        val viewModel = newViewModel()
        runCurrent()
        val talkBack = finger.copy(assistive = true)

        tap(viewModel, env = talkBack)
        assertThat(viewModel.announcement.value?.text).isEqualTo(app.getString(R.string.s_announce_recording))
        assertThat(recorder.starts).isEqualTo(0)
        advanceTimeBy(ChatViewModel.SPEECH_LEAD_MS)
        runCurrent()
        assertThat(recorder.starts).isEqualTo(1)
        viewModel.cancelRecording()

        pastTheGuard()
        hold(viewModel, env = talkBack)
        advanceTimeBy(ChatViewModel.SPEECH_LEAD_MS)
        runCurrent()
        letGo(viewModel, recordedMs = 2_000)
        viewModel.awaitStaged { it.voiceNote }
        assertThat(viewModel.hold.value.undo).isNull()
    }

    /** A held recording that never rose above silence is never sent: review, and the line (S2.3). */
    @Test
    fun aSilentHeldReleaseIsNeverSent() = recordingTest {
        taught()
        val viewModel = newViewModel()
        runCurrent()
        recorder.amplitude = 0
        hold(viewModel)

        letGo(viewModel, recordedMs = 2_000)

        viewModel.awaitStaged { it.voiceNote }
        assertThat(viewModel.awaitNotice().text).isEqualTo(app.getString(R.string.s_we_didnt_hear_anything))
        assertThat(viewModel.hold.value.undo).isNull()
        assertThat(rows()).isEmpty()
    }

    /** A hold let go under a second keeps recording, hands-free, and says so for three seconds (S2.3). */
    @Test
    fun aHoldReleasedUnderASecondKeepsRecording() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        hold(viewModel)

        letGo(viewModel, recordedMs = 500)

        assertThat(recorder.isRecording).isTrue()
        assertThat(viewModel.hold.value.recording).isEqualTo(ComposerSlot.Recording.HANDS_FREE)
        assertThat(viewModel.voiceLine.value).isEqualTo(ChatViewModel.VoiceLine.STILL_RECORDING)

        advanceTimeBy(ComposerSlot.STILL_RECORDING_HINT_MS + ChatViewModel.VOICE_TICK_MS)
        runCurrent()
        assertThat(viewModel.voiceLine.value).isNull()
        assertThat(recorder.isRecording).isTrue()
    }

    /** Sliding 100 toward the field arms cancel; letting go then deletes (S2.3). */
    @Test
    fun slidingTowardTheFieldArmsCancelAndLettingGoDeletes() = recordingTest {
        taught()
        val viewModel = newViewModel()
        runCurrent()
        val effects = screenEffects(viewModel)
        hold(viewModel)

        viewModel.micMove(micX - 100, micY)
        runCurrent()
        assertThat((viewModel.hold.value.phase as RecordGesture.Phase.Holding).armed).isTrue()

        letGo(viewModel, recordedMs = 4_000, x = micX - 100)

        assertThat(recorder.isRecording).isFalse()
        assertThat(recorder.files.none { it.exists() }).isTrue()
        assertThat(viewModel.staged.value).isEmpty()
        assertThat(settledParks()).isEmpty()
        assertThat(viewModel.announcement.value?.text).isEqualTo(app.getString(R.string.s_announce_recording_deleted))
        assertThat(haptics(effects)).containsExactly(
            RecordGesture.Haptic.MEDIUM, RecordGesture.Haptic.SELECTION, RecordGesture.Haptic.WARNING,
        ).inOrder()
    }

    /** In a right-to-left layout the leading edge is on the right (S1.1). */
    @Test
    fun inRightToLeftCancelIsTowardTheRight() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        viewModel.micDown(micX, micY, canHold = true, rtl = true, env = finger)
        advanceTimeBy(hMs)
        runCurrent()

        viewModel.micMove(micX - 100, micY)
        runCurrent()
        assertThat((viewModel.hold.value.phase as RecordGesture.Phase.Holding).armed).isFalse()
        viewModel.micMove(micX + 100, micY)
        runCurrent()
        assertThat((viewModel.hold.value.phase as RecordGesture.Phase.Holding).armed).isTrue()
    }

    /** Sliding 60 up locks: hands-free, the keyboard goes down, and the lift does nothing (S2.3). */
    @Test
    fun slidingUpLocksAndTheLiftDoesNothing() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        val effects = screenEffects(viewModel)
        hold(viewModel)

        viewModel.micMove(micX, micY - 60)
        runCurrent()
        assertThat(viewModel.hold.value.recording).isEqualTo(ComposerSlot.Recording.HANDS_FREE)
        assertThat(effects).contains(ChatViewModel.VoiceEffect.FocusSlot)
        assertThat(viewModel.announcement.value?.text).isEqualTo(app.getString(R.string.s_announce_recording_locked))

        letGo(viewModel, recordedMs = 5_000, y = micY - 60)
        assertThat(recorder.isRecording).isTrue()
    }

    /**
     * The system cancelling the touch LOCKS a hold rather than losing it —
     * and, when the app has gone to the background, stops and keeps it (S2.3).
     */
    @Test
    fun aSystemCancelLocksAHoldOrParksItInTheBackground() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        hold(viewModel)
        recorder.elapsed = 2_000

        viewModel.micCancel(background = false)
        runCurrent()
        assertThat(viewModel.hold.value.recording).isEqualTo(ComposerSlot.Recording.HANDS_FREE)
        assertThat(recorder.isRecording).isTrue()
        viewModel.cancelRecording()

        pastTheGuard()
        hold(viewModel)
        recorder.elapsed = 2_500
        viewModel.micCancel(background = true)
        runCurrent()
        assertThat(recorder.isRecording).isFalse()
        assertThat(viewModel.awaitNotSent().single().durationMs).isEqualTo(2_500)
    }

    /** Ctrl+Shift+R records, and pressed during one STOPS it into review — never sends (S1.6). */
    @Test
    fun theShortcutRecordsAndPressedAgainStopsIntoReview() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()

        viewModel.recordVoiceMessage(finger)
        runCurrent()
        assertThat(recorder.isRecording).isTrue()
        recorder.elapsed = 3_000
        viewModel.recordVoiceMessage(finger)
        runCurrent()

        viewModel.awaitStaged { it.voiceNote }
        advanceTimeBy(1_000)
        runCurrent()
        assertThat(rows()).isEmpty()
    }

    /**
     * With words typed, a recording runs BESIDE them: the slot is Stop, the
     * note is staged with the words, and a double tap on Stop does not send
     * what it staged (S1.3 row 3, S1.1).
     */
    @Test
    fun aRecordingBesideWordsStagesBesideThemAndTheirSendIsGuarded() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        type(viewModel, "for grandma")

        viewModel.recordVoiceMessage(finger)
        runCurrent()
        assertThat(viewModel.hold.value.recording).isEqualTo(ComposerSlot.Recording.HANDS_FREE_BESIDE_DRAFT)
        pastTheGuard()
        recorder.elapsed = 3_000
        viewModel.activateSlot(finger)
        // The second tap of a double tap, at once: guarded, so neither the
        // words nor the note it is staging leave.
        viewModel.sendFromSlot()
        runCurrent()
        assertThat(rows()).isEmpty()
        viewModel.awaitStaged { it.voiceNote }
        assertThat(viewModel.inputState.text.toString()).isEqualTo("for grandma")

        pastTheGuard()
        viewModel.sendFromSlot()
        val row = awaitAudioRow()
        assertThat(row.body).isEqualTo("for grandma")
    }

    /** Delete under ten seconds goes at once; from ten it stops FIRST, then asks (S2.5). */
    @Test
    fun deletingTenSecondsOrMoreStopsFirstThenAsks() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        tap(viewModel)
        recorder.elapsed = 9_999
        viewModel.deleteRecording()
        runCurrent()
        assertThat(recorder.isRecording).isFalse()
        assertThat(viewModel.hold.value.phase).isEqualTo(RecordGesture.Phase.Idle)
        assertThat(recorder.files.none { it.exists() }).isTrue()

        pastTheGuard()
        tap(viewModel)
        recorder.elapsed = 12_000
        viewModel.deleteRecording()
        runCurrent()
        assertThat(recorder.isRecording).isFalse()
        assertThat(viewModel.hold.value.phase).isEqualTo(RecordGesture.Phase.AskingDelete(12_000))

        viewModel.answerRecordingDelete(delete = false)
        val kept = viewModel.awaitStaged { it.voiceNote }
        assertThat(kept.durationMs).isEqualTo(12_000)

        pastTheGuard()
        viewModel.discardStaged(0)
        viewModel.answerStagedDelete(delete = true)
        assertThat(viewModel.staged.value).isEmpty()
        tap(viewModel)
        recorder.elapsed = 12_000
        viewModel.deleteRecording()
        viewModel.answerRecordingDelete(delete = true)
        runCurrent()
        assertThat(viewModel.staged.value).isEmpty()
        assertThat(recorder.files.none { it.exists() }).isTrue()
    }

    /** A staged voice note's ✕ asks at ten seconds or more, and Keep keeps it (S2.7). */
    @Test
    fun aLongStagedVoiceNoteAsksBeforeItsCrossDeletesIt() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        val long = tempPrepared(tag = 3).copy(
            kind = AttachmentDto.KIND_AUDIO, mime = "audio/mp4", voiceNote = true, durationMs = 10_000,
        )
        val short = tempPrepared(tag = 4).copy(
            kind = AttachmentDto.KIND_AUDIO, mime = "audio/mp4", voiceNote = true, durationMs = 9_999,
        )
        viewModel.stagePrepared(long)
        viewModel.stagePrepared(short)

        viewModel.discardStaged(1)
        assertThat(viewModel.staged.value).containsExactly(long)
        assertThat(short.file.exists()).isFalse()

        viewModel.discardStaged(0)
        assertThat(viewModel.stagedDeleteAsk.value).isEqualTo(long.file)
        viewModel.answerStagedDelete(delete = false)
        assertThat(viewModel.staged.value).containsExactly(long)
        assertThat(long.file.exists()).isTrue()

        viewModel.discardStaged(0)
        viewModel.answerStagedDelete(delete = true)
        assertThat(viewModel.staged.value).isEmpty()
        assertThat(long.file.exists()).isFalse()
        assertThat(viewModel.stagedDeleteAsk.value).isNull()
    }

    /** Not yet asked: a tap raises the prompt, and Allow records — the tap meant "record" (S2.2). */
    @Test
    fun aTapWithoutPermissionAsksAndAllowRecords() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        val effects = screenEffects(viewModel)

        tap(viewModel, env = finger.copy(permission = RecordGesture.Permission.NOT_ASKED))
        assertThat(effects).contains(ChatViewModel.VoiceEffect.AskPermission)
        assertThat(recorder.starts).isEqualTo(0)

        viewModel.permissionAnswered(granted = true, permanent = false)
        runCurrent()
        assertThat(recorder.starts).isEqualTo(1)
    }

    /** A prompt a hold raised never records, whatever the answer; Allow says "You can record now." (S2.3). */
    @Test
    fun aHoldWithoutPermissionAsksAndNeverRecords() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        val effects = screenEffects(viewModel)

        hold(viewModel, env = finger.copy(permission = RecordGesture.Permission.NOT_ASKED))
        assertThat(effects).contains(ChatViewModel.VoiceEffect.AskPermission)
        viewModel.permissionAnswered(granted = true, permanent = false)
        viewModel.micUp(micX, micY, inside = true)
        runCurrent()

        assertThat(recorder.starts).isEqualTo(0)
        assertThat(notice(viewModel)).isEqualTo(app.getString(R.string.s_you_can_record_now))
    }

    /** Refused: the denial sentence — with Open Settings once the refusal is for good (S2.2). */
    @Test
    fun aRefusalSaysSoAndForGoodOffersOpenSettings() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()

        tap(viewModel, env = finger.copy(permission = RecordGesture.Permission.NOT_ASKED))
        viewModel.permissionAnswered(granted = false, permanent = false)
        runCurrent()
        val once = viewModel.mediaState.value as ChatViewModel.MediaSendState.Failed
        assertThat(once.reason).isEqualTo(app.getString(R.string.e_microphone_permission))
        assertThat(once.opensSettings).isFalse()

        pastTheGuard()
        tap(viewModel, env = finger.copy(permission = RecordGesture.Permission.DENIED))
        val forGood = viewModel.mediaState.value as ChatViewModel.MediaSendState.Failed
        assertThat(forGood.opensSettings).isTrue()
        assertThat(recorder.starts).isEqualTo(0)
    }

    /**
     * A dimmed microphone says why instead of recording (S1.3 rows 7–9) — in
     * the notice line, or, while the strip shows an attachment's progress,
     * as a toast, so the progress is not overwritten.
     */
    @Test
    fun aDimmedMicrophoneSaysWhyInsteadOfRecording() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        val toasts = mutableListOf<String>()
        backgroundScope.launch { viewModel.transientMessages.collect { toasts += it } }
        viewModel.reportAttachmentBusy("Saving…")

        tap(viewModel)

        assertThat(recorder.starts).isEqualTo(0)
        assertThat(toasts).containsExactly(app.getString(R.string.e_wait_for_the_attachment))
        assertThat(viewModel.mediaState.value).isEqualTo(ChatViewModel.MediaSendState.Working("Saving…"))
    }

    /** No recording in the assistant's chat, nor during an edit (S1.3, S1.5). */
    @Test
    fun theAssistantsChatAndAnEditOfferNoRecording() = recordingTest {
        val assistant = newViewModel(kind = "ai")
        runCurrent()
        assistant.recordVoiceMessage(finger)
        runCurrent()
        assertThat(recorder.starts).isEqualTo(0)

        val direct = newViewModel()
        runCurrent()
        direct.beginEdit(messageId = 77, body = "hello")
        direct.recordVoiceMessage(finger)
        runCurrent()
        assertThat(recorder.starts).isEqualTo(0)
    }

    /**
     * The 4:30 warning, "30 seconds left", and the silence warning, "We
     * can't hear anything…", each shown in the level meter's place and said
     * once — and the silence line goes when sound arrives (S2.5, S2.9).
     */
    @Test
    fun theThirtySecondWarningAndTheSilenceWarning() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        recorder.amplitude = 0
        tap(viewModel)

        recorder.elapsed = ComposerSlot.SILENCE_WARNING_AFTER_MS
        advanceTimeBy(ChatViewModel.VOICE_TICK_MS)
        runCurrent()
        assertThat(viewModel.voiceLine.value).isEqualTo(ChatViewModel.VoiceLine.CANT_HEAR)
        assertThat(viewModel.announcement.value?.text).isEqualTo(app.getString(R.string.s_cant_hear_microphone_muted))

        recorder.amplitude = 5_000
        advanceTimeBy(ChatViewModel.VOICE_TICK_MS)
        runCurrent()
        assertThat(viewModel.voiceLine.value).isNull()
        assertThat(viewModel.voiceLevel.value).isGreaterThan(0)

        recorder.elapsed = ComposerSlot.VOICE_WARNING_MS
        advanceTimeBy(ChatViewModel.VOICE_TICK_MS)
        runCurrent()
        assertThat(viewModel.voiceLine.value).isEqualTo(ChatViewModel.VoiceLine.THIRTY_SECONDS_LEFT)
        assertThat(viewModel.announcement.value?.text).isEqualTo(app.getString(R.string.s_thirty_seconds_left))
    }

    /**
     * The coach mark, once per device, after the first hands-free voice
     * message SENT from a touch screen — never after a mouse's (S7.2).
     */
    @Test
    fun theCoachMarkComesOnceAfterTheFirstHandsFreeMessageSentByTouch() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()

        tap(viewModel, canHold = false)
        pastTheGuard()
        recorder.elapsed = 2_000
        viewModel.activateSlot(finger)
        runCurrent()
        assertThat(viewModel.coachMark.value).isFalse()

        pastTheGuard()
        tap(viewModel)
        pastTheGuard()
        recorder.elapsed = 2_000
        viewModel.activateSlot(finger)
        runCurrent()
        assertThat(viewModel.coachMark.value).isTrue()
        assertThat(settings.current.voiceCoachMarkShown).isTrue()

        viewModel.dismissCoachMark()
        pastTheGuard()
        tap(viewModel)
        pastTheGuard()
        recorder.elapsed = 2_000
        viewModel.activateSlot(finger)
        runCurrent()
        assertThat(viewModel.coachMark.value).isFalse()
    }

    /** Never shown while a screen reader runs (S7.2). */
    @Test
    fun theCoachMarkIsNeverShownUnderTalkBack() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        val talkBack = finger.copy(assistive = true)

        tap(viewModel, env = talkBack)
        advanceTimeBy(ChatViewModel.SPEECH_LEAD_MS)
        runCurrent()
        pastTheGuard()
        recorder.elapsed = 2_000
        viewModel.activateSlot(talkBack)
        runCurrent()

        assertThat(viewModel.coachMark.value).isFalse()
        assertThat(settings.current.voiceCoachMarkShown).isFalse()
    }

    /** Stop keeps it for review — "Ready to review, 0:42" — and a playing note says why it waits (S2.5, S1.7). */
    @Test
    fun stopReviewsAndAPlayControlWhileRecordingSaysWhy() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        tap(viewModel)

        viewModel.explainPlaybackWhileRecording()
        assertThat(notice(viewModel)).isEqualTo(app.getString(R.string.s_play_after_recording))

        recorder.elapsed = 42_000
        viewModel.stopRecording()
        viewModel.awaitStaged { it.voiceNote }
        assertThat(viewModel.announcement.value?.text)
            .isEqualTo(app.getString(R.string.s_announce_ready_to_review, "0:42"))
    }

    // -- Video messages (#79, Phase 3) ----------------------------------------------
    //
    // The chat's side of the video recorder: whether it may open from here
    // (S1.3 rows 7–9, S1.4, S1.5), one recording at a time (S1.7), and the
    // reply a sent video message spends (S1.5).

    @Test
    fun theRecorderOpensFromAFamilyOrADirectChat() = recordingTest {
        assertThat(newViewModel(kind = "family").also { runCurrent() }.mayOpenVideoRecorder()).isTrue()
        assertThat(newViewModel(kind = "direct").also { runCurrent() }.mayOpenVideoRecorder()).isTrue()
    }

    @Test
    fun theRecorderNeverOpensFromTheAssistantsChat() = recordingTest {
        val viewModel = newViewModel(kind = "ai")
        runCurrent()
        assertThat(viewModel.mayOpenVideoRecorder()).isFalse()
        assertThat(notice(viewModel)).isNull()
    }

    /** Row 7: dimmed, and it says why (S1.3, S1.4). */
    @Test
    fun theRecorderSaysWhyDuringACall() = recordingTest {
        callState.value = CallState.Incoming(callId = "c1", chatId = CHAT, peerUserId = PEER)
        val viewModel = newViewModel()
        runCurrent()
        assertThat(viewModel.mayOpenVideoRecorder()).isFalse()
        assertThat(notice(viewModel)).isEqualTo(app.getString(R.string.e_record_after_the_call))
    }

    /** Row 9 is about voice: a waiting not-sent voice message does not keep the camera shut. */
    @Test
    fun aNotSentVoiceMessageDoesNotStopTheRecorder() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        parkedNote(4_000)
        viewModel.awaitNotSent()
        assertThat(viewModel.mayOpenVideoRecorder()).isTrue()
    }

    /** One recording at a time (S1.7): a voice recording here is stopped and kept as "not sent". */
    @Test
    fun openingTheRecorderParksAVoiceRecording() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        recording(viewModel, 3_000)

        assertThat(viewModel.mayOpenVideoRecorder()).isTrue()
        runCurrent()

        assertThat(viewModel.recordingMs.value).isNull()
        assertThat(viewModel.awaitNotSent().single().durationMs).isEqualTo(3_000)
    }

    /** The sticker's rule (S1.5): the video carried the reply, so the composer's is spent — not another one. */
    @Test
    fun aSentVideoMessageSpendsTheReplyItCarried() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        viewModel.beginReply(aQuote)
        viewModel.videoMessageSent(anotherQuote.messageId)
        assertThat(viewModel.replyDraft.value).isEqualTo(aQuote)
        viewModel.videoMessageSent(null)
        assertThat(viewModel.replyDraft.value).isEqualTo(aQuote)
        viewModel.videoMessageSent(aQuote.messageId)
        assertThat(viewModel.replyDraft.value).isNull()
    }

    /** The recorder's last words are said in the composer's live region once it has gone (S6). */
    @Test
    fun theRecordersAnnouncementIsSaidHere() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        viewModel.announceFromRecorder(R.string.s_announce_video_message_sent)
        assertThat(viewModel.announcement.value?.text).isEqualTo(app.getString(R.string.s_announce_video_message_sent))
    }

    /** A server's keys are the whole capability check (S1.2). */
    @Test
    fun videoMessagesAreOfferedOnlyAgainstAServerThatHasThem() = recordingTest {
        val viewModel = newViewModel(kind = "family")
        runCurrent()
        assertThat(viewModel.roundVideoOffered.value).isFalse()
        settings.setRoundVideoLimits(60_000, 12_582_912)
        runCurrent()
        assertThat(viewModel.roundVideoOffered.value).isTrue()
    }

    // -- #79 polish: the voice note's waveform (protocol.md, "A voice note's waveform") --

    /** The recorder's waveform goes with the note into review, and with it into "not sent". */
    @Test
    fun theRecordersWaveformGoesIntoReviewAndThenIntoNotSent() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        recorder.waveform = WAVE
        recording(viewModel, 3_000)

        viewModel.stopRecording()
        val note = viewModel.awaitStaged { it.voiceNote }
        assertThat(note.waveform).isEqualTo(WAVE)

        viewModel.screenAttached()
        viewModel.screenDetached(changingConfigurations = false)
        runCurrent()
        assertThat(viewModel.awaitNotSent().single().waveform).isEqualTo(WAVE)
    }

    /** The Send arrow's note uploads the waveform the recorder made of its meter. */
    @Test
    fun aSentVoiceMessageUploadsItsWaveform() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        recorder.waveform = WAVE

        tap(viewModel)
        pastTheGuard()
        recorder.elapsed = 4_200
        viewModel.activateSlot(finger)
        runCurrent()

        assertThat(awaitAudioRow().attachmentList.single().waveform).isEqualTo(WAVE)
        realTimeUntil { attachmentApi.uploadedWaveforms.isNotEmpty() }
        assertThat(attachmentApi.uploadedWaveforms.single()).isEqualTo(WAVE)
    }

    /** A not-sent message's Send uploads the waveform it was parked with. */
    @Test
    fun aNotSentMessageSendsTheWaveformItWasParkedWith() = recordingTest {
        val source = File.createTempFile("voice-", ".m4a", app.cacheDir)
            .apply { writeBytes(FakeVoiceRecorder.M4A_HEAD + ByteArray(4096) { 3 }) }
        val id = requireNotNull(
            parked.park(CHAT, source, 3_000, null, "", epoch.current(), waveform = WAVE),
        ).id
        val viewModel = newViewModel()
        runCurrent()

        viewModel.sendNotSent(id)
        realTimeUntil { attachmentApi.uploadedWaveforms.isNotEmpty() }

        assertThat(attachmentApi.uploadedWaveforms.single()).isEqualTo(WAVE)
    }

    /** The hands-free row's live waveform is the meter's peaks as levels, newest last. */
    @Test
    fun theLiveWaveformScrollsInTheMetersPeaks() = recordingTest {
        val viewModel = newViewModel()
        runCurrent()
        recorder.amplitude = 0
        tap(viewModel)
        advanceTimeBy(ChatViewModel.VOICE_TICK_MS)
        runCurrent()
        recorder.amplitude = Waveform.FULL_SCALE
        advanceTimeBy(ChatViewModel.VOICE_TICK_MS)
        runCurrent()

        val levels = viewModel.voiceLevels.value
        assertThat(levels.last()).isEqualTo(Waveform.MAX_LEVEL)
        assertThat(levels).contains(0)
        assertThat(levels.size).isAtMost(ChatViewModel.LIVE_LEVELS)

        viewModel.cancelRecording()
        assertThat(viewModel.voiceLevels.value).isEmpty()
    }

    /**
     * A voice message's Save (#79): its bytes, downloaded if need be, copied
     * into the document the system's save screen made — and named as a sound
     * file, never "photo-77.jpg".
     */
    @Test
    fun aVoiceMessageSavesIntoTheDocumentThePersonChose() = runTest(dispatcher) {
        val viewModel = newViewModel()
        runCurrent()
        val note = AttachmentDto(id = 77, kind = AttachmentDto.KIND_AUDIO, mime = "audio/mp4", size = 6, durationMs = 3_000)
        assertThat(note.fallbackFileName).isEqualTo("voice-77.m4a")
        attachmentApi.downloadHandler = { _, _, destination ->
            destination.parentFile?.mkdirs()
            destination.writeBytes(byteArrayOf(1, 2, 3, 4, 5, 6))
            ApiResult.Ok(Unit)
        }
        val chosen = File(app.cacheDir, "chosen-${System.nanoTime()}.m4a")

        val saved = viewModel.saveToDocument(note, android.net.Uri.fromFile(chosen))

        assertThat(saved).isTrue()
        assertThat(chosen.readBytes()).isEqualTo(byteArrayOf(1, 2, 3, 4, 5, 6))
        assertThat(viewModel.mediaState.value).isEqualTo(ChatViewModel.MediaSendState.Idle)
    }

    @Test
    fun aVoiceMessageThatCannotBeDownloadedSaysSo() = runTest(dispatcher) {
        val viewModel = newViewModel()
        runCurrent()
        val note = AttachmentDto(id = 78, kind = AttachmentDto.KIND_AUDIO, mime = "audio/mp4", size = 6, durationMs = 3_000)

        val saved = viewModel.saveToDocument(note, android.net.Uri.fromFile(File(app.cacheDir, "never.m4a")))

        assertThat(saved).isFalse()
        assertThat(failure(viewModel)).isEqualTo(app.getString(R.string.e_download_to_save_failed))
    }

    /**
     * The save screen makes the document BEFORE anything is copied, so a Save
     * that cannot go on must take it away again — not leave an empty
     * "voice-78.m4a" where the person chose to save.
     */
    @Test
    fun aVoiceMessageThatCannotBeSavedLeavesNoEmptyDocument() = runTest(dispatcher) {
        val viewModel = newViewModel()
        runCurrent()
        val note = AttachmentDto(id = 78, kind = AttachmentDto.KIND_AUDIO, mime = "audio/mp4", size = 6, durationMs = 3_000)
        val created = File(app.cacheDir, "created-${System.nanoTime()}.m4a").apply { createNewFile() }

        val saved = viewModel.saveToDocument(note, android.net.Uri.fromFile(created))

        assertThat(saved).isFalse()
        assertThat(created.exists()).isFalse()
    }

    @Test
    fun anOrphanedDocumentIsDiscarded() = runTest(dispatcher) {
        val viewModel = newViewModel()
        runCurrent()
        val created = File(app.cacheDir, "orphan-${System.nanoTime()}.m4a").apply { createNewFile() }

        viewModel.discardDocument(android.net.Uri.fromFile(created))

        assertThat(created.exists()).isFalse()
    }
}
