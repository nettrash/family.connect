/*
 * MediaUploadWorkerTest.kt
 * Family Connect (Android)
 *
 * The background half of a media send (docs/protocol.md, "Sending on an
 * unreliable network": an upload may be handed to the system and land
 * while the app is not running).
 *
 * What is worth pinning is the worker's ANSWERS, because WorkManager acts
 * on them: `retry` keeps a send alive across process death and reboots,
 * and `success` is what stops a job coming back. Getting the second one
 * wrong on a send that can never go is a job that wakes the phone with
 * backoff for ever.
 *
 * iOS counterpart: ios/FamilyConnectTests/BackgroundUploadsTests.swift
 */

package me.nettrash.familyconnect.data.repo

import androidx.work.ListenableWorker
import androidx.work.WorkerFactory
import androidx.work.WorkerParameters
import androidx.work.testing.TestListenableWorkerBuilder
import androidx.work.workDataOf
import android.content.Context
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
import me.nettrash.familyconnect.data.db.MessageStatus
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.AttachmentResponse
import me.nettrash.familyconnect.data.net.dto.MessageDto
import me.nettrash.familyconnect.data.net.dto.MessageResponse
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
class MediaUploadWorkerTest {

    private companion object {
        const val ME = 7L
        const val CHAT = 42L
        const val NOW = 1_000_000L
    }

    private val dispatcher = StandardTestDispatcher()
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private lateinit var db: AppDatabase
    private lateinit var messageDao: MessageDao
    private lateinit var chatDao: ChatDao
    private lateinit var chatApi: FakeChatApi
    private val attachmentApi = FakeAttachmentApi()
    private val posterCache = FakePosterCache()
    private lateinit var socket: FakeChatSocket
    private lateinit var settings: FakeSettingsRepository

    @Before
    fun setUp() {
        db = createTestDb(dispatcher)
        messageDao = db.messageDao()
        chatDao = db.chatDao()
        chatApi = FakeChatApi()
        socket = FakeChatSocket()
        settings = FakeSettingsRepository(
            SettingsState(
                serverUrl = "https://chat.example.com",
                familyStatus = FamilyStatus.MEMBER,
                myUserId = ME,
            ),
        )
        chatApi.postMessageHandler = { chatId, clientMsgId, body ->
            ApiResult.Ok(
                MessageResponse(
                    MessageDto(
                        id = 900,
                        chatId = chatId,
                        senderId = ME,
                        clientMsgId = clientMsgId,
                        body = body,
                        createdAt = "2026-08-22T09:00:00Z",
                        attachment = FakeAttachmentApi.attachment(hasPreview = true),
                    ),
                ),
            )
        }
    }

    @After
    fun tearDown() {
        repoScope.cancel()
        db.close()
    }

    @Test
    fun `nothing owed is done, not retried`() = runTest(dispatcher) {
        insertChat()
        val repository = newRepository()
        val worker = worker(repository)

        assertThat(worker.doWork()).isEqualTo(ListenableWorker.Result.success())
    }

    @Test
    fun `a send the app could not finish is finished here`() = runTest(dispatcher) {
        insertChat()
        val repository = newRepository(MediaUploadScheduler.None)
        // The app's own leg fails on a network that is down...
        attachmentApi.uploadHandler = { _, _, _ -> ApiResult.NetworkError(RuntimeException("offline")) }
        val clientMsgId = repository.sendMedia(listOf(prepared()), caption = "", chatId = CHAT)!!
        advanceUntilIdle()
        // The row exists and is still on its way; nothing has been posted,
        // because a message claiming the placeholder ids a queued media row
        // carries is the one thing that must never go out.
        assertThat(messageDao.findByClientMsgId(clientMsgId)!!.status)
            .isEqualTo(MessageStatus.SENDING)
        assertThat(chatApi.postedMessages).isEmpty()

        // ...and the job, running later on a network that is back, does
        // exactly what the foreground would have done.
        attachmentApi.uploadHandler = { _, _, _ ->
            ApiResult.Ok(AttachmentResponse(FakeAttachmentApi.attachment(id = 34)))
        }
        assertThat(worker(repository).doWork()).isEqualTo(ListenableWorker.Result.success())
        advanceUntilIdle()

        val row = messageDao.findByClientMsgId(clientMsgId)!!
        assertThat(row.status).isEqualTo(MessageStatus.SENT)
        assertThat(chatApi.postedAttachmentIds).containsExactly(listOf(34L))
    }

    @Test
    fun `a send still owing an upload comes back`() = runTest(dispatcher) {
        insertChat()
        val repository = newRepository(MediaUploadScheduler.None)
        attachmentApi.uploadHandler = { _, _, _ -> ApiResult.NetworkError(RuntimeException("offline")) }
        repository.sendMedia(listOf(prepared()), caption = "", chatId = CHAT)
        advanceUntilIdle()

        // Retry, not failure: the row is still good and the bytes are
        // still here — this is the answer that survives a reboot.
        assertThat(worker(repository).doWork()).isEqualTo(ListenableWorker.Result.retry())
    }

    @Test
    fun `a refused send is never retried again`() = runTest(dispatcher) {
        insertChat()
        val repository = newRepository(MediaUploadScheduler.None)
        // A terminal refusal: these bytes will never be accepted.
        attachmentApi.uploadHandler = { _, _, _ ->
            ApiResult.HttpError(413, "attachment_too_large", "too big")
        }
        val clientMsgId = repository.sendMedia(listOf(prepared()), caption = "", chatId = CHAT)!!
        advanceUntilIdle()
        assertThat(messageDao.findByClientMsgId(clientMsgId)!!.status)
            .isEqualTo(MessageStatus.FAILED)

        // The item rows are still there — they are what a tap-to-retry
        // uses — so a worker that watched only them would wake the phone
        // with backoff for ever over a photo that cannot go.
        assertThat(db.pendingAttachmentDao().sendsOwingUploads()).contains(clientMsgId)
        assertThat(worker(repository).doWork()).isEqualTo(ListenableWorker.Result.success())
    }

    @Test
    fun `sending media asks for the work`() = runTest(dispatcher) {
        insertChat()
        val asked = mutableListOf<String>()
        val repository = newRepository(MediaUploadScheduler { asked += it })
        attachmentApi.uploadHandler = { _, _, _ -> ApiResult.NetworkError(RuntimeException("offline")) }

        val clientMsgId = repository.sendMedia(listOf(prepared()), caption = "", chatId = CHAT)!!
        advanceUntilIdle()

        assertThat(asked).containsExactly(clientMsgId)
    }

    // MARK: - fixtures

    private fun worker(repository: MessageRepository): MediaUploadWorker =
        TestListenableWorkerBuilder<MediaUploadWorker>(
            context = RuntimeEnvironment.getApplication(),
            inputData = workDataOf("client_msg_id" to null as String?),
        ).setWorkerFactory(
            object : WorkerFactory() {
                override fun createWorker(
                    appContext: Context,
                    workerClassName: String,
                    workerParameters: WorkerParameters,
                ): ListenableWorker = MediaUploadWorker(
                    appContext,
                    workerParameters,
                    repository,
                    db.pendingAttachmentDao(),
                )
            },
        ).build()

    private fun TestScope.newRepository(
        uploads: MediaUploadScheduler = MediaUploadScheduler.None,
    ): MessageRepository {
        val repository = MessageRepository(
            appContext = RuntimeEnvironment.getApplication(),
            chatApi = chatApi,
            attachmentApi = attachmentApi,
            messageDao = messageDao,
            chatDao = chatDao,
            socket = socket,
            settings = settings,
            chatRepository = testChatRepository(chatApi, chatDao, messageDao, socket, repoScope),
            posterCache = posterCache,
            scope = repoScope,
            clock = Clock { NOW },
            pendingAttachmentDao = db.pendingAttachmentDao(),
            staging = MediaStaging(RuntimeEnvironment.getApplication()),
            uploads = uploads,
        )
        runCurrent()
        return repository
    }

    private fun prepared(): MediaPrep.Prepared {
        val file = File.createTempFile(
            "fc-upload",
            ".jpg",
            File(RuntimeEnvironment.getApplication().cacheDir, "uploads").apply { mkdirs() },
        )
        file.writeBytes(byteArrayOf(0xFF.toByte(), 0xD8.toByte(), 0xFF.toByte(), 0xE0.toByte()))
        return MediaPrep.Prepared(
            file = file,
            mime = "image/jpeg",
            kind = "photo",
            width = 1600,
            height = 1200,
            durationMs = null,
            previewJpeg = ByteArray(64) { 0x7 },
        )
    }

    private suspend fun insertChat() {
        chatDao.upsertAll(
            listOf(
                ChatEntity(
                    id = CHAT,
                    kind = "family",
                    peerUserId = null,
                    title = "The Smiths",
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
}
