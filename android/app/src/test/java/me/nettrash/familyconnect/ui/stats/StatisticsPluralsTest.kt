/*
 * StatisticsPluralsTest.kt
 * Family Connect (Android)
 *
 * A member's line on the statistics screen counts things — attachments,
 * questions, pictures, recordings as text — and a count is a plural
 * resource in every language this app ships, with every form that
 * language's count takes. As a plain <string> it said "1 pictures from the
 * assistant" in English and "1 картинок" in Russian.
 */

package me.nettrash.familyconnect.ui.stats

import com.google.common.truth.Truth.assertWithMessage
import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import org.junit.Test
import org.w3c.dom.Element

class StatisticsPluralsTest {

    /** The nine locales, and the forms each one's counts need. */
    private val locales = mapOf(
        "values" to setOf("one", "other"),
        "values-de" to setOf("one", "other"),
        "values-es" to setOf("one", "many", "other"),
        "values-fr" to setOf("one", "many", "other"),
        "values-ja" to setOf("other"),
        "values-ru" to setOf("one", "few", "many", "other"),
        "values-sr" to setOf("one", "few", "other"),
        "values-b+sr+Latn" to setOf("one", "few", "other"),
        "values-zh-rCN" to setOf("other"),
    )

    /** Each count, and the arguments every one of its forms takes. */
    private val counts = mapOf(
        "s_attachments_and_size" to listOf("%1\$d", "%2\$s"),
        "s_questions_to_assistant" to listOf("%1\$d"),
        "s_pictures_from_assistant" to listOf("%1\$d"),
        "s_recordings_as_text" to listOf("%1\$d"),
    )

    private val placeholder = Regex("""%(\d+\$)?[a-z]""")

    @Test
    fun `every count is a plural with every form its language needs, in all nine languages`() {
        for ((locale, forms) in locales) {
            val doc = DocumentBuilderFactory.newInstance().newDocumentBuilder()
                .parse(File("src/main/res/$locale/strings.xml").canonicalFile)
            val strings = doc.getElementsByTagName("string").let { nodes ->
                (0 until nodes.length).map { (nodes.item(it) as Element).getAttribute("name") }.toSet()
            }
            val plurals = doc.getElementsByTagName("plurals").let { nodes ->
                (0 until nodes.length).map { nodes.item(it) as Element }.associateBy { it.getAttribute("name") }
            }
            for ((name, args) in counts) {
                assertWithMessage("$locale/$name is still a plain string").that(strings).doesNotContain(name)
                val plural = plurals[name]
                assertWithMessage("$locale/$name").that(plural).isNotNull()
                val items = plural!!.getElementsByTagName("item").let { nodes ->
                    (0 until nodes.length).map { nodes.item(it) as Element }
                        .associate { it.getAttribute("quantity") to it.textContent }
                }
                assertWithMessage("$locale/$name forms").that(items.keys).isEqualTo(forms)
                for ((quantity, text) in items) {
                    assertWithMessage("$locale/$name/$quantity")
                        .that(placeholder.findAll(text).map { it.value }.toList())
                        .isEqualTo(args)
                }
            }
        }
    }
}
