package me.nettrash.familyconnect.ui.chat

/**
 * Recognising `@ai` in a message body (docs/protocol.md, "Mentioning the
 * assistant in the family chat").
 *
 * This grammar is a WIRE CONTRACT, not a rendering detail. The SERVER
 * decides from it whether a family-chat message reaches the assistant at
 * all, and this file decides where the highlight goes. If the two disagree,
 * a family watches a highlighted question go unanswered with no way to tell
 * why — or, worse, an ordinary message quietly leaves the building.
 * Mirrored by value in `server/src/mentions.rs` and
 * `ios/FamilyConnect/Models/AssistantMention.swift`, with the same vectors
 * pinned on all three. Change it in three places or it is a bug.
 *
 * The rule, in full:
 *
 * - the token is the three characters `@ai`, matched case-insensitively but
 *   only over ASCII, so `@AI` and `@Ai` are mentions;
 * - the `@` must start the body or follow a character that is not an ASCII
 *   letter, digit or `_` — which is what stops `anna@ai.example`;
 * - the `i` must end the body or be followed by a character that is not an
 *   ASCII letter, digit or `_` — which is what stops `@aiden`.
 *
 * The boundary test is ASCII-only on purpose. A Unicode one would refuse
 * `@ai` written against Japanese or Russian with no space after it —
 * languages this app is translated into, where a space would not normally be
 * typed — and it would make three implementations depend on three Unicode
 * tables agreeing, which is exactly the trap `EmojiOnly` had to hand-roll
 * its own whitespace table to escape. The ambiguity the boundary exists to
 * resolve (`@aiden`) only arises in ASCII anyway.
 */
object AssistantMention {
    /** The token itself, so nothing spells it twice. */
    const val TOKEN = "@ai"

    /** Does this body address the assistant? */
    fun mentions(body: String): Boolean = ranges(body).isNotEmpty()

    /**
     * Every `@ai` in [body], as ranges of UTF-16 code-unit indices — which
     * is what `AnnotatedString` spans and `String.substring` both want.
     *
     * ALL of them, not just the first: the server only needs to know whether
     * there is one, but a bubble highlighting the first and leaving a second
     * plain would look like a typo.
     *
     * Indices are Kotlin's own (UTF-16) rather than the server's byte
     * offsets, on purpose: nothing compares the two, and converting here
     * would be an invitation to slice a string with an offset counted in the
     * wrong unit — the exact class of bug the link-preview parsers were
     * rewritten to eliminate.
     */
    fun ranges(body: String): List<IntRange> {
        if (body.length < 3) return emptyList()
        val found = mutableListOf<IntRange>()
        for (index in 0..body.length - 3) {
            if (body[index] != '@') continue
            val a = body[index + 1]
            val i = body[index + 2]
            if (!(a == 'a' || a == 'A') || !(i == 'i' || i == 'I')) continue
            if (index > 0 && !isBoundary(body[index - 1])) continue
            if (index + 3 < body.length && !isBoundary(body[index + 3])) continue
            found += index until index + 3
        }
        return found
    }

    /**
     * ASCII letters, digits and `_` are what a token may NOT sit against.
     * Anything else — punctuation, whitespace, any non-ASCII character — is
     * a boundary.
     */
    private fun isBoundary(char: Char): Boolean =
        !(char in '0'..'9' || char in 'a'..'z' || char in 'A'..'Z' || char == '_')
}
