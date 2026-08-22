/*
 * ReplyExcerptTest.kt
 * Family Connect (Android)
 *
 * The excerpt a client cuts while a send is pending must match the one the
 * server will send back, or the quote visibly changes when the ack lands.
 * Three platforms, three different native "take N characters" — this pins
 * ours to the server's rule (120 Unicode code points).
 *
 * iOS counterpart: ReplyToSnapshot.excerpt / MessageGroupingTests.
 */

package me.nettrash.familyconnect.data.net.dto

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class ReplyExcerptTest {

    @Test
    fun `a short body is quoted whole`() {
        assertThat(ReplyToDto.excerpt("See you at six")).isEqualTo("See you at six")
        assertThat(ReplyToDto.excerpt("")).isEqualTo("")
    }

    @Test
    fun `a long ascii body is cut to the documented length`() {
        val excerpt = ReplyToDto.excerpt("x".repeat(500))
        assertThat(excerpt.length).isEqualTo(120)
    }

    /**
     * The regression that matters: `String.take(120)` counts UTF-16 CODE
     * UNITS. An emoji is a surrogate PAIR, so `take` both keeps half as
     * much text as the server does and can slice a pair in half — leaving a
     * lone surrogate that renders as a replacement glyph inside a quote the
     * user never wrote.
     */
    @Test
    fun `astral characters are cut on code points, never mid-pair`() {
        val body = "\uD83D\uDE00".repeat(200) // 200 x U+1F600, each a surrogate pair
        val excerpt = ReplyToDto.excerpt(body)

        assertThat(excerpt.codePointCount(0, excerpt.length)).isEqualTo(120)
        // 120 code points = 240 UTF-16 units, i.e. no half pair survived.
        assertThat(excerpt.length).isEqualTo(240)
        assertThat(excerpt.none { it.isHighSurrogate() && excerpt.indexOf(it) == excerpt.lastIndex })
            .isTrue()
        // And what `take` would have produced is measurably different.
        assertThat(body.take(120).codePointCount(0, 120)).isEqualTo(60)
    }

    @Test
    fun `mixed width bodies keep exactly the server's count`() {
        val body = "\u00e9\u4e2d\uD83D\uDE00".repeat(100) // é 中 😀
        val excerpt = ReplyToDto.excerpt(body)
        assertThat(excerpt.codePointCount(0, excerpt.length)).isEqualTo(120)
        assertThat(body.startsWith(excerpt)).isTrue()
    }
}
