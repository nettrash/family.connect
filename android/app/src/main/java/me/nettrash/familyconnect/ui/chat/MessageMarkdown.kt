package me.nettrash.familyconnect.ui.chat

import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration

/**
 * Markdown in a chat bubble: the small subset people actually type.
 *
 * `**bold**`, `*italic*`, `` `code` ``, `~~strike~~`, `[label](url)`,
 * backslash escapes and ```` ```fenced``` ```` blocks. Nothing else.
 * Headings, lists, quotes and tables are deliberately absent — they turn a
 * balloon into a column of blocks, and a column of blocks is what would
 * break the two things this bubble depends on being ONE `Text`: the single
 * `TextLayoutResult` that [linkSpanAt]'s hit test resolves against, and the
 * single `combinedClickable` that carries the double-tap heart.
 *
 * This is a RENDERING convention, not a wire format (docs/protocol.md says
 * so explicitly): `body` is plain text on the wire, the server neither
 * parses nor validates any of it, and a client that renders none of this
 * shows the source — which is still exactly what was written. The SUBSET is
 * what this shares with iOS and macOS, not the implementation: they lean on
 * Foundation's own `AttributedString(markdown:)`, and Android has no
 * equivalent, so this is a hand-written pass over the same grammar (the
 * same one the `md.Android` app carries).
 *
 * **THE INVARIANT, and it is the whole reason this class returns [text]:**
 * markdown DELETES characters. Every offset-based pass downstream — the
 * Linkify detector, the `@ai` highlight, [linkSpanAt]'s tap resolution and
 * the TalkBack labels — must index the RENDERED string, never the raw body.
 * Detecting over the raw body and drawing the rendered one leaves every
 * link after the first markup token pointing at the wrong glyphs, with
 * nothing failing and no error to see.
 *
 * **The one known divergence from Foundation**, which the Apple side uses:
 * three delimiters closing at once — `**outer *inner***` — needs
 * CommonMark's full delimiter-STACK algorithm, and this parser matches
 * runs pairwise instead. It fails SAFELY: the markers are left in the text
 * exactly as typed, so nothing is mis-rendered and no offset moves. Pinned
 * by a test on both sides so it stays a stated limit rather than a
 * surprise.
 *
 * Deliberately NOT `LinkAnnotation.Url` for the links it finds, for the
 * reason [MessageLinks] documents at length: Compose mounts each link range
 * as its own clickable child that consumes the pointer down, which silently
 * kills the bubble's double-tap-heart and long-press-reaction over link
 * glyphs. Links come back as plain [LinkSpan] offsets, like the detector's.
 */
object MessageMarkdown {

    /**
     * A rendered body: what to draw, what it reads as, and the links the
     * markup itself declared.
     *
     * [text] is `annotated.text` — carried separately because every caller
     * needs it and because naming it is the clearest way to say that this,
     * not the raw body, is what offsets index.
     */
    data class Rendered(
        val annotated: AnnotatedString,
        val text: String,
        val links: List<LinkSpan>,
    )

    private val BOLD = SpanStyle(fontWeight = FontWeight.Bold)
    private val ITALIC = SpanStyle(fontStyle = FontStyle.Italic)
    private val STRIKE = SpanStyle(textDecoration = TextDecoration.LineThrough)
    private val CODE = SpanStyle(fontFamily = FontFamily.Monospace)

    /** The fence marker. Three backticks, at the start of a line. */
    private const val FENCE = "```"

    /** Render [body]. An unmarked body comes back untouched. */
    fun render(body: String): Rendered {
        if (body.isEmpty()) return Rendered(AnnotatedString(""), "", emptyList())
        val links = mutableListOf<LinkSpan>()
        val annotated = buildAnnotatedString {
            for (segment in segments(body)) {
                when (segment) {
                    is Segment.Code -> {
                        val start = length
                        append(segment.text)
                        addStyle(CODE, start, length)
                    }
                    is Segment.Text -> inline(segment.text, links)
                }
            }
        }
        return Rendered(annotated, annotated.text, links)
    }

    /** [body] with no markup applied at all — the emoji-only path. */
    fun plain(body: String): Rendered =
        Rendered(AnnotatedString(body), body, emptyList())

    // ---- fences ------------------------------------------------------------

    private sealed interface Segment {
        data class Text(val text: String) : Segment
        data class Code(val text: String) : Segment
    }

    /**
     * Split [body] into alternating plain and fenced-code segments.
     *
     * A fence must OPEN at the start of a line and CLOSE at the start of a
     * line, which is what stops a stray triple-backtick mid-sentence
     * swallowing the rest of the message. An unclosed fence is not a fence
     * at all: somebody typing the third backtick of an opening fence must
     * not watch the rest of their draft turn into a code block.
     */
    private fun segments(body: String): List<Segment> {
        if (!body.contains(FENCE)) return listOf(Segment.Text(body))
        val lines = body.split("\n")
        val result = mutableListOf<Segment>()
        val plain = mutableListOf<String>()
        val code = mutableListOf<String>()
        var inFence = false
        for (line in lines) {
            val marker = line.startsWith(FENCE)
            if (!inFence && marker) {
                // The language tag on an opening fence is dropped: nothing
                // here highlights syntax, so it would only be noise.
                inFence = true
                continue
            }
            if (inFence && marker) {
                inFence = false
                result += Segment.Text(plain.joinToString("\n"))
                plain.clear()
                result += Segment.Code(code.joinToString("\n"))
                code.clear()
                // Keep the newline that ended the fence, so text after a
                // block does not run onto its last line.
                plain += ""
                continue
            }
            if (inFence) code += line else plain += line
        }
        // Never closed — put it back exactly as it was typed.
        if (inFence) return listOf(Segment.Text(body))
        result += Segment.Text(plain.joinToString("\n"))
        return result.filter { it !is Segment.Text || it.text.isNotEmpty() }
    }

    // ---- inline ------------------------------------------------------------

    /**
     * The inline grammar, appended into the builder in one pass.
     *
     * Offsets are taken from the BUILDER's own length before and after each
     * emitted run, so a style or a link can never drift from the characters
     * it was measured against — which is exactly the class of bug this file
     * exists to prevent.
     */
    private fun AnnotatedString.Builder.inline(source: String, links: MutableList<LinkSpan>) {
        var i = 0
        val n = source.length
        val run = StringBuilder()

        fun flush() {
            if (run.isNotEmpty()) {
                append(run.toString())
                run.clear()
            }
        }

        while (i < n) {
            val c = source[i]

            // Backslash escapes the next character, whatever it is.
            if (c == '\\' && i + 1 < n) {
                run.append(source[i + 1])
                i += 2
                continue
            }

            // Code span. Nothing inside is markup.
            if (c == '`') {
                val close = source.indexOf('`', i + 1)
                if (close > i + 1) {
                    flush()
                    val start = length
                    append(source.substring(i + 1, close))
                    addStyle(CODE, start, length)
                    i = close + 1
                    continue
                }
            }

            // [label](url)
            if (c == '[') {
                val closeLabel = source.indexOf(']', i + 1)
                if (closeLabel > i && closeLabel + 1 < n && source[closeLabel + 1] == '(') {
                    val closeUrl = closingParen(source, closeLabel + 2)
                    if (closeUrl > closeLabel + 1) {
                        val label = source.substring(i + 1, closeLabel)
                        val url = source.substring(closeLabel + 2, closeUrl).trim()
                        if (label.isNotEmpty() && url.isNotEmpty()) {
                            flush()
                            val start = length
                            // The label may itself be marked up; the URL
                            // never is.
                            inline(label, links)
                            links += LinkSpan(start = start, end = length, url = normalize(url))
                            i = closeUrl + 1
                            continue
                        }
                    }
                }
            }

            // **bold** / __bold__
            if ((c == '*' || c == '_') && i + 1 < n && source[i + 1] == c) {
                val marker = "$c$c"
                val close = if (opensEmphasis(source, i, 2, c)) {
                    closingIndex(source, marker, i + 2)
                } else {
                    -1
                }
                if (close > i + 2) {
                    flush()
                    val start = length
                    inline(source.substring(i + 2, close), links)
                    addStyle(BOLD, start, length)
                    i = close + 2
                    continue
                }
            }

            // ~~strike~~
            if (c == '~' && i + 1 < n && source[i + 1] == '~') {
                val close = if (opensEmphasis(source, i, 2, c)) {
                    closingIndex(source, "~~", i + 2)
                } else {
                    -1
                }
                if (close > i + 2) {
                    flush()
                    val start = length
                    inline(source.substring(i + 2, close), links)
                    addStyle(STRIKE, start, length)
                    i = close + 2
                    continue
                }
            }

            // *italic* / _italic_
            if (c == '*' || c == '_') {
                val close = if (opensEmphasis(source, i, 1, c)) {
                    closingIndex(source, c.toString(), i + 1)
                } else {
                    -1
                }
                if (close > i + 1) {
                    flush()
                    val start = length
                    inline(source.substring(i + 1, close), links)
                    addStyle(ITALIC, start, length)
                    i = close + 1
                    continue
                }
            }

            run.append(c)
            i += 1
        }
        flush()
    }

    /**
     * The `)` that closes a link destination opened at [from], or -1.
     *
     * Balanced, NOT the first `)`. Wikipedia article URLs routinely end in
     * one — `…/wiki/Foo_(bar)` — and a first-`)` scan cuts the destination
     * in half, producing a link that silently goes to the wrong page while
     * the stray `)` is left in the text. CommonMark requires balance here
     * for exactly this reason.
     */
    private fun closingParen(source: String, from: Int): Int {
        var depth = 0
        var i = from
        while (i < source.length) {
            when (source[i]) {
                // A backslash escapes the next character, destination
                // included, so a literal paren can still be written.
                '\\' -> i += 1
                '(' -> depth += 1
                ')' -> {
                    if (depth == 0) return i
                    depth -= 1
                }
            }
            i += 1
        }
        return -1
    }

    /**
     * Is the delimiter at [at] an OPENER?
     *
     * CommonMark's flanking rules, which are not decoration: without the
     * first of them `2 * 3 * 4 = 24` comes out as `2  3  4 = 24` with an
     * italic `3`, and arithmetic is a perfectly ordinary thing to type in a
     * family chat. Foundation applies the same rules on Apple, so this is
     * what keeps the two platforms rendering the same message the same way.
     *
     * - a run must not be followed by whitespace (nothing opens ` * `);
     * - an underscore run must additionally not follow an alphanumeric,
     *   which is what keeps `snake_case_name` whole. Asterisks have no such
     *   rule — `5*6*7` really is CommonMark for `567` with an italic 6.
     */
    private fun opensEmphasis(source: String, at: Int, width: Int, delimiter: Char): Boolean {
        val after = at + width
        if (after >= source.length || source[after].isWhitespace()) return false
        if (delimiter == '_' && at > 0 && source[at - 1].isLetterOrDigit()) return false
        return true
    }

    /**
     * The index of the matching CLOSER for [marker] at or after [from], or
     * -1.
     *
     * A closer must not be preceded by whitespace (nothing closes ` * `),
     * and an underscore run must not be followed by an alphanumeric — the
     * other half of what keeps `snake_case_name` whole.
     */
    private fun closingIndex(source: String, marker: String, from: Int): Int {
        val delimiter = marker[0]
        var j = source.indexOf(marker, from)
        while (j >= 0) {
            val precededByText = j > from && !source[j - 1].isWhitespace()
            val after = j + marker.length
            val followedByWord = delimiter == '_' &&
                after < source.length &&
                source[after].isLetterOrDigit()
            // A closer must be a delimiter run of the SAME LENGTH as the
            // opener. Without this the outer `*` of
            // `*this is **very** important*` closes on the SECOND asterisk
            // of the inner `**` — because that one is preceded by a `*`,
            // which is not whitespace — and the message renders as italic
            // "this is *" followed by stray markers, with the bold lost
            // entirely. CommonMark calls these delimiter runs; Foundation
            // gets it right on Apple, so Android has to as well or the same
            // message reads differently on two platforms.
            val partOfLongerRun =
                (j > 0 && source[j - 1] == delimiter && j - 1 >= from) ||
                    (after < source.length && source[after] == delimiter)
            if (precededByText && !followedByWord && !partOfLongerRun) return j
            j = source.indexOf(marker, j + 1)
        }
        return -1
    }

    /**
     * A markdown link's destination, made openable.
     *
     * Linkify hands back ready-to-open URLs; a hand-typed one may not have
     * a scheme, and an intent with no scheme goes nowhere. Anything already
     * carrying one is left exactly as written.
     */
    private fun normalize(url: String): String =
        if (url.contains("://") || url.startsWith("mailto:") || url.startsWith("tel:")) {
            url
        } else {
            "https://$url"
        }
}
