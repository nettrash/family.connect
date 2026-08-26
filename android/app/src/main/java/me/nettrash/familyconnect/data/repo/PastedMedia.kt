/*
 * PastedMedia.kt
 * Family Connect (Android)
 *
 * What a clipboard item should be SENT AS, decided before a single byte
 * is read.
 *
 * Pasting adds no protocol: a pasted item becomes an ordinary attachment
 * upload followed by the existing claim-on-send (docs/protocol.md,
 * "Photos, videos, audio, files and locations"). What it does add is an
 * item nobody picked from a picker — so the kind, the media type and the
 * NAME all have to be worked out from what little the clipboard says.
 *
 * The three rules that matter, all of them server-imposed:
 *
 *  - A photo is one of four types. `image/jpeg`, `png`, `heic`, `heif`
 *    are magic-checked; anything else claiming to be an image is refused
 *    as a photo. An animated GIF is the case that stings: re-encoding it
 *    to JPEG silently kills the animation, so a GIF (and webp, and bmp)
 *    goes as `kind=file`, where nothing is verified and the bytes arrive
 *    exactly as they were copied.
 *  - Audio the server's magic check knows gets a player; anything else
 *    calling itself audio falls through to `file` rather than earning a
 *    400. Same rule the document picker already follows (ChatViewModel's
 *    stageFile).
 *  - `kind=file` REQUIRES a name of 1–255 characters, and a clipboard
 *    item very often has none — hence [nameFor], which builds one out of
 *    the media type. It is only ever a FALLBACK: MediaPrep still prefers
 *    the item's own DISPLAY_NAME when its provider has one.
 *
 * Pure and platform-free on purpose: the scheme and the media type arrive
 * as strings, so every branch here is a plain unit test.
 *
 * iOS counterpart: the paste handling in ios/FamilyConnect/Core/MediaPrep.swift
 */

package me.nettrash.familyconnect.data.repo

import me.nettrash.familyconnect.data.net.dto.AttachmentDto

object PastedMedia {

    /**
     * Whether this app can open the item's bytes at all.
     *
     * A copied LINK is on the clipboard as a `https://` Uri, and it is
     * TEXT — pasting one must put the address in the composer, not try to
     * download it and attach the result. Only the two schemes whose bytes
     * a ContentResolver can actually open are attachable.
     */
    fun isReadable(scheme: String?): Boolean =
        scheme.equals("content", ignoreCase = true) || scheme.equals("file", ignoreCase = true)

    /**
     * The kind this item should be uploaded as, or null when it is not
     * something to attach at all (the caller then lets it paste as text).
     */
    fun kindFor(scheme: String?, mime: String?): String? {
        if (!isReadable(scheme)) return null
        val type = normalise(mime)
        return when {
            type in PHOTO_TYPES -> AttachmentDto.KIND_PHOTO
            // Every video type, not just the two the server accepts:
            // MediaPrep re-encodes the rest, which is the only way a
            // .webm can be sent at all.
            type.startsWith("video/") -> AttachmentDto.KIND_VIDEO
            type in MediaPrep.SENDABLE_AUDIO_TYPES -> AttachmentDto.KIND_AUDIO
            // Everything else — an unknown type, no type at all, a GIF,
            // a PDF, a spreadsheet. A file accepts anything.
            else -> AttachmentDto.KIND_FILE
        }
    }

    /**
     * Which word a synthesised name is built on: the top-level media type,
     * lower-cased, or "" when there is nothing to go on.
     */
    fun topLevelType(mime: String?): String = normalise(mime).substringBefore('/', "")

    /**
     * A name for an item that arrived without one: the localised word the
     * caller picked, plus an extension worked out from the media type.
     *
     * Never the cache file's name — that is `upload-<UUID>.bin` and is an
     * implementation detail of this device; it must never be what the rest
     * of the family sees on the bubble.
     */
    fun nameFor(base: String, mime: String?): String {
        val word = base.trim().ifEmpty { "file" }
        return "$word.${extensionFor(mime)}".take(MediaPrep.MAX_NAME_LEN)
    }

    /**
     * The file extension for a media type.
     *
     * A short table for the types a family actually copies, then a
     * best-effort read of the subtype (`application/zip` → `zip`,
     * `image/svg+xml` → `svg`, `application/x-tar` → `tar`), and `bin`
     * when even that is not a word. Deliberately not MimeTypeMap: its
     * answers vary by device and it maps plenty of these to nothing.
     */
    fun extensionFor(mime: String?): String {
        val type = normalise(mime)
        KNOWN_EXTENSIONS[type]?.let { return it }
        val subtype = type.substringAfter('/', "")
            .substringBefore('+')
            .removePrefix("vnd.")
            .removePrefix("x-")
            .substringAfterLast('.')
        return if (subtype.length in 1..8 && subtype.all { it.isLetterOrDigit() }) {
            subtype
        } else {
            DEFAULT_EXTENSION
        }
    }

    /** `image/jpeg; charset=utf-8` and `IMAGE/JPEG` are the same type. */
    private fun normalise(mime: String?): String =
        mime?.substringBefore(';')?.trim()?.lowercase().orEmpty()

    /**
     * The four the server magic-checks as a photo (docs/protocol.md).
     * `image/jpg` is not a real media type but providers emit it, and it
     * means the same thing — a photo is re-encoded to JPEG anyway.
     */
    val PHOTO_TYPES = setOf(
        "image/jpeg",
        "image/jpg",
        "image/png",
        "image/heic",
        "image/heif",
    )

    private const val DEFAULT_EXTENSION = "bin"

    private val KNOWN_EXTENSIONS = mapOf(
        "image/jpeg" to "jpg",
        "image/jpg" to "jpg",
        "image/png" to "png",
        "image/gif" to "gif",
        "image/webp" to "webp",
        "image/bmp" to "bmp",
        "image/heic" to "heic",
        "image/heif" to "heif",
        "image/svg+xml" to "svg",
        "image/tiff" to "tiff",
        "video/mp4" to "mp4",
        "video/quicktime" to "mov",
        "video/webm" to "webm",
        "video/x-matroska" to "mkv",
        "audio/mp4" to "m4a",
        "audio/m4a" to "m4a",
        "audio/aac" to "aac",
        "audio/mpeg" to "mp3",
        "audio/mp3" to "mp3",
        "audio/wav" to "wav",
        "audio/x-wav" to "wav",
        "audio/vnd.wave" to "wav",
        "audio/ogg" to "ogg",
        "application/pdf" to "pdf",
        "application/zip" to "zip",
        "application/rtf" to "rtf",
        "application/json" to "json",
        "application/msword" to "doc",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" to "docx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" to "xlsx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" to "pptx",
        "text/plain" to "txt",
        "text/html" to "html",
        "text/csv" to "csv",
        "text/markdown" to "md",
    )
}
