/*
 * AnimatedFixtures.kt
 * Family Connect (Android) — tests
 *
 * Animated pictures built in the test, byte by byte (#96): a GIF from a
 * small LZW ENCODER written the classic way (compress/Poskanzer: the code
 * width grows after the code that fills it, a clear code when the table is
 * full), and an APNG or an animated WebP assembled around stills the
 * platform encoder made (Bitmap.compress under Robolectric's native
 * graphics). What the decoders under test have to read is then exactly
 * what the test says it is — and the GIF's first frame is also checked
 * against the platform's own GIF decoder, so this encoder cannot hide a
 * mistake the decoder shares with it.
 */

package me.nettrash.familyconnect.testutil

import android.graphics.Bitmap
import java.io.ByteArrayOutputStream
import java.util.zip.CRC32

object AnimatedFixtures {

    /** One GIF image: where it sits, its palette indices, and its Graphic Control Extension. */
    class GifImage(
        val x: Int,
        val y: Int,
        val width: Int,
        val height: Int,
        /** Row by row, in DISPLAY order (interlacing is applied by the writer). */
        val indices: ByteArray,
        val delayCs: Int = 10,
        /** 0/1 none, 2 background, 3 previous. */
        val disposal: Int = 0,
        val transparent: Int? = null,
        val localPalette: IntArray? = null,
        val interlaced: Boolean = false,
    )

    /** A GIF89a of [width] × [height] with a global [palette] (RGB ints; a power of two, 2…256 long). */
    fun gif(width: Int, height: Int, palette: IntArray?, images: List<GifImage>, loop: Boolean = true): ByteArray {
        val out = ByteArrayOutputStream()
        out.write("GIF89a".toByteArray(Charsets.US_ASCII))
        le16(out, width)
        le16(out, height)
        if (palette != null) {
            out.write(0x80 or (bits(palette.size) - 1))
        } else {
            out.write(0)
        }
        out.write(0) // background index
        out.write(0) // aspect
        palette?.let { writePalette(out, it) }
        if (loop) {
            out.write(byteArrayOf(0x21, 0xFF.toByte(), 0x0B))
            out.write("NETSCAPE2.0".toByteArray(Charsets.US_ASCII))
            out.write(byteArrayOf(3, 1, 0, 0, 0))
        }
        for (image in images) {
            out.write(byteArrayOf(0x21, 0xF9.toByte(), 4))
            out.write((image.disposal shl 2) or (if (image.transparent != null) 1 else 0))
            le16(out, image.delayCs)
            out.write(image.transparent ?: 0)
            out.write(0)
            out.write(0x2C)
            le16(out, image.x)
            le16(out, image.y)
            le16(out, image.width)
            le16(out, image.height)
            val local = image.localPalette
            out.write(
                (if (local != null) 0x80 or (bits(local.size) - 1) else 0) or (if (image.interlaced) 0x40 else 0),
            )
            local?.let { writePalette(out, it) }
            val colours = (local ?: palette)!!.size
            val minCodeSize = maxOf(2, bits(colours))
            out.write(minCodeSize)
            val stream = if (image.interlaced) interlace(image.indices, image.width, image.height) else image.indices
            val data = lzw(stream, minCodeSize)
            var at = 0
            while (at < data.size) {
                val n = minOf(255, data.size - at)
                out.write(n)
                out.write(data, at, n)
                at += n
            }
            out.write(0)
        }
        out.write(0x3B)
        return out.toByteArray()
    }

    /** GIF LZW, least significant bit first: clear, the codes, end. */
    fun lzw(indices: ByteArray, minCodeSize: Int): ByteArray {
        val clear = 1 shl minCodeSize
        val end = clear + 1
        val out = ByteArrayOutputStream()
        var bitBuffer = 0L
        var bitCount = 0
        var codeSize = minCodeSize + 1
        var next = clear + 2
        val table = HashMap<Int, Int>()
        fun emit(code: Int) {
            bitBuffer = bitBuffer or (code.toLong() shl bitCount)
            bitCount += codeSize
            while (bitCount >= 8) {
                out.write((bitBuffer and 0xFF).toInt())
                bitBuffer = bitBuffer ushr 8
                bitCount -= 8
            }
        }
        emit(clear)
        if (indices.isNotEmpty()) {
            var prefix = indices[0].toInt() and 0xFF
            for (i in 1 until indices.size) {
                val c = indices[i].toInt() and 0xFF
                val key = (prefix shl 8) or c
                val known = table[key]
                if (known != null) {
                    prefix = known
                    continue
                }
                emit(prefix)
                if (next >= (1 shl codeSize) && codeSize < 12) codeSize++
                if (next < 4096) {
                    table[key] = next++
                } else {
                    emit(clear)
                    table.clear()
                    codeSize = minCodeSize + 1
                    next = clear + 2
                }
                prefix = c
            }
            emit(prefix)
            if (next >= (1 shl codeSize) && codeSize < 12) codeSize++
        }
        emit(end)
        if (bitCount > 0) out.write((bitBuffer and 0xFF).toInt())
        return out.toByteArray()
    }

    /** Display rows into the stream order an interlaced GIF stores them in. */
    private fun interlace(indices: ByteArray, width: Int, height: Int): ByteArray {
        val out = ByteArray(indices.size)
        var r = 0
        for ((start, step) in listOf(0 to 8, 4 to 8, 2 to 4, 1 to 2)) {
            var y = start
            while (y < height) {
                System.arraycopy(indices, y * width, out, r * width, width)
                r++
                y += step
            }
        }
        return out
    }

    private fun writePalette(out: ByteArrayOutputStream, palette: IntArray) {
        for (rgb in palette) {
            out.write((rgb shr 16) and 0xFF)
            out.write((rgb shr 8) and 0xFF)
            out.write(rgb and 0xFF)
        }
    }

    private fun bits(size: Int): Int {
        var n = 1
        while ((1 shl n) < size) n++
        return n
    }

    private fun le16(out: ByteArrayOutputStream, value: Int) {
        out.write(value and 0xFF)
        out.write((value shr 8) and 0xFF)
    }

    // -- APNG -------------------------------------------------------------------

    /** One APNG frame: its pixels and its fcTL. */
    class ApngFrame(
        val bitmap: Bitmap,
        val x: Int = 0,
        val y: Int = 0,
        val delayNum: Int = 1,
        val delayDen: Int = 10,
        /** 0 none, 1 background, 2 previous. */
        val dispose: Int = 0,
        /** 0 source, 1 over. */
        val blend: Int = 0,
    )

    /**
     * An APNG of [width] × [height] whose frames are [frames], the first also
     * the default image when [firstIsDefault]; each frame's data is the IDAT
     * the platform's PNG encoder wrote for its bitmap. Null when the encoder
     * did not write every frame in the same colour type (they must share
     * the canvas IHDR).
     */
    fun apng(width: Int, height: Int, frames: List<ApngFrame>, firstIsDefault: Boolean = true): ByteArray? {
        val encoded = frames.map { chunks(png(it.bitmap)) }
        val header = encoded.first().first { it.first == "IHDR" }.second
        if (encoded.any { e -> e.first { it.first == "IHDR" }.second.copyOfRange(8, 13).toList() != header.copyOfRange(8, 13).toList() }) {
            return null
        }
        val out = ByteArrayOutputStream()
        out.write(byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A))
        val ihdr = header.copyOf().also {
            be32(it, 0, width)
            be32(it, 4, height)
        }
        chunk(out, "IHDR", ihdr)
        chunk(out, "acTL", ByteArray(8).also { be32(it, 0, frames.size) })
        var sequence = 0
        frames.forEachIndexed { index, frame ->
            val fctl = ByteArray(26)
            be32(fctl, 0, sequence++)
            be32(fctl, 4, frame.bitmap.width)
            be32(fctl, 8, frame.bitmap.height)
            be32(fctl, 12, frame.x)
            be32(fctl, 16, frame.y)
            fctl[20] = (frame.delayNum shr 8).toByte()
            fctl[21] = frame.delayNum.toByte()
            fctl[22] = (frame.delayDen shr 8).toByte()
            fctl[23] = frame.delayDen.toByte()
            fctl[24] = frame.dispose.toByte()
            fctl[25] = frame.blend.toByte()
            val data = encoded[index].filter { it.first == "IDAT" }.fold(ByteArray(0)) { acc, c -> acc + c.second }
            if (index == 0 && firstIsDefault) {
                chunk(out, "fcTL", fctl)
                chunk(out, "IDAT", data)
            } else {
                if (index == 0) chunk(out, "IDAT", data) // a default image outside the animation
                chunk(out, "fcTL", fctl)
                chunk(out, "fdAT", ByteArray(4).also { be32(it, 0, sequence++) } + data)
            }
        }
        chunk(out, "IEND", ByteArray(0))
        return out.toByteArray()
    }

    private fun png(bitmap: Bitmap): ByteArray = ByteArrayOutputStream().use {
        bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)
        it.toByteArray()
    }

    /** A PNG's chunks: (type, data). */
    fun chunks(png: ByteArray): List<Pair<String, ByteArray>> {
        val list = ArrayList<Pair<String, ByteArray>>()
        var at = 8
        while (at + 12 <= png.size) {
            val length = ((png[at].toInt() and 0xFF) shl 24) or ((png[at + 1].toInt() and 0xFF) shl 16) or
                ((png[at + 2].toInt() and 0xFF) shl 8) or (png[at + 3].toInt() and 0xFF)
            val type = String(png, at + 4, 4, Charsets.US_ASCII)
            list += type to png.copyOfRange(at + 8, at + 8 + length)
            at += 12 + length
        }
        return list
    }

    private fun chunk(out: ByteArrayOutputStream, type: String, data: ByteArray) {
        val typeBytes = type.toByteArray(Charsets.US_ASCII)
        out.write(ByteArray(4).also { be32(it, 0, data.size) })
        out.write(typeBytes)
        out.write(data)
        val crc = CRC32().apply {
            update(typeBytes)
            update(data)
        }.value.toInt()
        out.write(ByteArray(4).also { be32(it, 0, crc) })
    }

    private fun be32(into: ByteArray, at: Int, value: Int) {
        into[at] = (value shr 24).toByte()
        into[at + 1] = (value shr 16).toByte()
        into[at + 2] = (value shr 8).toByte()
        into[at + 3] = value.toByte()
    }

    // -- Animated WebP ------------------------------------------------------------

    /** One animated-WebP frame: a still the platform encoded, and its ANMF fields. */
    class WebPFrame(
        val bitmap: Bitmap,
        val x: Int = 0,
        val y: Int = 0,
        val durationMs: Int = 100,
        val blend: Boolean = true,
        val disposeToBackground: Boolean = false,
        val lossless: Boolean = true,
    )

    /**
     * An animated WebP of [width] × [height] around stills the platform
     * encoded; null when the platform wrote no WebP (an encoder this
     * Robolectric build lacks).
     */
    fun animatedWebP(width: Int, height: Int, frames: List<WebPFrame>): ByteArray? {
        val body = ByteArrayOutputStream()
        val vp8x = ByteArray(10)
        vp8x[0] = 0x12 // animation + alpha
        le24(vp8x, 4, width - 1)
        le24(vp8x, 7, height - 1)
        riffChunk(body, "VP8X", vp8x)
        riffChunk(body, "ANIM", ByteArray(6))
        for (frame in frames) {
            val still = webp(frame.bitmap, frame.lossless) ?: return null
            // The still's own chunks after RIFF/size/WEBP — minus any VP8X header.
            val parts = riffChunks(still).filter { it.first != "VP8X" }
            val anmf = ByteArrayOutputStream()
            val head = ByteArray(16)
            le24(head, 0, frame.x / 2)
            le24(head, 3, frame.y / 2)
            le24(head, 6, frame.bitmap.width - 1)
            le24(head, 9, frame.bitmap.height - 1)
            le24(head, 12, frame.durationMs)
            head[15] = ((if (frame.blend) 0 else 0x02) or (if (frame.disposeToBackground) 0x01 else 0)).toByte()
            anmf.write(head)
            parts.forEach { (type, data) -> riffChunk(anmf, type, data) }
            riffChunk(body, "ANMF", anmf.toByteArray())
        }
        val payload = body.toByteArray()
        val out = ByteArrayOutputStream()
        out.write("RIFF".toByteArray(Charsets.US_ASCII))
        out.write(le32(payload.size + 4))
        out.write("WEBP".toByteArray(Charsets.US_ASCII))
        out.write(payload)
        return out.toByteArray()
    }

    /** A still WebP's chunks: (type, data). */
    fun riffChunks(webp: ByteArray): List<Pair<String, ByteArray>> {
        val list = ArrayList<Pair<String, ByteArray>>()
        var at = 12
        while (at + 8 <= webp.size) {
            val type = String(webp, at, 4, Charsets.US_ASCII)
            val length = (webp[at + 4].toInt() and 0xFF) or ((webp[at + 5].toInt() and 0xFF) shl 8) or
                ((webp[at + 6].toInt() and 0xFF) shl 16) or ((webp[at + 7].toInt() and 0xFF) shl 24)
            list += type to webp.copyOfRange(at + 8, at + 8 + length)
            at += 8 + length + (length and 1)
        }
        return list
    }

    private fun webp(bitmap: Bitmap, lossless: Boolean): ByteArray? = runCatching {
        ByteArrayOutputStream().use {
            val format = if (lossless) Bitmap.CompressFormat.WEBP_LOSSLESS else Bitmap.CompressFormat.WEBP_LOSSY
            if (!bitmap.compress(format, if (lossless) 100 else 90, it)) return null
            it.toByteArray().takeIf { bytes -> bytes.size > 12 && String(bytes, 8, 4, Charsets.US_ASCII) == "WEBP" }
        }
    }.getOrNull()

    private fun riffChunk(out: ByteArrayOutputStream, type: String, data: ByteArray) {
        out.write(type.toByteArray(Charsets.US_ASCII))
        out.write(le32(data.size))
        out.write(data)
        if (data.size and 1 == 1) out.write(0)
    }

    private fun le24(into: ByteArray, at: Int, value: Int) {
        into[at] = value.toByte()
        into[at + 1] = (value shr 8).toByte()
        into[at + 2] = (value shr 16).toByte()
    }

    private fun le32(value: Int) = byteArrayOf(
        value.toByte(), (value shr 8).toByte(), (value shr 16).toByte(), (value shr 24).toByte(),
    )
}
