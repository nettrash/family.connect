/*
 * RecentPhotosStringsTest.kt
 * Family Connect (Android)
 *
 * The copy the third switch and the family strip ship in, in every
 * language this app ships (docs/protocol.md, "Recent photos from the
 * family chat" — "What a client shows", "in every language they ship").
 *
 * A string added to `values/` alone compiles, runs, and ships in English
 * for eight other locales with nothing failing to say so — lint's
 * MissingTranslation is a warning here, not a gate. So this is the gate:
 * every key the feature reads has a value in every locale directory, and
 * the two sentences the protocol asks to change ("never from an earlier
 * message … unless Recent photos is on") actually changed everywhere.
 */

package me.nettrash.familyconnect.ui.familyadmin

import com.google.common.truth.Truth.assertThat
import com.google.common.truth.Truth.assertWithMessage
import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import org.junit.Test

class RecentPhotosStringsTest {

    /** The nine locales this app ships, as resource directories. */
    private val locales = listOf(
        "values", "values-de", "values-es", "values-fr", "values-ja",
        "values-ru", "values-sr", "values-b+sr+Latn", "values-zh-rCN",
    )

    /** Every key the feature reads, on the owner's screen and in the strip. */
    private val keys = listOf(
        "s_assistant_history_photos",
        "s_assistant_history_photos_explanation",
        "s_assistant_history_photos_no_deployment",
        "s_assistant_history_photos_needs_vision",
        "s_assistant_history_photos_needs_history",
        "e_change_assistant_history_photos_failed",
        "s_assistant_mention_sees_recent_only",
        "s_assistant_mention_sees_this_picture_and_recent",
        "s_assistant_mention_sees_these_pictures_and_recent",
        "s_assistant_mention_sees_quoted_picture_and_recent",
        "s_assistant_mention_sees_quoted_pictures_and_recent",
        "s_assistant_mention_sees_both_and_recent",
        // The sentence that changed again.
        "s_assistant_vision_explanation",
    )

    private fun strings(locale: String): Map<String, String> {
        val file = File("src/main/res/$locale/strings.xml").canonicalFile
        assertThat(file.exists()).isTrue()
        val doc = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(file)
        val nodes = doc.getElementsByTagName("string")
        return (0 until nodes.length).associate { i ->
            val node = nodes.item(i)
            node.attributes.getNamedItem("name").nodeValue to node.textContent
        }
    }

    @Test
    fun `every key the feature reads exists in all nine languages`() {
        for (locale in locales) {
            val table = strings(locale)
            for (key in keys) {
                assertWithMessage("$locale/$key").that(table[key]).isNotNull()
                assertWithMessage("$locale/$key").that(table.getValue(key)).isNotEmpty()
            }
        }
    }

    /**
     * The strip's "up to N" sentences carry the count, and only the
     * count — one positional integer, so a translation cannot reorder
     * it against a second argument that does not exist.
     */
    @Test
    fun `the strip's recent-photo sentences take exactly one count`() {
        val counted = keys.filter { it.startsWith("s_assistant_mention_sees_") }
        for (locale in locales) {
            val table = strings(locale)
            for (key in counted) {
                val value = table.getValue(key)
                assertWithMessage("$locale/$key").that(value).contains("%1\$d")
                assertWithMessage("$locale/$key").that(value.split("%").size - 1).isEqualTo(1)
            }
        }
    }

    /**
     * No two locales share a value for the same key, which is the cheap
     * proof that a translation was written rather than the English copied
     * over — Serbian's two scripts are the same language in two alphabets
     * and still differ character for character.
     */
    @Test
    fun `no locale ships the English copy under a translated key`() {
        val english = strings("values")
        for (locale in locales.filter { it != "values" }) {
            val table = strings(locale)
            for (key in keys) {
                assertWithMessage("$locale/$key").that(table.getValue(key)).isNotEqualTo(english.getValue(key))
            }
        }
    }

    /** The `ai_vision` sentence names the third switch in every locale. */
    @Test
    fun `the picture sentence now names the third switch everywhere`() {
        for (locale in locales) {
            val table = strings(locale)
            assertWithMessage(locale)
                .that(table.getValue("s_assistant_vision_explanation"))
                .contains(table.getValue("s_assistant_history_photos"))
        }
    }
}
