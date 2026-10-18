/*
 * RichHostDeviceTest.kt
 * Family Connect (Android)
 *
 * The hidden web view really draws (#97): the one thing a JVM cannot check,
 * because Robolectric has no WebView. Every kind of rich block, through the
 * real bundled engines in assets/rich/, snapshotted the way a bubble gets
 * it — and the properties a bubble depends on: the size matches the result,
 * a formula keeps its transparent ground, a card has its colour, a refused
 * job fails rather than hanging, and a job never comes back with the
 * previous job's picture.
 *
 * Needs a device or an emulator; CI has no emulator lane. Run alone with
 *     ANDROID_SERIAL=<emulator> ./gradlew connectedStandardDebugAndroidTest -PnoBump \
 *     -Pandroid.testInstrumentationRunnerArguments.class=me.nettrash.familyconnect.rich.RichHostDeviceTest
 *
 * Each drawing is also written to the app's cache folder (rich-device-test/)
 * so a person can look at them: `adb shell run-as me.nettrash.familyconnect
 * ls cache/rich-device-test`.
 */

package me.nettrash.familyconnect.rich

import android.graphics.Bitmap
import android.graphics.Color
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.runBlocking
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

@RunWith(AndroidJUnit4::class)
class RichHostDeviceTest {

    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val density = context.resources.displayMetrics.density
    private val out = File(context.cacheDir, "rich-device-test").apply { mkdirs() }

    private fun job(kind: String, source: String, engine: String? = null, dark: Boolean = false): RichJob {
        val math = kind == "math" || kind == "math-inline"
        return RichJob(
            kind = kind,
            source = source,
            engine = engine,
            dark = dark,
            color = if (math) "#D0021B" else if (dark) RichJob.CARD_INK_DARK else RichJob.CARD_INK_LIGHT,
            background = if (math) RichJob.TRANSPARENT else if (dark) RichJob.CARD_DARK else RichJob.CARD_LIGHT,
            fontSize = 16f,
        )
    }

    private fun drawn(host: RichHost, job: RichJob, name: String): RichResult.Drawn {
        val result = runBlocking { host.render(job) }
        assertThat(result).isInstanceOf(RichResult.Drawn::class.java)
        val drawn = result as RichResult.Drawn
        File(out, "$name.png").outputStream().use { drawn.bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        val size = RichCapture.size(drawn.width, drawn.height, density)
        assertThat(drawn.bitmap.width).isEqualTo(size.width)
        assertThat(drawn.bitmap.height).isEqualTo(size.height)
        assertThat(drawn.bitmap.config).isEqualTo(Bitmap.Config.ARGB_8888)
        return drawn
    }

    /** Pixels whose alpha is at least [minAlpha]. */
    private fun opaque(bitmap: Bitmap, minAlpha: Int = 200): Int {
        val pixels = IntArray(bitmap.width * bitmap.height)
        bitmap.getPixels(pixels, 0, bitmap.width, 0, 0, bitmap.width, bitmap.height)
        return pixels.count { Color.alpha(it) >= minAlpha }
    }

    /**
     * The four corners, [inset] pixels in. A card's right and bottom edges
     * can end inside the last device pixel (fc-rich.js rounds its box UP to
     * a whole CSS pixel), so a card is sampled a few pixels in.
     */
    private fun corners(bitmap: Bitmap, inset: Int = 0): List<Int> = listOf(
        bitmap.getPixel(inset, inset),
        bitmap.getPixel(bitmap.width - 1 - inset, inset),
        bitmap.getPixel(inset, bitmap.height - 1 - inset),
        bitmap.getPixel(bitmap.width - 1 - inset, bitmap.height - 1 - inset),
    )

    /** Nothing but anti-aliasing at a formula's corners: no ground. */
    private fun bare(bitmap: Bitmap): Boolean = corners(bitmap).all { Color.alpha(it) < 32 }

    @Test
    fun everyKindDraws() {
        val host = RichHost(context)

        // A display formula: ink on NOTHING — transparent corners, red ink.
        val formula = drawn(host, job("math", "\\int_0^1 x^2\\,dx = \\frac{1}{3}"), "display-math")
        assertThat(bare(formula.bitmap)).isTrue()
        assertThat(opaque(formula.bitmap)).isGreaterThan(50)
        assertThat(inkIsRed(formula.bitmap)).isTrue()

        // An inline formula has a baseline inside its box.
        val inline = drawn(host, job("math-inline", "a_1^2 + \\ce{H2O}"), "inline-math")
        assertThat(inline.baseline).isGreaterThan(0f)
        assertThat(inline.baseline).isAtMost(inline.height)
        assertThat(bare(inline.bitmap)).isTrue()

        // Diagrams and graphs on the card: opaque white corners.
        val cards = listOf(
            "mermaid" to job("mermaid", "graph TD\nA[Start] --> B{Is it?}\nB -->|Yes| C[OK]\nB -->|No| D[End]"),
            "graphviz-dot" to job("graphviz", "digraph { a -> b -> c; a -> c }", "dot"),
            "graphviz-neato" to job("graphviz", "graph { a -- b -- c -- a }", "neato"),
            "plot" to job("plot", "x: -6..6\nsin(x)\nenvelope = exp(-abs(x)/5)"),
        )
        for ((name, card) in cards) {
            val bitmap = drawn(host, card, name).bitmap
            for (corner in corners(bitmap, inset = 3)) assertThat(corner).isEqualTo(Color.WHITE)
            // Something drawn on the card, not just the card.
            assertThat(countDark(bitmap)).isGreaterThan(20)
        }

        // The dark card.
        val dark = drawn(host, job("mermaid", "pie\n\"a\": 2\n\"b\": 1", dark = true), "mermaid-dark").bitmap
        for (corner in corners(dark, inset = 3)) assertThat(corner).isEqualTo(Color.parseColor("#1C1C1E"))
    }

    /**
     * The stale-frame guard: a formula drawn straight after a card must not
     * come back with the card's white in it, and a small formula after a
     * large one must be its own size.
     */
    @Test
    fun aJobNeverReturnsThePreviousPicture() {
        val host = RichHost(context)
        repeat(3) { round ->
            drawn(host, job("graphviz", "digraph { x$round -> y -> z }"), "seq-card-$round")
            val small = drawn(host, job("math-inline", "a$round"), "seq-small-$round")
            assertThat(bare(small.bitmap)).isTrue()
            assertThat(countWhite(small.bitmap)).isEqualTo(0)
            assertThat(small.width).isLessThan(60f)
        }
    }

    /**
     * Taller than the view starts (1024 CSS pixels): the view grows before
     * the snapshot, and the drawing comes back whole — its bottom corners
     * are card, not transparency.
     */
    @Test
    fun aTallDiagramIsDrawnWhole() {
        val host = RichHost(context)
        val chain = (1..24).joinToString(" -> ") { "n$it" }
        val tall = drawn(host, job("graphviz", "digraph { $chain }", "dot"), "graphviz-tall")
        assertThat(tall.height).isGreaterThan(RichHost.VIEW_CSS_HEIGHT.toFloat())
        for (corner in corners(tall.bitmap, inset = 3)) assertThat(corner).isEqualTo(Color.WHITE)
        assertThat(countDark(tall.bitmap.let { Bitmap.createBitmap(it, 0, it.height * 3 / 4, it.width, it.height / 4) }))
            .isGreaterThan(20)
    }

    @Test
    fun aRefusedJobFailsAndTheHostCarriesOn() {
        val host = RichHost(context)
        assertThat(runBlocking { host.render(job("math", "\\frac{")) }).isEqualTo(RichResult.Failed)
        assertThat(runBlocking { host.render(job("mermaid", "this is not a diagram")) }).isEqualTo(RichResult.Failed)
        assertThat(runBlocking { host.render(job("graphviz", "digraph {", "dot")) }).isEqualTo(RichResult.Failed)
        assertThat(runBlocking { host.render(job("plot", "%%%")) }).isEqualTo(RichResult.Failed)
        // Still working afterwards.
        drawn(host, job("math", "e^{i\\pi} + 1 = 0"), "after-failures")
    }

    /**
     * Nothing leaves the folder: a diagram that asks for a picture from the
     * network still draws (Mermaid strict / the content policy drop it) and
     * the host's interceptor answers the request, if any, with a refusal.
     */
    @Test
    fun aDiagramThatPointsOutsideStillDrawsOffline() {
        val host = RichHost(context)
        drawn(
            host,
            job("mermaid", "graph TD\nA[\"<img src='https://example.com/x.png'>\"] --> B\nclick A \"https://example.com\""),
            "mermaid-outside",
        )
    }

    private fun inkIsRed(bitmap: Bitmap): Boolean {
        val pixels = IntArray(bitmap.width * bitmap.height)
        bitmap.getPixels(pixels, 0, bitmap.width, 0, 0, bitmap.width, bitmap.height)
        val ink = pixels.filter { Color.alpha(it) > 240 }
        if (ink.isEmpty()) return false
        // Premultiplied or not, fully opaque ink is the colour itself: #D0021B.
        return ink.count { Color.red(it) > 160 && Color.green(it) < 60 && Color.blue(it) < 80 } > ink.size / 2
    }

    private fun countDark(bitmap: Bitmap): Int {
        val pixels = IntArray(bitmap.width * bitmap.height)
        bitmap.getPixels(pixels, 0, bitmap.width, 0, 0, bitmap.width, bitmap.height)
        return pixels.count { Color.red(it) < 120 && Color.green(it) < 120 && Color.blue(it) < 120 }
    }

    private fun countWhite(bitmap: Bitmap): Int {
        val pixels = IntArray(bitmap.width * bitmap.height)
        bitmap.getPixels(pixels, 0, bitmap.width, 0, 0, bitmap.width, bitmap.height)
        return pixels.count { it == Color.WHITE }
    }
}
