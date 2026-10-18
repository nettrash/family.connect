/*
 * RichBlocks.kt
 * Family Connect (Android)
 *
 * Formulas, diagrams and graphs in a chat bubble (#97; docs/protocol.md,
 * "Rich blocks", and the design doc's "In the bubble").
 *
 * Two shapes, both drawn by RichRenderer:
 *  - a RICH BLOCK (MessageMarkdown.Block.Rich) is its own block, like a
 *    table: the drawing scaled down — never up — to the bubble's width, a
 *    diagram or a graph on its card with rounded corners and a tap that opens
 *    it full size;
 *  - an INLINE FORMULA is a run of a text block, swapped for its drawing
 *    through Compose's InlineTextContent once there is one.
 *
 * Until a drawing lands, and for good when it cannot be drawn, each shows
 * its SOURCE in the monospaced face exactly as a code block shows today —
 * which is also what a client that renders none of this shows, so a failure
 * reads as a message nobody dressed up, never as a broken picture.
 *
 * The drawing's accessibility label is its source (no new strings).
 */

package me.nettrash.familyconnect.ui.chat

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.wrapContentSize
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.InlineTextContent
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.Placeholder
import androidx.compose.ui.text.PlaceholderVerticalAlign
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.core.graphics.toColorInt
import me.nettrash.familyconnect.rich.RichJob
import me.nettrash.familyconnect.rich.RichRenderer
import me.nettrash.familyconnect.rich.RichResult

/**
 * The app's RichRenderer, provided by MainActivity. Null in previews and in
 * tests that do not provide one — every rich block then shows its source,
 * which is exactly the not-yet-drawn state.
 */
val LocalRichRenderer = staticCompositionLocalOf<RichRenderer?> { null }

/**
 * The widest an inline formula may be drawn in the bubble this composes in:
 * a formula wider than the line would overhang the balloon, and the text
 * layout cannot wrap a picture. Unbounded where no bubble said otherwise.
 */
val LocalRichMaxWidth = staticCompositionLocalOf { Dp.Infinity }

/**
 * [job]'s drawing: what memory already holds, at once, and otherwise null
 * until the renderer answers. Null as well when there is no renderer.
 */
@Composable
private fun rememberRichDrawing(job: RichJob): RichResult? {
    val renderer = LocalRichRenderer.current ?: return null
    var result by remember(job) { mutableStateOf(renderer.cached(job)) }
    LaunchedEffect(job) {
        if (result == null) result = renderer.draw(job)
    }
    return result
}

/**
 * The theme facts a job needs, from where this composes: is the theme dark
 * (decided from the surface itself rather than from a setting, so whatever
 * picked the scheme decides this too), the ink, and the bubble's body text
 * size in CSS pixels — dp, with the reader's font scale applied, because the
 * page knows nothing about font scale.
 */
@Composable
private fun richTheme(): Triple<Boolean, Int, Float> {
    val dark = MaterialTheme.colorScheme.surface.luminance() < 0.5f
    val ink = LocalContentColor.current.toArgb()
    val fontSize = MaterialTheme.typography.bodyMedium.fontSize.value * LocalDensity.current.fontScale
    return Triple(dark, ink, fontSize)
}

/**
 * A rich block in the bubble.
 *
 * [onDoubleTap] and [onLongPress] are the bubble's own gestures, re-emitted
 * over a diagram's card the way the link-preview card re-emits them: the
 * card's tap opens the full-size view, and a clickable child that consumed
 * every press would take the heart and the reaction capsule away from the
 * part of the bubble most people will touch.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
internal fun RichBlockView(
    block: MessageMarkdown.Block.Rich,
    onDoubleTap: (() -> Unit)?,
    onLongPress: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val (dark, ink, fontSize) = richTheme()
    val job = remember(block, dark, ink, fontSize) { RichJob.forBlock(block, dark, ink, fontSize) }
    val drawn = rememberRichDrawing(job) as? RichResult.Drawn
    if (drawn == null) {
        // Not drawn yet, or never: the source, as a code block shows it.
        if (block.shown.isNotEmpty()) {
            Text(
                text = block.shown,
                style = MaterialTheme.typography.bodyMedium,
                fontFamily = FontFamily.Monospace,
                modifier = modifier,
            )
        }
        return
    }
    val image = remember(drawn) { drawn.bitmap.asImageBitmap() }
    var viewing by remember { mutableStateOf(false) }
    val card = job.isCard
    Image(
        bitmap = image,
        contentDescription = block.source,
        // Fit within the box below, which is the drawing's own shape: the
        // natural size at most (never scaled UP to the bubble), the
        // bubble's width at least as a cap (scaled DOWN to it).
        contentScale = ContentScale.Fit,
        modifier = modifier
            .widthIn(max = drawn.width.dp)
            .aspectRatio(drawn.width / drawn.height)
            .then(
                if (card) {
                    // The card's own ground under the picture as well: the
                    // page rounds a diagram's box UP to a whole CSS pixel, so
                    // the snapshot's last column and row can be part card,
                    // part nothing — a hairline of bubble showing through at
                    // the right and bottom edges without this.
                    Modifier
                        .clip(RoundedCornerShape(CARD_CORNER))
                        .background(Color(job.background.toColorInt()))
                } else {
                    Modifier
                },
            )
            .then(
                if (card) {
                    Modifier.combinedClickable(
                        interactionSource = remember { MutableInteractionSource() },
                        indication = null,
                        onClick = { viewing = true },
                        onDoubleClick = onDoubleTap,
                        onLongClick = onLongPress,
                    )
                } else {
                    Modifier
                },
            ),
    )
    if (viewing) {
        RichDrawingViewer(
            image = image,
            contentDescription = block.source,
            onDismiss = { viewing = false },
        )
    }
}

/**
 * The InlineTextContent for every inline formula in [text] that has been
 * drawn — keyed by the id MessageMarkdown put on its run. A formula not in
 * the map is drawn by Compose as the run's own text, its source in the
 * monospaced face, which is the not-yet-drawn state for free.
 *
 * Asks for every formula it finds, so calling this is what starts the
 * drawing. Empty (and allocation-free) for the text that has no formula,
 * which is every ordinary message.
 */
@Composable
internal fun rememberInlineMath(text: AnnotatedString): Map<String, InlineTextContent> {
    val sources = remember(text) { MessageMarkdown.mathSources(text) }
    if (sources.isEmpty()) return emptyMap()
    val (dark, ink, fontSize) = richTheme()
    val maxWidth = LocalRichMaxWidth.current
    val density = LocalDensity.current
    val drawings = sources.map { source ->
        key(source) {
            val job = remember(source, dark, ink, fontSize) { RichJob.forInline(source, dark, ink, fontSize) }
            source to (rememberRichDrawing(job) as? RichResult.Drawn)
        }
    }
    return remember(drawings, maxWidth, density) {
        drawings.mapNotNull { (source, drawn) ->
            drawn ?: return@mapNotNull null
            // Scaled down — never up — to the line, all three numbers alike.
            val fit = if (maxWidth == Dp.Infinity) 1f else minOf(1f, maxWidth.value / drawn.width)
            val width = drawn.width * fit
            val height = drawn.height * fit
            val baseline = drawn.baseline * fit
            val image = drawn.bitmap.asImageBitmap()
            MessageMarkdown.MATH_ID_PREFIX + source to inlineFormula(image, width, height, baseline, density)
        }.toMap()
    }
}

/**
 * One drawn inline formula as a placeholder in the line.
 *
 * ON THE BASELINE, as nearly as Compose allows: a placeholder can sit on the
 * baseline only by its BOTTOM (AboveBaseline), and a formula has a part below
 * its baseline — a subscript, the lower half of a fraction. So the
 * placeholder is only the part ABOVE the baseline, sitting on it, and the
 * drawing hangs out of it by its descent, the way the letters g and y hang
 * below the line beside it. The line makes room for the tall part (a
 * placeholder taller than the line raises the line) but not for the descent,
 * which lands in the space between lines that descenders already use.
 */
private fun inlineFormula(
    image: ImageBitmap,
    width: Float,
    height: Float,
    baseline: Float,
    density: androidx.compose.ui.unit.Density,
): InlineTextContent {
    val above = baseline.coerceAtLeast(1f)
    val placeholder = with(density) {
        Placeholder(
            width = width.dp.toSp(),
            height = above.dp.toSp(),
            placeholderVerticalAlign = PlaceholderVerticalAlign.AboveBaseline,
        )
    }
    return InlineTextContent(placeholder) {
        Box(
            modifier = Modifier
                .fillMaxSize()
                .wrapContentSize(Alignment.TopStart, unbounded = true),
        ) {
            // The text's own semantics already read the run's source; a
            // second label here would read it twice.
            Image(bitmap = image, contentDescription = null, modifier = Modifier.size(width.dp, height.dp))
        }
    }
}

private val CARD_CORNER = 10.dp
