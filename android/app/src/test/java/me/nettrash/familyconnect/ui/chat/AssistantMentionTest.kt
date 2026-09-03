package me.nettrash.familyconnect.ui.chat

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Pins the `@ai` grammar. It is a cross-platform contract — the SERVER
 * decides from the same rule whether a family-chat message reaches the
 * assistant at all (server/src/mentions.rs) and iOS draws the same
 * highlight (AssistantMention.swift) — so the vectors here ARE the spec and
 * the same table appears in all three places. A disagreement between them is
 * not a cosmetic bug: it is either a highlighted question that is never
 * answered, or an ordinary family message that quietly leaves the server.
 *
 * That agreement is CHECKED rather than trusted: the server's
 * `the_three_ports_carry_the_same_vectors` reads this file and iOS's
 * AssistantMentionTests.swift and compares all three tables character by
 * character, in order. Each port's own suite goes green against its own
 * copy, so drift is invisible from any single vantage point — which is how
 * two vectors once reached the server and only one client.
 */
class AssistantMentionTest {

    private val mentions = listOf(
        "@ai",
        "@AI",
        "@Ai",
        "@ai what is the weather",
        "hey @ai what is the weather",
        "hey @ai, what is the weather",
        "ask @ai.",
        "(@ai)",
        "\n@ai\n",
        "@@ai",
        "какая погода @ai",
        // No space after the token, which is normal in Japanese — the
        // ASCII-only boundary is what makes this a mention.
        "@aiこんにちは",
        "-@ai",
        "@ai?",
        "1+@ai",
    )

    private val notMentions = listOf(
        "",
        "@",
        "@a",
        "@aiden",
        "@ai_bot",
        "@ai2",
        "@aI3",
        "anna@ai.example",
        "x@ai",
        "1@ai",
        "_@ai",
        "ai",
        "email me at bob@aim.com",
        "@artificial intelligence",
    )

    @Test
    fun `every shared vector that is a mention`() {
        for (body in mentions) {
            assertTrue("should be a mention: $body", AssistantMention.mentions(body))
        }
    }

    @Test
    fun `every shared vector that is not`() {
        for (body in notMentions) {
            assertFalse("should NOT be a mention: $body", AssistantMention.mentions(body))
        }
    }

    @Test
    fun `the range covers the token and nothing else`() {
        val body = "hey @ai there"
        val ranges = AssistantMention.ranges(body)
        assertEquals(1, ranges.size)
        assertEquals("@ai", body.substring(ranges[0].first, ranges[0].last + 1))
    }

    /**
     * Where a byte-vs-code-unit mix-up would show. Kotlin indexes UTF-16 and
     * the server indexes UTF-8, so anything that carried the server's
     * offsets over would slice these in the wrong place.
     */
    @Test
    fun `a mention after non-ASCII text is sliced cleanly`() {
        for (body in listOf("Привет @ai", "こんにちは @ai です", "🇷🇸 @ai")) {
            val ranges = AssistantMention.ranges(body)
            assertEquals("one mention in: $body", 1, ranges.size)
            assertEquals("@ai", body.substring(ranges[0].first, ranges[0].last + 1))
        }
    }

    @Test
    fun `all of them, not just the first`() {
        val body = "@ai and also @AI, but not @aiden"
        val ranges = AssistantMention.ranges(body)
        assertEquals(2, ranges.size)
        assertEquals(
            listOf("@ai", "@AI"),
            ranges.map { body.substring(it.first, it.last + 1) },
        )
    }

    @Test
    fun `a near miss does not hide a real one`() {
        assertTrue(AssistantMention.mentions("@aiden asked @ai"))
        val ranges = AssistantMention.ranges("@@ai")
        assertEquals(1, ranges.size)
        assertEquals("@ai", "@@ai".substring(ranges[0].first, ranges[0].last + 1))
    }

    // -- Pictures ---------------------------------------------------------

    /**
     * The picture vectors, mirrored exactly as the mention ones are:
     * `server/src/mentions.rs` (`DRAWS`) and
     * `ios/FamilyConnectTests/AssistantMentionTests.swift` carry this same
     * table, prompt and all, and a change here is a change in three places
     * or it is a bug.
     *
     * Each pair is (body, the prompt that must come out of it). Asserting
     * the PROMPT rather than a bool is the point: what comes back is the
     * whole of what will leave the server, so a test that only asked "is
     * this a draw?" would not notice the token travelling with it.
     */
    private val draws = listOf(
        "/draw a cat" to "a cat",
        "/DRAW a cat" to "a cat",
        "/Draw a cat" to "a cat",
        "   /draw a cat  " to "a cat",
        "/draw\na cat" to "a cat",
        // The family chat: one leading mention, and the words after the
        // token are still the whole of what leaves.
        "@ai /draw a cat" to "a cat",
        "@AI    /draw a cat in a hat" to "a cat in a hat",
        "@ai\n/draw a cat" to "a cat",
        // A prompt that itself contains the tokens is still just words.
        "/draw @ai holding a sign" to "@ai holding a sign",
        "/draw /draw" to "/draw",
        // The PROMPT in the scripts this app is translated into.
        "/draw \u043A\u043E\u0442 \u0432 \u0448\u043B\u044F\u043F\u0435" to
            "\u043A\u043E\u0442 \u0432 \u0448\u043B\u044F\u043F\u0435",
        "/draw \u732B" to "\u732B",
        "@ai /draw \uD83D\uDC08 on a mat" to "\uD83D\uDC08 on a mat",
        // WHITESPACE IS UNICODE `White_Space`, and this is the code point
        // where Kotlin's own answer used to differ. U+0085 NEXT LINE IS
        // whitespace to Unicode and to the server; Java's
        // `Character.isWhitespace` — which is what `Char.isWhitespace()` is
        // built on — says it is not, so this body used to come back as an
        // ordinary message while the server drew a picture from it.
        "/draw\u0085a cat" to "a cat",
        // U+200B ZERO WIDTH SPACE is NOT whitespace, so it stays in the
        // prompt, and a body that is nothing but the token and one of them
        // IS a request. Kotlin agrees with the server here; Foundation's
        // `CharacterSet.whitespacesAndNewlines` does not, which is what
        // Apple's side had to hand-roll around.
        "/draw \u200Bcat" to "\u200Bcat",
        "/draw \u200B" to "\u200B",
    )

    private val notDraws = listOf(
        "",
        "/draw",
        "  /draw  ",
        "/drawer",
        "/draws a cat",
        "/draw,a cat",
        "draw a cat",
        // The token has to be FIRST. This is the rule that keeps a family
        // DISCUSSING the feature from generating pictures of it.
        "hey @ai /draw a cat",
        "what does /draw do?",
        "please /draw a cat",
        // A mention that is not at the start does not license the token
        // either, for the same reason.
        "look @ai /draw a cat",
        // Two mentions: the second is not a leading one.
        "@ai @ai /draw a cat",
        // NON-ASCII BODIES. On the server these are the shapes that used
        // to bring the request down: `str::split_at` traps when its index
        // is not a character boundary, and the fifth BYTE of a body that
        // opens in Cyrillic, Japanese or Chinese usually is not one. Kotlin
        // counts in UTF-16 and cannot trap the same way, but the same
        // table is pinned on all three ports so a port that DID slice by
        // byte would be caught here too.
        "\u041F\u0440\u0438\u0432\u0435\u0442",
        "\u041F\u0440\u0438\u0432\u0435\u0442, \u043A\u0430\u043A \u0434\u0435\u043B\u0430?",
        "@ai \u041F\u0440\u0438\u0432\u0435\u0442",
        "\u3053\u3093\u306B\u3061\u306F",
        "\u4F60\u597D\u4E16\u754C",
        "\u0417\u0434\u0440\u0430\u0432\u043E",
        // The same trap one byte further in: `/dra` then a three-byte
        // character.
        "/dra\u20AC a cat",
        // A BARE PREFIX OF THE TOKEN, in each script that trips the byte
        // index — three shorter than the token, then bodies of five bytes
        // or more with no character boundary at five.
        "/d",
        "/dr",
        "/dra",
        "/dr\u0438",
        "/dr\u3042",
        "/dr\uD83D\uDC08",
        "\u041F",
        "\u041F\u0440",
        "\u041F\u0440\u0438",
        "\u041F\u0440\u0438\u0432",
        "\u3053",
        "\u3053\u3093",
        "\uD83D\uDC08",
        "\uD83D\uDC08\uD83D\uDC08",
        "\uD83C\uDFA8 \u043D\u0430\u0440\u0438\u0441\u0443\u0439 \u043A\u043E\u0442\u0430",
        "@ai \uD83D\uDC08",
        // A HOMOGLYPH: Cyrillic \u0430 where the token wants ASCII `a`.
        "/dr\u0430w a cat",
        // U+001C-U+001F are NOT whitespace, so these are the token followed
        // by a longer word — which is what the server reads. Java's
        // `Character.isWhitespace` says they ARE whitespace, so this file
        // used to read four picture requests here that the server ignores.
        "/draw\u001Ca cat",
        "/draw\u001Da cat",
        "/draw\u001Ea cat",
        "/draw\u001Fa cat",
        // A COMBINING MARK on the token's last letter: one character past
        // `/draw` is U+0301, which is not whitespace, so this is a longer
        // word. Kotlin and the server agree; Swift counts grapheme
        // CLUSTERS, where `w` and its accent are one, and Apple's side had
        // to move to scalars to agree with both.
        "/draw\u0301 a cat",
        "@ai /draw\u0301 a cat",
    )

    @Test
    fun `a picture request is the token first and the words after it`() {
        for ((body, prompt) in draws) {
            assertEquals("$body asks for a picture of $prompt", prompt, AssistantMention.drawPrompt(body))
        }
    }

    @Test
    fun `everything else is an ordinary message`() {
        for (body in notDraws) {
            assertNull("$body is not a picture request", AssistantMention.drawPrompt(body))
        }
    }

    /**
     * The two grammars are independent, and the family chat needs both to
     * be true at once: the server only looks for `/draw` on a message that
     * already mentioned the assistant.
     */
    @Test
    fun `a family chat picture request is also a mention`() {
        assertTrue(AssistantMention.mentions("@ai /draw a cat"))
        assertEquals("a cat", AssistantMention.drawPrompt("@ai /draw a cat"))
        // And a picture request in a PRIVATE thread mentions nobody, which
        // is why the private path never consults the mention grammar.
        assertFalse(AssistantMention.mentions("/draw a cat"))
    }

    /**
     * The fold is ASCII-only, exactly as the mention's boundary test is.
     * Kotlin's own `ignoreCase` is a Unicode fold, and three
     * implementations depending on three Unicode tables agreeing is the
     * trap this file exists to avoid.
     */
    @Test
    fun `a look-alike token is not the token`() {
        // Cyrillic а and Latin a look identical and are not the same
        // character; the server compares bytes and would refuse this too.
        assertNull(AssistantMention.drawPrompt("/drаw a cat"))
    }

    /**
     * The highlight's half of the contract, computed by the same scan the
     * prompt is: the range covers the TOKEN and nothing else, wherever
     * the leading whitespace and the optional mention put it.
     */
    @Test
    fun `the range points at the picture token itself`() {
        for ((body, _) in draws) {
            val range = AssistantMention.drawRange(body)
            assertNotNull("a request must have a range: $body", range)
            assertEquals(
                AssistantMention.DRAW,
                body.substring(range!!.first, range.last + 1).lowercase(),
            )
        }
        for (body in notDraws) {
            assertNull("not a request, so no range: $body", AssistantMention.drawRange(body))
        }
    }

    /** Byte-vs-code-unit again: a multibyte prompt must still slice cleanly. */
    @Test
    fun `a picture request after non-ASCII text is sliced cleanly`() {
        val body = "@ai /draw кот в шляпе"
        val range = AssistantMention.drawRange(body)!!
        assertEquals("/draw", body.substring(range.first, range.last + 1))
        assertEquals("кот в шляпе", AssistantMention.drawPrompt(body))
    }

    // -- What the composer button leaves behind ---------------------------

    /**
     * The button's output has to be a body the SERVER will read as a
     * request — this is the composer's half of the same wire contract, so
     * every case is checked by feeding the result back through the
     * grammar rather than by comparing strings alone.
     */
    @Test
    fun `the composer builds a request the grammar accepts`() {
        assertEquals("/draw a cat", AssistantMention.withDraw("a cat", inFamilyChat = false))
        assertEquals("@ai /draw a cat", AssistantMention.withDraw("a cat", inFamilyChat = true))
        for (inFamily in listOf(false, true)) {
            assertEquals(
                "a cat",
                AssistantMention.drawPrompt(AssistantMention.withDraw("a cat", inFamily)),
            )
        }
    }

    /**
     * A mention the writer typed themselves is absorbed, never doubled:
     * `@ai @ai /draw a cat` is one of the NOT_DRAWS vectors, because the
     * second mention is not a leading one.
     */
    @Test
    fun `a mention already typed is not doubled`() {
        val built = AssistantMention.withDraw("@ai a cat", inFamilyChat = true)
        assertEquals("@ai /draw a cat", built)
        assertEquals("a cat", AssistantMention.drawPrompt(built))
    }

    /** Pressing it twice must not ask for a picture OF the token. */
    @Test
    fun `a body that already asks comes back untouched`() {
        for (inFamily in listOf(false, true)) {
            assertEquals("/draw a cat", AssistantMention.withDraw("/draw a cat", inFamily))
        }
        assertEquals(
            "@ai /draw a cat",
            AssistantMention.withDraw("@ai /draw a cat", inFamilyChat = true),
        )
    }

    /** An empty composer still produces a valid start to type into. */
    @Test
    fun `an empty draft becomes the bare token`() {
        assertEquals("/draw ", AssistantMention.withDraw("", inFamilyChat = false))
        assertEquals("@ai /draw ", AssistantMention.withDraw("   ", inFamilyChat = true))
        // Not yet a request — `/draw` alone is an ordinary message — which
        // is correct: the member has still to type what they want.
        assertNull(AssistantMention.drawPrompt(AssistantMention.withDraw("", false)))
    }

    /**
     * THE WHITESPACE TABLE, code point by code point, and the five names
     * in it are the whole reason this file hand-rolls the predicate.
     *
     * Whitespace is where three hand-written ports of one grammar part
     * company. The server is Rust's `char.is_whitespace`, which is the
     * Unicode `White_Space` property. Kotlin's `Char.isWhitespace()` is
     * Java's `Character.isWhitespace(c) || Character.isSpaceChar(c)`, and
     * that union calls U+001C, U+001D, U+001E and U+001F whitespace when
     * Unicode does not, and refuses U+0085 NEXT LINE when Unicode calls it
     * whitespace. Five code points, and each one is a body the server and
     * this bubble used to disagree about.
     *
     * `server/src/mentions.rs` asserts the same table in
     * `whitespace_is_the_unicode_property_and_nothing_else`, and iOS's
     * `AssistantMentionTests.whitespaceIsTheUnicodeProperty` again — where
     * a THIRD answer, Foundation's `CharacterSet.whitespacesAndNewlines`,
     * adds U+200B.
     */
    @Test
    fun `the five code points Java and Unicode disagree about`() {
        // Java says whitespace; Unicode does not. So each of these is the
        // token followed by a longer word, and no picture is requested.
        for (separator in listOf('\u001C', '\u001D', '\u001E', '\u001F')) {
            assertTrue(
                "Kotlin still calls U+00${Integer.toHexString(separator.code).uppercase()} whitespace",
                separator.isWhitespace(),
            )
            assertNull(
                "U+00${Integer.toHexString(separator.code).uppercase()} does not separate the token",
                AssistantMention.drawPrompt("/draw${separator}a cat"),
            )
            assertNull(AssistantMention.drawRange("/draw${separator}a cat"))
        }
        // Unicode says whitespace; Java does not. So this one DOES separate
        // the token, and it is trimmed out of the prompt.
        assertFalse("Kotlin still refuses U+0085", '\u0085'.isWhitespace())
        assertEquals("a cat", AssistantMention.drawPrompt("/draw\u0085a cat"))
        assertEquals(0..4, AssistantMention.drawRange("/draw\u0085a cat"))
        assertNull(AssistantMention.drawPrompt("/draw\u0085\u0085"))
        // Leading whitespace is skipped by the same predicate, so a body
        // that opens with U+0085 is still a request and one that opens with
        // U+001C is still ordinary text.
        assertEquals("a cat", AssistantMention.drawPrompt("\u0085/draw a cat"))
        assertNull(AssistantMention.drawPrompt("\u001C/draw a cat"))
        // …and the composer builds its prefix with the same predicate.
        assertEquals("/draw a cat", AssistantMention.withDraw("\u0085a cat", inFamilyChat = false))
    }

    /**
     * The whole `White_Space` property, so a future port can be checked
     * against this file rather than against a summary of it.
     */
    @Test
    fun `whitespace is the unicode property and nothing else`() {
        val whitespace = listOf(
            '\u0009', '\u000A', '\u000B', '\u000C', '\u000D', '\u0020', '\u0085', '\u00A0',
            '\u1680', '\u2000', '\u2001', '\u2002', '\u2003', '\u2004', '\u2005', '\u2006',
            '\u2007', '\u2008', '\u2009', '\u200A', '\u2028', '\u2029', '\u202F', '\u205F',
            '\u3000',
        )
        for (space in whitespace) {
            assertEquals(
                "U+${Integer.toHexString(space.code).uppercase()} separates the token",
                "a cat",
                AssistantMention.drawPrompt("/draw${space}a cat"),
            )
        }
        // U+200B ZERO WIDTH SPACE sits one past the end of the U+2000 quad
        // and is NOT in the property — it stays in the prompt.
        assertEquals("\u200Ba cat", AssistantMention.drawPrompt("/draw \u200Ba cat"))
        assertNull(AssistantMention.drawPrompt("/draw\u200Ba cat"))
    }
}
