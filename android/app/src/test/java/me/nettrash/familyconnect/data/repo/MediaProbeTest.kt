/*
 * MediaProbeTest.kt
 * Family Connect (Android)
 *
 * What the platform's readers say about a file, turned into what MediaPlan
 * decides on. The planner is held to the reference by its vectors; this is
 * the other half of "the same answer for the same file" — that this client
 * READS the same file the way the others do: the displayed size of a turned
 * clip, 29.97 rather than MPEG4Extractor's rounded 30, the reference's codec
 * names, and "unknown" (never a guess) for what a reader will not say.
 *
 * The readers are Robolectric's shadows, fed the tracks and metadata a real
 * extractor would report for such a file.
 */

package me.nettrash.familyconnect.data.repo

import android.media.MediaFormat
import android.media.MediaMetadataRetriever
import android.net.Uri
import com.google.common.truth.Truth.assertThat
import java.io.File
import kotlin.io.path.createTempFile
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.shadows.ShadowMediaExtractor
import org.robolectric.shadows.ShadowMediaMetadataRetriever
import org.robolectric.shadows.util.DataSource

@RunWith(RobolectricTestRunner::class)
class MediaProbeTest {

    private val context = RuntimeEnvironment.getApplication()

    // -- The reference's vocabulary ---------------------------------------------------

    @Test
    fun `codecs are named the way the reference names them`() {
        assertThat(MediaProbe.videoCodec("video/avc")).isEqualTo("h264")
        assertThat(MediaProbe.videoCodec("video/hevc")).isEqualTo("hevc")
        assertThat(MediaProbe.videoCodec("video/av01")).isEqualTo("av1")
        assertThat(MediaProbe.videoCodec("video/x-vnd.on2.vp9")).isEqualTo("vp9")
        assertThat(MediaProbe.videoCodec("video/dolby-vision")).isEqualTo("unknown")
        assertThat(MediaProbe.videoCodec(null)).isEqualTo("unknown")

        assertThat(MediaProbe.audioCodec("audio/mp4a-latm")).isEqualTo("aac")
        assertThat(MediaProbe.audioCodec("audio/mpeg")).isEqualTo("mp3")
        assertThat(MediaProbe.audioCodec("audio/raw")).isEqualTo("pcm")
        assertThat(MediaProbe.audioCodec("audio/flac")).isEqualTo("flac")
        assertThat(MediaProbe.audioCodec("audio/alac")).isEqualTo("alac")
        assertThat(MediaProbe.audioCodec("audio/vorbis")).isEqualTo("vorbis")
        assertThat(MediaProbe.audioCodec("audio/opus")).isEqualTo("opus")
        // Companded or ADPCM in a WAV is already compressed, and no rule names it.
        assertThat(MediaProbe.audioCodec("audio/g711-alaw")).isEqualTo("unknown")
        assertThat(MediaProbe.audioCodec(null)).isEqualTo("unknown")
    }

    @Test
    fun `a turned clip reports the size it is displayed at`() {
        assertThat(MediaProbe.displaySize(1920, 1080, 90)).isEqualTo(1080L to 1920L)
        assertThat(MediaProbe.displaySize(1920, 1080, 270)).isEqualTo(1080L to 1920L)
        assertThat(MediaProbe.displaySize(1920, 1080, -90)).isEqualTo(1080L to 1920L)
        assertThat(MediaProbe.displaySize(1920, 1080, 180)).isEqualTo(1920L to 1080L)
        assertThat(MediaProbe.displaySize(1920, 1080, null)).isEqualTo(1920L to 1080L)
        // No size is MediaPlan's "no size", which is its Fallback.
        assertThat(MediaProbe.displaySize(null, 1080, 0)).isEqualTo(0L to 0L)
        assertThat(MediaProbe.displaySize(0, 1080, 0)).isEqualTo(0L to 0L)
    }

    /**
     * MPEG4Extractor states an MP4's rate as an INTEGER: 29.97 reads as 30.
     * The count over the duration is the rate AVFoundation reports, to the
     * bit — which is the rate the reference's vectors are written in.
     */
    @Test
    fun `the frame rate is the sample count over the duration, else what is stated`() {
        assertThat(MediaProbe.frameRate(1_800, 60_060_000, stated = 30.0)).isEqualTo(30_000.0 / 1001.0)
        assertThat(MediaProbe.frameRate(1_500, 60_000_000, stated = 25.0)).isEqualTo(25.0)
        assertThat(MediaProbe.frameRate(null, 60_060_000, stated = 30.0)).isEqualTo(30.0)
        assertThat(MediaProbe.frameRate(1_800, null, stated = 29.97)).isEqualTo(29.97)
        assertThat(MediaProbe.frameRate(0, 0, stated = null)).isNull()
    }

    @Test
    fun `a media type's essence is lowercase with no parameters`() {
        assertThat(MediaProbe.essence("Video/MP4; codecs=\"avc1\"")).isEqualTo("video/mp4")
        assertThat(MediaProbe.essence(" video/quicktime ")).isEqualTo("video/quicktime")
        assertThat(MediaProbe.essence(null)).isEqualTo("")
    }

    // -- The readers --------------------------------------------------------------------

    @Test
    fun `a portrait 4K60 phone clip is read as the planner needs it`() {
        val uri = Uri.parse("content://media/external/video/media/4242")
        val source = DataSource.toDataSource(context, uri)
        ShadowMediaExtractor.addTrack(
            source,
            MediaFormat.createVideoFormat("video/hevc", 3840, 2160).apply {
                setInteger(MediaFormat.KEY_FRAME_RATE, 60)
                setLong(MediaFormat.KEY_DURATION, 10_010_000)
                setInteger(MediaFormat.KEY_BIT_RATE, 40_000_000)
            },
            ByteArray(0),
        )
        ShadowMediaExtractor.addTrack(
            source,
            MediaFormat.createAudioFormat("audio/mp4a-latm", 48_000, 2).apply {
                setInteger(MediaFormat.KEY_BIT_RATE, 192_000)
            },
            ByteArray(0),
        )
        metadata(source, MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH to "3840")
        metadata(source, MediaMetadataRetriever.METADATA_KEY_VIDEO_HEIGHT to "2160")
        metadata(source, MediaMetadataRetriever.METADATA_KEY_VIDEO_ROTATION to "90")
        metadata(source, MediaMetadataRetriever.METADATA_KEY_DURATION to "10010")
        // 600 frames in 10.01 s: 59.94…, where the extractor says 60.
        metadata(source, MediaMetadataRetriever.METADATA_KEY_VIDEO_FRAME_COUNT to "600")

        val read = MediaProbe.video(context, uri, "video/quicktime", 50_000_000)

        assertThat(read).isEqualTo(
            MediaPlan.VideoSource(
                width = 2160,
                height = 3840,
                frameRate = 600 * 1_000_000.0 / 10_010_000,
                container = "video/quicktime",
                videoCodec = "hevc",
                audioCodec = "aac",
                audioChannels = 2,
                videoBitrate = 40_000_000,
                audioBitrate = 192_000,
                sizeBytes = 50_000_000,
                durationMs = 10_010,
            ),
        )
        assertThat(MediaPlan.planVideo(read)).isEqualTo(
            MediaPlan.VideoPlan.Transcode(MediaPlan.VideoTarget(720, 1280, 30.0, 2_000_000, 128_000)),
        )
    }

    /** Some containers state the rate as a Float, and MediaFormat throws for the wrong getter. */
    @Test
    fun `a frame rate stated as a float is read too`() {
        val uri = Uri.parse("content://media/external/video/media/4243")
        val source = DataSource.toDataSource(context, uri)
        ShadowMediaExtractor.addTrack(
            source,
            MediaFormat.createVideoFormat("video/avc", 1280, 720).apply {
                setFloat(MediaFormat.KEY_FRAME_RATE, 25f)
            },
            ByteArray(0),
        )

        val read = MediaProbe.video(context, uri, "video/mp4", 1_000_000)

        assertThat(read.frameRate).isEqualTo(25.0)
        assertThat(read.width to read.height).isEqualTo(1280L to 720L)
        // No audio track at all is null — not "unknown", which is a track nobody could name.
        assertThat(read.audioCodec).isNull()
        // Nothing stated and no duration to estimate from: V is unknown, and rule A cannot hold.
        assertThat(read.videoBitrate).isNull()
        assertThat(MediaPlan.withinProfile(read)).isFalse()
    }

    /**
     * A reader that can make nothing of the file says nothing — and the
     * planner's answer to "no size" is its Fallback, which sends the video
     * the way 1.1 did.
     */
    @Test
    fun `a file no reader can open is unknown, not guessed`() {
        val read = MediaProbe.video(context, Uri.parse("content://nowhere/1"), "video/mp4", 5_000)

        assertThat(read.width).isEqualTo(0)
        assertThat(read.frameRate).isNull()
        assertThat(read.videoCodec).isEqualTo("unknown")
        assertThat(read.audioCodec).isNull()
        assertThat(MediaPlan.planVideo(read)).isEqualTo(MediaPlan.VideoPlan.Fallback)
    }

    @Test
    fun `a picked WAV is pcm, and the audio rules re-encode it`() {
        val file: File = createTempFile(suffix = ".wav").toFile()
        try {
            val source = DataSource.toDataSource(file.absolutePath)
            ShadowMediaExtractor.addTrack(
                source,
                MediaFormat.createAudioFormat("audio/raw", 44_100, 2),
                ByteArray(0),
            )
            metadata(source, MediaMetadataRetriever.METADATA_KEY_DURATION to "180000")

            val read = MediaProbe.audio(file, "audio/wav", 31_752_000)

            assertThat(read).isEqualTo(
                MediaPlan.AudioSource("audio/wav", "pcm", 2, null, 31_752_000, 180_000),
            )
            assertThat(MediaPlan.planAudio(read)).isEqualTo(MediaPlan.AudioPlan.Transcode(128_000))
        } finally {
            file.delete()
        }
    }

    /** A 128 kbit/s MP3 is the case 1.1's objection was about: left exactly as it is. */
    @Test
    fun `a stated 128k MP3 is kept`() {
        val file: File = createTempFile(suffix = ".mp3").toFile()
        try {
            val source = DataSource.toDataSource(file.absolutePath)
            ShadowMediaExtractor.addTrack(
                source,
                MediaFormat.createAudioFormat("audio/mpeg", 44_100, 2).apply {
                    setInteger(MediaFormat.KEY_BIT_RATE, 128_000)
                },
                ByteArray(0),
            )

            val read = MediaProbe.audio(file, "audio/mpeg", 3_000_000)

            assertThat(read.codec).isEqualTo("mp3")
            assertThat(read.bitrate).isEqualTo(128_000)
            assertThat(MediaPlan.planAudio(read)).isEqualTo(MediaPlan.AudioPlan.Keep)
        } finally {
            file.delete()
        }
    }

    private fun metadata(source: DataSource, entry: Pair<Int, String>) =
        ShadowMediaMetadataRetriever.addMetadata(source, entry.first, entry.second)
}
