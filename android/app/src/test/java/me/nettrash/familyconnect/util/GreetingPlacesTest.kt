/*
 * GreetingPlacesTest.kt
 * Family Connect (Android)
 *
 * The owner's greeting-weather places as rules (docs/protocol.md, "Today's
 * weather, for places the owner chose"): when the field is drawn, how a
 * name is folded and limited as it is typed, and what a Save sends. The
 * vectors in "prepare" follow `validate_greeting_places`'s own tests in
 * server/src/handlers_family.rs, so a Save this client offers is one the
 * server keeps unchanged.
 */

package me.nettrash.familyconnect.util

import com.google.common.truth.Truth.assertThat
import com.google.common.truth.Truth.assertWithMessage
import me.nettrash.familyconnect.util.GreetingPlaces.Prepared
import me.nettrash.familyconnect.util.GreetingPlaces.Problem
import org.junit.Test

class GreetingPlacesTest {

    // -- Visibility ------------------------------------------------------------------

    @Test
    fun `the field is drawn only for the owner on a server with greeting weather`() {
        assertThat(GreetingPlaces.isShown(isOwner = true, greetingWeather = true)).isTrue()
        assertThat(GreetingPlaces.isShown(isOwner = true, greetingWeather = false)).isFalse()
        assertThat(GreetingPlaces.isShown(isOwner = false, greetingWeather = true)).isFalse()
        assertThat(GreetingPlaces.isShown(isOwner = false, greetingWeather = false)).isFalse()
    }

    @Test
    fun `another field may be added only below three`() {
        assertThat(GreetingPlaces.canAdd(0)).isTrue()
        assertThat(GreetingPlaces.canAdd(2)).isTrue()
        assertThat(GreetingPlaces.canAdd(3)).isFalse()
        assertThat(GreetingPlaces.canAdd(4)).isFalse()
    }

    @Test
    fun `the editor opens on the stored places, or one empty field`() {
        assertThat(GreetingPlaces.fieldsFor(emptyList())).containsExactly("")
        assertThat(GreetingPlaces.fieldsFor(listOf("Moscow", "Belgrade"))).containsExactly("Moscow", "Belgrade").inOrder()
        // Never more fields than the limit, whatever arrives.
        assertThat(GreetingPlaces.fieldsFor(listOf("a", "b", "c", "d"))).hasSize(3)
    }

    // -- Folding ---------------------------------------------------------------------

    @Test
    fun `a name is trimmed and its whitespace runs folded`() {
        assertThat(GreetingPlaces.normalize("  Moscow ")).isEqualTo("Moscow")
        assertThat(GreetingPlaces.normalize("Novi\t\n  Sad")).isEqualTo("Novi Sad")
        assertThat(GreetingPlaces.normalize("Rio de　Janeiro")).isEqualTo("Rio de Janeiro")
        assertThat(GreetingPlaces.normalize(" \t ")).isEmpty()
    }

    @Test
    fun `whitespace is Unicode White_Space, as Rust's, not Kotlin's isWhitespace`() {
        // U+0085 NEXT LINE is White_Space (folded by the server); Kotlin's
        // isWhitespace says no.
        assertThat('\u0085'.isWhitespace()).isFalse()
        assertThat(GreetingPlaces.normalize("Novi\u0085Sad")).isEqualTo("Novi Sad")
        // U+001F UNIT SEPARATOR is NOT White_Space — the server keeps it and
        // then refuses it as a control character; Kotlin's isWhitespace
        // would have folded it away.
        assertThat('\u001F'.isWhitespace()).isTrue()
        assertThat(GreetingPlaces.normalize("Novi\u001FSad")).isEqualTo("Novi\u001FSad")
        assertThat(GreetingPlaces.prepare(listOf("Novi\u001FSad")))
            .isEqualTo(Prepared.Refused(Problem.CONTROL_CHARACTER, 1))
        // The whole White_Space set, every member folded.
        val whiteSpace = (0x09..0x0D) + listOf(0x20, 0x85, 0xA0, 0x1680) + (0x2000..0x200A) +
            listOf(0x2028, 0x2029, 0x202F, 0x205F, 0x3000)
        for (cp in whiteSpace) {
            val raw = "a" + String(Character.toChars(cp)) + "b"
            assertWithMessage("U+%s", Integer.toHexString(cp)).that(GreetingPlaces.normalize(raw)).isEqualTo("a b")
        }
        // A zero-width space is not White_Space.
        assertThat(GreetingPlaces.isWhiteSpace(0x200B)).isFalse()
    }

    // -- Typing --------------------------------------------------------------------------

    @Test
    fun `typing stops at eighty characters of the folded name`() {
        val eighty = "x".repeat(80)
        assertThat(GreetingPlaces.limitInput(eighty)).isEqualTo(eighty)
        assertThat(GreetingPlaces.limitInput(eighty + "y")).isEqualTo(eighty)
        // Spaces the server folds away cost nothing…
        assertThat(GreetingPlaces.limitInput("   $eighty   ")).isEqualTo("   $eighty   ")
        // …but one between words costs one.
        val words = "x".repeat(40) + "  " + "y".repeat(40)
        assertThat(GreetingPlaces.charCount(GreetingPlaces.normalize(GreetingPlaces.limitInput(words)))).isEqualTo(80)
        assertThat(GreetingPlaces.limitInput(words)).isEqualTo("x".repeat(40) + "  " + "y".repeat(39))
    }

    @Test
    fun `characters are code points, as the server counts them`() {
        val emoji = "🌧" // 🌧, two UTF-16 units
        val eighty = emoji.repeat(80)
        assertThat(eighty.length).isEqualTo(160)
        assertThat(GreetingPlaces.limitInput(eighty + emoji)).isEqualTo(eighty)
        assertThat(GreetingPlaces.prepare(listOf(eighty))).isEqualTo(Prepared.Ok(listOf(eighty)))
        // Cyrillic, as in the server's own vector.
        val cyrillic = "ж".repeat(80)
        assertThat(GreetingPlaces.prepare(listOf(cyrillic))).isEqualTo(Prepared.Ok(listOf(cyrillic)))
        assertThat(GreetingPlaces.prepare(listOf(cyrillic + "ж"))).isEqualTo(Prepared.Refused(Problem.TOO_LONG, 1))
    }

    @Test
    fun `a control character typed or pasted is dropped, whitespace kept for folding`() {
        assertThat(GreetingPlaces.limitInput("Bel\u0000grade")).isEqualTo("Belgrade")
        assertThat(GreetingPlaces.limitInput("Bel\u007Fgr\u009Fade")).isEqualTo("Belgrade")
        assertThat(GreetingPlaces.limitInput("Novi\tSad")).isEqualTo("Novi\tSad")
    }

    // -- Saving -------------------------------------------------------------------------

    @Test
    fun `names are kept as the server keeps them`() {
        assertThat(GreetingPlaces.prepare(listOf("  Moscow ", "Novi\t\n  Sad", "MOSCOW", "Belgrade")))
            .isEqualTo(Prepared.Ok(listOf("Moscow", "Novi Sad", "Belgrade")))
        // Unicode lower-casing, not ASCII.
        assertThat(GreetingPlaces.prepare(listOf("Москва", "МОСКВА")))
            .isEqualTo(Prepared.Ok(listOf("Москва")))
        assertThat(GreetingPlaces.prepare(emptyList())).isEqualTo(Prepared.Ok(emptyList()))
    }

    @Test
    fun `a blank field is not a place, so it is not sent`() {
        assertThat(GreetingPlaces.prepare(listOf("Moscow", "   ", "")))
            .isEqualTo(Prepared.Ok(listOf("Moscow")))
        // Clearing every field clears the list.
        assertThat(GreetingPlaces.prepare(listOf(""))).isEqualTo(Prepared.Ok(emptyList()))
    }

    @Test
    fun `what the server refuses is refused here first, by position`() {
        assertThat(GreetingPlaces.prepare(listOf("Moscow", "Belgrade", "Paris", "Tokyo")))
            .isEqualTo(Prepared.Refused(Problem.TOO_MANY, 0))
        assertThat(GreetingPlaces.prepare(listOf("Moscow", "x".repeat(81))))
            .isEqualTo(Prepared.Refused(Problem.TOO_LONG, 2))
        assertThat(GreetingPlaces.prepare(listOf("Bel\u0000grade")))
            .isEqualTo(Prepared.Refused(Problem.CONTROL_CHARACTER, 1))
        // Four names of which two are one: three after repeats, so kept.
        assertThat(GreetingPlaces.prepare(listOf("Moscow", "moscow", "Paris", "Tokyo")))
            .isEqualTo(Prepared.Ok(listOf("Moscow", "Paris", "Tokyo")))
    }

    @Test
    fun `a change is anything that would send a different list`() {
        val stored = listOf("Moscow", "Belgrade")
        assertThat(GreetingPlaces.isChanged(listOf("Moscow", "Belgrade"), stored)).isFalse()
        // Whitespace the server folds is no change.
        assertThat(GreetingPlaces.isChanged(listOf(" Moscow", "Belgrade  ", ""), stored)).isFalse()
        assertThat(GreetingPlaces.isChanged(listOf("Moscow"), stored)).isTrue()
        assertThat(GreetingPlaces.isChanged(listOf("Belgrade", "Moscow"), stored)).isTrue()
        // An empty editor over an empty list is no change; one over a list is.
        assertThat(GreetingPlaces.isChanged(listOf(""), emptyList())).isFalse()
        assertThat(GreetingPlaces.isChanged(listOf(""), stored)).isTrue()
        // Fields that cannot be saved still count as pending.
        assertThat(GreetingPlaces.isChanged(listOf("x".repeat(81)), emptyList())).isTrue()
    }
}
