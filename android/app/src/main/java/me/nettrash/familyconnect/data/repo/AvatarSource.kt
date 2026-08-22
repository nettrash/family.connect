/*
 * AvatarSource.kt
 * Family Connect (Android)
 *
 * Reading the bytes behind a picked image. It is a seam rather than a
 * ContentResolver call at the call site for two reasons: the read belongs
 * on viewModelScope (a Uri backed by a cloud photo library can take
 * seconds, and a read owned by the composable would be cancelled — and
 * its busy flag stranded — the moment the user backs out), and a
 * ViewModel that takes a Context is a ViewModel no plain-JVM test can
 * build.
 *
 * The size cap is a guard, not a policy: the picture is downscaled to a
 * 512 px square immediately afterwards, so nothing legitimate needs more,
 * and a 100 MB raw file would otherwise be read whole into a ByteArray
 * (doubled transiently) before a single pixel is inspected.
 *
 * iOS counterpart: PhotosPickerItem.loadTransferable(type: Data.self),
 * which the system already runs off the main actor.
 */

package me.nettrash.familyconnect.data.repo

import android.content.ContentResolver
import android.content.res.AssetFileDescriptor
import android.net.Uri
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.ByteArrayOutputStream
import java.io.InputStream
import javax.inject.Inject
import javax.inject.Singleton

fun interface AvatarSource {
    /** The image's bytes, or null when it can't be read or is too large. */
    suspend fun read(uri: Uri): ByteArray?
}

@Singleton
class ContentResolverAvatarSource @Inject constructor(
    private val contentResolver: ContentResolver,
) : AvatarSource {

    override suspend fun read(uri: Uri): ByteArray? = withContext(Dispatchers.IO) {
        runCatching {
            val declared = contentResolver.openAssetFileDescriptor(uri, "r")?.use { it.length }
            // UNKNOWN_LENGTH means the provider won't say — the read below
            // is bounded either way, this just avoids starting a doomed one.
            if (declared != null &&
                declared != AssetFileDescriptor.UNKNOWN_LENGTH &&
                declared > MAX_BYTES
            ) {
                null
            } else {
                contentResolver.openInputStream(uri)?.use(::readBounded)
            }
        }.getOrNull()
    }

    /**
     * Read the stream, giving up past [MAX_BYTES]. Hand-rolled because
     * `InputStream.readNBytes` is API 33 and this app runs from 26.
     */
    private fun readBounded(stream: InputStream): ByteArray? {
        val buffer = ByteArrayOutputStream()
        val chunk = ByteArray(64 * 1024)
        var total = 0L
        while (true) {
            val read = stream.read(chunk)
            if (read < 0) break
            total += read
            if (total > MAX_BYTES) return null
            buffer.write(chunk, 0, read)
        }
        return buffer.toByteArray()
    }

    private companion object {
        /** Comfortably past any phone photo; a 200 MP raw is not a face. */
        const val MAX_BYTES = 32L * 1024 * 1024
    }
}
