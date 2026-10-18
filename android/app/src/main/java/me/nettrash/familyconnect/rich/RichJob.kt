/*
 * RichJob.kt
 * Family Connect (Android)
 *
 * One rich block to draw (#97: formulas, Mermaid diagrams, Graphviz graphs,
 * plots), the key its drawing is kept under, and the snapshot arithmetic —
 * everything about a drawing that is pure data, so the JVM tests reach it
 * without a WebView (RichJobTest).
 *
 * The job is fc-rich.js's JOB (its header is the contract; the design doc,
 * docs/capitals-and-rich-markdown-2026-10-17.md, "The job a client sends",
 * says which value each field takes). Colours are CSS strings because that
 * is what the page reads, and because the key hashes exactly the string the
 * page was given — two spellings of one colour cost a second drawing, never
 * a wrong one.
 */

package me.nettrash.familyconnect.rich

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import me.nettrash.familyconnect.ui.chat.MessageMarkdown
import java.security.MessageDigest
import java.util.Locale
import kotlin.math.ceil
import kotlin.math.roundToInt

/**
 * fc-rich.js's JOB.
 *
 * [fontSize] is in CSS pixels, which on Android are dp: the bubble's body
 * text size with the reader's font scale already applied, because the page
 * is told nothing about font scale and its text must match the bubble's.
 * [engine] is null for everything but Graphviz, and is then left out of the
 * JSON rather than sent as `null`.
 */
@Serializable
data class RichJob(
    val kind: String,
    val source: String,
    val engine: String? = null,
    val dark: Boolean,
    val color: String,
    val background: String,
    val fontSize: Float,
) {

    /** A display formula, a diagram or a graph: drawn on its own line. */
    val isMath: Boolean get() = kind == KIND_MATH || kind == KIND_MATH_INLINE

    /** Drawn on a card (a diagram or a graph) rather than bare ink. */
    val isCard: Boolean get() = !isMath

    /** The job as the page reads it — `fcRich.drawAndPost(id, <this>)`. */
    fun toJson(): String = JSON.encodeToString(serializer(), this)

    /**
     * The key this job's drawing is kept under: SHA-256, in hex, of
     * fc-rich.js's [version] and every field that changes the pixels.
     *
     * The fields are joined with NUL, which none of the fixed ones can
     * contain, and the SOURCE goes last — so no two different jobs can
     * serialise to one string, whatever the source holds. The version is
     * what makes a drawing from an older engine unreachable the moment the
     * files change (web/rich's rule: bump VERSION with any file).
     */
    fun cacheKey(version: String): String {
        val text = listOf(
            version,
            kind,
            engine.orEmpty(),
            dark.toString(),
            color,
            background,
            fontSize.toString(),
            source,
        ).joinToString("\u0000")
        val digest = MessageDigest.getInstance("SHA-256").digest(text.toByteArray(Charsets.UTF_8))
        return digest.joinToString("") { "%02x".format(Locale.ROOT, it) }
    }

    companion object {
        const val KIND_MATH = "math"
        const val KIND_MATH_INLINE = "math-inline"

        /** The card a diagram or a graph is drawn on — fixed, not the bubble's. */
        const val CARD_LIGHT = "#FFFFFF"
        const val CARD_DARK = "#1C1C1E"

        /** The card's ink. */
        const val CARD_INK_LIGHT = "#1C1C1E"
        const val CARD_INK_DARK = "#F2F2F7"

        const val TRANSPARENT = "transparent"

        private val JSON = Json {
            explicitNulls = false
            encodeDefaults = true
        }

        /**
         * The job for a rich BLOCK. A formula is drawn in the bubble's own
         * [ink] on no background; a diagram or a graph on the fixed card,
         * in the card's ink — the bubble's tint never reaches it, so one
         * diagram reads the same in my bubble and in yours.
         */
        fun forBlock(block: MessageMarkdown.Block.Rich, dark: Boolean, ink: Int, fontSize: Float): RichJob {
            val math = block.kind == MessageMarkdown.RichKind.MATH
            return RichJob(
                kind = block.kind.wire,
                source = block.source,
                engine = block.engine,
                dark = dark,
                color = if (math) cssColor(ink) else if (dark) CARD_INK_DARK else CARD_INK_LIGHT,
                background = if (math) TRANSPARENT else if (dark) CARD_DARK else CARD_LIGHT,
                fontSize = roundedFontSize(fontSize),
            )
        }

        /** The job for an inline formula: always bare ink, like a block formula. */
        fun forInline(source: String, dark: Boolean, ink: Int, fontSize: Float): RichJob =
            RichJob(
                kind = KIND_MATH_INLINE,
                source = source,
                dark = dark,
                color = cssColor(ink),
                background = TRANSPARENT,
                fontSize = roundedFontSize(fontSize),
            )

        /**
         * [size] to a hundredth. A font scale of 1.15 makes 14 × 1.15 come
         * out as 16.099999 on one path and 16.1 on another; rounding keeps
         * one size one key.
         */
        fun roundedFontSize(size: Float): Float = (size * 100f).roundToInt() / 100f

        /**
         * An ARGB colour as CSS: `#RRGGBB` when opaque, `rgba(…)` otherwise.
         */
        fun cssColor(argb: Int): String {
            val a = (argb ushr 24) and 0xFF
            val r = (argb ushr 16) and 0xFF
            val g = (argb ushr 8) and 0xFF
            val b = argb and 0xFF
            if (a == 0xFF) return "#%02X%02X%02X".format(Locale.ROOT, r, g, b)
            val alpha = (a / 255.0 * 1000).roundToInt() / 1000.0
            return "rgba($r,$g,$b,$alpha)"
        }
    }
}

/**
 * How a drawing of `cssWidth × cssHeight` CSS pixels is snapshotted: at the
 * screen's [density], or smaller when that would make the longest side pass
 * [MAX_SIDE] pixels (the design doc: "capped so the longest side is at most
 * 4096 pixels"). A full-size view zooms, so a 1200 × 3000 diagram on a
 * 3× phone is drawn at 4096 px tall rather than 9000 — 64 MB of ARGB per
 * drawing would empty the heap long before the cache filled.
 */
object RichCapture {

    const val MAX_SIDE = 4096

    /** The scale (device pixels per CSS pixel) and the bitmap it fills. */
    data class Size(val scale: Float, val width: Int, val height: Int)

    fun size(cssWidth: Float, cssHeight: Float, density: Float): Size {
        val longest = maxOf(cssWidth, cssHeight)
        val scale = if (longest * density > MAX_SIDE) MAX_SIDE / longest else density
        // Rounded UP so the last row and column of the drawing are kept,
        // then clamped: ceil of exactly 4096.0001 must not become 4097.
        fun px(css: Float) = ceil(css * scale).toInt().coerceIn(1, MAX_SIDE)
        return Size(scale, px(cssWidth), px(cssHeight))
    }
}

/** What drawing a job came to. */
sealed interface RichResult {

    /**
     * The drawing, and its size in CSS pixels (dp). [baseline] is the
     * distance from the top to the formula's baseline — the height for
     * anything that is not a formula.
     */
    class Drawn(
        val bitmap: android.graphics.Bitmap,
        val width: Float,
        val height: Float,
        val baseline: Float,
    ) : RichResult

    /**
     * It could not be drawn — the engine refused it, it drew nothing, or it
     * took longer than fifteen seconds. Carries no reason on purpose: an
     * engine's message quotes the source, and nothing about a message's
     * text may reach a log.
     */
    data object Failed : RichResult
}
