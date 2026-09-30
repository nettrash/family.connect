/*
 * BackdropOutcome.kt
 * Family Connect (Android)
 *
 * What asking the assistant for an event's backdrop came to.
 */

package me.nettrash.familyconnect.data.repo

import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.AttachmentDto

/**
 * What asking for a backdrop came to (docs/protocol.md, "Board").
 *
 * More than "a picture or null", because the failures read differently: a
 * title the AI provider's own filter refused to draw (`picture_refused`)
 * gets the same refusal every time, so it says to put it another way,
 * where any other failure says only that it did not draw — and an author
 * who has not consented to the assistant is asked, not told it failed.
 */
sealed interface BackdropOutcome {
    /** The picture that landed — drawn over the note that is open. */
    data class Drawn(override val picture: AttachmentDto) : BackdropOutcome

    /** The provider's own filter refused to draw this title. Terminal. */
    data object Refused : BackdropOutcome

    /** Anything else: nothing was drawn, and asking again may work. */
    data object Failed : BackdropOutcome

    /**
     * The server has no consent from this author for their words to go to
     * the model (`assistant_consent_required`, 403): nothing was sent. Not
     * a failure to report but a question to ask — the consent screen, and
     * then this backdrop again (docs/protocol.md, "Consenting to the
     * assistant").
     */
    data object ConsentRequired : BackdropOutcome

    /**
     * The author was asked for that consent and did not give it: nothing
     * was sent, and there is nothing to report — they have just said so.
     */
    data object Declined : BackdropOutcome

    /** The picture, when one landed. */
    val picture: AttachmentDto? get() = null

    companion object {
        /** The protocol's code for a refused title: 400, terminal. */
        const val PICTURE_REFUSED = "picture_refused"

        /** The protocol's code for an author who has not consented: 403. */
        const val ASSISTANT_CONSENT_REQUIRED = "assistant_consent_required"

        /**
         * A failed request, read. Only the CODE decides a refusal or a
         * missing consent — never the status alone, which other 4xx share
         * (`pictures_unavailable` and `not_note_author` are 403s too), nor
         * the message, which is the server's English and carries no
         * provider text.
         */
        fun ofFailure(result: ApiResult<*>): BackdropOutcome = when {
            result !is ApiResult.HttpError -> Failed
            result.code == PICTURE_REFUSED -> Refused
            result.code == ASSISTANT_CONSENT_REQUIRED -> ConsentRequired
            else -> Failed
        }
    }
}
