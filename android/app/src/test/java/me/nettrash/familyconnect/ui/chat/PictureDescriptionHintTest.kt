package me.nettrash.familyconnect.ui.chat

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Pins when the "real names and brands are often refused" hint shows
 * (docs/protocol.md, "Pictures"): in the composer while a picture request
 * is being typed where this client offers `/draw`, and on the board beside
 * "Draw a backdrop" exactly when that control is there.
 *
 * Its own file rather than rows in AssistantMentionTest: that file's tables
 * are read by the server's `the_three_ports_carry_the_same_vectors` and
 * must match the other two ports character by character, and this rule is
 * this client's courtesy, not the wire contract.
 */
class PictureDescriptionHintTest {

    private fun composer(
        draft: String,
        picturesOffered: Boolean = true,
        inFamilyChat: Boolean = false,
        editing: Boolean = false,
    ) = PictureDescriptionHint.inComposer(
        draft = draft,
        picturesOffered = picturesOffered,
        inFamilyChat = inFamilyChat,
        editing = editing,
    )

    @Test
    fun `shows while a picture request is typed in the assistant chat`() {
        listOf(
            // What the "ask for a picture" button leaves behind — nothing
            // described yet, and exactly when the hint is most useful.
            "/draw ",
            "/draw a cat",
            "/draw a cat in a hat",
            // Leading whitespace is trimmed, the way the grammar reads it.
            "   /draw a cat",
            "\n/draw a cat",
            "/draw\na cat",
            // ASCII case folding, as the server compares.
            "/DRAW a cat",
            "/Draw ",
            // The one leading mention the grammar skips, in any chat.
            "@ai /draw a cat",
            "/draw кот",
        ).forEach { assertTrue(it, composer(it)) }
    }

    @Test
    fun `does not show when the draft is not a picture request`() {
        listOf(
            "",
            "   ",
            // No whitespace after the token yet: still being typed, and a
            // word like `/drawer` is not a request at all.
            "/draw",
            "/dra",
            "/drawer",
            "/draws a cat",
            "/draw,a cat",
            // Not first.
            "what does /draw do?",
            "please /draw a cat",
            "hey @ai /draw a cat",
            "@ai @ai /draw a cat",
            // Unicode White_Space and NOT Kotlin's: U+001C is not
            // whitespace to the server, so this is one long word.
            "/draw\u001Ca cat",
            "a cat",
        ).forEach { assertFalse(it, composer(it)) }
    }

    @Test
    fun `U+0085 after the token is whitespace, as the server reads it`() {
        assertTrue(composer("/draw\u0085a cat"))
    }

    @Test
    fun `never where this client does not offer drawing`() {
        // No images deployment, or a chat the assistant is not in: `/draw`
        // is just text there.
        assertFalse(composer("/draw a cat", picturesOffered = false))
        assertFalse(composer("/draw ", picturesOffered = false))
        assertFalse(composer("@ai /draw a cat", picturesOffered = false, inFamilyChat = true))
    }

    @Test
    fun `never while the composer is borrowed for an edit`() {
        assertFalse(composer("/draw a cat", editing = true))
        assertFalse(composer("@ai /draw a cat", inFamilyChat = true, editing = true))
    }

    @Test
    fun `the family chat needs the assistant mentioned`() {
        assertTrue(composer("@ai /draw ", inFamilyChat = true))
        assertTrue(composer("@ai /draw a cat", inFamilyChat = true))
        assertTrue(composer("  @AI   /draw a cat", inFamilyChat = true))
        // The server routes a family message on ANY mention and then reads
        // the token by the grammar alone, so this one IS drawn.
        assertTrue(composer("/draw a cat @ai", inFamilyChat = true))
        // No mention: an ordinary family message, drawn by nobody.
        assertFalse(composer("/draw a cat", inFamilyChat = true))
        assertFalse(composer("/draw ", inFamilyChat = true))
        assertFalse(composer("hey @ai /draw a cat", inFamilyChat = true))
    }

    @Test
    fun `the hint and the button's rewrite agree`() {
        // Whatever the "ask for a picture" button leaves behind shows the
        // hint at once — in both chats. (Not from `/draw a cat` typed by
        // hand in the family chat: `withDraw` leaves a body the grammar
        // already reads as a request untouched, without the mention the
        // family chat needs.)
        listOf("", "a cat", "  a cat", "@ai a cat", "@ai /draw a cat").forEach { typed ->
            assertTrue(typed, composer(AssistantMention.withDraw(typed, inFamilyChat = false)))
            assertTrue(
                typed,
                composer(AssistantMention.withDraw(typed, inFamilyChat = true), inFamilyChat = true),
            )
        }
    }

    @Test
    fun `isStartingPicture only drops the non-empty-prompt clause`() {
        // Everywhere drawPrompt finds a request, the hint's reading agrees.
        listOf("/draw a cat", "@ai /draw a cat", "  /draw\na cat", "/DRAW x").forEach {
            assertTrue(it, AssistantMention.drawPrompt(it) != null)
            assertTrue(it, AssistantMention.isStartingPicture(it))
        }
        // And the one difference: a token with nothing described yet.
        listOf("/draw ", "@ai /draw   ", "/draw\n").forEach {
            assertEquals(it, null, AssistantMention.drawPrompt(it))
            assertTrue(it, AssistantMention.isStartingPicture(it))
        }
    }

    @Test
    fun `the backdrop hint goes exactly where the backdrop control does`() {
        assertTrue(PictureDescriptionHint.onBackdrop(canEdit = true, canDraw = true))
        assertFalse(PictureDescriptionHint.onBackdrop(canEdit = false, canDraw = true))
        assertFalse(PictureDescriptionHint.onBackdrop(canEdit = true, canDraw = false))
        assertFalse(PictureDescriptionHint.onBackdrop(canEdit = false, canDraw = false))
    }
}
