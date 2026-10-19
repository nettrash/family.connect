/*
 * Mp4SoundTracks.kt
 * Family Connect (Android)
 *
 * How many sound tracks an MP4 or a QuickTime file has that PLAY — read from
 * the container's own track list, never from a decoder (docs/protocol.md,
 * "Preparing media before upload", Rule E, issue #98).
 *
 * WHY THE CONTAINER AND NOT MEDIA3. Media3's Transformer infers the output's
 * tracks from what ITS extractor can read, and a sound track it cannot read
 * — QuickTime PCM in `in24`/`in32`/`fl32`/`fl64`/`raw `, `apac`, `ima4` —
 * is not listed at all. The export then SUCCEEDS with the picture alone,
 * smaller than its source, and rule D kept it: a video that went up without
 * its sound and nobody was told. MediaExtractor is no better a witness — it
 * is the platform's reader, with its own list of what it cannot read. The
 * `moov` lists every track whatever its codec, so it is the one that is
 * asked: every `trak` whose `mdia`/`hdlr` handler type is `soun`.
 *
 * WHAT "PLAYS" MEANS (the protocol's words):
 *
 *  - an ENABLED track — `tkhd` flags & 0x1;
 *  - of enabled tracks sharing a non-zero `alternate_group`, only the
 *    first: a player picks ONE of them (an iPhone's spatial recording
 *    carries its stereo AAC and its spatial track as such a pair, and
 *    that is one voice, not two);
 *  - a file whose sound tracks are all disabled counts ONE: something
 *    plays it, and it is sound a transcode must not lose.
 *
 * WHICH tracks play is the other half (issue #98, amended): a source with
 * more than one is transcoded with them all MIXED, and Media3 is told which
 * by `tkhd` track_ID — the id its own MP4 extractor gives each track's
 * Format — never by position, since a track its extractor cannot read is
 * simply missing from its list. So each track carries its id, and the
 * channel count and sample rate its sample entry states, which decide the
 * mix's own (MediaTranscode's SoundMix).
 *
 * Null whenever the file cannot be walked — not an ISO base media file, a
 * box that runs past its parent, a `moov` with no `trak` (a compressed
 * `cmov`), a `tkhd` version this does not know. Null means "do as before
 * rule E": a parse failure never blocks a send.
 *
 * Walks boxes the way [Mp4Faststart] does (and through its top-level walk):
 * a 64-bit size when the 32-bit one is 1, "to the end of the parent" when it
 * is 0, and the `moov` wherever it is — in front, or behind the `mdat`.
 *
 * Pure JVM (java.io / java.nio only): Mp4SoundTracksTest builds the boxes.
 */

package me.nettrash.familyconnect.data.repo

import java.io.File
import java.io.RandomAccessFile
import java.nio.ByteBuffer
import java.nio.channels.FileChannel

object Mp4SoundTracks {

    /**
     * One `soun` trak: its `tkhd` (track_ID, enabled, alternate_group) and
     * what its first sample entry states — null where it states nothing this
     * reads (a sample rate past 16 bits in an ISO entry, an entry missing).
     */
    data class SoundTrack(
        val id: Int,
        val enabled: Boolean,
        val alternateGroup: Int,
        val channels: Int? = null,
        val sampleRate: Int? = null,
    )

    /** How many sound tracks of [file] play, or null when it cannot be walked. */
    fun count(file: File): Int? = playing(file)?.size

    /**
     * The same, for a channel that reads at a position — a provider's file
     * descriptor, for a source too big to copy. Never throws.
     */
    fun count(channel: FileChannel): Int? = playing(channel)?.size

    /** The sound tracks of [file] that play, in file order, or null when it cannot be walked. */
    fun playing(file: File): List<SoundTrack>? = runCatching {
        RandomAccessFile(file, "r").use { access -> playing(access.channel) }
    }.getOrNull()

    /** [playing] for a channel that reads at a position. Never throws. */
    fun playing(channel: FileChannel): List<SoundTrack>? =
        runCatching { tracks(channel)?.let(::playingOf) }.getOrNull()

    /**
     * The rule above, on the `soun` traks a walk found: every enabled track
     * in no group, the first enabled track of each group, in file order —
     * and, when none is enabled, the first track, which something plays.
     */
    internal fun playingOf(tracks: List<SoundTrack>): List<SoundTrack> {
        if (tracks.isEmpty()) return emptyList()
        val enabled = tracks.filter { it.enabled }
        if (enabled.isEmpty()) return listOf(tracks.first())
        val seenGroups = mutableSetOf<Int>()
        return enabled.filter { it.alternateGroup == 0 || seenGroups.add(it.alternateGroup) }
    }

    /** Every `soun` trak in the file's `moov`, or null when it cannot be walked. */
    internal fun tracks(channel: FileChannel): List<SoundTrack>? {
        val top = Mp4Faststart.topLevel(channel) ?: return null
        val moov = top.firstOrNull { it.type == "moov" } ?: return null
        if (moov.size > MAX_MOOV_BYTES) return null
        val buffer = ByteBuffer.allocate(moov.size.toInt())
        Mp4Faststart.readFully(channel, buffer, moov.offset)
        val header = if (buffer.getInt(0) == 1) 16 else 8
        val traks = children(buffer, header, buffer.capacity())?.filter { it.type == "trak" } ?: return null
        if (traks.isEmpty()) return null
        val sound = mutableListOf<SoundTrack>()
        for (trak in traks) {
            val parts = children(buffer, trak.body, trak.end) ?: return null
            val mdia = parts.firstOrNull { it.type == "mdia" } ?: return null
            val hdlr = children(buffer, mdia.body, mdia.end)?.firstOrNull { it.type == "hdlr" } ?: return null
            // A full box: version and flags (4), pre_defined — QuickTime's
            // component type (4), then the handler type — its subtype (4).
            if (hdlr.end - hdlr.body < 12) return null
            if (fourCc(buffer, hdlr.body + 8) != "soun") continue
            val tkhd = parts.firstOrNull { it.type == "tkhd" } ?: return null
            val track = soundTrack(buffer, tkhd) ?: return null
            sound += sampleEntry(buffer, mdia)?.let { (channels, rate) ->
                track.copy(channels = channels, sampleRate = rate)
            } ?: track
        }
        return sound
    }

    /**
     * Channels and sample rate from the first entry of `mdia/minf/stbl/stsd`
     * — never fatal: a track whose entry this cannot read still counts, it
     * only states nothing. After the entry's 8-byte header come 6 reserved
     * bytes and a data_reference_index, then the sound fields:
     *
     *  - ISO, and QuickTime versions 0 and 1: version (2), revision (2),
     *    vendor (4), channelcount (2), samplesize (2), two more (4), and the
     *    rate as 16.16 fixed point — whose integer half is all there is;
     *  - QuickTime version 2: the same 20 bytes, then sizeOfStructOnly (4),
     *    the rate as a 64-bit float and the channel count as 32 bits.
     */
    private fun sampleEntry(buffer: ByteBuffer, mdia: Child): Pair<Int?, Int?>? {
        val minf = children(buffer, mdia.body, mdia.end)?.firstOrNull { it.type == "minf" } ?: return null
        val stbl = children(buffer, minf.body, minf.end)?.firstOrNull { it.type == "stbl" } ?: return null
        val stsd = children(buffer, stbl.body, stbl.end)?.firstOrNull { it.type == "stsd" } ?: return null
        // A full box: version and flags (4), entry_count (4), then the entries.
        if (stsd.end - stsd.body < 8) return null
        val entry = children(buffer, stsd.body + 8, stsd.end)?.firstOrNull() ?: return null
        val at = entry.body + 8
        if (entry.end - at < 20) return null
        val version = buffer.getShort(at).toInt() and 0xFFFF
        return if (version == 2) {
            if (entry.end - at < 40) return null
            val rate = buffer.getDouble(at + 24)
            val channels = buffer.getInt(at + 32)
            channels.takeIf { it in 1..MAX_CHANNELS } to
                rate.takeIf { it.isFinite() && it >= 1 && it <= MAX_RATE }?.toInt()
        } else {
            val channels = buffer.getShort(at + 8).toInt() and 0xFFFF
            val rate = (buffer.getInt(at + 16) ushr 16)
            channels.takeIf { it in 1..MAX_CHANNELS } to rate.takeIf { it > 0 }
        }
    }

    /**
     * Enabled and `alternate_group` out of a `tkhd`. Version 0 holds its
     * times and duration in 32 bits, version 1 in 64, so the group sits at
     * a different offset in each: after flags, creation, modification,
     * track_ID, a reserved word, duration, 8 reserved bytes and `layer`.
     */
    private fun soundTrack(buffer: ByteBuffer, tkhd: Child): SoundTrack? {
        val length = tkhd.end - tkhd.body
        if (length < 4) return null
        val version = buffer.get(tkhd.body).toInt() and 0xFF
        val flags = buffer.getInt(tkhd.body) and 0x00FF_FFFF
        val groupAt = when (version) {
            0 -> TKHD_V0_ALTERNATE_GROUP
            1 -> TKHD_V1_ALTERNATE_GROUP
            else -> return null
        }
        if (length < groupAt + 2) return null
        // track_ID follows creation and modification: 32-bit times in
        // version 0, 64-bit in version 1.
        val idAt = if (version == 0) 4 + 4 + 4 else 4 + 8 + 8
        val id = buffer.getInt(tkhd.body + idAt)
        val group = buffer.getShort(tkhd.body + groupAt).toInt() and 0xFFFF
        return SoundTrack(id = id, enabled = flags and TRACK_ENABLED != 0, alternateGroup = group)
    }

    /** A box inside the `moov`: its type, where its payload starts, and where it ends. */
    private class Child(val type: String, val body: Int, val end: Int)

    /** The boxes that tile `[start, end)` exactly, or null when they do not. */
    private fun children(buffer: ByteBuffer, start: Int, end: Int): List<Child>? {
        val boxes = mutableListOf<Child>()
        var at = start
        while (at < end) {
            if (end - at < 8) return null
            val size32 = buffer.getInt(at).toLong() and 0xFFFF_FFFFL
            val type = fourCc(buffer, at + 4)
            var header = 8
            val size = when (size32) {
                0L -> (end - at).toLong()
                1L -> {
                    if (end - at < 16) return null
                    header = 16
                    buffer.getLong(at + 8)
                }
                else -> size32
            }
            if (size < header || size > end - at) return null
            boxes += Child(type, at + header, at + size.toInt())
            at += size.toInt()
        }
        return boxes
    }

    private fun fourCc(buffer: ByteBuffer, at: Int): String =
        String(ByteArray(4) { buffer.get(at + it) }, Charsets.ISO_8859_1)

    /** Past these, a stated value is not one: channel layouts stop at 24, rates at 768 kHz. */
    private const val MAX_CHANNELS = 64
    private const val MAX_RATE = 768_000.0

    /** `tkhd` flag: the track is enabled. */
    private const val TRACK_ENABLED = 0x1

    /** Offsets of `alternate_group` into a `tkhd`'s payload (after the box header). */
    private const val TKHD_V0_ALTERNATE_GROUP = 4 + 4 + 4 + 4 + 4 + 4 + 8 + 2
    private const val TKHD_V1_ALTERNATE_GROUP = 4 + 8 + 8 + 4 + 4 + 8 + 8 + 2

    /** As [Mp4Faststart]'s: a `moov` past this is not one worth holding in memory to count. */
    private const val MAX_MOOV_BYTES = 32L * 1024 * 1024
}
