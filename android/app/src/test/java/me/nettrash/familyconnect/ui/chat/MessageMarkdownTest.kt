package me.nettrash.familyconnect.ui.chat

import androidx.compose.ui.text.style.TextAlign
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * What a chat bubble renders, and — more importantly — what it leaves
 * completely alone.
 *
 * Most messages are not markdown. A renderer that quietly rewrites an
 * ordinary sentence is worse than no renderer at all, so half of these
 * tests assert that nothing happened.
 *
 * The other half pins the property the whole design rests on: markdown
 * DELETES characters, and every offset-based pass downstream — the Linkify
 * detector, the `@ai` highlight, `linkSpanAt`'s tap resolution and the
 * TalkBack labels — must index the RENDERED string, never the raw body. A
 * test that only checked "it came out bold" would miss a link pointing at
 * the wrong glyphs, which is exactly how that bug would ship.
 *
 * The subset mirrors iOS/macOS (MessageMarkdownTests.swift); the
 * IMPLEMENTATIONS differ on purpose — Apple leans on Foundation's parser
 * and Android has no equivalent — so these vectors are the contract
 * between them.
 */
class MessageMarkdownTest {

    /**
     * The single text block a body with no table renders to — ASSERTED,
     * not assumed. Every vector in this file goes through here, so all of
     * them also pin the invariant the architecture rests on: no table means
     * one block, which means one `Text`, one `TextLayoutResult` for
     * `linkSpanAt` to resolve a tap against, and one offset space.
     */
    private fun single(body: String): MessageMarkdown.Rendered {
        val blocks = MessageMarkdown.blocks(body)
        assertEquals("not one block: $blocks", 1, blocks.size)
        return (blocks.single() as MessageMarkdown.Block.Text).rendered
    }

    private fun plain(body: String) = single(body).text

    /** The table [body] renders to, which must be the whole of it. */
    private fun table(body: String): MessageMarkdown.Block.Table =
        MessageMarkdown.blocks(body).single() as MessageMarkdown.Block.Table

    private fun MessageMarkdown.Block.Table.cells(): List<List<String>> =
        listOf(header.map { it.text }) + rows.map { row -> row.map { it.text } }

    @Test
    fun `ordinary messages are left exactly as they were typed`() {
        val untouched = listOf(
            "Dinner at 7?",
            "see you at the shop",
            "2 * 3 * 4 = 24",
            "call me on 555-1234",
            "https://example.com/a_b_c",
            "he said \"what?\" and left",
            "snake_case_name stays whole",
            "a * b",
            "5 < 6 > 4",
            "cost: \$5 (a bargain)",
            "",
            "🎉🎉🎉",
        )
        for (body in untouched) {
            assertEquals("rewritten: $body", body, plain(body))
        }
    }

    @Test
    fun `the subset renders and its markers are consumed`() {
        assertEquals("bold", plain("**bold**"))
        assertEquals("bold", plain("__bold__"))
        assertEquals("italic", plain("*italic*"))
        assertEquals("italic", plain("_italic_"))
        assertEquals("gone", plain("~~gone~~"))
        assertEquals("code", plain("`code`"))
        assertEquals("say hello there", plain("say **hello** there"))
    }

    /**
     * The single most likely false positive in a family that talks about
     * code, and the arithmetic one right behind it. These are CommonMark's
     * flanking rules, and they are what Foundation applies on Apple — so
     * this table is the contract between the two implementations, checked
     * against Foundation's actual output rather than guessed.
     */
    @Test
    fun `CommonMark flanking rules, matching Foundation on Apple`() {
        // Underscores inside a word are not emphasis.
        assertEquals("call user_name_field now", plain("call user_name_field now"))
        assertEquals("a_b_c", plain("a_b_c"))
        // A delimiter run followed (or preceded) by a space opens nothing,
        // which is what keeps arithmetic readable.
        assertEquals("2 * 3 * 4 = 24", plain("2 * 3 * 4 = 24"))
        assertEquals("a * b", plain("a * b"))
        // ...but a real one still works, including intraword asterisks,
        // which CommonMark does allow.
        assertEquals("real italic", plain("*real italic*"))
        assertEquals("567", plain("5*6*7"))
        assertEquals("x", plain("_x_"))
        // `__init__` at a word boundary IS emphasis in CommonMark, and
        // Foundation agrees — so Android must too, however surprising.
        assertEquals("init is special", plain("__init__ is special"))
    }

    /**
     * Nested emphasis. The outer delimiter must not close on a character
     * that belongs to a LONGER run — CommonMark's delimiter-run rule.
     * Without it `*this is **very** important*` rendered as italic
     * "this is *" plus stray markers, and the bold vanished.
     */
    @Test
    fun `nested emphasis keeps both levels`() {
        assertEquals("this is very important", plain("*this is **very** important*"))
        assertEquals("a b c", plain("*a **b** c*"))
        assertEquals("a b c", plain("_a __b__ c_"))
    }

    /**
     * The one KNOWN divergence from Foundation, pinned so it is a stated
     * limit rather than a surprise: three delimiters closing at once
     * (`**outer *inner***`) needs CommonMark's full delimiter-STACK, which
     * this parser deliberately does not implement.
     *
     * It fails SAFELY — the markers are simply left in the text, which is
     * exactly what was typed. Nothing is mis-rendered and no link offset
     * moves, which is the property that actually matters here.
     */
    @Test
    fun `three delimiters closing at once are left as typed`() {
        assertEquals("**outer *inner***", plain("**outer *inner***"))
    }

    /**
     * A destination with parentheses in it. Wikipedia article URLs end in
     * one routinely; a first-`)` scan cut the URL in half and silently
     * linked somewhere else.
     */
    @Test
    fun `a link destination may contain balanced parentheses`() {
        val rendered = single("see [wiki](https://en.wikipedia.org/wiki/Foo_(bar)) today")
        assertEquals("see wiki today", rendered.text)
        assertEquals("https://en.wikipedia.org/wiki/Foo_(bar)", rendered.links.single().url)
    }

    @Test
    fun `a backslash escapes the next character`() {
        assertEquals("**not bold**", plain("\\*\\*not bold\\*\\*"))
    }

    @Test
    fun `a markdown link shows its label and carries its destination`() {
        val rendered = single("see [the menu](https://example.com/menu) today")
        assertEquals("see the menu today", rendered.text)
        assertEquals(1, rendered.links.size)
        val link = rendered.links.single()
        assertEquals("https://example.com/menu", link.url)
        assertEquals(
            "the menu",
            rendered.text.substring(link.start, link.end),
        )
    }

    /** A hand-typed destination with no scheme still has to open. */
    @Test
    fun `a schemeless link destination gains one`() {
        val rendered = single("[here](example.com/x)")
        assertEquals("https://example.com/x", rendered.links.single().url)
    }

    /**
     * THE invariant, stated as an assertion: a link's offsets must index
     * the rendered text even when markup ahead of it shortened the string.
     */
    @Test
    fun `link offsets index the rendered text, not the raw body`() {
        val body = "**important** see [the menu](https://example.com) now"
        val rendered = single(body)
        assertEquals("important see the menu now", rendered.text)
        val link = rendered.links.single()
        assertEquals("the menu", rendered.text.substring(link.start, link.end))
        // And the naive version — slicing the RAW body with these offsets —
        // would be wrong, which is what makes the test worth having.
        assertFalse(
            "the raw body must not happen to agree, or this proves nothing",
            body.substring(link.start, link.end) == "the menu",
        )
    }

    @Test
    fun `a fenced block keeps its contents verbatim, markup and all`() {
        val rendered = plain("try this:\n```\nlet x = **not bold**\n```\ndone")
        assertTrue(rendered, rendered.contains("let x = **not bold**"))
        assertTrue(rendered, rendered.contains("try this:"))
        assertTrue(rendered, rendered.contains("done"))
        assertFalse("the fence markers are consumed: $rendered", rendered.contains("```"))
    }

    /**
     * Somebody typing the third backtick of an opening fence must not watch
     * the rest of their draft turn into a code block.
     */
    @Test
    fun `an unclosed fence is not a fence`() {
        val body = "```\nhalf written"
        assertEquals(body, plain(body))
    }

    @Test
    fun `a language tag on the fence is dropped rather than drawn`() {
        val rendered = plain("```kotlin\nval x = 1\n```")
        assertTrue(rendered, rendered.contains("val x = 1"))
        assertFalse("the tag is metadata, not text: $rendered", rendered.contains("kotlin"))
    }

    @Test
    fun `line breaks are preserved`() {
        val body = "first line\nsecond line\n\nafter a gap"
        assertEquals(body, plain(body))
    }

    @Test
    fun `emoji survive the renderer untouched`() {
        for (body in listOf("😀", "👨‍👩‍👧‍👦", "🇷🇸", "❤️")) {
            assertEquals(body, plain(body))
        }
    }

    /** The emoji-only path skips markup entirely. */
    @Test
    fun `plain is the identity`() {
        val body = "**not touched**"
        val rendered = MessageMarkdown.plain(body)
        assertEquals(body, rendered.text)
        assertTrue(rendered.links.isEmpty())
    }

    /**
     * The bug both parsers had: the opening fence line `continue`d without
     * pushing a separator, so the code run welded onto the end of the
     * sentence above it — "try this:let x = 1". The closing side had
     * already been fixed; this is its mirror.
     *
     * Asserted WHOLE, not with `contains`, because `contains` is exactly
     * what let it live: every part was present, in the wrong shape.
     */
    @Test
    fun `a fenced block keeps the newline above it`() {
        assertEquals(
            "try this:\nlet x = 1\ndone",
            plain("try this:\n```\nlet x = 1\n```\ndone"),
        )
        // A body that STARTS with a fence gains no leading blank line.
        assertEquals("let x = 1", plain("```\nlet x = 1\n```"))
    }

    /**
     * THE structural invariant: a body with no table is ONE text block, so
     * it draws through exactly the code it drew through before blocks
     * existed — one Text, one layout result, one gesture detector. Every
     * other test here asserts it too (see `single`); this one says so out
     * loud, over the shapes most likely to break it.
     */
    @Test
    fun `a body with no table is exactly one text block`() {
        val bodies = listOf(
            "Dinner at 7?",
            "",
            "# Heading\n- one\n- two\nand a paragraph",
            "first line\nsecond line\n\nafter a gap",
            "before\n```\ncode\n```\nafter",
            "a | b, which is not a table",
        )
        for (body in bodies) {
            val blocks = MessageMarkdown.blocks(body)
            assertEquals("not one block: $body", 1, blocks.size)
            assertTrue(body, blocks.single() is MessageMarkdown.Block.Text)
        }
    }

    @Test
    fun `headings lose their markers and gain a size`() {
        assertEquals("Big", plain("# Big"))
        assertEquals("Medium", plain("## Medium"))
        assertEquals("Small", plain("### Small"))
        // Only at the start of a line, and the rest of the line is still
        // inline-parsed.
        assertEquals("not a # heading", plain("not a # heading"))
        assertEquals("bold heading", plain("# **bold** heading"))
        // No closing-sequence stripping: the second # is content.
        assertEquals("Done #", plain("# Done #"))
    }

    /**
     * The three refusals, and all three leave the line exactly as typed:
     * `#tag` is a hashtag people type, `####` is deeper than the ladder
     * goes, and `# ` alone is somebody mid-sentence.
     */
    @Test
    fun `what is not a heading is left as typed`() {
        for (body in listOf("#Heading", "#### deeper", "##### deeper still", "# ", "###")) {
            assertEquals("rewritten: $body", body, plain(body))
        }
    }

    /**
     * The ladder is the contract, not the point sizes — and it is sized in
     * `em` so a heading is a multiple of whatever the bubble's body font
     * currently is, which is what makes it follow the font-size setting.
     */
    @Test
    fun `the heading ladder is relative and descending`() {
        val sizes = listOf("# a", "## a", "### a").map {
            single(it).annotated.spanStyles.single().item.fontSize
        }
        for (size in sizes) {
            assertTrue("headings must be relative to the body font", size.isEm)
        }
        assertTrue("$sizes", sizes[0].value > sizes[1].value)
        assertTrue("$sizes", sizes[1].value > sizes[2].value)
    }

    @Test
    fun `list markers become bullets and keep their indent`() {
        assertEquals("• milk", plain("- milk"))
        assertEquals("• milk", plain("* milk"))
        assertEquals("• milk", plain("+ milk"))
        assertEquals("• milk\n• eggs", plain("- milk\n- eggs"))
        // Indent is copied verbatim, which is what nests a list visually
        // without a parser that can mis-nest one.
        assertEquals("  • nested", plain("  - nested"))
        // The content is still inline-parsed.
        assertEquals("• buy milk", plain("- buy **milk**"))
    }

    /**
     * `* X` at line start is a bullet and the emphasis parser never sees
     * the marker; a `*` anywhere else is untouched, which is what keeps
     * arithmetic readable. `---` is not a list item either — it is a
     * table's delimiter row, and on its own it is just three dashes.
     */
    @Test
    fun `what is not a list item is left as typed`() {
        for (body in listOf("---", "- ", "-", "*", "2 * 3 * 4 = 24", "a * b")) {
            assertEquals("rewritten: $body", body, plain(body))
        }
        // Ordered items are recognised by eye and deliberately left alone:
        // they already read as a list, and renumbering someone's message is
        // worse than leaving it.
        assertEquals("1. milk\n2) eggs", plain("1. milk\n2) eggs"))
    }

    @Test
    fun `a pipe table becomes a grid`() {
        val table = table("| day | who  |\n| --- | ---: |\n| Mon | Ann  |")
        assertEquals(listOf(listOf("day", "who"), listOf("Mon", "Ann")), table.cells())
        assertEquals(listOf(TextAlign.Start, TextAlign.End), table.alignments)
    }

    @Test
    fun `alignment comes from the delimiter row`() {
        val table = table("| a | b | c | d |\n| :-- | :-: | --: | --- |")
        assertEquals(
            listOf(TextAlign.Start, TextAlign.Center, TextAlign.End, TextAlign.Start),
            table.alignments,
        )
    }

    /** Leading and trailing pipes are optional, and `\|` is a literal one. */
    @Test
    fun `table rows may omit the outer pipes and escape an inner one`() {
        assertEquals(
            listOf(listOf("day", "who"), listOf("Mon", "Ann")),
            table("day | who\n--- | ---\nMon | Ann").cells(),
        )
        assertEquals(
            listOf(listOf("a | b", "c")),
            table("| a \\| b | c |\n| --- | --- |").cells(),
        )
    }

    /** Short rows are padded and long ones truncated, so the grid is square. */
    @Test
    fun `body rows are padded and truncated to the header`() {
        val table = table("| a | b |\n| - | - |\n| 1 |\n| 1 | 2 | 3 |")
        assertEquals(
            listOf(listOf("a", "b"), listOf("1", ""), listOf("1", "2")),
            table.cells(),
        )
    }

    /**
     * A header row alone is not a table — plenty of ordinary sentences
     * contain a pipe. Without a matching delimiter row directly underneath,
     * every line is left exactly as typed.
     */
    @Test
    fun `a table without a matching delimiter row is not a table`() {
        for (
            body in listOf(
                "| a | b |\n| --- |\n| 1 | 2 |",
                "| a | b |\nnot a delimiter",
                "| a | b |",
                "| --- | --- |",
                "5 | 6",
            )
        ) {
            assertEquals("rewritten: $body", body, plain(body))
        }
    }

    /**
     * Cells render emphasis, code, strikethrough and escapes — but a link
     * is left exactly as typed. That is the whole reason a table costs
     * nothing structurally: no per-cell hit test, no per-cell offset space,
     * and no second place where a label and a destination can disagree.
     */
    @Test
    fun `table cells render emphasis but never links`() {
        val table = table("| **bold** | [menu](https://example.com) |\n| --- | --- |")
        assertEquals(
            listOf(listOf("bold", "[menu](https://example.com)")),
            table.cells(),
        )
    }

    /**
     * A table splits the body, and the text on either side keeps its own
     * offset space — which is what the bubble gives its own layout result.
     */
    @Test
    fun `a table splits the body into blocks`() {
        val blocks = MessageMarkdown.blocks(
            "shopping [list](https://example.com):\n| a | b |\n| - | - |\n| 1 | 2 |\nthanks",
        )
        assertEquals(3, blocks.size)
        val first = blocks[0] as MessageMarkdown.Block.Text
        assertEquals("shopping list:", first.rendered.text)
        // The link's offsets index THIS block's string, not the body.
        val link = first.rendered.links.single()
        assertEquals("list", first.rendered.text.substring(link.start, link.end))
        assertTrue(blocks[1] is MessageMarkdown.Block.Table)
        assertEquals("thanks", (blocks[2] as MessageMarkdown.Block.Text).rendered.text)
        // No empty text block on either end of a table: a body that starts
        // or ends with one has nothing there to draw. (A body that ENDS in
        // a table is also where the streaming cursor needs a block of its
        // own — there is no last text block for it to ride.)
        val leading = MessageMarkdown.blocks("| a |\n| - |\nafter")
        assertEquals(2, leading.size)
        assertTrue(leading.first() is MessageMarkdown.Block.Table)
        val trailing = MessageMarkdown.blocks("here:\n| a |\n| - |")
        assertEquals(2, trailing.size)
        assertTrue(trailing.last() is MessageMarkdown.Block.Table)
    }

    /** The table ends at the first line that is not a row — a blank one included. */
    @Test
    fun `a table ends at a blank line`() {
        val blocks = MessageMarkdown.blocks("| a |\n| - |\n| 1 |\n\nafter")
        assertEquals(2, blocks.size)
        assertEquals(listOf(listOf("a"), listOf("1")), (blocks[0] as MessageMarkdown.Block.Table).cells())
        // The blank line the author typed is still there: only the newline
        // that ENDED the table's last row went with the split.
        assertEquals("\nafter", (blocks[1] as MessageMarkdown.Block.Text).rendered.text)
    }

    /** Nothing inside a fence is markup, a table included. */
    @Test
    fun `a table inside a fence stays code`() {
        val body = "```\n| a | b |\n| - | - |\n```"
        val blocks = MessageMarkdown.blocks(body)
        assertEquals(1, blocks.size)
        assertEquals("| a | b |\n| - | - |", plain(body))
    }

    /**
     * `# 😀` is not an emoji-only message — the raw body has a `#` in it —
     * so it renders as a heading that happens to contain an emoji. The
     * existing rule applied consistently, not a new one.
     */
    @Test
    fun `a heading may contain an emoji`() {
        assertEquals("😀", plain("# 😀"))
    }

    // -- the five near-misses, decided once for both platforms ---------------
    //
    // Where a heading, a bullet and a table could all describe the same
    // line, the two parsers were each answering plausibly and differently:
    // the same message drew as a grid on one phone and as a heading on the
    // other. These five vectors ARE the decision, and the same five appear
    // verbatim in MessageMarkdownTests.swift — a change to either parser
    // that is not also a change to the other turns one of these red.

    /**
     * 1. Precedence is heading → bullet → table.
     *
     * A pipe is ordinary punctuation inside a sentence, and a line that
     * opens with `# ` or `- ` said what it was in its first two
     * characters. Reading `# Trip | 2 people` as a header row turns the
     * marker itself into content — the cell literally reads "# Trip".
     */
    @Test
    fun `a heading or a bullet containing a pipe is not a table`() {
        assertEquals("Trip | 2 people\n--- | ---", plain("# Trip | 2 people\n--- | ---"))
        assertEquals("• milk | 2\n--- | ---", plain("- milk | 2\n--- | ---"))
    }

    /**
     * 2. A table ends at a heading or a bullet, as well as at a line with
     *    no pipes.
     *
     * The same rule as (1), applied where the table has already started:
     * a shopping list under a grid is a list, not a fourth row that eats
     * the `- `.
     */
    @Test
    fun `a table ends at a heading or a bullet`() {
        val blocks = MessageMarkdown.blocks(
            "| Day | Who |\n| --- | --- |\n| Mon | Ann |\n- and | maybe Bob",
        )
        assertEquals(2, blocks.size)
        assertEquals(
            listOf(listOf("Day", "Who"), listOf("Mon", "Ann")),
            (blocks[0] as MessageMarkdown.Block.Table).cells(),
        )
        assertEquals("• and | maybe Bob", (blocks[1] as MessageMarkdown.Block.Text).rendered.text)

        val heading = MessageMarkdown.blocks("| a |\n| - |\n| 1 |\n# Totals | so far")
        assertEquals(2, heading.size)
        assertEquals(
            listOf(listOf("a"), listOf("1")),
            (heading[0] as MessageMarkdown.Block.Table).cells(),
        )
        assertEquals("Totals | so far", (heading[1] as MessageMarkdown.Block.Text).rendered.text)
    }

    /**
     * 3. A body row that parses to ZERO cells ends the table, and the line
     *    is left as typed.
     *
     * A lone `|` pads to a row of empty strings if you let it: a blank
     * stripe in the grid, and the character the author typed gone. A
     * phantom row is worse than stopping.
     */
    @Test
    fun `a body row with no cells ends the table rather than padding`() {
        val blocks = MessageMarkdown.blocks("| a | b |\n| - | - |\n|")
        assertEquals(2, blocks.size)
        assertEquals(
            listOf(listOf("a", "b")),
            (blocks[0] as MessageMarkdown.Block.Table).cells(),
        )
        assertEquals("|", (blocks[1] as MessageMarkdown.Block.Text).rendered.text)
    }

    /**
     * 4. A delimiter row must contain at least one pipe.
     *
     * `---` on a line of its own is a rule or a signature separator far
     * more often than it is a one-column table, so a one-column table has
     * to be written `| --- |`. Accepting the bare form turned every
     * `something | else` above a dashed line into a grid.
     */
    @Test
    fun `a pipe-less delimiter row is not a delimiter row`() {
        assertEquals("| Total |\n---\n| 12 |", plain("| Total |\n---\n| 12 |"))
        // Written with the pipes, the same one-column table parses.
        assertEquals(
            listOf(listOf("Total"), listOf("12")),
            table("| Total |\n| --- |\n| 12 |").cells(),
        )
    }

    /**
     * 5. Whitespace-only content is not content.
     *
     * `# ` with nothing after it is somebody mid-sentence, and two spaces
     * after it is the same person. Both leave the line exactly as typed —
     * the marker never disappears into a heading whose text is a space.
     */
    @Test
    fun `a marker followed by only whitespace is left exactly as typed`() {
        for (body in listOf("#  ", "-  ", "+  ", "## \t")) {
            assertEquals("rewritten: $body", body, plain(body))
        }
    }

    // ---- rich blocks (#97) -------------------------------------------------
    //
    // docs/protocol.md, "Rich blocks: formulas, diagrams and graphs", and the
    // design doc's tables (docs/capitals-and-rich-markdown-2026-10-17.md,
    // "The cases every parser's tests hold"), row for row. The other three
    // parsers hold the same rows; these are the contract between them.

    /** Each block as one line: `text:<rendered>`, `rich:<kind>:<engine>:<source>`, `table`. */
    private fun shape(body: String): List<String> = MessageMarkdown.blocks(body).map { block ->
        when (block) {
            is MessageMarkdown.Block.Text -> "text:${block.rendered.text}"
            is MessageMarkdown.Block.Table -> "table"
            is MessageMarkdown.Block.Rich -> "rich:${block.kind.wire}:${block.engine ?: "-"}:${block.source}"
        }
    }

    /** The one rich block [body] is. */
    private fun rich(body: String): MessageMarkdown.Block.Rich =
        MessageMarkdown.blocks(body).single() as MessageMarkdown.Block.Rich

    /** The inline formulas of a single-text-block [body], by source. */
    private fun formulas(body: String): List<String> {
        val annotated = single(body).annotated
        return annotated.getStringAnnotations(MessageMarkdown.MATH_TAG, 0, annotated.length).map { it.item }
    }

    @Test
    fun `the design doc's block table`() {
        val rows = listOf(
            "```math\nx^2\n```" to listOf("rich:math:-:x^2"),
            "```Mermaid\ngraph TD\nA-->B\n```" to listOf("rich:mermaid:-:graph TD\nA-->B"),
            "```mermaid extra words\nA\n```" to listOf("rich:mermaid:-:A"),
            "```neato\ngraph {a--b}\n```" to listOf("rich:graphviz:neato:graph {a--b}"),
            "```gv\ndigraph{a->b}\n```" to listOf("rich:graphviz:dot:digraph{a->b}"),
            "```plot\nsin(x)\n```" to listOf("rich:plot:-:sin(x)"),
            "```js\nx\n```" to listOf("text:x"),
            "```mermaid\nA" to listOf("text:```mermaid\nA"),
            "Look:\n```math\nx\n```\nDone" to listOf("text:Look:", "rich:math:-:x", "text:Done"),
            "\$\$\nx^2\n\$\$" to listOf("rich:math:-:x^2"),
            "\$\$x^2\$\$" to listOf("rich:math:-:x^2"),
            "\\[\na\n\\]" to listOf("rich:math:-:a"),
            "  \$\$ y \$\$  " to listOf("rich:math:-:y"),
            "\$\$x\$\$ is it" to listOf("text:\$\$x\$\$ is it"),
            "Area \$\$x\$\$" to listOf("text:Area \$\$x\$\$"),
            "\$\$\nx" to listOf("text:\$\$\nx"),
            "\$\$ \$\$" to listOf("text:\$\$ \$\$"),
            "```\n\$\$x\$\$\n```" to listOf("text:\$\$x\$\$"),
        )
        for ((body, expected) in rows) {
            assertEquals("body: $body", expected, shape(body))
        }
        // The two rows that are text WITH a formula in them, said exactly.
        assertEquals(listOf("x"), formulas("\$\$x\$\$ is it"))
        assertEquals(listOf("x"), formulas("Area \$\$x\$\$"))
        // ...and the ones that are text with NONE.
        assertEquals(emptyList<String>(), formulas("\$\$\nx"))
        assertEquals(emptyList<String>(), formulas("\$\$ \$\$"))
        assertEquals(emptyList<String>(), formulas("```\n\$\$x\$\$\n```"))
    }

    /** Every tag of the protocol's table, and the layout each one names. */
    @Test
    fun `every rich tag, case-blind, and nothing else`() {
        val tags = mapOf(
            "math" to ("math" to null), "latex" to ("math" to null), "tex" to ("math" to null),
            "LaTeX" to ("math" to null), "mermaid" to ("mermaid" to null),
            "dot" to ("graphviz" to "dot"), "graphviz" to ("graphviz" to "dot"), "GV" to ("graphviz" to "dot"),
            "neato" to ("graphviz" to "neato"), "circo" to ("graphviz" to "circo"), "fdp" to ("graphviz" to "fdp"),
            "sfdp" to ("graphviz" to "sfdp"), "twopi" to ("graphviz" to "twopi"),
            "osage" to ("graphviz" to "osage"), "patchwork" to ("graphviz" to "patchwork"),
            "plot" to ("plot" to null),
        )
        for ((tag, kindAndEngine) in tags) {
            val block = rich("```$tag\nbody\n```")
            assertEquals(tag, kindAndEngine.first, block.kind.wire)
            assertEquals(tag, kindAndEngine.second, block.engine)
            assertEquals(tag, "body", block.source)
        }
        // A tag is a WORD, not a prefix, and the lower-casing is ASCII only:
        // a Kelvin sign is not a K and a dotless ı is not an i.
        for (tag in listOf("mathematics", "plotly", "dotnet", "kotlin", "MATH\u0130", "\u212Aotlin")) {
            assertEquals(tag, listOf("text:body"), shape("```$tag\nbody\n```"))
        }
        // Spaces and tabs around the tag are not part of it.
        assertEquals("graphviz", rich("```  \tneato  \nbody\n```").kind.wire)
    }

    /**
     * What a rich block shows until it is drawn: a fence's content, as a
     * code block shows it today; a display formula's lines as typed,
     * delimiters and all.
     */
    @Test
    fun `a rich block's fallback is the source as it shows today`() {
        assertEquals("x^2", rich("```math\nx^2\n```").shown)
        assertEquals("\$\$\nx^2\n\$\$", rich("\$\$\nx^2\n\$\$").shown)
        assertEquals("  \$\$ y \$\$  ", rich("  \$\$ y \$\$  ").shown)
        assertEquals("\\[a\\]", rich("\\[a\\]").shown)
        // A multi-line display formula keeps its inner lines and trims only
        // the edges of the whole.
        assertEquals("a\n  b", rich("\$\$\n a\n  b \n\$\$").source)
    }

    /** Rule 2: the FIRST closer decides, and must end its line. */
    @Test
    fun `a display formula closes at the first closer, which must end its line`() {
        assertEquals(listOf("rich:math:-:x"), shape("\$\$ x \$\$ \t"))
        // The first `$$` has words after it: not display math, even though
        // a later line would have closed cleanly.
        assertEquals(1, MessageMarkdown.blocks("\$\$\na \$\$ b\n\$\$").size)
        assertTrue(MessageMarkdown.blocks("\$\$\na \$\$ b\n\$\$").single() is MessageMarkdown.Block.Text)
        // `\[` closes only on `\]`, and `$$` only on `$$`.
        // (As text, the backslash escapes the bracket, as it always has.)
        assertEquals(listOf("text:[\na\n\$\$"), shape("\\[\na\n\$\$"))
        // Before tables: a formula with a pipe in it is a formula.
        assertEquals(listOf("rich:math:-:|a| \\\\\n|---|"), shape("\$\$\n|a| \\\\\n|---|\n\$\$"))
        // An escaped opener is not one.
        assertEquals(listOf("text:\$\$x\$\$"), shape("\\\$\$x\$\$"))
    }

    /** Rule 3: a rich block splits the text around it exactly as a table does. */
    @Test
    fun `a rich block stands alone`() {
        assertEquals(
            listOf("text:above", "rich:math:-:E=mc^2", "text:below"),
            shape("above\n\$\$E=mc^2\$\$\nbelow"),
        )
        assertEquals(
            listOf("rich:plot:-:sin(x)", "rich:mermaid:-:A"),
            shape("```plot\nsin(x)\n```\n```mermaid\nA\n```"),
        )
        // A blank line typed by the author is kept, as a table keeps one.
        assertEquals(listOf("text:a\n", "rich:math:-:x", "text:\nb"), shape("a\n\n\$\$x\$\$\n\nb"))
        // Code and rich blocks side by side keep their own shapes.
        assertEquals(
            listOf("text:let x = 1", "rich:math:-:x"),
            shape("```\nlet x = 1\n```\n```math\nx\n```"),
        )
        // The links of the text around a rich block index THEIR block.
        val blocks = MessageMarkdown.blocks("see [this](https://example.com)\n```math\nx\n```")
        val first = blocks.first() as MessageMarkdown.Block.Text
        val link = first.rendered.links.single()
        assertEquals("this", first.rendered.text.substring(link.start, link.end))
    }

    /** An empty rich fence is still a rich block — fc-rich.js calls it "empty". */
    @Test
    fun `an empty rich fence is a rich block with nothing to draw`() {
        val block = rich("```math\n```")
        assertEquals("", block.source)
        assertEquals("", block.shown)
    }

    @Test
    fun `the design doc's inline table`() {
        val rows = listOf(
            "\$x\$" to listOf("x"),
            "\$5 and \$10" to emptyList(),
            "costs \$5, or \$6." to emptyList(),
            "a\$b\$c" to emptyList(),
            "\$x\$s" to emptyList(),
            "\$ x\$" to emptyList(),
            "\$x \$" to emptyList(),
            "\\\$x\$" to emptyList(),
            "\$a\\\$b\$" to listOf("a\\\$b"),
            "see \$\$x\$\$ now" to listOf("x"),
            "\\(x^2\\)" to listOf("x^2"),
            "`\$x\$`" to emptyList(),
            "*see \$a*b\$*" to listOf("a*b"),
            "\$x\ny\$" to emptyList(),
            "[a](http://x.y/\$b\$)" to emptyList(),
            "€\$x\$" to listOf("x"),
            "2\$x\$" to emptyList(),
        )
        for ((body, expected) in rows) {
            assertEquals("body: $body", expected, formulas(body))
        }
        // The table row: `a` in the header, `c` in the first row.
        val table = table("| \$a\$ | b |\n|---|---|\n| \$c\$ | d |")
        fun cellFormulas(cell: androidx.compose.ui.text.AnnotatedString) =
            cell.getStringAnnotations(MessageMarkdown.MATH_TAG, 0, cell.length).map { it.item }
        assertEquals(listOf("a"), cellFormulas(table.header[0]))
        assertEquals(emptyList<String>(), cellFormulas(table.header[1]))
        assertEquals(listOf("c"), cellFormulas(table.rows[0][0]))
    }

    /**
     * What the inline rows READ as — the run is the source with its
     * delimiters, monospaced, so the not-yet-drawn bubble shows exactly
     * what was typed; the escapes outside a formula are still resolved.
     */
    @Test
    fun `an inline formula's run is its source with its delimiters`() {
        assertEquals("\$x\$", plain("\$x\$"))
        assertEquals("\$x\$", plain("\\\$x\$"))
        assertEquals("\$a\\\$b\$", plain("\$a\\\$b\$"))
        assertEquals("see \$\$x\$\$ now", plain("see \$\$x\$\$ now"))
        assertEquals("\\(x^2\\)", plain("\\(x^2\\)"))
        // Inside the emphasis, and the emphasis is still consumed.
        val rendered = single("*see \$a*b\$*")
        assertEquals("see \$a*b\$", rendered.text)
        val range = rendered.math.single()
        assertEquals("\$a*b\$", rendered.text.substring(range.first, range.last + 1))
        // The run is monospaced, and carries the inline-content id.
        val code = rendered.annotated.spanStyles.any {
            it.item.fontFamily == androidx.compose.ui.text.font.FontFamily.Monospace &&
                it.start == range.first && it.end == range.last + 1
        }
        assertTrue("the run is monospaced", code)
        val ids = rendered.annotated.getStringAnnotations(range.first, range.last + 1).map { it.item }
        assertTrue(ids.toString(), MessageMarkdown.MATH_ID_PREFIX + "a*b" in ids)
    }

    /** Markdown inside a formula is not markdown — its backticks included. */
    @Test
    fun `markdown inside a formula is left to KaTeX`() {
        assertEquals(listOf("a_1 + b_2"), formulas("\$a_1 + b_2\$"))
        assertEquals("\$a_1 + b_2\$", plain("\$a_1 + b_2\$"))
        assertEquals(listOf("**x**"), formulas("\$**x**\$"))
        assertEquals(listOf("\\frac{a}{b}"), formulas("then \$\\frac{a}{b}\$ ok"))
        // `_` is a word character, so in `_$x$_` no `$` can open or close:
        // the emphasis wins — and the answer is the same at every depth.
        assertEquals(emptyList<String>(), formulas("_\$x\$_"))
        assertEquals("\$x\$", plain("_\$x\$_"))
        // A code span wins over a formula that would reach into it.
        assertEquals(emptyList<String>(), formulas("\$a `b\$` c"))
    }

    /** The precedence of the three delimiters, and their content rules. */
    @Test
    fun `inline delimiters, by precedence`() {
        assertEquals(listOf("a", "b"), formulas("\$\$a\$\$ and \\(b\\)"))
        assertEquals(emptyList<String>(), formulas("\$\$ \$\$"))
        assertEquals(emptyList<String>(), formulas("\\( \\)"))
        assertEquals(emptyList<String>(), formulas("\\(a\nb\\)"))
        // An escaped `\(` is a literal parenthesis.
        assertEquals(emptyList<String>(), formulas("\\\\(a\\)"))
        // A failed closer is skipped and the search goes on.
        assertEquals(listOf("a \$b"), formulas("\$a \$b\$"))
        // Money next to a formula.
        assertEquals(listOf("x"), formulas("\$x\$ costs \$5"))
        // ...and the rule taken literally when money comes FIRST: the `$`
        // before `x` has a space before it and cannot close, so the opener
        // at `$5` runs on to the next `$` that can. Every port does this.
        assertEquals(listOf("5 but \$x"), formulas("it costs \$5 but \$x\$ is free"))
        // The same formula twice is two runs, one source.
        assertEquals(listOf("x", "x"), formulas("\$x\$ and \$x\$"))
        assertEquals(listOf("x"), MessageMarkdown.mathSources(single("\$x\$ and \$x\$").annotated))
    }

    /** ASCII on purpose: four languages, one idea of a word character. */
    @Test
    fun `the word guard is ASCII`() {
        assertEquals(listOf("x"), formulas("é\$x\$"))
        assertEquals(listOf("x"), formulas("Привет \$x\$, пока"))
        assertEquals(listOf("x"), formulas("Привет\$x\$пока"))
        assertEquals(listOf("x"), formulas("٣\$x\$"))
        assertEquals(emptyList<String>(), formulas("_\$x\$"))
        assertEquals(emptyList<String>(), formulas("Z\$x\$"))
        assertEquals(listOf("π r^2"), formulas("Площадь: \$π r^2\$ 🎉"))
    }

    /**
     * A formula in a link LABEL is flattened to what was typed — no
     * drawing inside a link; in the destination it is never a formula; and
     * a formula that swallows the `]` beats the link, as a code span does.
     */
    @Test
    fun `formulas and links, in the order a reader meets them`() {
        val rendered = single("[area \$r^2\$](https://example.com/\$x\$)")
        assertEquals("area \$r^2\$", rendered.text)
        assertEquals(emptyList<String>(), MessageMarkdown.mathSources(rendered.annotated))
        val link = rendered.links.single()
        assertEquals("https://example.com/\$x\$", link.url)
        assertEquals("area \$r^2\$", rendered.text.substring(link.start, link.end))
        // The formula reaches past the `]`: formula `a](b`, and no link.
        val swallowed = single("[\$a](b\$)")
        assertEquals(listOf("a](b"), MessageMarkdown.mathSources(swallowed.annotated))
        assertTrue(swallowed.links.isEmpty())
        // In a table cell links are not drawn, but a destination is still
        // a destination.
        val cell = table("| [a](u/\$b\$) |\n|---|").header.single()
        assertEquals(emptyList<String>(), MessageMarkdown.mathSources(cell))
    }

    /** The readings every port shares beyond the design doc's tables. */
    @Test
    fun `the shared readings of the rules`() {
        // A code span that a formula's closer would have to cross wins...
        assertEquals(emptyList<String>(), formulas("\$a `b\$` c\$"))
        // ...but a lone backtick opens nothing.
        assertEquals(listOf("a`b"), formulas("\$a`b\$"))
        // Inline `$$ x $$` keeps its spaces; only display math is trimmed.
        assertEquals(listOf(" x "), formulas("see \$\$ x \$\$ now"))
        // An unclosed `$$`: the first `$` is text and the next is tried.
        assertEquals(emptyList<String>(), formulas("x \$\$\$x\$"))
        // The first display closer decides: words after it, and it is text.
        assertEquals(listOf("text:\$\$a\$\$b\$\$"), shape("\$\$a\$\$b\$\$"))
        // The info string follows ALL the leading backticks.
        assertEquals(listOf("rich:math:-:x"), shape("````math\nx\n````"))
        // A carriage return is not blank — not in an info string, not
        // after a display closer.
        assertEquals(listOf("text:x"), shape("```math\r\nx\n```"))
        assertEquals(listOf("x"), formulas("\$\$x\$\$\r"))
        // A formula's opening line never becomes a row of the table above.
        assertEquals(
            listOf("table", "rich:math:-:|x|"),
            shape("| a |\n|---|\n| 1 |\n\$\$ |x| \$\$"),
        )
    }

    /** Headings, bullets and the text around a formula keep working. */
    @Test
    fun `formulas ride headings and bullets`() {
        assertEquals(listOf("x"), formulas("# Solve \$x\$"))
        assertEquals("• root \$\\sqrt{2}\$", plain("- root \$\\sqrt{2}\$"))
        assertEquals(listOf("\\sqrt{2}"), formulas("- root \$\\sqrt{2}\$"))
    }
}
