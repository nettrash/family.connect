/*
 * NowPlaying.kt
 * Family Connect (Android)
 *
 * The NOW-PLAYING OWNER (#79, docs/audio-video-messages-2026-10-04.md, S4,
 * S5.3): one per process, made by Hilt and handed to the whole tree by
 * MainActivity, so it outlives the activity a rotation, a fold or a resize
 * rebuilds. It is the [PlaybackCoordinator] every play control reports to —
 * one thing plays at a time across the app — plus the three things only an
 * owner outside the screen can do:
 *
 *  - AUDIO FOCUS. It asks for `AUDIOFOCUS_GAIN_TRANSIENT` when something of
 *    the app's starts playing and gives it back when nothing does; when
 *    another app or a call takes it, what plays PAUSES (S4: "Another app
 *    starts playing … pause"). Until now only calls asked for focus
 *    (calls/CallAudio.kt), so a voice note played straight over a podcast.
 *  - BECOMING NOISY. While something plays it listens for
 *    `ACTION_AUDIO_BECOMING_NOISY` — headphones pulled, Bluetooth gone — and
 *    pauses, so a message never jumps out of the phone's speaker (S4).
 *  - THE DOT. Which round videos THIS DEVICE has played (S5.2): kept per
 *    account — in the settings store, which a sign-out clears — never sent,
 *    and remembered for the newest 5 000. Voice messages keep a dot of their
 *    own the same way (#79, the approved design), under a key of their own.
 *  - THE VOICE SPEED. 1×, 1.5× or 2× (#79): a choice of this DEVICE, kept
 *    across a sign-out like the other voice-message choices.
 */

package me.nettrash.familyconnect.ui.chat

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.os.Handler
import android.os.Looper
import androidx.core.content.ContextCompat
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import dagger.hilt.android.qualifiers.ApplicationContext
import javax.inject.Inject
import javax.inject.Singleton
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.settings.VOICE_PLAYBACK_SPEED_KEY
import me.nettrash.familyconnect.di.AppScope

/**
 * What pauses playback from outside the app: the audio focus going and the
 * output becoming noisy (S4). A seam, so the coordinator's rules are tested
 * without the system's AudioManager.
 */
interface AudioInterruptions {
    /**
     * Something started playing. Hold the focus and listen until [end];
     * [onLost] pauses everything. Called again while held, it changes
     * nothing.
     */
    fun begin(onLost: () -> Unit)

    /** Nothing plays any more: give the focus back and stop listening. */
    fun end()

    companion object {
        /** A test's, a preview's: nothing outside ever interrupts. */
        val NONE: AudioInterruptions = object : AudioInterruptions {
            override fun begin(onLost: () -> Unit) = Unit
            override fun end() = Unit
        }
    }
}

/** The system's [AudioInterruptions]: transient audio focus and the noisy broadcast. */
class SystemAudioInterruptions(context: Context) : AudioInterruptions {

    private val appContext = context.applicationContext
    private val audio = appContext.getSystemService(AudioManager::class.java)
    private val main = Handler(Looper.getMainLooper())
    private var request: AudioFocusRequest? = null
    private var noisy: BroadcastReceiver? = null

    override fun begin(onLost: () -> Unit) {
        if (request != null) return
        val focus = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT)
            .setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_MEDIA)
                    .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                    .build(),
            )
            // A message is speech: ducked under somebody else's sound it is
            // missed, so it pauses instead of playing on quietly.
            .setWillPauseWhenDucked(true)
            .setOnAudioFocusChangeListener({ change ->
                when (change) {
                    AudioManager.AUDIOFOCUS_LOSS,
                    AudioManager.AUDIOFOCUS_LOSS_TRANSIENT,
                    AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK,
                    -> onLost()
                }
            }, main)
            .build()
        request = focus
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) {
                if (intent.action == AudioManager.ACTION_AUDIO_BECOMING_NOISY) onLost()
            }
        }
        noisy = receiver
        ContextCompat.registerReceiver(
            appContext,
            receiver,
            IntentFilter(AudioManager.ACTION_AUDIO_BECOMING_NOISY),
            ContextCompat.RECEIVER_NOT_EXPORTED,
        )
        // Refused — a call holds the audio: S4 says a call pauses playback,
        // so it does not start over one either. Posted, because the player
        // calling `started` is still finishing its own start.
        if (audio.requestAudioFocus(focus) == AudioManager.AUDIOFOCUS_REQUEST_FAILED) main.post { onLost() }
    }

    override fun end() {
        request?.let(audio::abandonAudioFocusRequest)
        request = null
        noisy?.let { receiver -> runCatching { appContext.unregisterReceiver(receiver) } }
        noisy = null
    }
}

/**
 * Which round videos THIS DEVICE has played (S5.2) — the dot beside a
 * circle's length is shown until its id is in here.
 */
interface PlayedRoundVideos {
    val ids: StateFlow<Set<Long>>

    fun markPlayed(attachmentId: Long)

    /** A test's, a preview's: forgotten with the process. */
    class InMemory : PlayedRoundVideos {
        private val state = MutableStateFlow<Set<Long>>(emptySet())
        override val ids: StateFlow<Set<Long>> = state
        override fun markPlayed(attachmentId: Long) = state.update { trim(it + attachmentId) }
    }

    companion object {
        /** The newest this many are remembered; an older one may get its dot back. */
        const val REMEMBERED = 5_000

        /**
         * Keep the [REMEMBERED] NEWEST — the highest ids, because attachment
         * ids only grow — so the set never grows without end.
         */
        fun trim(ids: Set<Long>): Set<Long> =
            if (ids.size <= REMEMBERED) ids else ids.sortedDescending().take(REMEMBERED).toSet()

        /** The stored spelling: ascending ids, comma-separated. */
        fun encode(ids: Set<Long>): String = ids.sorted().joinToString(",")

        fun decode(stored: String?): Set<Long> =
            stored?.split(',')?.mapNotNullTo(HashSet(), String::toLongOrNull).orEmpty()
    }
}

/**
 * The device's [PlayedRoundVideos], in the settings store. Under a key of
 * its own that [me.nettrash.familyconnect.data.settings.SettingsRepository]
 * does not keep across `resetKeepingServerUrl`, so a sign-out wipes it with
 * everything else the account held — and outside SettingsState, so a play
 * never re-emits the whole of the app's settings.
 */
class StoredPlayedRoundVideos(
    private val dataStore: DataStore<Preferences>,
    private val scope: CoroutineScope,
    /** [KEY] for the circles; [VOICE_KEY] for the voice messages' own dots. */
    private val key: Preferences.Key<String> = KEY,
) : PlayedRoundVideos {

    override val ids: StateFlow<Set<Long>> = dataStore.data
        .map { it[key] }
        .distinctUntilChanged()
        .map(PlayedRoundVideos::decode)
        .stateIn(scope, SharingStarted.Eagerly, emptySet())

    override fun markPlayed(attachmentId: Long) {
        scope.launch {
            dataStore.edit { prefs ->
                val now = PlayedRoundVideos.decode(prefs[key])
                if (attachmentId !in now) prefs[key] = PlayedRoundVideos.encode(PlayedRoundVideos.trim(now + attachmentId))
            }
        }
    }

    companion object {
        val KEY = stringPreferencesKey("played_round_videos")

        /** Which VOICE messages this device has played (#79) — per account, like the circles'. */
        val VOICE_KEY = stringPreferencesKey("played_voice_messages")
    }
}

/**
 * How fast voice messages play on THIS DEVICE (#79): 1×, then 1.5×, then 2×,
 * then 1× again — the speed chip and the menu's "Playback speed" step it, and
 * every voice bubble plays at it.
 */
interface VoiceSpeed {
    val speed: StateFlow<Float>

    fun set(speed: Float)

    /** The next step: 1× → 1.5× → 2× → 1×. */
    fun step() = set(next(speed.value))

    /** A test's, a preview's: forgotten with the process. */
    class InMemory(initial: Float = STEPS.first()) : VoiceSpeed {
        private val state = MutableStateFlow(normalised(initial))
        override val speed: StateFlow<Float> = state
        override fun set(speed: Float) {
            state.value = normalised(speed)
        }
    }

    companion object {
        val STEPS: List<Float> = listOf(1f, 1.5f, 2f)

        fun next(speed: Float): Float = STEPS[(STEPS.indexOf(normalised(speed)) + 1) % STEPS.size]

        /** Anything stored that is not one of the steps reads as 1×. */
        fun normalised(speed: Float?): Float = STEPS.firstOrNull { it == speed } ?: STEPS.first()
    }
}

/** The device's [VoiceSpeed], in the settings store — kept across a sign-out (SettingsRepository). */
class StoredVoiceSpeed(
    private val dataStore: DataStore<Preferences>,
    private val scope: CoroutineScope,
) : VoiceSpeed {

    override val speed: StateFlow<Float> = dataStore.data
        .map { VoiceSpeed.normalised(it[VOICE_PLAYBACK_SPEED_KEY]) }
        .distinctUntilChanged()
        .stateIn(scope, SharingStarted.Eagerly, VoiceSpeed.STEPS.first())

    override fun set(speed: Float) {
        scope.launch { dataStore.edit { it[VOICE_PLAYBACK_SPEED_KEY] = VoiceSpeed.normalised(speed) } }
    }
}

/** The app's now-playing owner — see the file header. */
@Singleton
class NowPlaying @Inject constructor(
    @ApplicationContext context: Context,
    dataStore: DataStore<Preferences>,
    @AppScope scope: CoroutineScope,
) : PlaybackCoordinator(
    interruptions = SystemAudioInterruptions(context),
    played = StoredPlayedRoundVideos(dataStore, scope),
    playedVoice = StoredPlayedRoundVideos(dataStore, scope, StoredPlayedRoundVideos.VOICE_KEY),
    voiceSpeed = StoredVoiceSpeed(dataStore, scope),
)
