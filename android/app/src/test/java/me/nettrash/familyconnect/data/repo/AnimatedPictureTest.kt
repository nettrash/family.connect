/*
 * AnimatedPictureTest.kt
 * Family Connect (Android)
 *
 * The frames an animated GIF, WebP or PNG becomes, composited (#96;
 * docs/protocol.md, "Animations" → "The file"): what the encoder samples
 * and lays on white. Under Robolectric's NATIVE graphics, so BitmapFactory
 * is the real platform decoder — the codec AnimatedPicture hands each WebP
 * and APNG frame to, and the independent judge of the hand-written GIF
 * decoder's first frame.
 *
 *  - Kind: the protocol's tests for "animated" (a second GIF image, the VP8X
 *    animation flag, an acTL before IDAT), and the stills and BMPs that go
 *    as files.
 *  - The compositor: "over" and plain copy, the three disposals, clipping,
 *    and the white laid under transparency.
 *  - GIF: the LZW decoder against the platform's decoder (a full 4096-entry
 *    table with its clear codes included), interlacing, transparency, the
 *    Graphic Control Extension's delay and disposal, truncation.
 *  - APNG and WebP: frames cut out as stills, their offsets, delays,
 *    blending and disposal.
 */

package me.nettrash.familyconnect.data.repo

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import com.google.common.truth.Truth.assertThat
import com.google.common.truth.Truth.assertWithMessage
import me.nettrash.familyconnect.data.repo.AnimatedPicture.Disposal
import me.nettrash.familyconnect.data.repo.AnimatedPicture.Frame
import me.nettrash.familyconnect.testutil.AnimatedFixtures
import me.nettrash.familyconnect.testutil.AnimatedFixtures.GifImage
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.GraphicsMode
import kotlin.random.Random

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class AnimatedPictureTest {

    private val red = 0xFFFF0000.toInt()
    private val green = 0xFF00FF00.toInt()
    private val blue = 0xFF0000FF.toInt()
    private val white = 0xFFFFFFFF.toInt()
    private val clear = 0

    // -- Kind ---------------------------------------------------------------------

    @Test
    fun `a gif with a second image is animated and one with a single image is a still`() {
        val palette = intArrayOf(0xFF0000, 0x00FF00)
        val one = GifImage(0, 0, 2, 2, ByteArray(4))
        assertThat(AnimatedPicture.kindOf(AnimatedFixtures.gif(2, 2, palette, listOf(one))))
            .isEqualTo(AnimatedPicture.Kind.STILL_GIF)
        assertThat(AnimatedPicture.kindOf(AnimatedFixtures.gif(2, 2, palette, listOf(one, one))))
            .isEqualTo(AnimatedPicture.Kind.ANIMATED)
    }

    @Test
    fun `an apng is animated and a plain png is not this file's business`() {
        val bitmap = solid(4, 4, red, alpha = true)
        val apng = requireNotNull(AnimatedFixtures.apng(4, 4, listOf(AnimatedFixtures.ApngFrame(bitmap), AnimatedFixtures.ApngFrame(bitmap))))
        assertThat(AnimatedPicture.kindOf(apng)).isEqualTo(AnimatedPicture.Kind.ANIMATED)
        assertThat(AnimatedPicture.kindOf(pngOf(bitmap))).isEqualTo(AnimatedPicture.Kind.OTHER)
    }

    @Test
    fun `a bmp, a jpeg and noise are told apart`() {
        assertThat(AnimatedPicture.kindOf("BM....".toByteArray())).isEqualTo(AnimatedPicture.Kind.BMP)
        assertThat(AnimatedPicture.kindOf(byteArrayOf(0xFF.toByte(), 0xD8.toByte(), 0xFF.toByte(), 0xE0.toByte())))
            .isEqualTo(AnimatedPicture.Kind.OTHER)
        assertThat(AnimatedPicture.kindOf(ByteArray(0))).isEqualTo(AnimatedPicture.Kind.OTHER)
        assertThat(AnimatedPicture.mayMatter("GIF89a".toByteArray())).isTrue()
        assertThat(AnimatedPicture.mayMatter("%PDF-1.7".toByteArray())).isFalse()
    }

    @Test
    fun `a still webp is a still and the animation flag makes one animated`() {
        val still = ByteArray(30).also {
            "RIFF".toByteArray().copyInto(it, 0)
            "WEBP".toByteArray().copyInto(it, 8)
            "VP8X".toByteArray().copyInto(it, 12)
        }
        assertThat(AnimatedPicture.kindOf(still)).isEqualTo(AnimatedPicture.Kind.STILL_WEBP)
        still[20] = 0x02
        assertThat(AnimatedPicture.kindOf(still)).isEqualTo(AnimatedPicture.Kind.ANIMATED)
    }

    // -- The compositor -------------------------------------------------------------

    private fun frame(x: Int, y: Int, w: Int, h: Int, blend: Boolean = true, disposal: Disposal = Disposal.NONE) =
        Frame(x, y, w, h, statedMs = 100, blend = blend, disposal = disposal)

    @Test
    fun `a blended frame is drawn over the canvas and its transparent pixels leave it alone`() {
        val c = AnimatedPicture.Compositor(2, 1)
        c.render(frame(0, 0, 2, 1), intArrayOf(red, red))
        val canvas = c.render(frame(0, 0, 2, 1), intArrayOf(blue, clear))
        assertThat(canvas.toList()).containsExactly(blue, red).inOrder()
    }

    @Test
    fun `a frame that does not blend replaces what is there, alpha and all`() {
        val c = AnimatedPicture.Compositor(2, 1)
        c.render(frame(0, 0, 2, 1), intArrayOf(red, red))
        val canvas = c.render(frame(0, 0, 2, 1, blend = false), intArrayOf(blue, clear))
        assertThat(canvas.toList()).containsExactly(blue, clear).inOrder()
    }

    @Test
    fun `disposal to background clears the frame's area before the next one`() {
        val c = AnimatedPicture.Compositor(3, 1)
        c.render(frame(0, 0, 3, 1), intArrayOf(red, red, red))
        c.render(frame(1, 0, 1, 1, disposal = Disposal.BACKGROUND), intArrayOf(blue))
        val canvas = c.render(frame(0, 0, 1, 1), intArrayOf(green))
        assertThat(canvas.toList()).containsExactly(green, clear, red).inOrder()
    }

    @Test
    fun `disposal to previous restores what the frame covered`() {
        val c = AnimatedPicture.Compositor(3, 1)
        c.render(frame(0, 0, 3, 1), intArrayOf(red, red, red))
        assertThat(c.render(frame(1, 0, 2, 1, disposal = Disposal.PREVIOUS), intArrayOf(blue, blue)).toList())
            .containsExactly(red, blue, blue).inOrder()
        val canvas = c.render(frame(0, 0, 1, 1), intArrayOf(green))
        assertThat(canvas.toList()).containsExactly(green, red, red).inOrder()
    }

    @Test
    fun `a frame reaching past the canvas is clipped, not wrapped`() {
        val c = AnimatedPicture.Compositor(2, 2)
        val canvas = c.render(frame(1, 1, 2, 2), intArrayOf(red, blue, green, white))
        assertThat(canvas.toList()).containsExactly(clear, clear, clear, red).inOrder()
    }

    @Test
    fun `half-transparent over opaque blends and white shows through what stays clear`() {
        val half = 0x80000000.toInt() or 0x0000FF // half-alpha blue
        assertThat(AnimatedPicture.over(half, red)).isEqualTo(0xFF7F0080.toInt())
        assertThat(AnimatedPicture.over(half, clear)).isEqualTo(half)
        val flat = AnimatedPicture.flattenOnWhite(intArrayOf(clear, red, half))
        assertThat(flat[0]).isEqualTo(white)
        assertThat(flat[1]).isEqualTo(red)
        assertThat(flat[2]).isEqualTo(0xFF7F7FFF.toInt())
    }

    // -- GIF --------------------------------------------------------------------------

    @Test
    fun `the lzw decoder reads a full table and its clear codes as the platform decoder does`() {
        val random = Random(96)
        val palette = IntArray(256) { random.nextInt(0x1000000) }
        val width = 97
        val height = 61
        val indices = ByteArray(width * height) { random.nextInt(256).toByte() }
        val gif = AnimatedFixtures.gif(width, height, palette, listOf(GifImage(0, 0, width, height, indices)))

        val picture = requireNotNull(AnimatedPicture.open(gif))
        val ours = requireNotNull(picture.pixels(0))
        val expected = IntArray(indices.size) { 0xFF000000.toInt() or palette[indices[it].toInt() and 0xFF] }
        assertThat(ours.toList()).isEqualTo(expected.toList())
        assertThat(platformFirstFrame(gif).toList()).isEqualTo(expected.toList())
    }

    @Test
    fun `a small palette, an interlaced image and a transparent index decode exactly`() {
        val palette = intArrayOf(0xFF0000, 0x00FF00, 0x0000FF, 0xFFFFFF)
        val width = 7
        val height = 11
        val indices = ByteArray(width * height) { ((it * 7 + it / width) % 4).toByte() }
        val gif = AnimatedFixtures.gif(
            width, height, palette,
            listOf(GifImage(0, 0, width, height, indices, transparent = 3, interlaced = true)),
        )
        val ours = requireNotNull(AnimatedPicture.open(gif)?.pixels(0))
        val expected = IntArray(indices.size) {
            val index = indices[it].toInt()
            if (index == 3) 0 else 0xFF000000.toInt() or palette[index]
        }
        assertThat(ours.toList()).isEqualTo(expected.toList())
        assertThat(platformFirstFrame(gif).toList()).isEqualTo(expected.toList())
    }

    @Test
    fun `a gif's frames carry their delays, places and disposals`() {
        val palette = intArrayOf(0xFF0000, 0x0000FF)
        val gif = AnimatedFixtures.gif(
            4, 2, palette,
            listOf(
                GifImage(0, 0, 4, 2, ByteArray(8), delayCs = 0),
                GifImage(1, 0, 2, 2, ByteArray(4) { 1 }, delayCs = 7, disposal = 2),
                GifImage(0, 1, 1, 1, ByteArray(1) { 1 }, delayCs = 150, disposal = 3),
            ),
        )
        val picture = requireNotNull(AnimatedPicture.open(gif))
        assertThat(picture.width).isEqualTo(4)
        assertThat(picture.height).isEqualTo(2)
        assertThat(picture.frames.map { it.statedMs }).containsExactly(0L, 70L, 1500L).inOrder()
        assertThat(picture.frames.map { it.disposal })
            .containsExactly(Disposal.NONE, Disposal.BACKGROUND, Disposal.PREVIOUS).inOrder()
        assertThat(picture.frames[1].x).isEqualTo(1)
        assertThat(picture.frames.all { it.blend }).isTrue()

        // Composited: the blue square, then cleared to transparent, then the dot.
        val c = AnimatedPicture.Compositor(4, 2)
        val shown = picture.frames.indices.map { c.render(picture.frames[it], picture.pixels(it)).toList() }
        assertThat(shown[1]).containsExactly(red, blue, blue, red, red, blue, blue, red).inOrder()
        assertThat(shown[2]).containsExactly(red, clear, clear, red, blue, clear, clear, red).inOrder()
    }

    @Test
    fun `a truncated gif keeps what decoded and a broken one is nothing`() {
        val palette = intArrayOf(0xFF0000, 0x0000FF)
        val gif = AnimatedFixtures.gif(8, 8, palette, listOf(GifImage(0, 0, 8, 8, ByteArray(64) { (it % 2).toByte() })))
        val cut = gif.copyOf(gif.size - 6)
        val picture = requireNotNull(AnimatedPicture.open(cut))
        assertThat(picture.frames).hasSize(1)
        assertThat(picture.pixels(0)).isNotNull()

        assertThat(AnimatedPicture.open("GIF89a".toByteArray())).isNull()
        assertThat(AnimatedPicture.open(ByteArray(100) { 7 })).isNull()
    }

    @Test
    fun `the lzw decoder stops at a code that cannot be rather than reading past it`() {
        val out = ByteArray(16)
        // min code size 2: clear (4), then 7 — a code the table cannot hold yet.
        val written = AnimatedPicture.Gif.lzw(byteArrayOf(0x3C, 0x00), 2, out)
        assertThat(written).isEqualTo(0)
        assertThat(AnimatedPicture.Gif.lzw(byteArrayOf(1, 2, 3), 0, out)).isEqualTo(0)
    }

    @Test
    fun `interlaced rows land on their display rows`() {
        assertThat(AnimatedPicture.Gif.interlacedRows(10).toList())
            .containsExactly(0, 8, 4, 2, 6, 1, 3, 5, 7, 9).inOrder()
    }

    // -- APNG -------------------------------------------------------------------------

    @Test
    fun `an apng's frames are cut out, timed and composited by their fcTL`() {
        val base = solid(4, 4, red, alpha = true)
        val patch = solid(2, 2, blue, alpha = true).apply { setPixel(1, 1, 0x00000000) }
        val apng = AnimatedFixtures.apng(
            4, 4,
            listOf(
                AnimatedFixtures.ApngFrame(base, delayNum = 1, delayDen = 3),
                AnimatedFixtures.ApngFrame(patch, x = 1, y = 2, delayNum = 5, delayDen = 0, dispose = 2, blend = 1),
                AnimatedFixtures.ApngFrame(solid(1, 1, green, alpha = true), x = 0, y = 0, blend = 0),
            ),
        )
        assumeTrue("the platform encoder wrote frames in different colour types", apng != null)
        val picture = requireNotNull(AnimatedPicture.open(requireNotNull(apng)))
        assertThat(picture.width).isEqualTo(4)
        assertThat(picture.frames.map { it.statedMs }).containsExactly(333L, 50L, 100L).inOrder()
        assertThat(picture.frames[1].disposal).isEqualTo(Disposal.PREVIOUS)
        assertThat(picture.frames[1].blend).isTrue()
        assertThat(picture.frames[2].blend).isFalse()

        val c = AnimatedPicture.Compositor(4, 4)
        c.render(picture.frames[0], picture.pixels(0))
        val second = c.render(picture.frames[1], picture.pixels(1)).copyOf()
        assertThat(second[2 * 4 + 1]).isEqualTo(blue)
        assertThat(second[3 * 4 + 2]).isEqualTo(red) // the patch's clear pixel blends over red
        val third = c.render(picture.frames[2], picture.pixels(2))
        // The patch was disposed to what was there before it.
        assertThat(third[2 * 4 + 1]).isEqualTo(red)
        assertThat(third[0]).isEqualTo(green)
    }

    @Test
    fun `a default image without an fcTL before it is not part of the animation`() {
        val apng = AnimatedFixtures.apng(
            2, 2,
            listOf(
                AnimatedFixtures.ApngFrame(solid(2, 2, red, alpha = true)),
                AnimatedFixtures.ApngFrame(solid(2, 2, blue, alpha = true)),
            ),
            firstIsDefault = false,
        )
        assumeTrue(apng != null)
        val picture = requireNotNull(AnimatedPicture.open(requireNotNull(apng)))
        // Both frames are fdAT frames; the IDAT default image is not one of them.
        assertThat(picture.frames).hasSize(2)
        assertThat(picture.pixels(0)!!.first()).isEqualTo(red)
        assertThat(picture.pixels(1)!!.first()).isEqualTo(blue)
    }

    // -- WebP -------------------------------------------------------------------------

    @Test
    fun `an animated webp's frames are cut out, placed and timed by their ANMF`() {
        val webp = AnimatedFixtures.animatedWebP(
            6, 4,
            listOf(
                AnimatedFixtures.WebPFrame(solid(6, 4, red, alpha = true), durationMs = 70, blend = false),
                AnimatedFixtures.WebPFrame(solid(2, 2, blue, alpha = true), x = 2, y = 2, durationMs = 0, disposeToBackground = true),
                AnimatedFixtures.WebPFrame(solid(2, 2, green, alpha = true), x = 0, y = 0, durationMs = 2000),
            ),
        )
        assumeTrue("this Robolectric build writes no WebP", webp != null)
        val picture = requireNotNull(AnimatedPicture.open(requireNotNull(webp)))
        assertThat(picture.width).isEqualTo(6)
        assertThat(picture.height).isEqualTo(4)
        assertThat(picture.frames.map { it.statedMs }).containsExactly(70L, 0L, 2000L).inOrder()
        assertThat(picture.frames.map { it.blend }).containsExactly(false, true, true).inOrder()
        assertThat(picture.frames[1].x).isEqualTo(2)
        assertThat(picture.frames[1].disposal).isEqualTo(Disposal.BACKGROUND)

        val c = AnimatedPicture.Compositor(6, 4)
        c.render(picture.frames[0], picture.pixels(0))
        assertThat(c.render(picture.frames[1], picture.pixels(1))[2 * 6 + 2]).isEqualTo(blue)
        val third = c.render(picture.frames[2], picture.pixels(2))
        assertThat(third[2 * 6 + 2]).isEqualTo(clear) // disposed to background
        assertThat(third[0]).isEqualTo(green)
        assertThat(third[5]).isEqualTo(red)
    }

    @Test
    fun `a lossy webp frame with an alpha plane decodes through a VP8X still`() {
        val webp = AnimatedFixtures.animatedWebP(
            4, 4,
            listOf(
                AnimatedFixtures.WebPFrame(solid(4, 4, red, alpha = true).apply { setPixel(0, 0, 0) }, lossless = false),
                AnimatedFixtures.WebPFrame(solid(4, 4, blue, alpha = true), lossless = false),
            ),
        )
        assumeTrue(webp != null)
        val picture = requireNotNull(AnimatedPicture.open(requireNotNull(webp)))
        val pixels = requireNotNull(picture.pixels(0))
        assertThat(pixels[0] ushr 24).isLessThan(0x40)
        assertWithMessage("a lossy red stays red").that((pixels[5] shr 16) and 0xFF).isGreaterThan(0xC0)
    }

    // -- Helpers ----------------------------------------------------------------------

    private fun solid(width: Int, height: Int, colour: Int, alpha: Boolean): Bitmap =
        Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888).apply {
            eraseColor(colour)
            // Every frame with an alpha channel, so the PNG encoder writes them all as RGBA.
            setHasAlpha(alpha)
        }

    private fun pngOf(bitmap: Bitmap): ByteArray = java.io.ByteArrayOutputStream().use {
        bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)
        it.toByteArray()
    }

    private fun platformFirstFrame(gif: ByteArray): IntArray {
        val options = BitmapFactory.Options().apply {
            inPreferredConfig = Bitmap.Config.ARGB_8888
            inPremultiplied = false
        }
        val bitmap = requireNotNull(BitmapFactory.decodeByteArray(gif, 0, gif.size, options)) {
            "the platform decoder could not read the GIF"
        }
        return IntArray(bitmap.width * bitmap.height).also {
            bitmap.getPixels(it, 0, bitmap.width, 0, 0, bitmap.width, bitmap.height)
        }
    }
}
