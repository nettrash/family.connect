/*
 * EmojiTyping.kt
 * Family Connect (Android)
 *
 * Emoji as you type (#101; docs/emoji-as-you-type-2026-10-18.md):
 * Telegram's emoticons — `:-)`, `<3`, `;-P` — with their no-nose forms, and
 * `:name:` shortcodes, become emoji in the field the moment the last
 * character goes in. A Backspace straight after takes the replacement back.
 *
 * A port of web/text/src/emoji_typing.rs, the original every port is held to
 * (emoji-typing-vectors.json, printed by board-oracle): the same table in the
 * same order, the same four steps, the same guards. Two differences in
 * spelling only: offsets are UTF-16 indexes into a Kotlin String where the
 * Rust counts UTF-8 bytes, and a typed character is a String holding one
 * code point (so one outside the Basic Multilingual Plane stays a pair).
 *
 * Guards are ASCII on purpose: the four languages' "is a letter" disagree past
 * it, so `Char.isLetterOrDigit` is never used here.
 *
 * Pure Kotlin — no android.* — so the JVM tests run it as it is. The fields
 * reach it through [EmojiTyping.Field], which judges a field's value change
 * (a phone keyboard gives no key events); ui/components/EmojiTypingField.kt
 * is the Compose side.
 *
 * iOS counterpart: Core/EmojiTyping.swift.
 */

package me.nettrash.familyconnect.util

object EmojiTyping {

    /**
     * The emoticons, as typed, and the emoji each becomes — `EMOTICONS` in
     * the Rust, entry for entry and in the same order (the vectors check).
     *
     * The first 27 are Telegram Desktop's own table; the rest are the
     * owner's addition (2026-10-18): the no-nose forms of the same faces,
     * plus the case twins Telegram's case-sensitive table misses. An entry
     * ending in a space is replaced when that space is typed, and the space
     * stays.
     */
    val EMOTICONS: List<Pair<String, String>> = listOf(
        // Telegram's.
        ":-)" to "😊",
        "8-)" to "😍",
        "<3" to "❤️",
        ":-*" to "😚",
        "B-)" to "😎",
        ":-D" to "😃",
        ";-)" to "😉",
        ";-P" to "😜",
        ":-p" to "😋",
        "3(" to "😔",
        ":-(" to "😞",
        ":]" to "😏",
        ":'(" to "😢",
        ":_(" to "😭",
        ":((" to "😩",
        ":|" to "😐",
        "3-)" to "😌",
        ">(" to "😠",
        ">((" to "😡",
        "O:)" to "😇",
        ";o" to "😰",
        "8|" to "😳",
        "8o" to "😲",
        ":X" to "😷",
        "}:)" to "😈",
        ":o " to "😨",
        "xD " to "😆",
        // The no-nose forms and the case twins.
        ":)" to "😊",
        ":D" to "😃",
        ";)" to "😉",
        ":(" to "😞",
        ":*" to "😚",
        ":P" to "😋",
        ":p " to "😋",
        ":-P" to "😋",
        ";P" to "😜",
        ";p" to "😜",
        ";-p" to "😜",
        "O:-)" to "😇",
        "}:-)" to "😈",
        "XD " to "😆",
    )

    /** The shortcode table's Java resource: a copy of web/text/data/emoji-names.tsv. */
    private const val NAMES_RESOURCE = "/emoji-names.tsv"

    /**
     * `name` to emoji, read once, on the first shortcode typed. An absolute
     * resource path, so R8 renaming this class's package cannot move it.
     * A missing resource is an empty table — shortcodes stop working, the
     * field never breaks.
     */
    private val names: Map<String, String> by lazy {
        val stream = EmojiTyping::class.java.getResourceAsStream(NAMES_RESOURCE)
            ?: return@lazy emptyMap()
        stream.bufferedReader(Charsets.UTF_8).useLines { lines ->
            buildMap {
                for (line in lines) {
                    if (line.isEmpty() || line.startsWith('#')) continue
                    val tab = line.indexOf('\t')
                    if (tab < 0) continue
                    // A later line for a name wins, as in the Rust's
                    // HashMap collect; the generator writes each name once.
                    put(line.substring(0, tab), line.substring(tab + 1))
                }
            }
        }
    }

    /** How many shortcode names the table holds. */
    val nameCount: Int get() = names.size

    /**
     * The emoji `name` (no colons) stands for, if any. Names are matched
     * exactly: `:Fire:` is not `:fire:`.
     */
    fun emojiNamed(name: String): String? = names[name]

    /**
     * A replacement the rule just made, kept so the Backspace pressed
     * straight after it can take it back — and so the next key can grow it.
     * Offsets are UTF-16 indexes into the field's text.
     */
    data class Swap(
        /** Where the replacement starts. */
        val at: Int,
        /** What was put in: the emoji, and the space after it for the emoticons that end in one. */
        val put: String,
        /** What was typed, all of it: what a Backspace puts back. */
        val typed: String,
    ) {
        /**
         * The field's text and caret after the swap, given what was before
         * the caret (without the typed character) and what is after it.
         */
        fun apply(before: String, after: String): Pair<String, Int> {
            val head = before.substring(0, at) + put
            return (head + after) to head.length
        }

        /**
         * A Backspace with the caret at `caret` and nothing selected: the text
         * with what was typed put back, and the caret right behind it — or
         * null when the caret is not right behind the replacement or the
         * replacement is no longer there, and the Backspace is an ordinary one.
         */
        fun backspace(text: String, caret: Int): Pair<String, Int>? {
            val end = at + put.length
            if (caret != end || at < 0 || end > text.length || !text.regionMatches(at, put, 0, put.length)) {
                return null
            }
            return (text.substring(0, at) + typed + text.substring(end)) to at + typed.length
        }
    }

    /**
     * What typing `typed` (one code point) turns into: `before` is the
     * field's text before the caret, without `typed`; `live` is the swap
     * the field still holds, if any. Null when the character goes in as it
     * is.
     *
     * In order: nothing in code; GROWING the live swap; a SHORTCODE on `:`;
     * the longest EMOTICON whose guard passes. See the Rust for the reasons.
     */
    fun replace(before: String, typed: String, live: Swap?): Swap? {
        // 1. Code is typed as it is.
        if (inCode(before)) return null
        // 2. Growing the swap just made.
        if (live != null &&
            live.at >= 0 &&
            live.at + live.put.length == before.length &&
            before.regionMatches(live.at, live.put, 0, live.put.length)
        ) {
            val grown = live.typed + typed
            EMOTICONS.firstOrNull { it.first == grown }?.let { (_, emoji) ->
                return Swap(at = live.at, put = putFor(grown, emoji), typed = grown)
            }
        }
        // 3. `:name:`.
        if (typed == ":") {
            val colon = before.lastIndexOf(':')
            if (colon >= 0) {
                val name = before.substring(colon + 1)
                if (name.isNotEmpty() &&
                    !name.containsUnicodeWhitespace() &&
                    !(colon > 0 && before[colon - 1].isAsciiLetterOrDigit())
                ) {
                    emojiNamed(name)?.let { emoji ->
                        return Swap(at = colon, put = emoji, typed = ":$name:")
                    }
                }
            }
        }
        // 4. An emoticon, the longest that may stand here.
        val typedText = before + typed
        return EMOTICONS
            .filter { (text, _) -> typedText.endsWith(text) }
            .filter { (text, _) -> guardPasses(typedText.substring(0, typedText.length - text.length), text) }
            // Rust's max_by_key keeps the last of equal keys; two distinct
            // suffixes of one text never share a length, so it cannot matter.
            .maxByOrNull { it.first.length }
            ?.let { (text, emoji) ->
                Swap(at = typedText.length - text.length, put = putFor(text, emoji), typed = text)
            }
    }

    /**
     * Whether the caret, with `before` in front of it, is in code: inside a
     * fence (an odd number of lines before it start with three backticks) or
     * a code span (an odd number of backticks on its own line). A line break
     * is `\n` or `\r`. A port of `in_code` in web/text/src/composer.rs (#97),
     * which the capitals use on the desktop.
     */
    fun inCode(before: String): Boolean {
        val lines = before.split('\n', '\r')
        val fences = lines.count { it.startsWith("```") }
        if (fences % 2 == 1) return true
        return lines.last().count { it == '`' } % 2 == 1
    }

    /**
     * Types `keys` into an empty field with the caret always at the end, the
     * way every port's vectors are checked: `⌫` is a Backspace, `→` is
     * "something else happened" (the live swap is dropped), every other
     * character is typed. An ordinary Backspace removes one code point.
     */
    fun typing(keys: String): String {
        var text = ""
        var live: Swap? = null
        var index = 0
        while (index < keys.length) {
            val codePoint = keys.codePointAt(index)
            val key = String(Character.toChars(codePoint))
            index += key.length
            when (key) {
                "⌫" -> {
                    val undone = live?.backspace(text, text.length)
                    text = when {
                        undone != null -> undone.first
                        text.isEmpty() -> text
                        else -> text.substring(0, text.offsetByCodePoints(text.length, -1))
                    }
                    live = null
                }
                "→" -> live = null
                else -> {
                    val swap = replace(text, key, live)
                    if (swap != null) {
                        text = swap.apply(text, "").first
                        live = swap
                    } else {
                        text += key
                        live = null
                    }
                }
            }
        }
        return text
    }

    /**
     * A field's text and selection, as UTF-16 indexes — what [Field] judges a
     * change between.
     */
    data class Value(val text: String, val selectionStart: Int, val selectionEnd: Int) {
        constructor(text: String, caret: Int) : this(text, caret, caret)

        val collapsed: Boolean get() = selectionStart == selectionEnd
    }

    /**
     * One prose field's side of the rule, for a keyboard that gives no key
     * events: it judges each change of the field's value and holds the live
     * swap between them. Every prose field owns one.
     *
     * - EXACTLY one code point inserted at the old caret (nothing was
     *   selected, nothing else changed, the caret is right behind it) and no
     *   composition open: [replace] runs, and a swap is put in with the caret
     *   right behind it.
     * - TAKE-BACK: a live swap, the caret right behind it, and the change is
     *   only the deletion of up to the replacement's length just before the
     *   caret — a Backspace, which on an emoji removes the whole cluster and
     *   on `😨 ` only the space: what was typed comes back, caret behind it.
     * - Anything else — a paste, a suggestion, a caret move, a selection, a
     *   change made by the app — drops the swap and passes the value
     *   through untouched.
     *
     * A change of the composition alone (same text, same selection) is not
     * "something else": keyboards re-mark words after the field is set, and
     * the state-based composer never sees such a change at all.
     */
    class Field {
        private var live: Swap? = null

        /** The value the field was left in by the last swap: the live swap holds only from there. */
        private var left: Value? = null

        /** Forget the live swap (the field was cleared, sent or reloaded). */
        fun reset() {
            live = null
            left = null
        }

        /**
         * The value the field should hold instead of `new`, or null to let
         * `new` stand. `composing` is whether `new` has an open composition
         * (an IME still working on a word); null when it cannot be known.
         */
        fun edit(old: Value, new: Value, composing: Boolean = false): Value? {
            if (old == new) return null
            val wasLive = live.takeIf { left == old }
            live = null
            left = null
            if (!old.collapsed || !new.collapsed) return null
            val caret = old.selectionStart
            if (caret < 0 || caret > old.text.length) return null

            // Taking it back.
            if (wasLive != null) {
                val gone = old.text.length - new.text.length
                if (gone in 1..wasLive.put.length &&
                    new.selectionStart == caret - gone &&
                    new.text.regionMatches(0, old.text, 0, caret - gone) &&
                    new.text.regionMatches(caret - gone, old.text, caret, old.text.length - caret)
                ) {
                    wasLive.backspace(old.text, caret)?.let { (text, at) -> return Value(text, at) }
                }
            }

            // One character typed.
            if (composing) return null
            val added = new.text.length - old.text.length
            if (added !in 1..2 || new.selectionStart != caret + added) return null
            if (!new.text.regionMatches(0, old.text, 0, caret) ||
                !new.text.regionMatches(caret + added, old.text, caret, old.text.length - caret)
            ) {
                return null
            }
            val typed = new.text.substring(caret, caret + added)
            val oneCodePoint = if (added == 1) {
                !typed[0].isSurrogate()
            } else {
                typed[0].isHighSurrogate() && typed[1].isLowSurrogate()
            }
            if (!oneCodePoint) return null
            val before = old.text.substring(0, caret)
            val swap = replace(before, typed, wasLive) ?: return null
            val (text, at) = swap.apply(before, old.text.substring(caret))
            val result = Value(text, at)
            live = swap
            left = result
            return result
        }
    }

    private fun putFor(text: String, emoji: String): String = if (text.endsWith(' ')) "$emoji " else emoji

    /** Rule 4's guard: what may stand before `emoticon`. */
    private fun guardPasses(head: String, emoticon: String): Boolean {
        if (head.isEmpty()) return true
        val previous = head[head.length - 1]
        return if (emoticon.startsWith(':') || emoticon.startsWith(';')) {
            !previous.isAsciiDigit()
        } else {
            !previous.isAsciiLetterOrDigit()
        }
    }

    private fun Char.isAsciiDigit(): Boolean = this in '0'..'9'

    private fun Char.isAsciiLetterOrDigit(): Boolean =
        this in '0'..'9' || this in 'a'..'z' || this in 'A'..'Z'

    /**
     * Rust's `char::is_whitespace` — the Unicode White_Space property —
     * which is not Kotlin's `isWhitespace` (that one counts U+001C..U+001F
     * and misses U+0085). Every White_Space code point is in the BMP.
     */
    private fun String.containsUnicodeWhitespace(): Boolean = any { c ->
        c in '\u0009'..'\u000D' || c == ' ' || c == '\u0085' || c == ' ' || c == ' ' ||
            c in ' '..' ' || c == ' ' || c == ' ' || c == ' ' ||
            c == ' ' || c == '　'
    }
}
