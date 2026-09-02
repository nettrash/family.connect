/*
 * LinkPreviewYouTubeTest.kt
 * Family Connect (Android)
 *
 * #50: a YouTube link showed no card. Not a parse bug, not a User-Agent
 * problem, not a consent wall — youtube.com answers this app's bot
 * User-Agent with a plain 200 and complete og: tags. The tags just sit
 * ~706KB into a 1.3MB page, behind ~700KB of inline player JSON, and
 * the fetcher stopped at 256KB while the parser stopped at 200K
 * characters. The first 256KB of that page contain no og:title and not
 * even a <title>, so the parser correctly refused to build a card and
 * the bubble showed a bare link.
 *
 * These tests run on the captured bytes (LinkPreviewFixtures), never on
 * the network, and pin both halves of the fix: the parser reaching past
 * the old scan limit, and the head-bounded read that gets the bytes
 * there without downloading the rest of the page.
 *
 * iOS mirror: LinkPreviewYouTubeTests.swift.
 */

package me.nettrash.familyconnect.data.net

import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.ui.chat.LinkPreviewParser
import org.junit.Test
import java.io.ByteArrayInputStream

class LinkPreviewYouTubeTest {

    private val watchUrl = "https://www.youtube.com/watch?v=dQw4w9WgXcQ"

    // The page itself

    @Test
    fun `the captured page hides its tags past a quarter megabyte`() {
        // Byte offsets, because that is the unit the fetcher's cap is in
        // — the page carries Cyrillic and an emoji, so character offsets
        // would not be the same number.
        val bytes = LinkPreviewFixtures.youTubeWatchPage().toByteArray(Charsets.UTF_8)
        assertThat(indexOf("<title", bytes)).isEqualTo(LinkPreviewFixtures.TITLE_TAG_OFFSET)
        assertThat(indexOf("og:title", bytes)).isEqualTo(LinkPreviewFixtures.OG_TITLE_OFFSET)
        assertThat(indexOf("</head", bytes)).isEqualTo(LinkPreviewFixtures.HEAD_END_OFFSET)
        assertThat(LinkPreviewFixtures.OG_TITLE_OFFSET).isGreaterThan(256 * 1024)
    }

    @Test
    fun `a watch page yields the full card`() {
        val preview = LinkPreviewParser.parse(LinkPreviewFixtures.youTubeWatchPage(), watchUrl)
        assertThat(preview).isNotNull()
        assertThat(preview?.title).isEqualTo(LinkPreviewFixtures.TITLE)
        assertThat(preview?.siteName).isEqualTo("YouTube")
        assertThat(preview?.imageUrl).isEqualTo(LinkPreviewFixtures.IMAGE_URL)
        assertThat(preview?.description).startsWith("The official video for")
    }

    @Test
    fun `the old 200K scan limit is what made the card vanish`() {
        val html = LinkPreviewFixtures.youTubeWatchPage()
        // Exactly what the parser used to see, and it is not enough for
        // even the <title> fallback — so the state was Unavailable and
        // the bubble drew a bare link.
        assertThat(LinkPreviewParser.parse(html.take(200_000), watchUrl)).isNull()
        assertThat(
            LinkPreviewParser.parse(html.take(LinkPreviewParser.SCAN_LIMIT), watchUrl),
        ).isNotNull()
    }

    @Test
    fun `the scan limit is never below the fetch cap`() {
        // A page is UTF-8: it can only ever decode to FEWER characters
        // than it has bytes, so a scan limit at least as large as the
        // byte cap always sees everything that was paid for. Raising one
        // alone re-opens #50.
        assertThat(LinkPreviewParser.SCAN_LIMIT).isAtLeast(1024 * 1024)
    }

    // The head-bounded read

    @Test
    fun `the read stops at the end of the real page's head`() {
        val bytes = LinkPreviewFixtures.youTubeWatchPage().toByteArray(Charsets.UTF_8)
        val read = readHead(ByteArrayInputStream(bytes), 1024L * 1024)
        // Through the ">" of "</head>", and not one byte of the 595KB
        // body below it.
        assertThat(read.size).isEqualTo(LinkPreviewFixtures.HEAD_END_OFFSET + 7)
        assertThat(LinkPreviewParser.parse(String(read, Charsets.UTF_8), watchUrl)?.title)
            .isEqualTo(LinkPreviewFixtures.TITLE)
    }

    @Test
    fun `a tag whose name merely starts with body or head is not the end`() {
        assertThat(stopAfter("<header><bodyguard>")).isEqualTo(-1)
        assertThat(stopAfter("<head><meta><bodyx")).isEqualTo(-1)
    }

    @Test
    fun `a page that omits the head end tag still stops at body`() {
        // 22 is the "<" of <body>; the space at 27 is what proves the
        // tag name ended.
        assertThat(stopAfter("<head><title>x</title><body class=\"a\">")).isEqualTo(28)
    }

    @Test
    fun `an uppercase head end tag stops the read too`() {
        assertThat(stopAfter("<HEAD><TITLE>x</TITLE></HEAD>")).isEqualTo(29)
    }

    @Test
    fun `a page with neither sentinel is read to the cap`() {
        val html = "<html><meta name=\"a\" content=\"b\">"
        assertThat(stopAfter(html)).isEqualTo(-1)
        assertThat(readHead(ByteArrayInputStream(html.toByteArray()), 1024L * 1024).size)
            .isEqualTo(html.length)
    }

    @Test
    fun `the cap still holds when a page never closes its head`() {
        val html = "<html><head><script>" + "a".repeat(1_100_000) +
            "</script><meta property=\"og:title\" content=\"Past The Cap\">"
        val read = readHead(ByteArrayInputStream(html.toByteArray()), 1024L * 1024)
        assertThat(read.size).isEqualTo(1024 * 1024)
        assertThat(LinkPreviewParser.parse(String(read, Charsets.UTF_8), watchUrl)).isNull()
    }

    @Test
    fun `tags below the head end are not read, and that is the deal`() {
        // The price of stopping at </head>: a page that puts its og:
        // tags in the BODY loses its card, where the old flat 256K read
        // would have found them. It is the documented contract of the
        // parser ("It reads the <head> only"), no page in the sample
        // does it, and it is what keeps a 1MB ceiling from meaning a 1MB
        // download. Pinned so the trade stays a decision, not an
        // accident.
        val html = "<html><head><meta name=\"nothing\" content=\"x\"></head><body>" +
            "<meta property=\"og:title\" content=\"Too Late\"></body></html>"
        val read = readHead(ByteArrayInputStream(html.toByteArray()), 1024L * 1024)
        assertThat(LinkPreviewParser.parse(String(read, Charsets.UTF_8), watchUrl)).isNull()
    }

    // Helpers

    /** Bytes kept when [html] is read head-bounded, or -1 if it never stops. */
    private fun stopAfter(html: String): Int {
        val scanner = HeadEndScanner()
        html.toByteArray(Charsets.UTF_8).forEachIndexed { index, byte ->
            if (scanner.consume(byte)) return index + 1
        }
        return -1
    }

    private fun indexOf(needle: String, bytes: ByteArray): Int {
        val pattern = needle.toByteArray(Charsets.UTF_8)
        outer@ for (start in 0..(bytes.size - pattern.size)) {
            for (offset in pattern.indices) {
                if (bytes[start + offset] != pattern[offset]) continue@outer
            }
            return start
        }
        return -1
    }
}
