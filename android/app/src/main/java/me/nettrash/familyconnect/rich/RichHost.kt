/*
 * RichHost.kt
 * Family Connect (Android)
 *
 * The one hidden web view that draws rich blocks (#97) — the design doc's
 * "native host" (docs/capitals-and-rich-markdown-2026-10-17.md).
 *
 * WHAT IT IS: an android.webkit.WebView that is never shown. It is made on
 * first need, on the main thread, and kept; it is measured and laid out BY
 * HAND to a fixed size, never attached to a window, and its pixels are taken
 * by drawing it into a Bitmap with a software Canvas (`View.draw`). That is
 * the md.Android EPUB exporter's technique (EpubExporter.renderRich), and
 * the reason it works is worth stating: a WebView that has never been
 * attached still runs its page — script, layout, fonts — and still answers a
 * software draw, which WebView rasterises on demand into the canvas it is
 * handed rather than from tiles a compositor would have to have produced for
 * a screen. RichHostDeviceTest proves it on an emulator, for every kind of
 * block, and keeps the snapshots for a person to look at. Attaching it to
 * the activity, even off-screen, would tie the
 * drawing of a bubble to whichever activity happened to be up, and would put
 * a 1280-pixel-wide view into the hierarchy the chat scrolls in.
 *
 * WHAT IT LOADS: `rich.html` from the app's own copy of web/rich (assets/
 * rich/, byte-identical — CI compares them), served by androidx.webkit's
 * WebViewAssetLoader on https://appassets.androidplatform.net/rich/, a
 * reserved host that never resolves. A real https origin rather than a
 * file:// URL or loadDataWithBaseURL's opaque origin, because the page's
 * content policy says `'self'` and an opaque origin has no self to match.
 * EVERY OTHER REQUEST IS REFUSED here, with an empty 403, before WebView
 * could go to the network — the page's own policy says the same thing to the
 * engine, and the two together are the "nothing is fetched" promise of
 * docs/protocol.md: a message cannot load a picture from somewhere, or tell
 * anyone it was read.
 *
 * HOW A JOB RUNS: one at a time (RichRenderer holds the lock), as
 * `fcRich.drawAndPost(id, job)`. WebView's evaluateJavascript cannot wait for
 * a promise, so the page posts its RESULT back through the `FcRichBridge`
 * JavaScript interface, matched to the job by id; fifteen seconds without an
 * answer is a failure, and the view is thrown away, because a page that has
 * not answered in fifteen seconds is busy with something it may never finish
 * and every job queued behind it would time out too. The result rectangle is
 * at the page's top left; it is snapshotted at the screen's density, capped
 * at 4096 pixels on the longest side (RichCapture), into an ARGB_8888 bitmap
 * that keeps the alpha — the view's background is transparent, and so is the
 * page's, so a formula comes out as ink on nothing.
 *
 * WHEN THE RENDERER DIES: WebView's rule is blunt — a client that does not
 * answer `onRenderProcessGone` has the whole app killed with it. This one
 * answers: the job in flight fails, the dead view is destroyed, and the next
 * job makes a new one.
 *
 * Logs never carry a source, an engine message or a key (which derives from
 * the source): the kind of block and the class of failure, nothing else.
 */

package me.nettrash.familyconnect.rich

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.util.Log
import android.view.View
import android.webkit.JavascriptInterface
import android.webkit.RenderProcessGoneDetail
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.core.graphics.createBitmap
import androidx.core.graphics.get
import androidx.webkit.WebSettingsCompat
import androidx.webkit.WebViewAssetLoader
import androidx.webkit.WebViewFeature
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.floatOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull
import java.io.ByteArrayInputStream
import java.util.concurrent.ConcurrentHashMap
import kotlin.coroutines.resume
import kotlin.math.ceil

/** Something that draws one job. RichHost in the app; a fake in tests. */
interface RichEngine {
    /** Draws [job], or says it could not. Never throws for a bad job. */
    suspend fun render(job: RichJob): RichResult
}

class RichHost(context: Context) : RichEngine {

    private val context = context.applicationContext

    /** The view, once made; null before the first job and after a loss. */
    private var view: WebView? = null

    /** Completes true when [view]'s page has loaded, false when it cannot. */
    private var loaded: CompletableDeferred<Boolean>? = null

    /** Jobs posted to the page and not yet answered, by id. */
    private val pending = ConcurrentHashMap<Long, CompletableDeferred<String?>>()

    private var nextId = 0L

    private val density: Float get() = context.resources.displayMetrics.density

    override suspend fun render(job: RichJob): RichResult = withContext(Dispatchers.Main.immediate) {
        val web = obtain() ?: return@withContext RichResult.Failed
        val answer = post(web, job) ?: return@withContext RichResult.Failed
        var result = parse(answer) ?: return@withContext RichResult.Failed
        // A formula measured before KaTeX's fonts had arrived was measured
        // in the fallback face, and its box is the wrong size for the
        // glyphs that land a moment later. Fonts load on first USE, so the
        // first formula of each face can race them: when the page says it
        // is still loading fonts, wait for them and draw the job again.
        if (job.isMath && web.eval("document.fonts ? document.fonts.status : 'loaded'") == "\"loading\"") {
            val again = post(web, job, afterFonts = true) ?: return@withContext RichResult.Failed
            result = parse(again) ?: return@withContext RichResult.Failed
        }
        capture(web, result, job.isCard)
    }

    /** The result of a draw: CSS pixels, at the stage's top left. */
    private class Box(val width: Float, val height: Float, val baseline: Float)

    /**
     * The page's RESULT for one answer, or null when it is a failure — the
     * engine's own message is dropped here unread, because it quotes the
     * source.
     */
    private fun parse(answer: String): Box? = runCatching {
        val result = Json.parseToJsonElement(answer).jsonObject["result"] as? JsonObject ?: return null
        if (result["ok"]?.jsonPrimitive?.booleanOrNull != true) return null
        val width = result["width"]?.jsonPrimitive?.floatOrNull ?: return null
        val height = result["height"]?.jsonPrimitive?.floatOrNull ?: return null
        val baseline = result["baseline"]?.jsonPrimitive?.floatOrNull ?: height
        if (width <= 0f || height <= 0f) return null
        Box(width, height, baseline.coerceIn(0f, height))
    }.getOrNull()

    /**
     * Posts [job] and waits for its answer: the JSON the page handed the
     * bridge, or null after fifteen seconds or when the renderer died.
     */
    private suspend fun post(web: WebView, job: RichJob, afterFonts: Boolean = false): String? {
        val id = ++nextId
        val answer = CompletableDeferred<String?>()
        pending[id] = answer
        // The job's JSON is a JavaScript expression as it stands.
        val call = "window.fcRich.drawAndPost($id, ${job.toJson()})"
        web.evaluateJavascript(
            if (afterFonts) "document.fonts.ready.then(function () { $call; })" else "$call;",
            null,
        )
        val text = withTimeoutOrNull(JOB_TIMEOUT_MS) { answer.await() }
        pending.remove(id)
        if (text == null && view === web) {
            // Unanswered for fifteen seconds, and not because the renderer
            // died (that path already dropped the view): whatever the page
            // is doing, every job queued behind it would wait as long.
            Log.w(TAG, "a ${job.kind} job went unanswered; starting over")
            discard()
        }
        return text
    }

    /**
     * The drawing in [box], as a bitmap at the screen's density — or a
     * failure when the view cannot produce it.
     */
    private suspend fun capture(web: WebView, box: Box, card: Boolean): RichResult {
        // Big enough to hold the drawing: a software draw paints what lies
        // inside the view's bounds, so a tall diagram needs a tall view.
        // Growing only ever adds room — the stage is shrink-to-fit at the
        // top left and nothing it holds is laid out against the height.
        val needWidth = ceil(box.width * density).toInt()
        val needHeight = ceil(box.height * density).toInt()
        if (web.width < needWidth || web.height < needHeight) {
            layout(web, maxOf(web.width, needWidth), maxOf(web.height, needHeight))
            delay(RESIZE_SETTLE_MS)
        }
        awaitVisualState(web)
        val size = RichCapture.size(box.width, box.height, density)
        repeat(CAPTURE_ATTEMPTS) { attempt ->
            if (attempt > 0) delay(CAPTURE_RETRY_MS)
            if (view !== web) return RichResult.Failed
            val bitmap = try {
                snapshot(web, size)
            } catch (_: OutOfMemoryError) {
                Log.w(TAG, "no memory for a ${size.width}x${size.height} drawing")
                return RichResult.Failed
            }
            // A blank snapshot is a page that has not painted the job yet,
            // never a real drawing: a formula always has ink, and a card
            // always has its ground.
            if (painted(bitmap, card)) return RichResult.Drawn(bitmap, box.width, box.height, box.baseline)
            bitmap.recycle()
        }
        Log.w(TAG, "a drawing never painted")
        return RichResult.Failed
    }

    /**
     * Waits until WebView says its next draw shows the DOM as it is now —
     * the job just drawn, not the one before it — or a short while, when
     * it does not say ([CAPTURE_ATTEMPTS] covers the rest).
     */
    private suspend fun awaitVisualState(web: WebView) {
        val ready = CompletableDeferred<Unit>()
        web.postVisualStateCallback(
            ++nextId,
            object : WebView.VisualStateCallback() {
                override fun onComplete(requestId: Long) {
                    ready.complete(Unit)
                }
            },
        )
        withTimeoutOrNull(VISUAL_STATE_TIMEOUT_MS) { ready.await() }
    }

    /** [web]'s top left at [size]. */
    private fun snapshot(web: WebView, size: RichCapture.Size): Bitmap {
        // The view draws in device pixels; a capped capture shrinks that to
        // the scale it was given. Worked out HERE, outside the Canvas: inside
        // `Canvas.apply` the name `density` is the canvas's own bitmap
        // density (420 dpi), not the screen's scale — that slip drew every
        // snapshot at a 160th of its size, two pixels in the corner.
        val factor = size.scale / density
        val bitmap = createBitmap(size.width, size.height, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bitmap)
        canvas.scale(factor, factor)
        web.draw(canvas)
        return bitmap
    }

    /**
     * Whether [bitmap] holds the job's drawing rather than a page that has
     * not painted it yet. A card is painted when its corner is — the card's
     * ground is opaque edge to edge; a formula when a meaningful share of
     * its box has ink (a glyph covers far more than this; a stray edge
     * pixel does not).
     */
    private fun painted(bitmap: Bitmap, card: Boolean): Boolean {
        if (card) return (bitmap[0, 0] ushr 24) == 0xFF
        val row = IntArray(bitmap.width)
        val needed = (bitmap.width.toLong() * bitmap.height / INK_SHARE).coerceAtLeast(1)
        var inked = 0L
        for (y in 0 until bitmap.height) {
            bitmap.getPixels(row, 0, bitmap.width, 0, y, bitmap.width, 1)
            for (pixel in row) if ((pixel ushr 24) != 0) inked += 1
            if (inked >= needed) return true
        }
        return false
    }

    // ---- the view ------------------------------------------------------------

    /** The loaded view, making it on first need; null when it cannot load. */
    private suspend fun obtain(): WebView? {
        val existing = view
        val ready = loaded
        if (existing != null && ready != null) {
            return if (ready.await()) existing else null
        }
        val web = create() ?: return null
        val loading = CompletableDeferred<Boolean>()
        view = web
        loaded = loading
        web.loadUrl(PAGE_URL)
        val ok = withTimeoutOrNull(JOB_TIMEOUT_MS) { loading.await() } == true &&
            web.eval("typeof window.fcRich === 'object' && typeof window.fcRich.drawAndPost === 'function'") == "true"
        if (!ok) {
            Log.w(TAG, "the drawing page did not load")
            if (view === web) discard()
            return null
        }
        return web
    }

    @SuppressLint("SetJavaScriptEnabled")
    private fun create(): WebView? {
        val web = try {
            WebView(context)
        } catch (e: RuntimeException) {
            // No WebView provider (it is being updated, or disabled): every
            // block shows its source, which is what a failure looks like.
            Log.w(TAG, "no web view: ${e.javaClass.simpleName}")
            return null
        }
        // Transparent ground: a formula is ink on nothing, and the page's
        // own body is transparent too (rich.html).
        web.setBackgroundColor(Color.TRANSPARENT)
        web.settings.apply {
            // fc-rich.js and the engines are script; nothing else is
            // allowed in, so this enables only our own files.
            javaScriptEnabled = true
            allowFileAccess = false
            allowContentAccess = false
            // The job carries the size in CSS pixels with the reader's font
            // scale already applied; WebView must not scale it again.
            textZoom = 100
            setSupportZoom(false)
        }
        // A dark app theme must not have WebView invert the drawing: the
        // colours are chosen by the job, and Mermaid's dark theme is asked
        // for by name.
        if (WebViewFeature.isFeatureSupported(WebViewFeature.ALGORITHMIC_DARKENING)) {
            WebSettingsCompat.setAlgorithmicDarkeningAllowed(web.settings, false)
        }
        web.addJavascriptInterface(Bridge(), BRIDGE_NAME)
        web.webViewClient = RichPageClient(
            loader = assetLoader(context),
            onLoaded = { loadedView -> if (loadedView === view) loaded?.complete(true) },
            onGone = { goneView ->
                if (goneView === view) discard() else goneView.destroy()
            },
        )
        layout(web, (VIEW_CSS_WIDTH * density).toInt(), (VIEW_CSS_HEIGHT * density).toInt())
        return web
    }

    /** Lays [web] out at [width] × [height] device pixels, as a window would. */
    private fun layout(web: WebView, width: Int, height: Int) {
        web.measure(
            View.MeasureSpec.makeMeasureSpec(width, View.MeasureSpec.EXACTLY),
            View.MeasureSpec.makeMeasureSpec(height, View.MeasureSpec.EXACTLY),
        )
        web.layout(0, 0, web.measuredWidth, web.measuredHeight)
    }

    /**
     * Throws the current view away: every job waiting on it fails, and the
     * next one makes a new view. Main thread only.
     */
    private fun discard() {
        val web = view ?: return
        view = null
        loaded?.complete(false)
        loaded = null
        for (id in pending.keys.toList()) pending.remove(id)?.complete(null)
        web.stopLoading()
        web.destroy()
    }

    /**
     * evaluateJavascript, awaited: the value as JSON text, or null when the
     * page does not answer within the job timeout (a busy page never runs
     * the callback at all).
     */
    private suspend fun WebView.eval(script: String): String? = withTimeoutOrNull(JOB_TIMEOUT_MS) {
        suspendCancellableCoroutine { continuation ->
            evaluateJavascript(script) { value -> if (continuation.isActive) continuation.resume(value) }
        }
    }

    /**
     * The page's way back: `fcRich.drawAndPost` calls `done` with
     * `{"id": …, "result": …}` once the job is drawn. Called on WebView's
     * bridge thread; [pending] and the deferred are both safe there.
     */
    private inner class Bridge {
        @JavascriptInterface
        fun done(json: String) {
            val id = runCatching {
                Json.parseToJsonElement(json).jsonObject["id"]?.jsonPrimitive?.longOrNull
            }.getOrNull() ?: return
            pending.remove(id)?.complete(json)
        }
    }

    companion object {
        private const val TAG = "RichHost"
        private const val BRIDGE_NAME = "FcRichBridge"

        /** The reserved, never-resolving host WebViewAssetLoader serves. */
        const val HOST = WebViewAssetLoader.DEFAULT_DOMAIN
        const val PAGE_URL = "https://$HOST/rich/rich.html"

        /** docs/protocol.md: "longer than fifteen seconds" is a failure. */
        const val JOB_TIMEOUT_MS = 15_000L

        /**
         * The layout viewport, in CSS pixels. Wider than fc-rich.js's
         * default 1200 maxWidth, so a diagram is never squeezed by the view,
         * and wide enough that an inline formula does not wrap; the height
         * grows on demand before a snapshot (see [capture]).
         */
        const val VIEW_CSS_WIDTH = 1280
        const val VIEW_CSS_HEIGHT = 1024

        private const val RESIZE_SETTLE_MS = 100L

        private const val VISUAL_STATE_TIMEOUT_MS = 500L
        private const val CAPTURE_ATTEMPTS = 6
        private const val CAPTURE_RETRY_MS = 150L

        /** A formula is painted once one pixel in this many has ink. */
        private const val INK_SHARE = 200L

        /**
         * The asset loader for assets/rich/ on `/rich/`. AssetsPathHandler
         * resolves the path AFTER the prefix against the assets root, so it
         * is handed `rich/<file>` — and only ever a file in that folder,
         * which is the whole of what this view may read.
         */
        fun assetLoader(context: Context): WebViewAssetLoader {
            val assets = WebViewAssetLoader.AssetsPathHandler(context)
            return WebViewAssetLoader.Builder()
                .addPathHandler("/rich/") { path -> assets.handle("rich/$path") }
                .build()
        }

    }
}

/**
 * The hidden web view's client: our files or nothing, no navigation, and an
 * answer for a dead renderer.
 *
 * The suppression is for a false positive, and only that: androidx.webkit's
 * MissingOnRenderProcessGone check reports EVERY `WebViewClient()`
 * constructor call — meant for `webViewClient = WebViewClient()` — and a
 * Kotlin class header's call to its superclass constructor is one of those,
 * however completely the class answers [onRenderProcessGone] below.
 */
@SuppressLint("MissingOnRenderProcessGone")
private class RichPageClient(
    private val loader: WebViewAssetLoader,
    private val onLoaded: (WebView) -> Unit,
    private val onGone: (WebView) -> Unit,
) : WebViewClient() {

    override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse =
        // Our files, or nothing at all: the refusal is a response, not a
        // null, because null means "go and fetch it".
        loader.shouldInterceptRequest(request.url) ?: refused()

    // Nothing navigates anywhere: Mermaid runs strict and KaTeX untrusted,
    // so a link should not exist — and if one did, it opens nothing from
    // here.
    override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean = true

    override fun onPageFinished(view: WebView, url: String?) = onLoaded(view)

    /**
     * WebView's rule is blunt: a client that does not answer this has the
     * whole app killed with its renderer. The job in flight fails, the dead
     * view is destroyed, and the next job makes a new one.
     */
    override fun onRenderProcessGone(view: WebView, detail: RenderProcessGoneDetail): Boolean {
        Log.w(TAG, "the drawing renderer went away (crashed: ${detail.didCrash()})")
        onGone(view)
        return true
    }

    private fun refused(): WebResourceResponse =
        WebResourceResponse("text/plain", "utf-8", 403, "Forbidden", emptyMap(), ByteArrayInputStream(ByteArray(0)))

    private companion object {
        const val TAG = "RichHost"
    }
}
