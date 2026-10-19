/*
 * AnimationDeviceTest.kt
 * Family Connect (Android)
 *
 * Issue #96 on a real encoder: an animated GIF and an animated PNG made into
 * the protocol's silent looping MP4 (docs/protocol.md, "Animations" → "The
 * file"), read back with the platform's own readers and a box walk, the way
 * another member's device will meet it:
 *
 *  - H.264 in MP4, `moov` before `mdat`, ONE track and it is video — no audio;
 *  - the size the plan gives (the profile's), and `duration_ms` the plan's;
 *  - a timescale that is a multiple of 30, and every sample lasting EXACTLY
 *    its ticks (the `stts`), the last one included;
 *  - transparent pixels drawn on white, and the poster a JPEG of frame zero;
 *  - and through MediaPrep's photo-picker door: an animation, kind video.
 *
 * The JVM tests prove the timing (AnimationPlanVectorsTest), the frames
 * (AnimatedPictureTest) and the routing (AnimationRoutingTest); only this
 * proves MediaCodec and the muxer then write it.
 *
 * Run on an emulator, never on a phone somebody uses: build
 * assembleStandardDebug and assembleStandardDebugAndroidTest with -PnoBump,
 * `adb -s <serial> install -r -t` both APKs, and
 *   adb -s <serial> shell am instrument -w \
 *     -e class me.nettrash.familyconnect.data.repo.AnimationDeviceTest \
 *     me.nettrash.familyconnect.test/androidx.test.runner.AndroidJUnitRunner
 * CI has no emulator lane, so CI does not run this.
 */

package me.nettrash.familyconnect.data.repo

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Color
import android.media.MediaExtractor
import android.media.MediaFormat
import android.media.MediaMetadataRetriever
import android.net.Uri
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.google.common.truth.Truth.assertThat
import com.google.common.truth.Truth.assertWithMessage
import kotlinx.coroutines.runBlocking
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import java.io.ByteArrayOutputStream
import java.io.File
import java.nio.ByteBuffer
import java.util.zip.CRC32

@RunWith(AndroidJUnit4::class)
class AnimationDeviceTest {

    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val work = File(context.cacheDir, "animation-device-test")

    @Before
    fun setUp() {
        work.deleteRecursively()
        work.mkdirs()
    }

    @After
    fun tearDown() {
        work.deleteRecursively()
    }

    /**
     * A 64 × 48 GIF: frame 0 red on the right half and TRANSPARENT on the
     * left; then blue squares that flash and hold — stated 100, 0 ("the
     * default"), 20 and 1000 ms.
     */
    private fun gif(): ByteArray {
        val w = 64
        val h = 48
        val frame0 = ByteArray(w * h) { if (it % w < w / 2) 2 else 0 }
        val square = ByteArray(16 * 16) { 1 }
        return Gif.write(
            w, h, intArrayOf(0xFF0000, 0x0000FF, 0x000000, 0xFFFFFF),
            listOf(
                Gif.Image(0, 0, w, h, frame0, delayCs = 10, transparent = 2),
                Gif.Image(40, 8, 16, 16, square, delayCs = 0),
                Gif.Image(40, 24, 16, 16, square, delayCs = 2),
                Gif.Image(4, 4, 16, 16, square, delayCs = 100),
            ),
        )
    }

    @Test
    fun anAnimatedGifBecomesTheProtocolsSilentLoopingMp4(): Unit = runBlocking {
        val source = gif()
        val picture = requireNotNull(AnimatedPicture.open(source))
        val plan = requireNotNull(
            AnimationPlan.plan(picture.width.toLong(), picture.height.toLong(), picture.frames.map { it.statedMs }),
        )
        val output = File(work, "out.mp4")

        val made = requireNotNull(PlatformAnimationMaker.make(source, output)) { "no animation was made" }

        assertThat(made.width).isEqualTo(plan.width)
        assertThat(made.height).isEqualTo(plan.height)
        assertThat(made.durationMs).isEqualTo(plan.durationMs)
        checkFile(output, plan)

        // Transparent pixels on white, red where it is red — frame zero.
        val first = requireNotNull(frameZero(output))
        val left = first.getPixel(first.width / 4, first.height / 2)
        val right = first.getPixel(first.width * 3 / 4, first.height / 2)
        assertWithMessage("left half was transparent: %s", Integer.toHexString(left))
            .that(minOf(Color.red(left), Color.green(left), Color.blue(left))).isAtLeast(235)
        assertWithMessage("right half is red: %s", Integer.toHexString(right))
            .that(Color.red(right) > 200 && Color.green(right) < 60 && Color.blue(right) < 60).isTrue()

        // The poster is a JPEG of frame zero.
        val poster = requireNotNull(made.poster)
        assertThat(poster.take(2)).containsExactly(0xFF.toByte(), 0xD8.toByte()).inOrder()
        val posterBitmap = requireNotNull(BitmapFactory.decodeByteArray(poster, 0, poster.size))
        assertThat(minOf(Color.red(posterBitmap.getPixel(4, 24)), Color.blue(posterBitmap.getPixel(4, 24)))).isAtLeast(235)
    }

    @Test
    fun anAnimatedPngBecomesOneToo(): Unit = runBlocking {
        val frames = listOf(Color.RED, Color.GREEN, Color.BLUE).map { colour ->
            Bitmap.createBitmap(100, 60, Bitmap.Config.ARGB_8888).apply {
                eraseColor(colour)
                setHasAlpha(true)
            }
        }
        val source = Apng.write(100, 60, frames, delayNum = 1, delayDen = 15)
        assertThat(AnimatedPicture.kindOf(source)).isEqualTo(AnimatedPicture.Kind.ANIMATED)
        val picture = requireNotNull(AnimatedPicture.open(source))
        val plan = requireNotNull(AnimationPlan.plan(100, 60, picture.frames.map { it.statedMs }))
        val output = File(work, "apng.mp4")

        requireNotNull(PlatformAnimationMaker.make(source, output))

        checkFile(output, plan)
        // 67 ms frames on the 33.3 ms grid: ticks 0-2 (66.7 ms is before 67)
        // show the first, 3-4 the second, 5-6 the third.
        assertThat(plan.samples.map { it.ticks }).containsExactly(3, 2, 2).inOrder()
    }

    @Test
    fun aPictureLargerThanTheProfileIsScaledToIt(): Unit = runBlocking {
        val w = 1600
        val h = 900
        val stripes = ByteArray(w * h) { (((it % w) / 100) % 2).toByte() }
        val source = Gif.write(
            w, h, intArrayOf(0xFF0000, 0x00FF00),
            listOf(Gif.Image(0, 0, w, h, stripes, delayCs = 5), Gif.Image(0, 0, 200, 200, ByteArray(200 * 200), delayCs = 5)),
        )
        val picture = requireNotNull(AnimatedPicture.open(source))
        val plan = requireNotNull(AnimationPlan.plan(w.toLong(), h.toLong(), picture.frames.map { it.statedMs }))
        assertThat(plan.width to plan.height).isEqualTo(1280 to 720)
        val output = File(work, "large.mp4")

        val made = requireNotNull(PlatformAnimationMaker.make(source, output))

        assertThat(made.width to made.height).isEqualTo(1280 to 720)
        checkFile(output, plan)
    }

    @Test
    fun aGifFromThePhotoPickerIsStagedAsAnAnimation(): Unit = runBlocking {
        val file = File(work, "picked.gif").apply { writeBytes(gif()) }
        val prepared = MediaPrep(context, context.contentResolver).preparePickedPicture(Uri.fromFile(file))
        try {
            assertThat(prepared.animation).isTrue()
            assertThat(prepared.kind).isEqualTo(AttachmentDto.KIND_VIDEO)
            assertThat(prepared.mime).isEqualTo("video/mp4")
            assertThat(prepared.previewJpeg).isNotNull()
            assertThat(Mp4Faststart.moovFirst(prepared.file)).isTrue()
            assertThat(AnimationSend.accepts(listOf(prepared), sticker = false, round = false)).isTrue()
        } finally {
            prepared.file.delete()
        }
    }

    // -- Reading the file back ------------------------------------------------------------

    private fun checkFile(file: File, plan: AnimationPlan.Plan) {
        assertWithMessage("moov before mdat").that(Mp4Faststart.moovFirst(file)).isTrue()

        val extractor = MediaExtractor().apply { setDataSource(file.path) }
        try {
            assertWithMessage("one track, and no audio").that(extractor.trackCount).isEqualTo(1)
            val format = extractor.getTrackFormat(0)
            assertThat(format.getString(MediaFormat.KEY_MIME)).isEqualTo(MediaFormat.MIMETYPE_VIDEO_AVC)
            assertThat(format.getInteger(MediaFormat.KEY_WIDTH)).isEqualTo(plan.width)
            assertThat(format.getInteger(MediaFormat.KEY_HEIGHT)).isEqualTo(plan.height)
        } finally {
            extractor.release()
        }

        val box = Boxes(file.readBytes())
        val moov = requireNotNull(box.find(0, box.bytes.size, "moov"))
        assertThat(box.all(moov, "trak")).hasSize(1)
        val trak = box.all(moov, "trak").single()
        val mdia = requireNotNull(box.find(trak, "mdia"))
        val hdlr = requireNotNull(box.find(mdia, "hdlr"))
        assertThat(String(box.bytes, hdlr.first + 16, 4, Charsets.US_ASCII)).isEqualTo("vide")
        val mdhd = requireNotNull(box.find(mdia, "mdhd"))
        val version = box.bytes[mdhd.first + 8].toInt()
        val timescale = box.u32(mdhd.first + if (version == 1) 28 else 20)
        val duration = if (version == 1) box.u64(mdhd.first + 32) else box.u32(mdhd.first + 24)
        assertWithMessage("timescale %s is a multiple of 30", timescale).that(timescale % 30).isEqualTo(0L)
        val perTick = timescale / 30
        assertThat(duration).isEqualTo(plan.totalTicks * perTick)

        val stbl = requireNotNull(box.find(requireNotNull(box.find(mdia, "minf")), "stbl"))
        val stts = requireNotNull(box.find(stbl, "stts"))
        val entries = box.u32(stts.first + 12).toInt()
        val deltas = ArrayList<Long>()
        for (i in 0 until entries) {
            val count = box.u32(stts.first + 16 + i * 8)
            val delta = box.u32(stts.first + 20 + i * 8)
            repeat(count.toInt()) { deltas += delta }
        }
        assertWithMessage("every sample lasts exactly its ticks")
            .that(deltas).containsExactlyElementsIn(plan.samples.map { it.ticks * perTick }).inOrder()

        val stsd = requireNotNull(box.find(stbl, "stsd"))
        assertThat(String(box.bytes, stsd.first + 20, 4, Charsets.US_ASCII)).isEqualTo("avc1")
    }

    private fun frameZero(file: File): Bitmap? {
        val retriever = MediaMetadataRetriever()
        return try {
            retriever.setDataSource(file.path)
            retriever.getFrameAtIndex(0)
        } finally {
            retriever.release()
        }
    }

    /** A box walk over a whole file: (start, end) of each box, header included. */
    private class Boxes(val bytes: ByteArray) {
        fun u32(at: Int): Long = ByteBuffer.wrap(bytes, at, 4).int.toLong() and 0xFFFF_FFFFL
        fun u64(at: Int): Long = ByteBuffer.wrap(bytes, at, 8).long

        private fun children(start: Int, end: Int): List<Pair<String, Pair<Int, Int>>> {
            val list = ArrayList<Pair<String, Pair<Int, Int>>>()
            var at = start
            while (at + 8 <= end) {
                val size = u32(at).toInt()
                if (size < 8 || at + size > end) break
                list += String(bytes, at + 4, 4, Charsets.US_ASCII) to (at to at + size)
                at += size
            }
            return list
        }

        fun find(start: Int, end: Int, type: String): Pair<Int, Int>? = children(start, end).firstOrNull { it.first == type }?.second
        fun find(parent: Pair<Int, Int>, type: String): Pair<Int, Int>? = find(parent.first + 8, parent.second, type)
        fun all(parent: Pair<Int, Int>, type: String): List<Pair<Int, Int>> =
            children(parent.first + 8, parent.second).filter { it.first == type }.map { it.second }
    }

    // -- Writing the sources ---------------------------------------------------------------

    /** A GIF89a writer: the classic LZW encoder (clear when the table fills). */
    private object Gif {
        class Image(val x: Int, val y: Int, val w: Int, val h: Int, val indices: ByteArray, val delayCs: Int, val transparent: Int? = null)

        fun write(width: Int, height: Int, palette: IntArray, images: List<Image>): ByteArray {
            val out = ByteArrayOutputStream()
            out.write("GIF89a".toByteArray(Charsets.US_ASCII))
            le16(out, width)
            le16(out, height)
            out.write(0x80 or (bits(palette.size) - 1))
            out.write(0)
            out.write(0)
            for (rgb in palette) {
                out.write((rgb shr 16) and 0xFF)
                out.write((rgb shr 8) and 0xFF)
                out.write(rgb and 0xFF)
            }
            for (image in images) {
                out.write(byteArrayOf(0x21, 0xF9.toByte(), 4))
                out.write(if (image.transparent != null) 1 else 0)
                le16(out, image.delayCs)
                out.write(image.transparent ?: 0)
                out.write(0)
                out.write(0x2C)
                le16(out, image.x)
                le16(out, image.y)
                le16(out, image.w)
                le16(out, image.h)
                out.write(0)
                val min = maxOf(2, bits(palette.size))
                out.write(min)
                val data = lzw(image.indices, min)
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

        private fun lzw(indices: ByteArray, min: Int): ByteArray {
            val clear = 1 shl min
            val out = ByteArrayOutputStream()
            var buffer = 0L
            var count = 0
            var size = min + 1
            var next = clear + 2
            val table = HashMap<Int, Int>()
            fun emit(code: Int) {
                buffer = buffer or (code.toLong() shl count)
                count += size
                while (count >= 8) {
                    out.write((buffer and 0xFF).toInt())
                    buffer = buffer ushr 8
                    count -= 8
                }
            }
            emit(clear)
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
                if (next >= (1 shl size) && size < 12) size++
                if (next < 4096) {
                    table[key] = next++
                } else {
                    emit(clear)
                    table.clear()
                    size = min + 1
                    next = clear + 2
                }
                prefix = c
            }
            emit(prefix)
            if (next >= (1 shl size) && size < 12) size++
            emit(clear + 1)
            if (count > 0) out.write((buffer and 0xFF).toInt())
            return out.toByteArray()
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
    }

    /** An APNG writer around the platform's PNG encoder: full-canvas frames, one delay. */
    private object Apng {
        fun write(width: Int, height: Int, frames: List<Bitmap>, delayNum: Int, delayDen: Int): ByteArray {
            val encoded = frames.map { bitmap ->
                ByteArrayOutputStream().use {
                    bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)
                    chunks(it.toByteArray())
                }
            }
            val out = ByteArrayOutputStream()
            out.write(byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A))
            chunk(out, "IHDR", encoded.first().first { it.first == "IHDR" }.second)
            chunk(out, "acTL", ByteArray(8).also { be32(it, 0, frames.size) })
            var sequence = 0
            encoded.forEachIndexed { index, parts ->
                val fctl = ByteArray(26)
                be32(fctl, 0, sequence++)
                be32(fctl, 4, width)
                be32(fctl, 8, height)
                fctl[21] = delayNum.toByte()
                fctl[23] = delayDen.toByte()
                chunk(out, "fcTL", fctl)
                val data = parts.filter { it.first == "IDAT" }.fold(ByteArray(0)) { acc, c -> acc + c.second }
                if (index == 0) {
                    chunk(out, "IDAT", data)
                } else {
                    chunk(out, "fdAT", ByteArray(4).also { be32(it, 0, sequence++) } + data)
                }
            }
            chunk(out, "IEND", ByteArray(0))
            return out.toByteArray()
        }

        private fun chunks(png: ByteArray): List<Pair<String, ByteArray>> {
            val list = ArrayList<Pair<String, ByteArray>>()
            var at = 8
            while (at + 12 <= png.size) {
                val length = ByteBuffer.wrap(png, at, 4).int
                list += String(png, at + 4, 4, Charsets.US_ASCII) to png.copyOfRange(at + 8, at + 8 + length)
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
    }
}
