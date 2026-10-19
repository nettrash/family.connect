/*
 * AnimationRoutingTest.kt
 * Family Connect (Android)
 *
 * Which door makes what of an animated picture (#96; docs/protocol.md,
 * "Animations" → "What becomes one"):
 *
 *  - the PHOTO PICKER: an animated GIF, WebP or PNG becomes an animation, a
 *    STILL GIF or WebP (or a BMP) a file — it used to be flattened to a JPEG,
 *    the bug the audit found — and a JPEG or a still PNG the photo it was;
 *  - the FILES, the CLIPBOARD and a SHARE: an animated picture becomes an
 *    animation, and anything else is the door's own business (null);
 *  - rule C: an animation that cannot be made goes exactly as it went before
 *    #96 — from the picker a GIF or WebP as a file and an animated PNG as a
 *    photo; from the other doors, whatever that door did (null here).
 *
 * MediaPrep is real, over Robolectric's ContentResolver; the encoder is a
 * fake (the real one needs a codec — MediaPrepDeviceTest's territory), so
 * this pins the ROUTING and what the composer is handed.
 */

package me.nettrash.familyconnect.data.repo

import android.graphics.Bitmap
import android.net.Uri
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.runBlocking
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.testutil.AnimatedFixtures
import me.nettrash.familyconnect.testutil.AnimatedFixtures.GifImage
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.GraphicsMode
import java.io.ByteArrayOutputStream
import java.io.File

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class AnimationRoutingTest {

    private val context = RuntimeEnvironment.getApplication()

    /** A maker that records what it was asked and answers as scripted. */
    private class FakeMaker(var works: Boolean = true) : AnimationMaker {
        val asked = mutableListOf<ByteArray>()
        override suspend fun make(source: ByteArray, output: File): AnimationMaker.Made? {
            asked += source
            if (!works) return null
            output.writeBytes(byteArrayOf(0, 0, 0, 24, 'f'.code.toByte(), 't'.code.toByte(), 'y'.code.toByte(), 'p'.code.toByte()))
            return AnimationMaker.Made(width = 320, height = 240, durationMs = 400, poster = byteArrayOf(1, 2, 3))
        }
    }

    private val maker = FakeMaker()
    private val mediaPrep = MediaPrep(context, context.contentResolver, maker)

    private val palette = intArrayOf(0xFF0000, 0x0000FF)
    private val animatedGif = AnimatedFixtures.gif(
        4, 4, palette,
        listOf(GifImage(0, 0, 4, 4, ByteArray(16)), GifImage(0, 0, 4, 4, ByteArray(16) { 1 })),
    )
    private val stillGif = AnimatedFixtures.gif(4, 4, palette, listOf(GifImage(0, 0, 4, 4, ByteArray(16))))

    private fun item(name: String, bytes: ByteArray): Uri =
        Uri.parse("content://me.nettrash.test.provider/items/$name").also {
            shadowOf(context.contentResolver).registerInputStreamSupplier(it) { bytes.inputStream() }
        }

    private fun bitmap(alpha: Boolean = false) = Bitmap.createBitmap(8, 6, Bitmap.Config.ARGB_8888).apply {
        eraseColor(0xFF336699.toInt())
        setHasAlpha(alpha)
    }

    private fun encoded(format: Bitmap.CompressFormat): ByteArray = ByteArrayOutputStream().use {
        bitmap().compress(format, 90, it)
        it.toByteArray()
    }

    // -- The photo picker ------------------------------------------------------------

    @Test
    fun `an animated gif from the picker becomes an animation`(): Unit = runBlocking {
        val prepared = mediaPrep.preparePickedPicture(item("dance.gif", animatedGif))

        assertThat(prepared.animation).isTrue()
        assertThat(prepared.kind).isEqualTo(AttachmentDto.KIND_VIDEO)
        assertThat(prepared.mime).isEqualTo("video/mp4")
        assertThat(prepared.width).isEqualTo(320)
        assertThat(prepared.height).isEqualTo(240)
        assertThat(prepared.durationMs).isEqualTo(400)
        assertThat(prepared.previewJpeg).isEqualTo(byteArrayOf(1, 2, 3))
        // Made of the picture's own bytes.
        assertThat(maker.asked.single()).isEqualTo(animatedGif)
        prepared.file.delete()
    }

    @Test
    fun `a still gif from the picker goes as a file, its bytes untouched — not a jpeg`(): Unit = runBlocking {
        val prepared = mediaPrep.preparePickedPicture(item("still.gif", stillGif))

        assertThat(prepared.animation).isFalse()
        assertThat(prepared.kind).isEqualTo(AttachmentDto.KIND_FILE)
        assertThat(prepared.name).isEqualTo("still.gif")
        assertThat(prepared.file.readBytes()).isEqualTo(stillGif)
        assertThat(maker.asked).isEmpty()
        prepared.file.delete()
    }

    @Test
    fun `a still webp from the picker goes as a file`(): Unit = runBlocking {
        val webp = ByteArray(40).also {
            "RIFF".toByteArray().copyInto(it, 0)
            "WEBP".toByteArray().copyInto(it, 8)
            "VP8L".toByteArray().copyInto(it, 12)
        }
        val prepared = mediaPrep.preparePickedPicture(item("still.webp", webp))
        assertThat(prepared.kind).isEqualTo(AttachmentDto.KIND_FILE)
        assertThat(prepared.file.readBytes()).isEqualTo(webp)
        prepared.file.delete()
    }

    @Test
    fun `a jpeg and a still png from the picker are photos, as before`(): Unit = runBlocking {
        for ((name, bytes) in listOf("p.jpg" to encoded(Bitmap.CompressFormat.JPEG), "p.png" to encoded(Bitmap.CompressFormat.PNG))) {
            val prepared = mediaPrep.preparePickedPicture(item(name, bytes))
            assertThat(prepared.kind).isEqualTo(AttachmentDto.KIND_PHOTO)
            assertThat(prepared.mime).isEqualTo("image/jpeg")
            assertThat(prepared.animation).isFalse()
            prepared.file.delete()
        }
        assertThat(maker.asked).isEmpty()
    }

    @Test
    fun `an animation that cannot be made goes as its own bytes, a file`(): Unit = runBlocking {
        maker.works = false
        val prepared = mediaPrep.preparePickedPicture(item("broken.gif", animatedGif))

        assertThat(prepared.kind).isEqualTo(AttachmentDto.KIND_FILE)
        assertThat(prepared.animation).isFalse()
        assertThat(prepared.file.readBytes()).isEqualTo(animatedGif)
        assertThat(maker.asked).hasSize(1)
        prepared.file.delete()
    }

    // -- Files, the clipboard, a share --------------------------------------------------

    @Test
    fun `from the other doors only an animated picture is this path's business`(): Unit = runBlocking {
        assertThat(mediaPrep.prepareIfAnimated(item("still.gif", stillGif))).isNull()
        assertThat(mediaPrep.prepareIfAnimated(item("report.pdf", "%PDF-1.7 hello".toByteArray()))).isNull()
        assertThat(mediaPrep.prepareIfAnimated(item("p.png", encoded(Bitmap.CompressFormat.PNG)))).isNull()
        assertThat(maker.asked).isEmpty()

        val prepared = requireNotNull(mediaPrep.prepareIfAnimated(item("dance.gif", animatedGif)))
        assertThat(prepared.animation).isTrue()
        prepared.file.delete()
    }

    @Test
    fun `an animated png that cannot be made goes as the photo it went as before`(): Unit = runBlocking {
        val frame = bitmap(alpha = true)
        val apng = requireNotNull(
            AnimatedFixtures.apng(8, 6, listOf(AnimatedFixtures.ApngFrame(frame), AnimatedFixtures.ApngFrame(frame))),
        )
        maker.works = false

        val picked = mediaPrep.preparePickedPicture(item("moving.png", apng))
        assertThat(picked.kind).isEqualTo(AttachmentDto.KIND_PHOTO)
        assertThat(picked.mime).isEqualTo("image/jpeg")
        assertThat(picked.animation).isFalse()
        picked.file.delete()

        // From the other doors: null, and the door's own path runs as it always did.
        assertThat(mediaPrep.prepareIfAnimated(item("1000042", apng))).isNull()
        assertThat(mediaPrep.prepareIfAnimated(item("broken.gif", animatedGif))).isNull()
        assertThat(maker.asked).hasSize(3)
    }

    @Test
    fun `a shared animated gif becomes an animation`(): Unit = runBlocking {
        val importer = DefaultShareImporter(context, mediaPrep)
        val prepared = importer.prepare(listOf(item("shared.gif", animatedGif)))

        assertThat(prepared).hasSize(1)
        assertThat(prepared.single().animation).isTrue()
        prepared.single().file.delete()
    }
}
