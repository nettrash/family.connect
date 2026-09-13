/*
 * MediaTypeHonestyTest.kt
 * Family Connect (Android)
 *
 * WHAT THIS CLIENT IS WILLING TO CLAIM ABOUT BYTES IT HAS NOT LOOKED AT.
 *
 * The server verifies that a declared type matches what the bytes ARE for
 * every photo, video and audio upload, and refuses the whole upload when it
 * does not (docs/protocol.md). So a client that types a file by its
 * extension - or by what a content provider says - can be wrong in a way the
 * sender cannot act on: the send just fails, again, for that file.
 *
 * The one that bit here is `.aac`. A raw ADTS stream is not ISO base media,
 * `audio/aac` is in SENDABLE_AUDIO_TYPES so the gate lets it through, and the
 * extension table then calls it `audio/mp4` - which is a 400 every time. The
 * video path needs no such check: it transcodes anything outside
 * SENDABLE_VIDEO_TYPES rather than relabelling it.
 *
 * iOS counterpart: FamilyConnectTests/MediaTypeHonestyTests.swift, which
 * pins the same table and the same fallback.
 */

package me.nettrash.familyconnect.data.repo

import android.net.Uri
import com.google.common.truth.Truth.assertThat
import java.io.File
import kotlin.io.path.createTempFile
import kotlinx.coroutines.runBlocking
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf

@RunWith(RobolectricTestRunner::class)
class MediaTypeHonestyTest {

    private val context = RuntimeEnvironment.getApplication()
    private val mediaPrep = MediaPrep(context, context.contentResolver)

    /** Bytes behind a Uri, the way a provider would serve them. */
    private fun item(uri: String, bytes: ByteArray): Uri =
        Uri.parse(uri).also {
            shadowOf(context.contentResolver).registerInputStreamSupplier(it) { bytes.inputStream() }
        }

    /** An ISO base media header: any brand will do, the check is `ftyp` at 4. */
    private val isoBaseMedia = byteArrayOf(
        0x00, 0x00, 0x00, 0x18,
        0x66, 0x74, 0x79, 0x70,
        0x6D, 0x70, 0x34, 0x32,
    )

    /** Matroska's own signature - an EBML header. */
    private val matroska = byteArrayOf(
        0x1A, 0x45, 0xDF.toByte(), 0xA3.toByte(),
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x23,
    )

    /** A raw AAC frame: ADTS sync, which is what most `.aac` files are. */
    private val adts = byteArrayOf(
        0xFF.toByte(), 0xF1.toByte(), 0x50, 0x80.toByte(),
        0x00, 0x1F, 0xFC.toByte(), 0x00, 0x00, 0x00, 0x00, 0x00,
    )

    @Test
    fun `the magic table answers what the server's answers`() {
        assertThat(MediaPrep.Magic.matches("video/mp4", isoBaseMedia)).isTrue()
        assertThat(MediaPrep.Magic.matches("audio/mp4", isoBaseMedia)).isTrue()
        assertThat(MediaPrep.Magic.matches("image/heic", isoBaseMedia)).isTrue()
        assertThat(MediaPrep.Magic.matches("video/mp4", matroska)).isFalse()

        // THE ONE THAT MATTERED: raw AAC is not an MP4 container.
        assertThat(MediaPrep.Magic.matches("audio/mp4", adts)).isFalse()

        assertThat(
            MediaPrep.Magic.matches(
                "image/jpeg",
                byteArrayOf(0xFF.toByte(), 0xD8.toByte(), 0xFF.toByte(), 0xE0.toByte()),
            ),
        ).isTrue()
        assertThat(
            MediaPrep.Magic.matches(
                "image/png",
                byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A),
            ),
        ).isTrue()
        assertThat(MediaPrep.Magic.matches("audio/mpeg", "ID3".toByteArray())).isTrue()
        assertThat(
            MediaPrep.Magic.matches("audio/mpeg", byteArrayOf(0xFF.toByte(), 0xFB.toByte())),
        ).isTrue()
        assertThat(
            MediaPrep.Magic.matches(
                "audio/wav",
                "RIFF".toByteArray() + byteArrayOf(0, 0, 0, 0) + "WAVE".toByteArray(),
            ),
        ).isTrue()
        assertThat(MediaPrep.Magic.matches("audio/ogg", "OggS".toByteArray())).isTrue()

        // A type the SERVER would not take is never honest to claim, whatever the bytes are.
        assertThat(MediaPrep.Magic.matches("video/x-matroska", matroska)).isFalse()
        assertThat(MediaPrep.Magic.matches("audio/aac", adts)).isFalse()
        // And too few bytes to judge is not a pass.
        assertThat(MediaPrep.Magic.matches("video/mp4", byteArrayOf(0x00, 0x00))).isFalse()
    }

    @Test
    fun `a file on disk is judged by its bytes and an unreadable one by nothing`() {
        val real = createTempFile(suffix = ".m4a").toFile()
        real.writeBytes(isoBaseMedia)
        val raw = createTempFile(suffix = ".aac").toFile()
        raw.writeBytes(adts)
        val missing = File(real.parent, "nothing-here.m4a")

        assertThat(MediaPrep.Magic.honest(real, "audio/mp4")).isTrue()
        assertThat(MediaPrep.Magic.honest(raw, "audio/mp4")).isFalse()
        // A file nobody can read is not honest to claim anything about: the send would fail
        // anyway, and the file path is the answer either way.
        assertThat(MediaPrep.Magic.honest(missing, "audio/mp4")).isFalse()

        real.delete()
        raw.delete()
    }

    /**
     * THE REGRESSION. A raw `.aac` reaches the audio path — `audio/aac` is in
     * SENDABLE_AUDIO_TYPES, so the gate lets it through — and the extension table would call it
     * `audio/mp4`. It goes as a FILE instead, which is the one thing the server does not verify,
     * so the family gets the sound rather than a send that fails for ever.
     */
    @Test
    fun `a raw aac file is prepared as a file rather than as refused audio`(): Unit = runBlocking {
        val prepared = mediaPrep.prepareAudio(
            item("content://media/external/audio/media/91", adts),
            declaredMime = "audio/aac",
            fallbackName = "Pasted sound.aac",
        )

        assertThat(prepared.kind).isEqualTo(AttachmentDto.KIND_FILE)
        assertThat(prepared.mime).isNotEqualTo("audio/mp4")
        // `kind=file` is refused without a name, and a file's name is its whole identity.
        assertThat(prepared.name).isEqualTo("Pasted sound.aac")
        // Nothing to look at, and no duration claimed for something not being sent as audio.
        assertThat(prepared.previewJpeg).isNull()
        assertThat(prepared.durationMs).isNull()
        prepared.file.delete()
    }

    /** An m4a that really is one keeps its player, through the same path. */
    @Test
    fun `a real m4a is still prepared as audio`(): Unit = runBlocking {
        val prepared = mediaPrep.prepareAudio(
            item("content://media/external/audio/media/92", isoBaseMedia),
            declaredMime = "audio/mp4",
            fallbackName = "Pasted sound.m4a",
        )

        assertThat(prepared.kind).isEqualTo(AttachmentDto.KIND_AUDIO)
        assertThat(prepared.mime).isEqualTo("audio/mp4")
        prepared.file.delete()
    }

    /** The two lists are the server's, and drift in either is a refused upload. */
    @Test
    fun `the sendable lists still name what the server accepts`() {
        assertThat(MediaPrep.SENDABLE_VIDEO_TYPES)
            .containsExactly("video/mp4", "video/quicktime")
        // `audio/aac` and `audio/mp3` are PROVIDER names, kept because a provider says them -
        // what goes on the wire is normalised by extension, and the bytes are checked before it
        // is claimed.
        assertThat(MediaPrep.SENDABLE_AUDIO_TYPES).containsAtLeast("audio/mp4", "audio/mpeg")
    }
}
