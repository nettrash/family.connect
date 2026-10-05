/*
 * AppVideoMessageRecorder.kt
 * Family Connect (Android)
 *
 * The app's ONE video message recorder (#79, Phase 3): VideoMessageRecorder
 * wired to the real camera (CameraXSession), MediaPrep's square pass, the
 * outbox, the call state and the server's limits. An app singleton — owned
 * outside the activity, so a rebuilt MainActivity finds the take still
 * running (S4, S8.4) — provided to the screens by MainActivity, as the
 * now-playing owner is.
 */

package me.nettrash.familyconnect.ui.chat

import android.content.Context
import android.os.SystemClock
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.geometry.Rect
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.calls.CallStateSource
import me.nettrash.familyconnect.data.repo.MediaPrep
import me.nettrash.familyconnect.data.repo.MessageRepository
import me.nettrash.familyconnect.data.repo.roundVideoLimits
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.util.Uptime
import java.io.File
import java.util.UUID
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class AppVideoMessageRecorder private constructor(
    deps: Deps,
) : VideoMessageRecorder(
    camera = deps.camera,
    clips = deps.clips,
    sink = deps.sink,
    calls = deps.calls,
    limits = { deps.settings.value.roundVideoLimits },
    teaching = deps.teaching,
    uptime = Uptime { SystemClock.uptimeMillis() },
    scope = deps.scope,
    newClipFile = deps.newClipFile,
    playback = deps.playback,
) {

    init {
        // Sign-out: everything recorded and not sent is deleted (S4).
        deps.scope.launch {
            var signedIn = false
            deps.settings.collect { state ->
                val now = state.myUserId != null
                if (signedIn && !now) signedOut()
                signedIn = now
            }
        }
    }

    /** The real camera, for the layer to hand its PreviewView to. */
    val cameraX: CameraXSession = deps.camera

    /**
     * The conversation pane in root pixels, as the open chat last laid it
     * out — where the recorder draws its circle and controls, and the
     * lighter part of the scrim (S3.3). Null: the whole window.
     */
    var paneBounds by mutableStateOf<Rect?>(null)

    @Inject
    constructor(
        @ApplicationContext context: Context,
        mediaPrep: MediaPrep,
        messageRepository: MessageRepository,
        settings: SettingsRepository,
        calls: CallStateSource,
        nowPlaying: NowPlaying,
    ) : this(Deps(context, mediaPrep, messageRepository, settings, calls, nowPlaying))

    private class Deps(
        context: Context,
        mediaPrep: MediaPrep,
        messageRepository: MessageRepository,
        settingsRepository: SettingsRepository,
        val calls: CallStateSource,
        val playback: NowPlaying,
    ) {
        /** Main: the camera, LifecycleRegistry and Transformer's callbacks all live there. */
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

        val settings: StateFlow<SettingsState> =
            settingsRepository.state.stateIn(scope, SharingStarted.Eagerly, SettingsState())

        val camera = CameraXSession(context)

        val clips = ClipPreparer { file, recordedMs, maxBytes ->
            val made = mediaPrep.prepareRoundVideo(file) ?: return@ClipPreparer null
            RoundClip(
                prepared = made.prepared,
                durationMs = made.prepared.durationMs?.toLong() ?: recordedMs,
                notRound = RoundRecorderRules.notRound(
                    prepared = made.prepared,
                    squared = made.squared,
                    sizeBytes = made.prepared.file.length(),
                    maxBytes = maxBytes,
                ),
                files = made.files,
            )
        }

        /** The outbox — the row written before the first byte goes up (S3.4, S5.6). */
        val sink = RoundVideoSink { chatId, clip, reply ->
            messageRepository.sendMedia(
                prepared = listOf(clip.prepared),
                caption = "",
                chatId = chatId,
                replyTo = reply,
                mentions = null,
                round = clip.round,
            ) != null
        }

        val teaching = object : PreviewTeaching {
            override fun taught(): Boolean = settings.value.roundPreviewTaught
            override fun markTaught() {
                scope.launch { settingsRepository.setRoundPreviewTaught() }
            }
        }

        /** In the cache: a REVIEW clip is kept while the app runs, never parked (S4). */
        private val directory = File(context.cacheDir, "round-video")

        val newClipFile: () -> File = {
            directory.mkdirs()
            File(directory, "round-${UUID.randomUUID()}.mp4")
        }
    }
}

/** The app's recorder, provided by MainActivity; null in a preview or a test that provides none. */
val LocalVideoMessageRecorder = staticCompositionLocalOf<AppVideoMessageRecorder?> { null }
