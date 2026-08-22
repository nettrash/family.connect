/*
 * AvatarImage.kt
 * Family Connect (Android)
 *
 * Turning whatever the photo picker hands over into something worth
 * uploading: a centre-cropped square JPEG with its edge at EDGE pixels.
 *
 * This is the client's job, not the server's. The server stores bytes and
 * never transcodes (docs/protocol.md — "Profile pictures"), and its
 * 256 KiB ceiling is a backstop rather than a resizer: a modern phone
 * photo is several megabytes and would simply be refused.
 *
 * Decoding goes through inJustDecodeBounds + inSampleSize so a
 * 12-megapixel original never becomes a 48 MB bitmap on the way to a
 * 512 px square.
 *
 * EXIF: BitmapFactory ignores orientation, so a portrait photo from the
 * camera decodes on its side. ExifInterface reads the tag and the
 * transform is applied by hand — ImageDecoder would do this for us but
 * needs API 28, and minSdk here is 26.
 *
 * iOS counterpart: ios/FamilyConnect/Core/AvatarImage.swift (ImageIO).
 */

package me.nettrash.familyconnect.data.repo

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Matrix
import androidx.core.graphics.createBitmap
import androidx.core.graphics.scale
import androidx.exifinterface.media.ExifInterface
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream

object AvatarImage {

    /**
     * Edge of the uploaded square, in pixels. 512 is sharp on every
     * avatar this app draws (the largest is 56 dp) and encodes to well
     * under the server's limit.
     */
    const val EDGE = 512

    /** No visible artefacts at avatar size, a fraction of the bytes of 100. */
    const val QUALITY = 80

    /**
     * Square JPEG for upload, or null when the bytes are not a decodable
     * image.
     */
    fun squareJpeg(source: ByteArray, edge: Int = EDGE): ByteArray? = runCatching {
        // Decode at ~2x the target so the final downscale still has
        // detail to work with, then let the scale step do the rest.
        val decoded = decode(source, maxPixels = edge * 2) ?: return null
        val oriented = applyExifOrientation(decoded, source)
        val square = centreCropSquare(oriented)
        val resized = scaleTo(square, edge)
        try {
            ByteArrayOutputStream().use { out ->
                // JPEG has no alpha: a transparent PNG would encode its
                // transparent pixels black, so it is drawn onto white
                // first.
                val opaque = flattenOntoWhite(resized)
                val ok = opaque.compress(Bitmap.CompressFormat.JPEG, QUALITY, out)
                if (opaque !== resized) opaque.recycle()
                if (ok) out.toByteArray() else null
            }
        } finally {
            resized.recycle()
        }
        // runCatching, because every step here allocates: an
        // OutOfMemoryError from a hostile image must fail the upload, not
        // the process — and must not leave the button spinning.
    }.getOrNull()

    private fun flattenOntoWhite(bitmap: Bitmap): Bitmap {
        if (!bitmap.hasAlpha()) return bitmap
        val flattened = createBitmap(bitmap.width, bitmap.height)
        Canvas(flattened).apply {
            drawColor(Color.WHITE)
            drawBitmap(bitmap, 0f, 0f, null)
        }
        return flattened
    }

    /**
     * Decode for display, bounded so a hostile or accidental
     * many-megapixel body can't blow up the heap. Null when the bytes are
     * not an image.
     */
    fun decode(source: ByteArray, maxPixels: Int): Bitmap? {
        if (source.isEmpty()) return null
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(source, 0, source.size, bounds)
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
        val options = BitmapFactory.Options().apply {
            inSampleSize = sampleSize(bounds.outWidth, bounds.outHeight, maxPixels)
        }
        return runCatching {
            BitmapFactory.decodeByteArray(source, 0, source.size, options)
        }.getOrNull()
    }

    /**
     * The power-of-two subsampling factor that brings the image down to
     * `maxPixels` or just above it — never below, so the later scale step
     * is always a downscale.
     *
     * It measures the SHORTEST edge, because that is the one the centre
     * crop keeps: measuring the longest would let a 6000x1000 panorama
     * subsample to 1500x250 and upload a 250 px avatar.
     *
     * The second loop is the memory backstop that the first one gives up:
     * a very long image can be enormous even with its short edge at
     * target, so the decoded area is capped as well.
     */
    internal fun sampleSize(width: Int, height: Int, maxPixels: Int): Int {
        if (maxPixels <= 0) return 1
        var sample = 1
        var shortest = minOf(width, height)
        while (shortest / 2 >= maxPixels) {
            shortest /= 2
            sample *= 2
        }
        while (width.toLong() * height / (sample.toLong() * sample) > MAX_DECODE_PIXELS) {
            sample *= 2
        }
        return sample
    }

    /**
     * Decoded-pixel ceiling, ~48 MB at ARGB_8888. Only a pathologically
     * long image (past about 12:1) trades avatar sharpness for it.
     */
    private const val MAX_DECODE_PIXELS = 12_000_000L

    /**
     * The largest centred square — what a circular avatar shows anyway,
     * so the bytes are not spent on pixels nobody will see.
     */
    internal fun centreCropSquare(bitmap: Bitmap): Bitmap {
        val side = minOf(bitmap.width, bitmap.height)
        if (bitmap.width == bitmap.height) return bitmap
        val x = (bitmap.width - side) / 2
        val y = (bitmap.height - side) / 2
        val cropped = Bitmap.createBitmap(bitmap, x, y, side, side)
        if (cropped !== bitmap) bitmap.recycle()
        return cropped
    }

    private fun scaleTo(bitmap: Bitmap, edge: Int): Bitmap {
        if (bitmap.width <= edge && bitmap.height <= edge) return bitmap
        val scaled = bitmap.scale(edge, edge)
        if (scaled !== bitmap) bitmap.recycle()
        return scaled
    }

    /**
     * Rotate/flip per the original's EXIF tag. Unreadable EXIF is not an
     * error — most images have none.
     */
    private fun applyExifOrientation(bitmap: Bitmap, source: ByteArray): Bitmap {
        val orientation = runCatching {
            ByteArrayInputStream(source).use {
                ExifInterface(it).getAttributeInt(
                    ExifInterface.TAG_ORIENTATION,
                    ExifInterface.ORIENTATION_NORMAL,
                )
            }
        }.getOrDefault(ExifInterface.ORIENTATION_NORMAL)

        val matrix = Matrix()
        when (orientation) {
            ExifInterface.ORIENTATION_ROTATE_90 -> matrix.postRotate(90f)
            ExifInterface.ORIENTATION_ROTATE_180 -> matrix.postRotate(180f)
            ExifInterface.ORIENTATION_ROTATE_270 -> matrix.postRotate(270f)
            ExifInterface.ORIENTATION_FLIP_HORIZONTAL -> matrix.postScale(-1f, 1f)
            ExifInterface.ORIENTATION_FLIP_VERTICAL -> matrix.postScale(1f, -1f)
            ExifInterface.ORIENTATION_TRANSPOSE -> {
                matrix.postRotate(90f)
                matrix.postScale(-1f, 1f)
            }
            ExifInterface.ORIENTATION_TRANSVERSE -> {
                matrix.postRotate(270f)
                matrix.postScale(-1f, 1f)
            }
            else -> return bitmap
        }
        val rotated = runCatching {
            Bitmap.createBitmap(bitmap, 0, 0, bitmap.width, bitmap.height, matrix, true)
        }.getOrNull() ?: return bitmap
        if (rotated !== bitmap) bitmap.recycle()
        return rotated
    }
}
