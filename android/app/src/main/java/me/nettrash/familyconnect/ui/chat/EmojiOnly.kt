/*
 * EmojiOnly.kt
 * Family Connect (Android)
 *
 * Emoji-only message detection behind the bubble's font ladder: a
 * message whose visible content is exclusively emoji renders big and
 * bare (no balloon), scaled by how many emoji it carries — one emoji
 * biggest, then smaller per extra emoji through four; five or more, or
 * any text mixed in, and it is an ordinary text message again.
 *
 * Deliberately NOT android.icu: the JVM unit tests must run it, and the
 * two apps must agree on what counts as "emoji only" — so both embed
 * this same hand-rolled scanner over the same code-point table (the
 * EmojiCatalog discipline, applied to logic). The table is
 * Extended_Pictographic in broad strokes: bare text-presentation
 * pictographs (☂, ™) count as emoji on purpose — Android renders most
 * of them as color emoji anyway, and a symbol-only message reads as an
 * emoji message either way. An explicit text selector (U+FE0E) opts the
 * whole message out. The unit tests sweep the full picker catalog
 * through the scanner on both platforms.
 *
 * Kept free of Compose and Android so the tests run as plain unit
 * tests. iOS counterpart: ios/FamilyConnect/Models/EmojiOnly.swift —
 * keep the table and the sequence grammar in lockstep.
 */

package me.nettrash.familyconnect.ui.chat

object EmojiOnly {

    /**
     * Bubble font size (sp) for an emoji-only message: 1 emoji → 96,
     * 2 → 80, 3 → 68, 4 → 56. null = render as a normal text message
     * (five or more emoji, any text, or nothing but whitespace).
     */
    fun displayFontSize(text: String): Float? = when (count(text)) {
        1 -> 96f
        2 -> 80f
        3 -> 68f
        4 -> 56f
        else -> null
    }

    /**
     * How many emoji an emoji-only message carries; null when the text
     * is not emoji-only. Whitespace between emoji is allowed and not
     * counted, but whitespace alone is not an emoji message.
     */
    fun count(text: String): Int? {
        val codePoints = text.codePoints().toArray()
        var index = 0
        var count = 0
        while (index < codePoints.size) {
            val codePoint = codePoints[index]
            if (isWhitespace(codePoint)) {
                index++
                continue
            }
            val length = emojiSequenceLength(codePoints, index)
            if (length == 0) return null
            index += length
            count++
        }
        return if (count > 0) count else null
    }

    /**
     * The whitespace the scanner skips — Unicode White_Space, encoded
     * here rather than delegated to a platform predicate: java.lang and
     * Swift's scalar properties disagree at the edges (U+0085,
     * U+001C–U+001F), and the two scanners must classify identically.
     */
    private fun isWhitespace(value: Int) = when (value) {
        in 0x09..0x0D, 0x20, 0x85, 0xA0, 0x1680,
        in 0x2000..0x200A, 0x2028, 0x2029, 0x202F, 0x205F, 0x3000,
        -> true
        else -> false
    }

    // Sequence grammar

    private const val ZWJ = 0x200D
    private const val TEXT_SELECTOR = 0xFE0E
    private const val EMOJI_SELECTOR = 0xFE0F
    private const val COMBINING_KEYCAP = 0x20E3

    /**
     * Length in code points of the one emoji sequence starting at
     * [start], or 0 when what starts there is not emoji.
     */
    private fun emojiSequenceLength(codePoints: IntArray, start: Int): Int {
        val value = codePoints[start]

        // A flag is two regional indicators; a lone indicator still
        // renders as an emoji letter symbol, so it counts on its own.
        if (isRegionalIndicator(value)) {
            val paired = start + 1 < codePoints.size && isRegionalIndicator(codePoints[start + 1])
            return if (paired) 2 else 1
        }

        // Keycaps (5️⃣, #️⃣): digits, # and * are ordinary text unless
        // at least one of the enclosing marks follows.
        if (isKeycapBase(value)) {
            var index = start + 1
            var marked = false
            if (index < codePoints.size && codePoints[index] == EMOJI_SELECTOR) {
                index++
                marked = true
            }
            if (index < codePoints.size && codePoints[index] == COMBINING_KEYCAP) {
                index++
                marked = true
            }
            return if (marked) index - start else 0
        }

        if (!isEmojiBase(value)) return 0

        var index = start + 1
        while (index < codePoints.size) {
            val next = codePoints[index]
            if (next == TEXT_SELECTOR) {
                // The author explicitly asked for the text glyph — the
                // message is not an emoji message.
                return 0
            }
            if (next == EMOJI_SELECTOR || isSkinTone(next) || isTag(next)) {
                index++
                continue
            }
            if (next == ZWJ && index + 1 < codePoints.size && isEmojiBase(codePoints[index + 1])) {
                index += 2
                continue
            }
            break
        }
        return index - start
    }

    private fun isRegionalIndicator(value: Int) = value in 0x1F1E6..0x1F1FF

    private fun isKeycapBase(value: Int) = value == 0x23 || value == 0x2A || value in 0x30..0x39

    private fun isSkinTone(value: Int) = value in 0x1F3FB..0x1F3FF

    /** Tag characters — the payload of subdivision flags (🏴󠁧󠁢󠁳󠁣󠁴󠁿). */
    private fun isTag(value: Int) = value in 0xE0020..0xE007F

    /**
     * Code points that can open an emoji sequence (and follow a ZWJ).
     * Extended_Pictographic in broad strokes; regional indicators and
     * keycap bases are handled by their own branches above.
     */
    private val BASE_RANGES = listOf(
        0x00A9..0x00A9, 0x00AE..0x00AE,
        0x203C..0x203C, 0x2049..0x2049, 0x2122..0x2122, 0x2139..0x2139,
        0x2194..0x2199, 0x21A9..0x21AA,
        0x231A..0x231B, 0x2328..0x2328, 0x23CF..0x23CF, 0x23E9..0x23F3, 0x23F8..0x23FA,
        0x24C2..0x24C2,
        0x25AA..0x25AB, 0x25B6..0x25B6, 0x25C0..0x25C0, 0x25FB..0x25FE,
        0x2600..0x27BF,
        0x2934..0x2935,
        0x2B05..0x2B07, 0x2B1B..0x2B1C, 0x2B50..0x2B50, 0x2B55..0x2B55,
        0x3030..0x3030, 0x303D..0x303D, 0x3297..0x3297, 0x3299..0x3299,
        0x1F000..0x1F0FF,
        0x1F170..0x1F171, 0x1F17E..0x1F17F, 0x1F18E..0x1F18E, 0x1F191..0x1F19A,
        0x1F201..0x1F202, 0x1F21A..0x1F21A, 0x1F22F..0x1F22F, 0x1F232..0x1F23A,
        0x1F250..0x1F251,
        0x1F300..0x1FAFF,
    )

    private fun isEmojiBase(value: Int) = BASE_RANGES.any { value in it }
}
