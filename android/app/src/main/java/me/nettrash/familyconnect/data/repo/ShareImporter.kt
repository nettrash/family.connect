/*
 * ShareImporter.kt
 * Family Connect (Android)
 *
 * Copy what another app shared into files this app owns, IMMEDIATELY:
 * the read grants on shared content Uris are transient — they die with
 * the sending activity's result — so the bytes are pulled through the
 * same MediaPrep pipeline a picked item takes (downscaled photo, poster
 * frame, magic-checked audio, named file) before anything else happens.
 *
 * An interface for the same reason CallStarter is one: MainViewModel
 * defaults it away so its tests need none of MediaPrep's platform
 * machinery, and Hilt injects the real one.
 */

package me.nettrash.familyconnect.data.repo

import android.content.Context
import android.net.Uri
import dagger.hilt.android.qualifiers.ApplicationContext
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import javax.inject.Inject
import javax.inject.Singleton

interface ShareImporter {

    /**
     * Prepare each shared Uri, in the sender's order. Per-item failures
     * skip the item rather than the share — nine readable photos and one
     * revoked grant should land nine photos. Empty means nothing at all
     * could be read.
     */
    suspend fun prepare(uris: List<Uri>): List<MediaPrep.Prepared>

    companion object {
        /** The test default: prepares nothing. The app binds [DefaultShareImporter]. */
        val NONE: ShareImporter = object : ShareImporter {
            override suspend fun prepare(uris: List<Uri>): List<MediaPrep.Prepared> = emptyList()
        }
    }
}

@Singleton
class DefaultShareImporter @Inject constructor(
    @param:ApplicationContext private val context: Context,
    private val mediaPrep: MediaPrep,
) : ShareImporter {

    override suspend fun prepare(uris: List<Uri>): List<MediaPrep.Prepared> {
        val prepared = ArrayList<MediaPrep.Prepared>(uris.size)
        for (uri in uris) {
            // The provider's own answer first, exactly like the paste
            // path: it describes THIS item, where the intent's type
            // describes the whole share.
            val mime = runCatching { context.contentResolver.getType(uri) }.getOrNull()
            val kind = PastedMedia.kindFor(uri.scheme, mime) ?: continue
            val item = runCatching {
                when (kind) {
                    // The SAME preparation the picker and the clipboard
                    // use. A third path would be a third set of bugs.
                    AttachmentDto.KIND_PHOTO -> mediaPrep.preparePhoto(uri)
                    AttachmentDto.KIND_VIDEO -> mediaPrep.prepareVideo(uri, declaredMime = mime)
                    AttachmentDto.KIND_AUDIO -> mediaPrep.prepareAudio(
                        uri,
                        declaredMime = mime,
                        fallbackName = sharedName(mime),
                    )
                    // `kind=file` REQUIRES a name; most shares carry a
                    // DISPLAY_NAME, and the localised fallback covers the
                    // ones that do not.
                    else -> mediaPrep.prepareFile(
                        uri,
                        declaredMime = mime,
                        fallbackName = sharedName(mime),
                    )
                }
            }.getOrNull() ?: continue
            prepared += item
        }
        return prepared
    }

    /**
     * A name for a shared item that arrived without one — what the rest
     * of the family will see on the bubble, so it is localised, never the
     * cache file's `upload-<UUID>` (the same rule the paste names follow).
     */
    private fun sharedName(mime: String?): String =
        PastedMedia.nameFor(context.getString(R.string.s_shared_with_family), mime)
}
