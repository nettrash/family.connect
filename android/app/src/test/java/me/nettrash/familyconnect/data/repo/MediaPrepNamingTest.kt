/*
 * MediaPrepNamingTest.kt
 * Family Connect (Android)
 *
 * What an attachment is CALLED and what type it claims to be, for an item
 * nobody picked from a picker.
 *
 * A pasted item usually arrives with neither: its Uri tail is an opaque id
 * and its provider answers nothing. `kind=file` is refused outright
 * without a name of 1–255 characters (docs/protocol.md, "Files"), and a
 * PDF uploaded as `application/octet-stream` is a document the recipient's
 * phone will not know how to open — so both have a fallback, and both
 * fallbacks have to lose to anything the item says about itself.
 *
 * runBlocking rather than runTest: MediaPrep copies on Dispatchers.IO, and
 * these tests want the real thing to finish rather than a virtual clock to
 * advance.
 */

package me.nettrash.familyconnect.data.repo

import android.net.Uri
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.runBlocking
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf

@RunWith(RobolectricTestRunner::class)
class MediaPrepNamingTest {

    private val context = RuntimeEnvironment.getApplication()
    private val mediaPrep = MediaPrep(context, context.contentResolver)

    private companion object {
        /** An ID3 tag: what the server's magic check wants of an `audio/mpeg` upload. */
        val MP3_HEAD = "ID3".toByteArray() + ByteArray(29) { 0 }
    }

    /** Bytes behind a Uri, the way a provider would serve them. */
    private fun item(uri: String, bytes: ByteArray = ByteArray(32) { 9 }): Uri =
        Uri.parse(uri).also {
            shadowOf(context.contentResolver).registerInputStreamSupplier(it) { bytes.inputStream() }
        }

    /**
     * The pasted case: an opaque id for a Uri, a provider that says
     * nothing, and a clip that knows what it is.
     */
    @Test
    fun `a pasted item with no name of its own takes the synthesised one`(): Unit = runBlocking {
        val prepared = mediaPrep.prepareFile(
            item("content://media/external/images/media/1000000042"),
            declaredMime = "image/gif",
            fallbackName = "Pasted image.gif",
        )

        assertThat(prepared.kind).isEqualTo(AttachmentDto.KIND_FILE)
        assertThat(prepared.name).isEqualTo("Pasted image.gif")
        // NOT application/octet-stream: the clipboard knew, so the
        // recipient's phone gets a type it can act on.
        assertThat(prepared.mime).isEqualTo("image/gif")
        // The cache file is this device's business; its name never travels.
        assertThat(prepared.file.name).startsWith("upload-")
        prepared.file.delete()
    }

    /** A name the item carries beats anything synthesised for it. */
    @Test
    fun `a name the item already carries wins`(): Unit = runBlocking {
        val prepared = mediaPrep.prepareFile(
            item("content://me.nettrash.familyconnect.fileprovider/uploads/Report.pdf"),
            declaredMime = "application/pdf",
            fallbackName = "Pasted file.pdf",
        )

        assertThat(prepared.name).isEqualTo("Report.pdf")
        prepared.file.delete()
    }

    /**
     * An id is not a name. This is the one the fallback exists for: before
     * it, a pasted document arrived called "1000000042".
     */
    @Test
    fun `an opaque id is never what the family sees`(): Unit = runBlocking {
        val prepared = mediaPrep.prepareFile(
            item("content://com.example.docs/document/msf%3A1000000042"),
            declaredMime = "application/pdf",
            fallbackName = "Pasted file.pdf",
        )

        assertThat(prepared.name).isEqualTo("Pasted file.pdf")
        prepared.file.delete()
    }

    /** With nothing to go on at all, the old behaviour is unchanged. */
    @Test
    fun `a picked document with no hints keeps the old chain`(): Unit = runBlocking {
        val prepared = mediaPrep.prepareFile(item("content://com.example.docs/Invoice.pdf"))

        assertThat(prepared.name).isEqualTo("Invoice.pdf")
        assertThat(prepared.mime).isEqualTo(MediaPrep.DEFAULT_FILE_MIME)
        prepared.file.delete()
    }

    /**
     * Audio keeps its player, and a synthesised name gives it its type.
     *
     * The bytes are a real ID3 header, because the claim is now CHECKED against them: an audio
     * upload whose bytes are not what it calls them is sent as a file instead of being refused by
     * the server (MediaPrep.Magic, and MediaTypeHonestyTest).
     */
    @Test
    fun `pasted audio is named and typed from what the clipboard said`(): Unit = runBlocking {
        val prepared = mediaPrep.prepareAudio(
            item("content://media/external/audio/media/77", MP3_HEAD),
            declaredMime = "audio/mpeg",
            fallbackName = "Pasted sound.mp3",
        )

        assertThat(prepared.kind).isEqualTo(AttachmentDto.KIND_AUDIO)
        assertThat(prepared.name).isEqualTo("Pasted sound.mp3")
        assertThat(prepared.mime).isEqualTo("audio/mpeg")
        // Audio has nothing to look at (docs/protocol.md, "Audio").
        assertThat(prepared.previewJpeg).isNull()
        prepared.file.delete()
    }

    /**
     * A voice note travels with NO name (#79, Decision 35). The server drops
     * an audio upload's name anyway, and until the echo arrived the sender's
     * own chat list showed the recorder's "voice-<ms>.m4a" — this device's
     * business, not the family's. A sound file picked from disk keeps its
     * title; only a recording is told apart, by its flag.
     */
    @Test
    fun `a voice note carries no name, and says it is one`(): Unit = runBlocking {
        val recording = java.io.File(context.cacheDir, "voice-1786795200000.m4a").apply {
            writeBytes(byteArrayOf(0, 0, 0, 0x20) + "ftypM4A ".toByteArray() + ByteArray(2048) { 5 })
        }

        val prepared = mediaPrep.prepareAudio(Uri.fromFile(recording), voiceNote = true)

        assertThat(prepared.kind).isEqualTo(AttachmentDto.KIND_AUDIO)
        assertThat(prepared.mime).isEqualTo("audio/mp4")
        assertThat(prepared.name).isNull()
        assertThat(prepared.voiceNote).isTrue()
        prepared.file.delete()
        recording.delete()
    }

    /** …while the same bytes picked from disk keep the name they were picked by. */
    @Test
    fun `a picked sound file keeps its name and is no voice note`(): Unit = runBlocking {
        val track = java.io.File(context.cacheDir, "Lullaby.m4a").apply {
            writeBytes(byteArrayOf(0, 0, 0, 0x20) + "ftypM4A ".toByteArray() + ByteArray(2048) { 5 })
        }

        val prepared = mediaPrep.prepareAudio(Uri.fromFile(track))

        assertThat(prepared.name).isEqualTo("Lullaby.m4a")
        assertThat(prepared.voiceNote).isFalse()
        prepared.file.delete()
        track.delete()
    }

    /**
     * One ceiling for everything, pasted or picked — and the same failure,
     * so the message the composer shows is one that is already translated.
     */
    @Test
    fun `a pasted item over the ceiling is refused like any other`(): Unit = runBlocking {
        try {
            mediaPrep.prepareFile(
                item("content://com.example.docs/huge", ByteArray(64) { 1 }),
                limit = 32,
                declaredMime = "application/pdf",
                fallbackName = "Pasted file.pdf",
            )
            throw AssertionError("expected the ceiling to refuse it")
        } catch (expected: MediaPrep.TooLargeAfterCompression) {
            assertThat(expected).isNotNull()
        }
    }
}
