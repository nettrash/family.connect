/*
 * GreetingWeatherCreditTest.kt
 * Family Connect (Android)
 *
 * A daily greeting that used the weather ends, after one blank line, in
 * the credit line a lookup answer carries — "[Weather data by
 * Open-Meteo.com](https://open-meteo.com/)", in the greeting's language
 * (docs/protocol.md, "Today's weather, for places the owner chose" — "The
 * credit, and the filter"). Plain markdown in the body: no new field, so
 * this client needs nothing new to draw it — only for the link to come out
 * tappable, and for the bubble to fetch no preview card for it, exactly as
 * under a lookup answer.
 *
 * The nine credit spellings are copied from `server/src/lookups.rs`.
 * Robolectric because the link detector goes through Linkify.
 */

package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertThat
import com.google.common.truth.Truth.assertWithMessage
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class GreetingWeatherCreditTest {

    private val credits = listOf(
        "Weather data by Open-Meteo.com",
        "Wetterdaten von Open-Meteo.com",
        "Datos meteorológicos de Open-Meteo.com",
        "Données météo par Open-Meteo.com",
        "気象データ: Open-Meteo.com",
        "Данные о погоде: Open-Meteo.com",
        "Подаци о времену: Open-Meteo.com",
        "Podaci o vremenu: Open-Meteo.com",
        "天气数据：Open-Meteo.com",
    )

    private fun greeting(credit: String) =
        "Доброе утро, семья! ☀️ Сегодня в Москве, Россия, до +12 °C и сухо, " +
            "а в Белграде, Сербия, вероятен дождь — зонтик пригодится. " +
            "Овнам и Ракам сегодня особенно удаётся всё новое.\n\n" +
            "[$credit](https://open-meteo.com/)"

    @Test
    fun `the credit renders as one tappable link to Open-Meteo, in every language`() {
        for (credit in credits) {
            val block = MessageMarkdown.blocks(greeting(credit)).single() as MessageMarkdown.Block.Text
            val links = MessageLinks.mergeSpans(block.rendered.links, MessageLinks.linkSpans(block.rendered.text))
            assertWithMessage(credit).that(links.map { it.url }).containsExactly("https://open-meteo.com/")
            val link = links.single()
            // The link covers the credit words, and the markdown is gone.
            assertWithMessage(credit).that(block.rendered.text.substring(link.start, link.end)).isEqualTo(credit)
            assertWithMessage(credit).that(block.rendered.text).endsWith("\n\n$credit")
            assertWithMessage(credit).that(block.rendered.text).doesNotContain("](")
        }
    }

    @Test
    fun `a greeting that used the weather fetches no preview card`() {
        for (credit in credits) {
            val body = greeting(credit)
            assertWithMessage(credit).that(LookupFooter.isPresent(body)).isTrue()
            assertWithMessage(credit).that(
                LookupFooter.suppressesPreview(fromAssistant = true, body = body, isStreaming = false, failed = false),
            ).isTrue()
        }
    }

    @Test
    fun `a greeting with no weather is exactly what it was`() {
        val body = "Доброе утро, семья! Овнам и Ракам сегодня особенно удаётся всё новое."
        val block = MessageMarkdown.blocks(body).single() as MessageMarkdown.Block.Text
        assertThat(block.rendered.links).isEmpty()
        assertThat(block.rendered.text).isEqualTo(body)
        assertThat(LookupFooter.isPresent(body)).isFalse()
    }
}
