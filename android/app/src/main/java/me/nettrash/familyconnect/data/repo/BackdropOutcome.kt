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
 * Three answers rather than "a picture or null", because two of them read
 * differently: a title the AI provider's own filter refused to draw
 * (`picture_refused`) gets the same refusal every time, so it says to put
 * it another way, where any other failure says only that it did not draw.
 */
sealed interface BackdropOutcome {
    /** The picture that landed — drawn over the note that is open. */
    data class Drawn(override val picture: AttachmentDto) : BackdropOutcome

    /** The provider's own filter refused to draw this title. Terminal. */
    data object Refused : BackdropOutcome

    /** Anything else: nothing was drawn, and asking again may work. */
    data object Failed : BackdropOutcome

    /** The picture, when one landed. */
    val picture: AttachmentDto? get() = null

    companion object {
        /** The protocol's code for a refused title: 400, terminal. */
        const val PICTURE_REFUSED = "picture_refused"

        /**
         * A failed request, read. Only the CODE decides a refusal — never
         * the status alone, which other 4xx share, nor the message, which
         * is the server's English and carries no provider text.
         */
        fun ofFailure(result: ApiResult<*>): BackdropOutcome =
            if (result is ApiResult.HttpError && result.code == PICTURE_REFUSED) Refused else Failed
    }
}
