/*
 * HeldMicHaloPixelTest.kt
 * Family Connect (Android)
 *
 * The held microphone's red halo is drawn WHOLE in the real composer (#79,
 * the approved design: the microphone at 1.35× with a 10-unit ring of the
 * recording red, which in the mockup spills past the bar). The slot sits in
 * the composer's tonal Surface, and a Material 3 Surface clips its content
 * to its bounds — so the top of the ring was cut off flat at the bar's top
 * edge. The slot's own pixel tests draw it alone and could not see that.
 *
 * In pixels, on Robolectric: a plain band above the composer, and the ring
 * must reach into it.
 */

package me.nettrash.familyconnect.ui.chat

import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.material3.surfaceColorAtElevation
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.unit.dp
import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.testutil.pixelsOf
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
@RunWith(RobolectricTestRunner::class)
@Config(qualifiers = "en")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class HeldMicHaloPixelTest {

    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    private val band = Color(0xFF00FF00)

    private fun showHeld() {
        compose.setContent {
            MaterialTheme(colorScheme = lightColorScheme()) {
                Column {
                    Box(
                        Modifier
                            .fillMaxWidth()
                            .height(80.dp)
                            .background(band)
                            .testTag("above"),
                    )
                    Box(Modifier.testTag("bar")) {
                    InputBar(
                        state = remember { TextFieldState() },
                        onSend = {},
                        replyDraft = null,
                        replyAuthorName = "",
                        onCancelReply = {},
                        focusRequester = remember { FocusRequester() },
                        isEditing = false,
                        onCancelEdit = {},
                        mediaState = ChatViewModel.MediaSendState.Idle,
                        staged = emptyList(),
                        onPickMedia = {},
                        onPickFile = {},
                        onPasteFromClipboard = {},
                        onPasteContent = { ChatViewModel.PasteResult.TEXT },
                        onPasteTruncated = {},
                        onTakePhoto = {},
                        onTakeVideo = {},
                        onRecordAudio = {},
                        showsRecordVoice = true,
                        slotInputs = ComposerSlot.SlotInputs(
                            recorderOpen = false,
                            recording = ComposerSlot.Recording.HELD,
                            editing = false,
                            draftBlank = true,
                            staged = false,
                            assistantChat = false,
                            canRecord = true,
                            call = false,
                            busy = false,
                            notSent = false,
                        ),
                        hold = RecordGesture.HoldState(
                            phase = RecordGesture.Phase.Holding(340.0, 780.0, rtl = false, armed = false),
                        ),
                        announcement = null,
                        recordingMs = 2_000L,
                        onStopRecording = {},
                        onDeleteRecording = {},
                        onUndoVoiceMessage = {},
                        onOtherAction = {},
                        showsPoll = false,
                        onStartPoll = {},
                        onDiscardStaged = {},
                        onDismissMediaError = {},
                        onShareLocation = {},
                        showsAssistantMention = false,
                        showsAssistantPicture = false,
                        onShowAssistantPicture = {},
                        showsDraw = false,
                        onAskForPicture = {},
                        pictureNotice = null,
                        mentionPictureNotice = null,
                        showsPictureDescriptionHint = false,
                        assistantProcessor = null,
                        assistantConsentNeeded = false,
                        assistantIsUnnamed = false,
                        onReviewAssistantConsent = {},
                    )
                    }
                }
            }
        }
        compose.waitForIdle()
    }

    @Test
    fun theHeldMicrophonesHaloIsNotCutOffAtTheTopOfTheComposer() {
        showHeld()
        val above = compose.onNodeWithTag("above").fetchSemanticsNode().boundsInWindow
        val slot = compose.onNodeWithContentDescription("Send voice message").fetchSemanticsNode().boundsInWindow
        val density = compose.density.density
        val pixels = compose.pixelsOf("above")
        val x = (slot.center.x - above.left).toInt()

        // The ring's outer edge: the 44-unit disc swollen 1.35×, plus 10.
        val reach = (22f * HELD_SCALE + 10f) * density
        val ringTop = slot.center.y - reach
        // Only meaningful if the ring does reach past the bar's top edge.
        assertThat(ringTop).isLessThan(above.bottom - 2 * density)

        // One unit above the bar's top edge, straight over the microphone: the
        // ring's red over the band, not the band alone.
        val y = (above.bottom - above.top - density).toInt()
        assertThat(pixels[x, y]).isNotEqualTo(band.toArgb())
        // And well clear of the ring, the band is untouched.
        assertThat(pixels[x, ((ringTop - above.top) - 3 * density).toInt()]).isEqualTo(band.toArgb())

        // The panel itself is still the colour the tonal Surface gave it.
        val bar = compose.pixelsOf("bar")
        assertThat(bar[(2 * density).toInt(), bar.height - (3 * density).toInt()])
            .isEqualTo(lightColorScheme().surfaceColorAtElevation(3.dp).toArgb())
    }
}
