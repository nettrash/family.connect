/*
 * EmojiTypingTest.kt
 * Family Connect (Android)
 *
 * Emoji as you type (#101) is the SHARED rule's — `fc_text::emoji_typing`,
 * printed by `cargo run -- emoji-typing` in win/tools/board-oracle into
 * `emoji-typing-vectors.json`, byte-identical in every port's fixtures:
 * the emoticon table in order, and every key sequence typed into an empty
 * field with what the field then holds. Plus `in_code` (#97's test, which
 * Android had no port of), the swap's apply/backspace in the middle of a
 * text, and EmojiTyping.Field — the value-change judge the phone's fields
 * use, since a phone keyboard gives no key events.
 *
 * Plain JUnit: EmojiTyping touches nothing from android.*.
 */

package me.nettrash.familyconnect.util

import com.google.common.truth.Truth.assertThat
import com.google.common.truth.Truth.assertWithMessage
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import me.nettrash.familyconnect.util.EmojiTyping.Swap
import me.nettrash.familyconnect.util.EmojiTyping.Value
import org.junit.Test

class EmojiTypingTest {

    private val vectors: JsonObject = run {
        val stream = requireNotNull(javaClass.classLoader?.getResourceAsStream(VECTORS)) {
            "$VECTORS is missing from the test classpath (app/src/test/resources)"
        }
        val text = stream.use { it.readBytes().toString(Charsets.UTF_8) }
        Json.parseToJsonElement(text).jsonObject
    }

    private fun JsonObject.string(key: String) = getValue(key).jsonPrimitive.content

    @Test
    fun `the emoticon table is the shared one, in order`() {
        val shared = vectors.getValue("emoticons").jsonArray.map {
            it.jsonObject.string("text") to it.jsonObject.string("emoji")
        }
        assertThat(EmojiTyping.EMOTICONS).containsExactlyElementsIn(shared).inOrder()
    }

    @Test
    fun `every typing case in the shared vectors holds`() {
        val cases = vectors.getValue("typing").jsonArray.map { it.jsonObject }
        assertThat(cases.size).isGreaterThan(1500)
        for (case in cases) {
            val keys = case.string("keys")
            assertWithMessage("typing %s", keys).that(EmojiTyping.typing(keys)).isEqualTo(case.string("text"))
        }
    }

    /**
     * The same cases again through EmojiTyping.Field, typed the way a phone
     * field sees them: each key a value change with the caret at the end,
     * `⌫` the deletion of the last code point — or, behind an emoji, of the
     * whole emoji, which is what a Backspace on a phone removes — and `→`
     * a caret move away and back.
     */
    @Test
    fun `every typing case holds through the field's value changes`() {
        val cases = vectors.getValue("typing").jsonArray.map { it.jsonObject }
        for (case in cases) {
            val keys = case.string("keys")
            val field = EmojiTyping.Field()
            var value = Value("", 0)
            var index = 0
            while (index < keys.length) {
                val key = String(Character.toChars(keys.codePointAt(index)))
                index += key.length
                val text = value.text
                when (key) {
                    "⌫" -> {
                        if (text.isEmpty()) continue
                        val emoji = EmojiTyping.EMOTICONS.map { it.second } +
                            listOf("🔥", "👌", "🍕")
                        val cut = emoji.filter { text.endsWith(it) }.maxOfOrNull { it.length }
                            ?: (text.length - text.offsetByCodePoints(text.length, -1))
                        val proposed = Value(text.substring(0, text.length - cut), text.length - cut)
                        value = field.edit(value, proposed) ?: proposed
                    }
                    "→" -> {
                        val away = Value(text, 0)
                        field.edit(value, away)
                        val back = Value(text, text.length)
                        field.edit(away, back)
                        value = back
                    }
                    else -> {
                        val proposed = Value(text + key, text.length + key.length)
                        value = field.edit(value, proposed) ?: proposed
                    }
                }
            }
            // A phone's Backspace on an emoji removes all of it, where the
            // vectors' ordinary Backspace removes one code point; the cases
            // differ only there, and the vectors press one only after ASCII.
            assertWithMessage("typing %s", keys).that(value.text).isEqualTo(case.string("text"))
            assertWithMessage("caret after %s", keys).that(value.selectionStart).isEqualTo(value.text.length)
        }
    }

    @Test
    fun `the names load from the resource`() {
        assertThat(EmojiTyping.nameCount).isGreaterThan(6000)
        assertThat(EmojiTyping.emojiNamed("fire")).isEqualTo("🔥")
        assertThat(EmojiTyping.emojiNamed("piñata")).isEqualTo("🪅")
        assertThat(EmojiTyping.emojiNamed("Fire")).isNull()
        assertThat(EmojiTyping.emojiNamed("")).isNull()
        assertThat(EmojiTyping.emojiNamed(":fire:")).isNull()
    }

    /** #97's in-code rows (web/text/src/composer.rs tests), asked directly. */
    @Test
    fun `in code is the composer's test`() {
        val table = listOf(
            // Inside a fence.
            "```\n" to true,
            // The fence closed.
            "```\ncode\n```\n" to false,
            // Inside a code span.
            "Use `x. " to true,
            // A lone carriage return is a line break.
            "```\r" to true,
            "```\rcode\r```\r" to false,
            // A fence line counts only at the start of a line; a closed code
            // span is not code.
            "x ```\n" to false,
            "Use `x`. " to false,
            "" to false,
            "Hi. " to false,
            // A span on an earlier line does not reach this one.
            "`x\n" to false,
            "```\r\n" to true,
        )
        for ((before, expected) in table) {
            assertWithMessage("in code %s", before).that(EmojiTyping.inCode(before)).isEqualTo(expected)
        }
    }

    @Test
    fun `a swap lands between before and after`() {
        val swap = EmojiTyping.replace("ok :", ")", null)
        assertThat(swap).isEqualTo(Swap(at = 3, put = "😊", typed = ":)"))
        val (text, caret) = swap!!.apply("ok :", " bye")
        assertThat(text).isEqualTo("ok 😊 bye")
        assertThat(caret).isEqualTo(3 + 2)
        assertThat(swap.backspace(text, caret)).isEqualTo("ok :) bye" to 5)
        // Not right behind it: an ordinary Backspace.
        assertThat(swap.backspace(text, caret + 1)).isNull()
        assertThat(swap.backspace("ok x bye", caret)).isNull()
        assertThat(swap.backspace("ok", caret)).isNull()
    }

    @Test
    fun `a shortcode in the middle of a text comes back whole`() {
        val swap = EmojiTyping.replace("so :fire", ":", null)!!
        assertThat(swap).isEqualTo(Swap(at = 3, put = "🔥", typed = ":fire:"))
        val (text, caret) = swap.apply("so :fire", " today")
        assertThat(text).isEqualTo("so 🔥 today")
        assertThat(swap.backspace(text, caret)).isEqualTo("so :fire: today" to 9)
    }

    @Test
    fun `a swap grows only from where it started`() {
        val fire = EmojiTyping.replace(":fire", ":", null)!!
        val text = fire.apply(":fire", "").first
        assertThat(EmojiTyping.replace(text, "(", fire)).isNull()
    }

    @Test
    fun `guards are ASCII only`() {
        // A letter outside ASCII is not a letter to the guard.
        assertThat(EmojiTyping.typing("é<3")).isEqualTo("é❤️")
        assertThat(EmojiTyping.typing("٣:)")).isEqualTo("٣😊")
        // An emoji before a face is not a letter either.
        assertThat(EmojiTyping.typing("🔥:)")).isEqualTo("🔥😊")
    }

    @Test
    fun `the field replaces only one character typed at the caret`() {
        val field = EmojiTyping.Field()
        // In the middle of a text.
        val out = field.edit(Value("ok : bye", 4), Value("ok :) bye", 5))
        assertThat(out).isEqualTo(Value("ok 😊 bye", 5))
        // Its Backspace: the emoji deleted, the face comes back.
        val back = field.edit(out!!, Value("ok  bye", 3))
        assertThat(back).isEqualTo(Value("ok :) bye", 5))
        // Only once.
        assertThat(field.edit(back!!, Value("ok : bye", 4))).isNull()

        // A paste of two characters is left alone.
        assertThat(EmojiTyping.Field().edit(Value("ok ", 3), Value("ok :)", 5))).isNull()
        // A character typed over a selection is left alone.
        assertThat(EmojiTyping.Field().edit(Value("ok :x", 4, 5), Value("ok :)", 5))).isNull()
        // While a composition is open, nothing is replaced.
        assertThat(EmojiTyping.Field().edit(Value("ok :", 4), Value("ok :)", 5), composing = true)).isNull()
        // The caret somewhere else than behind the character.
        assertThat(EmojiTyping.Field().edit(Value("ok :", 4), Value("ok :)", 4))).isNull()
    }

    @Test
    fun `a typed character outside the BMP is one character`() {
        val field = EmojiTyping.Field()
        // A pair typed is judged as one character (and replaces nothing).
        assertThat(field.edit(Value(":", 1), Value(":😀", 3))).isNull()
        // Half a pair is not a character.
        assertThat(field.edit(Value(":", 1), Value(":\uD83D", 2))).isNull()
        // Two BMP characters are not one.
        assertThat(field.edit(Value(":", 1), Value(":)(", 3))).isNull()
    }

    @Test
    fun `anything else in between drops the swap`() {
        val field = EmojiTyping.Field()
        val out = field.edit(Value(":", 1), Value(":)", 2))!!
        // The caret moved away and back.
        assertThat(field.edit(out, Value(out.text, 0))).isNull()
        assertThat(field.edit(Value(out.text, 0), out)).isNull()
        assertThat(field.edit(out, Value("", 0))).isNull()

        // The app changed the field (a mention picked, a draft restored).
        val again = EmojiTyping.Field()
        val swapped = again.edit(Value(":", 1), Value(":)", 2))!!
        val elsewhere = Value("x" + swapped.text, swapped.text.length + 1)
        assertThat(again.edit(elsewhere, Value("x", 1))).isNull()
    }

    @Test
    fun `a composition alone keeps the swap`() {
        val field = EmojiTyping.Field()
        val out = field.edit(Value(":", 1), Value(":)", 2))!!
        // Same text, same selection: nothing happened to the swap.
        assertThat(field.edit(out, out, composing = true)).isNull()
        assertThat(field.edit(out, Value("", 0))).isEqualTo(Value(":)", 2))
    }

    @Test
    fun `a space-ended face comes back from a Backspace on its space`() {
        val field = EmojiTyping.Field()
        val out = field.edit(Value(":o", 2), Value(":o ", 3))!!
        assertThat(out).isEqualTo(Value("😨 ", 3))
        // The keyboard deletes only the space; what was typed comes back.
        assertThat(field.edit(out, Value("😨", 2))).isEqualTo(Value(":o ", 3))
    }

    @Test
    fun `a word deleted is not a Backspace`() {
        val field = EmojiTyping.Field()
        val out = field.edit(Value("hi :", 4), Value("hi :)", 5))!!
        assertThat(out).isEqualTo(Value("hi 😊", 5))
        assertThat(field.edit(out, Value("", 0))).isNull()
    }

    @Test
    fun `growing works through the field`() {
        val field = EmojiTyping.Field()
        val first = field.edit(Value(":", 1), Value(":(", 2))!!
        assertThat(first).isEqualTo(Value("😞", 2))
        val grown = field.edit(first, Value("😞(", 3))!!
        assertThat(grown).isEqualTo(Value("😩", 2))
        assertThat(field.edit(grown, Value("", 0))).isEqualTo(Value(":((", 3))
    }

    @Test
    fun `code is typed as it is through the field`() {
        val field = EmojiTyping.Field()
        assertThat(field.edit(Value("`:", 2), Value("`:)", 3))).isNull()
        assertThat(field.edit(Value("```\n:fire", 9), Value("```\n:fire:", 10))).isNull()
    }

    private companion object {
        const val VECTORS = "emoji-typing-vectors.json"
    }
}
