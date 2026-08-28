/*
 * CallAudio.kt
 * Family Connect (Android)
 *
 * The platform's side of a call's sound: the communication audio mode,
 * audio focus (so music pauses), the earpiece/speaker route, and the
 * ringback the caller hears while the far side rings. Behind an interface
 * so CallManager's tests need no AudioManager.
 *
 * The ringback is an AudioTrack looping the one cycle RingbackTone
 * synthesises, tagged USAGE_VOICE_COMMUNICATION like the call's own audio
 * — NOT the signalling usage: that one rides the DTMF strategy, which
 * `setCommunicationDevice` does not steer, so a video call's speaker
 * default (or a headset) would have carried the voice and not the ring.
 * With the call's usage it plays at the voice-call volume and through
 * whatever the call is routed to. Nothing on the wire; CallManager starts
 * it on `call_ringing` and stops it on the answer or any end. Every step
 * is best effort: a tone must never take the call's signalling down with
 * it (a throw here would end the socket-frame collector for good).
 *
 * iOS counterpart: Core/Calls/CallRingback.swift (RingbackPlayer).
 */

package me.nettrash.familyconnect.calls

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioDeviceInfo
import android.media.AudioFocusRequest
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioTrack
import android.os.Build
import android.util.Log
import dagger.hilt.android.qualifiers.ApplicationContext
import java.util.Locale
import javax.inject.Inject
import javax.inject.Singleton

interface CallAudio {
    /**
     * A call is starting: take the communication mode and audio focus.
     * A VIDEO call starts on the SPEAKER — a screen watched at arm's
     * length is not held to the ear; a voice call keeps the earpiece.
     */
    fun begin(video: Boolean)

    /** The call is over: give both back, silence any ringback, and return to the earpiece. */
    fun end()

    fun setSpeaker(on: Boolean)

    /**
     * The far side is ringing: sound the caller's ringback until
     * [stopRingback]. Idempotent — a second start while one plays is a
     * no-op, so a duplicate `call_ringing` cannot restart the cadence.
     */
    fun startRingback()

    /** Silence the ringback; safe when nothing plays. */
    fun stopRingback()
}

@Singleton
class AndroidCallAudio @Inject constructor(
    @param:ApplicationContext private val context: Context,
) : CallAudio {

    private val manager: AudioManager get() = context.getSystemService(AudioManager::class.java)
    private var focus: AudioFocusRequest? = null
    private var previousMode = AudioManager.MODE_NORMAL
    private var ringback: AudioTrack? = null

    override fun begin(video: Boolean) {
        val audio = manager
        previousMode = audio.mode
        audio.mode = AudioManager.MODE_IN_COMMUNICATION
        val request = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT)
            .setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION)
                    .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                    .build(),
            )
            .build()
        focus = request
        audio.requestAudioFocus(request)
        if (video) setSpeaker(true)
    }

    override fun end() {
        val audio = manager
        stopRingback()
        setSpeaker(false)
        focus?.let(audio::abandonAudioFocusRequest)
        focus = null
        audio.mode = previousMode
    }

    override fun setSpeaker(on: Boolean) {
        val audio = manager
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            if (on) {
                audio.availableCommunicationDevices
                    .firstOrNull { it.type == AudioDeviceInfo.TYPE_BUILTIN_SPEAKER }
                    ?.let(audio::setCommunicationDevice)
            } else {
                audio.clearCommunicationDevice()
            }
        } else {
            @Suppress("DEPRECATION")
            audio.isSpeakerphoneOn = on
        }
    }

    override fun startRingback() {
        if (ringback != null) return
        val cadence = RingbackTone.cadence(Locale.getDefault().country)
        val pcm = cycles.getOrPut(cadence) { RingbackTone.cycle(cadence) }
        var track: AudioTrack? = null
        try {
            val built = AudioTrack.Builder()
                .setAudioAttributes(
                    AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION)
                        .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                        .build(),
                )
                .setAudioFormat(
                    AudioFormat.Builder()
                        .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                        .setSampleRate(RingbackTone.SAMPLE_RATE)
                        .setChannelMask(AudioFormat.CHANNEL_OUT_MONO)
                        .build(),
                )
                .setTransferMode(AudioTrack.MODE_STATIC)
                .setBufferSizeInBytes(pcm.size * 2)
                .build()
            track = built
            // A static track is STATE_NO_STATIC_DATA until its buffer is
            // written; only then is it initialised — and only an
            // initialised track may loop and play.
            val written = built.write(pcm, 0, pcm.size)
            check(written == pcm.size && built.state == AudioTrack.STATE_INITIALIZED) {
                "not initialised (wrote $written of ${pcm.size}, state ${built.state})"
            }
            check(built.setLoopPoints(0, pcm.size, -1) == AudioTrack.SUCCESS) { "loop points refused" }
            if (built.setVolume(RINGBACK_GAIN) != AudioTrack.SUCCESS) Log.w(TAG, "ringback gain refused")
            built.play()
            ringback = built
        } catch (error: Exception) {
            // No audio output at all (an emulator without one, a device in
            // a strange state), or a refusal along the way: the call still
            // rings, just silently — and the signalling is untouched.
            Log.w(TAG, "ringback not started: $error")
            track?.release()
        }
    }

    override fun stopRingback() {
        val track = ringback ?: return
        ringback = null
        try {
            // Gain to zero first: the answer lands anywhere in a burst, and
            // AudioFlinger's ramp takes the edge off what would be a click.
            track.setVolume(0f)
            track.stop()
        } catch (error: Exception) {
            // Never played, or already dead — nothing to stop, still ours
            // to release.
            Log.i(TAG, "ringback stop: $error")
        }
        track.release()
    }

    /** The synthesised cycles, one per cadence, kept once made (~96 KB at most). */
    private val cycles = java.util.EnumMap<RingbackTone.Cadence, ShortArray>(RingbackTone.Cadence::class.java)

    private companion object {
        const val TAG = "CallAudio"

        /**
         * Loud enough over a receiver, not painful over a speaker; the
         * tone itself already peaks at about -6 dBFS (RingbackTone), and
         * the voice-call volume scales it further. iOS uses the same.
         */
        const val RINGBACK_GAIN = 0.6f
    }
}
