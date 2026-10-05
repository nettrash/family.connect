/*
 * RecordStringsTest.kt
 * Family Connect (Android)
 *
 * The shared voice-message rules speak in the catalogue's English keys (#79,
 * S10) and this app in its string resources; RecordStrings is the one map
 * between them. Every entry's ENGLISH resource must be the key the reference
 * uses — so the vectors that hold the rules hold the words too, and a
 * reworded resource cannot quietly part from the catalogue the other
 * clients translate from.
 */

package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertWithMessage
import me.nettrash.familyconnect.ui.chat.ComposerSlot.Slot
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(qualifiers = "en")
class RecordStringsTest {

    private val resources = RuntimeEnvironment.getApplication().resources

    /** The English resource, its Android placeholders written as the catalogue's. */
    private fun english(id: Int): String =
        resources.getString(id).replace("%1\$s", "%@").replace("%1\$d", "%lld")

    @Test
    fun everyDimmedSentenceIsTheReferencesOwn() {
        for (reason in ComposerSlot.Dimmed.entries) {
            assertWithMessage(reason.name).that(english(RecordStrings.of(reason))).isEqualTo(reason.notice)
        }
    }

    @Test
    fun everyHintIsTheReferencesOwn() {
        for (hint in RecordGesture.Hint.entries) {
            assertWithMessage(hint.name).that(english(RecordStrings.of(hint))).isEqualTo(hint.text)
        }
    }

    @Test
    fun everyAnnouncementIsTheReferencesOwn() {
        val all = listOf(
            RecordGesture.Announcement.Recording,
            RecordGesture.Announcement.RecordingLocked,
            RecordGesture.Announcement.RecordingDeleted,
            RecordGesture.Announcement.VoiceMessageSent,
            RecordGesture.Announcement.ReadyToReview(42_000),
            RecordGesture.Announcement.TooShort,
            RecordGesture.Announcement.StoppedAtFiveMinutes,
        )
        for (announcement in all) {
            assertWithMessage(announcement.toString())
                .that(english(RecordStrings.of(announcement)))
                .isEqualTo(announcement.text)
        }
    }

    /** S6's labels for every row the slot can be. */
    @Test
    fun everySlotLabelIsTheReferencesOwn() {
        val all = listOf(
            Slot.Recorder, Slot.HeldMicrophone, Slot.SendVoice, Slot.StopRecording, Slot.Save(true),
            Slot.Save(false), Slot.Send, Slot.SendDisabled, Slot.Dimmed(ComposerSlot.Dimmed.CALL),
            Slot.Dimmed(ComposerSlot.Dimmed.BUSY), Slot.Dimmed(ComposerSlot.Dimmed.NOT_SENT), Slot.Microphone,
        )
        for (slot in all) {
            assertWithMessage(slot.toString())
                .that(RecordStrings.label(slot)?.let(::english))
                .isEqualTo(slot.label)
        }
    }
}
