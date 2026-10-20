/*
 * CodecMemoTest.kt
 * Family Connect (Android)
 *
 * The stored-JSON codecs remember what they parsed (util/LruMemo.kt): an
 * equal column text hands back the SAME value — not merely an equal one —
 * without parsing again, and what they answer is exactly what a fresh parse
 * would have. Unreadable text still answers as it always did.
 */

package me.nettrash.familyconnect.data.net.dto

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class CodecMemoTest {

    private fun photo(id: Long) = AttachmentDto(
        id = id,
        kind = "photo",
        mime = "image/jpeg",
        size = 10,
        width = 4,
        height = 3,
        durationMs = null,
        hasPreview = true,
    )

    @Test
    fun equalAttachmentTextHandsBackTheSameParsedList() {
        val stored = AttachmentsCodec.encode(listOf(photo(1), photo(2)))
        // A different String instance with the same text — what each Room
        // emission hands the item builder for an unchanged row.
        val again = String(stored.toCharArray())
        val first = AttachmentsCodec.decode(stored)
        val second = AttachmentsCodec.decode(again)
        assertThat(first).isEqualTo(listOf(photo(1), photo(2)))
        assertThat(second).isSameInstanceAs(first)
    }

    @Test
    fun attachmentsAnswerExactlyWhatTheyDidForNullAndUnreadableText() {
        assertThat(AttachmentsCodec.decode(null)).isNull()
        assertThat(AttachmentsCodec.decode("not json")).isNull()
        assertThat(AttachmentsCodec.decode("not json")).isNull()
        assertThat(AttachmentsCodec.decode("[]")).isEmpty()
    }

    @Test
    fun reactionsAreRememberedByTheirText() {
        val stored = ReactionsCodec.encode(listOf(ReactionDto(userId = 7, emoji = "❤️")))
        val first = ReactionsCodec.decode(stored)
        assertThat(first).containsExactly(ReactionDto(userId = 7, emoji = "❤️"))
        assertThat(ReactionsCodec.decode(String(stored.toCharArray()))).isSameInstanceAs(first)
        assertThat(ReactionsCodec.decode(null)).isEmpty()
        assertThat(ReactionsCodec.decode("{broken")).isEmpty()
    }

    @Test
    fun pollsAreRememberedByTheirText() {
        val poll = PollDto(
            pollSeq = 3,
            closed = false,
            options = listOf(PollOptionDto(id = 1, text = "Yes", votes = listOf(7L))),
        )
        val stored = PollCodec.encode(poll)
        val first = PollCodec.decode(stored)
        assertThat(first).isEqualTo(poll)
        assertThat(PollCodec.decode(String(stored.toCharArray()))).isSameInstanceAs(first)
        assertThat(PollCodec.decode(null)).isNull()
        assertThat(PollCodec.decode("nope")).isNull()
    }

    @Test
    fun mentionsAreRememberedByTheirText() {
        val stored = MentionsCodec.encode(listOf(MentionDto(userId = 9, name = "Ann")))
        val first = MentionsCodec.decode(stored)
        assertThat(first).containsExactly(MentionDto(userId = 9, name = "Ann"))
        assertThat(MentionsCodec.decode(String(stored.toCharArray()))).isSameInstanceAs(first)
        assertThat(MentionsCodec.decode(null)).isEmpty()
    }
}
