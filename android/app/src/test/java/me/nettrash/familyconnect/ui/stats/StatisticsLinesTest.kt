/*
 * StatisticsLinesTest.kt
 * Family Connect (Android)
 *
 * The assistant's share of Family statistics (docs/protocol.md, "Family
 * statistics"): questions and tokens, the pictures it made, and the
 * recordings turned into text with their length — billed by audio length,
 * not tokens, and no longer counted as questions. A family that only ever
 * asked for transcripts must still see an assistant section, or what it
 * costs reads as free (migration 0049's reason). iOS, the web and Windows
 * draw exactly these rows.
 */

package me.nettrash.familyconnect.ui.stats

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import com.google.common.truth.Truth.assertThat
import kotlinx.serialization.json.Json
import me.nettrash.familyconnect.data.net.dto.AiStatsDto
import me.nettrash.familyconnect.data.net.dto.AttachmentStatsDto
import me.nettrash.familyconnect.data.net.dto.MemberStatsDto
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
class StatisticsLinesTest {

    private val context: Context = ApplicationProvider.getApplicationContext()
    private val json = Json { ignoreUnknownKeys = true }

    @Test
    fun `the assistant's pictures and transcripts are read, and zero from an older server`() {
        val read = json.decodeFromString<AiStatsDto>(
            """{"questions": 1, "prompt_tokens": 10, "completion_tokens": 5,
               "images": 2, "transcripts": 3, "transcript_duration_ms": 222400}""",
        )
        assertThat(read).isEqualTo(
            AiStatsDto(
                questions = 1, promptTokens = 10, completionTokens = 5,
                images = 2, transcripts = 3, transcriptDurationMs = 222_400,
            ),
        )
        val older = json.decodeFromString<AiStatsDto>("""{"questions": 1, "prompt_tokens": 0, "completion_tokens": 0}""")
        assertThat(older.images).isEqualTo(0)
        assertThat(older.transcripts).isEqualTo(0)
        assertThat(older.transcriptDurationMs).isEqualTo(0L)
    }

    @Test
    fun `a family that only asked for transcripts still sees the assistant, with the recording time`() {
        val rows = StatisticsLines.assistantRows(AiStatsDto(transcripts = 2, transcriptDurationMs = 222_400), context)

        assertThat(rows).containsExactly(
            "Questions" to "0",
            "Tokens" to "0",
            "Recordings as text" to "2",
            "Recording time" to "3:42",
        ).inOrder()
    }

    @Test
    fun `pictures are counted, and nothing at all is no section`() {
        assertThat(StatisticsLines.assistantRows(AiStatsDto(images = 4), context))
            .contains("Pictures" to "4")
        assertThat(StatisticsLines.assistantRows(AiStatsDto(), context)).isEmpty()
        assertThat(
            StatisticsLines.assistantRows(AiStatsDto(questions = 3, promptTokens = 7, completionTokens = 2), context),
        ).containsExactly("Questions" to "3", "Tokens" to "9").inOrder()
    }

    @Test
    fun `web searches are read, and zero from an older server`() {
        val read = json.decodeFromString<AiStatsDto>(
            """{"questions": 4, "prompt_tokens": 10, "completion_tokens": 5, "searches": 3}""",
        )
        assertThat(read.searches).isEqualTo(3)
        val older = json.decodeFromString<AiStatsDto>("""{"questions": 1, "prompt_tokens": 0, "completion_tokens": 0}""")
        assertThat(older.searches).isEqualTo(0)
    }

    @Test
    fun `web searches get their own row, after pictures, and none when there were none`() {
        val rows = StatisticsLines.assistantRows(
            AiStatsDto(questions = 4, promptTokens = 7, completionTokens = 2, images = 1, searches = 3),
            context,
        )
        assertThat(rows).containsExactly(
            "Questions" to "4",
            "Tokens" to "9",
            "Pictures" to "1",
            "Web searches" to "3",
        ).inOrder()
        assertThat(StatisticsLines.assistantRows(AiStatsDto(questions = 4), context).map { it.first })
            .doesNotContain("Web searches")
    }

    @Test
    fun `a member's web searches are a counted plural`() {
        fun line(searches: Int) = StatisticsLines.summaryFor(
            MemberStatsDto(userId = 7, displayName = "Anna", ai = AiStatsDto(questions = 2, searches = searches)),
            context,
        )
        assertThat(line(1)).isEqualTo("2 questions to the assistant · 1 web search")
        assertThat(line(3)).isEqualTo("2 questions to the assistant · 3 web searches")
        assertThat(line(0)).isEqualTo("2 questions to the assistant")
    }

    @Test
    @Config(qualifiers = "ru")
    fun `in Russian the count takes its own form`() {
        fun line(searches: Int) = StatisticsLines.summaryFor(
            MemberStatsDto(userId = 7, displayName = "Анна", ai = AiStatsDto(searches = searches)),
            ApplicationProvider.getApplicationContext(),
        )
        assertThat(line(1)).isEqualTo("1 поиск в интернете")
        assertThat(line(3)).isEqualTo("3 поиска в интернете")
        assertThat(line(5)).isEqualTo("5 поисков в интернете")
        assertThat(line(21)).isEqualTo("21 поиск в интернете")
    }

    @Test
    fun `recording time is a duration, past an hour too`() {
        assertThat(StatisticsLines.recordingTime(0)).isEqualTo("0:00")
        assertThat(StatisticsLines.recordingTime(-5)).isEqualTo("0:00")
        assertThat(StatisticsLines.recordingTime(222_400)).isEqualTo("3:42")
        assertThat(StatisticsLines.recordingTime(3_822_000)).isEqualTo("1:03:42")
    }

    @Test
    fun `a member's line names their pictures and recordings as text`() {
        val member = MemberStatsDto(
            userId = 7, displayName = "Olive", messages = 3,
            ai = AiStatsDto(questions = 1, images = 2, transcripts = 5),
        )
        assertThat(StatisticsLines.summaryFor(member, context)).isEqualTo(
            "1 question to the assistant · 2 pictures from the assistant · 5 recordings as text",
        )
        assertThat(StatisticsLines.summaryFor(member.copy(ai = AiStatsDto()), context)).isEqualTo("Words only")
    }

    @Test
    fun `one of anything is said in the singular, and more in the plural`() {
        val one = MemberStatsDto(
            userId = 7, displayName = "Olive",
            attachments = AttachmentStatsDto(count = 1, bytes = 0),
            ai = AiStatsDto(questions = 1, images = 1, transcripts = 1),
        )
        assertThat(StatisticsLines.summaryFor(one, context)).isEqualTo(
            "1 attachment, 0 B · 1 question to the assistant · 1 picture from the assistant · 1 recording as text",
        )
        val more = one.copy(
            attachments = AttachmentStatsDto(count = 3, bytes = 0),
            ai = AiStatsDto(questions = 2, images = 4, transcripts = 6),
        )
        assertThat(StatisticsLines.summaryFor(more, context)).isEqualTo(
            "3 attachments, 0 B · 2 questions to the assistant · 4 pictures from the assistant · 6 recordings as text",
        )
    }

    @Test
    @Config(qualifiers = "ru")
    fun `Russian counts take all three of its forms`() {
        fun line(n: Int) = StatisticsLines.summaryFor(
            MemberStatsDto(userId = 7, displayName = "Olive", ai = AiStatsDto(images = n)),
            context,
        )
        assertThat(line(1)).isEqualTo("1 картинка от ассистента")
        assertThat(line(3)).isEqualTo("3 картинки от ассистента")
        assertThat(line(5)).isEqualTo("5 картинок от ассистента")
        assertThat(line(21)).isEqualTo("21 картинка от ассистента")
    }
}
