package me.nettrash.familyconnect.ui.chat

/**
 * Recognising what a message body ASKS the assistant for: `@ai`
 * (docs/protocol.md, "Mentioning the assistant in the family chat") and
 * `/draw` (protocol.md, "Pictures").
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

    // -- Pictures ---------------------------------------------------------

    /** The picture token, so nothing spells it twice. */
    const val DRAW = "/draw"

    /**
     * The picture this body asks for, or null because it asks for none
     * (docs/protocol.md, "Pictures").
     *
     * The SECOND wire-contract grammar in this file, and a contract for
     * the same reason the first one is: the SERVER decides from it
     * whether a request goes to an entirely different provider, and this
     * client must highlight and offer exactly what the server will act
     * on. Mirrored by value in `server/src/mentions.rs` (`draw_prompt`)
     * and `ios/FamilyConnect/Models/AssistantMention.swift`, with the
     * same vectors pinned on all three — change it in three places or it
     * is a bug.
     *
     * The rule, in full:
     *
     * - the token is the five characters `/draw`, matched
     *   case-insensitively over ASCII, so `/DRAW` and `/Draw` ask too;
     * - it must be the FIRST thing in the body, ignoring leading
     *   whitespace and — this is what makes the family chat work — one
     *   leading `@ai` and the whitespace after it. `@ai /draw a cat` asks
     *   for a picture; `hey @ai /draw a cat` does not, and neither does
     *   `what does /draw do?`;
     * - it must be followed by whitespace or the end of the body, so
     *   `/drawer` is just a word;
     * - what follows, trimmed, is the PROMPT and must not be empty.
     *   `/draw` alone is an ordinary message.
     *
     * "Whitespace" throughout is Unicode `White_Space` and NOT Kotlin's
     * `Char.isWhitespace()`, which answers differently for five code
     * points - see [isUnicodeWhitespace].
     *
     * First-and-nowhere-else is the same decision the `@ai` boundary
     * rules made: a token that could hide anywhere in a sentence would
     * turn a family DISCUSSING this feature into a family generating
     * pictures of it.
     *
     * What comes back is the WHOLE of what will leave the server on this
     * request — not the thread, not the transcript, not the family's
     * language, and not any picture the message carries.
     */
    fun drawPrompt(body: String): String? {
        val start = tokenStart(body) ?: return null
        return body.substring(start + DRAW.length)
            .trim { it.isUnicodeWhitespace() }
            .takeIf { it.isNotEmpty() }
    }

    /**
     * Where the picture token sits in [body], as UTF-16 code-unit indices
     * — or null because this body asks for no picture.
     *
     * The highlight's half of the same contract [drawPrompt] reads, and
     * it is the reason both are computed by ONE scan: a bubble that
     * highlighted a token the server will not act on, or left one plain
     * that it will, puts the "does this leave the server, and to whom"
     * question somewhere a reader cannot see at a glance — which is the
     * very thing first-and-nowhere-else was chosen to avoid.
     *
     * The TOKEN alone, never the prompt: the words after it are the
     * member's own and read as ordinary text, exactly as the words around
     * a highlighted `@ai` do.
     */
    fun drawRange(body: String): IntRange? =
        tokenStart(body)?.let { it until it + DRAW.length }

    /**
     * The index of the picture token, if this body is a request.
     *
     * Scans over the ORIGINAL string rather than over trimmed copies, so
     * the index it returns is one the caller can slice with. The rule
     * itself is [drawPrompt]'s.
     */
    private fun tokenStart(body: String): Int? {
        var index = skipWhitespace(body, 0)
        // One LEADING mention, and only a leading one: `ranges` finds the
        // token anywhere, so the position is checked rather than trusted.
        // Nothing but whitespace precedes `index`, so a mention starting
        // there is necessarily the first one in the whole body.
        val leading = ranges(body).firstOrNull()
        if (leading != null && leading.first == index) {
            index = skipWhitespace(body, leading.last + 1)
        }
        if (index + DRAW.length > body.length) return null
        // ASCII folding, hand-written, exactly as the mention's boundary
        // test is: the server compares BYTES with `eq_ignore_ascii_case`,
        // and Kotlin's `ignoreCase` is a Unicode fold that would answer
        // differently for characters outside ASCII. Three implementations
        // agreeing is the whole point of this file.
        for (offset in DRAW.indices) {
            if (!body[index + offset].equalsIgnoringAsciiCase(DRAW[offset])) return null
        }
        // End of body, or whitespace. Anything else is a longer word:
        // `/drawer` is not a request, and neither is `/draw,`.
        val after = index + DRAW.length
        if (after >= body.length) return null
        if (!body[after].isUnicodeWhitespace()) return null
        // …and the prompt itself must not be empty: `/draw` with nothing
        // but spaces after it is an ordinary message.
        if (body.substring(after).all { it.isUnicodeWhitespace() }) return null
        return index
    }

    private fun skipWhitespace(body: String, from: Int): Int {
        var index = from
        while (index < body.length && body[index].isUnicodeWhitespace()) index++
        return index
    }

    /**
     * Unicode `White_Space`, hand-rolled - the SERVER's definition of
     * whitespace and therefore this grammar's.
     *
     * Kotlin's `Char.isWhitespace()` is not it. It is Java's
     * `Character.isWhitespace(c) || Character.isSpaceChar(c)`, and that
     * union disagrees with Unicode on FIVE code points:
     *
     * - U+001C FILE SEPARATOR, U+001D GROUP SEPARATOR, U+001E RECORD
     *   SEPARATOR and U+001F UNIT SEPARATOR - Java calls these whitespace,
     *   Unicode does not. Rust's `char::is_whitespace` says no, so
     *   `/draw\u001Ca cat` is the token followed by a longer word and the
     *   server ignores it - while this file used to read a picture request
     *   out of it and highlight one.
     * - U+0085 NEXT LINE - Unicode calls it whitespace, Java does not. So
     *   `/draw\u0085a cat` IS a picture request, the server acts on it,
     *   and this file used to call it an ordinary message.
     *
     * Both directions are the failure this grammar exists to prevent: a
     * highlight the server will not act on, or a token it will act on left
     * plain. The table below is the WHOLE property rather than the five
     * code points that differ, so it can be read against
     * `server/src/mentions.rs` (`whitespace_is_the_unicode_property_and_nothing_else`)
     * instead of trusted as a summary. Apple's side hand-rolls the same
     * table for the same reason, against a THIRD answer: Foundation's
     * `CharacterSet.whitespacesAndNewlines` contains U+200B ZERO WIDTH
     * SPACE, which is not `White_Space` either.
     *
     * Every code point in the property is in the BMP, so one `Char` holds
     * each of them and there is no surrogate pair to think about.
     */
    private fun Char.isUnicodeWhitespace(): Boolean = when (this) {
        '\u0009', '\u000A', '\u000B', '\u000C', '\u000D',
        '\u0020', '\u0085', '\u00A0', '\u1680',
        '\u2028', '\u2029', '\u202F', '\u205F', '\u3000',
        -> true
        // U+2000..U+200A, the en/em/thin/hair quad. U+200B ZERO WIDTH SPACE
        // sits just past the end of it on purpose: it is not White_Space.
        else -> this in '\u2000'..'\u200A'
    }

    /** ASCII-only case folding — see [drawPrompt]. */
    private fun Char.equalsIgnoringAsciiCase(other: Char): Boolean =
        this == other || (this in 'A'..'Z' && this + 32 == other) ||
            (this in 'a'..'z' && this - 32 == other)

    /**
     * The body that composer's "ask for a picture" button should leave
     * behind, given what is already typed.
     *
     * The token cannot simply be appended the way `@ai` is — it has to be
     * FIRST, and in the family chat it has to sit after one leading
     * mention — so the prefix is BUILT here rather than guessed at the
     * call site, from the same grammar [drawPrompt] reads. A body that
     * already asks for a picture is returned untouched: pressing the
     * button twice must not produce `/draw /draw`, which the server would
     * read as a request to draw the word.
     *
     * [inFamilyChat] adds the mention, because there the server only
     * looks for the picture token on a message that already mentioned the
     * assistant. In an `ai` chat every message is addressed to it already
     * and a mention would just be five characters of prompt.
     */
    fun withDraw(body: String, inFamilyChat: Boolean): String {
        if (drawPrompt(body) != null) return body
        val prefix = if (inFamilyChat) "$TOKEN $DRAW " else "$DRAW "
        // The typed words are kept, trimmed at the FRONT only: leading
        // space would push the token off the start and make the whole
        // thing an ordinary message, while a trailing one is the caret
        // sitting where the writer left it. Trimmed by the SERVER's
        // definition of whitespace ([isUnicodeWhitespace]), because the
        // body this builds has to read as a request to the server and not
        // merely to Kotlin.
        val rest = body.trimStart { it.isUnicodeWhitespace() }
        // A leading mention the writer typed themselves is dropped rather
        // than doubled — `@ai @ai /draw` has a second mention that is not
        // a leading one, which is exactly a NOT_DRAW vector.
        val existing = ranges(rest).firstOrNull()
        val words = if (existing != null && existing.first == 0) {
            rest.substring(existing.last + 1).trimStart { it.isUnicodeWhitespace() }
        } else {
            rest
        }
        return prefix + words
    }
}
