/*
 * FamilyLanguagesTest.kt
 * Family Connect (Android)
 *
 * The nine languages a family may declare, pinned against two things
 * that cannot be checked by reading the picker on one device: the tags
 * docs/protocol.md fixes (anything else is `invalid_language`, and a
 * misspelling here is a setting that silently refuses), and the SCRIPT
 * each name is written in.
 *
 * The script is the part that had drifted. Two of the nine name an
 * alphabet rather than a language, which is the whole reason the list has
 * nine entries and not eight — and a name written in the wrong one is
 * unreadable to precisely the reader it was split out for. iOS spells the
 * same list in FamilyLanguage.swift; these assertions are what keeps the
 * two from wording it differently again.
 */

package me.nettrash.familyconnect.ui.familyadmin

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class FamilyLanguagesTest {

    private fun hasCyrillic(text: String) = text.any { it in 'Ѐ'..'ӿ' }

    @Test
    fun `the nine tags are the protocol's, spelled and ordered as it spells them`() {
        assertEquals(
            listOf("en", "de", "es", "fr", "ja", "ru", "sr", "sr-Latn", "zh-Hans"),
            FAMILY_LANGUAGES.map { it.first },
        )
    }

    /**
     * The Latin-alphabet option is written in LATIN letters.
     *
     * Both Serbian rows in Cyrillic makes the two choices identical to a
     * Latin-script reader scanning for the one word they recognise —
     * which is the one thing this row exists to give them.
     */
    @Test
    fun `the Latin-alphabet Serbian option is written in Latin letters`() {
        val latin = FAMILY_LANGUAGES.single { it.first == "sr-Latn" }.second
        assertEquals("Srpski (latinica)", latin)
        assertFalse("Cyrillic in the Latin option: $latin", hasCyrillic(latin))

        // …and its Cyrillic sibling stays Cyrillic, so the pair really is
        // one choice per alphabet rather than two of the same.
        val cyrillic = FAMILY_LANGUAGES.single { it.first == "sr" }.second
        assertEquals("Српски", cyrillic)
        assertTrue("the Cyrillic option must be Cyrillic: $cyrillic", hasCyrillic(cyrillic))
    }

    /** Every name is in its own language — none of them is an English gloss. */
    @Test
    fun `each language is named in itself`() {
        val named = FAMILY_LANGUAGES.toMap()
        assertEquals("English", named["en"])
        assertEquals("Deutsch", named["de"])
        assertEquals("Español", named["es"])
        assertEquals("Français", named["fr"])
        assertEquals("日本語", named["ja"])
        assertEquals("Русский", named["ru"])
        assertEquals("简体中文", named["zh-Hans"])
    }
}
