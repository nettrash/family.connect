/*
 * StickerImage.kt
 * Family Connect (Android)
 *
 * How a chat STICKER is drawn (docs/protocol.md, "Sticker pack" → "How it
 * is drawn"): the picture alone, fitted WHOLE into one fixed box, its
 * transparency showing whatever is behind it — animated where this platform
 * can, frame zero where it cannot, and both are correct.
 *
 * FROM THE ORIGINAL BYTES, ALWAYS. A preview is a JPEG — no transparency,
 * one frame — so nothing here ever asks for one, whatever `has_preview`
 * says. That is why this takes a FILE and not the decoded bitmap
 * AttachmentRepository keeps for photographs: an animated sticker is played
 * from its bytes, and a Bitmap is a single frame.
 *
 * ANIMATION is the platform's own `AnimatedImageDrawable` (API 28+), which
 * `ImageDecoder` hands back for an animated WebP. No image-loading library:
 * this app's dependency posture is deliberately minimal
 * (gradle/libs.versions.toml), and the one class it would be added for is
 * already in the framework. Below API 28 — and for a format the decoder
 * does not animate, an animated PNG among them — what is drawn is frame
 * zero, through BitmapFactory.
 *
 * "Sticker" here is the chat picture, not a board note.
 *
 * iOS counterpart: the sticker view in the chat bubble.
 */

package me.nettrash.familyconnect.ui.stickers

import android.content.res.Resources
import android.graphics.BitmapFactory
import android.graphics.ImageDecoder
import android.graphics.drawable.Animatable
import android.graphics.drawable.AnimatedImageDrawable
import android.graphics.drawable.BitmapDrawable
import android.graphics.drawable.Drawable
import android.os.Build
import android.os.Handler
import android.os.Looper
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.LocalContentColor
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.RememberObserver
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.graphics.painter.Painter
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.core.graphics.drawable.toDrawable
import kotlinx.coroutines.Dispatchers
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.repo.PackPicture
import me.nettrash.familyconnect.ui.chat.animationsRemoved
import me.nettrash.familyconnect.ui.components.LocalAttachments
import me.nettrash.familyconnect.ui.components.LocalMotionHold
import kotlinx.coroutines.withContext
import java.io.File
import kotlin.math.roundToInt

/** The numbers a sticker is drawn at, and the decode that feeds them. */
object StickerDrawing {

    /**
     * The ONE box every sticker in a chat is fitted into on this client.
     *
     * Larger than an emoji (the emoji-only ladder tops out at 96sp) and
     * smaller than a photograph (a photo tile is up to 240dp wide) — never
     * the picture's own pixel size, which would make a 96-pixel sticker a
     * speck and a 2000-pixel one a poster.
     *
     * 160 on every client — points, dp, CSS pixels — so a conversation is
     * the same size whichever screen it is read on.
     */
    val CHAT_BOX = 160.dp

    /** The larger view a tap opens. */
    val ENLARGED_BOX = 288.dp

    /**
     * The longest edge ANY box is decoded at. Twice the 512 a made sticker
     * is fitted to, so the enlarged view stays sharp on a dense screen.
     *
     * A ceiling, not the size every sticker is decoded at: each box asks
     * for its own pixels ([decodeEdge]). The pack takes a finished sticker
     * as given whatever its pixel size, and flat-colour art at 2000 x 2000
     * fits well inside the byte ceiling — decoded at that size it is 16 MB
     * a cell, and a panel shows two dozen cells at once.
     */
    const val DECODE_EDGE = 1024

    /**
     * The pixels a box [boxPx] on a side is decoded for: its own size, and
     * never more than [DECODE_EDGE]. A box not yet measured decodes at the
     * ceiling rather than at nothing.
     */
    fun decodeEdge(boxPx: Int): Int = if (boxPx <= 0) DECODE_EDGE else minOf(boxPx, DECODE_EDGE)

    /**
     * The size [width] x [height] is decoded at for a box of [edge] pixels,
     * or null to decode it as it is: fitted WHOLE inside the box, never
     * enlarged — a 96-pixel sticker is left at 96 and scaled up by the
     * layout, which costs nothing.
     *
     * An exact size, which a power-of-two sample cannot give: sampling a
     * 2000-pixel sticker for a 1024 box leaves it at 2000 (one more halving
     * would undershoot), and that is the very case the ceiling is for.
     */
    fun targetSize(width: Int, height: Int, edge: Int = DECODE_EDGE): Pair<Int, Int>? {
        if (width <= 0 || height <= 0 || edge <= 0) return null
        if (width <= edge && height <= edge) return null
        return PackPicture.fit(width, height, edge)
    }

    /**
     * Below API 28 only, where the decoder takes a sample and not a size:
     * the power-of-two factor that brings [width] x [height] down to
     * [edge] or just above it — never below, so what is drawn is only ever
     * scaled DOWN by the layout, and never as much as twice the edge.
     */
    fun sampleSize(width: Int, height: Int, edge: Int = DECODE_EDGE): Int {
        if (width <= 0 || height <= 0 || edge <= 0) return 1
        var sample = 1
        var largest = maxOf(width, height)
        while (largest / 2 >= edge) {
            largest /= 2
            sample *= 2
        }
        return sample
    }

    /**
     * Decode a sticker's bytes for drawing, or null when they are not an
     * image this device can read. Blocking — call it off the main thread.
     *
     * An animated WebP comes back as an [AnimatedImageDrawable] on API 28+
     * and as its first frame below that.
     *
     * [edge] is the box it is decoded FOR, in pixels ([decodeEdge]).
     */
    fun decode(file: File, edge: Int = DECODE_EDGE): Drawable? = runCatching {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            ImageDecoder.decodeDrawable(ImageDecoder.createSource(file)) { decoder, info, _ ->
                targetSize(info.size.width, info.size.height, edge)?.let { (width, height) ->
                    decoder.setTargetSize(width, height)
                }
            }.also { drawable ->
                // A sticker loops for as long as it is on screen, whatever
                // loop count its file was exported with.
                if (drawable is AnimatedImageDrawable) {
                    drawable.repeatCount = AnimatedImageDrawable.REPEAT_INFINITE
                }
            }
        } else {
            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeFile(file.path, bounds)
            if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
            val options = BitmapFactory.Options().apply {
                inSampleSize = sampleSize(bounds.outWidth, bounds.outHeight, edge)
            }
            // The system's resources only to satisfy the constructor: the
            // density that matters is set where the drawable is drawn.
            BitmapFactory.decodeFile(file.path, options)?.toDrawable(Resources.getSystem())
        }
        // runCatching: a truncated download or a hostile file must cost one
        // sticker its picture, not the chat its process.
    }.getOrNull()
}

/**
 * A sticker, drawn from its original bytes.
 *
 * [load] fetches the file — from the pack's own store, or from the
 * attachment cache for a sticker somebody sent — and may answer null, in
 * which case nothing is drawn and [retryKey] changing asks again.
 *
 * The caller gives the BOX through [modifier]; the picture is fitted whole
 * inside it and never cropped. [decodeBox] says how large that box can be,
 * so a panel cell does not hold the pixels an enlarged view needs.
 */
@Composable
fun StickerImage(
    /** What identifies the bytes: an attachment id never names different ones. */
    key: Any,
    load: suspend () -> File?,
    contentDescription: String?,
    modifier: Modifier = Modifier,
    /** Bumped when a failed fetch is worth repeating (the network returned). */
    retryKey: Any? = null,
    /** Reports whether the picture has landed, for a caller that draws a placeholder. */
    onLoaded: (Boolean) -> Unit = {},
    /**
     * The largest edge this picture is drawn at. What it is DECODED for —
     * the layout still decides what it is drawn at.
     */
    decodeBox: Dp = StickerDrawing.ENLARGED_BOX,
) {
    val edge = StickerDrawing.decodeEdge(with(LocalDensity.current) { decodeBox.roundToPx() })
    // NOT keyed on [key]: the sender's own sticker changes id when its
    // upload lands (a negative placeholder becomes the server's), and
    // dropping the picture for the frames the second decode takes would
    // blink it. What is shown stays until its replacement is ready.
    //
    // It STARTS from the drawable this very sticker left behind when it
    // last scrolled away, when there is one (StickerDrawables): a row
    // scrolling back in used to read the file and decode it all over again,
    // drawing an empty box until it had.
    var shown by remember { mutableStateOf(StickerDrawables.take(key, edge)?.let { Shown(key, edge, it) }) }
    LaunchedEffect(key, retryKey) {
        if (shown?.key == key) {
            // Landed already — from the pool, before this effect ran. Said,
            // so a caller drawing a placeholder takes it down; saying it a
            // second time after a retry is the same answer again.
            onLoaded(true)
            return@LaunchedEffect
        }
        val file = load() ?: return@LaunchedEffect
        val decoded = withContext(Dispatchers.IO) { StickerDrawing.decode(file, edge) }
        if (decoded != null) shown = Shown(key, edge, decoded)
        onLoaded(decoded != null)
    }
    val held = shown ?: return
    val current = held.drawable
    // Back to the pool when this sticker leaves the screen or its picture is
    // replaced. Declared BEFORE the painter on purpose: what was remembered
    // first is forgotten last, so the painter has stopped the drawable and
    // let go of its callback by the time anybody else can take it.
    DisposableEffect(held) { onDispose { StickerDrawables.give(held.key, held.edge, held.drawable) } }
    // The density goes onto a bitmap drawable so its intrinsic size is in
    // the same units the painter reports; the layout then fits it whole.
    val resources = LocalResources.current
    // Frame zero, still, whenever the system asks for less motion — in the
    // chat, the panel, the pack and the enlarged view alike (docs/protocol.md,
    // "Sticker pack", amended for #96). An AnimatedImageDrawable that is never
    // started draws its first frame.
    val still = animationsRemoved()
    val hold = LocalMotionHold.current
    val painter = remember(current, still) {
        (current as? BitmapDrawable)?.setTargetDensity(resources.displayMetrics)
        DrawablePainter(current, animate = !still, hold = hold)
    }
    // Still while the thread scrolls or a viewer is over it (LocalMotionHold).
    // Read here, in an effect, so a scroll starting or stopping reaches the
    // drawable without recomposing the sticker.
    LaunchedEffect(painter, hold) { snapshotFlow { hold.value }.collect { painter.setHeld(it) } }
    Image(
        painter = painter,
        contentDescription = contentDescription,
        // Fit, never Crop: a sticker is shown whole.
        contentScale = ContentScale.Fit,
        modifier = modifier,
    )
}

/**
 * A sticker somebody SENT, as it sits in a thread: no bubble, the one fixed
 * box, the picture whole, transparency showing the chat behind it.
 *
 * It IS the message, so it carries the bubble's own gestures — tap to see
 * it larger, long-press for the reaction menu, double-tap for the heart —
 * exactly as a caption-less photo tile does, and for the same reason: a
 * child that takes the press is the only thing the finger can reach.
 *
 * No ripple: a rectangle flashing behind a transparent picture draws the
 * very box a sticker exists not to have.
 *
 * The bytes are the ORIGINAL ones, asked of the attachment cache by id —
 * never the preview, whatever `has_preview` says.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun ChatSticker(
    attachment: AttachmentDto,
    onOpen: () -> Unit,
    onLongPress: () -> Unit,
    onDoubleTap: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val attachments = LocalAttachments.current
    var loaded by remember { mutableStateOf(false) }
    val description = stringResource(R.string.s_sticker)
    Box(
        modifier = modifier
            .size(StickerDrawing.CHAT_BOX)
            // A faint wash only until the picture lands: before that this
            // square is all there is to look at, and after it the sticker's
            // own outline is the edge.
            .then(
                if (loaded) {
                    Modifier
                } else {
                    Modifier.background(
                        LocalContentColor.current.copy(alpha = 0.08f),
                        RoundedCornerShape(14.dp),
                    )
                },
            )
            .combinedClickable(
                interactionSource = remember { MutableInteractionSource() },
                indication = null,
                onClick = onOpen,
                onLongClick = onLongPress,
                onDoubleClick = onDoubleTap,
            )
            .semantics { contentDescription = description },
    ) {
        StickerImage(
            key = attachment.id,
            load = { attachments?.originalFile(attachment.id) },
            // Said once, by the box.
            contentDescription = null,
            modifier = Modifier.fillMaxSize(),
            retryKey = attachments?.retryToken,
            onLoaded = { loaded = it },
            decodeBox = StickerDrawing.CHAT_BOX,
        )
    }
}

/** A decoded sticker on screen: what it is, the box it was decoded for, and the drawable. */
private class Shown(val key: Any, val edge: Int, val drawable: Drawable)

/**
 * Decoded stickers that are OFF screen right now, kept so the same sticker
 * coming back — a row scrolling back into the thread, the panel reopening —
 * is drawn at once instead of being read and decoded again.
 *
 * A checkout pool, not a shared cache: a Drawable has ONE callback and ONE
 * set of bounds, so two places drawing one instance at the same time would
 * fight over both (the second to come on screen would steal the first's
 * redraws, and the first to leave would stop the other's animation). So a
 * drawable is TAKEN out while it is on screen and GIVEN back when it leaves;
 * a second copy of a sticker already showing — the recents row and the pack
 * grid in the panel — simply decodes its own, as it always did.
 *
 * Keyed by what identifies the bytes (an attachment or pack-item id — an id
 * never names different bytes) and by the box edge it was decoded for, so a
 * panel cell never hands its small decode to the enlarged view. Bounded by
 * an estimate of the pixels it holds; main thread only, like the
 * composition that uses it.
 */
internal object StickerDrawables {

    /**
     * What the pool may hold, in estimated bytes of pixels: a couple of
     * screens of chat stickers at a dense screen's 160dp box. Bitmaps live
     * in the native heap on every API this app supports, but they are still
     * the app's memory, so the pool stays small and the oldest go first.
     */
    private const val BUDGET_BYTES = 16L * 1024 * 1024

    private val pool = CheckoutPool<Pair<Any, Int>, Drawable>(BUDGET_BYTES) { estimatedBytes(it) }

    /** The pooled decode of [key] for [edge], removed from the pool, or null. */
    fun take(key: Any, edge: Int): Drawable? = pool.take(key to edge)

    /** [drawable] is off screen again: keep it for the next time [key] is wanted at [edge]. */
    fun give(key: Any, edge: Int, drawable: Drawable) = pool.give(key to edge, drawable)

    /** For the tests: start from nothing. */
    internal fun clear() = pool.clear()

    /**
     * The pixels a drawable holds, roughly: a bitmap's real allocation, and
     * for anything else (an animated drawable keeps a frame or two of its
     * own) its intrinsic size at four bytes a pixel, twice over.
     */
    internal fun estimatedBytes(drawable: Drawable): Long {
        (drawable as? BitmapDrawable)?.bitmap?.let { return it.allocationByteCount.toLong() }
        val pixels = drawable.intrinsicWidth.coerceAtLeast(1).toLong() * drawable.intrinsicHeight.coerceAtLeast(1)
        return pixels * 4 * 2
    }
}

/**
 * A size-bounded checkout pool: [take] removes what it hands out, [give]
 * puts it back as the most recently used, and the least recently given go
 * first once [budget] is passed. One entry per key — giving a second value
 * under a key replaces the first. Not thread-safe; see [StickerDrawables].
 */
internal class CheckoutPool<K : Any, V : Any>(
    private val budget: Long,
    private val sizeOf: (V) -> Long,
) {
    private val entries = LinkedHashMap<K, V>(16, 0.75f, true)
    private var held = 0L

    /** Estimated bytes held now — for the tests. */
    val heldBytes: Long get() = held

    /** How many values are held now — for the tests. */
    val size: Int get() = entries.size

    fun take(key: K): V? {
        val value = entries.remove(key) ?: return null
        held -= sizeOf(value)
        return value
    }

    fun give(key: K, value: V) {
        val size = sizeOf(value)
        entries.remove(key)?.let { held -= sizeOf(it) }
        // One value bigger than the whole budget is not kept at all: it
        // would only push everything else out and then be evicted itself.
        if (size > budget) return
        entries[key] = value
        held += size
        val oldestFirst = entries.entries.iterator()
        while (held > budget && oldestFirst.hasNext()) {
            val eldest = oldestFirst.next()
            held -= sizeOf(eldest.value)
            oldestFirst.remove()
        }
    }

    fun clear() {
        entries.clear()
        held = 0
    }
}

/**
 * A [Painter] over a platform [Drawable] that may animate.
 *
 * An `AnimatedImageDrawable` advances itself and asks its callback to
 * redraw; Compose has no View to be that callback, so this is one — every
 * `invalidateDrawable` bumps a state the draw reads, which is what makes
 * the next frame happen. The animation runs only while the painter is in
 * the composition: a sticker scrolled out of a LazyColumn stops costing
 * frames, and one scrolled back in starts again.
 */
private class DrawablePainter(
    private val drawable: Drawable,
    /** False: frame zero, never started — what "animations off" asks for. */
    private val animate: Boolean = true,
    /**
     * Whether to hold still for now (LocalMotionHold): read once when the
     * painter comes on screen, and followed after that through [setHeld].
     */
    private val hold: State<Boolean>? = null,
) : Painter(), RememberObserver {

    private var tick by mutableIntStateOf(0)
    private val handler = Handler(Looper.getMainLooper())
    private var onScreen = false
    private var held = false

    /**
     * Hold the animation still, or let it run again. A held sticker STOPS:
     * on a hardware canvas an AnimatedImageDrawable is advanced by the render
     * thread itself, which keeps drawing the window for as long as one runs —
     * not drawing it here would not be enough. It shows the frame it stopped
     * on; started again, the platform may begin the loop from the top, which
     * is the same thing a sticker scrolling back into view has always done.
     */
    fun setHeld(value: Boolean) {
        if (held == value) return
        held = value
        if (!onScreen || !animate) return
        val animatable = drawable as? Animatable ?: return
        if (value) animatable.stop() else animatable.start()
    }

    private val callback = object : Drawable.Callback {
        override fun invalidateDrawable(who: Drawable) {
            tick++
        }

        override fun scheduleDrawable(who: Drawable, what: Runnable, time: Long) {
            handler.postAtTime(what, time)
        }

        override fun unscheduleDrawable(who: Drawable, what: Runnable) {
            handler.removeCallbacks(what)
        }
    }

    override val intrinsicSize: Size
        get() {
            val width = drawable.intrinsicWidth
            val height = drawable.intrinsicHeight
            return if (width > 0 && height > 0) Size(width.toFloat(), height.toFloat()) else Size.Unspecified
        }

    override fun DrawScope.onDraw() {
        // Read so that a bump above invalidates this draw.
        tick
        drawIntoCanvas { canvas ->
            drawable.setBounds(0, 0, size.width.roundToInt(), size.height.roundToInt())
            drawable.draw(canvas.nativeCanvas)
        }
    }

    override fun onRemembered() {
        onScreen = true
        // Outside composition, so this read subscribes nothing.
        held = hold?.value == true
        drawable.callback = callback
        drawable.setVisible(true, true)
        if (animate && !held) (drawable as? Animatable)?.start()
    }

    override fun onForgotten() = stop()

    override fun onAbandoned() = stop()

    private fun stop() {
        onScreen = false
        (drawable as? Animatable)?.stop()
        drawable.setVisible(false, false)
        drawable.callback = null
        handler.removeCallbacksAndMessages(null)
    }
}
