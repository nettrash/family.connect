/*
 * TranscriptOutcome.kt
 * Family Connect (Android)
 *
 * What asking for the text of a recording came to (docs/protocol.md,
 * "Transcripts on request").
 */

package me.nettrash.familyconnect.data.repo

import me.nettrash.familyconnect.data.net.ApiResult

/**
 * More than "text or null", because the failures read differently: a
 * recording the provider's filter refused is refused every time, so it is
 * not offered again; a recording this member may not ask about, or that
 * this server cannot send, is "not available"; an asker who has not agreed
 * to the assistant is ASKED, not told it failed; and anything else may work
 * on a second try.
 */
sealed interface TranscriptOutcome {
    /** The answer. [text] `""` means nothing was said: "No speech", not an error. */
    data class Text(val text: String, val language: String?) : TranscriptOutcome

    /** `transcript_refused`: the provider's filter refused this recording. Terminal. */
    data object Refused : TranscriptOutcome

    /**
     * `not_transcribable`, `transcript_not_allowed` or
     * `transcripts_unavailable`: asking again will not change the answer.
     */
    data object Unavailable : TranscriptOutcome

    /** Anything else — `internal`, a timeout, no connection: try again. */
    data object Failed : TranscriptOutcome

    /**
     * The sound this device could make is over `transcribe_max_bytes` even
     * at 64 kbit/s — or the recording's stated length already says so.
     * Nothing was sent. Terminal.
     */
    data object TooLong : TranscriptOutcome

    /**
     * This device could not take sound out of the file: it has none, or
     * none this device can decode. Nothing was sent. Terminal.
     */
    data object Unreadable : TranscriptOutcome

    /**
     * `assistant_consent_required` (403): the ASKER has not agreed, so no
     * sound was sent. Answered with the consent screen, then the same
     * request again.
     */
    data object ConsentRequired : TranscriptOutcome

    companion object {
        const val TRANSCRIPT_REFUSED = "transcript_refused"
        const val NOT_TRANSCRIBABLE = "not_transcribable"
        const val TRANSCRIPT_NOT_ALLOWED = "transcript_not_allowed"
        const val TRANSCRIPTS_UNAVAILABLE = "transcripts_unavailable"
        const val ASSISTANT_CONSENT_REQUIRED = "assistant_consent_required"

        /**
         * A failed request, read. Only the CODE decides — never the status,
         * which the codes share (four of them are 403), nor the message,
         * which is the server's English.
         */
        fun ofFailure(result: ApiResult<*>): TranscriptOutcome = when {
            result !is ApiResult.HttpError -> Failed
            result.code == TRANSCRIPT_REFUSED -> Refused
            result.code == ASSISTANT_CONSENT_REQUIRED -> ConsentRequired
            result.code == NOT_TRANSCRIBABLE ||
                result.code == TRANSCRIPT_NOT_ALLOWED ||
                result.code == TRANSCRIPTS_UNAVAILABLE -> Unavailable
            else -> Failed
        }
    }
}
