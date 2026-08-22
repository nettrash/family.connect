/*
 * GallerySaver.kt
 * Family Connect (Android)
 *
 * Copying a photo or video out of the chat and into the phone's own
 * gallery.
 *
 * This exists because Android's share chooser — unlike iOS's share sheet,
 * which carries "Save Image" itself — has no save-to-gallery action. Share
 * reaches Drive, Files and whatever apps accept a stream; putting a picture
 * in Photos is a MediaStore write the app has to do.
 *
 * TWO PATHS, and the split is the whole complexity here:
 *   - API 29+: insert into MediaStore with IS_PENDING, stream the bytes in,
 *     clear the flag. No permission at all — the app only ever writes rows
 *     it created.
 *   - API 26–28: MediaStore has no scoped-write, so the file is copied into
 *     the public Pictures/Movies directory and the scanner is told about
 *     it. That path DOES need WRITE_EXTERNAL_STORAGE, which the manifest
 *     therefore declares with maxSdkVersion="28" — it must never be asked
 *     for on a modern device.
 *
 * iOS counterpart: none — the share sheet's own "Save Image" does this.
 */

package me.nettrash.familyconnect.data.repo

import android.content.ContentValues
import android.content.Context
import android.media.MediaScannerConnection
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import androidx.annotation.RequiresApi
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class GallerySaver @Inject constructor() {

    /** Everything a caller needs to tell the user what happened. */
    enum class Result { SAVED, NEEDS_PERMISSION, FAILED }

    /** True when this device needs WRITE_EXTERNAL_STORAGE to save at all. */
    val needsLegacyPermission: Boolean
        get() = Build.VERSION.SDK_INT < Build.VERSION_CODES.Q

    suspend fun save(
        context: Context,
        file: File,
        mime: String,
        displayName: String,
        isVideo: Boolean,
    ): Result = withContext(Dispatchers.IO) {
        runCatching {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                saveViaMediaStore(context, file, mime, displayName, isVideo)
            } else {
                saveToPublicDirectory(context, file, mime, displayName, isVideo)
            }
        }.getOrElse { error ->
            if (error is SecurityException) Result.NEEDS_PERMISSION else Result.FAILED
        }
    }

    @RequiresApi(Build.VERSION_CODES.Q)
    private fun saveViaMediaStore(
        context: Context,
        file: File,
        mime: String,
        displayName: String,
        isVideo: Boolean,
    ): Result {
        val resolver = context.contentResolver
        val collection = if (isVideo) {
            MediaStore.Video.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
        } else {
            MediaStore.Images.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
        }
        val folder = if (isVideo) Environment.DIRECTORY_MOVIES else Environment.DIRECTORY_PICTURES
        val values = ContentValues().apply {
            put(MediaStore.MediaColumns.DISPLAY_NAME, displayName)
            put(MediaStore.MediaColumns.MIME_TYPE, mime.ifEmpty { fallbackMime(isVideo) })
            put(MediaStore.MediaColumns.RELATIVE_PATH, "$folder/$ALBUM")
            // Hidden from the gallery until the bytes are all there, so a
            // half-copied video never shows up as a broken thumbnail.
            put(MediaStore.MediaColumns.IS_PENDING, 1)
        }

        val uri = resolver.insert(collection, values) ?: return Result.FAILED
        try {
            resolver.openOutputStream(uri)?.use { output ->
                file.inputStream().use { input -> input.copyTo(output) }
            } ?: return Result.FAILED
        } catch (error: Exception) {
            // Leave nothing half-written in the user's gallery.
            resolver.delete(uri, null, null)
            throw error
        }
        values.clear()
        values.put(MediaStore.MediaColumns.IS_PENDING, 0)
        resolver.update(uri, values, null, null)
        return Result.SAVED
    }

    private fun saveToPublicDirectory(
        context: Context,
        file: File,
        mime: String,
        displayName: String,
        isVideo: Boolean,
    ): Result {
        @Suppress("DEPRECATION")
        val root = Environment.getExternalStoragePublicDirectory(
            if (isVideo) Environment.DIRECTORY_MOVIES else Environment.DIRECTORY_PICTURES,
        )
        val folder = File(root, ALBUM)
        if (!folder.exists() && !folder.mkdirs()) return Result.FAILED

        val target = uniqueFile(folder, displayName)
        file.inputStream().use { input ->
            target.outputStream().use { output -> input.copyTo(output) }
        }
        // Without this the file is on disk and invisible: the gallery only
        // knows what the scanner has told it about.
        MediaScannerConnection.scanFile(
            context,
            arrayOf(target.absolutePath),
            arrayOf(mime.ifEmpty { fallbackMime(isVideo) }),
            null,
        )
        return Result.SAVED
    }

    /** "photo-3.jpg", then "photo-3 (1).jpg" — never overwrite. */
    private fun uniqueFile(folder: File, displayName: String): File {
        val base = displayName.substringBeforeLast('.', displayName)
        val ext = displayName.substringAfterLast('.', "")
        var candidate = File(folder, displayName)
        var index = 1
        while (candidate.exists()) {
            val suffix = if (ext.isEmpty()) "" else ".$ext"
            candidate = File(folder, "$base ($index)$suffix")
            index++
        }
        return candidate
    }

    private fun fallbackMime(isVideo: Boolean) = if (isVideo) "video/mp4" else "image/jpeg"

    private companion object {
        /** Its own album, so saved pictures are findable and a bulk delete
         *  is one folder rather than a hunt through the camera roll. */
        const val ALBUM = "Family"
    }
}
