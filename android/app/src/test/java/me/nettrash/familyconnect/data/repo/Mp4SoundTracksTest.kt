/*
 * Mp4SoundTracksTest.kt
 * Family Connect (Android)
 *
 * Rule E's count (docs/protocol.md, "Preparing media before upload", issue
 * #98): the sound tracks that PLAY, read from the container's own track
 * list, on MP4s built box by box here — what matters is the `trak` layout,
 * `hdlr`'s handler type and `tkhd`'s flags and `alternate_group`, never what
 * the samples decode to.
 *
 * Plain JVM: Mp4SoundTracks is java.io and java.nio only.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import java.io.ByteArrayOutputStream
import java.io.File
import java.nio.ByteBuffer
import kotlin.io.path.createTempDirectory
import org.junit.After
import org.junit.Test

class Mp4SoundTracksTest {

    private val directory: File = createTempDirectory("soundtracks").toFile()

    @After
    fun cleanUp() {
        directory.deleteRecursively()
    }

    @Test
    fun `a file with only a picture has no sound`() {
        assertThat(count(video())).isEqualTo(0)
    }

    @Test
    fun `one enabled sound track is one`() {
        assertThat(count(video(), sound())).isEqualTo(1)
    }

    /** A screen recording's app sound and microphone: two voices. */
    @Test
    fun `two enabled sound tracks are two`() {
        assertThat(count(video(), sound(), sound())).isEqualTo(2)
    }

    /** An iPhone's spatial recording: its stereo AAC and its spatial track, one picked by the player. */
    @Test
    fun `two sound tracks in one alternate group are one`() {
        assertThat(count(video(), sound(group = 1), sound(group = 1))).isEqualTo(1)
        // A second group, and a track in none, are voices of their own.
        assertThat(count(video(), sound(group = 1), sound(group = 1), sound(group = 2), sound())).isEqualTo(3)
    }

    @Test
    fun `a disabled sound track beside an enabled one does not count`() {
        assertThat(count(video(), sound(), sound(enabled = false))).isEqualTo(1)
    }

    /** Something plays it, and a transcode must not lose it. */
    @Test
    fun `sound tracks none of which is enabled count one`() {
        assertThat(count(video(), sound(enabled = false))).isEqualTo(1)
        assertThat(count(video(), sound(enabled = false), sound(enabled = false))).isEqualTo(1)
    }

    /** Version 1 holds its times in 64 bits, and `alternate_group` twelve bytes further on. */
    @Test
    fun `a version 1 tkhd is read at its own offsets`() {
        assertThat(count(video(), sound(version = 1, group = 3), sound(version = 1, group = 3))).isEqualTo(1)
        assertThat(count(video(), sound(version = 1, group = 3), sound(version = 1, group = 4))).isEqualTo(2)
        assertThat(count(video(), sound(version = 1, enabled = false), sound(version = 1))).isEqualTo(1)
    }

    @Test
    fun `boxes with a 64-bit size, and a size of zero, are walked`() {
        val moov = largeBox("moov", box("mvhd", ByteArray(100)) + video() + largeBox("trak", soundBody()) + sound())
        assertThat(count(file(ftyp(), moov, mdat()))).isEqualTo(2)

        // A last box whose size is 0 runs to the end of its parent — here, the file.
        val toEnd = ByteBuffer.allocate(8).putInt(0).put("mdat".ascii()).array() + ByteArray(64)
        assertThat(count(file(ftyp(), moov(video(), sound()), toEnd))).isEqualTo(1)
    }

    @Test
    fun `the moov is found at the end as well as at the start`() {
        val moov = moov(video(), sound(), sound())
        assertThat(count(file(ftyp(), moov, mdat()))).isEqualTo(2)
        assertThat(count(file(ftyp(), box("free", ByteArray(40)), mdat(), moov))).isEqualTo(2)
    }

    /** QuickTime's `mhlr` component type sits where MP4's pre_defined does; the subtype is what counts. */
    @Test
    fun `only the media handler is asked, not the data handler inside minf`() {
        val trak = box(
            "trak",
            tkhd(),
            box(
                "mdia",
                box("mdhd", ByteArray(24)),
                hdlr("soun", component = "mhlr"),
                box("minf", hdlr("alis", component = "dhlr"), box("stbl")),
            ),
        )
        assertThat(count(video(), trak)).isEqualTo(1)
    }

    @Test
    fun `what cannot be walked is null, never a guess`() {
        // Not ISO base media at all.
        assertThat(Mp4SoundTracks.count(write(ByteArray(300) { (it * 31).toByte() }))).isNull()
        // A WebM: EBML, which does not tile as boxes.
        assertThat(Mp4SoundTracks.count(write(byteArrayOf(0x1A, 0x45, 0xDF.toByte(), 0xA3.toByte()) + ByteArray(60)))).isNull()
        // No moov.
        assertThat(count(file(ftyp(), mdat()))).isNull()
        // A moov with no trak — a compressed `cmov`.
        assertThat(count(file(ftyp(), box("moov", box("cmov", ByteArray(20))), mdat()))).isNull()
        // A box that claims more than its parent holds.
        val overrun = ByteBuffer.allocate(16).putInt(4_000).put("trak".ascii()).putLong(0).array()
        assertThat(count(file(ftyp(), box("moov", overrun), mdat()))).isNull()
        // A sound trak with a tkhd too short for its alternate_group, or a version nobody wrote.
        val shortTkhd = box("trak", box("tkhd", ByteArray(20)), mdia("soun"))
        assertThat(count(video(), shortTkhd)).isNull()
        assertThat(count(video(), sound(version = 2))).isNull()
        // Empty, and missing.
        assertThat(Mp4SoundTracks.count(write(ByteArray(0)))).isNull()
        assertThat(Mp4SoundTracks.count(File(directory, "nowhere.mp4"))).isNull()
    }

    @Test
    fun `the rule on what plays, on its own`() {
        fun on(id: Int, group: Int = 0) = Mp4SoundTracks.SoundTrack(id, enabled = true, alternateGroup = group)
        fun off(id: Int) = Mp4SoundTracks.SoundTrack(id, enabled = false, alternateGroup = 0)
        fun ids(vararg tracks: Mp4SoundTracks.SoundTrack) = Mp4SoundTracks.playingOf(tracks.toList()).map { it.id }
        assertThat(ids()).isEmpty()
        // None enabled: the first, which something plays.
        assertThat(ids(off(4), off(5), off(6))).containsExactly(4)
        assertThat(ids(on(2), off(3), on(4))).containsExactly(2, 4).inOrder()
        // Of a group, the first; a later group and a track in none are voices of their own.
        assertThat(ids(on(2, group = 7), on(3, group = 7), off(4), on(5, group = 8), on(6))).containsExactly(2, 5, 6).inOrder()
    }

    /** Which tracks the mix asks Media3 for: by `tkhd` track_ID, the id its extractor gives each Format. */
    @Test
    fun `the playing tracks carry their tkhd ids, in either tkhd version`() {
        val file = file(
            ftyp(),
            moov(video(), sound(id = 7), sound(id = 3, enabled = false), sound(id = 12, version = 1)),
            mdat(),
        )
        assertThat(Mp4SoundTracks.playing(file)!!.map { it.id }).containsExactly(7, 12).inOrder()
        assertThat(Mp4SoundTracks.count(file)).isEqualTo(2)
    }

    @Test
    fun `channels and rate come from the sample entry, ISO or QuickTime version 2`() {
        val file = file(
            ftyp(),
            moov(
                video(),
                sound(id = 2, entry = isoEntry(channels = 1, rate = 44_100)),
                sound(id = 3, entry = quickTimeV2Entry(channels = 6, rate = 96_000.0)),
                // No stsd at all: still a voice, stating nothing.
                sound(id = 4, entry = null),
            ),
            mdat(),
        )
        val tracks = Mp4SoundTracks.playing(file)!!
        assertThat(tracks.map { Triple(it.id, it.channels, it.sampleRate) }).containsExactly(
            Triple(2, 1, 44_100),
            Triple(3, 6, 96_000),
            Triple(4, null, null),
        ).inOrder()
    }

    // -- Building boxes ---------------------------------------------------------

    /** ftyp, moov (mvhd + [traks]), mdat: the layout a camera writes. */
    private fun count(vararg traks: ByteArray): Int? = count(file(ftyp(), moov(*traks), mdat()))

    private fun count(file: File): Int? = Mp4SoundTracks.count(file)

    private fun file(vararg boxes: ByteArray): File =
        write(ByteArrayOutputStream().apply { boxes.forEach { write(it) } }.toByteArray())

    private fun write(bytes: ByteArray): File =
        File(directory, "clip-${bytes.size}-${bytes.contentHashCode()}.mp4").apply { writeBytes(bytes) }

    private fun ftyp(): ByteArray = box("ftyp", "isom".ascii() + ByteArray(4) + "isommp42".ascii())

    private fun mdat(): ByteArray = box("mdat", ByteArray(64) { it.toByte() })

    private fun moov(vararg traks: ByteArray): ByteArray = box("moov", box("mvhd", ByteArray(100)), *traks)

    private fun video(): ByteArray = box("trak", tkhd(id = 1), mdia("vide"))

    private fun sound(
        enabled: Boolean = true,
        group: Int = 0,
        version: Int = 0,
        id: Int = 2,
        entry: ByteArray? = isoEntry(channels = 2, rate = 48_000),
    ): ByteArray = box("trak", soundBody(enabled, group, version, id, entry))

    private fun soundBody(
        enabled: Boolean = true,
        group: Int = 0,
        version: Int = 0,
        id: Int = 2,
        entry: ByteArray? = isoEntry(channels = 2, rate = 48_000),
    ): ByteArray = tkhd(enabled, group, version, id) + mdia("soun", entry)

    private fun mdia(handler: String, entry: ByteArray? = null): ByteArray {
        val stbl = if (entry == null) box("stbl") else box("stbl", box("stsd", ByteArray(4), int(1), entry))
        return box("mdia", box("mdhd", ByteArray(24)), hdlr(handler), box("minf", stbl))
    }

    /** An ISO AudioSampleEntry (`mp4a`): reserved, data_reference_index, then the sound fields. */
    private fun isoEntry(channels: Int, rate: Int): ByteArray = box(
        "mp4a",
        ByteBuffer.allocate(28)
            .put(ByteArray(6)).putShort(1)
            .putShort(0).putShort(0).putInt(0)
            .putShort(channels.toShort()).putShort(16).putShort(0).putShort(0)
            .putInt(rate shl 16)
            .array(),
    )

    /** A QuickTime version 2 sound description: the rate as a double, the channels as 32 bits. */
    private fun quickTimeV2Entry(channels: Int, rate: Double): ByteArray = box(
        "lpcm",
        ByteBuffer.allocate(8 + 20 + 4 + 8 + 4 + 20)
            .put(ByteArray(6)).putShort(1)
            .putShort(2).putShort(0).putInt(0)
            .putShort(3).putShort(16).putShort(-2).putShort(0).putInt(65_536)
            .putInt(72).putDouble(rate).putInt(channels)
            .put(ByteArray(20))
            .array(),
    )

    private fun int(value: Int): ByteArray = ByteBuffer.allocate(4).putInt(value).array()

    /** version/flags, pre_defined (QuickTime: component type), handler type, 12 reserved, an empty name. */
    private fun hdlr(handler: String, component: String? = null): ByteArray = box(
        "hdlr",
        ByteArray(4),
        component?.ascii() ?: ByteArray(4),
        handler.ascii(),
        ByteArray(13),
    )

    /**
     * A `tkhd` as ISO/IEC 14496-12 lays it out: version 0 is 84 bytes of
     * payload, version 1 is 96. Flags 0x7 (enabled, in movie, in preview) or
     * 0x6 for a disabled track. Any other version is written in version 0's
     * shape, for the "version nobody wrote" case.
     */
    private fun tkhd(enabled: Boolean = true, group: Int = 0, version: Int = 0, id: Int = 2): ByteArray {
        val wide = version == 1
        val buffer = ByteBuffer.allocate(if (wide) 96 else 84)
        buffer.putInt((version shl 24) or if (enabled) 0x7 else 0x6)
        if (wide) {
            buffer.putLong(1L).putLong(2L).putInt(id).putInt(0).putLong(1_000L)
        } else {
            buffer.putInt(1).putInt(2).putInt(id).putInt(0).putInt(1_000)
        }
        buffer.putLong(0L) // reserved
        buffer.putShort(0) // layer
        buffer.putShort(group.toShort()) // alternate_group
        buffer.putShort(0x0100) // volume
        return box("tkhd", buffer.array())
    }

    private fun box(type: String, vararg payload: ByteArray): ByteArray {
        val body = payload.fold(ByteArray(0)) { all, part -> all + part }
        return ByteBuffer.allocate(8 + body.size).putInt(8 + body.size).put(type.ascii()).put(body).array()
    }

    /** size = 1, then the real size in 64 bits. */
    private fun largeBox(type: String, payload: ByteArray): ByteArray =
        ByteBuffer.allocate(16 + payload.size)
            .putInt(1).put(type.ascii()).putLong((16 + payload.size).toLong()).put(payload).array()

    private fun String.ascii(): ByteArray = toByteArray(Charsets.US_ASCII)
}
