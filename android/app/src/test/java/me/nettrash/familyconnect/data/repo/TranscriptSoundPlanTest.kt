/*
 * TranscriptSoundPlanTest.kt
 * Family Connect (Android)
 *
 * The decision behind the sound a transcript request supplies
 * (docs/protocol.md, "Transcripts on request"): PASSTHROUGH for an AAC
 * track, RE-ENCODE to 64 kbit/s mono for anything else, UNAVAILABLE when
 * neither can come out within `transcribe_max_bytes` — and the loop that
 * tries them, deletes what came out too large, and falls through a way the
 * platform could not do.
 *
 * The export itself needs a real codec: androidTest/…/TranscriptSoundDeviceTest.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.repo.TranscriptSoundPlan.Result
import me.nettrash.familyconnect.data.repo.TranscriptSoundPlan.Way
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.File

class TranscriptSoundPlanTest {

    @get:Rule
    val folder = TemporaryFolder()

    // -- Which ways ------------------------------------------------------------------

    @Test
    fun `an AAC track is copied first, with the re-encode behind it`() {
        assertThat(TranscriptSoundPlan.ways("aac", 128_000, 60_000, MAX))
            .containsExactly(Way.PASSTHROUGH, Way.REENCODE).inOrder()
    }

    @Test
    fun `any other codec is re-encoded, never copied`() {
        for (codec in listOf("opus", "vorbis", "flac", "mp3", "pcm", "alac")) {
            assertThat(TranscriptSoundPlan.ways(codec, 96_000, 60_000, MAX)).containsExactly(Way.REENCODE)
        }
    }

    @Test
    fun `a track the platform could not name is still tried as a re-encode`() {
        // Media3 reads with its own extractor and may find what the
        // platform's did not; if it does not, the export fails and the
        // answer is "not available".
        assertThat(TranscriptSoundPlan.ways("unknown", null, null, MAX)).containsExactly(Way.REENCODE)
        assertThat(TranscriptSoundPlan.ways(null, null, null, MAX)).containsExactly(Way.REENCODE)
    }

    @Test
    fun `an AAC copy estimated over the ceiling goes straight to the re-encode`() {
        // 30 minutes at 256 kbit/s is about 57 MB; at 64 kbit/s it is 14 MB.
        assertThat(TranscriptSoundPlan.ways("aac", 256_000, 30 * 60_000L, MAX)).containsExactly(Way.REENCODE)
    }

    @Test
    fun `sound too long to fit even at 64 kbit s is not tried at all`() {
        // 25 MiB at 64 kbit/s, with the margin, is a little under 54 minutes.
        assertThat(TranscriptSoundPlan.ways("opus", 32_000, 60 * 60_000L, MAX)).isEmpty()
        // A low-rate AAC copy can still fit where a re-encode would not.
        assertThat(TranscriptSoundPlan.ways("aac", 32_000, 60 * 60_000L, MAX)).containsExactly(Way.PASSTHROUGH)
    }

    @Test
    fun `what cannot be estimated is tried, and the size decides afterwards`() {
        assertThat(TranscriptSoundPlan.ways("aac", null, 10 * 3_600_000L, MAX))
            .containsExactly(Way.PASSTHROUGH).inOrder()
        assertThat(TranscriptSoundPlan.ways("aac", 256_000, null, MAX))
            .containsExactly(Way.PASSTHROUGH, Way.REENCODE).inOrder()
        assertThat(TranscriptSoundPlan.ways("aac", 0, 60_000, MAX))
            .containsExactly(Way.PASSTHROUGH, Way.REENCODE).inOrder()
    }

    @Test
    fun `no ceiling means nothing is tried`() {
        assertThat(TranscriptSoundPlan.ways("aac", 128_000, 60_000, 0)).isEmpty()
    }

    // -- The size bound ----------------------------------------------------------------

    @Test
    fun `the bound is the server's, inclusive at both ends`() {
        assertThat(TranscriptSoundPlan.fits(1, MAX)).isTrue()
        assertThat(TranscriptSoundPlan.fits(MAX, MAX)).isTrue()
        assertThat(TranscriptSoundPlan.fits(MAX + 1, MAX)).isFalse()
        // An empty part is `not_transcribable` on the server.
        assertThat(TranscriptSoundPlan.fits(0, MAX)).isFalse()
        assertThat(TranscriptSoundPlan.fits(1, 0)).isFalse()
    }

    @Test
    fun `the estimate is bitrate times duration plus the container's margin`() {
        // 64 000 bit/s for 100 s is 800 000 bytes; 2 % on top.
        assertThat(TranscriptSoundPlan.estimatedBytes(64_000, 100_000)).isEqualTo(816_000)
        assertThat(TranscriptSoundPlan.overCeiling(64_000, 100_000, 816_000)).isFalse()
        assertThat(TranscriptSoundPlan.overCeiling(64_000, 100_000, 815_999)).isTrue()
    }

    // -- Trying them ------------------------------------------------------------------------

    private fun fileOf(bytes: Int): File = folder.newFile().apply { writeBytes(ByteArray(bytes)) }

    @Test
    fun `a copy that fits is sent, and the re-encode is never run`() = runTest {
        val tried = mutableListOf<Way>()
        val copy = fileOf(1_000)

        val result = TranscriptSoundPlan.take(listOf(Way.PASSTHROUGH, Way.REENCODE), 2_000) { way ->
            tried += way
            copy
        }

        assertThat(result).isEqualTo(Result.Ready(copy, Way.PASSTHROUGH))
        assertThat(tried).containsExactly(Way.PASSTHROUGH)
    }

    @Test
    fun `a copy that came out too large is deleted and the sound re-encoded`() = runTest {
        val copy = fileOf(3_000)
        val reencoded = fileOf(1_500)

        val result = TranscriptSoundPlan.take(listOf(Way.PASSTHROUGH, Way.REENCODE), 2_000) { way ->
            if (way == Way.PASSTHROUGH) copy else reencoded
        }

        assertThat(result).isEqualTo(Result.Ready(reencoded, Way.REENCODE))
        assertThat(copy.exists()).isFalse()
    }

    @Test
    fun `a way the platform could not do falls through to the next`() = runTest {
        val reencoded = fileOf(500)

        val result = TranscriptSoundPlan.take(listOf(Way.PASSTHROUGH, Way.REENCODE), 2_000) { way ->
            if (way == Way.PASSTHROUGH) null else reencoded
        }

        assertThat(result).isEqualTo(Result.Ready(reencoded, Way.REENCODE))
    }

    @Test
    fun `a device that cannot decode the track could not read its sound`() = runTest {
        assertThat(TranscriptSoundPlan.take(listOf(Way.REENCODE), 2_000) { null }).isEqualTo(Result.Unreadable)
    }

    @Test
    fun `sound still over the ceiling after re-encoding is too long, and is deleted`() = runTest {
        val reencoded = fileOf(2_001)

        val result = TranscriptSoundPlan.take(listOf(Way.REENCODE), 2_000) { reencoded }

        assertThat(result).isEqualTo(Result.TooLong)
        assertThat(reencoded.exists()).isFalse()
    }

    @Test
    fun `an empty result is not sound`() = runTest {
        val empty = fileOf(0)

        assertThat(TranscriptSoundPlan.take(listOf(Way.REENCODE), 2_000) { empty }).isEqualTo(Result.Unreadable)
        assertThat(empty.exists()).isFalse()
    }

    @Test
    fun `no ways at all is too long, and nothing is extracted`() = runTest {
        // ways() is empty only when even the re-encode is estimated over the ceiling.
        var ran = false
        assertThat(TranscriptSoundPlan.take(emptyList(), 2_000) { ran = true; null }).isEqualTo(Result.TooLong)
        assertThat(ran).isFalse()
    }

    @Test
    fun `a copy too large and a re-encode that failed is too long, not unreadable`() = runTest {
        val copied = fileOf(2_001)
        val result = TranscriptSoundPlan.take(listOf(Way.PASSTHROUGH, Way.REENCODE), 2_000) { way ->
            if (way == Way.PASSTHROUGH) copied else null
        }
        assertThat(result).isEqualTo(Result.TooLong)
    }

    @Test
    fun `a stated length too long even at 64 kbit s is known before anything is fetched`() {
        // The web's and iOS's arithmetic: the sound alone at 64 kbit/s, no margin.
        assertThat(TranscriptSoundPlan.knownTooLong(55 * 60_000L, MAX)).isTrue()
        assertThat(TranscriptSoundPlan.knownTooLong(50 * 60_000L, MAX)).isFalse()
        assertThat(TranscriptSoundPlan.knownTooLong(60_000L, 100_000)).isTrue()
        // A length nobody stated is not "too long".
        assertThat(TranscriptSoundPlan.knownTooLong(null, MAX)).isFalse()
        assertThat(TranscriptSoundPlan.knownTooLong(0, MAX)).isFalse()
    }

    private companion object {
        /** The server's default and largest `transcribe_max_bytes`: 25 MiB. */
        const val MAX = 26_214_400L
    }
}
