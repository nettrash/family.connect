package me.nettrash.familyconnect.ui.chat

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

    private fun plain(body: String) = MessageMarkdown.render(body).text

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
        val rendered = MessageMarkdown.render("see [wiki](https://en.wikipedia.org/wiki/Foo_(bar)) today")
        assertEquals("see wiki today", rendered.text)
        assertEquals("https://en.wikipedia.org/wiki/Foo_(bar)", rendered.links.single().url)
    }

    @Test
    fun `a backslash escapes the next character`() {
        assertEquals("**not bold**", plain("\\*\\*not bold\\*\\*"))
    }

    @Test
    fun `a markdown link shows its label and carries its destination`() {
        val rendered = MessageMarkdown.render("see [the menu](https://example.com/menu) today")
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
        val rendered = MessageMarkdown.render("[here](example.com/x)")
        assertEquals("https://example.com/x", rendered.links.single().url)
    }

    /**
     * THE invariant, stated as an assertion: a link's offsets must index
     * the rendered text even when markup ahead of it shortened the string.
     */
    @Test
    fun `link offsets index the rendered text, not the raw body`() {
        val body = "**important** see [the menu](https://example.com) now"
        val rendered = MessageMarkdown.render(body)
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
}
