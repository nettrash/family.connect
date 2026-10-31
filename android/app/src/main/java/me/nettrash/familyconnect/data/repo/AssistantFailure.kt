/*
 * AssistantFailure.kt
 * Family Connect (Android)
 *
 * Why an assistant answer failed, as far as this client may say.
 */

package me.nettrash.familyconnect.data.repo

import me.nettrash.familyconnect.data.net.ws.ServerFrame

/**
 * How an assistant answer failed — what the failed row REMEMBERS alongside
 * the fact that it failed, so a redraw says the same sentence it said the
 * first time (docs/protocol.md, "The assistant": "the failed row remembers
 * WHICH sentence it failed with for exactly as long as it remembers that it
 * failed").
 *
 * One case per `reason` the protocol defines, and one for none: a client
 * must read any other `reason` as absent.
 */
sealed interface AssistantFailure {
    /** Any failure the server gave no reason for: asking again may work. */
    data object Stopped : AssistantFailure

    /**
     * The AI provider's own safety or content filter refused it. Asking
     * again in the same words gets the same refusal, so the sentence says
     * to put it another way instead.
     */
    data object Refused : AssistantFailure

    /**
     * A PAID family has used this month's assistant allowance and the
     * provider was never called (docs/protocol.md, "Paid families", added
     * 2026-10-30). The sentence names the day it starts again,
     * [renewsAtMillis] — remembered with the row, so a redraw names the
     * same day.
     */
    data class AllowanceUsed(val renewsAtMillis: Long) : AssistantFailure

    companion object {
        /**
         * An unknown `reason` is ABSENT: [Stopped], never a guess. An
         * `allowance_used` without a readable `renews_at` names the start of
         * next month, which is what the server would have sent.
         */
        fun of(frame: ServerFrame.AiError, nowMillis: Long = System.currentTimeMillis()): AssistantFailure = when {
            frame.isRefused -> Refused
            frame.isAllowanceUsed -> AllowanceUsed(PaidFamilies.renewsAt(frame.renewsAt, nowMillis))
            else -> Stopped
        }
    }
}
