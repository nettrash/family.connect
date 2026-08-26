/*
 * CallRecordWording.kt
 * Family Connect (Android)
 *
 * What a call record's bubble says (docs/protocol.md, "Voice calls" —
 * "a client that knows the object draws its OWN wording from the outcome,
 * the duration and which side of the call it was on"). Pure, so the table
 * is tested on the JVM; the composable maps each line to its string.
 */

package me.nettrash.familyconnect.ui.chat

import me.nettrash.familyconnect.data.net.dto.CallDto

sealed interface CallRecordLine {
    /** Answered and hung up: "Voice call · 3:42". */
    data class Completed(val durationSecs: Int) : CallRecordLine

    /** I called and nobody answered. */
    data object NoAnswer : CallRecordLine

    /** They called and I never answered. */
    data object Missed : CallRecordLine

    /** They said no to my call. */
    data object DeclinedByThem : CallRecordLine

    /** I said no to theirs. */
    data object DeclinedByMe : CallRecordLine

    /** The media never came up, or died; a duration when it was ever up. */
    data class Failed(val durationSecs: Int?) : CallRecordLine
}

object CallRecordWording {

    /** [isMine]: the record's sender — the CALLER — is me. */
    fun line(call: CallDto, isMine: Boolean): CallRecordLine = when (call.outcome) {
        CallDto.COMPLETED -> CallRecordLine.Completed(call.durationSecs ?: 0)
        CallDto.MISSED -> if (isMine) CallRecordLine.NoAnswer else CallRecordLine.Missed
        CallDto.DECLINED -> if (isMine) CallRecordLine.DeclinedByThem else CallRecordLine.DeclinedByMe
        else -> CallRecordLine.Failed(call.durationSecs)
    }

    /** "3:42", or "1:02:05" past an hour. */
    fun duration(secs: Int): String {
        val total = secs.coerceAtLeast(0)
        val hours = total / 3600
        val minutes = (total % 3600) / 60
        val seconds = total % 60
        return if (hours > 0) {
            "%d:%02d:%02d".format(hours, minutes, seconds)
        } else {
            "%d:%02d".format(minutes, seconds)
        }
    }
}
