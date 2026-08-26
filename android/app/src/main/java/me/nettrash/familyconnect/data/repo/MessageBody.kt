/*
 * MessageBody.kt
 * Family Connect (Android)
 *
 * How long a message body may be, measured the way the SERVER measures
 * it — and what happens to words that do not fit.
 *
 * The limit is 4000 characters (docs/protocol.md, "Limits") and until now
 * no client counted them anywhere: a pasted wall of text filled the
 * composer, looked as though it had worked, and came back from
 * `POST /chats/{id}/messages` as `message_too_long` at SEND — long after
 * the clipboard had moved on. A ceiling has to be applied where the text
 * ARRIVES, and said out loud when it bites.
 *
 * Three details make it a rule rather than a `length` check:
 *
 *  - CHARACTERS, not UTF-16 units. The server counts with Rust's
 *    `chars().count()`, which is Unicode scalar values; Kotlin's
 *    `String.length` counts UTF-16 units, and every emoji outside the
 *    BMP is two of those. Counting the Kotlin way would refuse a
 *    2001-emoji message the server would happily accept.
 *  - TRIMMED. `validate_body` trims before it counts, so trailing
 *    whitespace is not what pushes a draft over. (Kotlin trims by
 *    `Char.isWhitespace`, Rust by Unicode White_Space — the same set for
 *    anything a person types.)
 *  - CUTTING happens at GRAPHEME boundaries, not scalar ones. A cut in
 *    the middle of a cluster leaves an emoji's debris in the field: a
 *    skin tone with no hand, a flag with half its letters. The platform's
 *    own segmenter decides where those boundaries are.
 *
 * TRUNCATE rather than refuse, matching the Apple clients: what fits is
 * kept, and the sentence says the rest was not pasted. The clipboard
 * still holds all of it either way, and keeping the first 4000 characters
 * is nearly always closer to what somebody meant than keeping none.
 *
 * Pure and platform-free on purpose: every branch here is a plain unit
 * test, and the composer's input transformation and the attach menu's
 * Paste both defer to it rather than each carrying their own arithmetic.
 *
 * iOS counterpart: ios/FamilyConnect/Core/ComposerText.swift
 */

package me.nettrash.familyconnect.data.repo

import java.text.BreakIterator

object MessageBody {

    /**
     * The server's ceiling on a body (docs/protocol.md, "Limits" —
     * `limits.max_message_chars`, 4000 by default).
     *
     * It is configurable server-side and nothing on the wire announces
     * it, so this is the DEFAULT and a family that raised it simply gets
     * a client that is stricter than it had to be. Being stricter is the
     * safe direction: the alternative is a send that fails.
     */
    const val MAX_CHARS = 4000

    /**
     * The number of characters the server would count in [text].
     *
     * Unicode scalar values of the TRIMMED text, exactly as
     * `validate_body` counts them.
     */
    fun length(text: CharSequence): Int {
        val trimmed = text.trim()
        if (trimmed.isEmpty()) return 0
        return Character.codePointCount(trimmed, 0, trimmed.length)
    }

    /** Whether [text] is a body the server would accept the length of. */
    fun fits(text: CharSequence, limit: Int = MAX_CHARS): Boolean = length(text) <= limit

    /**
     * The space between what is already written and what is being added.
     *
     * A single space, unless there is nothing to separate or the draft
     * already ends in whitespace of its own.
     */
    fun separator(current: CharSequence): String = when {
        current.isEmpty() -> ""
        current.last().isWhitespace() -> ""
        else -> " "
    }

    // -- Cutting to fit -------------------------------------------------------

    /**
     * The UTF-16 index to cut [text] at so what remains is inside [limit],
     * or null when it already is.
     *
     * An INDEX rather than a string, because the one caller that matters
     * is editing a live buffer with somebody's caret in it: deleting the
     * tail leaves the caret where it was, while replacing the whole text
     * moves it.
     *
     * Whole graphemes only, so nothing is cut through the middle of an
     * emoji.
     */
    fun cutIndex(text: CharSequence, limit: Int = MAX_CHARS): Int? {
        if (fits(text, limit)) return null
        val string = text.toString()
        val clusters = BreakIterator.getCharacterInstance()
        clusters.setText(string)
        var characters = 0
        var end = clusters.first()
        var kept = end
        while (true) {
            val next = clusters.next()
            if (next == BreakIterator.DONE) break
            val width = Character.codePointCount(string, end, next)
            if (characters + width > limit) break
            characters += width
            kept = next
            end = next
        }
        return kept
    }

    /**
     * [text] cut down to [limit] characters, or null when it already fits.
     */
    fun cutToFit(text: CharSequence, limit: Int = MAX_CHARS): String? =
        cutIndex(text, limit)?.let { text.substring(0, it) }

    // -- Appending ------------------------------------------------------------

    /** What became of words appended to a draft. */
    sealed interface Paste {
        /** All of it fit. The new draft. */
        data class Appended(val draft: String) : Paste

        /** Some of it fit. The new draft, with the tail dropped. */
        data class Truncated(val draft: String) : Paste

        /** None of it fit — the draft is already at the ceiling. */
        data object Full : Paste
    }

    /**
     * Append pasted words to a draft, within the ceiling.
     *
     * Three outcomes rather than a boolean because they are three
     * different things to say: nothing, "the end was left out", and
     * "there was no room at all". A paste that quietly drops half of what
     * was on the clipboard is the failure this replaces.
     */
    fun appending(
        addition: CharSequence,
        draft: CharSequence,
        limit: Int = MAX_CHARS,
    ): Paste {
        val head = draft.toString() + separator(draft)
        val joined = head + addition
        if (fits(joined, limit)) return Paste.Appended(joined)
        val cut = cutToFit(joined, limit) ?: return Paste.Appended(joined)
        // Nothing of the ADDITION survived the cut: the draft was already
        // at the ceiling, and saying "the rest wasn't pasted" when none of
        // it was would be the wrong sentence.
        return if (cut.length <= head.length) Paste.Full else Paste.Truncated(cut)
    }

    // -- The composer's own buffer --------------------------------------------

    /** What a change to the composer's own buffer should do. */
    enum class Edit {
        /** It fits; leave it alone. */
        ACCEPT,

        /**
         * Over the limit, and it came a character at a time — somebody is
         * typing into a full field. Cut back in silence, the way every
         * length-capped field on this platform behaves; an error strip
         * flashing on every keystroke would be worse than the cap.
         */
        CUT_QUIETLY,

        /**
         * Over the limit, and it arrived ALL AT ONCE — a paste, a drop,
         * something a keyboard committed. Part of it visibly vanished, so
         * it has to be told.
         */
        CUT_AND_SAY,
    }

    /**
     * Judge one change to the composer, given what the buffer holds now
     * ([proposed]) and what it held before ([original]).
     *
     * Pure so the composer's InputTransformation carries no arithmetic and
     * no policy of its own — see ui/chat/BodyLengthLimit.kt.
     */
    fun review(
        proposed: CharSequence,
        original: CharSequence,
        limit: Int = MAX_CHARS,
    ): Edit = when {
        fits(proposed, limit) -> Edit.ACCEPT
        // Measured in UTF-16 units on purpose: this is not the limit, it
        // is "how much arrived", and one keystroke is one unit — except a
        // typed astral emoji, which is two and would be worth explaining
        // if it were what tipped the field over.
        proposed.length - original.length > 1 -> Edit.CUT_AND_SAY
        else -> Edit.CUT_QUIETLY
    }
}
