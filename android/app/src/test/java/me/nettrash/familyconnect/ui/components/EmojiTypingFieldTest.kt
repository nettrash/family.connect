/*
 * EmojiTypingFieldTest.kt
 * Family Connect (Android)
 *
 * Emoji as you type (#101) carried to Compose's fields: a TextFieldValue
 * field (thread reply, board note), the state-based composer's
 * InputTransformation (chained before the length limit), and a String-backed
 * field through proseValue (event title, list line, poll, report note).
 * The rule itself is EmojiTypingTest's; this pins only that each kind of
 * field hands it the right change and puts the caret where it says.
 *
 * Text goes in through the test framework's text input (the IME path: an
 * insertion at the caret) and Backspace through a hardware key event — a
 * real phone keyboard (Gboard, Samsung) is not reproduced here.
 */

package me.nettrash.familyconnect.ui.components

import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.foundation.text.input.insert
import androidx.compose.foundation.text.input.placeCursorAtEnd
import androidx.compose.foundation.text.input.then
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.ui.chat.BodyLengthLimit
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@OptIn(ExperimentalTestApi::class)
@RunWith(RobolectricTestRunner::class)
class EmojiTypingFieldTest {

    @get:Rule
    val compose = createComposeRule()

    private val smile = "😊"
    private val fire = "🔥"

    @Test
    fun `a TextFieldValue field gets the emoji and the caret behind it`() {
        val emoji = EmojiTypingField()
        val old = TextFieldValue("ok : bye", TextRange(4))
        val out = emoji.onValueChange(old, TextFieldValue("ok :) bye", TextRange(5)))
        assertThat(out.text).isEqualTo("ok $smile bye")
        assertThat(out.selection).isEqualTo(TextRange(5))
        // Backspace: Compose removes the whole emoji.
        val back = emoji.onValueChange(out, TextFieldValue("ok  bye", TextRange(3)))
        assertThat(back.text).isEqualTo("ok :) bye")
        assertThat(back.selection).isEqualTo(TextRange(5))
        // A composing change is passed through untouched.
        val composing = TextFieldValue(":)", TextRange(2), composition = TextRange(0, 2))
        assertThat(EmojiTypingField().onValueChange(TextFieldValue(":", TextRange(1)), composing))
            .isSameInstanceAs(composing)
    }

    @Test
    fun `the composer's transformation replaces and takes back`() {
        val state = TextFieldState()
        compose.setContent {
            val input = remember { EmojiTypingInput().then(BodyLengthLimit {}) }
            BasicTextField(state = state, inputTransformation = input, modifier = Modifier.testTag("field"))
        }
        val field = compose.onNodeWithTag("field")
        field.performClick()
        field.performTextInput("so :fire")
        field.performTextInput(":")
        compose.runOnIdle {
            assertThat(state.text.toString()).isEqualTo("so $fire")
            assertThat(state.selection).isEqualTo(TextRange(5))
        }
        field.performKeyInput { pressKey(Key.Backspace) }
        compose.runOnIdle {
            assertThat(state.text.toString()).isEqualTo("so :fire:")
            assertThat(state.selection).isEqualTo(TextRange(9))
        }
        // The second Backspace is an ordinary one.
        field.performKeyInput { pressKey(Key.Backspace) }
        compose.runOnIdle { assertThat(state.text.toString()).isEqualTo("so :fire") }
    }

    @Test
    fun `the composer's app edits drop the swap`() {
        val state = TextFieldState()
        compose.setContent {
            val input = remember { EmojiTypingInput() }
            BasicTextField(state = state, inputTransformation = input, modifier = Modifier.testTag("field"))
        }
        val field = compose.onNodeWithTag("field")
        field.performClick()
        field.performTextInput(":")
        field.performTextInput(")")
        compose.runOnIdle {
            assertThat(state.text.toString()).isEqualTo(smile)
            // The app changes the field (a mention picked): never run
            // through the transformation, and not the swap's field any more.
            state.edit {
                insert(0, "@Ann ")
                placeCursorAtEnd()
            }
        }
        field.performKeyInput { pressKey(Key.Backspace) }
        compose.runOnIdle { assertThat(state.text.toString()).isEqualTo("@Ann ") }
    }

    @Test
    fun `a String field keeps its model a String and gets the caret right`() {
        var model by mutableStateOf("")
        compose.setContent {
            val (value, onValue) = proseValue(model) { model = it.take(LIMIT) }
            BasicTextField(value = value, onValueChange = onValue, modifier = Modifier.testTag("field"))
        }
        val field = compose.onNodeWithTag("field")
        field.performClick()
        field.performTextInput("hi :")
        field.performTextInput(")")
        compose.runOnIdle { assertThat(model).isEqualTo("hi $smile") }
        field.performKeyInput { pressKey(Key.Backspace) }
        compose.runOnIdle { assertThat(model).isEqualTo("hi :)") }
        // The caret is behind what came back: the next key lands after it.
        field.performTextInput("!")
        compose.runOnIdle { assertThat(model).isEqualTo("hi :)!") }
        // The model's own cap still has the last word.
        field.performTextInput("0123456789")
        compose.runOnIdle { assertThat(model).hasLength(LIMIT) }
    }

    private companion object {
        const val LIMIT = 12
    }
}
