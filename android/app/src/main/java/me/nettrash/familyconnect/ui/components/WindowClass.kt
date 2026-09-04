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

/**
 * Whether the chat list and a thread share the window side by side —
 * the iPad's split view. A window at least medium-wide (Material's
 * 600dp) that is not a phone lying down: a phone in landscape is ~900dp
 * wide and ~400dp tall, and two panes there would leave the thread four
 * lines high.
 */
@Composable
fun isTwoPaneWindow(): Boolean = windowWidthDp() >= 600 && windowHeightDp() >= 480

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

/** The readable column a thread, a form or a settings list is held to on a wide window (iOS: threadMaxWidth). */
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
