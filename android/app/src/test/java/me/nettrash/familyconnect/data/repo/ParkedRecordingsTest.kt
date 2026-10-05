/*
 * ParkedRecordingsTest.kt
 * Family Connect (Android)
 *
 * Where a voice message that was not sent waits (#79,
 * docs/audio-video-messages-2026-10-04.md, S2.8): per chat, in the app's own
 * storage — the file moved out of the evictable cache into `filesDir`, its
 * length, its reply and its caption in the index — until it is sent or
 * deleted; files no entry names swept once per process; and nothing a
 * session recorded allowed to land after that session has ended.
 *
 * runBlocking rather than runTest: the store moves files on Dispatchers.IO,
 * and these tests want the real thing to finish.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import me.nettrash.familyconnect.data.net.dto.ReplyToDto
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import org.junit.After
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

@RunWith(RobolectricTestRunner::class)
class ParkedRecordingsTest {

    private val context = RuntimeEnvironment.getApplication()
    private val settings = FakeSettingsRepository()
    private val epoch = SessionEpoch()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)

    @After
    fun tearDown() {
        scope.cancel()
    }

    /**
     * This test's own folder: Robolectric shares one filesDir between every
     * test in a JVM, and another test's store sweeping it would take these
     * files.
     */
    @get:org.junit.Rule
    val folder = org.junit.rules.TemporaryFolder()

    private val root by lazy { folder.newFolder("parked-recordings") }

    private fun store() = ParkedRecordings(settings, epoch, scope, root)

    private fun recording(bytes: Int = 4096): File =
        File.createTempFile("voice-", ".m4a", File(context.cacheDir, "recordings").apply { mkdirs() })
            .apply { writeBytes(ByteArray(bytes) { 7 }) }

    private val quote = ReplyToDto(messageId = 501, senderId = 9, excerpt = "Are you coming?")

    @Test
    fun `a park moves the file out of the cache and remembers all four things`(): Unit = runBlocking {
        val store = store()
        val source = recording()

        val entry = requireNotNull(
            store.park(42, source, 42_000, quote, "for grandma", epoch.current()),
        )

        assertThat(source.exists()).isFalse()
        val kept = store.file(entry)
        assertThat(kept.parentFile).isEqualTo(root)
        assertThat(kept.readBytes()).hasLength(4096)
        assertThat(entry.chatId).isEqualTo(42)
        assertThat(entry.durationMs).isEqualTo(42_000)
        assertThat(entry.replyTo).isEqualTo(quote)
        assertThat(entry.caption).isEqualTo("for grandma")
        assertThat(settings.current.parkedRecordings).containsExactly(entry)
    }

    @Test
    fun `each chat sees its own, oldest first`(): Unit = runBlocking {
        val store = store()
        val a = store.park(42, recording(), 1_000, null, "", epoch.current())!!
        val b = store.park(7, recording(), 2_000, null, "", epoch.current())!!
        val c = store.park(42, recording(), 3_000, null, "", epoch.current())!!

        assertThat(store.forChat(42).first()).containsExactly(a, c).inOrder()
        assertThat(store.forChat(7).first()).containsExactly(b)
    }

    @Test
    fun `a removal takes the entry and its file`(): Unit = runBlocking {
        val store = store()
        val entry = store.park(42, recording(), 5_000, null, "", epoch.current())!!

        store.remove(entry.id)

        assertThat(settings.current.parkedRecordings).isEmpty()
        assertThat(store.file(entry).exists()).isFalse()
    }

    /**
     * Sign-out deletes everything recorded and not sent — including a park
     * still on its way when the session ended, which must not land in the
     * next account's store.
     */
    @Test
    fun `a park from a session that has ended is dropped with its file`(): Unit = runBlocking {
        val store = store()
        val session = epoch.current()
        val source = recording()

        epoch.advance()
        val entry = store.park(42, source, 5_000, quote, "for grandma", session)

        assertThat(entry).isNull()
        assertThat(source.exists()).isFalse()
        assertThat(settings.current.parkedRecordings).isEmpty()
        assertThat(root.listFiles().orEmpty()).isEmpty()
    }

    @Test
    fun `the sweep reclaims files nothing names and entries whose file is gone`(): Unit = runBlocking {
        val store = store()
        val kept = store.park(42, recording(), 5_000, null, "", epoch.current())!!
        val lost = store.park(42, recording(), 6_000, null, "", epoch.current())!!
        store.file(lost).delete()
        val stray = File(root, "stray.m4a").apply { writeBytes(ByteArray(10)) }

        val swept = store.sweep()

        assertThat(swept).isEqualTo(1)
        assertThat(stray.exists()).isFalse()
        assertThat(store.file(kept).exists()).isTrue()
        assertThat(settings.current.parkedRecordings).containsExactly(kept)
    }

    /** What an earlier run left behind with no entry goes the moment the store exists — at launch. */
    @Test
    fun `the launch sweep reclaims what an earlier run left behind`(): Unit = runBlocking {
        val stray = File(root.apply { mkdirs() }, "left-behind.m4a")
            .apply { writeBytes(ByteArray(10)) }

        val store = store()
        store.launchSweep.join()

        assertThat(stray.exists()).isFalse()
    }

    /** An entry whose file went missing is not offered: its Send would have nothing to send. */
    @Test
    fun `an entry whose file is gone is not shown`(): Unit = runBlocking {
        val store = store()
        val entry = store.park(42, recording(), 5_000, null, "", epoch.current())!!

        store.file(entry).delete()

        assertThat(store.forChat(42).first()).isEmpty()
    }

    // -- Phase 1: the Undo window's "sending" mark (S2.6) ------------------

    /** A note in its Undo window is on its way — not a not-sent row, and it does not block recording. */
    @Test
    fun `a note marked sending is not a not-sent row`(): Unit = runBlocking {
        val store = store()
        val sending = store.park(42, recording(), 3_000, quote, "", epoch.current(), sending = true)!!

        assertThat(sending.sending).isTrue()
        assertThat(store.forChat(42).first()).isEmpty()
        assertThat(settings.current.parkedRecordings).containsExactly(sending)
        assertThat(store.file(sending).exists()).isTrue()
    }

    /**
     * A crash cannot lose it: at launch, an entry still marked "sending"
     * becomes a not-sent row — never an orphan the sweep takes while its
     * sender believes it went (S2.6).
     */
    @Test
    fun `a launch turns a note a crash left sending into a not-sent row`(): Unit = runBlocking {
        val sending = store().park(42, recording(), 3_000, quote, "", epoch.current(), sending = true)!!

        // The next process: a new store, whose launch sweep runs first.
        val next = store()
        next.launchSweep.join()

        val row = next.forChat(42).first().single()
        assertThat(row.id).isEqualTo(sending.id)
        assertThat(row.sending).isFalse()
        assertThat(row.replyTo).isEqualTo(quote)
        assertThat(next.file(row).exists()).isTrue()
    }

    /** A note whose hand-off could not be queued becomes what any interrupted recording is. */
    @Test
    fun `markNotSent turns a sending note into a not-sent row`(): Unit = runBlocking {
        val store = store()
        val sending = store.park(42, recording(), 3_000, null, "", epoch.current(), sending = true)!!
        val plain = store.park(42, recording(), 2_000, null, "", epoch.current())!!

        store.markNotSent(sending.id)

        assertThat(store.forChat(42).first().map { it.id }).containsExactly(sending.id, plain.id).inOrder()
    }

    /** An index Phase 0 wrote — no "sending" key at all — still reads, as not sending. */
    @Test
    fun `an entry written before the sending mark reads as not sending`() {
        val decoded = ParkedRecording.decode(
            """[{"id":"a","chat_id":42,"file":"a.m4a","duration_ms":1000}]""",
        ).single()
        assertThat(decoded.sending).isFalse()
        // And a not-sending entry writes no key for it.
        assertThat(ParkedRecording.encode(listOf(decoded))).doesNotContain("sending")
    }

    /**
     * In the app the bytes live in filesDir/parked-recordings — which the
     * system does not reclaim, and which the session wipe deletes. Checked on
     * a store whose scope never runs, so it touches nothing on the disk.
     */
    @Test
    fun `the app keeps them in filesDir, under parked-recordings`() {
        val idle = CoroutineScope(SupervisorJob().apply { cancel() })
        val app = ParkedRecordings(context, settings, epoch, idle)
        val entry = ParkedRecording(id = "a", chatId = 42, file = "a.m4a", durationMs = 1_000)

        assertThat(app.file(entry).parentFile).isEqualTo(File(context.filesDir, "parked-recordings"))
        assertThat(ParkedRecordings.directory(context)).isEqualTo(File(context.filesDir, "parked-recordings"))
    }
}
