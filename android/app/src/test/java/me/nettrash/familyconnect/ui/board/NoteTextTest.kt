/*
 * NoteTextTest.kt
 * Family Connect (Android) — tests
 *
 * The board's 280-character cap, enforced where the author is typing
 * (docs/protocol.md, "Board").
 *
 * The number is counted the way the SERVER counts it — Rust's
 * `chars().count()`, which is Unicode scalars — and not the way Kotlin
 * counts a String, which is UTF-16 units and therefore two for every emoji.
 * A family writing in emoji would otherwise lose half its allowance, and a
 * note that looked under the cap here could still come back `validation`.
 */

package me.nettrash.familyconnect.ui.board

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class NoteTextTest {

    @Test
    fun `a note under the cap is left exactly as typed`() {
        val note = "Dinner at 7?"
        assertThat(NoteText.capped(note)).isEqualTo(note)
        assertThat(NoteText.remaining(note)).isEqualTo(280 - note.length)
        assertThat(NoteText.shouldShowCounter(note)).isFalse()
    }

    @Test
    fun `the cap counts code points, not UTF-16 units`() {
        // Every one of these is a single character to the server and two
        // to String.length. 280 of them are a full note, not half of one.
        val emoji = "👩" // U+1F469
        val full = emoji.repeat(280)
        assertThat(full.length).isEqualTo(560)
        assertThat(NoteText.capped(full)).isEqualTo(full)
        assertThat(NoteText.remaining(full)).isEqualTo(0)

        // And the cut never splits a surrogate pair, which would leave an
        // unpaired half the server would store as a replacement character.
        val over = emoji.repeat(281)
        val capped = NoteText.capped(over)
        assertThat(capped.codePointCount(0, capped.length)).isEqualTo(280)
        assertThat(capped).isEqualTo(full)
    }

    @Test
    fun `a note over the cap is cut to exactly the cap`() {
        val over = "x".repeat(300)
        assertThat(NoteText.capped(over)).hasLength(280)
        assertThat(NoteText.remaining(NoteText.capped(over))).isEqualTo(0)
    }

    @Test
    fun `the counter appears only near the end`() {
        assertThat(NoteText.shouldShowCounter("x".repeat(239))).isFalse()
        assertThat(NoteText.shouldShowCounter("x".repeat(240))).isTrue()
        assertThat(NoteText.shouldShowCounter("x".repeat(280))).isTrue()
    }
}
