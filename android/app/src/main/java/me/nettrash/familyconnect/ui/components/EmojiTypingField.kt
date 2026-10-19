/*
 * EmojiTypingField.kt
 * Family Connect (Android)
 *
 * Emoji as you type (#101; docs/emoji-as-you-type-2026-10-18.md, "Where it
 * runs") on every prose field — the same set as ProseKeyboard.Sentences: the
 * chat composer (which is also where a message is edited), the thread reply,
 * a board note, a list's line, an event's title, a poll's question and
 * options, and a report's note. NEVER a username, a password, a server
 * address, an invite code, a search field, a sticker label — or an event's
 * place, which is a name rather than a sentence (ProseKeyboard.Words).
 *
 * A phone keyboard gives no key events, so the field's VALUE CHANGE is
 * judged: exactly one character inserted at the caret runs the rule, a
 * Backspace straight after a replacement takes it back, and anything else
 * passes through untouched. The judging is EmojiTyping.Field's (pure Kotlin,
 * JVM-tested); this file only carries it to Compose's three kinds of field:
 *
 * - [EmojiTypingField], for a field holding a TextFieldValue;
 * - [EmojiTypingInput], an InputTransformation for a state-based field (the
 *   chat composer) — chained BEFORE the length limit, so the limit is
 *   always the last word;
 * - [proseValue], for a field whose model is a plain String: it keeps the
 *   TextFieldValue (and so the caret) the String overload would have kept
 *   privately, which the take-back needs to put the caret behind what it
 *   restores.
 */

package me.nettrash.familyconnect.ui.components

import androidx.compose.foundation.text.input.InputTransformation
import androidx.compose.foundation.text.input.TextFieldBuffer
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import me.nettrash.familyconnect.util.EmojiTyping

/** One TextFieldValue field's emoji typing: hand it every value change. */
class EmojiTypingField {
    private val field = EmojiTyping.Field()

    /**
     * What the field should hold after the keyboard proposed `new` while it
     * held `old`: `new` itself, or the value with the emoji put in (or the
     * typed text put back) and the caret right behind it.
     */
    fun onValueChange(old: TextFieldValue, new: TextFieldValue): TextFieldValue {
        val out = field.edit(old.judged(), new.judged(), composing = new.composition != null)
            ?: return new
        return TextFieldValue(out.text, TextRange(out.selectionStart, out.selectionEnd))
    }

    private fun TextFieldValue.judged() = EmojiTyping.Value(text, selection.start, selection.end)
}

/**
 * The chat composer's emoji typing, as an InputTransformation over its
 * TextFieldState. It runs for every user edit, caret moves included, and
 * never for the app's own edits (a mention picked, a draft restored, the
 * field cleared after Send) — which the live swap notices, because the field
 * is then no longer as the swap left it.
 *
 * One thing this side cannot see: whether the keyboard still has a
 * composition open after the edit (Compose keeps that internal here). The
 * one-character-at-the-caret test stands alone; an emoticon's last
 * character is punctuation, which keyboards commit rather than compose.
 */
class EmojiTypingInput : InputTransformation {
    private val field = EmojiTyping.Field()

    override fun TextFieldBuffer.transformInput() {
        val current = asCharSequence().toString()
        val out = field.edit(
            old = EmojiTyping.Value(originalText.toString(), originalSelection.start, originalSelection.end),
            new = EmojiTyping.Value(current, selection.start, selection.end),
        ) ?: return
        // Only the part that differs is replaced, so the edit stays one
        // small change for undo.
        val wanted = out.text
        var head = 0
        val most = minOf(current.length, wanted.length)
        while (head < most && current[head] == wanted[head]) head++
        var tail = 0
        while (tail < most - head &&
            current[current.length - 1 - tail] == wanted[wanted.length - 1 - tail]
        ) {
            tail++
        }
        replace(head, current.length - tail, wanted.substring(head, wanted.length - tail))
        selection = TextRange(out.selectionStart, out.selectionEnd)
    }
}

/**
 * A String-backed prose field's TextFieldValue, kept here so the caret is
 * ours to place. Mirrors what BasicTextField's String overload keeps
 * privately: the text is always the model's, the selection is the last one
 * the field reported (clamped when the model's text changed from outside).
 */
@Stable
class ProseText internal constructor(initial: String) {
    private var held by mutableStateOf(TextFieldValue(text = initial))
    private val emoji = EmojiTypingField()

    /**
     * The model's text when this last handed a new text to it. Until the
     * screen recomposes, the model's text the next change sees is still
     * that one, while the field already holds [held]: two keyboard changes
     * inside one frame must not take the field to be the stale text.
     */
    private var handedFrom: String? = null

    /** The value to show for the model's `text`. */
    fun value(text: String): TextFieldValue = if (held.text == text) held else held.copy(text = text)

    /** The field proposed `new` while the model, as last composed, held `text`. */
    fun onValueChange(new: TextFieldValue, text: String, onTextChange: (String) -> Unit) {
        val old = if (text == handedFrom) held else value(text)
        val out = emoji.onValueChange(old, new)
        held = out
        handedFrom = null
        if (out.text != text) {
            handedFrom = text
            onTextChange(out.text)
        }
    }
}

/**
 * Emoji typing for a prose field whose model is a String: the value and the
 * onValueChange to give an OutlinedTextField in place of the String pair.
 *
 * ```
 * val (titleValue, onTitleValue) = proseValue(title) { title = NoteText.capped(it) }
 * OutlinedTextField(value = titleValue, onValueChange = onTitleValue, …)
 * ```
 */
@Composable
fun proseValue(text: String, onTextChange: (String) -> Unit): Pair<TextFieldValue, (TextFieldValue) -> Unit> {
    val field = remember { ProseText(text) }
    val latestText by rememberUpdatedState(text)
    val latestChange by rememberUpdatedState(onTextChange)
    val onValueChange = remember(field) {
        { new: TextFieldValue -> field.onValueChange(new, latestText) { latestChange(it) } }
    }
    return field.value(text) to onValueChange
}
