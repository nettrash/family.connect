package me.nettrash.familyconnect.ui.components

import androidx.compose.ui.text.input.KeyboardCapitalization
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * The prose fields' keyboard (#97): a keyboard capitalises only a field
 * that asks, so the one shared value must keep asking — a refactor that
 * dropped it would put every message back in lower case with nothing
 * failing anywhere else.
 */
class ProseKeyboardTest {

    @Test
    fun `prose fields ask for sentence capitals`() {
        assertEquals(KeyboardCapitalization.Sentences, ProseKeyboard.Sentences.capitalization)
    }

    @Test
    fun `a place asks for capital words`() {
        assertEquals(KeyboardCapitalization.Words, ProseKeyboard.Words.capitalization)
    }
}
