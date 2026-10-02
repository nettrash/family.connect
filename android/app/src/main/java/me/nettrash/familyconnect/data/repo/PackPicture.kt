/*
 * PackPicture.kt
 * Family Connect (Android)
 *
 * What a picture has to be before it can join the family's sticker pack
 * (docs/protocol.md, "Sticker pack" → "What a sticker is made of"), and the
 * one thing this client may do to make it so.
 *
 * THE RULES, in the protocol's words and order:
 *
 *  - A pack item is `image/webp` or `image/png`, and nothing else.
 *  - A `.webp` or `.png` that is already a sticker is taken AS GIVEN,
 *    whatever its pixel size, so long as it is within the byte ceiling:
 *    re-encoding somebody's finished sticker buys nothing and, for an
 *    animated one, is not possible.
 *  - A client that MAKES an item out of a larger still picture scales it to
 *    fit 512 x 512 — whole, never cropped, proportions and transparency
 *    kept — and writes PNG, or WebP where its platform can.
 *  - No client re-encodes an animated sticker. So an ANIMATED picture that
 *    is not already a sticker this client can take as given — an animated
 *    GIF, say — is REFUSED, with a sentence saying animated stickers must be
 *    WebP. Never flattened to its first frame without a word: somebody who
 *    picked a moving picture and got a still one was told nothing.
 *
 * THIS IS NOT MediaPrep, AND MUST NEVER GO THROUGH IT. `preparePhoto`
 * scales to 2048 px and writes JPEG, which has no transparency and one
 * frame — exactly the two things that make a sticker one. Nothing here
 * calls it, and the send path for a sticker (PackRepository.stagedCopy →
 * MessageRepository.sendMedia) hands over the ORIGINAL bytes with no
 * preview, so a sticker's upload hashes to the pack item's file.
 *
 * The decisions are plain functions over bytes and numbers so the JVM tests
 * can pin them without an image decoder: Robolectric's BitmapFactory
 * answers for any non-empty array, so the header parsing below is
 * hand-rolled rather than asked of the platform.
 *
 * iOS counterpart: the same rules on the Apple clients' pack add path.
 */

package me.nettrash.familyconnect.data.repo

import android.graphics.Bitmap
import android.graphics.ImageDecoder
import android.os.Build
import androidx.core.graphics.scale
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer

object PackPicture {

    const val MIME_WEBP = "image/webp"
    const val MIME_PNG = "image/png"

    /** The box a MADE sticker is fitted into. A client rule; the server never decodes. */
    const val EDGE = 512

    /** The label's ceiling, fixed by the protocol ("Limits"). */
    const val MAX_LABEL_CHARS = 64

    /**
     * The number of characters the server would count in a label: Unicode
     * scalar values of the TRIMMED text, as it counts a message's body —
     * not UTF-16 units, which would refuse thirty-two emoji as sixty-four
     * letters too many.
     */
    fun labelLength(label: CharSequence): Int = MessageBody.length(label)

    /** Whether the server would take [label]. None at all always fits. */
    fun labelFits(label: CharSequence?): Boolean =
        label == null || labelLength(label) <= MAX_LABEL_CHARS

    /** Whether a media type may be a pack item or a sticker message at all. */
    fun isStickerType(mime: String?): Boolean = mime == MIME_WEBP || mime == MIME_PNG

    /** The bytes a pack item is made of, ready to upload exactly as they are. */
    class Encoded(
        val bytes: ByteArray,
        val mime: String,
        /** Pixel size when it could be read; sent with the upload like any photo's. */
        val width: Int?,
        val height: Int?,
    )

    /** What became of a picked picture. */
    sealed interface Outcome {
        data class Ready(val picture: Encoded) : Outcome

        /**
         * Over the family's per-item ceiling and not something this client
         * may shrink — an animated sticker, or a still one that stayed too
         * big even fitted into the box. Said beside the picker.
         */
        data object TooLarge : Outcome

        /**
         * A moving picture this client would have to re-encode, which no
         * client can do without losing the movement: an animated GIF, or an
         * animated PNG over the ceiling. Refused, and said — "animated
         * stickers must be WebP" — rather than flattened to one frame.
         */
        data object AnimatedNotWebp : Outcome

        /** Not an image this device can read. */
        data object Unreadable : Outcome
    }

    /**
     * What the bytes ARE, by their magic number — never by the name or the
     * type a provider claimed. The server checks the same bytes: `RIFF` at
     * offset 0 and `WEBP` at offset 8 (the four between are the file's
     * length and are not checked), or PNG's eight-byte signature.
     */
    fun sniff(bytes: ByteArray): String? = when {
        bytes.size >= 12 &&
            bytes.matches(0, 'R', 'I', 'F', 'F') &&
            bytes.matches(8, 'W', 'E', 'B', 'P') -> MIME_WEBP
        bytes.size >= 8 &&
            (bytes[0].toInt() and 0xFF) == 0x89 &&
            bytes.matches(1, 'P', 'N', 'G') &&
            bytes[4].toInt() == 0x0D && bytes[5].toInt() == 0x0A &&
            bytes[6].toInt() == 0x1A && bytes[7].toInt() == 0x0A -> MIME_PNG
        else -> null
    }

    /**
     * Whether the file carries more than one frame.
     *
     * WebP says so in the VP8X chunk's flags (the ANIM bit, 0x02). An
     * animated PNG says so with an `acTL` chunk, which the APNG
     * specification requires BEFORE the first `IDAT` — so the scan stops
     * there and never walks a megabyte of pixel data. A GIF says so by
     * holding a second image ([gifHasSecondFrame]).
     *
     * Wrong in the safe direction when the file is malformed: "not
     * animated" only ever lets a still re-encode be attempted, and that
     * then fails or succeeds on its own.
     */
    fun isAnimated(bytes: ByteArray): Boolean = when (sniff(bytes)) {
        MIME_WEBP ->
            bytes.size >= 21 && bytes.matches(12, 'V', 'P', '8', 'X') &&
                (bytes[20].toInt() and 0x02) != 0
        MIME_PNG -> pngHasChunkBeforeData(bytes, "acTL")
        else -> gifHasSecondFrame(bytes)
    }

    /**
     * Whether the bytes are a GIF holding more than one image.
     *
     * A GIF has no "animated" flag: it is animated when a second Image
     * Descriptor (0x2C) follows the first. So the blocks are walked — by
     * their own length bytes, never decoded — until a second image, the
     * trailer, or the end of what was given. Every offset is checked
     * against the array before it is read: the lengths are numbers somebody
     * else wrote.
     */
    private fun gifHasSecondFrame(bytes: ByteArray): Boolean {
        if (bytes.size < 13 || !bytes.matches(0, 'G', 'I', 'F', '8') ||
            (bytes[4].toInt() != '7'.code && bytes[4].toInt() != '9'.code) ||
            bytes[5].toInt() != 'a'.code
        ) {
            return false
        }
        // Header (6), logical screen descriptor (7), then the global colour
        // table when the packed byte's top bit says there is one.
        var offset = 13 + colourTableBytes(bytes[10].toInt())
        var images = 0
        while (offset < bytes.size) {
            when (bytes[offset].toInt() and 0xFF) {
                0x2C -> {
                    images += 1
                    if (images > 1) return true
                    // Separator (1), position and size (8), packed (1).
                    if (offset + 10 > bytes.size) return false
                    // The local colour table, then the LZW code size (1).
                    offset += 10 + colourTableBytes(bytes[offset + 9].toInt()) + 1
                    offset = skipGifSubBlocks(bytes, offset)
                }
                // An extension: introducer (1), label (1), then sub-blocks.
                0x21 -> offset = skipGifSubBlocks(bytes, offset + 2)
                // The trailer (0x3B), or something that is not a GIF block.
                else -> return false
            }
            if (offset < 0) return false
        }
        return false
    }

    /** The bytes a colour table takes, from the packed byte that announces it. */
    private fun colourTableBytes(packed: Int): Int =
        if ((packed and 0x80) != 0) 3 * (1 shl ((packed and 0x07) + 1)) else 0

    /** The offset just past a run of GIF data sub-blocks, or -1 when the file ends inside it. */
    private fun skipGifSubBlocks(bytes: ByteArray, start: Int): Int {
        var offset = start
        while (offset < bytes.size) {
            val length = bytes[offset].toInt() and 0xFF
            offset += 1
            // A zero length ends the run.
            if (length == 0) return offset
            offset += length
        }
        return -1
    }

    /**
     * The picture's own pixel size, read from its header — or null when the
     * header is not one of the shapes below, which costs the upload its
     * `width`/`height` and nothing else (they are optional on the wire).
     */
    fun dimensions(bytes: ByteArray): Pair<Int, Int>? = when (sniff(bytes)) {
        MIME_PNG -> pngDimensions(bytes)
        MIME_WEBP -> webpDimensions(bytes)
        else -> null
    }

    private fun pngDimensions(bytes: ByteArray): Pair<Int, Int>? {
        // Signature (8), chunk length (4), "IHDR" (4), then width and height
        // as big-endian 32-bit integers.
        if (bytes.size < 24 || !bytes.matches(12, 'I', 'H', 'D', 'R')) return null
        val width = bytes.be32(16)
        val height = bytes.be32(20)
        return positive(width, height)
    }

    private fun webpDimensions(bytes: ByteArray): Pair<Int, Int>? {
        // Each shape needs a different number of bytes, so each checks its
        // own: a lossless header is complete at 25, the other two at 30.
        return when {
            // Extended: canvas width-1 and height-1, 24 bits each, little-endian.
            bytes.matches(12, 'V', 'P', '8', 'X') -> {
                if (bytes.size < 30) return null
                positive(bytes.le24(24) + 1, bytes.le24(27) + 1)
            }
            // Lossless: one signature byte (0x2F), then 14 bits of width-1
            // and 14 bits of height-1 packed little-endian.
            bytes.matches(12, 'V', 'P', '8', 'L') -> {
                if (bytes.size < 25 || (bytes[20].toInt() and 0xFF) != 0x2F) return null
                val bits = (bytes[21].toInt() and 0xFF) or
                    ((bytes[22].toInt() and 0xFF) shl 8) or
                    ((bytes[23].toInt() and 0xFF) shl 16) or
                    ((bytes[24].toInt() and 0xFF) shl 24)
                positive((bits and 0x3FFF) + 1, ((bits shr 14) and 0x3FFF) + 1)
            }
            // Lossy: a three-byte frame tag, the start code 9D 01 2A, then
            // 14 bits of width and 14 of height (the top two are scaling).
            bytes.matches(12, 'V', 'P', '8', ' ') -> {
                if (bytes.size < 30 ||
                    (bytes[23].toInt() and 0xFF) != 0x9D ||
                    (bytes[24].toInt() and 0xFF) != 0x01 ||
                    (bytes[25].toInt() and 0xFF) != 0x2A
                ) {
                    return null
                }
                positive(bytes.le16(26) and 0x3FFF, bytes.le16(28) and 0x3FFF)
            }
            else -> null
        }
    }

    private fun positive(width: Int, height: Int): Pair<Int, Int>? =
        if (width > 0 && height > 0) width to height else null

    private fun pngHasChunkBeforeData(bytes: ByteArray, name: String): Boolean {
        var offset = 8
        while (offset + 8 <= bytes.size) {
            val length = bytes.be32(offset)
            val type = String(bytes, offset + 4, 4, Charsets.US_ASCII)
            if (type == name) return true
            if (type == "IDAT" || type == "IEND") return false
            // A length the file cannot hold ends the walk — and it is
            // compared BEFORE it is added, against what is left: a declared
            // 0x7FFFFFF0 added to the offset wraps negative, passes the
            // loop's own bound and reads before the start of the array.
            // The length is a number somebody else wrote; a picked file
            // must never be able to take the process down with it.
            if (length < 0 || length > bytes.size - offset - 12) return false
            // Length, type, data, CRC.
            offset += 12 + length
        }
        return false
    }

    /**
     * The size a [width] x [height] picture becomes inside the box: WHOLE,
     * never cropped, proportions kept — and never enlarged, because a
     * 96-pixel sticker scaled up to 512 is the same sticker, blurrier and
     * five times the bytes.
     */
    fun fit(width: Int, height: Int, edge: Int = EDGE): Pair<Int, Int> {
        if (width <= 0 || height <= 0) return 0 to 0
        if (width <= edge && height <= edge) return width to height
        val scale = minOf(edge.toDouble() / width, edge.toDouble() / height)
        return maxOf(1, Math.round(width * scale).toInt()) to
            maxOf(1, Math.round(height * scale).toInt())
    }

    /** What to do with a picked picture, decided from facts and no decoder. */
    enum class Plan {
        /** Already a sticker: upload the bytes exactly as they are. */
        AS_GIVEN,

        /** Make one: decode, fit into the box, write PNG (or WebP). */
        REFIT,

        /** Over the ceiling and not shrinkable. */
        TOO_LARGE,

        /** Animated, and not a sticker that can go up as it is: refused, in words. */
        ANIMATED_NOT_WEBP,
    }

    /**
     * The decision itself.
     *
     * A WebP or PNG within the ceiling is a finished sticker and is left
     * alone — including a 2000-pixel one, which the protocol says to take
     * as given. One OVER the ceiling cannot be added as it is, so a STILL
     * one is made into a sticker the way any other picture is, which is the
     * helpful reading of "too big"; an ANIMATED one cannot be, because
     * re-encoding it would lose the animation, so that is refused — an
     * animated WebP as too big, which is all that is wrong with it, and
     * anything else that moves (an animated GIF, an animated PNG over the
     * ceiling) as not being a WebP, which is what its adder can act on.
     * Every other STILL picture (a JPEG, a HEIC) is always re-made: its
     * type is not one a pack may hold.
     */
    fun plan(mime: String?, sizeBytes: Long, animated: Boolean, maxItemBytes: Long): Plan = when {
        isStickerType(mime) && sizeBytes <= maxItemBytes -> Plan.AS_GIVEN
        animated && mime == MIME_WEBP -> Plan.TOO_LARGE
        animated -> Plan.ANIMATED_NOT_WEBP
        else -> Plan.REFIT
    }

    /**
     * Turn picked bytes into a pack item's bytes, or say why not.
     *
     * [maxItemBytes] is the family's ceiling from `GET /families/mine`; the
     * refusal is made HERE, where the person is choosing, so it arrives as
     * "that one is too big" beside the picker rather than as a rejected
     * request.
     */
    fun prepare(source: ByteArray, maxItemBytes: Long): Outcome {
        if (source.isEmpty()) return Outcome.Unreadable
        val mime = sniff(source)
        return when (plan(mime, source.size.toLong(), isAnimated(source), maxItemBytes)) {
            Plan.AS_GIVEN -> {
                val size = dimensions(source)
                // `mime` is non-null here: AS_GIVEN is only planned for a
                // sticker type.
                Outcome.Ready(Encoded(source, mime ?: MIME_PNG, size?.first, size?.second))
            }
            Plan.TOO_LARGE -> Outcome.TooLarge
            Plan.ANIMATED_NOT_WEBP -> Outcome.AnimatedNotWebp
            // The header walk above knows WebP, PNG and GIF. Any OTHER
            // container that moves (an animated HEIF or AVIF) is one only
            // the platform can recognise, so it is asked before a decode
            // that would keep frame zero and say nothing.
            Plan.REFIT ->
                if (platformSaysAnimated(source)) Outcome.AnimatedNotWebp else refit(source, maxItemBytes)
        }
    }

    /**
     * Whether the PLATFORM's decoder reads more than one frame in [source].
     * Header only in effect: the decode it has to ask for is aimed at one
     * pixel. Any failure is "no" — the refit that follows then succeeds or
     * fails on its own, exactly as before this was asked.
     */
    private fun platformSaysAnimated(source: ByteArray): Boolean = runCatching {
        // ImageDecoder arrived in API 28. Before it nothing on the platform
        // reads those containers as animated either — or at all.
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.P) return false
        var animated = false
        ImageDecoder.decodeDrawable(ImageDecoder.createSource(ByteBuffer.wrap(source))) { decoder, info, _ ->
            animated = info.isAnimated
            decoder.setTargetSize(1, 1)
        }
        animated
    }.getOrDefault(false)

    /**
     * Make a sticker out of a still picture: decoded bounded, turned the
     * way its EXIF says, fitted whole into the box with its alpha kept, and
     * written as PNG — or as WebP when the PNG is over the ceiling, which a
     * photograph at 512 px usually is.
     *
     * Runs a decoder, so it is the one function here a plain-JVM test
     * cannot judge; everything it DECIDES with is above.
     */
    private fun refit(source: ByteArray, maxItemBytes: Long): Outcome = runCatching {
        // Decoded at up to twice the box, so the scale below still has
        // detail to work with — the same margin the avatar path takes.
        val decoded = AvatarImage.decode(source, maxPixels = EDGE * 2) ?: return Outcome.Unreadable
        val oriented = AvatarImage.orientedByExif(decoded, source)
        val (width, height) = fit(oriented.width, oriented.height)
        if (width <= 0 || height <= 0) return Outcome.Unreadable
        val fitted = if (width == oriented.width && height == oriented.height) {
            oriented
        } else {
            // `filter = true`: a sticker is looked at, and nearest-neighbour
            // shows at this size.
            oriented.scale(width, height, filter = true).also {
                if (it !== oriented) oriented.recycle()
            }
        }
        try {
            // PNG first: lossless, and every client here draws it.
            encode(fitted, Bitmap.CompressFormat.PNG, 100)
                ?.takeIf { it.size <= maxItemBytes }
                ?.let { return Outcome.Ready(Encoded(it, MIME_PNG, width, height)) }
            // Then WebP, which keeps the alpha a JPEG would not, stepping
            // down until it fits.
            for (quality in WEBP_QUALITY_STEPS) {
                encode(fitted, webpFormat(), quality)
                    ?.takeIf { it.size <= maxItemBytes && sniff(it) == MIME_WEBP }
                    ?.let { return Outcome.Ready(Encoded(it, MIME_WEBP, width, height)) }
            }
            Outcome.TooLarge
        } finally {
            fitted.recycle()
        }
        // runCatching, because every step allocates: an OutOfMemoryError
        // from a hostile image must fail the add, not the process.
    }.getOrElse { Outcome.Unreadable }

    @Suppress("DEPRECATION")
    private fun webpFormat(): Bitmap.CompressFormat =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            Bitmap.CompressFormat.WEBP_LOSSY
        } else {
            // Before API 30 there is one WebP format, lossy below quality
            // 100 — which is what the ladder asks for.
            Bitmap.CompressFormat.WEBP
        }

    private fun encode(bitmap: Bitmap, format: Bitmap.CompressFormat, quality: Int): ByteArray? =
        ByteArrayOutputStream().use { out ->
            if (bitmap.compress(format, quality, out)) out.toByteArray() else null
        }

    private val WEBP_QUALITY_STEPS = intArrayOf(90, 80, 65, 50)

    private fun ByteArray.matches(offset: Int, vararg chars: Char): Boolean {
        if (offset + chars.size > size) return false
        return chars.indices.all { this[offset + it].toInt() == chars[it].code }
    }

    private fun ByteArray.be32(offset: Int): Int =
        ((this[offset].toInt() and 0xFF) shl 24) or
            ((this[offset + 1].toInt() and 0xFF) shl 16) or
            ((this[offset + 2].toInt() and 0xFF) shl 8) or
            (this[offset + 3].toInt() and 0xFF)

    private fun ByteArray.le24(offset: Int): Int =
        (this[offset].toInt() and 0xFF) or
            ((this[offset + 1].toInt() and 0xFF) shl 8) or
            ((this[offset + 2].toInt() and 0xFF) shl 16)

    private fun ByteArray.le16(offset: Int): Int =
        (this[offset].toInt() and 0xFF) or ((this[offset + 1].toInt() and 0xFF) shl 8)
}
