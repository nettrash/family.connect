/*
 * TranscriptRulesTest.kt
 * Family Connect (Android)
 *
 * Whether "Show text" is offered under a recording (docs/protocol.md,
 * "Transcripts on request"), pinned against the server's `allowed()` and
 * its stored-copy checks. A disagreement is a button that can only fail,
 * or one missing where the server would have answered.
 */

package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.net.dto.AssistantDto
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import org.junit.Test

class TranscriptRulesTest {

    private companion object {
        const val ME = 7L
        const val ANNA = 8L
        const val ASSISTANT = 1L
        const val MAX = AssistantDto.DEFAULT_TRANSCRIBE_MAX_BYTES
    }

    private fun mayAsk(chatKind: String?, sender: Long, familyAllowsOthers: Boolean) =
        TranscriptRules.mayAsk(
            chatKind = chatKind,
            senderId = sender,
            myUserId = ME,
            assistantUserId = ASSISTANT,
            familyAllowsOthers = familyAllowsOthers,
        )

    // -- Whose recording, in which chat ------------------------------------------

    @Test
    fun `my own recording may be asked about in every kind of chat`() {
        for (kind in listOf("family", "direct", "ai")) {
            assertThat(mayAsk(kind, ME, familyAllowsOthers = false)).isTrue()
        }
    }

    @Test
    fun `another member's recording in the family chat follows the owner's switch`() {
        assertThat(mayAsk("family", ANNA, familyAllowsOthers = false)).isFalse()
        assertThat(mayAsk("family", ANNA, familyAllowsOthers = true)).isTrue()
    }

    @Test
    fun `another member's recording in a direct chat is never asked about`() {
        // Not even with the switch on: nothing reads a direct chat.
        assertThat(mayAsk("direct", ANNA, familyAllowsOthers = true)).isFalse()
        assertThat(mayAsk("direct", ANNA, familyAllowsOthers = false)).isFalse()
    }

    @Test
    fun `the assistant's own messages are never asked about`() {
        assertThat(mayAsk("ai", ASSISTANT, familyAllowsOthers = true)).isFalse()
        assertThat(mayAsk("family", ASSISTANT, familyAllowsOthers = true)).isFalse()
    }

    @Test
    fun `an unknown chat kind or an unknown me offers nothing`() {
        assertThat(mayAsk(null, ANNA, familyAllowsOthers = true)).isFalse()
        assertThat(mayAsk("group", ANNA, familyAllowsOthers = true)).isFalse()
        assertThat(
            TranscriptRules.mayAsk("family", ME, myUserId = null, assistantUserId = ASSISTANT, familyAllowsOthers = true),
        ).isFalse()
    }

    // -- What the server will send from its own copy -----------------------------

    @Test
    fun `the four stored types qualify, in any case and with parameters`() {
        for (mime in listOf("audio/mp4", "audio/m4a", "audio/mpeg", "audio/wav", "AUDIO/MP4", "audio/mp4; codecs=mp4a.40.2")) {
            assertThat(TranscriptRules.storedCopyQualifies("audio", mime, 1_000, MAX)).isTrue()
        }
    }

    @Test
    fun `ogg, video and other kinds are not sent from the stored copy`() {
        assertThat(TranscriptRules.storedCopyQualifies("audio", "audio/ogg", 1_000, MAX)).isFalse()
        assertThat(TranscriptRules.storedCopyQualifies("audio", "audio/webm", 1_000, MAX)).isFalse()
        assertThat(TranscriptRules.storedCopyQualifies("video", "video/mp4", 1_000, MAX)).isFalse()
        // A video's type on an audio kind, or an audio type on a file: the
        // server checks the KIND first.
        assertThat(TranscriptRules.storedCopyQualifies("file", "audio/mpeg", 1_000, MAX)).isFalse()
        assertThat(TranscriptRules.storedCopyQualifies("photo", "audio/mp4", 1_000, MAX)).isFalse()
    }

    @Test
    fun `the ceiling is inclusive, and nothing qualifies without one`() {
        assertThat(TranscriptRules.storedCopyQualifies("audio", "audio/mp4", MAX, MAX)).isTrue()
        assertThat(TranscriptRules.storedCopyQualifies("audio", "audio/mp4", MAX + 1, MAX)).isFalse()
        assertThat(TranscriptRules.storedCopyQualifies("audio", "audio/mp4", 1_000, 0)).isFalse()
        assertThat(TranscriptRules.storedCopyQualifies("audio", "audio/mp4", 0, MAX)).isFalse()
    }

    // -- The whole question ------------------------------------------------------

    private val voiceNote = AttachmentDto(id = 40, kind = "audio", mime = "audio/mp4", size = 48_000, durationMs = 6_000)

    private fun offers(
        serverTranscribes: Boolean = true,
        processor: String? = "Microsoft — Azure OpenAI",
        chatKind: String? = "family",
        messageServerId: Long? = 500L,
        sender: Long = ME,
        familyAllowsOthers: Boolean = false,
        attachment: AttachmentDto = voiceNote,
        maxBytes: Long = MAX,
    ) = TranscriptRules.offersShowText(
        serverTranscribes = serverTranscribes,
        maxBytes = maxBytes,
        processor = processor,
        chatKind = chatKind,
        messageServerId = messageServerId,
        senderId = sender,
        myUserId = ME,
        assistantUserId = ASSISTANT,
        familyAllowsOthers = familyAllowsOthers,
        attachment = attachment,
    )

    @Test
    fun `my voice note on a transcribing server is offered`() {
        assertThat(offers()).isTrue()
    }

    @Test
    fun `nothing is offered on a server that cannot transcribe`() {
        assertThat(offers(serverTranscribes = false)).isFalse()
    }

    @Test
    fun `nothing is offered where the server names nobody to send the sound to`() {
        // The consent the request needs could not be asked.
        assertThat(offers(processor = null)).isFalse()
        assertThat(offers(processor = "  ")).isFalse()
    }

    @Test
    fun `a message still in the outbox has nothing on the server to ask about`() {
        assertThat(offers(messageServerId = null)).isFalse()
    }

    @Test
    fun `another member's voice note follows the rule and the switch`() {
        assertThat(offers(sender = ANNA)).isFalse()
        assertThat(offers(sender = ANNA, familyAllowsOthers = true)).isTrue()
        assertThat(offers(sender = ANNA, familyAllowsOthers = true, chatKind = "direct")).isFalse()
    }

    // -- Phase 3: what this device supplies --------------------------------------------

    private val clip = AttachmentDto(id = 70, kind = "video", mime = "video/mp4", size = 40_000_000, durationMs = 90_000)

    @Test
    fun `an oversized or ogg recording is offered, its sound to be supplied`() {
        assertThat(offers(attachment = voiceNote.copy(size = MAX + 1))).isTrue()
        assertThat(offers(attachment = voiceNote.copy(mime = "audio/ogg"))).isTrue()
        assertThat(offers(attachment = voiceNote.copy(mime = "audio/webm"))).isTrue()
    }

    @Test
    fun `a video is offered under the same rule as a voice note`() {
        assertThat(offers(attachment = clip)).isTrue()
        assertThat(offers(attachment = clip, sender = ANNA)).isFalse()
        assertThat(offers(attachment = clip, sender = ANNA, familyAllowsOthers = true)).isTrue()
        assertThat(offers(attachment = clip, sender = ANNA, familyAllowsOthers = true, chatKind = "direct")).isFalse()
        assertThat(offers(attachment = clip, sender = ASSISTANT)).isFalse()
        assertThat(offers(attachment = clip, serverTranscribes = false)).isFalse()
    }

    @Test
    fun `photos, files and locations are never offered`() {
        for (kind in listOf("photo", "file", "location")) {
            assertThat(offers(attachment = clip.copy(kind = kind, mime = "audio/mp4"))).isFalse()
        }
    }

    /**
     * The decision for a device that cannot take the sound out, or whose
     * sound will not fit: the action is NOT hidden — that is only known once
     * the device holds the file — and the answer is "Not available for this
     * message" (TranscriptRepositoryTest, TranscriptSoundPlanTest).
     */
    @Test
    fun `whether this device can extract does not hide the action`() {
        assertThat(TranscriptRules.askable("video", size = 1, maxBytes = MAX)).isTrue()
        assertThat(TranscriptRules.askable("video", size = 10 * MAX, maxBytes = MAX)).isTrue()
        assertThat(TranscriptRules.askable("audio", size = 10 * MAX, maxBytes = MAX)).isTrue()
        // Nothing to take the sound out of, or no ceiling to keep it under.
        assertThat(TranscriptRules.askable("video", size = 0, maxBytes = MAX)).isFalse()
        assertThat(TranscriptRules.askable("video", size = 1_000, maxBytes = 0)).isFalse()
    }

    @Test
    fun `the stored copy where it qualifies, supplied sound everywhere else`() {
        assertThat(TranscriptRules.route("audio", "audio/mp4", 48_000, MAX)).isEqualTo(TranscriptRules.Route.STORED)
        assertThat(TranscriptRules.route("audio", "audio/mpeg", MAX, MAX)).isEqualTo(TranscriptRules.Route.STORED)
        assertThat(TranscriptRules.route("audio", "audio/mp4", MAX + 1, MAX)).isEqualTo(TranscriptRules.Route.SUPPLIED)
        assertThat(TranscriptRules.route("audio", "audio/ogg", 48_000, MAX)).isEqualTo(TranscriptRules.Route.SUPPLIED)
        assertThat(TranscriptRules.route("audio", "audio/aac", 48_000, MAX)).isEqualTo(TranscriptRules.Route.SUPPLIED)
        assertThat(TranscriptRules.route("video", "video/mp4", 48_000, MAX)).isEqualTo(TranscriptRules.Route.SUPPLIED)
        assertThat(TranscriptRules.route("video", "video/quicktime", 48_000, MAX)).isEqualTo(TranscriptRules.Route.SUPPLIED)
    }
}
