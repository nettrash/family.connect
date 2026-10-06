/*
 * AndroidVoiceRecorderTest.kt
 * Family Connect (Android)
 *
 * The microphone, made safe (#79, docs/audio-video-messages-2026-10-04.md,
 * Phase 0), against Robolectric's MediaRecorder and AudioManager:
 *
 *  - the five-minute cap is HEARD (an OnInfoListener) and handed to the
 *    owner with what was recorded — before, the hardware stopped at 5:00,
 *    nothing told the composer, and its counter ran on;
 *  - the counter runs on a monotonic clock, and stops with the recording;
 *  - a failure, and another app taking the audio (the focus lost
 *    transiently), end it the same way, with what was kept; a chime that only
 *    asks the others to duck does not, and neither does another app starting
 *    to play, which takes the focus for good (S4);
 *  - one recording at a time in the whole app (S1.7): a second owner's
 *    start hands the first one its recording back first;
 *  - transient audio focus is taken while recording and given back after.
 */

package me.nettrash.familyconnect.data.repo

import android.media.AudioManager
import android.media.MediaRecorder
import com.google.common.truth.Truth.assertThat
import java.io.File
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.shadows.ShadowMediaRecorder

@RunWith(RobolectricTestRunner::class)
class AndroidVoiceRecorderTest {

    private val context = RuntimeEnvironment.getApplication()
    private val audioManager = context.getSystemService(AudioManager::class.java)

    /** The recorder's clock, by hand: nothing here waits for real time to pass. */
    private var now = 1_000_000L

    private val made = mutableListOf<MediaRecorder>()

    private val recorder = AndroidVoiceRecorder(
        context = context,
        clock = { now },
        newRecorder = {
            @Suppress("DEPRECATION")
            MediaRecorder().also { made += it }
        },
    )

    /** What each owner was told, in order. */
    private class Heard : VoiceRecorder.Listener {
        val endings = mutableListOf<Pair<VoiceRecorder.Ending, VoiceRecorder.Recording?>>()
        override fun onEnded(ending: VoiceRecorder.Ending, recording: VoiceRecorder.Recording?) {
            endings += ending to recording
        }
    }

    private val ShadowMediaRecorder.output: File get() = File(outputPath)

    /** The encoder writing: enough bytes to be a recording rather than an empty container. */
    private fun ShadowMediaRecorder.writeSound(bytes: Int = 4096) {
        output.writeBytes(ByteArray(bytes) { 1 })
    }

    @Test
    fun `it records the voice-note row with the five-minute cap and takes transient focus`() {
        val owner = Heard()

        assertThat(recorder.start(owner)).isTrue()

        val shadow = shadowOf(made.single())
        assertThat(shadow.state).isEqualTo(ShadowMediaRecorder.STATE_RECORDING)
        assertThat(shadow.maxDuration).isEqualTo(5 * 60 * 1000)
        assertThat(shadow.audioChannels).isEqualTo(1)
        assertThat(shadow.audioSamplingRate).isEqualTo(44_100)
        assertThat(shadow.outputFormat).isEqualTo(MediaRecorder.OutputFormat.MPEG_4)
        assertThat(shadow.audioEncoder).isEqualTo(MediaRecorder.AudioEncoder.AAC)
        assertThat(shadow.infoListener).isNotNull()
        assertThat(shadow.errorListener).isNotNull()
        val focus = shadowOf(audioManager).lastAudioFocusRequest
        assertThat(focus.durationHint).isEqualTo(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT)
        assertThat(recorder.isRecording).isTrue()
    }

    @Test
    fun `the cap is heard, and the owner gets the five minutes it recorded`() {
        val owner = Heard()
        recorder.start(owner)
        val target = made.single()
        val shadow = shadowOf(target)
        shadow.writeSound()
        now += VoiceRecorder.MAX_DURATION_MS + 250L

        shadow.infoListener.onInfo(target, MediaRecorder.MEDIA_RECORDER_INFO_MAX_DURATION_REACHED, 0)

        val (ending, kept) = owner.endings.single()
        assertThat(ending).isEqualTo(VoiceRecorder.Ending.CAP)
        assertThat(kept!!.file).isEqualTo(shadow.output)
        // The recorder's own clock, held to the cap it enforces.
        assertThat(kept.durationMs).isEqualTo(VoiceRecorder.MAX_DURATION_MS.toLong())
        assertThat(recorder.isRecording).isFalse()
        assertThat(recorder.elapsedMs).isEqualTo(0)
        assertThat(shadow.state).isEqualTo(ShadowMediaRecorder.STATE_RELEASED)
        // The focus goes back with it.
        assertThat(shadowOf(audioManager).lastAbandonedAudioFocusRequest).isNotNull()
    }

    /** Any other info — a file-size warning, say — is not the end of anything. */
    @Test
    fun `other information from the recorder ends nothing`() {
        val owner = Heard()
        recorder.start(owner)
        val target = made.single()

        shadowOf(target).infoListener.onInfo(target, MediaRecorder.MEDIA_RECORDER_INFO_UNKNOWN, 0)

        assertThat(owner.endings).isEmpty()
        assertThat(recorder.isRecording).isTrue()
    }

    @Test
    fun `the counter runs on the recorder's own clock and stops with the recording`() {
        recorder.start(Heard())
        val shadow = shadowOf(made.single())
        shadow.writeSound()

        now += 42_000
        assertThat(recorder.elapsedMs).isEqualTo(42_000)

        val kept = recorder.stop()
        assertThat(kept!!.durationMs).isEqualTo(42_000)
        now += 10_000
        assertThat(recorder.elapsedMs).isEqualTo(0)
    }

    @Test
    fun `a recorder that fails hands back what is readable`() {
        val owner = Heard()
        recorder.start(owner)
        val target = made.single()
        shadowOf(target).writeSound()
        now += 8_000

        shadowOf(target).errorListener.onError(target, MediaRecorder.MEDIA_RECORDER_ERROR_UNKNOWN, 0)

        val (ending, kept) = owner.endings.single()
        assertThat(ending).isEqualTo(VoiceRecorder.Ending.FAILED)
        assertThat(kept!!.durationMs).isEqualTo(8_000)
        assertThat(recorder.isRecording).isFalse()
    }

    @Test
    fun `losing the audio to another app ends it, and a duck does not`() {
        val owner = Heard()
        recorder.start(owner)
        shadowOf(made.single()).writeSound()
        now += 3_000
        val listener = shadowOf(audioManager).lastAudioFocusRequest.listener

        listener.onAudioFocusChange(AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK)
        assertThat(owner.endings).isEmpty()
        assertThat(recorder.isRecording).isTrue()

        listener.onAudioFocusChange(AudioManager.AUDIOFOCUS_LOSS_TRANSIENT)
        val (ending, kept) = owner.endings.single()
        assertThat(ending).isEqualTo(VoiceRecorder.Ending.INTERRUPTED)
        assertThat(kept!!.durationMs).isEqualTo(3_000)
        assertThat(recorder.isRecording).isFalse()
    }

    /**
     * Another app STARTING TO PLAY takes the focus for good — a music app an
     * earbud tap resumes asks for AUDIOFOCUS_GAIN — and S4's "Another app
     * starts playing" row leaves a recording alone: it records on, and is
     * still the owner's to end, with everything it recorded. Only a
     * transient loss (above) is something else taking the audio.
     */
    @Test
    fun `another app's playback taking the focus for good records on`() {
        val owner = Heard()
        recorder.start(owner)
        shadowOf(made.single()).writeSound()
        now += 2_000

        shadowOf(audioManager).lastAudioFocusRequest.listener
            .onAudioFocusChange(AudioManager.AUDIOFOCUS_LOSS)

        assertThat(owner.endings).isEmpty()
        assertThat(recorder.isRecording).isTrue()
        assertThat(recorder.elapsedMs).isEqualTo(2_000)
        now += 1_000
        assertThat(recorder.stop()!!.durationMs).isEqualTo(3_000)
        assertThat(owner.endings).isEmpty()
    }

    /** One recording at a time in the whole app (S1.7): the first owner gets its recording back. */
    @Test
    fun `a second owner's start hands the first its recording back first`() {
        val first = Heard()
        val second = Heard()
        recorder.start(first)
        val firstShadow = shadowOf(made.first())
        firstShadow.writeSound()
        now += 2_000

        assertThat(recorder.start(second)).isTrue()

        val (ending, kept) = first.endings.single()
        assertThat(ending).isEqualTo(VoiceRecorder.Ending.SUPERSEDED)
        assertThat(kept!!.file).isEqualTo(firstShadow.output)
        assertThat(kept.durationMs).isEqualTo(2_000)
        assertThat(firstShadow.state).isEqualTo(ShadowMediaRecorder.STATE_RELEASED)
        assertThat(made).hasSize(2)
        // The new recording writes somewhere else: the stopped one is still
        // its owner's to move out, and must not be written over.
        assertThat(shadowOf(made[1]).output).isNotEqualTo(firstShadow.output)
        assertThat(kept.file.length()).isEqualTo(4096)
        assertThat(recorder.isRecording).isTrue()
        assertThat(second.endings).isEmpty()
    }

    /** The owner asked: a stop is not reported back to it. */
    @Test
    fun `a stop the owner asked for tells it nothing`() {
        val owner = Heard()
        recorder.start(owner)
        shadowOf(made.single()).writeSound()

        recorder.stop()

        assertThat(owner.endings).isEmpty()
    }

    /** A recording that never got any audio is a few bytes of container, not a voice note. */
    @Test
    fun `a recording with nothing in it is no recording`() {
        recorder.start(Heard())
        val shadow = shadowOf(made.single())
        shadow.writeSound(bytes = 100)

        assertThat(recorder.stop()).isNull()
        assertThat(shadow.output.exists()).isFalse()
    }

    @Test
    fun `cancel deletes the file and gives the focus back`() {
        recorder.start(Heard())
        val shadow = shadowOf(made.single())
        shadow.writeSound()

        recorder.cancel()

        assertThat(shadow.output.exists()).isFalse()
        assertThat(recorder.isRecording).isFalse()
        assertThat(shadowOf(audioManager).lastAbandonedAudioFocusRequest).isNotNull()
    }

    /**
     * The waveform (#79; protocol.md, "A voice note's waveform") is the meter
     * the composer read: every read is a peak, the first after a start is
     * dropped (MediaRecorder answers 0 to it whatever was heard), and the
     * rest go with the recording as the wire's 48 digits. Robolectric's
     * recorder hears nothing, so every peak is silence: level 0.
     */
    @Test
    fun `the meter's reads become the recording's waveform`() {
        recorder.start(Heard())
        val shadow = shadowOf(made.single())
        shadow.writeSound()
        repeat(6) { recorder.maxAmplitude() }

        val kept = recorder.stop()!!

        assertThat(kept.waveform).isEqualTo("0".repeat(48))
    }

    @Test
    fun `a recording whose meter was never read has no waveform, and the next one starts afresh`() {
        recorder.start(Heard())
        shadowOf(made.last()).writeSound()
        recorder.maxAmplitude()
        assertThat(recorder.stop()!!.waveform).isNull()

        recorder.start(Heard())
        shadowOf(made.last()).writeSound()
        // One read: the dropped first of THIS recording, not a peak left from the last.
        recorder.maxAmplitude()
        assertThat(recorder.stop()!!.waveform).isNull()
    }
}
