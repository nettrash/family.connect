/*
 * GreetingPlacesStringsTest.kt
 * Family Connect (Android)
 *
 * The copy "Weather in the greeting" ships in, in every language this app
 * ships — the catalogue's wording (ios/FamilyConnect/Localizable.xcstrings,
 * #72). Lint's MissingTranslation is a warning here, not a gate, so this is
 * the gate: every key in all nine languages, no placeholder anywhere (the
 * limit's 3 is written into the text), and the provider's name kept as
 * "Open-Meteo" in every footnote, since that sentence is the disclosure of
 * where the names go.
 */

package me.nettrash.familyconnect.ui.familyadmin

import com.google.common.truth.Truth.assertWithMessage
import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import org.junit.Test

class GreetingPlacesStringsTest {

    private val locales = listOf(
        "values", "values-de", "values-es", "values-fr", "values-ja",
        "values-ru", "values-sr", "values-b+sr+Latn", "values-zh-rCN",
    )

    private val keys = listOf(
        "s_greeting_weather",
        "s_greeting_weather_place_placeholder",
        "s_greeting_weather_add_place",
        "s_greeting_weather_remove_place",
        "s_greeting_weather_limit",
        "s_greeting_weather_footer",
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
    fun `every key exists in all nine languages, without placeholders`() {
        for (locale in locales) {
            val table = strings(locale)
            for (key in keys) {
                val value = table[key].orEmpty()
                assertWithMessage("$locale/$key").that(value).isNotEmpty()
                assertWithMessage("$locale/$key").that(placeholder.containsMatchIn(value)).isFalse()
            }
        }
    }

    @Test
    fun `the footnote names Open-Meteo and the limit names three, in every language`() {
        for (locale in locales) {
            val table = strings(locale)
            assertWithMessage(locale).that(table.getValue("s_greeting_weather_footer")).contains("Open-Meteo")
            assertWithMessage(locale).that(table.getValue("s_greeting_weather_limit")).contains("3")
        }
    }
}
