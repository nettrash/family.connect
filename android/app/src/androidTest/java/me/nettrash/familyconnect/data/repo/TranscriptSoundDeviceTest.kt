/*
 * TranscriptSoundDeviceTest.kt
 * Family Connect (Android)
 *
 * Issue #62, phase 3, on a real codec: the sound a transcript request
 * supplies (docs/protocol.md, "Transcripts on request"), taken out of a
 * fixture file and read back with the platform's own reader — the way the
 * server's `ftyp` check and its provider will meet it.
 *
 * The JVM tests prove the decision (TranscriptSoundPlanTest) and what Media3
 * is told (MediaTranscodeTest); only this proves Media3 then does it: an AAC
 * track copied sample for sample with the picture gone, anything else
 * re-encoded to mono AAC, a file with no sound giving nothing to send.
 *
 * Run it the way MediaPrepDeviceTest says — on an emulator or a spare
 * device, never a phone somebody uses (the Gradle task uninstalls the app):
 *   ANDROID_SERIAL=emulator-5554 ./gradlew -PnoBump \
 *     connectedStandardDebugAndroidTest \
 *     -Pandroid.testInstrumentationRunnerArguments.class=me.nettrash.familyconnect.data.repo.TranscriptSoundDeviceTest
 * CI has no emulator lane, so CI does not run this.
 */

package me.nettrash.familyconnect.data.repo

import android.media.MediaExtractor
import android.media.MediaFormat
import android.os.Build
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.SdkSuppress
import androidx.test.platform.app.InstrumentationRegistry
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.nio.ByteBuffer

@RunWith(AndroidJUnit4::class)
class TranscriptSoundDeviceTest {

    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val mediaPrep = MediaPrep(context, context.contentResolver)
    private val inputs = File(context.cacheDir, "transcript-device-test")
    private val uploads = File(context.cacheDir, "uploads")

    @Before
    fun setUp() {
        inputs.deleteRecursively()
        inputs.mkdirs()
        uploads.deleteRecursively()
    }

    @After
    fun tearDown() {
        inputs.deleteRecursively()
        uploads.deleteRecursively()
    }

    private suspend fun soundOf(source: File, mime: String, maxBytes: Long = MAX): TranscriptSoundPlan.Result =
        DeviceTranscriptSound.soundOf(mediaPrep, source, mime, declaredDurationMs = null, maxBytes = maxBytes)

    @Test
    fun aVideosAacTrackIsCopiedSampleForSampleWithThePictureGone(): Unit = runBlocking {
        val source = File(inputs, "clip.mp4")
        TestClips.video(source, width = 640, height = 360, fps = 30, frames = 90, bitrate = 600_000, audioChannels = 2)

        val result = soundOf(source, "video/mp4")

        val ready = result as TranscriptSoundPlan.Result.Ready
        assertThat(ready.way).isEqualTo(TranscriptSoundPlan.Way.PASSTHROUGH)
        // What the server checks for: an MPEG-4 file.
        assertThat(MediaPrep.Magic.honest(ready.file, "audio/mp4")).isTrue()
        val out = tracks(ready.file)
        assertThat(out.video).isNull()
        assertThat(out.audio?.getString(MediaFormat.KEY_MIME)).isEqualTo(MediaFormat.MIMETYPE_AUDIO_AAC)
        assertThat(out.audio?.getInteger(MediaFormat.KEY_CHANNEL_COUNT)).isEqualTo(2)
        // Not re-encoded: every AAC sample is the source's, byte for byte.
        assertThat(audioSamples(ready.file)).isEqualTo(audioSamples(source))
        assertThat(ready.file.length()).isLessThan(source.length())
        ready.file.delete()
    }

    @Test
    fun aCopyOverTheCeilingIsReEncodedToMono64k(): Unit = runBlocking {
        val source = File(inputs, "loud.mp4")
        TestClips.video(
            source, width = 320, height = 180, fps = 30, frames = 150, bitrate = 200_000,
            audioChannels = 2, audioBitrate = 256_000,
        )
        // How large the copy comes out, then a ceiling just under it.
        val copy = checkNotNull(mediaPrep.soundTrackOrNull(source, TranscriptSoundPlan.Way.PASSTHROUGH))
        val ceiling = copy.length() - 1
        copy.delete()

        val ready = soundOf(source, "video/mp4", maxBytes = ceiling) as TranscriptSoundPlan.Result.Ready

        assertThat(ready.way).isEqualTo(TranscriptSoundPlan.Way.REENCODE)
        assertThat(ready.file.length()).isAtMost(ceiling)
        val out = tracks(ready.file)
        assertThat(out.video).isNull()
        assertThat(out.audio?.getString(MediaFormat.KEY_MIME)).isEqualTo(MediaFormat.MIMETYPE_AUDIO_AAC)
        assertThat(out.audio?.getInteger(MediaFormat.KEY_CHANNEL_COUNT)).isEqualTo(1)
        // Nothing left behind but the one file handed back.
        assertThat(uploads.listFiles().orEmpty().map { it.name }).containsExactly(ready.file.name)
        ready.file.delete()
    }

    @Test
    @SdkSuppress(minSdkVersion = Build.VERSION_CODES.Q) // MediaMuxer writes Ogg from API 29.
    fun anOggOpusVoiceNoteIsReEncodedToAnM4a(): Unit = runBlocking {
        val source = File(inputs, "memo.ogg")
        TestClips.oggOpus(source, seconds = 4, bitrate = 32_000)

        val ready = soundOf(source, "audio/ogg") as TranscriptSoundPlan.Result.Ready

        assertThat(ready.way).isEqualTo(TranscriptSoundPlan.Way.REENCODE)
        assertThat(MediaPrep.Magic.honest(ready.file, "audio/mp4")).isTrue()
        val out = tracks(ready.file)
        assertThat(out.audio?.getString(MediaFormat.KEY_MIME)).isEqualTo(MediaFormat.MIMETYPE_AUDIO_AAC)
        assertThat(out.audio?.getInteger(MediaFormat.KEY_CHANNEL_COUNT)).isEqualTo(1)
        ready.file.delete()
    }

    @Test
    fun aVideoWithNoSoundHasNothingToSend(): Unit = runBlocking {
        val source = File(inputs, "silent.mp4")
        TestClips.video(source, width = 320, height = 180, fps = 30, frames = 30, bitrate = 200_000, audioChannels = null)

        assertThat(soundOf(source, "video/mp4")).isEqualTo(TranscriptSoundPlan.Result.Unreadable)
        assertThat(uploads.listFiles().orEmpty()).isEmpty()
    }

    // -- Reading the result back ---------------------------------------------------

    private class Tracks(val video: MediaFormat?, val audio: MediaFormat?)

    private fun tracks(file: File): Tracks {
        val extractor = MediaExtractor()
        try {
            extractor.setDataSource(file.absolutePath)
            var video: MediaFormat? = null
            var audio: MediaFormat? = null
            for (index in 0 until extractor.trackCount) {
                val format = extractor.getTrackFormat(index)
                val mime = format.getString(MediaFormat.KEY_MIME).orEmpty()
                if (mime.startsWith("video/")) video = format
                if (mime.startsWith("audio/")) audio = format
            }
            return Tracks(video, audio)
        } finally {
            extractor.release()
        }
    }

    /**
     * Every sample of the audio track, in order — the bytes only: a remux
     * may start the clock at a different offset, which changes no sound.
     */
    private fun audioSamples(file: File): List<List<Byte>> {
        val extractor = MediaExtractor()
        try {
            extractor.setDataSource(file.absolutePath)
            val index = (0 until extractor.trackCount).first {
                extractor.getTrackFormat(it).getString(MediaFormat.KEY_MIME).orEmpty().startsWith("audio/")
            }
            extractor.selectTrack(index)
            val buffer = ByteBuffer.allocate(1 shl 20)
            val all = mutableListOf<List<Byte>>()
            while (true) {
                val size = extractor.readSampleData(buffer, 0)
                if (size < 0) break
                val bytes = ByteArray(size).also { buffer.position(0); buffer.get(it) }
                all += bytes.toList()
                extractor.advance()
            }
            return all
        } finally {
            extractor.release()
        }
    }

    private companion object {
        const val MAX = 26_214_400L
    }
}
