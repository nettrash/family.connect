//
//  LookupFooter.kt
//  FamilyConnect
//
//  The sources footer the SERVER appends to an answer that looked
//  something up (docs/protocol.md, "Looking things up" — "How sources are
//  shown"), recognised so the bubble can keep those links out of the
//  link-preview card (decision 7 of docs/information-streams-2026-10-03.md).
//
//  The footer itself needs nothing from this client: it is plain markdown,
//  and the bubble's renderer already makes every `[title](url)` in it a
//  tappable link. What it needs is for this device NOT to fetch the cited
//  pages on its own: a preview card is a request from every device that
//  shows the message to a site a web search chose, and the protocol's
//  answer to a page trying to steer the model is that the family's
//  devices never fetch what it cites unasked.
//
//  The shape, exactly as `server/src/lookups.rs` `finish_answer` writes it:
//
//      <answer>
//
//      Sources: [Title 1](url1) · [Title 2](url2) · [Title 3](url3)
//      [Weather data by Open-Meteo.com](https://open-meteo.com/) · Wikipedia, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/) · Powered by Brave
//
//  — a blank line, then a sources line, a credit line, or both. The
//  sources word and the credit words are translated (nine languages); the
//  link targets and "Powered by Brave" are fixed. A title never holds `[`,
//  `]`, `\` or a backtick, and a URL never holds whitespace or a
//  parenthesis (the server percent-encodes them), which is what lets one
//  strict pattern match every language without listing its words.
//
//  Kept free of Compose and Android so a plain JUnit test pins it.
//

package me.nettrash.familyconnect.ui.chat

object LookupFooter {

    /** One footer link: a bracket-free label and a URL with no space or parenthesis. */
    private const val LINK = """\[[^\[\]\n]+\]\([^\s()]+\)"""

    /** The separator between links and between credits: ` · `. */
    private const val DOT = """ · """

    /**
     * "Sources: …" in any language: a short label with no markup, a colon,
     * a space, then one to five links. The server sends at most three;
     * five leaves room without letting a whole paragraph of links pass.
     */
    private val SOURCES_LINE = Regex("""^[^\[\]()\n:]{1,40}: $LINK(?:$DOT$LINK){0,4}$""")

    /** Each of the three credits the server writes, in any language. */
    private val CREDITS = listOf(
        Regex("""^\[[^\[\]\n]+\]\(https://open-meteo\.com/\)$"""),
        Regex("""^[^\[\]()\n,]{1,40}, \[CC BY-SA 4\.0\]\(https://creativecommons\.org/licenses/by-sa/4\.0/\)$"""),
        Regex("""^Powered by Brave$"""),
    )

    private fun isCreditLine(line: String): Boolean {
        val credits = line.split(DOT)
        return credits.isNotEmpty() && credits.size <= CREDITS.size &&
            credits.all { credit -> CREDITS.any { it.matches(credit) } }
    }

    /**
     * The footer at the end of [body], or null when it ends in none.
     *
     * Only the LAST paragraph is looked at, and it must be the footer
     * whole: one or two lines, a sources line first if there is one, a
     * credit line last if there is one. Anything else — prose, a list, a
     * stray link — is the answer, and the answer's links are its own.
     */
    fun footer(body: String): String? {
        val trimmed = body.trimEnd()
        val start = trimmed.lastIndexOf("\n\n")
        if (start < 0) return null
        val tail = trimmed.substring(start + 2)
        // Something has to be above it: a footer is appended to an answer.
        if (trimmed.substring(0, start).isBlank()) return null
        val lines = tail.split('\n')
        val matches = when (lines.size) {
            1 -> SOURCES_LINE.matches(lines[0]) || isCreditLine(lines[0])
            2 -> SOURCES_LINE.matches(lines[0]) && isCreditLine(lines[1])
            else -> false
        }
        return tail.takeIf { matches }
    }

    /** Does [body] end in the server's sources footer? */
    fun isPresent(body: String): Boolean = footer(body) != null

    /**
     * Must the bubble draw NO link-preview card for this message?
     *
     * Only the assistant's own messages, because only the server writes
     * this footer and only on them; a member's message keeps its card
     * whatever it looks like. Within those, three cases:
     *
     * - **it ends in a sources footer** — every link in such an answer is
     *   one the lookup returned: the server removed every other link from
     *   the model's words before adding the footer. So "keep source links
     *   out of preview cards" is "no card for this message";
     * - **it is still being written** — the `ai_delta` text is the model's
     *   words BEFORE the server's link filter has run; only the finished
     *   row is filtered. A card fetched mid-stream could be for a link a
     *   web page talked the model into writing;
     * - **it stopped early** (`ai_error`) — the row keeps whatever deltas
     *   arrived, unfiltered, for the same reason.
     */
    fun suppressesPreview(
        fromAssistant: Boolean,
        body: String,
        isStreaming: Boolean,
        failed: Boolean,
    ): Boolean = fromAssistant && (isStreaming || failed || isPresent(body))
}
