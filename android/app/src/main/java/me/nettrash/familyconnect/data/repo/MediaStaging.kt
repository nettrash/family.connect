/*
 * MediaStaging.kt
 * Family Connect (Android)
 *
 * Where the bytes of a queued media send live until the server has them.
 *
 * WHY NOT cacheDir. MediaPrep stages into `cacheDir/uploads`, which the
 * system may evict at any time — the right home for a file the composer is
 * still holding, and the wrong home for a message somebody has pressed
 * Send on. Once a send is a row in the database its bytes have to outlive
 * a process death, so they move to `filesDir/outbox`, which the system
 * does not reclaim.
 *
 * ONE DIRECTORY PER ITEM, and the file keeps its real name inside it: the
 * name is a file attachment's whole identity, and two people sending
 * `Invoice.pdf` must not collide.
 *
 * iOS counterpart: ios/FamilyConnect/Core/PendingMediaStaging.swift.
 */

package me.nettrash.familyconnect.data.repo

import android.content.Context
import dagger.hilt.android.qualifiers.ApplicationContext
import java.io.File
import java.util.UUID
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class MediaStaging @Inject constructor(
    @param:ApplicationContext private val context: Context,
) {

    private val root: File get() = File(context.filesDir, OUTBOX_DIR).apply { mkdirs() }

    /** What [adopt] wrote: the two absolute paths the row stores. */
    data class Adopted(val path: String, val previewPath: String?)

    /**
     * Take ownership of a prepared item's bytes.
     *
     * A RENAME when the source is one of MediaPrep's own staged files, a
     * COPY otherwise. The difference matters: a share import or a picked
     * document can hand back a path the person owns, and moving that is
     * not staging, it is taking their file away from them.
     *
     * The preview is written out beside it, which is what lets a poster
     * survive a relaunch — it used to exist only in memory.
     */
    fun adopt(item: MediaPrep.Prepared, itemId: String = UUID.randomUUID().toString()): Adopted? {
        val directory = File(root, itemId).apply { mkdirs() }
        val destination = File(directory, item.file.name)
        val moved = isOurs(item.file) && item.file.renameTo(destination)
        if (!moved) {
            runCatching { item.file.copyTo(destination, overwrite = true) }
                .getOrElse { directory.deleteRecursively(); return null }
        }
        var previewPath: String? = null
        item.previewJpeg?.let { jpeg ->
            val preview = File(directory, "preview.jpg")
            runCatching { preview.writeBytes(jpeg) }.onSuccess { previewPath = preview.absolutePath }
        }
        return Adopted(destination.absolutePath, previewPath)
    }

    /** Everything one item owns, gone. */
    fun remove(path: String?) {
        val file = path?.let(::File) ?: return
        // The item's directory, not just the file: the preview lives there too.
        file.parentFile?.takeIf { it.parentFile == root }?.deleteRecursively() ?: file.delete()
    }

    /**
     * Delete every staged directory no row names any more.
     *
     * The counterpart of MediaPrep's cache, and deliberately not the same
     * sweep: these bytes belong to messages somebody pressed Send on, so
     * only a directory nothing points at may go.
     */
    fun sweepOrphans(livePaths: Set<String>, now: Long = System.currentTimeMillis()): Int {
        val keep = livePaths.mapNotNull { File(it).parentFile?.absolutePath }.toSet()
        var removed = 0
        root.listFiles()?.forEach { directory ->
            // An AGE FLOOR as well as the keep-set, and it is load-bearing:
            // a send stages its bytes and THEN writes its rows, so a sweep
            // landing between the two sees files nothing names yet and
            // would delete the message somebody just pressed Send on.
            // Nothing this young can be an orphan.
            val young = now - directory.lastModified() < MINIMUM_ORPHAN_AGE_MS
            if (directory.absolutePath !in keep && !young) {
                if (directory.deleteRecursively()) removed++
            }
        }
        return removed
    }

    private fun isOurs(file: File): Boolean =
        file.absolutePath.startsWith(context.cacheDir.absolutePath)

    private companion object {
        /** Below this, a directory may simply be one being written now. */
        const val MINIMUM_ORPHAN_AGE_MS = 10 * 60 * 1000L
        const val OUTBOX_DIR = "outbox"
    }
}
