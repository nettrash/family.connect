/*
 * LinkPreview.kt
 * Family Connect (Android)
 *
 * The card under a message's first web link: what it holds, and how it
 * is read out of a page's HTML.
 *
 * Parsing is a deliberately small, tolerant scanner rather than a real
 * HTML parser — it only needs the handful of <meta> tags every site
 * publishes for exactly this purpose (Open Graph, with Twitter cards
 * and plain <title>/<meta description> as fallbacks), and it must
 * behave identically to the Swift original in
 * ios/FamilyConnect/Models/LinkPreview.swift. Mirrored tests on both
 * platforms are the spec; change them together.
 *
 * Everything works on a CharArray with integer indices, and case
 * folding is ASCII-only. That is not fussiness: matching against a
 * lowercase() COPY and slicing the original with the indices it yields
 * corrupts output. Unicode lowercasing is not length-preserving
 * (İ → i̇, ẞ → ss), so the two strings drift apart and titles come out
 * with raw markup in them, cards silently vanish, or the substring
 * throws — and the same bug traps the process outright on iOS. Tag and
 * attribute names are ASCII by definition, so ASCII-only folding is
 * both sufficient and length-preserving by construction.
 *
 * It reads the <head> only. Bodies are megabytes of markup that cannot
 * contain the tags we want, and scanning them would cost far more than
 * the fetch.
 *
 * PRIVACY: building one of these means THIS device contacts the linked
 * site (see LinkPreviewRepository). That is the trade the feature
 * makes, and why Settings can switch it off.
 *
 * Kept free of Compose and Android so the tests run as plain unit
 * tests.
 */

package me.nettrash.familyconnect.ui.chat

import java.net.URI

data class LinkPreview(
    /** The link this describes — the canonical cache key. */
    val url: String,
    val title: String,
    /** Publisher name (og:site_name), falling back to the host. */
    val siteName: String,
    val description: String?,
    /** Absolute URL of the card image, when the page offers one. */
    val imageUrl: String?,
)

object LinkPreviewParser {

    /**
     * Longest prefix of a page worth scanning: everything read lives in
     * <head>, and the fetcher caps the download anyway.
     *
     * Deliberately the SAME number as the fetcher's byte cap, and a
     * second reason #50 showed no card for a YouTube link: a scan limit
     * under the fetch cap silently throws away bytes already paid for.
     * A UTF-8 page never decodes to more characters than it has bytes,
     * so matching the cap means the parser always sees everything that
     * was downloaded. Raising one without the other fixes nothing —
     * YouTube's og:title sits at ~706K.
     */
    const val SCAN_LIMIT = 1_048_576

    /**
     * Both limits count UNICODE CODE POINTS, on both platforms —
     * clamping by Kotlin's UTF-16 length and Swift's grapheme count
     * would cut the same page at two different places, and UTF-16
     * truncation splits surrogate pairs into tofu.
     */
    const val MAX_TITLE_LENGTH = 140
    const val MAX_DESCRIPTION_LENGTH = 300

    /**
     * Longest entity this decodes, `&` and `;` excluded — bounds the
     * lookahead so a page full of stray ampersands stays linear.
     */
    private const val MAX_ENTITY_LENGTH = 10

    /**
     * Read a preview out of [html] for [pageUrl]. Returns null when the
     * page offers no usable title — a card with no title is just a
     * second copy of the URL.
     */
    fun parse(html: String, pageUrl: String): LinkPreview? {
        val head = (if (html.length > SCAN_LIMIT) html.substring(0, SCAN_LIMIT) else html)
            .toCharArray()
        val metas = metaTags(head)

        val title = firstNonEmpty(metas["og:title"], metas["twitter:title"], titleTag(head))
            ?: return null

        val description = firstNonEmpty(
            metas["og:description"],
            metas["twitter:description"],
            metas["description"],
        )

        val siteName = firstNonEmpty(metas["og:site_name"]) ?: displayHost(pageUrl)

        val image = firstNonEmpty(
            metas["og:image"],
            metas["og:image:url"],
            metas["twitter:image"],
        )?.let { absoluteUrl(it, pageUrl) }

        return LinkPreview(
            url = pageUrl,
            title = clamp(title, MAX_TITLE_LENGTH),
            siteName = siteName,
            description = description?.let { clamp(it, MAX_DESCRIPTION_LENGTH) },
            imageUrl = image,
        )
    }

    /**
     * The host as a card would show it, without a leading "www.".
     * java.net.URI rejects hosts browsers accept (underscores), so the
     * authority is used when the strict parse yields nothing — showing
     * a whole URL as the publisher label looks broken.
     */
    fun displayHost(url: String): String {
        val parsed = runCatching { URI(url) }.getOrNull()
        val host = parsed?.host
            ?: parsed?.authority?.substringAfter('@')?.substringBefore(':')
            ?: url.substringAfter("//").substringBefore('/').substringBefore(':')
        return host.removePrefix("www.")
    }

    // Scanning

    /**
     * Every <meta> tag's key → content, keys lowercased. A tag's key is
     * its `property` (Open Graph's spelling) or its `name` (everything
     * else); the FIRST occurrence of a key wins, matching how browsers
     * treat duplicated tags.
     */
    fun metaTags(html: CharArray): Map<String, String> {
        val result = LinkedHashMap<String, String>()
        for (tag in tags("meta", html)) {
            val attributes = attributes(tag)
            val key = (attributes["property"] ?: attributes["name"])
                ?.let(::asciiLowercase) ?: continue
            val content = attributes["content"] ?: continue
            if (key !in result) {
                result[key] = collapseWhitespace(decodeEntities(content))
            }
        }
        return result
    }

    /** The text of the first <title> element. */
    fun titleTag(html: CharArray): String? {
        val open = indexOf("<title", html, 0)
        if (open < 0 || !isTagNameBoundary(html, open + 6)) return null
        val contentStart = endOfTag(html, open + 6)
        if (contentStart < 0) return null
        val close = indexOf("</title", html, contentStart + 1)
        if (close < 0) return null
        val raw = String(html, contentStart + 1, close - contentStart - 1)
        return collapseWhitespace(decodeEntities(raw)).ifEmpty { null }
    }

    /**
     * Bodies of every `<name …>` tag (the part after the name, before
     * the closing `>`), as raw strings.
     */
    private fun tags(name: String, html: CharArray): List<String> {
        val tags = mutableListOf<String>()
        val opening = "<$name"
        var index = 0
        while (index < html.size) {
            val open = indexOf(opening, html, index)
            if (open < 0) break
            val afterName = open + opening.length
            if (!isTagNameBoundary(html, afterName)) {
                index = afterName
                continue
            }
            val end = endOfTag(html, afterName)
            if (end < 0) break
            tags += String(html, afterName, end - afterName)
            index = end + 1
        }
        return tags
    }

    /**
     * True when the tag name really ends here — "<metadata" must not
     * match "<meta".
     */
    private fun isTagNameBoundary(html: CharArray, index: Int): Boolean {
        if (index >= html.size) return true
        return !html[index].isLetterOrDigit()
    }

    /**
     * Offset of the `>` that closes a tag whose body starts at [start],
     * skipping any `>` inside a quoted attribute value (real pages put
     * them in descriptions), or -1.
     */
    private fun endOfTag(html: CharArray, start: Int): Int {
        var index = start
        var quote: Char? = null
        while (index < html.size) {
            val character = html[index]
            when {
                quote != null -> if (character == quote) quote = null
                character == '"' || character == '\'' -> quote = character
                character == '>' -> return index
            }
            index++
        }
        return -1
    }

    /**
     * First offset at or after [from] where [needle] matches, compared
     * ASCII-case-insensitively. [needle] must already be lowercase.
     */
    private fun indexOf(needle: String, html: CharArray, from: Int): Int {
        if (needle.isEmpty() || html.size < needle.length) return -1
        var start = maxOf(0, from)
        while (start <= html.size - needle.length) {
            var offset = 0
            while (offset < needle.length &&
                asciiLowercase(html[start + offset]) == needle[offset]
            ) {
                offset++
            }
            if (offset == needle.length) return start
            start++
        }
        return -1
    }

    /**
     * Attributes of one tag body, names lowercased. Tolerates single
     * quotes, double quotes and unquoted values, in any order.
     */
    fun attributes(tag: String): Map<String, String> {
        val result = LinkedHashMap<String, String>()
        var index = 0

        fun skipWhitespace() {
            while (index < tag.length && tag[index].isWhitespace()) index++
        }

        while (index < tag.length) {
            skipWhitespace()
            val name = StringBuilder()
            while (index < tag.length &&
                !tag[index].isWhitespace() &&
                tag[index] != '=' &&
                tag[index] != '/'
            ) {
                name.append(tag[index])
                index++
            }
            skipWhitespace()
            if (index >= tag.length || tag[index] != '=') {
                // Valueless attribute — skip a stray "/" and continue.
                if (index < tag.length && tag[index] == '/') index++
                if (name.isEmpty() && index < tag.length) index++
                continue
            }
            index++ // '='
            skipWhitespace()
            val value = StringBuilder()
            if (index < tag.length && (tag[index] == '"' || tag[index] == '\'')) {
                val quote = tag[index]
                index++
                while (index < tag.length && tag[index] != quote) {
                    value.append(tag[index])
                    index++
                }
                if (index < tag.length) index++ // closing quote
            } else {
                while (index < tag.length && !tag[index].isWhitespace()) {
                    value.append(tag[index])
                    index++
                }
            }
            if (name.isNotEmpty()) {
                val key = asciiLowercase(name.toString())
                if (key !in result) result[key] = value.toString()
            }
        }
        return result
    }

    // Text helpers

    /**
     * ASCII-only lowercasing: length-preserving by construction, which
     * is what keeps parallel indices valid (see the header), and
     * locale-independent unlike String.lowercase() with a default
     * locale.
     */
    fun asciiLowercase(text: String): String {
        val out = StringBuilder(text.length)
        for (ch in text) out.append(asciiLowercase(ch))
        return out.toString()
    }

    private fun asciiLowercase(ch: Char): Char = if (ch in 'A'..'Z') ch + 32 else ch

    /**
     * The named and numeric HTML entities that actually show up in
     * titles and descriptions. The lookahead for the terminating `;`
     * is bounded, so a page of stray ampersands cannot make this
     * quadratic.
     */
    fun decodeEntities(text: String): String {
        if ('&' !in text) return text
        val out = StringBuilder(text.length)
        var index = 0
        while (index < text.length) {
            if (text[index] != '&') {
                out.append(text[index])
                index++
                continue
            }
            var semicolon = -1
            var lookahead = index + 1
            val limit = minOf(text.length, index + MAX_ENTITY_LENGTH + 2)
            while (lookahead < limit) {
                if (text[lookahead] == ';') {
                    semicolon = lookahead
                    break
                }
                lookahead++
            }
            val replacement = if (semicolon < 0) {
                null
            } else {
                replacementFor(text.substring(index + 1, semicolon))
            }
            if (replacement == null) {
                out.append(text[index])
                index++
            } else {
                out.append(replacement)
                index = semicolon + 1
            }
        }
        return out.toString()
    }

    private fun replacementFor(entity: String): String? {
        when (asciiLowercase(entity)) {
            "amp" -> return "&"
            "lt" -> return "<"
            "gt" -> return ">"
            "quot" -> return "\""
            "apos", "#39" -> return "'"
            "nbsp" -> return " "
            "hellip" -> return "…"
            "mdash" -> return "—"
            "ndash" -> return "–"
            "rsquo", "#8217" -> return "’"
            "lsquo" -> return "‘"
            "ldquo" -> return "“"
            "rdquo" -> return "”"
        }
        if (!entity.startsWith("#")) return null
        val digits = entity.substring(1)
        val value = if (digits.startsWith("x") || digits.startsWith("X")) {
            digits.substring(1).toIntOrNull(16)
        } else {
            digits.toIntOrNull()
        } ?: return null
        if (value !in 0..0x10FFFF || value in 0xD800..0xDFFF) return null
        return String(Character.toChars(value))
    }

    /**
     * Runs of whitespace (including the newlines inside a wrapped meta
     * tag, and the NBSP a decoded &nbsp; leaves behind) collapsed to
     * single spaces, then trimmed — the same normalisation Swift gets
     * from split(whereSeparator:)/joined(separator:).
     */
    fun collapseWhitespace(text: String): String {
        val out = StringBuilder(text.length)
        var pendingSpace = false
        for (ch in text) {
            if (ch.isWhitespace()) {
                if (out.isNotEmpty()) pendingSpace = true
                continue
            }
            if (pendingSpace) {
                out.append(' ')
                pendingSpace = false
            }
            out.append(ch)
        }
        return out.toString()
    }

    private fun firstNonEmpty(vararg candidates: String?): String? =
        candidates.firstOrNull { !it.isNullOrBlank() }?.trim()

    /**
     * Clamped to [limit] UNICODE CODE POINTS — the one unit both
     * platforms can count identically, and the one that cannot leave
     * half a surrogate pair behind (see MAX_TITLE_LENGTH).
     */
    private fun clamp(text: String, limit: Int): String {
        if (text.codePointCount(0, text.length) <= limit) return text
        val end = text.offsetByCodePoints(0, limit)
        return text.substring(0, end).trimEnd() + "…"
    }

    /**
     * Absolute form of a possibly-relative URL found in the page,
     * restricted to http(s) — a card must never point at file: or a
     * custom scheme just because a page said so.
     */
    fun absoluteUrl(raw: String, base: String): String? {
        val trimmed = raw.trim()
        if (trimmed.isEmpty()) return null
        val resolved = runCatching {
            val baseUri = URI(base)
            if (trimmed.startsWith("//")) {
                URI((baseUri.scheme ?: "https") + ":" + trimmed)
            } else {
                baseUri.resolve(trimmed)
            }
        }.getOrNull() ?: return null
        val scheme = resolved.scheme?.lowercase()
        if (scheme != "http" && scheme != "https") return null
        return resolved.toString()
    }
}
