/*
 * VoiceMessageStringsTest.kt
 * Family Connect (Android)
 *
 * The copy voice messages speak (#79, docs/audio-video-messages-2026-10-04.md,
 * S10) ships in every language this app ships. Phase 0: the "Voice message
 * not sent" row and its two buttons, the question before a long recording is
 * deleted, and the sentences for a call, a waiting recording, the cap and a
 * failed recorder. Phase 1: the slot's labels and actions, the hold, recording
 * and Undo rows, the teaching lines, the warnings, the announcements, the
 * setting, and the paperclip's two renames. Phase 2: a received video
 * message — its chat-list words, its TalkBack label and state, "Open full
 * screen", and the failed line. Phase 3: recording one — the entries, the
 * recorder's controls, status lines, notices and announcements. Lint's MissingTranslation is a
 * warning here, not a gate, so this is the gate — and each sentence takes
 * exactly the arguments it is given.
 *
 * The translations are the apps' catalogue's own
 * (ios/FamilyConnect/Localizable.xcstrings, the source of truth for the nine
 * languages), so the two phones say the same thing.
 */

package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertWithMessage
import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import org.junit.Test

class VoiceMessageStringsTest {

    private val locales = listOf(
        "values", "values-de", "values-es", "values-fr", "values-ja",
        "values-ru", "values-sr", "values-b+sr+Latn", "values-zh-rCN",
    )

    /** Every key, and the placeholders each takes in every language. */
    private val keys = mapOf(
        "s_voice_message_not_sent" to listOf("%1\$s"),
        "s_send_voice_message" to emptyList(),
        "s_delete_recording" to emptyList(),
        "s_keep" to emptyList(),
        "s_delete_this_recording" to emptyList(),
        "e_record_after_the_call" to emptyList(),
        "e_send_or_delete_the_unsent_first" to emptyList(),
        "s_recording_stopped_at_five_minutes" to emptyList(),
        "e_recording_stopped_unexpectedly" to emptyList(),
        "s_voice_message_with_length" to listOf("%1\$s"),
        // Phase 1: voice in the Send slot (S1-S2, S6, S7, S9).
        "s_record_voice_message" to emptyList(),
        "s_take_video" to emptyList(),
        "s_undo" to emptyList(),
        "s_open_settings" to emptyList(),
        "s_stop_recording" to emptyList(),
        "s_stop_and_listen_first" to emptyList(),
        "s_start_recording_action" to emptyList(),
        "s_sending_voice_message" to listOf("%1\$s"),
        "s_sending_in" to listOf("%1\$d"),
        "s_slide_to_cancel" to emptyList(),
        "s_release_to_cancel" to emptyList(),
        "s_still_recording_tap_send" to emptyList(),
        "s_next_time_letting_go_sends" to emptyList(),
        "s_hold_the_microphone_coach" to emptyList(),
        "s_play_after_recording" to emptyList(),
        "s_thirty_seconds_left" to emptyList(),
        "s_voice_messages" to emptyList(),
        "s_review_before_sending" to emptyList(),
        "s_review_before_sending_explanation" to emptyList(),
        "s_you_can_record_now" to emptyList(),
        "s_we_didnt_hear_anything" to emptyList(),
        "s_cant_hear_microphone_muted" to emptyList(),
        "s_announce_recording" to emptyList(),
        "s_announce_recording_locked" to emptyList(),
        "s_announce_recording_deleted" to emptyList(),
        "s_announce_voice_message_sent" to emptyList(),
        "s_announce_ready_to_review" to listOf("%1\$s"),
        "e_wait_for_the_attachment" to emptyList(),
        // Phase 2: video messages received (S5, S6).
        "s_video_message" to emptyList(),
        "s_video_message_a11y" to listOf("%1\$s"),
        "s_not_played" to emptyList(),
        "s_open_full_screen" to emptyList(),
        "s_couldnt_load_video_tap" to emptyList(),
        // Phase 3: recording a video message (S1.4–S1.6, S3, S4, S6).
        "s_record_video_message" to emptyList(),
        "s_send_video_message" to emptyList(),
        "s_retake" to emptyList(),
        "s_record" to emptyList(),
        "s_switch_camera" to emptyList(),
        "s_video_message_with_length" to listOf("%1\$s"),
        "s_not_recording" to emptyList(),
        "s_only_you_can_see_this" to emptyList(),
        "s_starting_camera" to emptyList(),
        "s_cant_see_anything" to emptyList(),
        "s_ten_seconds_left" to emptyList(),
        "s_recording_stopped_at_one_minute" to emptyList(),
        "s_video_too_short" to emptyList(),
        "s_delete_video_message" to emptyList(),
        "s_video_messages_need_camera_and_microphone" to emptyList(),
        "e_camera_permission_settings" to emptyList(),
        "s_camera_in_use_by_another_app" to emptyList(),
        "s_couldnt_make_it_round" to emptyList(),
        "s_too_big_for_video_message" to emptyList(),
        "s_announce_camera_ready" to emptyList(),
        "s_announce_recording_video" to emptyList(),
        "s_announce_video_message_sent" to emptyList(),
        "s_announce_camera_turned_off" to emptyList(),
        "s_record_voice_message_instead" to emptyList(),
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

    /** A translation that is still the English source is a missed translation, not a choice. */
    @Test
    fun `nothing is left in English outside English`() {
        val english = strings("values")
        for (locale in locales.drop(1)) {
            val table = strings(locale)
            for (key in keys.keys) {
                assertWithMessage("$locale/$key").that(table.getValue(key)).isNotEqualTo(english.getValue(key))
            }
        }
    }

    /**
     * The paperclip's two renames (S1.5, Decision 31): "Record audio" is
     * "Record voice message" now, and the system camera's "Record video" is
     * "Take video" — so neither old key may linger in any language.
     */
    @Test
    fun `the renamed menu items are gone everywhere`() {
        for (locale in locales) {
            val table = strings(locale)
            assertWithMessage("$locale/s_record_audio").that(table).doesNotContainKey("s_record_audio")
            assertWithMessage("$locale/s_record_video").that(table).doesNotContainKey("s_record_video")
        }
    }

    /** Android's sentence case where S10 names one (S10: "Android uses sentence case for menu items and buttons"). */
    @Test
    fun `the Android spellings are sentence case`() {
        val english = strings("values")
        assertWithMessage("menu item").that(english["s_record_voice_message"]).isEqualTo("Record voice message")
        assertWithMessage("settings section").that(english["s_voice_messages"]).isEqualTo("Voice messages")
        assertWithMessage("settings switch").that(english["s_review_before_sending"]).isEqualTo("Review before sending")
        assertWithMessage("system camera").that(english["s_take_video"]).isEqualTo("Take video")
        assertWithMessage("message menu").that(english["s_open_full_screen"]).isEqualTo("Open full screen")
        assertWithMessage("video menu item").that(english["s_record_video_message"]).isEqualTo("Record video message")
    }
}
