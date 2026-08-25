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
}
