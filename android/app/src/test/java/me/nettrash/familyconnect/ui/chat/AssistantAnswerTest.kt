/*
 * AssistantAnswerTest.kt
 * Family Connect (Android)
 *
 * When an empty row IS the assistant's "still working" state, decided from
 * the ROW — because a history page, a cold start and every picture answer
 * populate no live set at all (docs/protocol.md, "How a picture comes
 * back"). The mirror of iOS's AssistantStreamTests cases.
 */

package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.data.db.MessageStatus
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.net.dto.AttachmentsCodec
import me.nettrash.familyconnect.data.net.dto.PollCodec
import me.nettrash.familyconnect.data.net.dto.PollDto
import me.nettrash.familyconnect.data.net.dto.PollOptionDto
import org.junit.Test

class AssistantAnswerTest {

    private companion object {
        const val ME = 7L
        const val ASSISTANT = 99L
    }

    private fun row(
        serverId: Long? = 100L,
        senderId: Long = ASSISTANT,
        body: String = "",
        attachmentsJson: String? = null,
        pollJson: String? = null,
        callOutcome: String? = null,
    ) = MessageEntity(
        clientMsgId = "s$serverId",
        serverId = serverId,
        chatId = 42L,
        senderId = senderId,
        body = body,
        createdAt = 1_000_000L,
        status = MessageStatus.SENT,
        attachmentsJson = attachmentsJson,
        pollJson = pollJson,
        callOutcome = callOutcome,
    )

    /** The whole point: no live set, and the row still says "working". */
    @Test
    fun `an empty assistant row is the working state`() {
        assertThat(
            AssistantAnswer.isAwaited(row(), isAssistantChat = true, assistantUserId = null, myUserId = ME),
        ).isTrue()
    }

    @Test
    fun `a row that carries a picture is an answer that arrived`() {
        val picture = AttachmentsCodec.encode(
            listOf(
                AttachmentDto(
                    id = 77, kind = AttachmentDto.KIND_PHOTO, mime = "image/png",
                    size = 1, width = 512, height = 512, durationMs = null,
                    hasPreview = false, name = null,
                ),
            ),
        )
        assertThat(
            AssistantAnswer.isAwaited(
                row(attachmentsJson = picture),
                isAssistantChat = true, assistantUserId = null, myUserId = ME,
            ),
        ).isFalse()
    }

    @Test
    fun `a row with a body is an answer that arrived`() {
        assertThat(
            AssistantAnswer.isAwaited(
                row(body = "Here you go."),
                isAssistantChat = true, assistantUserId = null, myUserId = ME,
            ),
        ).isFalse()
    }

    @Test
    fun `a poll and a call record are both things a row carries`() {
        val poll = PollCodec.encode(
            PollDto(
                options = listOf(
                    PollOptionDto(id = 1, text = "Pizza", votes = emptyList()),
                    PollOptionDto(id = 2, text = "Pasta", votes = emptyList()),
                ),
                closed = false,
                pollSeq = 1,
            ),
        )
        assertThat(
            AssistantAnswer.isAwaited(
                row(pollJson = poll),
                isAssistantChat = true, assistantUserId = null, myUserId = ME,
            ),
        ).isFalse()
        assertThat(
            AssistantAnswer.isAwaited(
                row(callOutcome = "missed"),
                isAssistantChat = true, assistantUserId = null, myUserId = ME,
            ),
        ).isFalse()
    }

    /** My own empty row is a send in flight, not somebody else's answer. */
    @Test
    fun `my own empty row is never an assistant answer`() {
        assertThat(
            AssistantAnswer.isAwaited(
                row(senderId = ME),
                isAssistantChat = true, assistantUserId = null, myUserId = ME,
            ),
        ).isFalse()
    }

    /**
     * In the FAMILY chat there is no two-participants shortcut: only the
     * assistant's reserved account names it there.
     */
    @Test
    fun `in the family chat only the reserved account is the assistant`() {
        assertThat(
            AssistantAnswer.isAwaited(
                row(senderId = 9L),
                isAssistantChat = false, assistantUserId = ASSISTANT, myUserId = ME,
            ),
        ).isFalse()
        assertThat(
            AssistantAnswer.isAwaited(
                row(senderId = ASSISTANT),
                isAssistantChat = false, assistantUserId = ASSISTANT, myUserId = ME,
            ),
        ).isTrue()
    }

    /**
     * A failure this launch saw outranks the row's own shape: the cursor
     * and the "ask again" line are drawn in the same place, and the newer
     * fact wins.
     */
    @Test
    fun `a failure this launch saw beats the empty row`() {
        assertThat(
            AssistantAnswer.isWorking(
                entity = row(),
                isAssistantChat = true, assistantUserId = null, myUserId = ME,
                streamingIds = emptySet(), failedIds = setOf(100L),
            ),
        ).isFalse()
    }

    /**
     * A text answer PART-WAY through has a body, so the row alone would
     * call it finished — the live set is what keeps the cursor after its
     * last fragment.
     */
    @Test
    fun `a half written text answer is working because the set says so`() {
        val half = row(body = "Sure — the ")
        assertThat(
            AssistantAnswer.isAwaited(half, isAssistantChat = true, assistantUserId = null, myUserId = ME),
        ).isFalse()
        assertThat(
            AssistantAnswer.isWorking(
                entity = half,
                isAssistantChat = true, assistantUserId = null, myUserId = ME,
                streamingIds = setOf(100L), failedIds = emptySet(),
            ),
        ).isTrue()
    }

    /**
     * A row still in flight has no server id, so neither set can name it —
     * and it is mine, so it is a send rather than an answer.
     */
    @Test
    fun `a row with no server id is decided by the row alone`() {
        assertThat(
            AssistantAnswer.isWorking(
                entity = row(serverId = null, senderId = ME),
                isAssistantChat = true, assistantUserId = null, myUserId = ME,
                streamingIds = emptySet(), failedIds = emptySet(),
            ),
        ).isFalse()
    }
}
