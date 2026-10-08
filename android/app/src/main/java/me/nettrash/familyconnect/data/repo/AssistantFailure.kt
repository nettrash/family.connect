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
 * Two cases and no more, because the protocol defines one `reason` and says
 * a client must read any other as absent.
 */
enum class AssistantFailure {
    /** Any failure the server gave no reason for: asking again may work. */
    STOPPED,

    /**
     * The AI provider's own safety or content filter refused it. Asking
     * again in the same words gets the same refusal, so the sentence says
     * to put it another way instead.
     */
    REFUSED;

    companion object {
        /** An unknown `reason` is ABSENT: [STOPPED], never a guess. */
        fun of(frame: ServerFrame.AiError): AssistantFailure =
            if (frame.isRefused) REFUSED else STOPPED
    }
}
