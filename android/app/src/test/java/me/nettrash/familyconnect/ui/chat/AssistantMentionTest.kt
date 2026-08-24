package me.nettrash.familyconnect.ui.chat

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
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
}
