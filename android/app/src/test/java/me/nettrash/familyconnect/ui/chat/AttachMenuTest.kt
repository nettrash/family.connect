package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertThat
import com.google.common.truth.Truth.assertWithMessage
import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import me.nettrash.familyconnect.ui.chat.AttachMenu.Item
import org.junit.Test

/**
 * Pins the paperclip's menu (issue #78, docs/attachment-menu-2026-10-07.md):
 * the same items, in the same order, in the same three groups as every other
 * client, with no group that came out empty — and the words it needs in all
 * nine languages.
 */
class AttachMenuTest {

    private fun groups(
        assistantChat: Boolean = false,
        assistantPictures: Boolean = false,
        hasCamera: Boolean = true,
        recordVoice: Boolean = true,
        recordVideo: Boolean = true,
        poll: Boolean = false,
    ) = AttachMenu.groups(
        assistantChat = assistantChat,
        assistantPictures = assistantPictures,
        hasCamera = hasCamera,
        recordVoice = recordVoice,
        recordVideo = recordVideo,
        poll = poll,
    )

    @Test
    fun theFamilyChatHasEverythingInThreeGroups() {
        assertThat(groups(poll = true)).containsExactly(
            listOf(Item.PHOTO_OR_VIDEO, Item.CAMERA, Item.FILE, Item.PASTE),
            listOf(Item.RECORD_VOICE, Item.RECORD_VIDEO),
            listOf(Item.LOCATION, Item.POLL),
        ).inOrder()
    }

    @Test
    fun aDirectChatHasNoPoll() {
        assertThat(groups(poll = false)).containsExactly(
            listOf(Item.PHOTO_OR_VIDEO, Item.CAMERA, Item.FILE, Item.PASTE),
            listOf(Item.RECORD_VOICE, Item.RECORD_VIDEO),
            listOf(Item.LOCATION),
        ).inOrder()
    }

    @Test
    fun withoutRoundVideoTheRecordingGroupIsVoiceAlone() {
        assertThat(groups(recordVideo = false)[1]).containsExactly(Item.RECORD_VOICE)
    }

    @Test
    fun withoutACameraThereIsNoCameraItem() {
        assertThat(groups(hasCamera = false).first())
            .containsExactly(Item.PHOTO_OR_VIDEO, Item.FILE, Item.PASTE).inOrder()
        assertThat(groups(assistantChat = true, assistantPictures = true, hasCamera = false).first())
            .containsExactly(Item.SHOW_ASSISTANT_PICTURE, Item.FILE, Item.PASTE).inOrder()
    }

    /** Never both picture lines; a direct "Take photo" rather than a page of one. */
    @Test
    fun theAssistantChatWithPicturesShowsTheAssistantAPhotoInstead() {
        assertThat(groups(assistantChat = true, assistantPictures = true, poll = true)).containsExactly(
            listOf(Item.SHOW_ASSISTANT_PICTURE, Item.TAKE_PHOTO, Item.FILE, Item.PASTE),
            listOf(Item.LOCATION),
        ).inOrder()
    }

    /** Neither picture line, no camera, and no recording group — so no empty divider. */
    @Test
    fun theAssistantChatWithoutPicturesHasNoPictureDoorAndNoCamera() {
        assertThat(groups(assistantChat = true, assistantPictures = false)).containsExactly(
            listOf(Item.FILE, Item.PASTE),
            listOf(Item.LOCATION),
        ).inOrder()
    }

    /** Even if the caller forgot: the assistant chat records nothing and polls nothing. */
    @Test
    fun theAssistantChatNeverRecordsOrPolls() {
        val all = groups(assistantChat = true, recordVoice = true, recordVideo = true, poll = true).flatten()
        assertThat(all).containsNoneOf(Item.RECORD_VOICE, Item.RECORD_VIDEO, Item.POLL)
    }

    /** A thread: nothing to record, so the recording group is gone, not empty. */
    @Test
    fun anEmptyRecordingGroupLeavesNoGroupBehind() {
        val g = groups(recordVoice = false, recordVideo = false)
        assertThat(g).hasSize(2)
        assertThat(g.none { it.isEmpty() }).isTrue()
    }

    @Test
    fun cameraOpensTakePhotoThenTakeVideo() {
        assertThat(AttachMenu.cameraChoices).containsExactly(Item.TAKE_PHOTO, Item.TAKE_VIDEO).inOrder()
    }

    /** "Ask for a picture" is its own button now (#78), never a menu line. */
    @Test
    fun theMenuNeverHoldsTheCameraPageItemsOrADrawLine() {
        for (assistant in listOf(false, true)) for (pictures in listOf(false, true)) {
            val all = groups(assistantChat = assistant, assistantPictures = pictures, poll = true).flatten()
            assertThat(all).doesNotContain(Item.TAKE_VIDEO)
            assertThat(all).containsNoDuplicates()
        }
    }

    // --- The words, in every language (lint's MissingTranslation is not a gate) ---------

    private val locales = listOf(
        "values", "values-de", "values-es", "values-fr", "values-ja",
        "values-ru", "values-sr", "values-b+sr+Latn", "values-zh-rCN",
    )

    private val keys = listOf(
        "s_attach_a_photo_video_or_file", "s_photo_or_video", "s_show_the_assistant_a_picture",
        "s_camera", "s_back", "s_take_photo", "s_take_video", "s_file", "s_paste",
        "s_record_voice_message", "s_record_video_message", "s_share_your_location", "s_poll",
        "s_ask_for_a_picture",
    )

    private fun strings(locale: String): Map<String, String> {
        val file = listOf(
            File("src/main/res/$locale/strings.xml"),
            File("app/src/main/res/$locale/strings.xml"),
        ).first { it.exists() }
        val doc = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(file)
        val nodes = doc.getElementsByTagName("string")
        return (0 until nodes.length).associate { i ->
            val node = nodes.item(i)
            node.attributes.getNamedItem("name").nodeValue to node.textContent
        }
    }

    @Test
    fun everyMenuWordIsInEveryLanguage() {
        for (locale in locales) {
            val strings = strings(locale)
            for (key in keys) {
                assertWithMessage("$locale/$key").that(strings[key].isNullOrBlank()).isFalse()
            }
            // The two-line assistant item is one line now (#78), everywhere.
            assertWithMessage("$locale still has the note")
                .that(strings).doesNotContainKey("s_show_the_assistant_a_picture_note")
        }
        assertThat(strings("values")["s_camera"]).isEqualTo("Camera")
        // A translation that is just English again is a translation nobody did.
        assertThat(strings("values-de")["s_camera"]).isEqualTo("Kamera")
    }
}
