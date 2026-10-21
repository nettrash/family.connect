/*
 * WindowClass.kt
 * Family Connect (Android)
 *
 * The few size decisions the app makes from the WINDOW — not the display:
 * a split-screen half, a Chromebook window and a foldable's outer panel
 * all differ from the panel behind them. Read from LocalWindowInfo, which
 * is the window, and kept to three questions so the rest of the UI can
 * ask them by name rather than by number.
 *
 * iOS counterpart: the idiom/size-class reads in ChatListView (two
 * shapes), AttachmentView (tile cap) and ConversationView.threadMaxWidth.
 */

package me.nettrash.familyconnect.ui.components

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.wrapContentWidth
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.layout
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/** The window's width in dp. */
@Composable
fun windowWidthDp(): Int {
    val size = LocalWindowInfo.current.containerSize
    return (size.width / LocalDensity.current.density).toInt()
}

/** The window's height in dp. */
@Composable
fun windowHeightDp(): Int {
    val size = LocalWindowInfo.current.containerSize
    return (size.height / LocalDensity.current.density).toInt()
}

/** The list pane's width beside a conversation: the iPad sidebar's 320pt, which fits a two-line family name beside its time. */
const val LIST_PANE_WIDTH_DP = 340

/**
 * The narrowest the CONVERSATION may be left beside the list (#106,
 * docs/composer-and-wide-layout-2026-10-20.md, section 3, fix 3). The
 * old rule split any 600dp window, which left the conversation 259dp —
 * a composer field 35dp wide, where the video button alone needs 48 —
 * so the split now waits until the conversation keeps a phone's width.
 */
const val MIN_CONVERSATION_PANE_DP = 400

/** The hairline between the list and the conversation. */
private const val PANE_DIVIDER_DP = 1

/**
 * Whether the chat list and a thread share a window of this size side by
 * side — the iPad's split view. Two conditions, both about what is LEFT
 * for the conversation rather than about the window as such:
 *
 *  - beside the [LIST_PANE_WIDTH_DP] list (and its divider) the
 *    conversation keeps at least [MIN_CONVERSATION_PANE_DP] — so the
 *    split starts at 741dp, not at Material's 600dp, and a 600dp window
 *    (a tablet's split-screen half, a small foldable) gets the phone's
 *    shape, a whole screen of conversation;
 *  - the window is not a phone lying down: a phone in landscape is
 *    ~900dp wide and ~400dp tall, and two panes there would leave the
 *    thread four lines high.
 *
 * Pure, so the threshold is pinned by a unit test rather than by a
 * screenshot.
 */
fun isTwoPane(widthDp: Int, heightDp: Int): Boolean =
    widthDp - LIST_PANE_WIDTH_DP - PANE_DIVIDER_DP >= MIN_CONVERSATION_PANE_DP && heightDp >= 480

/** [isTwoPane], asked of this window. */
@Composable
fun isTwoPaneWindow(): Boolean = isTwoPane(windowWidthDp(), windowHeightDp())

/**
 * Whether the window is at least medium-wide: tablets, foldables open,
 * most desktop windows. Media tiles and sticky notes grow with it.
 */
@Composable
fun isWideWindow(): Boolean = windowWidthDp() >= 600

/**
 * Widest a bubble's media gets. The phone's 240dp everywhere a window is
 * phone-narrow — a third of a tablet in split screen included, or the
 * tile would overrun it — and 320dp on a wide window, where a 240dp tile
 * in a 560dp column read as a postage stamp (iOS: AttachmentView).
 */
@Composable
fun attachmentMaxWidth(): Dp = if (isWideWindow()) 320.dp else 240.dp

/**
 * The readable column a form or a settings list is held to on a wide
 * window. The CONVERSATION no longer is (#106): it has [ConversationColumn].
 */
val READABLE_COLUMN: Dp = 560.dp

/**
 * Hold content to [READABLE_COLUMN], centred, on any window wider than it;
 * a no-op on a phone, which is narrower. The outer fill takes the window,
 * wrapContentWidth lets the inner be narrower and centres it, widthIn
 * caps it, and the inner fill takes everything up to the cap.
 */
fun Modifier.readableColumn(): Modifier = this
    .fillMaxWidth()
    .wrapContentWidth(Alignment.CenterHorizontally)
    .widthIn(max = READABLE_COLUMN)
    .fillMaxWidth()

/**
 * The conversation's column on any window (#106,
 * docs/composer-and-wide-layout-2026-10-20.md, section 2.2) — one rule on
 * all six clients:
 *
 *  - the COLUMN, messages and composer alike, is `min(pane − 2 × gutter,
 *    1024)`, centred, so the composer is exactly as wide as the messages
 *    above it. It replaces #44's 560dp cap on the whole thread, which on a
 *    1280dp tablet left 379dp of the conversation pane empty and squeezed
 *    the composer into a phone's width under a pane-wide bar;
 *  - a BUBBLE is at most `min(column − 48, 560)` — #44's own worry, a
 *    phone-sized photo in a pane-wide balloon, stays solved by capping the
 *    balloon rather than the column. On Android the phone's bubble was
 *    never the 48dp spacer but four fifths of the row, and the phone's
 *    bubble must not change, so that bound stays too: the smallest of the
 *    three wins, which on every phone is the four fifths it always was;
 *  - media in a bubble keeps [attachmentMaxWidth], unchanged.
 *
 * The GUTTER is the doc's 16dp on a wide window and, below 600dp, the
 * 12dp the phone's thread has always had: 16 there would narrow every
 * phone's bubbles by 6dp, which is a change nobody asked for. The two
 * meet at 600dp, where the media cap already steps.
 *
 * Plain arithmetic on [Dp], no Compose runtime, so every number is pinned
 * by an ordinary unit test.
 */
object ConversationColumn {
    /** Widest the column gets: a reply stays near the message it answers on a 13" tablet. */
    val MAX: Dp = 1024.dp

    /** Widest a bubble gets — a line of text stays readable. */
    val BUBBLE_MAX: Dp = 560.dp

    /** What a bubble leaves of the row at least: the other side's 48dp spacer. */
    val BUBBLE_SPACER: Dp = 48.dp

    /** The phone's bound on a bubble, four fifths of its row, unchanged since the first build. */
    const val BUBBLE_FRACTION: Float = 0.8f

    /** The gutter between the pane's edge and the column below [WIDE_FROM]. */
    val PHONE_GUTTER: Dp = 12.dp

    /** The gutter on a wide window: the doc's 2 × 16. */
    val WIDE_GUTTER: Dp = 16.dp

    /** Where a window counts as wide — the same 600dp as [isWideWindow]. */
    val WIDE_FROM: Dp = 600.dp

    /** The composer row's own horizontal padding, which the column already includes. */
    val COMPOSER_ROW_PADDING: Dp = 8.dp

    fun gutter(pane: Dp): Dp = if (pane >= WIDE_FROM) WIDE_GUTTER else PHONE_GUTTER

    /** The column's width in a pane this wide. */
    fun width(pane: Dp): Dp = minOf(pane - gutter(pane) * 2, MAX).coerceAtLeast(0.dp)

    /**
     * The message list's horizontal content padding: what centres the
     * column — the gutter on any pane up to 1024dp plus the gutters, and
     * then whatever the cap leaves, split evenly.
     */
    fun listPadding(pane: Dp): Dp = ((pane - width(pane)) / 2).coerceAtLeast(0.dp)

    /**
     * How far in from the pane's edges the composer is drawn — its strips
     * and its row. Zero up to the cap: on a phone the bar is the width it
     * always was, and its row keeps its own 8dp padding. Past the cap, the
     * row's buttons start [COMPOSER_ROW_PADDING] outside the column, as
     * they do on every wide pane below it: the paperclip's 48dp target
     * reaches 12dp past its glyph, so the glyph lines up with the messages.
     */
    fun composerInset(pane: Dp): Dp = (listPadding(pane) - gutter(pane)).coerceAtLeast(0.dp)

    /**
     * Widest a bubble may be in a row this wide. The row is the column —
     * the list is padded to it — so `column − 48` is `row − 48`.
     */
    fun bubbleMaxWidth(row: Dp): Dp =
        minOf(row * BUBBLE_FRACTION, row - BUBBLE_SPACER, BUBBLE_MAX).coerceAtLeast(0.dp)
}

/**
 * Hold the composer to the conversation column ([ConversationColumn.composerInset]),
 * centred. A layout modifier rather than a BoxWithConstraints: the
 * composer holds the focused text field, and a subcomposition around it
 * buys nothing but a second pass per keystroke.
 */
fun Modifier.conversationComposerColumn(): Modifier = this
    .fillMaxWidth()
    .layout { measurable, constraints ->
        if (!constraints.hasBoundedWidth) {
            val placeable = measurable.measure(constraints)
            return@layout layout(placeable.width, placeable.height) { placeable.place(0, 0) }
        }
        val inset = ConversationColumn.composerInset(constraints.maxWidth.toDp()).roundToPx()
        val inner = (constraints.maxWidth - inset * 2).coerceAtLeast(0)
        val placeable = measurable.measure(
            constraints.copy(minWidth = inner, maxWidth = inner),
        )
        layout(constraints.maxWidth, placeable.height) { placeable.place(inset, 0) }
    }
