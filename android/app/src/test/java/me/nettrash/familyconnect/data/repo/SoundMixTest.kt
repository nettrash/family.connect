/*
 * SoundMixTest.kt
 * Family Connect (Android)
 *
 * Rule E's mix (docs/protocol.md, "Preparing media before upload", issue
 * #98, amended): a source with more than one sound track that plays is
 * transcoded with them all MIXED into the one AAC-LC track — stereo when any
 * of them is, at the highest sample rate any of them has. This pins what can
 * be pinned without a codec: which tracks, in what order, how wide; how a
 * track is found among what Media3's extractor offers (by id, never by
 * position); and the composition Media3 is handed. Whether the sum comes out
 * audible needs a real decoder and encoder: androidTest/…/MediaPrepDeviceTest.
 *
 * Robolectric only because Presentation and the audio processors want an
 * android.* around them.
 */

package me.nettrash.familyconnect.data.repo

import android.net.Uri
import androidx.media3.common.C
import androidx.media3.common.audio.AudioProcessor
import androidx.media3.common.audio.AudioProcessor.AudioFormat
import androidx.media3.common.util.UnstableApi
import androidx.media3.transformer.Composition
import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.repo.Mp4SoundTracks.SoundTrack
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@UnstableApi
@RunWith(RobolectricTestRunner::class)
class SoundMixTest {

    private fun track(id: Int, channels: Int? = 2, rate: Int? = 48_000) =
        SoundTrack(id, enabled = true, alternateGroup = 0, channels = channels, sampleRate = rate)

    // -- Which tracks, in what order, how wide --------------------------------------

    @Test
    fun `one sound track, none, or an unreadable file is no mix - the single path as it always was`() {
        assertThat(SoundMix.plan(null)).isNull()
        assertThat(SoundMix.plan(emptyList())).isNull()
        assertThat(SoundMix.plan(listOf(track(2)))).isNull()
        // Two traks under one id: a selector by id cannot tell them apart.
        assertThat(SoundMix.plan(listOf(track(2), track(2)))).isNull()
    }

    /** A screen recording: the app's sound and the microphone. */
    @Test
    fun `two tracks are both mixed, the one stating the highest rate leading`() {
        val mix = SoundMix.plan(listOf(track(2, rate = 44_100), track(3, rate = 48_000)))!!
        assertThat(mix.trackIds).containsExactly(3, 2).inOrder()

        // Equal rates: the wider one leads, then file order.
        val tie = SoundMix.plan(listOf(track(2, channels = 1), track(3, channels = 2), track(4, channels = 2)))!!
        assertThat(tie.trackIds).containsExactly(3, 4, 2).inOrder()

        // A rate nobody stated never leads over one that is.
        val unknown = SoundMix.plan(listOf(track(2, rate = null), track(3, rate = 22_050)))!!
        assertThat(unknown.trackIds).containsExactly(3, 2).inOrder()
    }

    @Test
    fun `stereo when any of them is, mono only when all say so`() {
        assertThat(SoundMix.plan(listOf(track(2, channels = 1), track(3, channels = 1)))!!.channels).isEqualTo(1)
        assertThat(SoundMix.plan(listOf(track(2, channels = 1), track(3, channels = 2)))!!.channels).isEqualTo(2)
        // 5.1 is mixed down to stereo, and a track that states nothing counts as stereo.
        assertThat(SoundMix.plan(listOf(track(2, channels = 1), track(3, channels = 6)))!!.channels).isEqualTo(2)
        assertThat(SoundMix.plan(listOf(track(2, channels = 1), track(3, channels = null)))!!.channels).isEqualTo(2)
    }

    /** The profile's rate for the mix's width — not the one voice the probe happened to see. */
    @Test
    fun `a mix is encoded at the profile's rate for its width`() {
        assertThat(SoundMix(listOf(2, 3), channels = 2).audioBitrate).isEqualTo(128_000)
        assertThat(SoundMix(listOf(2, 3), channels = 1).audioBitrate).isEqualTo(64_000)
    }

    // -- Finding a track among what Media3 offers -----------------------------------

    @Test
    fun `a Format is a track by its id, renumbered or not`() {
        assertThat(SoundMix.isTrack("3", 3)).isTrue()
        assertThat(SoundMix.isTrack("1:3", 3)).isTrue()
        assertThat(SoundMix.isTrack("13", 3)).isFalse()
        assertThat(SoundMix.isTrack("3", 13)).isFalse()
        assertThat(SoundMix.isTrack(null, 3)).isFalse()
    }

    /**
     * The extractor could not read track 2 (QuickTime `in24`, say), so it is
     * not offered at all: track 3 is then the FIRST audio group. Chosen by
     * position, "the second sound track" would be nothing, and "the first"
     * the wrong voice.
     */
    @Test
    fun `a track is located by id, never by position`() {
        val offered = listOf(
            C.TRACK_TYPE_VIDEO to listOf(listOf<String?>("1")),
            C.TRACK_TYPE_AUDIO to listOf(listOf<String?>("3"), listOf<String?>("5")),
            C.TRACK_TYPE_TEXT to listOf(listOf<String?>("2")),
        )
        assertThat(SoundMix.locate(offered, 3)).isEqualTo(Triple(1, 0, 0))
        assertThat(SoundMix.locate(offered, 5)).isEqualTo(Triple(1, 1, 0))
        // Not offered to an audio renderer: nothing is selected, and that item has no sound.
        assertThat(SoundMix.locate(offered, 2)).isNull()
        assertThat(SoundMix.locate(offered, 1)).isNull()
    }

    // -- What Media3 is handed -------------------------------------------------------

    @Test
    fun `the mix is one sequence per track of the same file, the picture in the first`() {
        val settings = TranscodeSettings.forVideo(
            MediaPlan.VideoTarget(width = 1280, height = 720, frameRate = 30.0, videoBitrate = 2_000_000, audioBitrate = 128_000),
            MediaPlan.VideoSource(
                width = 1920, height = 1080, frameRate = 60.0, container = "video/mp4", videoCodec = "h264",
                audioCodec = "aac", audioChannels = 2, videoBitrate = 20_000_000, audioBitrate = 128_000,
                sizeBytes = 90_000_000, durationMs = 30_000,
            ),
        )
        val input = Uri.parse("file:///screen.mp4")
        val composition = TranscodeRecipe.mixedComposition(
            input,
            SoundMix(trackIds = listOf(3, 2), channels = 2),
            TranscodeRecipe.videoEffects(settings),
            TranscodeRecipe.hdrMode(settings),
        )

        assertThat(composition.hdrMode).isEqualTo(Composition.HDR_MODE_TONE_MAP_HDR_TO_SDR_USING_OPEN_GL)
        assertThat(composition.sequences).hasSize(2)
        val (first, second) = composition.sequences.map { it.editedMediaItems.single() }
        // Left to infer: a sequence whose track cannot be read fails, rather than being filled with silence.
        assertThat(composition.sequences.map { it.trackTypes }).containsExactly(
            setOf(C.TRACK_TYPE_NONE),
            setOf(C.TRACK_TYPE_NONE),
        )
        assertThat(first.mediaItem.localConfiguration!!.tag).isEqualTo(SoundTrackChoice(3))
        assertThat(second.mediaItem.localConfiguration!!.tag).isEqualTo(SoundTrackChoice(2))
        assertThat(first.mediaItem.localConfiguration!!.uri).isEqualTo(input)
        assertThat(second.mediaItem.localConfiguration!!.uri).isEqualTo(input)
        assertThat(first.removeVideo).isFalse()
        assertThat(second.removeVideo).isTrue()
        assertThat(first.removeAudio || second.removeAudio).isFalse()
        assertThat(first.effects.videoEffects).hasSize(TranscodeRecipe.videoEffects(settings).size)
        assertThat(second.effects.videoEffects).isEmpty()
        // Only the lead is spread to the mix's width; the others are only folded down from surround.
        assertThat(first.effects.audioProcessors).hasSize(1)
        assertThat(second.effects.audioProcessors).hasSize(1)
        fun widthOfMono(processor: AudioProcessor): Int {
            val configured = processor.configure(AudioFormat(48_000, 1, C.ENCODING_PCM_16BIT))
            return if (processor.isActive) configured.channelCount else 1
        }
        assertThat(widthOfMono(first.effects.audioProcessors.single())).isEqualTo(2)
        // Media3's own conversion spreads it — doing it here stalled the export on a device.
        assertThat(widthOfMono(second.effects.audioProcessors.single())).isEqualTo(1)
    }

    /** Whichever input the mix takes its format from, it is already the mix's width. */
    @Test
    fun `every input is brought to the mix's width`() {
        fun out(channels: Int, input: Int): Int {
            val processor = TranscodeRecipe.mixProcessors(channels).single()
            return processor.configure(AudioFormat(48_000, input, C.ENCODING_PCM_16BIT)).let { configured ->
                if (processor.isActive) configured.channelCount else input
            }
        }
        // Stereo mix: mono spread to both sides, surround folded down, stereo untouched.
        assertThat(out(channels = 2, input = 1)).isEqualTo(2)
        assertThat(out(channels = 2, input = 2)).isEqualTo(2)
        assertThat(out(channels = 2, input = 6)).isEqualTo(2)
        // Mono mix: every input to one channel.
        assertThat(out(channels = 1, input = 1)).isEqualTo(1)
        assertThat(out(channels = 1, input = 2)).isEqualTo(1)
        assertThat(out(channels = 1, input = 6)).isEqualTo(1)
    }
}
