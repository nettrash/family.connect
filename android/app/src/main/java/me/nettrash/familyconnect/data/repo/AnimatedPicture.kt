/*
 * AnimatedPicture.kt
 * Family Connect (Android)
 *
 * The frames of an animated GIF, WebP or PNG, COMPOSITED — each one drawn as
 * a viewer would show it at that moment, its disposal and blending applied
 * (issue #96, docs/protocol.md "Animations" → "The file"). What the encoder
 * turns into the silent looping MP4 an animation travels as.
 *
 * WHY THIS IS HAND-WRITTEN. The platform will PLAY an animated picture
 * (ImageDecoder → AnimatedImageDrawable, API 28+), but it cannot hand over a
 * chosen frame: AnimatedImageDrawable has no seek and no frame accessor, and
 * it composites on its own clock. `android.graphics.Movie` can seek a GIF,
 * but it is deprecated (a compiler warning in a zero-warning build), its
 * GIF decoder is the old one the platform itself stopped using, and it knows
 * neither WebP nor APNG. And nothing below API 28 decodes an animation at
 * all. So the three containers are read here:
 *
 *  - GIF: a small pure-Kotlin LZW decoder ([Gif]). Each frame's pixels are
 *    an index into a colour table, so decoding them is the whole job — and
 *    it is exactly testable on the JVM, against the platform's own decoder
 *    of the first frame (AnimatedPictureTest, under Robolectric's native
 *    graphics).
 *  - WebP and APNG: the CONTAINER is parsed here, and each frame is cut out
 *    as a standalone still — a WebP of its `ALPH`/`VP8`/`VP8L` chunks, a PNG
 *    of its `fdAT` data renamed `IDAT` — which the platform's BitmapFactory
 *    decodes on every API level this app runs on ([StillDecoder]). The
 *    codecs are the platform's; only the timeline is ours.
 *
 * Compositing is one pure-Kotlin [Compositor] for all three, on a canvas of
 * non-premultiplied ARGB ints: "over" for a blended frame, a plain copy for
 * one that does not blend, and the three disposals (none, back to
 * transparent, back to what was there). "Back to background" is TRANSPARENT
 * for every format, as every browser draws it — the background colour a GIF
 * or a WebP names is ignored there too — and the white the protocol asks for
 * is laid under the result only at the end ([flattenOnWhite]).
 *
 * Everything that reads a length reads it as a number somebody else wrote:
 * every offset is checked against the array before it is used, and a file
 * this does not understand is null — rule C's "goes exactly as before".
 */

package me.nettrash.familyconnect.data.repo

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import java.io.ByteArrayOutputStream
import java.util.zip.CRC32

object AnimatedPicture {

    /** What a picture is, by its own bytes: what the composer's routing reads (#96). */
    enum class Kind {
        /** A GIF, WebP or PNG that says it moves — the protocol's tests, [PackPicture.isAnimated]. */
        ANIMATED,

        /** A GIF with one image: a file, as before. */
        STILL_GIF,

        /** A WebP with one frame: a file, as before. */
        STILL_WEBP,

        /** A BMP: a file, as before. */
        BMP,

        /** Anything else — a JPEG, a still PNG, a HEIC: not this file's business. */
        OTHER,
    }

    /** The bytes these formats start with — enough to decide whether reading the rest is worth it. */
    const val HEAD_BYTES = 16

    /** Whether [head] starts a GIF, a WebP, a PNG or a BMP — the only kinds [kindOf] can call anything but OTHER. */
    fun mayMatter(head: ByteArray): Boolean = isGif(head) || PackPicture.sniff(head) != null || isBmp(head)

    /** [Kind] of [bytes]. */
    fun kindOf(bytes: ByteArray): Kind = when {
        isGif(bytes) -> if (PackPicture.isAnimated(bytes)) Kind.ANIMATED else Kind.STILL_GIF
        PackPicture.sniff(bytes) == PackPicture.MIME_WEBP ->
            if (PackPicture.isAnimated(bytes)) Kind.ANIMATED else Kind.STILL_WEBP
        PackPicture.sniff(bytes) == PackPicture.MIME_PNG ->
            if (PackPicture.isAnimated(bytes)) Kind.ANIMATED else Kind.OTHER
        isBmp(bytes) -> Kind.BMP
        else -> Kind.OTHER
    }

    private fun isGif(bytes: ByteArray): Boolean =
        bytes.size >= 6 && bytes[0] == 'G'.code.toByte() && bytes[1] == 'I'.code.toByte() &&
            bytes[2] == 'F'.code.toByte() && bytes[3] == '8'.code.toByte() &&
            (bytes[4] == '7'.code.toByte() || bytes[4] == '9'.code.toByte()) && bytes[5] == 'a'.code.toByte()

    private fun isBmp(bytes: ByteArray): Boolean =
        bytes.size >= 2 && bytes[0] == 'B'.code.toByte() && bytes[1] == 'M'.code.toByte()

    // -- The model ---------------------------------------------------------

    /** What happens to a frame's area once it has been shown, before the next is drawn. */
    enum class Disposal { NONE, BACKGROUND, PREVIOUS }

    /** One frame's place on the canvas and its timing — everything but its pixels. */
    data class Frame(
        val x: Int,
        val y: Int,
        val width: Int,
        val height: Int,
        /** As the file states it, in milliseconds (before AnimationPlan's shortest-frame rule). */
        val statedMs: Long,
        /** Drawn OVER what is there (true), or replacing it, alpha and all (false). */
        val blend: Boolean,
        val disposal: Disposal,
    )

    /** A decoded animation: its canvas, its frames in order, and each frame's own pixels on demand. */
    interface Source {
        val width: Int
        val height: Int
        val frames: List<Frame>

        /**
         * Frame [index]'s pixels — `frames[index].width × height`, row by row,
         * non-premultiplied ARGB — or null when they cannot be decoded.
         */
        fun pixels(index: Int): IntArray?
    }

    /** One decoded still: what a [StillDecoder] answers. */
    class Still(val width: Int, val height: Int, val argb: IntArray)

    /** Decodes a standalone still WebP or PNG — the platform's codec in the app, anything in a test. */
    fun interface StillDecoder {
        fun decode(bytes: ByteArray): Still?
    }

    /** The app's [StillDecoder]: BitmapFactory, unscaled and unpremultiplied. */
    val PLATFORM: StillDecoder = StillDecoder { bytes ->
        runCatching {
            val options = BitmapFactory.Options().apply {
                inPreferredConfig = Bitmap.Config.ARGB_8888
                inPremultiplied = false
                inScaled = false
            }
            val bitmap = BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options) ?: return@runCatching null
            try {
                val argb = IntArray(bitmap.width * bitmap.height)
                bitmap.getPixels(argb, 0, bitmap.width, 0, 0, bitmap.width, bitmap.height)
                Still(bitmap.width, bitmap.height, argb)
            } finally {
                bitmap.recycle()
            }
        }.getOrNull()
    }

    /** The largest canvas this composites — a few dozen megabytes of ints already. Larger is rule C. */
    const val MAX_CANVAS_PIXELS = 2048 * 2048

    /** The most frames read from one file. Past it the file is rule C rather than minutes of encoding. */
    const val MAX_FRAMES = 10_000

    /**
     * The animation in [bytes], or null when it is not a GIF, WebP or PNG this
     * can read, has no frame, or is larger than this composites.
     */
    fun open(bytes: ByteArray, stills: StillDecoder = PLATFORM): Source? = runCatching {
        val source = when {
            isGif(bytes) -> Gif.open(bytes)
            PackPicture.sniff(bytes) == PackPicture.MIME_WEBP -> WebP.open(bytes, stills)
            PackPicture.sniff(bytes) == PackPicture.MIME_PNG -> Apng.open(bytes, stills)
            else -> null
        } ?: return@runCatching null
        source.takeIf {
            it.frames.isNotEmpty() && it.width > 0 && it.height > 0 &&
                it.width.toLong() * it.height <= MAX_CANVAS_PIXELS
        }
    }.getOrNull()

    // -- Compositing -------------------------------------------------------

    /**
     * The canvas a viewer draws an animation on. [render] each frame IN ORDER:
     * the previous frame's disposal is applied, this one is drawn, and the
     * canvas is what is showing now. Transparent where nothing was ever drawn.
     */
    class Compositor(val width: Int, val height: Int) {

        /** What is showing: non-premultiplied ARGB, row by row. Read it, never keep it. */
        val canvas = IntArray(width * height)

        private var shown: Frame? = null
        private var saved: IntArray? = null

        /** Draw [frame] with [pixels] (null: nothing could be decoded — its area is left as it is). */
        fun render(frame: Frame, pixels: IntArray?): IntArray {
            shown?.let(::dispose)
            saved = if (frame.disposal == Disposal.PREVIOUS) copyArea(frame) else null
            if (pixels != null && pixels.size >= frame.width * frame.height) draw(frame, pixels)
            shown = frame
            return canvas
        }

        private fun dispose(frame: Frame) {
            when (frame.disposal) {
                Disposal.NONE -> Unit
                Disposal.BACKGROUND -> forEachInArea(frame) { canvasIndex, _ -> canvas[canvasIndex] = 0 }
                Disposal.PREVIOUS -> saved?.let { area ->
                    forEachInArea(frame) { canvasIndex, frameIndex -> canvas[canvasIndex] = area[frameIndex] }
                }
            }
        }

        private fun copyArea(frame: Frame): IntArray {
            val area = IntArray(frame.width * frame.height)
            forEachInArea(frame) { canvasIndex, frameIndex -> area[frameIndex] = canvas[canvasIndex] }
            return area
        }

        private fun draw(frame: Frame, pixels: IntArray) {
            forEachInArea(frame) { canvasIndex, frameIndex ->
                val source = pixels[frameIndex]
                canvas[canvasIndex] = if (frame.blend) over(source, canvas[canvasIndex]) else source
            }
        }

        /** Every pixel of [frame]'s area that lies on the canvas: (canvas index, index within the frame). */
        private inline fun forEachInArea(frame: Frame, action: (Int, Int) -> Unit) {
            val left = frame.x.coerceAtLeast(0)
            val top = frame.y.coerceAtLeast(0)
            val right = minOf(width.toLong(), frame.x.toLong() + frame.width).toInt()
            val bottom = minOf(height.toLong(), frame.y.toLong() + frame.height).toInt()
            for (y in top until bottom) {
                val canvasRow = y * width
                val frameRow = (y - frame.y) * frame.width
                for (x in left until right) action(canvasRow + x, frameRow + (x - frame.x))
            }
        }
    }

    /** [source] drawn over [destination], both non-premultiplied ARGB. */
    fun over(source: Int, destination: Int): Int {
        val sa = source ushr 24
        if (sa == 255) return source
        if (sa == 0) return destination
        val da = destination ushr 24
        if (da == 0) return source
        // The result's alpha, times 255: sa + da × (1 − sa), all in 0…255.
        val weightBelow = da * (255 - sa)
        val alpha255 = sa * 255 + weightBelow
        fun channel(shift: Int): Int {
            val s = (source shr shift) and 0xFF
            val d = (destination shr shift) and 0xFF
            return ((s * sa * 255 + d * weightBelow + alpha255 / 2) / alpha255).coerceIn(0, 255)
        }
        val alpha = (alpha255 + 127) / 255
        return (alpha shl 24) or (channel(16) shl 16) or (channel(8) shl 8) or channel(0)
    }

    /** [canvas] laid on white (`#FFFFFF`), opaque — what a video frame can hold. */
    fun flattenOnWhite(canvas: IntArray, into: IntArray = IntArray(canvas.size)): IntArray {
        for (i in canvas.indices) {
            val pixel = canvas[i]
            val a = pixel ushr 24
            into[i] = when (a) {
                255 -> pixel
                0 -> AnimationPlan.BACKGROUND_ARGB
                else -> {
                    val inverse = 255 * (255 - a)
                    fun channel(shift: Int) = (((pixel shr shift) and 0xFF) * a + inverse + 127) / 255
                    (0xFF shl 24) or (channel(16) shl 16) or (channel(8) shl 8) or channel(0)
                }
            }
        }
        return into
    }

    // -- GIF ---------------------------------------------------------------

    /**
     * GIF87a/89a: the blocks walked by their own lengths, each image's LZW
     * data decoded on demand. Interlaced images, local and global colour
     * tables and the Graphic Control Extension's delay, disposal and
     * transparent index are honoured; every other extension is skipped.
     */
    internal object Gif {

        private class Image(
            val frame: Frame,
            val colours: IntArray?,
            val transparent: Int,
            val interlaced: Boolean,
            val minCodeSize: Int,
            val dataStart: Int,
        )

        fun open(bytes: ByteArray): Source? {
            if (bytes.size < 13) return null
            var width = le16(bytes, 6)
            var height = le16(bytes, 8)
            val packed = bytes[10].toInt() and 0xFF
            var at = 13
            val global = if (packed and 0x80 != 0) {
                val count = 1 shl ((packed and 0x07) + 1)
                if (at + count * 3 > bytes.size) return null
                colourTable(bytes, at, count).also { at += count * 3 }
            } else {
                null
            }
            val images = ArrayList<Image>()
            // The pending Graphic Control Extension: it applies to the NEXT image only.
            var delay = 0
            var disposal = Disposal.NONE
            var transparent = -1
            walk@ while (at < bytes.size && images.size < MAX_FRAMES) {
                when (bytes[at].toInt() and 0xFF) {
                    0x21 -> {
                        if (at + 2 > bytes.size) break@walk
                        val label = bytes[at + 1].toInt() and 0xFF
                        if (label == 0xF9 && at + 7 < bytes.size && (bytes[at + 2].toInt() and 0xFF) >= 4) {
                            val flags = bytes[at + 3].toInt() and 0xFF
                            disposal = when ((flags shr 2) and 0x07) {
                                2 -> Disposal.BACKGROUND
                                3 -> Disposal.PREVIOUS
                                else -> Disposal.NONE
                            }
                            delay = le16(bytes, at + 4)
                            transparent = if (flags and 0x01 != 0) bytes[at + 6].toInt() and 0xFF else -1
                        }
                        at = skipSubBlocks(bytes, at + 2) ?: break@walk
                    }
                    0x2C -> {
                        if (at + 10 > bytes.size) break@walk
                        val x = le16(bytes, at + 1)
                        val y = le16(bytes, at + 3)
                        val w = le16(bytes, at + 5)
                        val h = le16(bytes, at + 7)
                        val flags = bytes[at + 9].toInt() and 0xFF
                        at += 10
                        val local = if (flags and 0x80 != 0) {
                            val count = 1 shl ((flags and 0x07) + 1)
                            if (at + count * 3 > bytes.size) break@walk
                            colourTable(bytes, at, count).also { at += count * 3 }
                        } else {
                            null
                        }
                        if (at >= bytes.size) break@walk
                        val minCodeSize = bytes[at].toInt() and 0xFF
                        val dataStart = at + 1
                        // A truncated last image still counts: what of it decodes is drawn.
                        val next = skipSubBlocks(bytes, dataStart)
                        if (w > 0 && h > 0) {
                            images += Image(
                                frame = Frame(
                                    x = x,
                                    y = y,
                                    width = w,
                                    height = h,
                                    statedMs = AnimationPlan.gifDelayMs(delay),
                                    blend = true,
                                    disposal = disposal,
                                ),
                                colours = local ?: global,
                                transparent = transparent,
                                interlaced = flags and 0x40 != 0,
                                minCodeSize = minCodeSize,
                                dataStart = dataStart,
                            )
                        }
                        delay = 0
                        disposal = Disposal.NONE
                        transparent = -1
                        at = next ?: break@walk
                    }
                    0x3B -> break@walk
                    else -> break@walk
                }
            }
            if (images.isEmpty()) return null
            // A screen that says 0 × 0 is drawn as large as its images reach, as browsers do.
            if (width == 0 || height == 0) {
                width = images.maxOf { it.frame.x + it.frame.width }
                height = images.maxOf { it.frame.y + it.frame.height }
            }
            val canvasWidth = width
            val canvasHeight = height
            return object : Source {
                override val width: Int = canvasWidth
                override val height: Int = canvasHeight
                override val frames: List<Frame> = images.map { it.frame }
                override fun pixels(index: Int): IntArray? = images.getOrNull(index)?.let { decodeImage(bytes, it) }
            }
        }

        private fun decodeImage(bytes: ByteArray, image: Image): IntArray? {
            val frame = image.frame
            val count = frame.width.toLong() * frame.height
            if (count > MAX_CANVAS_PIXELS) return null
            val colours = image.colours ?: return null
            val data = subBlockData(bytes, image.dataStart)
            val indices = ByteArray(count.toInt())
            val decoded = lzw(data, image.minCodeSize, indices)
            val rows = if (image.interlaced) interlacedRows(frame.height) else null
            val argb = IntArray(count.toInt())
            // Pixels past what decoded stay transparent: a truncated GIF shows what it has.
            for (i in 0 until decoded) {
                val index = indices[i].toInt() and 0xFF
                val colour = if (index == image.transparent || index >= colours.size) 0 else colours[index]
                val row = i / frame.width
                val target = (rows?.get(row) ?: row) * frame.width + i % frame.width
                argb[target] = colour
            }
            return argb
        }

        /** Row r of the stream lands on display row rows[r]: passes from 0 by 8, 4 by 8, 2 by 4, 1 by 2. */
        internal fun interlacedRows(height: Int): IntArray {
            val rows = IntArray(height)
            var r = 0
            for ((start, step) in INTERLACE_PASSES) {
                var y = start
                while (y < height) {
                    rows[r++] = y
                    y += step
                }
            }
            return rows
        }

        private val INTERLACE_PASSES = listOf(0 to 8, 4 to 8, 2 to 4, 1 to 2)

        /**
         * GIF's variable-width LZW, least significant bit first: [data] decoded
         * into [out]; returns how many indices were written. Stops at the end
         * code, at the end of the data, or at a code that cannot be — what was
         * written by then stands. A full table (4096 codes) is used as it is
         * until a clear code, the "deferred clear" GIF allows.
         */
        internal fun lzw(data: ByteArray, minCodeSize: Int, out: ByteArray): Int {
            if (minCodeSize !in 1..11) return 0
            val clear = 1 shl minCodeSize
            val end = clear + 1
            val prefix = IntArray(MAX_CODES)
            val suffix = ByteArray(MAX_CODES)
            for (i in 0 until clear) suffix[i] = i.toByte()
            val stack = ByteArray(MAX_CODES + 1)
            var codeSize = minCodeSize + 1
            var next = clear + 2
            var previous = -1
            var first = 0
            var written = 0
            var bits = 0
            var bitCount = 0
            var at = 0
            while (written < out.size) {
                while (bitCount < codeSize) {
                    if (at >= data.size) return written
                    bits = bits or ((data[at++].toInt() and 0xFF) shl bitCount)
                    bitCount += 8
                }
                var code = bits and ((1 shl codeSize) - 1)
                bits = bits ushr codeSize
                bitCount -= codeSize
                if (code == clear) {
                    codeSize = minCodeSize + 1
                    next = clear + 2
                    previous = -1
                    continue
                }
                if (code == end) return written
                if (previous == -1) {
                    if (code >= clear) return written
                    out[written++] = code.toByte()
                    first = code
                    previous = code
                    // The width can grow after the very first code too — with a
                    // one-bit minimum, where clear + 2 already fills it.
                    if (next >= (1 shl codeSize) && codeSize < MAX_CODE_SIZE) codeSize++
                    continue
                }
                val incoming = code
                var depth = 0
                if (code >= next) {
                    // The one code the decoder has not made yet: the previous string plus its own first byte.
                    if (code > next || next >= MAX_CODES) return written
                    stack[depth++] = first.toByte()
                    code = previous
                }
                while (code >= clear) {
                    if (depth >= stack.size) return written
                    stack[depth++] = suffix[code]
                    code = prefix[code]
                }
                first = code
                stack[depth++] = first.toByte()
                while (depth > 0 && written < out.size) out[written++] = stack[--depth]
                if (next < MAX_CODES) {
                    prefix[next] = previous
                    suffix[next] = first.toByte()
                    next++
                    if (next >= (1 shl codeSize) && codeSize < MAX_CODE_SIZE) codeSize++
                }
                previous = incoming
            }
            return written
        }

        private const val MAX_CODES = 4096
        private const val MAX_CODE_SIZE = 12

        private fun colourTable(bytes: ByteArray, at: Int, count: Int): IntArray = IntArray(count) { i ->
            val r = bytes[at + i * 3].toInt() and 0xFF
            val g = bytes[at + i * 3 + 1].toInt() and 0xFF
            val b = bytes[at + i * 3 + 2].toInt() and 0xFF
            (0xFF shl 24) or (r shl 16) or (g shl 8) or b
        }

        /** Past a run of sub-blocks starting at [start], or null when the file ends inside it. */
        private fun skipSubBlocks(bytes: ByteArray, start: Int): Int? {
            var at = start
            while (at < bytes.size) {
                val length = bytes[at].toInt() and 0xFF
                if (length == 0) return at + 1
                at += 1 + length
            }
            return null
        }

        /** The sub-blocks from [start], joined — as much of them as the file holds. */
        private fun subBlockData(bytes: ByteArray, start: Int): ByteArray {
            val out = ByteArrayOutputStream()
            var at = start
            while (at < bytes.size) {
                val length = bytes[at].toInt() and 0xFF
                if (length == 0) break
                val available = minOf(length, bytes.size - at - 1)
                out.write(bytes, at + 1, available)
                at += 1 + length
            }
            return out.toByteArray()
        }

        private fun le16(bytes: ByteArray, at: Int): Int =
            (bytes[at].toInt() and 0xFF) or ((bytes[at + 1].toInt() and 0xFF) shl 8)
    }

    // -- WebP --------------------------------------------------------------

    /**
     * An animated WebP: `VP8X` for the canvas, one `ANMF` per frame with its
     * offset, size, duration, blending and disposal. Each frame's own chunks
     * become a standalone still WebP for [StillDecoder]: a lossless `VP8L` on
     * its own, a lossy `VP8 ` on its own, or `VP8X` + `ALPH` + `VP8 ` for a
     * lossy frame with an alpha plane.
     */
    internal object WebP {

        private class Chunk(val type: String, val start: Int, val length: Int)

        fun open(bytes: ByteArray, stills: StillDecoder): Source? {
            val top = chunks(bytes, 12, bytes.size)
            val vp8x = top.firstOrNull { it.type == "VP8X" }?.takeIf { it.length >= 10 } ?: return null
            val canvasWidth = le24(bytes, vp8x.start + 4) + 1
            val canvasHeight = le24(bytes, vp8x.start + 7) + 1
            val read = ArrayList<Frame>()
            val stillsOf = ArrayList<ByteArray>()
            for (anmf in top.filter { it.type == "ANMF" }) {
                if (read.size >= MAX_FRAMES) break
                if (anmf.length < 16) return null
                val s = anmf.start
                val flags = bytes[s + 15].toInt() and 0xFF
                val frame = Frame(
                    x = le24(bytes, s) * 2,
                    y = le24(bytes, s + 3) * 2,
                    width = le24(bytes, s + 6) + 1,
                    height = le24(bytes, s + 9) + 1,
                    statedMs = le24(bytes, s + 12).toLong(),
                    blend = flags and 0x02 == 0,
                    disposal = if (flags and 0x01 != 0) Disposal.BACKGROUND else Disposal.NONE,
                )
                val still = still(bytes, chunks(bytes, s + 16, s + anmf.length), frame) ?: return null
                read += frame
                stillsOf += still
            }
            if (read.isEmpty()) return null
            return object : Source {
                override val width: Int = canvasWidth
                override val height: Int = canvasHeight
                override val frames: List<Frame> = read
                override fun pixels(index: Int): IntArray? {
                    val frame = read.getOrNull(index) ?: return null
                    val decoded = stills.decode(stillsOf[index]) ?: return null
                    return decoded.argb.takeIf { decoded.width == frame.width && decoded.height == frame.height }
                }
            }
        }

        /** One frame's chunks as a WebP of its own. */
        private fun still(bytes: ByteArray, parts: List<Chunk>, frame: Frame): ByteArray? {
            val lossless = parts.firstOrNull { it.type == "VP8L" }
            val lossy = parts.firstOrNull { it.type == "VP8 " }
            val alpha = parts.firstOrNull { it.type == "ALPH" }
            val body = ByteArrayOutputStream()
            when {
                lossless != null -> writeChunk(body, bytes, lossless)
                lossy != null && alpha != null -> {
                    val header = ByteArray(10)
                    header[0] = 0x10 // the alpha flag
                    putLe24(header, 4, frame.width - 1)
                    putLe24(header, 7, frame.height - 1)
                    writeChunk(body, "VP8X", header, 0, header.size)
                    writeChunk(body, bytes, alpha)
                    writeChunk(body, bytes, lossy)
                }
                lossy != null -> writeChunk(body, bytes, lossy)
                else -> return null
            }
            val payload = body.toByteArray()
            val out = ByteArrayOutputStream(payload.size + 12)
            out.write("RIFF".toByteArray(Charsets.US_ASCII))
            out.write(le32(payload.size + 4))
            out.write("WEBP".toByteArray(Charsets.US_ASCII))
            out.write(payload)
            return out.toByteArray()
        }

        /** The chunks between [start] and [end], each padded to an even length as RIFF requires. */
        private fun chunks(bytes: ByteArray, start: Int, end: Int): List<Chunk> {
            val list = ArrayList<Chunk>()
            var at = start
            val limit = minOf(end, bytes.size)
            while (at + 8 <= limit) {
                val type = String(bytes, at, 4, Charsets.US_ASCII)
                val length = le32At(bytes, at + 4)
                if (length < 0 || length > limit - at - 8) break
                list += Chunk(type, at + 8, length)
                at += 8 + length + (length and 1)
            }
            return list
        }

        private fun writeChunk(out: ByteArrayOutputStream, bytes: ByteArray, chunk: Chunk) =
            writeChunk(out, chunk.type, bytes, chunk.start, chunk.length)

        private fun writeChunk(out: ByteArrayOutputStream, type: String, bytes: ByteArray, start: Int, length: Int) {
            out.write(type.toByteArray(Charsets.US_ASCII))
            out.write(le32(length))
            out.write(bytes, start, length)
            if (length and 1 == 1) out.write(0)
        }

        private fun le24(bytes: ByteArray, at: Int): Int =
            (bytes[at].toInt() and 0xFF) or ((bytes[at + 1].toInt() and 0xFF) shl 8) or
                ((bytes[at + 2].toInt() and 0xFF) shl 16)

        private fun le32At(bytes: ByteArray, at: Int): Int =
            le24(bytes, at) or ((bytes[at + 3].toInt() and 0xFF) shl 24)

        private fun le32(value: Int) = byteArrayOf(
            value.toByte(), (value shr 8).toByte(), (value shr 16).toByte(), (value shr 24).toByte(),
        )

        private fun putLe24(into: ByteArray, at: Int, value: Int) {
            into[at] = value.toByte()
            into[at + 1] = (value shr 8).toByte()
            into[at + 2] = (value shr 16).toByte()
        }
    }

    // -- APNG --------------------------------------------------------------

    /**
     * An animated PNG: `IHDR` for the canvas, an `fcTL` before each frame with
     * its offset, size, delay, disposal and blending, then that frame's data
     * — the `IDAT`s for a first frame that is also the default image, `fdAT`s
     * after. Each frame becomes a PNG of its own (its own `IHDR` size, the
     * file's `PLTE` and `tRNS`, its data as one `IDAT`) for [StillDecoder].
     * A default image with no `fcTL` before it is not part of the animation,
     * as the APNG specification says.
     */
    internal object Apng {

        private class Building(val frame: Frame, val data: ByteArrayOutputStream = ByteArrayOutputStream())

        fun open(bytes: ByteArray, stills: StillDecoder): Source? {
            var at = 8
            var ihdr: ByteArray? = null
            var plte: ByteArray? = null
            var trns: ByteArray? = null
            val built = ArrayList<Building>()
            var current: Building? = null
            var seenData = false
            while (at + 12 <= bytes.size) {
                val length = be32(bytes, at)
                if (length < 0 || length > bytes.size - at - 12) break
                val type = String(bytes, at + 4, 4, Charsets.US_ASCII)
                val data = at + 8
                when (type) {
                    "IHDR" -> if (length >= 13) ihdr = bytes.copyOfRange(data, data + 13)
                    "PLTE" -> plte = bytes.copyOfRange(data, data + length)
                    "tRNS" -> trns = bytes.copyOfRange(data, data + length)
                    "fcTL" -> {
                        if (length < 26 || built.size >= MAX_FRAMES) break
                        val dispose = bytes[data + 24].toInt() and 0xFF
                        val frame = Frame(
                            x = be32(bytes, data + 12),
                            y = be32(bytes, data + 16),
                            width = be32(bytes, data + 4),
                            height = be32(bytes, data + 8),
                            statedMs = AnimationPlan.apngDelayMs(be16(bytes, data + 20), be16(bytes, data + 22)),
                            blend = (bytes[data + 25].toInt() and 0xFF) == 1,
                            disposal = when (dispose) {
                                1 -> Disposal.BACKGROUND
                                // "If the first fcTL chunk uses a dispose_op of
                                // APNG_DISPOSE_OP_PREVIOUS it should be treated
                                // as APNG_DISPOSE_OP_BACKGROUND."
                                2 -> if (built.isEmpty()) Disposal.BACKGROUND else Disposal.PREVIOUS
                                else -> Disposal.NONE
                            },
                        )
                        if (frame.width <= 0 || frame.height <= 0 || frame.x < 0 || frame.y < 0) return null
                        current = Building(frame).also { built += it }
                    }
                    "IDAT" -> {
                        // Part of the animation only when an fcTL came first.
                        if (!seenData || current?.data?.size() != 0) current?.data?.write(bytes, data, length)
                        seenData = true
                    }
                    "fdAT" -> if (length >= 4) current?.data?.write(bytes, data + 4, length - 4)
                    "IEND" -> break
                }
                at += 12 + length
            }
            val header = ihdr ?: return null
            val withData = built.filter { it.data.size() > 0 }
            if (withData.isEmpty()) return null
            val canvasWidth = be32(header, 0)
            val canvasHeight = be32(header, 4)
            return object : Source {
                override val width: Int = canvasWidth
                override val height: Int = canvasHeight
                override val frames: List<Frame> = withData.map { it.frame }
                override fun pixels(index: Int): IntArray? {
                    val building = withData.getOrNull(index) ?: return null
                    val png = still(header, plte, trns, building)
                    val decoded = stills.decode(png) ?: return null
                    return decoded.argb.takeIf {
                        decoded.width == building.frame.width && decoded.height == building.frame.height
                    }
                }
            }
        }

        private fun still(header: ByteArray, plte: ByteArray?, trns: ByteArray?, building: Building): ByteArray {
            val out = ByteArrayOutputStream()
            out.write(SIGNATURE)
            val ihdr = header.copyOf()
            putBe32(ihdr, 0, building.frame.width)
            putBe32(ihdr, 4, building.frame.height)
            writeChunk(out, "IHDR", ihdr)
            plte?.let { writeChunk(out, "PLTE", it) }
            trns?.let { writeChunk(out, "tRNS", it) }
            writeChunk(out, "IDAT", building.data.toByteArray())
            writeChunk(out, "IEND", ByteArray(0))
            return out.toByteArray()
        }

        private fun writeChunk(out: ByteArrayOutputStream, type: String, data: ByteArray) {
            val typeBytes = type.toByteArray(Charsets.US_ASCII)
            val length = ByteArray(4).also { putBe32(it, 0, data.size) }
            out.write(length)
            out.write(typeBytes)
            out.write(data)
            val crc = CRC32().apply {
                update(typeBytes)
                update(data)
            }.value.toInt()
            out.write(ByteArray(4).also { putBe32(it, 0, crc) })
        }

        private val SIGNATURE = byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A)

        private fun be32(bytes: ByteArray, at: Int): Int =
            ((bytes[at].toInt() and 0xFF) shl 24) or ((bytes[at + 1].toInt() and 0xFF) shl 16) or
                ((bytes[at + 2].toInt() and 0xFF) shl 8) or (bytes[at + 3].toInt() and 0xFF)

        private fun be16(bytes: ByteArray, at: Int): Int =
            ((bytes[at].toInt() and 0xFF) shl 8) or (bytes[at + 1].toInt() and 0xFF)

        private fun putBe32(into: ByteArray, at: Int, value: Int) {
            into[at] = (value shr 24).toByte()
            into[at + 1] = (value shr 16).toByte()
            into[at + 2] = (value shr 8).toByte()
            into[at + 3] = value.toByte()
        }
    }
}
