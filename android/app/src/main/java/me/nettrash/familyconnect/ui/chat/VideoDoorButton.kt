/*
 * VideoDoorButton.kt
 * Family Connect (Android)
 *
 * The video button (#79, docs/audio-video-messages-2026-10-04.md, S1.4):
 * inside the empty text field, at its trailing edge — the space an empty
 * field is not using, so the composer row never needs room for another
 * control. A 22-dp glyph in a 48-dp target; "Record video message".
 *
 * Whether it shows, is dimmed or is live is ComposerSlot.videoDoor's — the
 * shared rule. What is decided here is only the press:
 *
 *  - Activation OPENS THE RECORDER (S3). A long press does nothing special.
 *  - It ignores activation for 600 ms after it APPEARS (S1.1): it appears
 *    beside the slot the moment a text Send empties the field, and a second
 *    tap that drifts left must not turn the camera on.
 *  - Dimmed (rows 7 and 8) is not disabled: it stays focusable and hittable,
 *    and activating it says why — the ChatViewModel's notice, the same
 *    sentence as the microphone's (S1.3).
 */

package me.nettrash.familyconnect.ui.chat

import android.os.SystemClock
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.VideoCameraFront
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import me.nettrash.familyconnect.R

@Composable
internal fun VideoDoorButton(
    door: ComposerSlot.Door,
    onOpen: () -> Unit,
    /** The monotonic clock the 600-ms guard is measured on — a test hands in its own. */
    uptime: () -> Long = SystemClock::uptimeMillis,
) {
    if (door == ComposerSlot.Door.Hidden) return
    // When THIS button appeared: remembered for as long as it stays composed.
    val appearedAt = remember { uptime() }
    val label = stringResource(R.string.s_record_video_message)
    val reason = (door as? ComposerSlot.Door.Dimmed)?.reason?.let { stringResource(RecordStrings.of(it)) }
    val dimmed = reason != null
    Box(
        modifier = Modifier
            .size(ComposerSlot.MIN_TARGET_ANDROID_DP.dp)
            .testTag("video-door")
            .clickable(role = Role.Button, onClickLabel = label) {
                if (uptime() - appearedAt < ComposerSlot.ACTIVATION_GUARD_MS) return@clickable
                onOpen()
            }
            .semantics {
                contentDescription = label
                // Dimmed is not disabled: TalkBack hears why (S6).
                if (reason != null) stateDescription = reason
            },
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            imageVector = Icons.Outlined.VideoCameraFront,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = if (dimmed) 0.38f else 1f),
            modifier = Modifier.size(22.dp),
        )
    }
}
