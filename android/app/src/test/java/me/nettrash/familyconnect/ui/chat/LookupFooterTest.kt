/*
 * LookupFooterTest.kt
 * Family Connect (Android)
 *
 * The sources footer the server appends to an answer that looked
 * something up (docs/protocol.md, "Looking things up" — "How sources are
 * shown"): its links are tappable in the bubble like any markdown link,
 * and they are NOT turned into a link-preview card — nor is anything the
 * assistant is still writing, or stopped writing (decision 7).
 *
 * The footers below are the server's own shapes, copied from
 * `server/src/lookups.rs` (`finish_answer`, `footer`, and its tests).
 * Robolectric because the preview rule goes through Linkify.
 */

package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertThat
import com.google.common.truth.Truth.assertWithMessage
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class LookupFooterTest {

    private val credits =
        "[Weather data by Open-Meteo.com](https://open-meteo.com/) · " +
            "Wikipedia, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/) · " +
            "Powered by Brave"

    private val english =
        "Tomorrow in Tromsø: snow showers, around −2 °C.\n\n" +
            "Sources: [Tromsø – Wikipedia](https://en.wikipedia.org/wiki/Troms%C3%B8) · " +
            "[Weather in Tromsø](https://example.org/tromso) · [Third one](https://c.example.net/x?y=1)\n" +
            credits

    private val russian =
        "Завтра в Тромсё снег.\n\n" +
            "Источники: [Тромсё — Википедия](https://ru.wikipedia.org/wiki/%D0%A2%D1%80%D0%BE%D0%BC%D1%81%D1%91)\n" +
            "[Данные о погоде: Open-Meteo.com](https://open-meteo.com/) · " +
            "Википедия, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/)"

    private val japanese =
        "明日のトロムソは雪です。\n\n" +
            "出典: [トロムソ - Wikipedia](https://ja.wikipedia.org/wiki/%E3%83%88%E3%83%AD%E3%83%A0%E3%82%BD)\n" +
            "[気象データ: Open-Meteo.com](https://open-meteo.com/) · " +
            "ウィキペディア, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/)"

    /** SearXNG gets no credit, so its footer is the sources line alone. */
    private val searxngOnly =
        "The match ended 2–1.\n\nSources: [Match report](https://news.example.com/report) · [Live](https://b.example.org/live)"

    /** Weather alone returns no linkable source, so its footer is the credit line alone. */
    private val weatherOnly =
        "Sunny, 18 °C.\n\n[Weather data by Open-Meteo.com](https://open-meteo.com/)"

    /** A Wikipedia title with parentheses: the server percent-encodes them in the URL. */
    private val parenthesised =
        "Mercury is a planet.\n\n" +
            "Sources: [Mercury (planet) – Wikipedia](https://en.wikipedia.org/wiki/Mercury_%28planet%29)\n" +
            "Wikipedia, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/)"

    // -- Recognising it ------------------------------------------------------------

    @Test
    fun `the server's footer is recognised in every shape it comes in`() {
        for (body in listOf(english, russian, japanese, searxngOnly, weatherOnly, parenthesised)) {
            assertWithMessage(body).that(LookupFooter.isPresent(body)).isTrue()
        }
        assertThat(LookupFooter.footer(searxngOnly))
            .isEqualTo("Sources: [Match report](https://news.example.com/report) · [Live](https://b.example.org/live)")
        assertThat(LookupFooter.footer(english)!!.lines().last()).isEqualTo(credits)
    }

    @Test
    fun `an ordinary answer with a link is not a footer`() {
        val bodies = listOf(
            "See https://example.com for more.",
            "Here you go:\n\n[the docs](https://example.com/docs)",
            "Two links:\n\n[a](https://a.example) and [b](https://b.example)",
            "Sources: [a](https://a.example)", // nothing above it
            "Answer.\n\nSources: [a](https://a.example)\nand some prose after it",
            "Answer.\n\nPowered by Brave and friends",
            "Answer.\n\n[Weather](https://not-open-meteo.example/)",
            "Answer.\n\nSources: [a](https://a.example)\n[b](https://b.example)",
            "Answer.\n\n$credits\nSources: [a](https://a.example)", // credits must come last
            "",
        )
        for (body in bodies) {
            assertWithMessage(body).that(LookupFooter.isPresent(body)).isFalse()
        }
    }

    @Test
    fun `a footer must be the whole last paragraph`() {
        assertThat(LookupFooter.isPresent("$english\n\nP.S. one more thing")).isFalse()
        // Trailing whitespace is not a paragraph.
        assertThat(LookupFooter.isPresent("$english\n")).isTrue()
    }

    // -- Tappable --------------------------------------------------------------------

    @Test
    fun `every footer link renders as a tappable link to exactly its URL`() {
        val block = MessageMarkdown.blocks(english).single() as MessageMarkdown.Block.Text
        val urls = MessageLinks.mergeSpans(block.rendered.links, MessageLinks.linkSpans(block.rendered.text))
            .map { it.url }
        assertThat(urls).containsExactly(
            "https://en.wikipedia.org/wiki/Troms%C3%B8",
            "https://example.org/tromso",
            "https://c.example.net/x?y=1",
            "https://open-meteo.com/",
            "https://creativecommons.org/licenses/by-sa/4.0/",
        ).inOrder()
        // The markdown is gone from what the reader sees.
        assertThat(block.rendered.text).contains("Sources: Tromsø – Wikipedia · Weather in Tromsø · Third one")
        assertThat(block.rendered.text).endsWith("Wikipedia, CC BY-SA 4.0 · Powered by Brave")
    }

    @Test
    fun `a percent-encoded parenthesis keeps the Wikipedia link whole`() {
        val block = MessageMarkdown.blocks(parenthesised).single() as MessageMarkdown.Block.Text
        assertThat(block.rendered.links.map { it.url })
            .contains("https://en.wikipedia.org/wiki/Mercury_%28planet%29")
        assertThat(block.rendered.text).contains("Mercury (planet) – Wikipedia")
    }

    // -- Kept out of the preview card --------------------------------------------------

    @Test
    fun `an answer with sources draws no preview card, though its links would otherwise`() {
        val blocks = MessageMarkdown.blocks(english)
        // Without the rule the card would fetch the first source.
        assertThat(MessageLinks.firstDrawnWebLinkUrl(blocks))
            .isEqualTo("https://en.wikipedia.org/wiki/Troms%C3%B8")
        assertThat(
            LookupFooter.suppressesPreview(fromAssistant = true, body = english, isStreaming = false, failed = false),
        ).isTrue()
        for (body in listOf(russian, japanese, searxngOnly, weatherOnly)) {
            assertWithMessage(body).that(
                LookupFooter.suppressesPreview(fromAssistant = true, body = body, isStreaming = false, failed = false),
            ).isTrue()
        }
    }

    @Test
    fun `an assistant answer that looked nothing up keeps its card`() {
        assertThat(
            LookupFooter.suppressesPreview(
                fromAssistant = true,
                body = "The recipe is at https://example.com/cake",
                isStreaming = false,
                failed = false,
            ),
        ).isFalse()
    }

    @Test
    fun `nothing the assistant is still writing, or stopped writing, is fetched`() {
        val partial = "Let me check https://attacker.example/?q=family+secrets"
        assertThat(LookupFooter.suppressesPreview(fromAssistant = true, body = partial, isStreaming = true, failed = false))
            .isTrue()
        assertThat(LookupFooter.suppressesPreview(fromAssistant = true, body = partial, isStreaming = false, failed = true))
            .isTrue()
    }

    @Test
    fun `a member's message keeps its card whatever it looks like`() {
        assertThat(LookupFooter.suppressesPreview(fromAssistant = false, body = english, isStreaming = false, failed = false))
            .isFalse()
        assertThat(LookupFooter.suppressesPreview(fromAssistant = false, body = "x", isStreaming = true, failed = true))
            .isFalse()
    }
}
