/*
 * Mp4Faststart.kt
 * Family Connect (Android)
 *
 * Putting an MP4's `moov` box in front of its `mdat`.
 *
 * docs/protocol.md, "Preparing media before upload": the container is MP4
 * with `moov` before `mdat`, "so a player can start on the first bytes of a
 * `Range` read". The server honours Range, so a file with its index at the
 * END still plays — but only after the player has gone to the tail for it,
 * which on a phone on a train is the difference between a video that starts
 * and one that spins.
 *
 * WHY THIS EXISTS AT ALL. Media3's muxer only ATTEMPTS it: InAppMp4Muxer
 * reserves 400 000 bytes after `ftyp` and writes the `moov` there if it fits,
 * and at the end of the file if it does not (media3-muxer Mp4Writer,
 * DEFAULT_MOOV_BOX_SIZE_BYTES; "Setting to true does not guarantee a
 * streamable MP4 output"). A `moov` is a few bytes per sample and per chunk,
 * so a clip of several minutes — exactly the ones near the ceiling — can
 * outgrow the reservation. So the transcoder's output is checked, and moved
 * when it has to be: the same thing `qt-faststart` does.
 *
 * Moving `moov` forward shifts every byte of `mdat` by the same amount, and
 * the `stco`/`co64` tables inside `moov` hold absolute file offsets into
 * `mdat` — so each one is shifted by that amount too. That is the whole
 * algorithm. Anything it does not understand leaves the file EXACTLY as it
 * was: a file with `moov` at the end still plays everywhere, and a
 * half-rewritten one plays nowhere.
 *
 * Pure JVM (java.io / java.nio only), so it is tested on synthetic boxes
 * without a device: Mp4FaststartTest.
 */

package me.nettrash.familyconnect.data.repo

import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.io.RandomAccessFile
import java.nio.ByteBuffer
import java.nio.channels.FileChannel

object Mp4Faststart {

    /** One top-level box: where it starts and how long it is, header included. */
    internal data class Box(val type: String, val offset: Long, val size: Long)

    /**
     * Whether `moov` comes before the first `mdat` — what the protocol's
     * Container row asks. Null when this is not an MP4 that can be walked.
     */
    fun moovFirst(file: File): Boolean? = runCatching {
        RandomAccessFile(file, "r").use { access ->
            val top = topLevel(access.channel) ?: return@use null
            val moov = top.indexOfFirst { it.type == "moov" }
            val mdat = top.indexOfFirst { it.type == "mdat" }
            if (moov < 0 || mdat < 0) null else moov < mdat
        }
    }.getOrNull()

    /**
     * Rewrite [file] so its `moov` comes first.
     *
     * @return true when the file IS moov-first afterwards (including when it
     *   already was); false when it was left exactly as it was — not an MP4
     *   this understands, an offset that points somewhere it cannot follow,
     *   or an I/O failure. Never throws: this is an optimisation on top of a
     *   file that is already valid.
     */
    fun apply(file: File): Boolean = runCatching { rewrite(file) }.getOrDefault(false)

    private fun rewrite(file: File): Boolean {
        val temp = File(file.parentFile, "${file.name}.faststart")
        val rewritten = RandomAccessFile(file, "r").use { access ->
            val channel = access.channel
            val top = topLevel(channel) ?: return false
            val moovIndex = top.indexOfFirst { it.type == "moov" }
            val mdatIndex = top.indexOfFirst { it.type == "mdat" }
            if (moovIndex < 0 || mdatIndex < 0) return false
            if (moovIndex < mdatIndex) return true

            // Only padding may follow the `moov` — that is what a muxer that
            // wrote it last leaves. Anything else after it would move by a
            // different amount than `mdat` does, and this does not chase that.
            if (top.drop(moovIndex + 1).any { it.type !in PADDING }) return false
            val moov = top[moovIndex]
            if (moov.size > MAX_MOOV_BYTES) return false

            // Everything before the first `mdat` except padding (the 400 000
            // bytes Media3 reserved for a `moov` that did not fit are dropped
            // here), then the `moov`, then the block from the first `mdat` up
            // to where the `moov` was — which moves as ONE block, so every
            // offset into it moves by one amount.
            val head = top.subList(0, mdatIndex).filter { it.type !in PADDING }
            val movedStart = top[mdatIndex].offset
            val movedEnd = moov.offset
            val delta = head.sumOf { it.size } + moov.size - movedStart

            val index = ByteBuffer.allocate(moov.size.toInt())
            readFully(channel, index, moov.offset)
            val shifter = OffsetShifter(delta, movedStart, movedEnd)
            if (!shifter.container(index, 0, index.capacity())) return false

            try {
                FileOutputStream(temp).channel.use { out ->
                    head.forEach { copy(channel, it.offset, it.size, out) }
                    index.rewind()
                    while (index.hasRemaining()) out.write(index)
                    copy(channel, movedStart, movedEnd - movedStart, out)
                }
                true
            } catch (_: Exception) {
                // Anything at all, not only IOException: the temp file is
                // deleted below whatever went wrong while it was written.
                false
            }
        }
        // Replaced only once the whole new file is written: a rename, so the
        // original is either untouched or fully replaced, never half of each.
        if (!rewritten || !temp.renameTo(file)) {
            temp.delete()
            return false
        }
        return true
    }

    /**
     * Adds [delta] to every chunk offset in a `moov`, refusing (false) on any
     * offset outside `[movedStart, movedEnd)` — data this rewrite is not
     * moving — or one a 32-bit `stco` can no longer hold.
     */
    private class OffsetShifter(
        private val delta: Long,
        private val movedStart: Long,
        private val movedEnd: Long,
    ) {
        fun container(buffer: ByteBuffer, start: Int, end: Int): Boolean {
            var at = start
            while (at < end) {
                if (end - at < 8) return false
                val size32 = buffer.getInt(at).toLong() and 0xFFFF_FFFFL
                val type = fourCc(buffer, at + 4)
                var header = 8
                val size = when (size32) {
                    0L -> (end - at).toLong()
                    1L -> {
                        if (end - at < 16) return false
                        header = 16
                        buffer.getLong(at + 8)
                    }
                    else -> size32
                }
                if (size < header || size > end - at) return false
                val boxEnd = at + size.toInt()
                val fine = when (type) {
                    in CONTAINERS -> container(buffer, at + header, boxEnd)
                    "stco" -> offsets(buffer, at + header, boxEnd, width = 4)
                    "co64" -> offsets(buffer, at + header, boxEnd, width = 8)
                    else -> true
                }
                if (!fine) return false
                at = boxEnd
            }
            return true
        }

        /** A full box: version and flags (4), an entry count (4), then the entries. */
        private fun offsets(buffer: ByteBuffer, start: Int, end: Int, width: Int): Boolean {
            if (end - start < 8) return false
            val count = buffer.getInt(start + 4).toLong() and 0xFFFF_FFFFL
            if (8 + count * width > end - start) return false
            for (entry in 0 until count.toInt()) {
                val at = start + 8 + entry * width
                val old = if (width == 4) buffer.getInt(at).toLong() and 0xFFFF_FFFFL else buffer.getLong(at)
                if (old < movedStart || old >= movedEnd) return false
                val moved = old + delta
                if (moved < 0) return false
                if (width == 4) {
                    if (moved > 0xFFFF_FFFFL) return false
                    buffer.putInt(at, moved.toInt())
                } else {
                    buffer.putLong(at, moved)
                }
            }
            return true
        }
    }

    /** The top-level boxes, or null when they do not tile the file exactly. */
    internal fun topLevel(channel: FileChannel): List<Box>? {
        val length = channel.size()
        val boxes = mutableListOf<Box>()
        val header = ByteBuffer.allocate(16)
        var at = 0L
        while (at < length) {
            if (length - at < 8) return null
            header.clear().limit(8)
            readFully(channel, header, at)
            val size32 = header.getInt(0).toLong() and 0xFFFF_FFFFL
            val type = fourCc(header, 4)
            val size = when (size32) {
                0L -> length - at
                1L -> {
                    if (length - at < 16) return null
                    header.clear().limit(16)
                    readFully(channel, header, at)
                    header.getLong(8).takeIf { it >= 16 } ?: return null
                }
                else -> size32.takeIf { it >= 8 } ?: return null
            }
            if (size > length - at) return null
            boxes += Box(type, at, size)
            at += size
        }
        return boxes
    }

    private fun readFully(channel: FileChannel, buffer: ByteBuffer, position: Long) {
        var at = position
        while (buffer.hasRemaining()) {
            val read = channel.read(buffer, at)
            if (read < 0) throw IOException("unexpected end of file at $at")
            at += read
        }
    }

    /** transferTo may move fewer bytes than asked; this does not return until all of them have. */
    private fun copy(from: FileChannel, position: Long, count: Long, to: FileChannel) {
        var done = 0L
        while (done < count) {
            val moved = from.transferTo(position + done, count - done, to)
            if (moved <= 0) throw IOException("could not copy at ${position + done}")
            done += moved
        }
    }

    private fun fourCc(buffer: ByteBuffer, at: Int): String =
        String(ByteArray(4) { buffer.get(at + it) }, Charsets.ISO_8859_1)

    /** Boxes on the path from `moov` down to a sample table. Nothing else holds a chunk offset. */
    private val CONTAINERS = setOf("moov", "trak", "mdia", "minf", "stbl")

    /** Boxes that are only space, and may be dropped. */
    private val PADDING = setOf("free", "skip")

    /**
     * A `moov` is held in memory while its offsets are shifted. One for a
     * 100 MB clip is well under a megabyte; one past this is not a file this
     * client produced.
     */
    private const val MAX_MOOV_BYTES = 32L * 1024 * 1024
}
