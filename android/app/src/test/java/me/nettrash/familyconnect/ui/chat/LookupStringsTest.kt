/*
 * LookupStringsTest.kt
 * Family Connect (Android)
 *
 * The copy "Looking things up" ships in, in every language this app ships
 * (docs/protocol.md, "Looking things up"): the owner's switch and its
 * footnote, the consent line and its two buttons, the member's Settings
 * rows, the provider joiners and the statistics row. Lint's
 * MissingTranslation is a warning here, not a gate, so this is the gate —
 * and the sentences that name the providers take exactly one string
 * argument everywhere, the joiners exactly two or three.
 */

package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertWithMessage
import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import org.junit.Test

class LookupStringsTest {

    private val locales = listOf(
        "values", "values-de", "values-es", "values-fr", "values-ja",
        "values-ru", "values-sr", "values-b+sr+Latn", "values-zh-rCN",
    )

    /** Every key, and the placeholders each takes in every language. */
    private val keys = mapOf(
        "s_looking_things_up" to emptyList(),
        "s_assistant_lookups" to emptyList(),
        "s_assistant_lookups_explanation" to listOf("%1\$s"),
        "s_consent_lookups_with_history" to listOf("%1\$s"),
        "s_consent_lookups_alone" to listOf("%1\$s"),
        "s_agree_with_lookups" to emptyList(),
        "s_agree_without_lookups" to emptyList(),
        "s_review_and_allow_lookups" to emptyList(),
        "s_stop_lookups" to emptyList(),
        "s_lookups_until_you_allow" to listOf("%1\$s"),
        "s_lookups_stopping_takes_effect" to listOf("%1\$s"),
        "s_list_two" to listOf("%1\$s", "%2\$s"),
        "s_list_three" to listOf("%1\$s", "%2\$s", "%3\$s"),
        "s_stats_web_searches" to emptyList(),
        "e_change_assistant_lookups_failed" to emptyList(),
    )

    private val placeholder = Regex("""%(\d+\$)?[a-z]""")

    private fun strings(locale: String): Map<String, String> {
        val file = File("src/main/res/$locale/strings.xml").canonicalFile
        val doc = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(file)
        val nodes = doc.getElementsByTagName("string")
        return (0 until nodes.length).associate { i ->
            val node = nodes.item(i)
            node.attributes.getNamedItem("name").nodeValue to node.textContent
        }
    }

    @Test
    fun `every key exists in all nine languages`() {
        for (locale in locales) {
            val table = strings(locale)
            for (key in keys.keys) {
                assertWithMessage("$locale/$key").that(table[key].orEmpty()).isNotEmpty()
            }
        }
    }

    @Test
    fun `each sentence takes exactly the arguments it is given`() {
        for (locale in locales) {
            val table = strings(locale)
            for ((key, expected) in keys) {
                val found = placeholder.findAll(table.getValue(key)).map { it.value }.toList()
                assertWithMessage("$locale/$key").that(found).isEqualTo(expected)
            }
        }
    }

    @Test
    fun `the two consent lines differ only where the family chat's recent messages are`() {
        // With `ai_history` off a mention takes only itself, so the line
        // that says "possibly from recent messages too" must be the longer
        // one in every language — a swapped pair would promise the wrong
        // thing on exactly the families that switched history off.
        for (locale in locales) {
            val table = strings(locale)
            val withHistory = table.getValue("s_consent_lookups_with_history")
            val alone = table.getValue("s_consent_lookups_alone")
            assertWithMessage("$locale: with history is longer").that(withHistory.length).isGreaterThan(alone.length)
        }
    }
}
