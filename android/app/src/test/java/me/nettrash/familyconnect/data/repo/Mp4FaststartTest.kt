/*
 * Mp4FaststartTest.kt
 * Family Connect (Android)
 *
 * `moov` before `mdat` (docs/protocol.md, "Preparing media before upload"),
 * on MP4s built box by box here rather than recorded: what matters is the
 * box LAYOUT a muxer leaves — `ftyp`, the `free` Media3 reserved, `mdat`,
 * then a `moov` that did not fit — and the one invariant a move has to keep,
 * which is that every chunk offset still points at the same bytes.
 *
 * Plain JVM: Mp4Faststart is java.io and java.nio only.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.RandomAccessFile
import java.nio.ByteBuffer
import kotlin.io.path.createTempDirectory
import org.junit.After
import org.junit.Test

class Mp4FaststartTest {

    private val directory: File = createTempDirectory("faststart").toFile()

    @After
    fun cleanUp() {
        directory.deleteRecursively()
    }

    /** The samples: every byte different, so an offset that moved wrongly reads the wrong ones. */
    private val samples = ByteArray(1_000) { (it * 7 + 3).toByte() }

    /** Where each "chunk" starts inside [samples]. */
    private val chunks = listOf(0, 96, 250, 511, 777, 996)

    @Test
    fun `a moov at the end is moved to the front and every chunk follows it`() {
        val file = write(
            box("ftyp", "isom".ascii() + ByteArray(4) + "isommp42".ascii()),
            // The 400 000 bytes Media3 reserves for a moov that then did not fit, in miniature.
            box("free", ByteArray(400)),
            mdat = box("mdat", samples),
            moov = { mdatPayload -> moov(stco(chunks.map { mdatPayload + it })) },
        )
        val before = file.readBytes()
        val offsetsBefore = chunkOffsets(before)
        assertThat(Mp4Faststart.moovFirst(file)).isFalse()

        assertThat(Mp4Faststart.apply(file)).isTrue()

        val after = file.readBytes()
        assertThat(types(file)).containsExactly("ftyp", "moov", "mdat").inOrder()
        assertThat(Mp4Faststart.moovFirst(file)).isTrue()
        // The reserved space is gone, and nothing else is.
        assertThat(after.size).isEqualTo(before.size - 408)
        assertSameChunks(before, offsetsBefore, after, chunkOffsets(after))
        assertThat(directory.list()!!.toList()).containsExactly(file.name)
    }

    /**
     * Media3 writes `mdat` with a 64-bit size, and a long clip's table is
     * `co64`. Two tracks, so the walk has to find both tables.
     */
    @Test
    fun `a 64-bit mdat and co64 tables in two tracks move too`() {
        val file = write(
            box("ftyp", "mp42".ascii() + ByteArray(4)),
            mdat = largeBox("mdat", samples),
            moov = { mdatPayload ->
                moov(
                    co64(chunks.take(3).map { mdatPayload + it }),
                    co64(chunks.drop(3).map { mdatPayload + it }),
                )
            },
        )
        val before = file.readBytes()
        val offsetsBefore = chunkOffsets(before)
        assertThat(offsetsBefore).hasSize(chunks.size)

        assertThat(Mp4Faststart.apply(file)).isTrue()

        val after = file.readBytes()
        assertThat(types(file)).containsExactly("ftyp", "moov", "mdat").inOrder()
        assertThat(after.size).isEqualTo(before.size)
        assertSameChunks(before, offsetsBefore, after, chunkOffsets(after))
    }

    @Test
    fun `a file that is already moov-first is left byte for byte`() {
        val ftyp = box("ftyp", "isom".ascii() + ByteArray(4))
        // Offsets are wherever the mdat payload ends up: after ftyp, moov and the mdat header.
        val probe = moov(stco(chunks.map { 0L }))
        val payloadAt = (ftyp.size + probe.size + 8).toLong()
        val bytes = ftyp + moov(stco(chunks.map { payloadAt + it })) + box("mdat", samples)
        val file = File(directory, "kept.mp4").apply { writeBytes(bytes) }

        assertThat(Mp4Faststart.moovFirst(file)).isTrue()
        assertThat(Mp4Faststart.apply(file)).isTrue()
        assertThat(file.readBytes()).isEqualTo(bytes)
    }

    /**
     * Something other than padding after the moov would move by a different
     * amount than the mdat does. Not chased: the file keeps its moov at the
     * end, which still plays.
     */
    @Test
    fun `a box after the moov leaves the file exactly as it was`() {
        val file = write(
            box("ftyp", "isom".ascii() + ByteArray(4)),
            mdat = box("mdat", samples),
            moov = { mdatPayload -> moov(stco(chunks.map { mdatPayload + it })) },
            trailer = box("uuid", ByteArray(24)),
        )
        val before = file.readBytes()

        assertThat(Mp4Faststart.apply(file)).isFalse()
        assertThat(file.readBytes()).isEqualTo(before)
        assertThat(directory.list()!!.toList()).containsExactly(file.name)
    }

    /** An offset into the ftyp is not one the move can follow — refused, not guessed at. */
    @Test
    fun `an offset outside the mdat leaves the file exactly as it was`() {
        val file = write(
            box("ftyp", "isom".ascii() + ByteArray(4)),
            mdat = box("mdat", samples),
            moov = { mdatPayload -> moov(stco(listOf(mdatPayload, 4L))) },
        )
        val before = file.readBytes()

        assertThat(Mp4Faststart.apply(file)).isFalse()
        assertThat(file.readBytes()).isEqualTo(before)
        assertThat(directory.list()!!.toList()).containsExactly(file.name)
    }

    @Test
    fun `bytes that are not an MP4 are left alone and said to be unknown`() {
        val junk = File(directory, "junk.mp4").apply { writeBytes(ByteArray(64) { 0x7F }) }
        val truncated = File(directory, "truncated.mp4").apply {
            // A box that claims more bytes than the file has.
            writeBytes(ByteBuffer.allocate(12).putInt(4_000).put("ftyp".ascii()).putInt(0).array())
        }
        for (file in listOf(junk, truncated)) {
            val before = file.readBytes()
            assertThat(Mp4Faststart.moovFirst(file)).isNull()
            assertThat(Mp4Faststart.apply(file)).isFalse()
            assertThat(file.readBytes()).isEqualTo(before)
        }
    }

    // -- Building and reading boxes -----------------------------------------------

    /**
     * An MP4 with its moov LAST, the way a muxer that ran out of reserved
     * space leaves it. [moov] is given the file offset of the mdat's payload,
     * so its table can point into it.
     */
    private fun write(
        vararg head: ByteArray,
        mdat: ByteArray,
        moov: (mdatPayload: Long) -> ByteArray,
        trailer: ByteArray = ByteArray(0),
    ): File {
        val headSize = head.sumOf { it.size }
        val mdatHeader = mdat.size - samples.size
        val bytes = ByteArrayOutputStream().apply {
            head.forEach { write(it) }
            write(mdat)
            write(moov((headSize + mdatHeader).toLong()))
            write(trailer)
        }.toByteArray()
        return File(directory, "clip.mp4").apply { writeBytes(bytes) }
    }

    private fun moov(vararg tables: ByteArray): ByteArray = box(
        "moov",
        box("mvhd", ByteArray(100)),
        *tables.map { table ->
            box(
                "trak",
                box("tkhd", ByteArray(84)),
                box(
                    "mdia",
                    box("mdhd", ByteArray(24)),
                    box("minf", box("stbl", box("stsd", ByteArray(16)), table)),
                ),
            )
        }.toTypedArray(),
    )

    private fun stco(offsets: List<Long>): ByteArray = box(
        "stco",
        ByteBuffer.allocate(8 + 4 * offsets.size).apply {
            putInt(0)
            putInt(offsets.size)
            offsets.forEach { putInt(it.toInt()) }
        }.array(),
    )

    private fun co64(offsets: List<Long>): ByteArray = box(
        "co64",
        ByteBuffer.allocate(8 + 8 * offsets.size).apply {
            putInt(0)
            putInt(offsets.size)
            offsets.forEach { putLong(it) }
        }.array(),
    )

    private fun box(type: String, vararg payload: ByteArray): ByteArray {
        val body = payload.fold(ByteArray(0)) { all, part -> all + part }
        return ByteBuffer.allocate(8 + body.size).putInt(8 + body.size).put(type.ascii()).put(body).array()
    }

    /** size = 1, then the real size in 64 bits: how Media3 writes an mdat. */
    private fun largeBox(type: String, payload: ByteArray): ByteArray =
        ByteBuffer.allocate(16 + payload.size)
            .putInt(1).put(type.ascii()).putLong((16 + payload.size).toLong()).put(payload).array()

    private fun String.ascii(): ByteArray = toByteArray(Charsets.US_ASCII)

    private fun types(file: File): List<String> =
        RandomAccessFile(file, "r").use { access ->
            requireNotNull(Mp4Faststart.topLevel(access.channel)).map { it.type }
        }

    /** Every chunk offset in the file's stco/co64 tables, in order, found by scanning for the tags. */
    private fun chunkOffsets(bytes: ByteArray): List<Long> {
        val buffer = ByteBuffer.wrap(bytes)
        val offsets = mutableListOf<Long>()
        for (at in 4..bytes.size - 12) {
            val tag = String(bytes, at, 4, Charsets.US_ASCII)
            if (tag != "stco" && tag != "co64") continue
            val count = buffer.getInt(at + 8)
            for (entry in 0 until count) {
                offsets += if (tag == "stco") {
                    buffer.getInt(at + 12 + entry * 4).toLong() and 0xFFFF_FFFFL
                } else {
                    buffer.getLong(at + 12 + entry * 8)
                }
            }
        }
        return offsets
    }

    /** The invariant: the bytes each offset points at are the same bytes, before and after. */
    private fun assertSameChunks(
        before: ByteArray,
        offsetsBefore: List<Long>,
        after: ByteArray,
        offsetsAfter: List<Long>,
    ) {
        assertThat(offsetsAfter).hasSize(offsetsBefore.size)
        offsetsBefore.zip(offsetsAfter).forEach { (old, new) ->
            assertThat(after.copyOfRange(new.toInt(), new.toInt() + 4))
                .isEqualTo(before.copyOfRange(old.toInt(), old.toInt() + 4))
        }
        assertThat(offsetsAfter).isNotEqualTo(offsetsBefore)
    }
}
