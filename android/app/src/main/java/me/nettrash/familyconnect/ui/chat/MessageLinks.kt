/*
 * MessageLinks.kt
 * Family Connect (Android)
 *
 * Tappable data in message bodies: web links open the browser, email
 * addresses open the mail app, phone numbers open the dialer with the
 * number filled in. Detection is android.text.util.Linkify — the
 * platform's own linkification (phones go through the ICU phone-number
 * matcher), so chat bubbles agree with what the rest of the OS
 * considers a link or a phone number.
 *
 * Rendering is deliberately NOT LinkAnnotation.Url: Compose mounts
 * each link range as its own clickable child that consumes the pointer
 * down, which silently kills the bubble's double-tap-heart and
 * long-press-reaction gestures over link glyphs (and turns a long
 * press on a link into a navigation on release). Instead the spans
 * are styled here and BubbleContent hit-tests taps against the text
 * layout itself, so one detector arbitrates tap-to-open vs
 * double-tap-heart vs long-press-picker.
 *
 * Unlike EmojiOnly this is deliberately NOT a byte-identical
 * cross-platform scanner: URL and especially phone grammars are
 * platform-tuned (locales, carriers), so each app leans on its own
 * platform detector and the two agree on the CATEGORIES (web, email,
 * phone), not on every edge case. iOS counterpart:
 * ios/FamilyConnect/Models/MessageLinks.swift (NSDataDetector).
 *
 * linkSpans is kept Compose-free so MessageLinksTest can pin it under
 * Robolectric.
 */

package me.nettrash.familyconnect.ui.chat

import android.text.SpannableString
import android.text.style.URLSpan
import android.text.util.Linkify
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString

/** One tappable range in a message body. */
data class LinkSpan(
    val start: Int,
    val end: Int,
    /** Ready-to-open URL: http(s)://…, mailto:… or tel:…. */
    val url: String,
)

/**
 * The first http(s) link among these spans — what the preview card
 * describes. Phone numbers and email addresses are detected too but
 * have nothing to preview, and previewing every link in a message would
 * bury the message itself.
 */
fun List<LinkSpan>.firstWebLinkUrl(): String? =
    firstOrNull { it.url.startsWith("http://") || it.url.startsWith("https://") }?.url

object MessageLinks {

    /**
     * Every web URL, email address and phone number in [text], in
     * order of appearance. Linkify hands back ready-to-open URLs:
     * schemeless web matches gain http://, emails become mailto:,
     * phones become tel:.
     */
    fun linkSpans(text: String): List<LinkSpan> {
        if (text.isEmpty()) return emptyList()
        val spannable = SpannableString(text)
        val found = Linkify.addLinks(
            spannable,
            Linkify.WEB_URLS or Linkify.EMAIL_ADDRESSES or Linkify.PHONE_NUMBERS,
        )
        if (!found) return emptyList()
        return spannable.getSpans(0, spannable.length, URLSpan::class.java)
            .map { span ->
                LinkSpan(
                    start = spannable.getSpanStart(span),
                    end = spannable.getSpanEnd(span),
                    url = span.url,
                )
            }
            .sortedBy { it.start }
    }

    /**
     * TalkBack label for the custom action that opens [span] in [body]:
     * the matched text with the verb its scheme implies. Needed because
     * the tap detector is a raw pointerInput, which exposes no click
     * action of its own — without these the links would be sighted-only.
     */
    fun accessibilityLabel(body: String, span: LinkSpan): String {
        val text = body.substring(span.start.coerceIn(0, body.length), span.end.coerceIn(0, body.length))
        return when {
            span.url.startsWith("tel:") -> "Call $text"
            span.url.startsWith("mailto:") -> "Email $text"
            else -> "Open $text"
        }
    }

    /**
     * [text] with [style] applied over every span in [spans] — the link
     * LOOK only; tap handling lives in BubbleContent's own detector
     * (see the header for why this is not LinkAnnotation.Url).
     */
    fun styled(text: String, spans: List<LinkSpan>, style: SpanStyle): AnnotatedString {
        if (spans.isEmpty()) return AnnotatedString(text)
        return buildAnnotatedString {
            append(text)
            for (span in spans) {
                addStyle(style, span.start, span.end)
            }
        }
    }
}
