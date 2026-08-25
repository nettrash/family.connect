package me.nettrash.familyconnect.ui.chat

import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.em

/**
 * Markdown in a chat bubble: the small subset people actually type.
 *
 * `**bold**`, `*italic*`, `` `code` ``, `~~strike~~`, `[label](url)`,
 * backslash escapes and ```` ```fenced``` ```` blocks, plus `#` headings
 * (three levels), `- ` bullets and GFM pipe tables. Nothing else.
 *
 * **THE RULE THAT KEEPS THE ARCHITECTURE:** headings and bullets are
 * RUNS, not blocks. A heading changes a font; a bullet replaces two
 * characters. Both live inside the one attributed string this class
 * already built, so a body with no table comes back as exactly ONE
 * [Block.Text] and is drawn by exactly the code that drew it before —
 * one `Text`, one `TextLayoutResult` for [linkSpanAt] to resolve a tap
 * against, one gesture detector carrying the double-tap heart and the
 * long-press capsule. Only a TABLE, which is a grid and cannot be a run,
 * turns a body into a column of blocks; and even then the extra blocks
 * bring their own layout results, never a second gesture arbiter.
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
 * **THE INVARIANT, and it is the whole reason a block carries [Rendered]:**
 * markdown DELETES characters. Every offset-based pass downstream — the
 * Linkify detector, the `@ai` highlight, [linkSpanAt]'s tap resolution and
 * the TalkBack labels — must index the RENDERED string, never the raw body.
 * Detecting over the raw body and drawing the rendered one leaves every
 * link after the first markup token pointing at the wrong glyphs, with
 * nothing failing and no error to see. Each block carries its OWN rendered
 * string, and its links index THAT one: a block is a whole offset space,
 * which is why the hit test needs one layout result per block and nothing
 * may be spliced into a string after its links were measured against it.
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

    /**
     * One piece of a rendered body.
     *
     * A run of consecutive non-table lines is one [Text]; a table is a
     * [Table]. A body with no table is a single [Text] and nothing else —
     * see the class header for why that is load-bearing rather than an
     * optimisation.
     */
    sealed interface Block {

        /** A run of inline text, with its own offset space. */
        data class Text(val rendered: Rendered) : Block

        /**
         * A GFM pipe table: header cells, per-column alignment, body rows.
         *
         * Every row has exactly [header]`.size` cells — short rows are
         * padded and long ones truncated at parse time, so the drawing
         * code never has to think about a ragged grid.
         *
         * Cells are already-rendered inline strings that carry NO links,
         * deliberately: see [cell].
         */
        data class Table(
            val header: List<AnnotatedString>,
            val rows: List<List<AnnotatedString>>,
            val alignments: List<TextAlign>,
        ) : Block
    }

    private val BOLD = SpanStyle(fontWeight = FontWeight.Bold)
    private val ITALIC = SpanStyle(fontStyle = FontStyle.Italic)
    private val STRIKE = SpanStyle(textDecoration = TextDecoration.LineThrough)
    private val CODE = SpanStyle(fontFamily = FontFamily.Monospace)

    /**
     * The heading ladder, indexed by level - 1.
     *
     * Sized in `em`, not `sp`: a heading is a multiple of whatever the
     * bubble's body font currently is, so it scales with the font-size
     * setting for free and can never be pinned to a size the rest of the
     * balloon has moved away from. The Apple side spends `.title2` /
     * `.title3` / `.headline` on the same three steps — the LADDER is the
     * contract, not the point sizes.
     */
    private val HEADINGS = listOf(
        SpanStyle(fontSize = 1.29.em, fontWeight = FontWeight.Bold),
        SpanStyle(fontSize = 1.18.em, fontWeight = FontWeight.Bold),
        SpanStyle(fontSize = 1.0.em, fontWeight = FontWeight.SemiBold),
    )

    /** What a `- ` / `* ` / `+ ` marker becomes: U+2022 and a normal space. */
    private const val BULLET = "• "

    /** The fence marker. Three backticks, at the start of a line. */
    private const val FENCE = "```"

    /**
     * Render [body] into the blocks the bubble stacks. The ONLY way in —
     * there is deliberately no "just give me the string" entry point left
     * for a caller to reach for and silently lose a table down.
     *
     * The list is never empty: an empty body is one empty text block, so
     * nothing downstream has to special-case "no blocks at all".
     *
     * The one pass: fences first, then lines, then the inline grammar.
     * Fences win over everything because their whole point is that nothing
     * inside them is markup — a `| x |` or a `# x` in a code block is code.
     * Within a plain segment, a table CLOSES the text block it interrupts
     * and the next line opens a new one, which is what makes each block a
     * self-contained offset space. The newline on either side of the table
     * goes with that split: the blocks are stacked, so drawing it as well
     * would show up as an empty line.
     */
    fun blocks(body: String): List<Block> {
        val out = mutableListOf<Block>()
        var builder = AnnotatedString.Builder()
        var links = mutableListOf<LinkSpan>()

        // [force] emits even an empty block, which is what keeps the list
        // non-empty for an empty body.
        fun flush(force: Boolean) {
            val annotated = builder.toAnnotatedString()
            if (force || annotated.isNotEmpty()) {
                out += Block.Text(Rendered(annotated, annotated.text, links))
            }
            builder = AnnotatedString.Builder()
            links = mutableListOf()
        }

        for (segment in segments(body)) {
            when (segment) {
                is Segment.Code -> {
                    val start = builder.length
                    builder.append(segment.text)
                    builder.addStyle(CODE, start, builder.length)
                }
                is Segment.Text -> {
                    val lines = segment.text.split("\n")
                    val pending = mutableListOf<String>()
                    var i = 0
                    while (i < lines.size) {
                        // PRECEDENCE: heading, then bullet, then table —
                        // the order Apple walks in (MessageMarkdown.swift,
                        // `pieces(of:)`), and the order this file's own
                        // header claims when it says a marker is consumed
                        // before anything else sees the line. Probing the
                        // table first turned `# Trip | 2 people` into a
                        // grid whose header cell read "# Trip", because a
                        // heading is allowed to contain a pipe and the
                        // line below it looked like a delimiter row.
                        val marked =
                            headingAt(lines[i]) != null || bulletAt(lines[i]) != null
                        val table = if (marked) null else tableAt(lines, i)
                        if (table == null) {
                            pending += lines[i]
                            i += 1
                            continue
                        }
                        appendLines(builder, pending, links)
                        pending.clear()
                        flush(force = false)
                        out += table.block
                        i = table.end
                    }
                    appendLines(builder, pending, links)
                }
            }
        }
        flush(force = out.isEmpty())
        return out
    }

    /** [body] with no markup applied at all — the emoji-only path. */
    fun plain(body: String): Rendered =
        Rendered(AnnotatedString(body), body, emptyList())

    // ---- blocks ------------------------------------------------------------

    /**
     * Append [lines] to [builder], one text block's worth.
     *
     * Consecutive lines with no block marker are inline-parsed TOGETHER, in
     * a single call over the joined run, so an ordinary multi-line message
     * goes through this file exactly as it did before blocks existed —
     * emphasis may still span a line break, and `"first\nsecond\n\nafter"`
     * still comes back byte-identical. A heading or a bullet is its own
     * run: the marker is consumed before the inline parser can see it,
     * which is what makes `* X` at line start a bullet without teaching the
     * emphasis parser anything new (and leaves `2 * 3 * 4 = 24` alone).
     */
    private fun appendLines(
        builder: AnnotatedString.Builder,
        lines: List<String>,
        links: MutableList<LinkSpan>,
    ) {
        if (lines.isEmpty()) return
        val plain = mutableListOf<String>()
        // Runs are separated by exactly one newline — the one that ended
        // the previous line. A plain run already carries its own inside.
        var emitted = false

        fun flushPlain() {
            if (plain.isEmpty()) return
            if (emitted) builder.append("\n")
            builder.inline(plain.joinToString("\n"), links)
            plain.clear()
            emitted = true
        }

        for (line in lines) {
            val heading = headingAt(line)
            val bullet = if (heading == null) bulletAt(line) else null
            if (heading == null && bullet == null) {
                plain += line
                continue
            }
            flushPlain()
            if (emitted) builder.append("\n")
            if (heading != null) {
                val start = builder.length
                builder.inline(heading.text, links)
                builder.addStyle(HEADINGS[heading.level - 1], start, builder.length)
            } else if (bullet != null) {
                // The indent is copied verbatim, which is what gives a
                // visually nested list for free — without a parser that can
                // mis-nest one.
                builder.append(bullet.indent)
                builder.append(BULLET)
                builder.inline(bullet.text, links)
            }
            emitted = true
        }
        flushPlain()
    }

    private data class Heading(val level: Int, val text: String)

    private data class Bullet(val indent: String, val text: String)

    /**
     * `# X` through `### X`, or null.
     *
     * Three deliberate refusals, and all three leave the line exactly as
     * typed: `#Heading` (no space) is a hashtag, which people type;
     * `#### X` is deeper than the ladder goes; and `# ` with nothing after
     * it is somebody mid-sentence. No closing-sequence stripping either —
     * `# Done #` renders as "Done #", because the second `#` is content.
     */
    private fun headingAt(line: String): Heading? {
        var level = 0
        while (level < line.length && line[level] == '#') level += 1
        if (level !in 1..3) return null
        if (level >= line.length || line[level] != ' ') return null
        val text = line.substring(level + 1)
        if (text.isBlank()) return null
        return Heading(level, text)
    }

    /**
     * `- X` / `* X` / `+ X` after any indent, or null.
     *
     * `---` is not one (the marker must be followed by a space), which is
     * also what leaves a table's delimiter row available to [tableAt], and
     * a line of just `- ` is not one either.
     */
    private fun bulletAt(line: String): Bullet? {
        var i = 0
        while (i < line.length && (line[i] == ' ' || line[i] == '\t')) i += 1
        if (i >= line.length) return null
        val marker = line[i]
        if (marker != '-' && marker != '*' && marker != '+') return null
        if (i + 1 >= line.length || line[i + 1] != ' ') return null
        val text = line.substring(i + 2)
        if (text.isBlank()) return null
        return Bullet(indent = line.substring(0, i), text = text)
    }

    // Ordered items (`1. X`, `1) X`) are recognised by eye and deliberately
    // left as typed: they already read as a list, and renumbering somebody
    // else's message is worse than leaving it.

    // ---- tables ------------------------------------------------------------

    /** A parsed table and the line index just past it. */
    private class ParsedTable(val block: Block.Table, val end: Int)

    /**
     * The table starting at [from], or null.
     *
     * A header row alone is not a table — plenty of ordinary sentences
     * contain a pipe. It takes a delimiter row directly underneath with
     * the SAME number of cells, and if that does not hold every line is
     * left exactly as typed. The table then runs to the first line that is
     * not a row — a blank line, a heading and a bullet all included.
     */
    private fun tableAt(lines: List<String>, from: Int): ParsedTable? {
        if (from + 1 >= lines.size) return null
        val header = rowCells(lines[from]) ?: return null
        val delimiter = rowCells(lines[from + 1]) ?: return null
        if (delimiter.size != header.size) return null
        val alignments = delimiter.map { alignmentOf(it) ?: return null }
        val rows = mutableListOf<List<AnnotatedString>>()
        var i = from + 2
        while (i < lines.size) {
            // A heading or a bullet ENDS the table, exactly as a
            // pipe-less line does. Both markers win over a table
            // wherever the two could describe the same line (see
            // `blocks`), so `- and | maybe Bob` under a grid is the
            // bullet it says it is and not a fourth row that swallows
            // the marker. Apple breaks on the same two.
            if (headingAt(lines[i]) != null || bulletAt(lines[i]) != null) break
            val cells = rowCells(lines[i]) ?: break
            // Padded and truncated here so every row has the header's
            // width — a ragged grid would have to be handled in the
            // drawing code, per row, forever.
            rows += List(header.size) { column -> cell(cells.getOrElse(column) { "" }) }
            i += 1
        }
        return ParsedTable(
            Block.Table(
                header = header.map(::cell),
                rows = rows,
                alignments = alignments,
            ),
            end = i,
        )
    }

    /**
     * The cells of one table row, or null when [line] is not a row.
     *
     * Leading and trailing pipes are optional, and `\|` is a literal pipe
     * rather than a cell boundary.
     */
    private fun rowCells(line: String): List<String>? {
        val trimmed = line.trim()
        if (trimmed.isEmpty()) return null
        val cells = mutableListOf<String>()
        val cell = StringBuilder()
        var pipes = 0
        var i = 0
        while (i < trimmed.length) {
            val c = trimmed[i]
            if (c == '\\' && i + 1 < trimmed.length) {
                // Kept escaped: the cell goes through the inline parser,
                // which is where a backslash escape is resolved.
                cell.append(c).append(trimmed[i + 1])
                i += 2
                continue
            }
            if (c == '|') {
                pipes += 1
                cells += cell.toString()
                cell.clear()
                i += 1
                continue
            }
            cell.append(c)
            i += 1
        }
        if (pipes == 0) return null
        val closed = cell.isEmpty() && trimmed.endsWith("|")
        cells += cell.toString()
        var result: List<String> = cells
        if (trimmed.startsWith("|")) result = result.drop(1)
        if (closed && result.isNotEmpty()) result = result.dropLast(1)
        if (result.isEmpty()) return null
        return result.map { it.trim() }
    }

    /**
     * The alignment a delimiter cell declares, or null when it is not a
     * delimiter cell at all — which is what decides whether these lines
     * are a table or just text with pipes in it.
     */
    private fun alignmentOf(cell: String): TextAlign? {
        val left = cell.startsWith(":")
        val right = cell.length > 1 && cell.endsWith(":")
        val dashes = cell.removePrefix(":").removeSuffix(":")
        if (dashes.isEmpty() || dashes.any { it != '-' }) return null
        return when {
            left && right -> TextAlign.Center
            right -> TextAlign.End
            else -> TextAlign.Start
        }
    }

    /**
     * One table cell: emphasis, code, strikethrough and escapes — but NOT
     * links, which are left exactly as typed.
     *
     * That is the whole reason a table costs nothing structurally. A link
     * needs an offset space to be hit-tested in, and a cell is not one:
     * it would mean a hit test per cell, or a clickable child per cell
     * fighting the balloon for the pointer down — and a second place where
     * a label and a destination could disagree, which is the phishing
     * shape [MessageLinks.mergeSpans] already exists to close. A null link
     * sink is how that is said to the inline parser.
     */
    private fun cell(source: String): AnnotatedString =
        buildAnnotatedString { inline(source, links = null) }

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
                //
                // Keep the newline that ENDED the line above the fence, the
                // mirror of what the closing side does below. Without it
                // the joined text has no separator and the code run welds
                // onto the end of the previous sentence — "try this:" and a
                // block under it came out as "try this:let x = 1". An empty
                // element here is dropped by the filter at the end when the
                // fence opens the body, so a message that STARTS with a
                // block still does.
                inFence = true
                plain += ""
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
     *
     * A null [links] sink means links are not allowed HERE — a table cell —
     * and `[label](url)` is then left exactly as typed. See [cell].
     */
    private fun AnnotatedString.Builder.inline(source: String, links: MutableList<LinkSpan>?) {
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
            if (c == '[' && links != null) {
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
