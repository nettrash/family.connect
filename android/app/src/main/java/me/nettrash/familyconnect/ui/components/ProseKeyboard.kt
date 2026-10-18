/*
 * ProseKeyboard.kt
 * Family Connect (Android)
 *
 * Capital letters where people write sentences (#97; the design doc,
 * docs/capitals-and-rich-markdown-2026-10-17.md, "Capital letters").
 *
 * Android needs no rule of its own for this — the keyboard applies its own
 * — but a keyboard only capitalises a field that ASKS, and before #97 no
 * field here asked, so most keyboards typed every message in lower case.
 * One value, used by every prose field, so the set cannot drift field by
 * field: the chat composer (which is also where a message is edited), the
 * thread reply, a board note, a list's line, an event's title, a poll's
 * question and options, and a report's note. NEVER a username, a password,
 * a server address, an invite code, a search field or a sticker label —
 * capitalising those changes what they mean.
 */

package me.nettrash.familyconnect.ui.components

import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.text.input.KeyboardCapitalization

object ProseKeyboard {

    /** A field people write sentences in. */
    val Sentences = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences)

    /**
     * A NAME of a place ("Grandma's House", "Central Park"): every word.
     * Only an event's place uses it — the one prose-adjacent field whose
     * content is a proper noun rather than a sentence.
     */
    val Words = KeyboardOptions(capitalization = KeyboardCapitalization.Words)
}
