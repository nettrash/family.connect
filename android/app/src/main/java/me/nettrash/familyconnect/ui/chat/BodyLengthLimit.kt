/*
 * BodyLengthLimit.kt
 * Family Connect (Android)
 *
 * The composer's ceiling: a draft never grows past what the server will
 * accept as a body (docs/protocol.md, "Limits" — 4000 characters).
 *
 * Why a TRANSFORMATION and not a check on each paste door: the text
 * field's own paste hands the words back to the FIELD, which inserts them
 * at the caret — that is the whole reason this platform keeps a caret
 * paste while the attach menu appends. Nothing in this app ever sees that
 * insertion, so a per-door check would leave the one door that matters
 * most uncapped. An InputTransformation runs after every change the field
 * makes to its own buffer, whichever way the text arrived: typed,
 * committed by a keyboard, pasted, dropped, or set by an accessibility
 * service.
 *
 * CUT THE TAIL, not revert the whole change. `revertAllChanges` is what
 * `maxLength` does, and it is the worse half of the choice here: a paste
 * one character over the ceiling would vanish entirely, and the
 * clipboard has usually moved on by the time anybody notices. Keeping
 * what fits is what MessageBody.appending does at the attach menu's
 * Paste and what the Apple clients do (ComposerText), so both doors and
 * both platforms end at the same draft. The cut is a delete at the END,
 * so the caret stays where it was.
 *
 * The saying-why is rationed. A cut is REPORTED only when the change
 * added more than one character — a paste or a drop, something that
 * arrived all at once and visibly lost part of itself. Typing into a
 * full field is cut in silence, the way every length-capped field on
 * this platform behaves; an error strip flashing on every keystroke
 * would be worse than the cap itself.
 *
 * The counting is not here: [MessageBody] owns it, because the server
 * counts CHARACTERS (Unicode scalar values, trimmed) and a buffer's
 * `length` is UTF-16 units.
 */

package me.nettrash.familyconnect.ui.chat

import androidx.compose.foundation.text.input.InputTransformation
import androidx.compose.foundation.text.input.TextFieldBuffer
import androidx.compose.foundation.text.input.delete
import androidx.compose.ui.semantics.SemanticsPropertyReceiver
import androidx.compose.ui.semantics.maxTextLength
import me.nettrash.familyconnect.data.repo.MessageBody

class BodyLengthLimit(
    private val limit: Int = MessageBody.MAX_CHARS,
    /** Told once, when part of a paste or drop was cut off. */
    private val onRefused: () -> Unit,
) : InputTransformation {

    override fun SemanticsPropertyReceiver.applySemantics() {
        // An approximation on purpose: accessibility services want UTF-16
        // units and the protocol counts characters. They agree for every
        // message that is not mostly emoji, and the real gate is below.
        maxTextLength = limit
    }

    override fun TextFieldBuffer.transformInput() {
        // Judged before the revert: afterwards there is no change left to
        // judge. The judging itself is MessageBody's — this door carries
        // no arithmetic and no policy, only the two Compose calls that
        // nothing else can make.
        val verdict = MessageBody.review(
            proposed = asCharSequence(),
            original = originalText,
            limit = limit,
        )
        if (verdict == MessageBody.Edit.ACCEPT) return
        // The TAIL is dropped, never the whole change: cutting back to the
        // ceiling keeps what fits and, being a delete at the end, leaves
        // the caret wherever it already was.
        MessageBody.cutIndex(asCharSequence(), limit)?.let { delete(it, length) }
        if (verdict == MessageBody.Edit.CUT_AND_SAY) onRefused()
    }
}
