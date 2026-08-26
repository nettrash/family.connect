package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.net.dto.CallDto
import me.nettrash.familyconnect.data.repo.MessageRepository
import org.junit.Test

/**
 * The wording table a call record's bubble draws from (docs/protocol.md,
 * "Voice calls": outcome, duration, and which side of the call it was on),
 * and the chat-list preview beside it.
 */
class CallRecordWordingTest {

    @Test
    fun aCompletedCallShowsItsDurationOnBothSides() {
        val call = CallDto(outcome = "completed", durationSecs = 222)
        assertThat(CallRecordWording.line(call, isMine = true)).isEqualTo(CallRecordLine.Completed(222))
        assertThat(CallRecordWording.line(call, isMine = false)).isEqualTo(CallRecordLine.Completed(222))
    }

    @Test
    fun aMissedCallIsNoAnswerToTheCallerAndMissedToTheCallee() {
        val call = CallDto(outcome = "missed")
        assertThat(CallRecordWording.line(call, isMine = true)).isEqualTo(CallRecordLine.NoAnswer)
        assertThat(CallRecordWording.line(call, isMine = false)).isEqualTo(CallRecordLine.Missed)
    }

    @Test
    fun aDeclinedCallNamesWhoDeclined() {
        val call = CallDto(outcome = "declined")
        // The record's sender is the CALLER, so "mine" means they declined me.
        assertThat(CallRecordWording.line(call, isMine = true)).isEqualTo(CallRecordLine.DeclinedByThem)
        assertThat(CallRecordWording.line(call, isMine = false)).isEqualTo(CallRecordLine.DeclinedByMe)
    }

    @Test
    fun aFailedCallCarriesADurationOnlyWhenItWasEverUp() {
        assertThat(CallRecordWording.line(CallDto(outcome = "failed"), isMine = true))
            .isEqualTo(CallRecordLine.Failed(null))
        assertThat(CallRecordWording.line(CallDto(outcome = "failed", durationSecs = 12), isMine = false))
            .isEqualTo(CallRecordLine.Failed(12))
    }

    @Test
    fun anUnknownOutcomeReadsAsFailedRatherThanCrashing() {
        // Forward compatibility: a future outcome must not blank the bubble.
        assertThat(CallRecordWording.line(CallDto(outcome = "video"), isMine = true))
            .isEqualTo(CallRecordLine.Failed(null))
    }

    @Test
    fun durationsFormatAsMinutesAndSecondsAndGrowAnHourField() {
        assertThat(CallRecordWording.duration(0)).isEqualTo("0:00")
        assertThat(CallRecordWording.duration(7)).isEqualTo("0:07")
        assertThat(CallRecordWording.duration(222)).isEqualTo("3:42")
        assertThat(CallRecordWording.duration(3725)).isEqualTo("1:02:05")
        assertThat(CallRecordWording.duration(-5)).isEqualTo("0:00")
    }

    @Test
    fun theChatListPreviewSaysWhatTheServerBodySays() {
        // The record's body IS this text on the wire; the preview says the
        // same thing in the same words, and a call beats any body.
        assertThat(MessageRepository.previewText("Voice call", null, CallDto("completed", 222)))
            .isEqualTo("Voice call")
        assertThat(MessageRepository.previewText("Missed voice call", null, CallDto("missed")))
            .isEqualTo("Missed voice call")
        assertThat(MessageRepository.previewText("whatever", null, CallDto("declined")))
            .isEqualTo("Voice call")
        // Untouched without one.
        assertThat(MessageRepository.previewText("Dinner at 7?", null)).isEqualTo("Dinner at 7?")
    }
}
