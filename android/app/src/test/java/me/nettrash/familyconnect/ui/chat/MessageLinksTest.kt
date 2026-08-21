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

import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.style.TextDecoration
import com.google.common.truth.Truth.assertThat
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class MessageLinksTest {

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

    @Test
    fun styledKeepsTheTextAndStylesExactlyTheSpanRanges() {
        val text = "docs: https://example.com ok"
        val spans = MessageLinks.linkSpans(text)
        val style = SpanStyle(textDecoration = TextDecoration.Underline)
        val styled = MessageLinks.styled(text, spans, style)
        assertThat(styled.text).isEqualTo(text)
        assertThat(styled.spanStyles).hasSize(1)
        assertThat(styled.spanStyles[0].start).isEqualTo(spans[0].start)
        assertThat(styled.spanStyles[0].end).isEqualTo(spans[0].end)
        assertThat(styled.spanStyles[0].item).isEqualTo(style)
    }
}
