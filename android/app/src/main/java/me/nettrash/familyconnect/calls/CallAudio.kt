/*
 * CallAudio.kt
 * Family Connect (Android)
 *
 * The platform's side of a call's sound: the communication audio mode,
 * audio focus (so music pauses), and the earpiece/speaker route. Behind an
 * interface so CallManager's tests need no AudioManager.
 */

package me.nettrash.familyconnect.calls

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioDeviceInfo
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.os.Build
import dagger.hilt.android.qualifiers.ApplicationContext
import javax.inject.Inject
import javax.inject.Singleton

interface CallAudio {
    /** A call is starting: take the communication mode and audio focus. */
    fun begin()

    /** The call is over: give both back and return to the earpiece. */
    fun end()

    fun setSpeaker(on: Boolean)
}

@Singleton
class AndroidCallAudio @Inject constructor(
    @param:ApplicationContext private val context: Context,
) : CallAudio {

    private val manager: AudioManager get() = context.getSystemService(AudioManager::class.java)
    private var focus: AudioFocusRequest? = null
    private var previousMode = AudioManager.MODE_NORMAL

    override fun begin() {
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
    }

    override fun end() {
        val audio = manager
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
}
