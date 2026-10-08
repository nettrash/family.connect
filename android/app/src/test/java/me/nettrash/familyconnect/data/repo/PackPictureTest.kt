/*
 * PackPictureTest.kt
 * Family Connect (Android)
 *
 * What a picture has to be to join the family's sticker pack, decided from
 * its bytes (docs/protocol.md, "What a sticker is made of"): the magic
 * numbers, whether it is animated, its size in pixels, the 512 x 512 box,
 * and — the rule everything else serves — when a picture is left ALONE.
 *
 * The headers are written out by hand on purpose. Robolectric's
 * BitmapFactory answers for any non-empty array, so a test that asked the
 * platform would prove nothing; these are the bytes the formats specify.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class PackPictureTest {

    private fun bytes(vararg values: Int) = ByteArray(values.size) { values[it].toByte() }

    private fun ascii(text: String) = text.toByteArray(Charsets.US_ASCII)

    private fun be32(value: Int) = bytes(value ushr 24, value ushr 16, value ushr 8, value)

    private fun le24(value: Int) = bytes(value, value ushr 8, value ushr 16)

    private fun pngChunk(type: String, data: ByteArray) =
        be32(data.size) + ascii(type) + data + ByteArray(4)

    private val pngSignature = bytes(0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A)

    private fun png(width: Int, height: Int, animated: Boolean = false): ByteArray {
        val header = pngChunk("IHDR", be32(width) + be32(height) + bytes(8, 6, 0, 0, 0))
        val control = if (animated) pngChunk("acTL", be32(3) + be32(0)) else ByteArray(0)
        return pngSignature + header + control + pngChunk("IDAT", ByteArray(16)) + pngChunk("IEND", ByteArray(0))
    }

    private fun riff(payload: ByteArray) = ascii("RIFF") + ByteArray(4) + ascii("WEBP") + payload

    /** An extended WebP: the flags byte, three reserved, then the canvas size minus one. */
    private fun webpExtended(width: Int, height: Int, animated: Boolean) = riff(
        ascii("VP8X") + bytes(10, 0, 0, 0) + bytes(if (animated) 0x12 else 0x10, 0, 0, 0) +
            le24(width - 1) + le24(height - 1),
    )

    private fun webpLossy(width: Int, height: Int) = riff(
        ascii("VP8 ") + ByteArray(4) + bytes(0, 0, 0) + bytes(0x9D, 0x01, 0x2A) +
            bytes(width, width ushr 8, height, height ushr 8),
    )

    private fun webpLossless(width: Int, height: Int): ByteArray {
        val bits = (width - 1) or ((height - 1) shl 14)
        return riff(
            ascii("VP8L") + ByteArray(4) + bytes(0x2F) + bytes(bits, bits ushr 8, bits ushr 16, bits ushr 24),
        )
    }

    private val jpeg = bytes(0xFF, 0xD8, 0xFF, 0xE0) + ByteArray(40)

    /**
     * A GIF of [frames] images, block by block as GIF89a lays them out: the
     * header, the logical screen descriptor with a two-colour global table,
     * then per image a graphic-control extension, the image descriptor, the
     * LZW code size and one data sub-block — and the trailer.
     */
    private fun gif(frames: Int, localTables: Boolean = false): ByteArray {
        var out = ascii("GIF89a") + bytes(8, 0, 8, 0, 0x80, 0, 0) + ByteArray(6)
        repeat(frames) {
            out += bytes(0x21, 0xF9, 0x04, 0, 10, 0, 0, 0x00)
            out += bytes(0x2C, 0, 0, 0, 0, 8, 0, 8, 0, if (localTables) 0x80 else 0x00)
            if (localTables) out += ByteArray(6)
            out += bytes(0x02, 0x02, 0x4C, 0x01, 0x00)
        }
        return out + bytes(0x3B)
    }

    /** A PNG whose chunk after IHDR declares [length] bytes of data and carries sixteen. */
    private fun pngWithDeclaredLength(length: Int): ByteArray {
        val header = pngChunk("IHDR", be32(64) + be32(64) + bytes(8, 6, 0, 0, 0))
        return pngSignature + header + be32(length) + ascii("tEXt") + ByteArray(16) + ByteArray(4) +
            pngChunk("acTL", be32(3) + be32(0)) + pngChunk("IDAT", ByteArray(16))
    }

    // -- What the bytes are --------------------------------------------------

    @Test
    fun `a WebP is RIFF at 0 and WEBP at 8, whatever the length between says`() {
        assertThat(PackPicture.sniff(webpLossy(64, 64))).isEqualTo("image/webp")
        val oddLength = webpLossy(64, 64).also { it[4] = 0x7F; it[7] = 0x7F }
        assertThat(PackPicture.sniff(oddLength)).isEqualTo("image/webp")
        // RIFF alone is not enough: a WAV is RIFF too.
        assertThat(PackPicture.sniff(ascii("RIFF") + ByteArray(4) + ascii("WAVE") + ByteArray(8))).isNull()
    }

    // -- Files that lie ---------------------------------------------------------

    @Test
    fun `a chunk length the file cannot hold ends the walk instead of the process`() {
        // Near Int.MAX_VALUE: added to the offset it WRAPS NEGATIVE, which
        // passes a bound written as `offset + 8 <= size` and then reads
        // before the start of the array. A picked file is somebody else's
        // numbers.
        for (length in listOf(0x7FFFFFF0, Int.MAX_VALUE, 0x7FFFFFFF - 11, -1, Int.MIN_VALUE, 1 shl 30, 17)) {
            val hostile = pngWithDeclaredLength(length)
            assertThat(PackPicture.isAnimated(hostile)).isFalse()
            // The whole road a picked picture takes, which runs on the app
            // scope with nothing to catch a throw: within the ceiling it is
            // taken as given, and nothing on the way reads out of bounds.
            val outcome = PackPicture.prepare(hostile, maxItemBytes = 524_288)
            assertThat(outcome).isInstanceOf(PackPicture.Outcome.Ready::class.java)
        }
        // The honest length is still walked past, to the acTL behind it.
        assertThat(PackPicture.isAnimated(pngWithDeclaredLength(16))).isTrue()
    }

    @Test
    fun `a file cut short anywhere is judged without reading past its end`() {
        // Every prefix of every shape, through every header reader: a
        // download that stopped, or a file made to stop there.
        val shapes = listOf(
            png(64, 64, animated = true),
            png(64, 64),
            webpExtended(512, 512, animated = true),
            webpLossy(64, 64),
            webpLossless(64, 64),
        )
        for (shape in shapes) {
            for (cut in 0..shape.size) {
                val prefix = shape.copyOf(cut)
                PackPicture.sniff(prefix)
                PackPicture.isAnimated(prefix)
                PackPicture.dimensions(prefix)
            }
        }
    }

    @Test
    fun `a PNG is its eight-byte signature and a JPEG is neither`() {
        assertThat(PackPicture.sniff(png(10, 10))).isEqualTo("image/png")
        assertThat(PackPicture.sniff(jpeg)).isNull()
        assertThat(PackPicture.sniff(ByteArray(0))).isNull()
        assertThat(PackPicture.sniff(bytes(0x89, 0x50))).isNull()
    }

    @Test
    fun `only WebP and PNG may be a sticker`() {
        assertThat(PackPicture.isStickerType("image/webp")).isTrue()
        assertThat(PackPicture.isStickerType("image/png")).isTrue()
        assertThat(PackPicture.isStickerType("image/jpeg")).isFalse()
        assertThat(PackPicture.isStickerType("image/gif")).isFalse()
        assertThat(PackPicture.isStickerType(null)).isFalse()
    }

    @Test
    fun `animation is read from the file's own header`() {
        assertThat(PackPicture.isAnimated(webpExtended(512, 512, animated = true))).isTrue()
        assertThat(PackPicture.isAnimated(webpExtended(512, 512, animated = false))).isFalse()
        assertThat(PackPicture.isAnimated(webpLossy(64, 64))).isFalse()
        assertThat(PackPicture.isAnimated(png(64, 64, animated = true))).isTrue()
        assertThat(PackPicture.isAnimated(png(64, 64))).isFalse()
        assertThat(PackPicture.isAnimated(jpeg)).isFalse()
    }

    @Test
    fun `a GIF is animated when it holds a second image`() {
        assertThat(PackPicture.isAnimated(gif(frames = 1))).isFalse()
        assertThat(PackPicture.isAnimated(gif(frames = 2))).isTrue()
        assertThat(PackPicture.isAnimated(gif(frames = 5, localTables = true))).isTrue()
        assertThat(PackPicture.isAnimated(gif(frames = 1, localTables = true))).isFalse()
        // And it is still not a type a pack may hold.
        assertThat(PackPicture.sniff(gif(frames = 2))).isNull()
    }

    @Test
    fun `a GIF cut short anywhere is judged without reading past its end`() {
        val whole = gif(frames = 3, localTables = true)
        // Every prefix, as for the other two formats: the block lengths are
        // numbers somebody else wrote.
        for (length in 0..whole.size) {
            PackPicture.isAnimated(whole.copyOf(length))
        }
        // A sub-block that claims more than the file holds ends the walk.
        val lying = gif(frames = 1).let { it.copyOf(it.size - 5) + bytes(0xFF, 0x01) }
        assertThat(PackPicture.isAnimated(lying)).isFalse()
    }

    @Test
    fun `pixel size is read from each header shape`() {
        assertThat(PackPicture.dimensions(png(512, 384))).isEqualTo(512 to 384)
        assertThat(PackPicture.dimensions(webpExtended(2000, 1500, animated = true))).isEqualTo(2000 to 1500)
        assertThat(PackPicture.dimensions(webpLossy(300, 200))).isEqualTo(300 to 200)
        assertThat(PackPicture.dimensions(webpLossless(96, 128))).isEqualTo(96 to 128)
        // Unknown costs the upload its width and height and nothing else.
        assertThat(PackPicture.dimensions(jpeg)).isNull()
        assertThat(PackPicture.dimensions(riff(ascii("VP8 ") + ByteArray(20)))).isNull()
    }

    // -- The box -------------------------------------------------------------

    @Test
    fun `a larger picture is fitted whole, keeping its proportions`() {
        assertThat(PackPicture.fit(2048, 1024)).isEqualTo(512 to 256)
        assertThat(PackPicture.fit(1000, 4000)).isEqualTo(128 to 512)
        assertThat(PackPicture.fit(1024, 1024)).isEqualTo(512 to 512)
    }

    @Test
    fun `a smaller picture is never enlarged`() {
        assertThat(PackPicture.fit(96, 96)).isEqualTo(96 to 96)
        assertThat(PackPicture.fit(512, 200)).isEqualTo(512 to 200)
    }

    @Test
    fun `a sliver keeps at least one pixel`() {
        assertThat(PackPicture.fit(10_000, 4)).isEqualTo(512 to 1)
        assertThat(PackPicture.fit(0, 10)).isEqualTo(0 to 0)
    }

    // -- What happens to a picked picture --------------------------------------

    private val limit = 512L * 1024

    @Test
    fun `a finished sticker within the ceiling is taken as given, whatever its pixels`() {
        assertThat(PackPicture.plan("image/webp", 40_000, animated = false, maxItemBytes = limit))
            .isEqualTo(PackPicture.Plan.AS_GIVEN)
        assertThat(PackPicture.plan("image/png", limit, animated = false, maxItemBytes = limit))
            .isEqualTo(PackPicture.Plan.AS_GIVEN)
        // Animated, and exactly as it is: nobody re-encodes one.
        assertThat(PackPicture.plan("image/webp", 300_000, animated = true, maxItemBytes = limit))
            .isEqualTo(PackPicture.Plan.AS_GIVEN)
    }

    @Test
    fun `an animated sticker over the ceiling is refused, never re-encoded`() {
        assertThat(PackPicture.plan("image/webp", limit + 1, animated = true, maxItemBytes = limit))
            .isEqualTo(PackPicture.Plan.TOO_LARGE)
    }

    @Test
    fun `an animated picture that is not a WebP is refused in words, never flattened`() {
        // An animated GIF: not a type a pack may hold, and re-making it
        // would keep one frame of it.
        assertThat(PackPicture.plan(null, 40_000, animated = true, maxItemBytes = limit))
            .isEqualTo(PackPicture.Plan.ANIMATED_NOT_WEBP)
        // An animated PNG over the ceiling: the still path would flatten it.
        assertThat(PackPicture.plan("image/png", limit + 1, animated = true, maxItemBytes = limit))
            .isEqualTo(PackPicture.Plan.ANIMATED_NOT_WEBP)
        // Within the ceiling a PNG goes up byte for byte, animated or not:
        // nothing is flattened because nothing is touched.
        assertThat(PackPicture.plan("image/png", 40_000, animated = true, maxItemBytes = limit))
            .isEqualTo(PackPicture.Plan.AS_GIVEN)
    }

    @Test
    fun `an animated GIF picked as a sticker is refused and a decoder is never asked`() {
        // No Robolectric here: had this reached the refit, the decoder it
        // calls would not exist. The refusal is made from the bytes alone.
        assertThat(PackPicture.prepare(gif(frames = 2), maxItemBytes = limit))
            .isEqualTo(PackPicture.Outcome.AnimatedNotWebp)
        assertThat(PackPicture.prepare(png(64, 64, animated = true), maxItemBytes = 10))
            .isEqualTo(PackPicture.Outcome.AnimatedNotWebp)
    }

    // -- The label -----------------------------------------------------------------

    @Test
    fun `a label is counted as the server counts it`() {
        assertThat(PackPicture.labelFits(null)).isTrue()
        assertThat(PackPicture.labelFits("")).isTrue()
        assertThat(PackPicture.labelFits("a".repeat(64))).isTrue()
        assertThat(PackPicture.labelFits("a".repeat(65))).isFalse()
        // Unicode scalar values, not UTF-16 units: sixty-four emoji are
        // sixty-four characters and a hundred and twenty-eight units.
        val emoji = "\uD83D\uDE3A"
        assertThat(emoji.repeat(64).length).isEqualTo(128)
        assertThat(PackPicture.labelFits(emoji.repeat(64))).isTrue()
        assertThat(PackPicture.labelFits(emoji.repeat(65))).isFalse()
        // AFTER trimming: the spaces around it are not the label.
        assertThat(PackPicture.labelFits("   " + "a".repeat(64) + "  ")).isTrue()
        assertThat(PackPicture.labelLength("  party cat ")).isEqualTo(9)
    }

    @Test
    fun `anything else is made into a sticker`() {
        // Not a type a pack may hold.
        assertThat(PackPicture.plan(null, 40_000, animated = false, maxItemBytes = limit))
            .isEqualTo(PackPicture.Plan.REFIT)
        // A still sticker too big to add as it is.
        assertThat(PackPicture.plan("image/png", limit + 1, animated = false, maxItemBytes = limit))
            .isEqualTo(PackPicture.Plan.REFIT)
    }

    @Test
    fun `a sticker taken as given keeps its very bytes`() {
        val source = webpExtended(2000, 1500, animated = true)

        val outcome = PackPicture.prepare(source, maxItemBytes = limit)

        val ready = outcome as PackPicture.Outcome.Ready
        // The SAME array: not a byte of an animated sticker is touched.
        assertThat(ready.picture.bytes).isSameInstanceAs(source)
        assertThat(ready.picture.mime).isEqualTo("image/webp")
        assertThat(ready.picture.width).isEqualTo(2000)
        assertThat(ready.picture.height).isEqualTo(1500)
    }

    @Test
    fun `an oversize animated sticker and an empty pick say why`() {
        val animated = webpExtended(512, 512, animated = true)

        assertThat(PackPicture.prepare(animated, maxItemBytes = 10)).isEqualTo(PackPicture.Outcome.TooLarge)
        assertThat(PackPicture.prepare(ByteArray(0), maxItemBytes = limit))
            .isEqualTo(PackPicture.Outcome.Unreadable)
    }
}
