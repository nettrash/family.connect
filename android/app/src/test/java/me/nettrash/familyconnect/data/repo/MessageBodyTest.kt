/*
 * MessageBodyTest.kt
 * Family Connect (Android)
 *
 * How long a body may be, measured the way the SERVER measures it.
 *
 * The rules worth a test each are the ways a client gets this subtly
 * wrong and only finds out at Send — or worse, does not:
 *
 *  - CHARACTERS, not UTF-16 units. Kotlin's `length` counts an emoji as
 *    two, Rust's `chars().count()` counts it as one — so counting the
 *    Kotlin way would refuse messages the server accepts.
 *  - TRIMMED, because `validate_body` trims before it counts.
 *  - CUT at grapheme boundaries, or an emoji leaves its debris behind in
 *    somebody's message.
 *
 * Pure: no Android, no Robolectric.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class MessageBodyTest {

    /** docs/protocol.md, "Limits". */
    @Test
    fun `the limit is the protocol's`() {
        assertThat(MessageBody.MAX_CHARS).isEqualTo(4000)
    }

    // -- Counting -------------------------------------------------------------

    @Test
    fun `an ordinary message counts its characters`() {
        assertThat(MessageBody.length("dinner at 7")).isEqualTo(11)
        assertThat(MessageBody.length("")).isEqualTo(0)
    }

    /**
     * The one that matters: an emoji outside the BMP is ONE character to
     * the server and two to Kotlin. Counting Kotlin's way would refuse a
     * message of 2001 emoji that the server would have taken.
     */
    @Test
    fun `an emoji is one character, not two`() {
        val family = "👨‍👩‍👧" // man+ZWJ+woman+ZWJ+girl
        assertThat(family.length).isEqualTo(8)
        assertThat(MessageBody.length(family)).isEqualTo(5)

        val limit = 10
        // Ten emoji: twenty UTF-16 units, ten characters. It fits.
        assertThat(MessageBody.fits("🎉".repeat(10), limit)).isTrue()
        assertThat(MessageBody.fits("🎉".repeat(11), limit)).isFalse()
    }

    /** `validate_body` trims first, so trailing spaces are not what tip it over. */
    @Test
    fun `whitespace at the ends is not counted`() {
        assertThat(MessageBody.length("   hello   ")).isEqualTo(5)
        assertThat(MessageBody.length("   ")).isEqualTo(0)
        assertThat(MessageBody.length("\n\nhi\n\n")).isEqualTo(2)
        assertThat(MessageBody.fits("x".repeat(4000) + "   ")).isTrue()
    }

    @Test
    fun `exactly the limit fits and one more does not`() {
        assertThat(MessageBody.fits("x".repeat(MessageBody.MAX_CHARS))).isTrue()
        assertThat(MessageBody.fits("x".repeat(MessageBody.MAX_CHARS + 1))).isFalse()
    }

    // -- Appending ------------------------------------------------------------

    @Test
    fun `a separator appears only where one is needed`() {
        assertThat(MessageBody.separator("")).isEmpty()
        assertThat(MessageBody.separator("see you at")).isEqualTo(" ")
        assertThat(MessageBody.separator("see you at ")).isEmpty()
        assertThat(MessageBody.separator("see you at\n")).isEmpty()
    }

    @Test
    fun `appending joins the draft and the words`() {
        assertThat(MessageBody.appending("7", "see you at"))
            .isEqualTo(MessageBody.Paste.Appended("see you at 7"))
        assertThat(MessageBody.appending("dinner", ""))
            .isEqualTo(MessageBody.Paste.Appended("dinner"))
        assertThat(MessageBody.appending("at 7", "dinner "))
            .isEqualTo(MessageBody.Paste.Appended("dinner at 7"))
    }

    /**
     * What fits is kept and the sentence says the rest was left out —
     * the same choice the Apple clients make, so a family that uses both
     * sees one behaviour.
     */
    @Test
    fun `appending keeps what fits and drops the tail`() {
        val draft = "x".repeat(3990)
        assertThat(MessageBody.appending("y".repeat(9), draft))
            .isInstanceOf(MessageBody.Paste.Appended::class.java)

        val outcome = MessageBody.appending("y".repeat(20), draft)
        assertThat(outcome).isInstanceOf(MessageBody.Paste.Truncated::class.java)
        val kept = (outcome as MessageBody.Paste.Truncated).draft
        assertThat(MessageBody.length(kept)).isEqualTo(MessageBody.MAX_CHARS)
        // 3990 x's, the separator, then as many y's as there was room for.
        assertThat(kept).startsWith(draft + " ")
        assertThat(kept.substringAfter(draft + " ")).isEqualTo("y".repeat(9))
    }

    /**
     * Nothing of the addition survived: a different sentence, because
     * "the rest wasn't pasted" is wrong when none of it was — and the
     * draft is left exactly as it was.
     */
    @Test
    fun `a draft already at the ceiling takes nothing`() {
        val full = "x".repeat(MessageBody.MAX_CHARS)
        assertThat(MessageBody.appending("more", full)).isEqualTo(MessageBody.Paste.Full)
    }

    // -- Cutting --------------------------------------------------------------

    @Test
    fun `text inside the limit is not cut at all`() {
        assertThat(MessageBody.cutToFit("hello", limit = 10)).isNull()
        assertThat(MessageBody.cutIndex("x".repeat(MessageBody.MAX_CHARS))).isNull()
    }

    @Test
    fun `text over the limit is cut down to it`() {
        val cut = MessageBody.cutToFit("x".repeat(4500))
        assertThat(cut).isNotNull()
        assertThat(MessageBody.length(cut!!)).isEqualTo(MessageBody.MAX_CHARS)
    }

    /**
     * The cut never lands inside an emoji: half a surrogate pair is not a
     * character, it is a replacement glyph in somebody's message.
     */
    @Test
    fun `a cut never splits an emoji`() {
        // Five parties: ten UTF-16 units, five characters.
        val cut = MessageBody.cutToFit("🎉".repeat(5), limit = 3)
        assertThat(cut).isEqualTo("🎉".repeat(3))
        // And a cut that falls where an emoji begins keeps whole ones only.
        assertThat(MessageBody.cutToFit("ab🎉cd", limit = 3)).isEqualTo("ab🎉")
    }

    // -- The composer's own buffer --------------------------------------------
    //
    // The door that cannot be capped by checking a clipboard: the text
    // field's paste hands the words to the FIELD, which inserts them at
    // the caret without this app ever seeing them. What it can be capped
    // by is a review of every change the field makes to its buffer — see
    // ui/chat/BodyLengthLimit.kt.

    @Test
    fun `a change that fits is accepted`() {
        assertThat(MessageBody.review(proposed = "hello", original = "hell"))
            .isEqualTo(MessageBody.Edit.ACCEPT)
    }

    /**
     * A wall of text arriving all at once loses a visible amount of
     * itself, so the loss is explained.
     */
    @Test
    fun `a paste that overflows is cut and said out loud`() {
        assertThat(
            MessageBody.review(proposed = "x".repeat(MessageBody.MAX_CHARS + 500), original = ""),
        ).isEqualTo(MessageBody.Edit.CUT_AND_SAY)
    }

    /**
     * Typing into a full field stops the way every length-capped field on
     * this platform stops: the field does not grow, in silence. An error
     * strip on every keystroke would be worse than the cap.
     */
    @Test
    fun `a keystroke into a full field is cut back in silence`() {
        val full = "x".repeat(MessageBody.MAX_CHARS)
        assertThat(MessageBody.review(proposed = full + "y", original = full))
            .isEqualTo(MessageBody.Edit.CUT_QUIETLY)
    }

    /** The separator counts too — it is part of what gets sent. */
    @Test
    fun `the separator is inside the limit`() {
        // 3999 + separator + 1 = 4001, so the "y" has nowhere to go.
        assertThat(MessageBody.appending("y", "x".repeat(3999)))
            .isEqualTo(MessageBody.Paste.Full)
        // No separator wanted after whitespace, so the same word fits.
        assertThat(MessageBody.appending("y", "x".repeat(3998) + " "))
            .isInstanceOf(MessageBody.Paste.Appended::class.java)
    }
}
