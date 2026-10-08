/*
 * TranscriptRules.kt
 * Family Connect (Android)
 *
 * Whether a recording is offered "Show text" (docs/protocol.md,
 * "Transcripts on request"). Arithmetic, free of Compose and Android, so a
 * plain JUnit test can pin it — and one function, so the chat and the
 * thread cannot disagree about it.
 *
 * It MIRRORS `allowed()` in server/src/handlers_transcript.rs plus the
 * stored-copy checks before it. A disagreement is either a button that can
 * only ever say "Not available for this message", or one missing where the
 * server would have answered. What the client cannot know — whether ANOTHER
 * member has agreed to the assistant — is left to the server, whose
 * `transcript_not_allowed` is drawn as "Not available for this message".
 */

package me.nettrash.familyconnect.ui.chat

import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import java.util.Locale

object TranscriptRules {

    /**
     * The stored types the server sends as they are: AAC in MPEG-4, MP3 and
     * WAV. Ogg, video and anything else need the sound taken out on the
     * device first ([Route.SUPPLIED]).
     */
    val STORED_TYPES: Set<String> = setOf("audio/mp4", "audio/m4a", "audio/mpeg", "audio/wav")

    /**
     * May this member ask about a recording on this message at all?
     *
     * Their own message, in any chat they are in (direct chats and their
     * own assistant chat included). Another member's only in the family
     * chat — its threads are in the family chat too — and only while the
     * owner's `ai_transcripts` is on. Never another member's in a direct
     * chat, and never the assistant's own messages.
     */
    fun mayAsk(
        chatKind: String?,
        senderId: Long,
        myUserId: Long?,
        assistantUserId: Long?,
        familyAllowsOthers: Boolean,
    ): Boolean = when {
        myUserId == null -> false
        assistantUserId != null && senderId == assistantUserId -> false
        senderId == myUserId -> true
        chatKind == "family" -> familyAllowsOthers
        else -> false
    }

    /**
     * Will the server send its STORED copy? A voice note or audio file of a
     * type in [STORED_TYPES], no larger than `transcribe_max_bytes`.
     */
    fun storedCopyQualifies(kind: String, mime: String, size: Long, maxBytes: Long): Boolean =
        kind == AttachmentDto.KIND_AUDIO &&
            baseType(mime) in STORED_TYPES &&
            maxBytes > 0 &&
            size in 1..maxBytes

    /** Where the sound of a recording comes from. */
    enum class Route {
        /** The server sends its own stored copy: no body. */
        STORED,

        /** This device takes the sound out of the file and sends it (TranscriptSound). */
        SUPPLIED,
    }

    /**
     * [Route.STORED] when [storedCopyQualifies], otherwise [Route.SUPPLIED]:
     * a video, an Ogg file, a type outside the list, one over the ceiling.
     */
    fun route(kind: String, mime: String, size: Long, maxBytes: Long): Route =
        if (storedCopyQualifies(kind, mime, size, maxBytes)) Route.STORED else Route.SUPPLIED

    /**
     * Is this an attachment a transcript can be asked about at all — a voice
     * note, an audio file or a video, with bytes, on a server that states a
     * ceiling?
     *
     * Deliberately NOT a question about this device: whether it can take
     * the sound out, and whether the sound will fit, is only known once it
     * holds the file (the server types a video `video/mp4`, never by its
     * sound). So the action is offered, and a device that cannot is answered
     * "Not available for this message" — the same line, every time
     * (TranscriptSound's file comment).
     */
    fun askable(kind: String, size: Long, maxBytes: Long): Boolean =
        (kind == AttachmentDto.KIND_AUDIO || kind == AttachmentDto.KIND_VIDEO) &&
            maxBytes > 0 &&
            size > 0

    /** `audio/MP4; codecs=…` → `audio/mp4`. */
    private fun baseType(mime: String): String =
        mime.substringBefore(';').trim().lowercase(Locale.ROOT)

    /**
     * The whole question: is "Show text" drawn under this attachment?
     *
     * [serverTranscribes] is `assistant.transcribe`; [processor] must be
     * nameable, or this client could not ask the consent the request needs;
     * [messageServerId] must exist, because a message still in the outbox
     * has nothing on the server to ask about.
     */
    fun offersShowText(
        serverTranscribes: Boolean,
        maxBytes: Long,
        processor: String?,
        chatKind: String?,
        messageServerId: Long?,
        senderId: Long,
        myUserId: Long?,
        assistantUserId: Long?,
        familyAllowsOthers: Boolean,
        attachment: AttachmentDto,
    ): Boolean =
        serverTranscribes &&
            AssistantConsent.isAvailable(processor) &&
            messageServerId != null &&
            mayAsk(chatKind, senderId, myUserId, assistantUserId, familyAllowsOthers) &&
            askable(attachment.kind, attachment.size, maxBytes)

    /**
     * The long-press menu's transcript item (#79): the line's own decision,
     * as one action. [held] is null with no text on this device, true while
     * it shows, false while it is folded away.
     */
    fun menuAction(
        held: Boolean?,
        status: TranscriptRequests.Status?,
        offered: Boolean,
        canAsk: Boolean,
        reveal: () -> Unit,
        hide: () -> Unit,
        ask: () -> Unit,
    ): TranscriptMenuAction? = when {
        // Being fetched: nothing to do until it lands.
        status == TranscriptRequests.Status.LOADING -> null
        held == true && status == null -> TranscriptMenuAction(R.string.s_transcript_hide, hide)
        held != null -> TranscriptMenuAction(R.string.s_transcript_show, reveal)
        // A refusal, or what this device could not do with the file, is said
        // under the player; asking again from the menu would only earn it again.
        status != null && status != TranscriptRequests.Status.FAILED -> null
        offered && canAsk -> TranscriptMenuAction(R.string.s_transcript_show, ask)
        else -> null
    }
}
