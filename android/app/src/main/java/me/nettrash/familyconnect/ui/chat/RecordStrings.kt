/*
 * RecordStrings.kt
 * Family Connect (Android)
 *
 * The shared voice-message rules speak in the catalogue's English keys
 * (ComposerSlot, RecordGesture — #79, S10); this app says each in its
 * reader's language through its own string resources. This is the one map
 * between the two, and RecordStringsTest holds every entry's ENGLISH
 * resource to the key the reference uses — so the vectors that check the
 * rules check the words too.
 */

package me.nettrash.familyconnect.ui.chat

import androidx.annotation.StringRes
import me.nettrash.familyconnect.R

object RecordStrings {

    /** A dimmed microphone's sentence (S1.3 rows 7–9). */
    @StringRes
    fun of(reason: ComposerSlot.Dimmed): Int = when (reason) {
        ComposerSlot.Dimmed.CALL -> R.string.e_record_after_the_call
        ComposerSlot.Dimmed.BUSY -> R.string.e_wait_for_the_attachment
        ComposerSlot.Dimmed.NOT_SENT -> R.string.e_send_or_delete_the_unsent_first
    }

    /** A line the reducer shows. */
    @StringRes
    fun of(hint: RecordGesture.Hint): Int = when (hint) {
        RecordGesture.Hint.STILL_RECORDING -> R.string.s_still_recording_tap_send
        RecordGesture.Hint.NEXT_TIME_SENDS -> R.string.s_next_time_letting_go_sends
        RecordGesture.Hint.NOTHING_HEARD -> R.string.s_we_didnt_hear_anything
        RecordGesture.Hint.STOPPED_AT_FIVE_MINUTES -> R.string.s_recording_stopped_at_five_minutes
        RecordGesture.Hint.TOO_SHORT -> R.string.e_recording_too_short
        RecordGesture.Hint.CAN_RECORD_NOW -> R.string.s_you_can_record_now
    }

    /** What is spoken; [RecordGesture.Announcement.ReadyToReview] takes its length as m:ss. */
    @StringRes
    fun of(announcement: RecordGesture.Announcement): Int = when (announcement) {
        RecordGesture.Announcement.Recording -> R.string.s_announce_recording
        RecordGesture.Announcement.RecordingLocked -> R.string.s_announce_recording_locked
        RecordGesture.Announcement.RecordingDeleted -> R.string.s_announce_recording_deleted
        RecordGesture.Announcement.VoiceMessageSent -> R.string.s_announce_voice_message_sent
        is RecordGesture.Announcement.ReadyToReview -> R.string.s_announce_ready_to_review
        RecordGesture.Announcement.TooShort -> R.string.e_recording_too_short
        RecordGesture.Announcement.StoppedAtFiveMinutes -> R.string.s_recording_stopped_at_five_minutes
    }

    /** The slot's TalkBack label (S6), or null where the recorder owns the row. */
    @StringRes
    fun label(slot: ComposerSlot.Slot): Int? = when (slot) {
        ComposerSlot.Slot.Recorder -> null
        ComposerSlot.Slot.HeldMicrophone, ComposerSlot.Slot.SendVoice -> R.string.s_send_voice_message
        ComposerSlot.Slot.StopRecording -> R.string.s_stop_recording
        is ComposerSlot.Slot.Save -> R.string.s_save
        ComposerSlot.Slot.Send, ComposerSlot.Slot.SendDisabled -> R.string.s_send
        is ComposerSlot.Slot.Dimmed, ComposerSlot.Slot.Microphone -> R.string.s_record_voice_message
    }
}
