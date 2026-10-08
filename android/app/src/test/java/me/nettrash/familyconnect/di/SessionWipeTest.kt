/*
 * SessionWipeTest.kt
 * Family Connect (Android)
 *
 * What the session wipe takes with it (AppModule's LocalDataWiper — the one
 * SessionRepository runs at sign-out, on a 401, at account deletion and when
 * this member is removed from the family). Pinned here for the voice
 * messages that were not sent (#79, docs/audio-video-messages-2026-10-04.md,
 * S2.8 and S4's sign-out row): "everything recorded and not sent is deleted"
 * — the index AND the bytes, so neither is left naming, or holding, the
 * other, and nothing one account recorded surfaces in the next.
 */

package me.nettrash.familyconnect.di

import androidx.work.testing.WorkManagerTestInitHelper
import com.google.common.truth.Truth.assertThat
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import me.nettrash.familyconnect.data.repo.ParkedRecording
import me.nettrash.familyconnect.data.repo.ParkedRecordings
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.createTestDb
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

@RunWith(RobolectricTestRunner::class)
class SessionWipeTest {

    private val context = RuntimeEnvironment.getApplication()
    private val db = createTestDb(Dispatchers.IO)
    private val settings = FakeSettingsRepository()

    @Before
    fun setUp() {
        // The wipe also cancels the queued uploads, which needs a WorkManager.
        WorkManagerTestInitHelper.initializeTestWorkManager(context)
    }

    @After
    fun tearDown() {
        db.close()
    }

    @Test
    fun `the wipe deletes every voice message that was not sent, index and bytes`(): Unit = runBlocking {
        val directory = ParkedRecordings.directory(context).apply { mkdirs() }
        val bytes = File(directory, "a.m4a").apply { writeBytes(ByteArray(64) { 1 }) }
        settings.updateParkedRecordings {
            listOf(ParkedRecording(id = "a", chatId = 42, file = "a.m4a", durationMs = 4_000))
        }

        AppModule.provideLocalDataWiper(db, context, settings).wipeAll()

        assertThat(settings.current.parkedRecordings).isEmpty()
        assertThat(bytes.exists()).isFalse()
        assertThat(directory.exists()).isFalse()
    }
}
