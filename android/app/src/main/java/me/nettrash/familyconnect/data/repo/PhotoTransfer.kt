/*
 * PhotoTransfer.kt
 * Family Connect (Android)
 *
 * Prepare a full, local still photo for a temporary clipboard or Share URI.
 */

package me.nettrash.familyconnect.data.repo

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import java.io.DataInputStream
import java.io.File

internal object PhotoTransfer {
    const val DIRECTORY = "photo-transfers"
    const val MIME = "image/png"

    // Match the attachment decoder's pixel budget. Copy cannot silently
    // subsample an oversized original, so it fails without changing the clip.
    private const val MAX_PIXELS = 12_000_000L
    private const val MAX_BYTES = 32 * 1024 * 1024
    private const val MAX_AGE_MS = 60 * 60 * 1000L
    private const val MAX_FILES = 6

    fun accepts(attachment: AttachmentDto): Boolean =
        attachment.kind == AttachmentDto.KIND_PHOTO && !attachment.isSticker

    fun isComplete(source: File, size: Long): Boolean =
        size in 1..MAX_BYTES.toLong() && source.isFile && source.length() == size

    fun stage(source: File, size: Long, directory: File): File? = runCatching {
        if (!isComplete(source, size)) return null
        val bytes = ByteArray(size.toInt())
        DataInputStream(source.inputStream()).use {
            it.readFully(bytes)
            if (it.read() != -1) return null
        }
        if (AnimatedPicture.kindOf(bytes) == AnimatedPicture.Kind.ANIMATED) return null
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0 ||
            bounds.outWidth.toLong() * bounds.outHeight > MAX_PIXELS
        ) return null

        val bitmap = BitmapFactory.decodeByteArray(bytes, 0, bytes.size) ?: return null
        // The attachment display bitmap can be sampled and does not apply EXIF.
        val oriented = AvatarImage.orientedByExif(bitmap, bytes)
        try {
            directory.mkdirs()
            val part = File.createTempFile("photo-", ".part", directory)
            try {
                val complete = File(directory, part.nameWithoutExtension + ".png")
                val written = part.outputStream().use { oriented.compress(Bitmap.CompressFormat.PNG, 100, it) }
                complete.takeIf { written && part.renameTo(it) }
            } finally {
                part.delete()
            }
        } finally {
            oriented.recycle()
        }
    }.getOrNull()

    // A failed Copy must leave the previous image readable, so prune only
    // after delivering a replacement. Keep the newest targets for Share.
    fun pruneAfterTransfer(keep: File, now: Long = System.currentTimeMillis()) {
        keep.parentFile?.listFiles()
            ?.filter { it.isFile && it.extension == "png" && it != keep }
            ?.sortedByDescending { it.lastModified() }
            ?.forEachIndexed { index, file ->
                if (now - file.lastModified() > MAX_AGE_MS || index >= MAX_FILES - 1) file.delete()
            }
    }
}
