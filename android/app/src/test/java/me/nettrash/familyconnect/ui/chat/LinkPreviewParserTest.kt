/*
 * LinkPreviewParserTest.kt
 * Family Connect (Android)
 *
 * Pins how a page turns into the card under a link. The parser is a
 * cross-platform contract (transcribed from Swift, with these same
 * vectors mirrored in LinkPreviewParserTests.swift), so the vectors
 * here ARE the spec: the Open Graph → Twitter → plain-HTML fallback
 * ladder, attribute forms real pages use (single quotes, reversed
 * order, self-closing), entity decoding, whitespace collapsing, length
 * clamping, relative image resolution, and the refusal to build a card
 * with no title.
 */

package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class LinkPreviewParserTest {

    private val page = "https://example.com/articles/1"

    @Test
    fun openGraphTagsWin() {
        val html = """
            <html><head>
            <title>Ignored</title>
            <meta property="og:title" content="The Real Title">
            <meta property="og:site_name" content="Example News">
            <meta property="og:description" content="A short summary.">
            <meta property="og:image" content="https://cdn.example.com/a.jpg">
            </head><body>…</body></html>
        """.trimIndent()
        val preview = LinkPreviewParser.parse(html, page)
        assertThat(preview?.title).isEqualTo("The Real Title")
        assertThat(preview?.siteName).isEqualTo("Example News")
        assertThat(preview?.description).isEqualTo("A short summary.")
        assertThat(preview?.imageUrl).isEqualTo("https://cdn.example.com/a.jpg")
    }

    @Test
    fun twitterCardsAreSecondChoicePlainHtmlThird() {
        val twitter = """
            <head><meta name="twitter:title" content="Twitter Title">
            <meta name="twitter:description" content="Twitter summary">
            <meta name="twitter:image" content="https://cdn.example.com/t.png"></head>
        """.trimIndent()
        val fromTwitter = LinkPreviewParser.parse(twitter, page)
        assertThat(fromTwitter?.title).isEqualTo("Twitter Title")
        assertThat(fromTwitter?.description).isEqualTo("Twitter summary")
        assertThat(fromTwitter?.imageUrl).isEqualTo("https://cdn.example.com/t.png")

        val plain = """
            <head><title>Plain Title</title>
            <meta name="description" content="Plain summary"></head>
        """.trimIndent()
        val fromPlain = LinkPreviewParser.parse(plain, page)
        assertThat(fromPlain?.title).isEqualTo("Plain Title")
        assertThat(fromPlain?.description).isEqualTo("Plain summary")
        assertThat(fromPlain?.imageUrl).isNull()
    }

    @Test
    fun noTitleMeansNoCard() {
        assertThat(LinkPreviewParser.parse("<head></head>", page)).isNull()
        assertThat(LinkPreviewParser.parse("<head><title>   </title></head>", page)).isNull()
        assertThat(LinkPreviewParser.parse("", page)).isNull()
    }

    @Test
    fun siteNameFallsBackToHostWithoutWww() {
        val preview = LinkPreviewParser.parse(
            "<head><title>T</title></head>",
            "https://www.example.com/x",
        )
        assertThat(preview?.siteName).isEqualTo("example.com")
    }

    @Test
    fun attributeFormsRealPagesUse() {
        // Single quotes, reversed order, self-closing, extra attributes.
        val html = """
            <head>
            <meta content='Reversed Order' property='og:title' />
            <meta charset="utf-8">
            <meta data-rh="true" property="og:description" content="Desc" />
            </head>
        """.trimIndent()
        val preview = LinkPreviewParser.parse(html, page)
        assertThat(preview?.title).isEqualTo("Reversed Order")
        assertThat(preview?.description).isEqualTo("Desc")
    }

    @Test
    fun firstOccurrenceOfAKeyWins() {
        val html = """
            <head><meta property="og:title" content="First">
            <meta property="og:title" content="Second"></head>
        """.trimIndent()
        assertThat(LinkPreviewParser.parse(html, page)?.title).isEqualTo("First")
    }

    @Test
    fun entitiesAreDecodedAndWhitespaceCollapsed() {
        val html = """
            <head><meta property="og:title"
            content="Tom &amp; Jerry&#39;s
                 big   day &hellip;"></head>
        """.trimIndent()
        assertThat(LinkPreviewParser.parse(html, page)?.title)
            .isEqualTo("Tom & Jerry's big day …")
    }

    @Test
    fun overLongTextIsClampedWithAnEllipsis() {
        val long = "a".repeat(400)
        val html = """<head><meta property="og:title" content="$long"></head>"""
        val title = LinkPreviewParser.parse(html, page)?.title ?: ""
        assertThat(title).hasLength(LinkPreviewParser.MAX_TITLE_LENGTH + 1) // + the ellipsis
        assertThat(title).endsWith("…")
    }

    @Test
    fun relativeAndProtocolRelativeImagesResolveAgainstThePage() {
        val relative =
            """<head><title>T</title><meta property="og:image" content="/img/a.png"></head>"""
        assertThat(LinkPreviewParser.parse(relative, page)?.imageUrl)
            .isEqualTo("https://example.com/img/a.png")

        val protocolRelative =
            """<head><title>T</title><meta property="og:image" content="//cdn.example.com/b.png"></head>"""
        assertThat(LinkPreviewParser.parse(protocolRelative, page)?.imageUrl)
            .isEqualTo("https://cdn.example.com/b.png")
    }

    @Test
    fun nonHttpImageSchemesAreRefused() {
        val html =
            """<head><title>T</title><meta property="og:image" content="file:///etc/passwd"></head>"""
        assertThat(LinkPreviewParser.parse(html, page)?.imageUrl).isNull()
    }

    @Test
    fun aTagWhoseNameMerelyStartsWithMetaIsNotAMetaTag() {
        val html =
            """<head><metadata property="og:title" content="Nope"><title>Real</title></head>"""
        assertThat(LinkPreviewParser.parse(html, page)?.title).isEqualTo("Real")
    }

    @Test
    fun firstWebLinkPicksHttpOverTelAndMailto() {
        val spans = listOf(
            LinkSpan(0, 5, "tel:5551234567"),
            LinkSpan(6, 10, "mailto:nettrash@nettrash.me"),
            LinkSpan(11, 20, "https://example.com/a"),
            LinkSpan(21, 30, "https://example.com/b"),
        )
        assertThat(spans.firstWebLinkUrl()).isEqualTo("https://example.com/a")
        assertThat(listOf(LinkSpan(0, 5, "tel:5551234567")).firstWebLinkUrl()).isNull()
        assertThat(emptyList<LinkSpan>().firstWebLinkUrl()).isNull()
    }

    @Test
    fun unicodeThatLowercasesToADifferentLengthDoesNotCorrupt() {
        // İ lowercases to two chars and ẞ to "ss": matching on a
        // lowercase() COPY and slicing the original with its indices
        // leaks markup into titles here and traps outright on iOS.
        assertThat(LinkPreviewParser.parse("<head><title>\u0130stanbul Haber</title></head>", page)?.title)
            .isEqualTo("\u0130stanbul Haber")
        assertThat(LinkPreviewParser.parse("<head><title>\u0130\u0130\u0130\u0130\u0130\u0130\u0130\u0130\u0130\u0130</title></head>", page)?.title)
            .isEqualTo("\u0130\u0130\u0130\u0130\u0130\u0130\u0130\u0130\u0130\u0130")
        assertThat(LinkPreviewParser.parse("<head><title>GRO\u1E9EE STRA\u1E9EE</title></head>", page)?.title)
            .isEqualTo("GRO\u1E9EE STRA\u1E9EE")
        // Raw strings do NOT process \u escapes, so the drifting
        // character is built in a normal literal and interpolated.
        val dotted = "\u0130".repeat(8)
        val afterDrift =
            """<head><p>$dotted</p><meta property="og:title" content="Found"></head>"""
        assertThat(LinkPreviewParser.parse(afterDrift, page)?.title).isEqualTo("Found")
        val inContent =
            """<head><meta property="og:description" content="${dotted + dotted}"><title>Real</title></head>"""
        assertThat(LinkPreviewParser.parse(inContent, page)?.title).isEqualTo("Real")
    }

    @Test
    fun strayAmpersandPageStaysLinear() {
        val hostile = "<head><title>" + "&".repeat(30_000) + "x;</title></head>"
        val started = System.nanoTime()
        LinkPreviewParser.parse(hostile, page)
        val seconds = (System.nanoTime() - started) / 1_000_000_000.0
        assertThat(seconds).isLessThan(2.0)
    }

    @Test
    fun quotedAngleBracketDoesNotTruncateTheTag() {
        val html =
            """<head><meta property="og:title" content="A > B"><title>Fallback</title></head>"""
        assertThat(LinkPreviewParser.parse(html, page)?.title).isEqualTo("A > B")
    }

    @Test
    fun clampingCountsCodePointsAndNeverSplitsAPair() {
        val emoji = "\uD83D\uDE00".repeat(200)
        val html = """<head><meta property="og:title" content="$emoji"></head>"""
        val title = LinkPreviewParser.parse(html, page)?.title ?: ""
        // 140 code points kept + the ellipsis, no lone surrogate.
        assertThat(title.codePointCount(0, title.length))
            .isEqualTo(LinkPreviewParser.MAX_TITLE_LENGTH + 1)
        assertThat(title).endsWith("…")
        assertThat(title.dropLast(1)).isEqualTo("\uD83D\uDE00".repeat(140))
    }

    @Test
    fun hostLabelSurvivesAUriJavaRejects() {
        // Underscores are legal in practice but java.net.URI says no.
        assertThat(LinkPreviewParser.displayHost("https://my_host.example.com/x"))
            .isEqualTo("my_host.example.com")
    }
}
