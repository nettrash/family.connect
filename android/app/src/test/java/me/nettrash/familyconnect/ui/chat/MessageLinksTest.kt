/*
 * MessageLinksTest.kt
 * Family Connect (Android)
 *
 * Pins the tappable-data layer over message bodies: which categories
 * are detected (web URLs, emails, phone numbers — the same three iOS
 * detects), what URL each category opens (scheme'd web, mailto:,
 * tel:), and that span ranges land on the matched text. The detector
 * itself is Linkify (Robolectric provides the real implementation), so
 * these vectors deliberately use robust, locale-independent shapes
 * rather than pinning the platform's whole grammar.
 */

package me.nettrash.familyconnect.ui.chat

import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.style.TextDecoration
import com.google.common.truth.Truth.assertThat
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class MessageLinksTest {

    /**
     * The single text block a table-free body renders to. `blocks` is the
     * only way into the renderer — a body with a table is a stack of them,
     * and none of these vectors has one.
     */
    private fun render(body: String): MessageMarkdown.Rendered =
        (MessageMarkdown.blocks(body).single() as MessageMarkdown.Block.Text).rendered

    @Test
    fun webUrlsBecomeTappableLinks() {
        val text = "release notes at https://example.com/notes?v=1 today"
        val spans = MessageLinks.linkSpans(text)
        assertThat(spans).hasSize(1)
        assertThat(spans[0].url).isEqualTo("https://example.com/notes?v=1")
        assertThat(text.substring(spans[0].start, spans[0].end))
            .isEqualTo("https://example.com/notes?v=1")
    }

    @Test
    fun schemelessWwwHostsGainAScheme() {
        val spans = MessageLinks.linkSpans("see www.example.com")
        assertThat(spans).hasSize(1)
        assertThat(spans[0].url).isEqualTo("http://www.example.com")
    }

    @Test
    fun emailAddressesBecomeMailtoLinks() {
        val spans = MessageLinks.linkSpans("write to nettrash@nettrash.me please")
        assertThat(spans).hasSize(1)
        assertThat(spans[0].url).isEqualTo("mailto:nettrash@nettrash.me")
    }

    @Test
    fun phoneNumbersBecomeTelLinks() {
        val spans = MessageLinks.linkSpans("call +1 555-123-4567 tonight")
        assertThat(spans).hasSize(1)
        assertThat(spans[0].url).startsWith("tel:")
        // Whatever separators Linkify keeps, the digits must survive.
        assertThat(spans[0].url.filter { it.isDigit() }).isEqualTo("15551234567")
    }

    @Test
    fun plainTextYieldsNoSpans() {
        assertThat(MessageLinks.linkSpans("just words, nothing else. really")).isEmpty()
        assertThat(MessageLinks.linkSpans("😀😀")).isEmpty()
        assertThat(MessageLinks.linkSpans("")).isEmpty()
    }

    @Test
    fun multipleMatchesKeepTheirOrder() {
        val spans = MessageLinks.linkSpans("docs: https://example.com and mail nettrash@nettrash.me")
        assertThat(spans).hasSize(2)
        assertThat(spans[0].url).startsWith("https:")
        assertThat(spans[1].url).startsWith("mailto:")
        assertThat(spans[0].start).isLessThan(spans[1].start)
    }

    @Test
    fun accessibilityLabelsNameTheActionByScheme() {
        val web = "docs at https://example.com ok"
        assertThat(MessageLinks.accessibilityLabel(web, MessageLinks.linkSpans(web)[0]))
            .isEqualTo("Open https://example.com")

        val mail = "write to nettrash@nettrash.me"
        assertThat(MessageLinks.accessibilityLabel(mail, MessageLinks.linkSpans(mail)[0]))
            .isEqualTo("Email nettrash@nettrash.me")

        val phone = "call 555-123-4567 tonight"
        assertThat(MessageLinks.accessibilityLabel(phone, MessageLinks.linkSpans(phone)[0]))
            .isEqualTo("Call 555-123-4567")
    }

    /**
     * The phishing shape, and the reason `mergeSpans` exists: a markdown
     * link whose LABEL is itself a URL renders as that URL's text, which
     * Linkify then matches as a link to the place it NAMES — two spans over
     * the same glyphs with different destinations. The author's own
     * destination is what the message declares, so it is the only one left.
     */
    @Test
    fun aLinkLabelThatLooksLikeAUrlKeepsTheAuthorsDestination() {
        val rendered = render("[https://www.paypal.com](https://evil.example)")
        val merged = MessageLinks.mergeSpans(
            rendered.links,
            MessageLinks.linkSpans(rendered.text),
        )
        assertThat(merged).hasSize(1)
        assertThat(merged[0].url).isEqualTo("https://evil.example")
    }

    /** A detected link that overlaps nothing still survives the merge. */
    @Test
    fun mergeSpansKeepsDetectedLinksOutsideMarkdownOnes() {
        val rendered = render("[menu](https://a.example) and https://b.example")
        val merged = MessageLinks.mergeSpans(
            rendered.links,
            MessageLinks.linkSpans(rendered.text),
        )
        assertThat(merged.map { it.url })
            .containsExactly("https://a.example", "https://b.example")
            .inOrder()
    }

    @Test
    fun styledKeepsTheTextAndStylesExactlyTheSpanRanges() {
        val text = "docs: https://example.com ok"
        val spans = MessageLinks.linkSpans(text)
        val style = SpanStyle(textDecoration = TextDecoration.Underline)
        // `styled` takes the RENDERED AnnotatedString now — markdown runs
        // before it, and every span offset indexes what that produced.
        val styled = MessageLinks.styled(AnnotatedString(text), spans, style)
        assertThat(styled.text).isEqualTo(text)
        assertThat(styled.spanStyles).hasSize(1)
        assertThat(styled.spanStyles[0].start).isEqualTo(spans[0].start)
        assertThat(styled.spanStyles[0].end).isEqualTo(spans[0].end)
        assertThat(styled.spanStyles[0].item).isEqualTo(style)
    }

    // -- what the preview card describes ----------------------------------

    /**
     * A URL that appears ONLY inside a table cell still gets a card.
     *
     * The cell is deliberately not tappable (MessageMarkdown.cell says
     * why), and the block a table renders to carries no link spans at
     * all — so sourcing the preview from the spans alone left the reader
     * looking at a URL with no way whatsoever to reach it. Apple has
     * always scanned the flat render, tables included; this is Android
     * agreeing.
     */
    @Test
    fun aUrlOnlyInsideATableCellStillGetsAPreview() {
        val blocks = MessageMarkdown.blocks(
            "| site | url |\n| --- | --- |\n| menu | https://example.com |",
        )
        assertThat(blocks.single()).isInstanceOf(MessageMarkdown.Block.Table::class.java)
        assertThat(MessageLinks.firstDrawnWebLinkUrl(blocks))
            .isEqualTo("https://example.com")
    }

    /**
     * FIRST means first as DRAWN, in block order — a link above the grid
     * is what the one card describes, not whatever a cell happens to
     * hold. Both platforms have to mean the same message by "first" or
     * the same body previews two different pages.
     */
    @Test
    fun theFirstDrawnLinkWinsAcrossBlocks() {
        val above = MessageMarkdown.blocks(
            "see [menu](https://first.example)\n| a |\n| --- |\n| https://second.example |",
        )
        assertThat(MessageLinks.firstDrawnWebLinkUrl(above)).isEqualTo("https://first.example")

        val below = MessageMarkdown.blocks(
            "| a |\n| --- |\n| https://second.example |\nand https://third.example",
        )
        assertThat(MessageLinks.firstDrawnWebLinkUrl(below)).isEqualTo("https://second.example")
    }

    /**
     * The phishing rule holds here too: where a label and a destination
     * disagree, the card describes the destination the AUTHOR wrote —
     * the page a tap would actually open.
     */
    @Test
    fun theCardDescribesTheAuthorsDestinationNotTheLabel() {
        val blocks = MessageMarkdown.blocks("[https://www.paypal.com](https://evil.example)")
        assertThat(MessageLinks.firstDrawnWebLinkUrl(blocks)).isEqualTo("https://evil.example")
    }

    // -- The tokens the assistant acts on ---------------------------------

    private val mentionStyle = SpanStyle(textDecoration = TextDecoration.Underline)

    /**
     * The marks a body gets when it is its own raw source — the shape of
     * every body with no markdown in it, which is almost all of them.
     */
    private fun styledRanges(body: String, drawMayStartHere: Boolean = true): List<String> =
        markedIn(rendered = body, rawBody = if (drawMayStartHere) body else null)

    /** The marks [rendered] gets when the server is reading [rawBody]. */
    private fun markedIn(rendered: String, rawBody: String?): List<String> {
        val styled = MessageLinks.withMentions(AnnotatedString(rendered), mentionStyle, rawBody)
        return styled.spanStyles
            .filter { it.item == mentionStyle }
            .map { rendered.substring(it.start, it.end) }
    }

    /**
     * Both tokens are highlighted, and only the tokens — the words after
     * `/draw` are the member's own prompt and read as ordinary text.
     */
    @Test
    fun theAssistantsTokensAreMarkedAndNothingElseIs() {
        assertThat(styledRanges("@ai /draw a cat in a hat"))
            .containsExactly("@ai", "/draw")
        assertThat(styledRanges("/draw a cat")).containsExactly("/draw")
        assertThat(styledRanges("hey @ai, what is the weather")).containsExactly("@ai")
    }

    /**
     * A `/draw` the SERVER will not act on must not be highlighted: the
     * whole point of mirroring the grammar is that the bubble and the
     * server agree about what leaves the building.
     */
    @Test
    fun aDrawTheServerIgnoresIsNotMarked() {
        assertThat(styledRanges("what does /draw do?")).isEmpty()
        assertThat(styledRanges("hey @ai /draw a cat")).containsExactly("@ai")
        // `/draw` alone is an ordinary message.
        assertThat(styledRanges("/draw")).isEmpty()
    }

    /**
     * A body is split into blocks around markdown tables, and the picture
     * token is a request only at the very start of the WHOLE body — so a
     * paragraph after a table is told it cannot start one.
     */
    @Test
    fun onlyTheFirstBlockMayCarryThePictureToken() {
        assertThat(styledRanges("/draw a cat", drawMayStartHere = false)).isEmpty()
        // The mention needs no such flag: it is one wherever it appears.
        assertThat(styledRanges("ask @ai about it", drawMayStartHere = false))
            .containsExactly("@ai")
    }

    /**
     * THE MARK IS DECIDED FROM THE RAW BODY, not from the rendered one.
     *
     * The server reads `/draw` off the body as typed. Everything else in
     * this file reads the body as DRAWN, because markdown deletes
     * characters and every link offset has to index what is on screen. For
     * this one token the two strings are not interchangeable: wherever
     * markup sits AHEAD of the token, rendering removes it and the token
     * slides to the front of a string the server never sees. Each body
     * below renders to something this grammar would call a picture
     * request while the server, reading the raw body, calls it an ordinary
     * message — so a mark computed from the rendered text was a family
     * watching a marked request go unanswered with no way to tell why.
     *
     * Both halves are asserted on purpose. Proving the RENDERED text
     * would have been marked is what keeps this test from passing
     * vacuously if the renderer ever stops stripping one of these.
     *
     * iOS's `MessageLinksTests.drawMarkComesFromTheRawBody` pins the same
     * bodies.
     */
    @Test
    fun theDrawMarkIsDecidedFromTheRawBody() {
        val markupAheadOfTheToken = listOf(
            "**/draw** a cat",
            "*/draw* a cat",
            "`/draw` a cat",
            "~~/draw~~ a cat",
            "# /draw a cat",
        )
        for (body in markupAheadOfTheToken) {
            val rendered = MessageMarkdown.blocks(body)
                .filterIsInstance<MessageMarkdown.Block.Text>()
                .first().rendered.text
            // The server, reading what was typed, sees no request.
            assertThat(AssistantMention.drawRange(body)).isNull()
            // The bubble, reading what is drawn, used to see one — which
            // is the divergence itself, asserted so this test cannot pass
            // vacuously if the renderer ever stops stripping one of these.
            assertThat(AssistantMention.drawRange(rendered)).isNotNull()
            // And now nothing is marked, because the two must agree.
            assertThat(markedIn(rendered = rendered, rawBody = body)).isEmpty()
        }
        // A bullet is the near miss that proves the list above is doing
        // work: its marker renders to U+2022 and a space, so the drawn text
        // does not begin with the token either and the two AGREE without
        // any of this. It is here so a reader can see where the line is.
        val bullet = MessageMarkdown.blocks("- /draw a cat")
            .filterIsInstance<MessageMarkdown.Block.Text>()
            .first().rendered.text
        assertThat(bullet).startsWith("\u2022 ")
        assertThat(AssistantMention.drawRange(bullet)).isNull()
    }

    /**
     * The other direction: markdown in the PROMPT removes nothing ahead of
     * the token, so a real request is still marked and the mark still sits
     * on the drawn glyphs rather than on the raw ones.
     */
    @Test
    fun aRealRequestIsStillMarkedThroughMarkdown() {
        val body = "/draw a **fluffy** cat"
        val rendered = MessageMarkdown.blocks(body)
            .filterIsInstance<MessageMarkdown.Block.Text>()
            .first().rendered.text
        assertThat(rendered).isEqualTo("/draw a fluffy cat")
        assertThat(markedIn(rendered = rendered, rawBody = body)).containsExactly("/draw")
        assertThat(markedIn(rendered = "@ai /draw a cat", rawBody = "@ai /draw a cat"))
            .containsExactly("@ai", "/draw")
    }

    /** No web link anywhere is no card — a phone number is not one. */
    @Test
    fun aBodyWithNoWebLinkHasNothingToPreview() {
        assertThat(MessageLinks.firstDrawnWebLinkUrl(MessageMarkdown.blocks("call 555-1234")))
            .isNull()
        assertThat(
            MessageLinks.firstDrawnWebLinkUrl(
                MessageMarkdown.blocks("| a | b |\n| --- | --- |\n| 1 | 2 |"),
            ),
        ).isNull()
    }
}
