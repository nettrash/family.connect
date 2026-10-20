/*
 * BubbleBodies.kt
 * Family Connect (Android)
 *
 * A message body as a bubble draws it: markdown applied, split into blocks,
 * and the tappable spans of each block — the markup's own links, what the
 * platform detector finds, and the members it names — resolved against that
 * block's RENDERED text.
 *
 * It used to be worked out inside MessageBubble, in a `remember`, which is
 * the right answer for a row that stays on screen and the wrong one for a
 * thread being scrolled: a LazyColumn disposes a row that scrolls away, so
 * every row that came back in ran the markdown pass and Linkify again — and
 * Linkify's phone matcher is the slowest thing a bubble does — on the main
 * thread, in the frame it had to appear in. Opening a chat after a long
 * absence and scrolling down through hundreds of new messages was exactly
 * that, a row at a time.
 *
 * Now the answer is kept by what it is a function of (the body, the
 * mentions, the locale the phone matcher reads), and the chat's view model
 * works it out for the rows of its window on a background thread before the
 * list asks ([prewarm]), so a row scrolling in usually finds it done. What
 * a bubble draws is exactly what it drew before: this is the same function,
 * remembered for longer and called earlier.
 */

package me.nettrash.familyconnect.ui.chat

import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import me.nettrash.familyconnect.data.net.dto.MentionsCodec
import me.nettrash.familyconnect.util.LruMemo
import java.util.Locale

/**
 * One block of a rendered body with the links that live INSIDE it.
 *
 * The pairing is the point: [links] index `block`'s own rendered string
 * and are meaningless against any other, so the two travel together from
 * the one place that resolves them down to the one layout result they are
 * hit-tested against. A table block carries none — its cells hold no links.
 */
internal data class BodyBlock(
    val block: MessageMarkdown.Block,
    val links: List<LinkSpan> = emptyList(),
)

internal object BubbleBodies {

    /**
     * Everything the answer depends on. The LOCALE because Linkify's phone
     * matcher reads the default country: the same digits can be a number in
     * one region and not in another, and a language change must not keep
     * handing back the old answer.
     */
    private data class Key(val body: String, val mentionsJson: String?, val locale: Locale)

    /**
     * A few screens' worth, and then some: a body is small, and the window
     * the view model pre-renders is a hundred rows to begin with.
     */
    private val memo = LruMemo<Key, List<BodyBlock>>(MEMO_CAPACITY)

    /**
     * The blocks of [body]. [emojiOnly] is the bubble's emoji-ladder
     * decision (EmojiOnly.displayFontSize != null): those bodies branch
     * around markdown entirely, and cost nothing to answer.
     */
    fun blocks(body: String, mentionsJson: String?, emojiOnly: Boolean): List<BodyBlock> {
        // Emoji-only bodies branch around it: the ladder's whole subject is
        // that the message is nothing but glyphs, so a markup pass could
        // only take something away, and there is nothing in one to detect.
        // Same rule as iOS and macOS.
        if (emojiOnly) return listOf(BodyBlock(MessageMarkdown.Block.Text(MessageMarkdown.plain(body))))
        val key = Key(body, mentionsJson, Locale.getDefault())
        return memo.getOrCompute(key) { render(body, mentionsJson) } ?: render(body, mentionsJson)
    }

    /**
     * Work out the bodies of [items] ahead of the list. Off the main thread
     * — the view model runs it on its builder's dispatcher — and stopped
     * the moment a newer window replaces it. Newest first, which is the
     * order the list is in and the end a chat opens at.
     */
    suspend fun prewarm(items: List<ChatListItem>) {
        for (item in items) {
            currentCoroutineContext().ensureActive()
            val entity = (item as? ChatListItem.MessageItem)?.entity ?: continue
            if (entity.body.isEmpty()) continue
            blocks(entity.body, entity.mentionsJson, emojiOnly = EmojiOnly.displayFontSize(entity.body) != null)
        }
    }

    /** How many bodies are held — for the tests. */
    internal val heldCount: Int get() = memo.size

    /** For the tests: start from nothing. */
    internal fun clear() = memo.clear()

    // MARKDOWN FIRST, and the order is load-bearing. Markdown DELETES
    // characters (`**`, backticks, `](url)`), so detecting links over the
    // raw body and drawing the rendered one would leave every link after
    // the first markup token pointing at the wrong glyphs — silently, with
    // nothing failing. Everything below indexes the block's own
    // `rendered.text`.
    //
    // Almost always ONE block: only a table splits a body, and each block
    // is then a whole offset space of its own — its links index its own
    // string, and the hit test gets one layout result per block. A span
    // from one block resolved against another's layout would point at
    // whatever glyphs happened to sit at those offsets.
    private fun render(body: String, mentionsJson: String?): List<BodyBlock> =
        MessageMarkdown.blocks(body).map { block ->
            when (block) {
                // The markup's own links first, then whatever the detector
                // finds in the text NOT already covered by one. Both index
                // this block's rendered text, and both feed one hit test.
                //
                // The overlap rule is not tidiness. `[https://www.paypal.com](https://evil.example)`
                // renders the label "https://www.paypal.com", which Linkify then
                // detects as a link to PayPal — two spans over the same glyphs,
                // one going somewhere else. Whichever the hit test picked, a tap
                // could open a destination the reader had every reason to think
                // was the one they could see. Dropping the detector's overlap
                // leaves exactly one answer: the destination the author wrote.
                is MessageMarkdown.Block.Text -> BodyBlock(
                    block = block,
                    // The members named ride the same list and the same hit
                    // test as links, under the same overlap rule — a link
                    // over a name is still the link.
                    //
                    // Neither may land on an inline formula: its characters
                    // are covered by a drawing, and its source is KaTeX's
                    // input, not words to tap (MessageLinks.outside).
                    links = MessageLinks.mergeSpans(
                        MessageLinks.mergeSpans(
                            block.rendered.links,
                            MessageLinks.outside(
                                MessageLinks.linkSpans(block.rendered.text),
                                block.rendered.math,
                            ),
                        ),
                        MessageLinks.outside(
                            MessageLinks.memberMentionSpans(
                                block.rendered.text,
                                MentionsCodec.decode(mentionsJson),
                            ),
                            block.rendered.math,
                        ),
                    ),
                )
                // A table's cells carry no links by construction
                // (MessageMarkdown.cell says why), so there is nothing
                // inside one to hit-test — and a rich block is a drawing,
                // with none either.
                is MessageMarkdown.Block.Table -> BodyBlock(block)
                is MessageMarkdown.Block.Rich -> BodyBlock(block)
            }
        }

    private const val MEMO_CAPACITY = 256
}
