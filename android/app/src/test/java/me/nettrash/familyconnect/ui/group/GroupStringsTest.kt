/*
 * GroupStringsTest.kt
 * Family Connect (Android)
 *
 * Every word groups added (#107; docs/protocol.md, "Groups"), and every
 * disclosure groups reworded, in all nine languages this app ships —
 * TranscriptStringsTest's gate, for these keys. Each formatted sentence
 * takes exactly the arguments it is given, and the reworded disclosures
 * keep theirs.
 */

package me.nettrash.familyconnect.ui.group

import com.google.common.truth.Truth.assertWithMessage
import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import org.junit.Test

class GroupStringsTest {

    private val locales = listOf(
        "values", "values-de", "values-es", "values-fr", "values-ja",
        "values-ru", "values-sr", "values-b+sr+Latn", "values-zh-rCN",
    )

    /** key → the placeholders it must carry, in order. */
    private val keys: Map<String, List<String>> = mapOf(
        "s_new_group" to emptyList(),
        "s_group_name" to emptyList(),
        "s_create_group" to emptyList(),
        "s_group_info" to emptyList(),
        "s_add_members" to emptyList(),
        "s_rename_group" to emptyList(),
        "s_remove_from_group" to emptyList(),
        "s_leave_group" to emptyList(),
        "s_delete_group" to emptyList(),
        "s_group_admin" to emptyList(),
        "s_group" to emptyList(),
        "s_group_members_count" to listOf("%1\$d"),
        "s_delete_group_confirm" to listOf("%1\$s"),
        "s_leave_group_confirm" to listOf("%1\$s"),
        "e_group_choose_member" to emptyList(),
        "e_group_name_length" to emptyList(),
        "e_too_many_groups" to emptyList(),
        "e_not_group_admin" to emptyList(),
        "e_create_group_failed" to emptyList(),
        "e_change_group_failed" to emptyList(),
        "s_everyone_already_in_group" to emptyList(),
        // Reworded for "the family chat or a group" (amended 2026-10-22).
        "s_consent_transcript_direct_chat" to listOf("%1\$s"),
        "s_consent_lookups_with_history" to listOf("%1\$s"),
        "s_consent_family_history_travels" to emptyList(),
        "s_consent_family_mention_alone" to emptyList(),
        "s_family_language_explanation" to emptyList(),
        "s_assistant_vision_explanation" to emptyList(),
        "s_assistant_history_explanation" to emptyList(),
        "s_assistant_history_photos_explanation" to emptyList(),
        "s_assistant_faces_explanation" to emptyList(),
    )

    /** The reworded lines, which now say "or a group" — in English, where it can be read. */
    private val sayGroup = listOf(
        "s_consent_transcript_direct_chat",
        "s_consent_lookups_with_history",
        "s_consent_family_history_travels",
        "s_consent_family_mention_alone",
        "s_family_language_explanation",
        "s_assistant_vision_explanation",
        "s_assistant_history_explanation",
        "s_assistant_history_photos_explanation",
        "s_assistant_faces_explanation",
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
        val placeholder = Regex("""%(\d+\$)?[a-z]""")
        for (locale in locales) {
            val table = strings(locale)
            for ((key, expected) in keys) {
                val found = placeholder.findAll(table.getValue(key)).map { it.value }.toList()
                assertWithMessage("$locale/$key").that(found).isEqualTo(expected)
            }
        }
    }

    @Test
    fun `the reworded disclosures name groups`() {
        val english = strings("values")
        for (key in sayGroup) {
            assertWithMessage(key).that(english.getValue(key)).contains("group")
        }
        // The transcripts switch alone keeps "the family chat": that is all it covers.
        assertWithMessage("s_assistant_transcripts_explanation")
            .that(english.getValue("s_assistant_transcripts_explanation")).doesNotContain("group")
    }
}
