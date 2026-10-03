/*
 * TranscriptStringsTest.kt
 * Family Connect (Android)
 *
 * The copy "Show text", the owner's switch and the consent line ship in,
 * in every language this app ships (docs/protocol.md, "Transcripts on
 * request"). Lint's MissingTranslation is a warning here, not a gate, so
 * this is the gate — and the two sentences that name the processor take
 * exactly one string argument everywhere.
 */

package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertWithMessage
import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import org.junit.Test

class TranscriptStringsTest {

    private val locales = listOf(
        "values", "values-de", "values-es", "values-fr", "values-ja",
        "values-ru", "values-sr", "values-b+sr+Latn", "values-zh-rCN",
    )

    private val keys = listOf(
        "s_transcript_show",
        "s_transcript_hide",
        "s_transcript_getting",
        "s_transcript_no_speech",
        "s_transcript_failed",
        "s_transcript_refused",
        "s_transcript_unavailable",
        "s_transcript_too_long",
        "s_transcript_unreadable",
        "s_transcript_a11y",
        "s_assistant_transcripts",
        "s_assistant_transcripts_explanation",
        "s_assistant_transcripts_no_server",
        "s_consent_transcript_sound_sent",
        "e_change_assistant_transcripts_failed",
    )

    /** The two that say WHERE the sound goes: `%1$s` is `assistant.processor`. */
    private val namingTheProcessor = setOf("s_assistant_transcripts_explanation", "s_consent_transcript_sound_sent")

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
            for (key in keys) {
                assertWithMessage("$locale/$key").that(table[key].orEmpty()).isNotEmpty()
            }
        }
    }

    @Test
    fun `the processor is named exactly once, and nothing else is formatted`() {
        val placeholder = Regex("""%(\d+\$)?[a-z]""")
        for (locale in locales) {
            val table = strings(locale)
            for (key in keys) {
                val found = placeholder.findAll(table.getValue(key)).map { it.value }.toList()
                val expected = if (key in namingTheProcessor) listOf("%1\$s") else emptyList()
                assertWithMessage("$locale/$key").that(found).isEqualTo(expected)
            }
        }
    }
}
