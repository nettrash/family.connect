/*
 * VoiceWaveformSendTest.kt
 * Family Connect (Android)
 *
 * A voice note's WAVEFORM on the send path (#79; docs/protocol.md, "A voice
 * note's waveform"), through the real MessageRepository and the real outbox
 * as RoundSendTest goes: the recorder's 48 digits ride on the staged note
 * into the queued row, so the sender's own bubble draws its shape before a
 * byte has gone up; they go up as `waveform=` with the bytes; a retry from a
 * second process (which knows only the rows) still sends them; and only on
 * audio — a picked photo carrying one by mistake never sends it.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.db.ChatDao
import me.nettrash.familyconnect.data.db.ChatEntity
import me.nettrash.familyconnect.data.db.MessageDao
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.net.dto.AttachmentResponse
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.testutil.FakeAttachmentApi
import me.nettrash.familyconnect.testutil.FakeChatApi
import me.nettrash.familyconnect.testutil.FakeChatSocket
import me.nettrash.familyconnect.testutil.FakePosterCache
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.createTestDb
import me.nettrash.familyconnect.testutil.testChatRepository
import me.nettrash.familyconnect.util.Clock
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import java.io.File

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class VoiceWaveformSendTest {

    private companion object {
        const val ME = 7L
        const val CHAT = 42L
        const val NOW = 1_000_000L
        const val WAVE = "0124689abcddeeedcba987654321001245678aabbba98642"
    }

    private val dispatcher = StandardTestDispatcher()
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private lateinit var db: AppDatabase
    private lateinit var messageDao: MessageDao
    private lateinit var chatDao: ChatDao
    private lateinit var chatApi: FakeChatApi
    private lateinit var attachmentApi: FakeAttachmentApi
    private lateinit var socket: FakeChatSocket
    private lateinit var posters: FakePosterCache

    @Before
    fun setUp() {
        db = createTestDb(dispatcher)
        messageDao = db.messageDao()
        chatDao = db.chatDao()
        chatApi = FakeChatApi()
        attachmentApi = FakeAttachmentApi()
        socket = FakeChatSocket()
        posters = FakePosterCache()
        var nextUpload = 91L
        attachmentApi.uploadHandler = { file, mime, kind ->
            ApiResult.Ok(
                AttachmentResponse(
                    AttachmentDto(id = nextUpload++, kind = kind, mime = mime, size = file.length(), durationMs = 12_000),
                ),
            )
        }
    }

    @After
    fun tearDown() {
        repoScope.cancel()
        db.close()
    }

    private fun TestScope.newMessages(state: SettingsState = SettingsState(myUserId = ME)): MessageRepository {
        val repository = MessageRepository(
            appContext = RuntimeEnvironment.getApplication(),
            chatApi = chatApi,
            attachmentApi = attachmentApi,
            messageDao = messageDao,
            chatDao = chatDao,
            socket = socket,
            settings = FakeSettingsRepository(state),
            chatRepository = testChatRepository(chatApi, chatDao, messageDao, socket, repoScope),
            posterCache = posters,
            scope = repoScope,
            clock = Clock { NOW },
            pendingAttachmentDao = db.pendingAttachmentDao(),
            staging = MediaStaging(RuntimeEnvironment.getApplication()),
        )
        runCurrent()
        return repository
    }

    private suspend fun insertChat() {
        chatDao.upsertAll(
            listOf(
                ChatEntity(
                    id = CHAT, kind = "family", peerUserId = null, title = "The Smiths", unreadCount = 0,
                    myLastReadId = null, peerLastReadId = null, lastMessageBody = null, lastMessageAt = null,
                    lastMessageSenderId = null,
                ),
            ),
        )
    }

    /** A voice note as the composer stages it: the recorder's file, its length and its waveform. */
    private fun voiceNote(waveform: String? = WAVE): MediaPrep.Prepared {
        val file = File.createTempFile(
            "fc-voice", ".m4a",
            File(RuntimeEnvironment.getApplication().cacheDir, "uploads").apply { mkdirs() },
        ).apply { writeBytes(byteArrayOf(1, 2, 3)) }
        return MediaPrep.Prepared(
            file = file, mime = "audio/mp4", kind = AttachmentDto.KIND_AUDIO, width = null, height = null,
            durationMs = 12_000, previewJpeg = null, voiceNote = true, waveform = waveform,
        )
    }

    @Test
    fun `the waveform goes up with the bytes`() = runTest(dispatcher) {
        insertChat()
        val messages = newMessages()

        messages.sendMedia(listOf(voiceNote()), caption = "", chatId = CHAT)
        advanceUntilIdle()

        assertThat(attachmentApi.uploadedMetadata.single().first).isEqualTo("audio")
        assertThat(attachmentApi.uploadedWaveforms.single()).isEqualTo(WAVE)
    }

    @Test
    fun `the sender's own bubble draws its shape before a byte has gone up`() = runTest(dispatcher) {
        insertChat()
        val messages = newMessages()
        attachmentApi.uploadHandler = { _, _, _ -> ApiResult.NetworkError(IllegalStateException("offline")) }

        messages.sendMedia(listOf(voiceNote()), caption = "", chatId = CHAT)
        runCurrent()

        val placeholder = messageDao.pendingSending().single().attachmentList.single()
        assertThat(placeholder.id).isLessThan(0)
        assertThat(placeholder.waveform).isEqualTo(WAVE)
    }

    @Test
    fun `an upload resumed by a second process still carries it`() = runTest(dispatcher) {
        insertChat()
        val ok = attachmentApi.uploadHandler
        attachmentApi.uploadHandler = { _, _, _ -> ApiResult.NetworkError(IllegalStateException("offline")) }
        newMessages().sendMedia(listOf(voiceNote()), caption = "", chatId = CHAT)
        advanceUntilIdle()
        val clientMsgId = messageDao.pendingSending().single().clientMsgId

        // "The process died": a new repository, which knows only the rows.
        attachmentApi.uploadHandler = ok
        newMessages().uploadPending(clientMsgId)
        advanceUntilIdle()

        assertThat(attachmentApi.uploadedWaveforms.last()).isEqualTo(WAVE)
        assertThat(chatApi.postedAttachmentIds.last()).isNotEmpty()
    }

    @Test
    fun `a note with no waveform, and anything that is not audio, send none`() = runTest(dispatcher) {
        insertChat()
        val messages = newMessages()
        val photo = voiceNote().copy(mime = "image/jpeg", kind = AttachmentDto.KIND_PHOTO, voiceNote = false)

        messages.sendMedia(listOf(voiceNote(waveform = null)), caption = "", chatId = CHAT)
        messages.sendMedia(listOf(photo), caption = "", chatId = CHAT)
        advanceUntilIdle()

        assertThat(attachmentApi.uploadedWaveforms).containsExactly(null, null)
    }
}
