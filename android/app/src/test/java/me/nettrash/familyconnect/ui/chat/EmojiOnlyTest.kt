/*
 * EmojiOnlyTest.kt
 * Family Connect (Android)
 *
 * Pins the emoji-only detector behind the bubble's font ladder. The
 * scanner is a cross-platform contract (iOS embeds the identical table
 * and grammar, with these same vectors mirrored in EmojiOnlyTests.swift),
 * so the vectors here ARE the spec: sizes 96/80/68/56 for one through
 * four emoji, normal text from five up or for any mixed content,
 * multi-code-point sequences (variation selectors, skin tones, ZWJ
 * families, flags, keycaps) counting as ONE emoji each — plus a sweep
 * of the full picker catalog, which must classify as one emoji per
 * entry without exception.
 */

package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertThat
import com.google.common.truth.Truth.assertWithMessage
import org.junit.Test

class EmojiOnlyTest {

    @Test
    fun fontLadderOneEmojiBiggestFiveBackToText() {
        assertThat(EmojiOnly.displayFontSize("😀")).isEqualTo(96f)
        assertThat(EmojiOnly.displayFontSize("😀😂")).isEqualTo(80f)
        assertThat(EmojiOnly.displayFontSize("😀😂🥳")).isEqualTo(68f)
        assertThat(EmojiOnly.displayFontSize("😀😂🥳😎")).isEqualTo(56f)
        assertThat(EmojiOnly.displayFontSize("😀😂🥳😎😍")).isNull()
        // Five IS still emoji-only — it just renders at text size.
        assertThat(EmojiOnly.count("😀😂🥳😎😍")).isEqualTo(5)
    }

    @Test
    fun textMixedContentAndEmptyInputAreNotEmojiOnly() {
        assertThat(EmojiOnly.count("hi")).isNull()
        assertThat(EmojiOnly.count("hi 😀")).isNull()
        assertThat(EmojiOnly.count("😀 hi")).isNull()
        assertThat(EmojiOnly.count("😀!")).isNull()
        assertThat(EmojiOnly.count("a😀")).isNull()
        assertThat(EmojiOnly.count("")).isNull()
        assertThat(EmojiOnly.count("  \n ")).isNull()
        // Bare digits, # and * are text; only the keycap marks make
        // them emoji.
        assertThat(EmojiOnly.count("1")).isNull()
        assertThat(EmojiOnly.count("123")).isNull()
        assertThat(EmojiOnly.count("#")).isNull()
    }

    @Test
    fun whitespaceBetweenEmojiIsAllowedAndNotCounted() {
        assertThat(EmojiOnly.count("😀 😀")).isEqualTo(2)
        assertThat(EmojiOnly.count("😀\n😀")).isEqualTo(2)
        assertThat(EmojiOnly.count(" 😀 ")).isEqualTo(1)
        // The scanner's own White_Space table, not the platform's
        // predicate — these two code points are where java.lang and
        // Swift disagree, so they pin cross-platform agreement: NEL is
        // whitespace, the C0 separators are not.
        assertThat(EmojiOnly.count("😀\u0085😀")).isEqualTo(2)
        assertThat(EmojiOnly.count("😀\u001C😀")).isNull()
    }

    @Test
    fun multiCodePointSequencesCountAsOneEmoji() {
        assertThat(EmojiOnly.count("❤️")).isEqualTo(1) // VS16
        assertThat(EmojiOnly.count("☀️")).isEqualTo(1) // BMP + VS16
        assertThat(EmojiOnly.count("☀")).isEqualTo(1) // bare BMP pictograph, by design
        assertThat(EmojiOnly.count("👍🏽")).isEqualTo(1) // skin tone
        assertThat(EmojiOnly.count("👨‍👩‍👧‍👦")).isEqualTo(1) // ZWJ family
        assertThat(EmojiOnly.count("👮‍♀️")).isEqualTo(1) // ZWJ + BMP + VS16
        assertThat(EmojiOnly.count("🏳️‍🌈")).isEqualTo(1) // flag + ZWJ
        assertThat(EmojiOnly.count("🇩🇪")).isEqualTo(1) // regional pair
        assertThat(EmojiOnly.count("🏴󠁧󠁢󠁳󠁣󠁴󠁿")).isEqualTo(1) // tag sequence
        assertThat(EmojiOnly.count("1️⃣")).isEqualTo(1) // keycap
        assertThat(EmojiOnly.count("❤️❤️")).isEqualTo(2)
        assertThat(EmojiOnly.count("🇩🇪🇫🇷")).isEqualTo(2)
        assertThat(EmojiOnly.count("👍🏽👍🏿")).isEqualTo(2)
    }

    @Test
    fun explicitTextSelectorOptsTheMessageOut() {
        // U+2602 U+FE0E — the author asked for the text glyph.
        assertThat(EmojiOnly.count("☂︎")).isNull()
    }

    @Test
    fun everyCatalogEntryCountsAsExactlyOneEmoji() {
        for (category in EMOJI_CATALOG) {
            for (emoji in category.emoji) {
                assertWithMessage("${category.name}: $emoji")
                    .that(EmojiOnly.count(emoji))
                    .isEqualTo(1)
            }
        }
    }

    @Test
    fun quickReactionsCountAsExactlyOneEmoji() {
        for (emoji in QUICK_REACTIONS) {
            assertWithMessage(emoji).that(EmojiOnly.count(emoji)).isEqualTo(1)
        }
    }
}
