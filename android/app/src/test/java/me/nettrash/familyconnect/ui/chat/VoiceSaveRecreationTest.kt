/*
 * VoiceSaveRecreationTest.kt
 * Family Connect (Android)
 *
 * A voice message's Save survives the screen being recreated while the
 * system's save screen is up (#79). MainActivity declares no configChanges,
 * so a rotation or a theme change recreates the chat; when the recording
 * being saved was held in plain `remember`, the answer came back to a screen
 * that had forgotten it, and the document the save screen had just made was
 * left behind empty.
 *
 * The save screen is scripted through the activity-result registry; the
 * recreation is Compose's own saved-state round trip.
 */

package me.nettrash.familyconnect.ui.chat

import android.net.Uri
import androidx.activity.ComponentActivity
import androidx.activity.compose.LocalActivityResultRegistryOwner
import androidx.activity.result.ActivityResultRegistry
import androidx.activity.result.ActivityResultRegistryOwner
import androidx.activity.result.contract.ActivityResultContract
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.core.app.ActivityOptionsCompat
import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class VoiceSaveRecreationTest {

    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    /** The system's save screen, answered by the test. */
    private class ScriptedRegistry : ActivityResultRegistry() {
        val launched = mutableListOf<Pair<Int, Any?>>()
        override fun <I, O> onLaunch(
            requestCode: Int,
            contract: ActivityResultContract<I, O>,
            input: I,
            options: ActivityOptionsCompat?,
        ) {
            launched += requestCode to input
        }
    }

    private val registry = ScriptedRegistry()
    private val owner = object : ActivityResultRegistryOwner {
        override val activityResultRegistry: ActivityResultRegistry = registry
    }

    private val note = AttachmentDto(id = 77, kind = AttachmentDto.KIND_AUDIO, mime = "audio/mp4", size = 4096, durationMs = 42_000)
    private val destination = Uri.parse("content://com.android.externalstorage.documents/document/primary%3Avoice-77.m4a")

    private val chosen = mutableListOf<Pair<AttachmentDto, Uri>>()
    private val orphaned = mutableListOf<Uri>()
    private var save: ((AttachmentDto) -> Unit)? = null

    @androidx.compose.runtime.Composable
    private fun Content() {
        CompositionLocalProvider(LocalActivityResultRegistryOwner provides owner) {
            save = rememberVoiceSave(
                onChosen = { attachment, uri -> chosen += attachment to uri },
                onOrphaned = { uri -> orphaned += uri },
            )
        }
    }

    @Test
    fun theRecordingIsStillKnownAfterTheScreenIsRecreated() {
        val restoration = StateRestorationTester(compose)
        restoration.setContent { Content() }
        compose.runOnIdle { save!!(note) }
        val (requestCode, input) = registry.launched.single()
        assertThat(input).isEqualTo("audio/mp4" to "voice-77.m4a")

        // Rotated while the save screen was up.
        restoration.emulateSavedInstanceStateRestore()
        compose.runOnIdle { registry.dispatchResult(requestCode, destination) }
        compose.waitForIdle()

        assertThat(chosen).containsExactly(note to destination)
        assertThat(orphaned).isEmpty()
    }

    @Test
    fun aCancelledSaveScreenDoesNothing() {
        compose.setContent { Content() }
        compose.runOnIdle { save!!(note) }
        val (requestCode, _) = registry.launched.single()
        compose.runOnIdle { registry.dispatchResult(requestCode, null) }
        compose.waitForIdle()

        assertThat(chosen).isEmpty()
        assertThat(orphaned).isEmpty()
    }

    @Test
    fun theSavedFormRoundTrips() {
        val saved = with(PendingAttachmentSaver) {
            androidx.compose.runtime.saveable.SaverScope { true }.save(note)
        }
        assertThat(saved).isNotNull()
        assertThat(PendingAttachmentSaver.restore(saved!!)).isEqualTo(note)
    }
}
