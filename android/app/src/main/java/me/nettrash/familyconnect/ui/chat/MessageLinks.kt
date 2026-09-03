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
        // [body] is the RENDERED text, not the raw message. Markdown
        // deletes characters, so slicing the raw body here would announce
        // the wrong words — the same offset trap the hit test has.
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
     *
     * Takes an ALREADY-RENDERED [AnnotatedString] rather than a raw body:
     * markdown runs first (MessageMarkdown), and every offset in [spans]
     * indexes what it produced. Appending the rendered string preserves the
     * emphasis it carries — rebuilding from `text` alone would throw the
     * markdown away.
     */
    fun styled(text: AnnotatedString, spans: List<LinkSpan>, style: SpanStyle): AnnotatedString {
        if (spans.isEmpty()) return text
        return buildAnnotatedString {
            append(text)
            for (span in spans) {
                addStyle(style, span.start.coerceIn(0, length), span.end.coerceIn(0, length))
            }
        }
    }

    /**
     * The markdown's own links, plus every detected one that does not
     * overlap them, in order of appearance.
     *
     * The overlap rule is a SAFETY rule, not tidiness. A markdown link
     * whose label is itself a URL — `[https://www.paypal.com](https://evil.example)`
     * — renders as that URL's text, which the detector then matches as a
     * link to the place it names. Two spans, same glyphs, different
     * destinations. Whichever won, a tap could open somewhere the reader
     * had every reason to think was what they were looking at. The author's
     * own destination is the one the message actually declares, so it wins
     * and the detector's duplicate is dropped — leaving exactly one answer
     * for every glyph, which is also what the hit test assumes.
     */
    fun mergeSpans(markdown: List<LinkSpan>, detected: List<LinkSpan>): List<LinkSpan> {
        if (markdown.isEmpty()) return detected
        val kept = detected.filterNot { span ->
            markdown.any { span.start < it.end && it.start < span.end }
        }
        return (markdown + kept).sortedBy { it.start }
    }

    /**
     * The first web link a body DRAWS — what the preview card under the
     * balloon describes.
     *
     * TABLE CELLS COUNT, and that is the whole reason this exists rather
     * than a `flatMap { it.links }` at the call site. A cell carries no
     * links by construction ([MessageMarkdown.cell] says why: a cell is
     * not an offset space, so a link in one could not be hit-tested), and
     * a URL typed into one is therefore drawn as characters nobody can
     * tap. The card is then the ONLY way in — a better answer than
     * pretending the reader never saw it, and the answer Apple already
     * gives (`MessageLinks.firstWebLinkAsDrawn`, which scans the flat
     * render, tables included).
     *
     * In block order, and inside a table in reading order, so "the first"
     * means the same message on both platforms. Text blocks go through
     * [mergeSpans] exactly as the drawing does — the author's own
     * destination beats a label that disagrees with it, so a
     * `[label](url)` message previews the URL it actually opens.
     */
    fun firstDrawnWebLinkUrl(blocks: List<MessageMarkdown.Block>): String? {
        for (block in blocks) {
            when (block) {
                is MessageMarkdown.Block.Text -> {
                    val links = mergeSpans(block.rendered.links, linkSpans(block.rendered.text))
                    links.firstWebLinkUrl()?.let { return it }
                }
                is MessageMarkdown.Block.Table -> {
                    val cells = block.header.asSequence() + block.rows.asSequence().flatten()
                    for (cell in cells) {
                        linkSpans(cell.text).firstWebLinkUrl()?.let { return it }
                    }
                }
            }
        }
        return null
    }

    /**
     * The assistant's tokens — every `@ai`, and the leading `/draw` when
     * the body is a picture request — marked so they read as instructions
     * rather than as words.
     *
     * Same GRAMMAR the SERVER decides by ([AssistantMention], mirrored in
     * three places and pinned by one shared table): a token the server acts
     * on that the bubble draws as ordinary text, or the reverse, is a
     * family watching a question go unanswered with no way to tell why.
     * `/draw` needs it even more sharply — the server decides from it
     * whether the request goes to an entirely DIFFERENT provider, so "each
     * client must highlight exactly what the server will act on"
     * (docs/protocol.md, "Pictures").
     *
     * **What this is NOT is a claim about the chat.** This function has no
     * chat kind and asks for none, so both tokens are marked in a DIRECT
     * chat too, where the server acts on neither: there is no assistant in
     * a two-member thread, `@ai` reaches nobody there and `/draw` is a
     * word. Deliberate, for three reasons: the two tokens then have ONE
     * rule between them (`@ai` has been marked everywhere since it existed,
     * so scoping only `/draw` would draw two assistant tokens by two rules
     * in one bubble); the mark says what the token IS, and nothing here
     * INVITES it — the `/draw` button, the `@ai` button and the picture
     * door are each scoped to the surface that honours them, so a token in
     * a direct chat is one somebody typed for themselves; and this is
     * memoized per rendered body, which a chat kind would turn into a
     * second cache dimension here and on iOS, bought for a bold word in a
     * chat with no assistant. If that trade is revisited, revisit it for
     * BOTH tokens.
     *
     * Only the tokens, never the words after them: those are the member's
     * own and read as ordinary text.
     *
     * Applied over the RENDERED text, like everything else here, and only
     * as a style — so it never competes with a link span for a tap. The
     * picture token is DECIDED from the raw body all the same; see
     * [rawBody].
     */
    fun withMentions(
        text: AnnotatedString,
        style: SpanStyle,
        /**
         * The RAW message body — the exact string the server parses —
         * or null when this block cannot begin a picture request.
         *
         * The server reads `/draw` off the body as typed. This file is
         * handed the body as DRAWN, after markdown has deleted the `**`,
         * the backticks and the `](url)`. Deciding from the rendered text
         * is therefore deciding from a different string than the server
         * does, and wherever markdown removes characters ahead of the
         * token the two disagree: a `/draw` wrapped in a pair of asterisks
         * renders to `/draw a cat`, so the bubble highlighted a picture
         * request that the server — looking at the asterisks it was
         * actually sent — reads as an ordinary message and never acts on.
         * Six such shapes were found, and `theDrawMarkIsDecidedFromTheRawBody`
         * in `MessageLinksTest` lists them; that is the whole class.
         * (They cannot be spelled here: a bold marker in front of the
         * token closes a KDoc block.)
         *
         * So the DECISION comes from this string and the POSITION from the
         * rendered one, and the highlight is drawn only where both agree
         * that a request starts. Where markdown has mangled the token
         * beyond the grammar's recognition the bubble simply says nothing,
         * which is the safe half of the disagreement: a missing highlight
         * on a request the server honours, never a highlight on one it
         * ignores.
         *
         * Null for every block but the first. A message is split into
         * blocks around markdown tables, and `/draw` is a request only at
         * the very beginning of the whole body — without that, a paragraph
         * that happens to follow a table and begin `/draw …` would be
         * highlighted as a request nothing will act on. `@ai` needs no
         * such treatment: it is a mention wherever it appears.
         */
        rawBody: String? = null,
    ): AnnotatedString {
        val ranges = AssistantMention.ranges(text.text)
        val draw = rawBody
            ?.takeIf { AssistantMention.drawRange(it) != null }
            ?.let { AssistantMention.drawRange(text.text) }
        if (ranges.isEmpty() && draw == null) return text
        return buildAnnotatedString {
            append(text)
            for (range in ranges) {
                addStyle(style, range.first, range.last + 1)
            }
            if (draw != null) addStyle(style, draw.first, draw.last + 1)
        }
    }
}
