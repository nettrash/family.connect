/*
 * NoteFontsTest.kt
 * Family Connect (Android) — tests
 *
 * The four hands a note can be written in (docs/protocol.md, "Board").
 *
 * A font is an INTENT resolved to a SYSTEM face: the wire never carries a
 * family name, and this platform bundles no fonts. The vocabulary and the
 * fallback are pinned here against the iOS counterpart, NoteFontTests.
 */

package me.nettrash.familyconnect.ui.board

import androidx.compose.ui.text.font.FontFamily
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class NoteFontsTest {

    @Test
    fun `the vocabulary is the protocol's, in picker order`() {
        assertThat(NoteFonts.hands)
            .containsExactly("plain", "serif", "mono", "casual").inOrder()
        assertThat(NoteFonts.PLAIN).isEqualTo("plain")
    }

    /**
     * A name from a NEWER server must draw as something rather than fail,
     * and plain is what every note was written in before fonts existed —
     * the same forgiveness `color` and `size` get.
     */
    @Test
    fun `an unknown hand falls back to plain`() {
        assertThat(NoteFonts.resolve("comic-sans")).isEqualTo("plain")
        assertThat(NoteFonts.resolve("")).isEqualTo("plain")
        assertThat(NoteFonts.family("comic-sans")).isEqualTo(FontFamily.Default)
        assertThat(NoteFonts.label("comic-sans")).isEqualTo(NoteFonts.label("plain"))
        for (hand in NoteFonts.hands) {
            assertThat(NoteFonts.resolve(hand)).isEqualTo(hand)
        }
    }

    /**
     * Every hand resolves to a family the platform already has. Nothing is
     * bundled and nothing is downloaded, which is the theme's written rule
     * as well as the protocol's.
     */
    @Test
    fun `each hand is a generic system family, and they differ`() {
        assertThat(NoteFonts.family("plain")).isEqualTo(FontFamily.Default)
        assertThat(NoteFonts.family("serif")).isEqualTo(FontFamily.Serif)
        assertThat(NoteFonts.family("mono")).isEqualTo(FontFamily.Monospace)
        assertThat(NoteFonts.family("casual")).isEqualTo(FontFamily.Cursive)
        assertThat(NoteFonts.hands.map { NoteFonts.family(it) }.toSet()).hasSize(4)
    }

    @Test
    fun `every hand has its own label`() {
        assertThat(NoteFonts.hands.map { NoteFonts.label(it) }.toSet()).hasSize(4)
    }
}
