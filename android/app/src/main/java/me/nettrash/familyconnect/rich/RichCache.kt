/*
 * RichCache.kt
 * Family Connect (Android)
 *
 * What the app keeps of the rich blocks it drew, so a message is drawn once
 * rather than every time it scrolls into view (docs/protocol.md, "Rich
 * blocks": "A native client keeps what it drew").
 *
 * Two tiers, keyed by RichJob.cacheKey:
 *  - memory: a small LRU of bitmaps, sized in BYTES — a dozen formulas cost
 *    less than one tall diagram, and counting entries would let three
 *    diagrams take the heap;
 *  - disk: the app's cache folder, one PNG per drawing beside a tiny text
 *    file with its width, height and baseline in CSS pixels. PNG because it
 *    keeps the alpha a formula is drawn on; the cache folder because the
 *    system may empty it whenever it likes, and a drawing is only ever a
 *    redraw away.
 *
 * A FAILURE is remembered in memory only: a newer engine (or a renderer
 * that is not out of memory this time) gets another try on the next launch,
 * and a stored "this cannot be drawn" would outlive the reason for it.
 */

package me.nettrash.familyconnect.rich

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.util.Log
import android.util.LruCache
import java.io.File

/** The memory tier: drawings and failures, both bounded. */
class RichMemoryCache(maxBytes: Int = DEFAULT_MAX_BYTES) {

    private val drawn = object : LruCache<String, RichResult.Drawn>(maxBytes) {
        override fun sizeOf(key: String, value: RichResult.Drawn): Int =
            value.bitmap.allocationByteCount.coerceAtLeast(1)
    }

    /** Keys that failed this launch. Bounded too: a key is 64 characters. */
    private val failed = LruCache<String, Unit>(MAX_FAILURES)

    /** A drawing, a remembered failure, or null for "never tried". */
    fun get(key: String): RichResult? = drawn.get(key) ?: failed.get(key)?.let { RichResult.Failed }

    fun put(key: String, result: RichResult) {
        when (result) {
            is RichResult.Drawn -> drawn.put(key, result)
            RichResult.Failed -> failed.put(key, Unit)
        }
    }

    companion object {
        /**
         * 24 MB: a screenful of diagrams at full density with room to
         * scroll back, and a small share of any heap this app runs in.
         */
        const val DEFAULT_MAX_BYTES = 24 * 1024 * 1024
        const val MAX_FAILURES = 256
    }
}

/**
 * The disk tier. Every call does file I/O — callers keep it off the main
 * thread. A damaged or half-written entry reads as a miss, never as an
 * error: the cost of a miss is one redraw.
 */
class RichDiskCache(private val dir: File, private val maxBytes: Long = DEFAULT_MAX_BYTES) {

    fun read(key: String): RichResult.Drawn? {
        val png = File(dir, "$key.png")
        val meta = File(dir, "$key.meta")
        if (!png.isFile || !meta.isFile) return null
        return runCatching {
            val (width, height, baseline) = meta.readText().trim().split(' ').map { it.toFloat() }
            val options = BitmapFactory.Options().apply { inPreferredConfig = Bitmap.Config.ARGB_8888 }
            val bitmap = BitmapFactory.decodeFile(png.path, options) ?: return null
            // Touched, so the trim below drops what nobody has looked at.
            png.setLastModified(System.currentTimeMillis())
            RichResult.Drawn(bitmap, width, height, baseline)
        }.getOrNull()
    }

    fun write(key: String, drawn: RichResult.Drawn) {
        runCatching {
            dir.mkdirs()
            // Written beside, then renamed: a crash mid-write leaves a
            // stray .tmp, never a truncated PNG under a real key.
            val pngTmp = File(dir, "$key.png.tmp")
            pngTmp.outputStream().use { drawn.bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
            val metaTmp = File(dir, "$key.meta.tmp")
            metaTmp.writeText("${drawn.width} ${drawn.height} ${drawn.baseline}")
            // The metadata goes in LAST: `read` wants both, so an entry
            // with a PNG and no metadata is a miss rather than a wrong size.
            check(pngTmp.renameTo(File(dir, "$key.png")))
            check(metaTmp.renameTo(File(dir, "$key.meta")))
            trim()
        }.onFailure {
            // The class of failure only — never a key or a path, which
            // derive from the source.
            Log.w(TAG, "could not keep a drawing: ${it.javaClass.simpleName}")
        }
    }

    /** Drops the least recently used drawings past [maxBytes]. */
    private fun trim() {
        val files = dir.listFiles() ?: return
        var total = files.sumOf { it.length() }
        if (total <= maxBytes) return
        for (png in files.filter { it.name.endsWith(".png") }.sortedBy { it.lastModified() }) {
            if (total <= maxBytes) break
            val meta = File(dir, png.name.removeSuffix(".png") + ".meta")
            total -= png.length() + meta.length()
            png.delete()
            meta.delete()
        }
    }

    companion object {
        private const val TAG = "RichCache"

        /** 32 MB of PNGs: thousands of formulas, hundreds of diagrams. */
        const val DEFAULT_MAX_BYTES = 32L * 1024 * 1024
    }
}
