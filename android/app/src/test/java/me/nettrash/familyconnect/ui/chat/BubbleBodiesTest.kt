/*
 * BubbleBodiesTest.kt
 * Family Connect (Android)
 *
 * A bubble's body — markdown blocks and the spans inside them — is kept by
 * what it is a function of (BubbleBodies.kt), so a row scrolling back in,
 * or one the view model pre-rendered, finds it done. Kept answers must be
 * the answers: the same blocks a fresh render gives, and a fresh render when
 * anything they depend on (the mentions, the locale) is different.
 *
 * Robolectric, because the link detector is the platform's Linkify.
 */

package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.runBlocking
import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.data.db.MessageStatus
import me.nettrash.familyconnect.data.net.dto.MentionDto
import me.nettrash.familyconnect.data.net.dto.MentionsCodec
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import java.util.Locale

@RunWith(RobolectricTestRunner::class)
class BubbleBodiesTest {

    private lateinit var savedLocale: Locale

    @Before
    fun setUp() {
        savedLocale = Locale.getDefault()
        BubbleBodies.clear()
    }

    @After
    fun tearDown() {
        Locale.setDefault(savedLocale)
        BubbleBodies.clear()
    }

    @Test
    fun anEqualBodyIsHandedBackAsTheSameBlocks() {
        val body = "**see** https://example.com and call 555-123-4567"
        val first = BubbleBodies.blocks(body, null, emojiOnly = false)
        val second = BubbleBodies.blocks(String(body.toCharArray()), null, emojiOnly = false)
        assertThat(second).isSameInstanceAs(first)
        val links = first.single().links.map { it.url }
        assertThat(links).contains("https://example.com")
        assertThat(links.any { it.startsWith("tel:") }).isTrue()
    }

    @Test
    fun aKeptAnswerIsWhatAFreshRenderGives() {
        val body = "hi @Ann, the [notes](https://example.com/n) are up"
        val mentions = MentionsCodec.encode(listOf(MentionDto(userId = 9, name = "Ann")))
        val kept = BubbleBodies.blocks(body, mentions, emojiOnly = false)
        BubbleBodies.clear()
        val fresh = BubbleBodies.blocks(body, mentions, emojiOnly = false)
        assertThat(fresh).isNotSameInstanceAs(kept)
        assertThat(fresh).isEqualTo(kept)
    }

    @Test
    fun differentMentionsAreADifferentAnswer() {
        val body = "thanks @Ann"
        val named = BubbleBodies.blocks(body, MentionsCodec.encode(listOf(MentionDto(9, "Ann"))), emojiOnly = false)
        val unnamed = BubbleBodies.blocks(body, null, emojiOnly = false)
        assertThat(named.single().links).hasSize(1)
        assertThat(unnamed.single().links).isEmpty()
    }

    @Test
    fun aLocaleChangeIsNotAnsweredFromTheOldLocale() {
        val body = "call 555-123-4567"
        Locale.setDefault(Locale.US)
        val american = BubbleBodies.blocks(body, null, emojiOnly = false)
        Locale.setDefault(Locale.GERMANY)
        val german = BubbleBodies.blocks(body, null, emojiOnly = false)
        assertThat(german).isNotSameInstanceAs(american)
    }

    @Test
    fun anEmojiOnlyBodyIsPlainTextAndNeverKept() {
        val blocks = BubbleBodies.blocks("🎉🎉", null, emojiOnly = true)
        val text = blocks.single().block as MessageMarkdown.Block.Text
        assertThat(text.rendered.text).isEqualTo("🎉🎉")
        assertThat(blocks.single().links).isEmpty()
        assertThat(BubbleBodies.heldCount).isEqualTo(0)
    }

    @Test
    fun prewarmRendersTheWindowsBodiesAheadOfTheList() {
        val items = buildChatItems(
            messagesNewestFirst = listOf(
                MessageEntity("s2", 2, 42, 9, "see www.example.com", 2_000, MessageStatus.SENT),
                MessageEntity("s1", 1, 42, 9, "", 1_000, MessageStatus.SENT),
                MessageEntity("s0", 0, 42, 9, "🎉", 500, MessageStatus.SENT),
            ),
            isFamilyChat = false,
            myUserId = 7,
            memberNames = emptyMap(),
            nowMillis = 3_000,
        )
        runBlocking { BubbleBodies.prewarm(items) }
        // The text body only: an empty body has nothing to render, and an
        // emoji-only one is never kept.
        assertThat(BubbleBodies.heldCount).isEqualTo(1)
        val warmed = BubbleBodies.blocks("see www.example.com", null, emojiOnly = false)
        assertThat(warmed.single().links.single().url).isEqualTo("http://www.example.com")
    }
}
