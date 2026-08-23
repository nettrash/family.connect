/*
 * MediaPrep.kt
 * Family Connect (Android)
 *
 * Turning what the photo picker hands over into something worth
 * uploading: a downscaled JPEG for a photo, a re-encoded MP4 for a video
 * that would otherwise be too big, and in both cases a small preview the
 * bubble can draw before the full file has been fetched.
 *
 * Everything here produces a FILE IN THE CACHE DIRECTORY, never a
 * ByteArray. A 100 MB video read into memory is 100 MB resident on a
 * phone that also has to render a chat — and OkHttp can stream a request
 * body straight off the disk.
 *
 * The server never decodes an image or a video (docs/protocol.md), which
 * is exactly why the preview is made here.
 *
 * VIDEO POLICY: re-encode only when the original is over the ceiling.
 * Keep what the sender shot when it fits, compress when it does not, and
 * refuse only if compression was not enough — nettrash's call, and the
 * reason media3-transformer is in the build at all.
 *
 * iOS counterpart: ios/FamilyConnect/Core/MediaPrep.swift
 */

package me.nettrash.familyconnect.data.repo

import android.content.ContentResolver
import android.content.Context
import android.content.res.AssetFileDescriptor
import android.graphics.Bitmap
import android.media.MediaMetadataRetriever
import android.net.Uri
import android.provider.OpenableColumns
import androidx.annotation.OptIn
import androidx.core.graphics.scale
import androidx.media3.common.Effect
import androidx.media3.common.MediaItem
import androidx.media3.common.MimeTypes
import androidx.media3.common.util.UnstableApi
import androidx.media3.effect.Presentation
import androidx.media3.transformer.Composition
import androidx.media3.transformer.EditedMediaItem
import androidx.media3.transformer.Effects
import androidx.media3.transformer.ExportException
import androidx.media3.transformer.ExportResult
import androidx.media3.transformer.Transformer
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import java.io.ByteArrayOutputStream
import java.io.File
import java.util.UUID
import javax.inject.Inject
import javax.inject.Singleton
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

@Singleton
class MediaPrep @Inject constructor(
    @param:ApplicationContext private val context: Context,
    private val contentResolver: ContentResolver,
) {

    /** What the picker gave us, ready to upload. */
    data class Prepared(
        /** Lives in the cache directory; the sender deletes it after. */
        val file: File,
        val mime: String,
        /** [AttachmentDto.KIND_PHOTO] or [AttachmentDto.KIND_VIDEO]. */
        val kind: String,
        val width: Int?,
        val height: Int?,
        val durationMs: Int?,
        /** The bubble's thumbnail: the downscaled photo, or a poster frame. */
        val previewJpeg: ByteArray?,
        /** Files only: the name the sender picked it by. */
        val name: String? = null,
    )

    /** The item could not be read or decoded at all. */
    class UnreadableMedia : Exception("that item could not be read")

    /**
     * A video still over the ceiling after re-encoding — the only failure
     * the user can do something about, so the composer says so.
     */
    class TooLargeAfterCompression(val bytes: Long) :
        Exception("still $bytes bytes after compressing")

    // -- Photos ------------------------------------------------------------

    suspend fun preparePhoto(uri: Uri, limit: Long = SIZE_LIMIT): Prepared =
        withContext(Dispatchers.IO) {
            val source = readBytes(uri, PHOTO_READ_LIMIT) ?: throw UnreadableMedia()

            val full = AvatarImage.decode(source, maxPixels = PHOTO_EDGE)
                ?.let { AvatarImage.orientedByExif(it, source) }
                ?: throw UnreadableMedia()
            // Read the dimensions BEFORE recycling: they are what the
            // bubble reserves its shape from, and a recycled bitmap has
            // no defined answer for anything.
            val width = full.width.takeIf { it > 0 }
            val height = full.height.takeIf { it > 0 }
            val jpeg = try {
                encode(full, PHOTO_QUALITY)
            } finally {
                // The preview is decoded fresh from the original rather
                // than from this bitmap, so nothing else needs it.
                full.recycle()
            }
            // A downscaled photo is far under any sane ceiling, but a
            // pathological one could still exceed it — better refused here
            // than by the server.
            if (jpeg.size > limit) throw TooLargeAfterCompression(jpeg.size.toLong())

            val file = cacheFile("jpg")
            file.writeBytes(jpeg)

            val preview = AvatarImage.decode(source, maxPixels = PREVIEW_EDGE)
                ?.let { AvatarImage.orientedByExif(it, source) }
                ?.let { bitmap ->
                    try {
                        encode(bitmap, PREVIEW_QUALITY)
                    } finally {
                        bitmap.recycle()
                    }
                }

            Prepared(
                file = file,
                mime = "image/jpeg",
                kind = AttachmentDto.KIND_PHOTO,
                width = width,
                height = height,
                durationMs = null,
                previewJpeg = preview,
            )
        }

    // -- Videos ------------------------------------------------------------

    suspend fun prepareVideo(uri: Uri, limit: Long = SIZE_LIMIT): Prepared {
        val declared = declaredSize(uri)
        // The protocol accepts exactly two video types, and the server
        // verifies the bytes against the type declared. A .webm or .mkv is
        // neither — so re-encoding it is not about SIZE, it is the only way
        // it can be sent at all. Sending it as-is under the limit meant
        // labelling it video/mp4 and being refused, forever, with a message
        // that said "try again".
        val sourceMime = contentResolver.getType(uri).orEmpty()
        val mustTranscode = sourceMime !in SENDABLE_VIDEO_TYPES
        val file = if (mustTranscode || (declared != null && declared > limit)) {
            compress(uri, limit)
        } else {
            // Unknown length is treated as "probably fits": copy first,
            // then compress if the copy turns out to be over.
            val copy = withContext(Dispatchers.IO) { copyToCache(uri) }
            if (copy.length() > limit) {
                copy.delete()
                compress(uri, limit)
            } else {
                copy
            }
        }
        // What was actually produced: the transcoder writes MP4; an
        // untouched original keeps whichever accepted type it already was.
        val mime = if (mustTranscode) "video/mp4" else sourceMime.ifEmpty { "video/mp4" }

        return withContext(Dispatchers.IO) {
            val meta = readVideoMetadata(file)
            Prepared(
                file = file,
                mime = mime,
                kind = AttachmentDto.KIND_VIDEO,
                width = meta.width,
                height = meta.height,
                durationMs = meta.durationMs,
                previewJpeg = meta.poster,
            )
        }
    }

    /**
     * Re-encode to 720p, then insist the result fits.
     *
     * Transformer must be built and driven from a thread with a Looper and
     * calls back on that same thread — the main one here, which is safe
     * because every frame of the actual work happens on its own threads.
     */
    @OptIn(UnstableApi::class)
    private suspend fun compress(uri: Uri, limit: Long): File {
        val output = cacheFile("mp4")
        try {
            withContext(Dispatchers.Main) {
                suspendCancellableCoroutine<Unit> { continuation ->
                    val transformer = Transformer.Builder(context)
                        .setVideoMimeType(MimeTypes.VIDEO_H264)
                        .setAudioMimeType(MimeTypes.AUDIO_AAC)
                        .addListener(object : Transformer.Listener {
                            override fun onCompleted(
                                composition: Composition,
                                result: ExportResult,
                            ) {
                                continuation.resume(Unit)
                            }

                            override fun onError(
                                composition: Composition,
                                result: ExportResult,
                                exception: ExportException,
                            ) {
                                continuation.resumeWithException(exception)
                            }
                        })
                        .build()

                    val scale: Effect = Presentation.createForShortSide(COMPRESSED_SHORT_SIDE)
                    val item = EditedMediaItem.Builder(MediaItem.fromUri(uri))
                        .setEffects(Effects(emptyList(), listOf(scale)))
                        .build()

                    continuation.invokeOnCancellation { transformer.cancel() }
                    transformer.start(item, output.absolutePath)
                }
            }
        } catch (e: CancellationException) {
            output.delete()
            throw e
        } catch (_: Exception) {
            output.delete()
            throw UnreadableMedia()
        }

        val size = output.length()
        if (size > limit) {
            output.delete()
            throw TooLargeAfterCompression(size)
        }
        return output
    }

    private data class VideoMetadata(
        val width: Int?,
        val height: Int?,
        val durationMs: Int?,
        val poster: ByteArray?,
    )

    /**
     * Dimensions, duration, and a poster frame, in one pass over the file.
     *
     * A rotated video reports its stored dimensions, not its displayed
     * ones — so a portrait clip would tell the bubble it is landscape
     * unless the rotation is folded in here.
     */
    private fun readVideoMetadata(file: File): VideoMetadata {
        val retriever = MediaMetadataRetriever()
        return try {
            retriever.setDataSource(file.absolutePath)
            fun key(id: Int) = retriever.extractMetadata(id)?.toIntOrNull()

            val stored = key(MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH)
            val storedHeight = key(MediaMetadataRetriever.METADATA_KEY_VIDEO_HEIGHT)
            val rotation = key(MediaMetadataRetriever.METADATA_KEY_VIDEO_ROTATION) ?: 0
            val upright = rotation == 90 || rotation == 270
            val width = if (upright) storedHeight else stored
            val height = if (upright) stored else storedHeight

            // The first frame of a video that fades in is black, so a
            // little way in is a better thumbnail than time zero.
            val poster = POSTER_TIMES_US
                .firstNotNullOfOrNull { micros ->
                    runCatching {
                        retriever.getFrameAtTime(
                            micros,
                            MediaMetadataRetriever.OPTION_CLOSEST_SYNC,
                        )
                    }.getOrNull()
                }
                ?.let { frame ->
                    try {
                        encode(scaleWithin(frame, PREVIEW_EDGE), PREVIEW_QUALITY)
                    } finally {
                        frame.recycle()
                    }
                }

            VideoMetadata(
                width = width?.takeIf { it > 0 },
                height = height?.takeIf { it > 0 },
                durationMs = key(MediaMetadataRetriever.METADATA_KEY_DURATION),
                poster = poster,
            )
        } catch (_: Exception) {
            // A file we cannot read metadata from is still a file the
            // server will accept; the bubble just gets no shape and no
            // thumbnail. Not worth failing the send over.
            VideoMetadata(null, null, null, null)
        } finally {
            runCatching { retriever.release() }
        }
    }

    // -- Files -------------------------------------------------------------

    /**
     * Copy a picked document somewhere we own, and read its name and size.
     *
     * The copy is not incidental: the picker hands back a content Uri whose
     * read permission is granted to THIS process for the life of the
     * activity result — uploading straight from it works until the moment
     * a process death or a retry happens. Nothing is re-encoded and nothing
     * is inspected; a file is whatever the sender picked (protocol.md).
     */
    /**
     * Prepare a piece of audio — a recording, or a track off a disk.
     *
     * Nothing is re-encoded. A voice note is already recorded straight into
     * the container the server checks (AAC in MP4), and re-encoding
     * someone's music to save a few megabytes would be a worse trade than
     * refusing it. No preview: audio has nothing to look at, so a bubble
     * draws a play control, the duration and a scrubber (protocol.md,
     * "Audio").
     */
    suspend fun prepareAudio(uri: Uri, limit: Long = SIZE_LIMIT): Prepared =
        withContext(Dispatchers.IO) {
            val name = displayName(uri)
            val destination = cacheFile(name.substringAfterLast('.', "m4a"))
            val copied = runCatching {
                contentResolver.openInputStream(uri)?.use { input ->
                    destination.outputStream().use { output -> input.copyTo(output) }
                }
            }.getOrNull()
            if (copied == null) {
                destination.delete()
                throw UnreadableMedia()
            }

            val size = destination.length()
            if (size > limit) {
                destination.delete()
                throw TooLargeAfterCompression(size)
            }

            // NOT `use {}`: MediaMetadataRetriever only became AutoCloseable
            // in API 29 and minSdk here is 26, so the implicit cast would
            // crash on older devices. Lint catches this; the explicit
            // release is the fix, not a baseline entry.
            val retriever = MediaMetadataRetriever()
            val durationMs = try {
                runCatching {
                    retriever.setDataSource(destination.absolutePath)
                    retriever
                        .extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)
                        ?.toIntOrNull()
                }.getOrNull()
            } finally {
                runCatching { retriever.release() }
            }

            Prepared(
                file = destination,
                mime = audioMime(uri, destination),
                kind = AttachmentDto.KIND_AUDIO,
                width = null,
                height = null,
                durationMs = durationMs,
                previewJpeg = null,
                // A name only when there is one worth showing: a recording's
                // identity is its length, a track's is its title.
                name = name.takeIf { it.isNotBlank() },
            )
        }

    /**
     * The type the SERVER will accept, which is narrower than what the
     * provider might name. Anything unrecognised is left to the file path,
     * where nothing is verified.
     */
    private fun audioMime(uri: Uri, file: File): String =
        when (file.extension.lowercase()) {
            "m4a", "mp4", "aac" -> "audio/mp4"
            "mp3" -> "audio/mpeg"
            "wav", "wave" -> "audio/wav"
            "ogg", "oga" -> "audio/ogg"
            else -> contentResolver.getType(uri) ?: DEFAULT_FILE_MIME
        }

    suspend fun prepareFile(uri: Uri, limit: Long = SIZE_LIMIT): Prepared =
        withContext(Dispatchers.IO) {
            val name = displayName(uri)
            val destination = cacheFile(name.substringAfterLast('.', "bin"))
            val copied = runCatching {
                contentResolver.openInputStream(uri)?.use { input ->
                    destination.outputStream().use { output -> input.copyTo(output) }
                }
            }.getOrNull()
            if (copied == null) {
                destination.delete()
                throw UnreadableMedia()
            }

            val size = destination.length()
            if (size > limit) {
                // A document cannot be made smaller the way a video can, so
                // this really is the end of the road.
                destination.delete()
                throw TooLargeAfterCompression(size)
            }

            Prepared(
                file = destination,
                mime = contentResolver.getType(uri) ?: DEFAULT_FILE_MIME,
                kind = AttachmentDto.KIND_FILE,
                width = null,
                height = null,
                durationMs = null,
                previewJpeg = null,
                name = name,
            )
        }

    /**
     * The name the picker shows for this document.
     *
     * DISPLAY_NAME from the provider, never the Uri's last path segment —
     * for a `content://` Uri that is an opaque id ("msf:1000000042"), which
     * is exactly the useless label a file must not arrive with.
     */
    private fun displayName(uri: Uri): String {
        val fromProvider = runCatching {
            contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
                ?.use { cursor ->
                    val column = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                    if (column >= 0 && cursor.moveToFirst()) cursor.getString(column) else null
                }
        }.getOrNull()
        val name = fromProvider?.trim()?.takeIf { it.isNotEmpty() }
            ?: uri.lastPathSegment?.substringAfterLast('/')?.trim()?.takeIf { it.isNotEmpty() }
            ?: "file"
        return name.take(MAX_NAME_LEN)
    }

    // -- Shared ------------------------------------------------------------

    /** A file in the app's cache; the sender deletes it after uploading. */
    fun cacheFile(extension: String): File {
        val directory = File(context.cacheDir, UPLOAD_DIR).apply { mkdirs() }
        return File(directory, "upload-${UUID.randomUUID()}.$extension")
    }

    private fun declaredSize(uri: Uri): Long? = runCatching {
        contentResolver.openAssetFileDescriptor(uri, "r")?.use { descriptor ->
            descriptor.length.takeIf { it != AssetFileDescriptor.UNKNOWN_LENGTH }
        }
    }.getOrNull()

    private fun copyToCache(uri: Uri): File {
        val file = cacheFile("mp4")
        val copied = runCatching {
            contentResolver.openInputStream(uri)?.use { input ->
                file.outputStream().use { output -> input.copyTo(output) }
            }
        }.getOrNull()
        if (copied == null) {
            file.delete()
            throw UnreadableMedia()
        }
        return file
    }

    /** Bounded read — a picked photo has no business being huge. */
    private fun readBytes(uri: Uri, limit: Long): ByteArray? = runCatching {
        contentResolver.openInputStream(uri)?.use { stream ->
            val buffer = ByteArrayOutputStream()
            val chunk = ByteArray(64 * 1024)
            var total = 0L
            while (true) {
                val read = stream.read(chunk)
                if (read < 0) break
                total += read
                if (total > limit) return@use null
                buffer.write(chunk, 0, read)
            }
            buffer.toByteArray()
        }
    }.getOrNull()

    private fun encode(bitmap: Bitmap, quality: Int): ByteArray =
        ByteArrayOutputStream().use { out ->
            bitmap.compress(Bitmap.CompressFormat.JPEG, quality, out)
            out.toByteArray()
        }

    private fun scaleWithin(bitmap: Bitmap, maxEdge: Int): Bitmap {
        val longest = maxOf(bitmap.width, bitmap.height)
        if (longest <= maxEdge) return bitmap
        val factor = maxEdge.toFloat() / longest
        return bitmap.scale(
            (bitmap.width * factor).toInt().coerceAtLeast(1),
            (bitmap.height * factor).toInt().coerceAtLeast(1),
        )
    }

    companion object {
        /**
         * The protocol's default ceiling (docs/protocol.md). A self-hosted
         * server may be configured lower and does not advertise its limit,
         * so this is what the client PREPARES to; a stricter server still
         * answers `attachment_too_large` and the send fails with a message
         * rather than silently. Same value as iOS's MediaPrep.sizeLimit.
         */
        const val SIZE_LIMIT = 100L * 1024 * 1024

        /** Longest edge of an uploaded photo. Matches iOS. */
        const val PHOTO_EDGE = 2048

        /** Longest edge of the thumbnail drawn in a bubble. Matches iOS. */
        const val PREVIEW_EDGE = 600

        const val PHOTO_QUALITY = 85
        const val PREVIEW_QUALITY = 70

        /**
         * The video types the protocol accepts (docs/protocol.md). Anything
         * else has to go through the transcoder whatever its size.
         */
        val SENDABLE_VIDEO_TYPES = setOf("video/mp4", "video/quicktime")

        /**
         * Audio types the server's magic-number check knows. Anything else
         * claiming to be audio goes as a FILE — where the type is metadata
         * and nothing is verified — rather than earning a 400.
         */
        val SENDABLE_AUDIO_TYPES = setOf(
            "audio/mp4", "audio/m4a", "audio/aac",
            "audio/mpeg", "audio/mp3",
            "audio/wav", "audio/x-wav", "audio/vnd.wave",
            "audio/ogg",
        )

        /** 1280x720-ish: small enough to fit, big enough to watch. */
        const val COMPRESSED_SHORT_SIDE = 720

        /** Poster candidates, in order: half a second in, then the start. */
        val POSTER_TIMES_US = listOf(500_000L, 0L, 2_000_000L)

        /** What a file's bytes are called when the provider will not say. */
        const val DEFAULT_FILE_MIME = "application/octet-stream"

        /** The server's own ceiling on a name (protocol.md, "Files"). */
        const val MAX_NAME_LEN = 255

        /** A picked photo past this is not a photo. */
        private const val PHOTO_READ_LIMIT = 64L * 1024 * 1024

        private const val UPLOAD_DIR = "uploads"
    }
}
