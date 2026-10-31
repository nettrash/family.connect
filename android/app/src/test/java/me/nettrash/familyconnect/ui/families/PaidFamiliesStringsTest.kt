/*
 * PaidFamiliesStringsTest.kt
 * Family Connect (Android)
 *
 * Every word paid families added (#89; docs/protocol.md, "Paid families"),
 * in all nine languages this app ships — TranscriptStringsTest's gate, for
 * these keys. Each formatted sentence takes exactly the arguments it is
 * given (Japanese puts the limit before the amount, by position), and none
 * of them — in any language — names another app, a store or a price:
 * Google Play's payments policy, and decision D6.
 */

package me.nettrash.familyconnect.ui.families

import com.google.common.truth.Truth.assertWithMessage
import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import org.junit.Test

class PaidFamiliesStringsTest {

    private val locales = listOf(
        "values", "values-de", "values-es", "values-fr", "values-ja",
        "values-ru", "values-sr", "values-b+sr+Latn", "values-zh-rCN",
    )

    /** key → the placeholders it must carry, in the order they appear. */
    private val keys: Map<String, List<String>> = mapOf(
        "s_gate_cannot_start_here" to emptyList(),
        "e_subscription_required" to emptyList(),
        "s_banner_member" to listOf("%1\$s", "%2\$s", "%3\$s"),
        "s_banner_owner_not_here" to listOf("%1\$s", "%2\$s"),
        "s_allowance_title" to emptyList(),
        "s_allowance_storage" to emptyList(),
        "s_allowance_assistant" to emptyList(),
        "s_allowance_pictures" to emptyList(),
        "s_allowance_transcripts" to emptyList(),
        "s_allowance_of" to listOf("%1\$s", "%2\$s"),
        "s_allowance_percent" to listOf("%1\$d"),
        "s_allowance_minutes" to listOf("%1\$d", "%2\$d"),
        "s_allowance_renews_on" to listOf("%1\$s"),
        "e_storage_allowance_used" to emptyList(),
        "e_ai_allowance_used" to listOf("%1\$s"),
    )

    /** Where the source puts the limit first, the placeholders appear swapped. */
    private val reordered = mapOf(
        "values-ja" to mapOf(
            "s_allowance_of" to listOf("%2\$s", "%1\$s"),
            "s_allowance_minutes" to listOf("%2\$d", "%1\$d"),
        ),
    )

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
        // `%%` is a literal percent sign, not an argument.
        val placeholder = Regex("""%(\d+\$)?[a-z]""")
        for (locale in locales) {
            val table = strings(locale)
            for ((key, expected) in keys) {
                val text = table.getValue(key).replace("%%", "")
                val found = placeholder.findAll(text).map { it.value }.toList()
                val wanted = reordered[locale]?.get(key) ?: expected
                assertWithMessage("$locale/$key").that(found).isEqualTo(wanted)
            }
        }
    }

    @Test
    fun `the percentage keeps its percent sign in every language`() {
        for (locale in locales) {
            assertWithMessage(locale).that(strings(locale).getValue("s_allowance_percent")).contains("%%")
        }
    }

    @Test
    fun `nothing names another app, a store or a price`() {
        val banned = listOf("iPhone", "iPad", "Mac", "Apple", "App Store", "Google Play")
        // A price: a currency sign, or a dollar sign before a digit (`%1$s` is an argument).
        val price = Regex("""[€£¥₽]|\$\d""")
        for (locale in locales) {
            val table = strings(locale)
            for (key in keys.keys) {
                val text = table.getValue(key)
                for (word in banned) {
                    assertWithMessage("$locale/$key names $word").that(text).doesNotContain(word)
                }
                assertWithMessage("$locale/$key names a price").that(price.containsMatchIn(text)).isFalse()
            }
        }
    }
}
