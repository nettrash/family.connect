/*
 * AttachmentPhotoTransferTest.kt
 * Family Connect (Android)
 *
 * Cache-only still-photo delivery and the bytes the file provider exposes.
 */

package me.nettrash.familyconnect.data.repo

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Color
import androidx.core.content.FileProvider
import androidx.exifinterface.media.ExifInterface
import androidx.test.core.app.ApplicationProvider
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.testutil.AnimatedFixtures
import me.nettrash.familyconnect.testutil.FakeAttachmentApi
import me.nettrash.familyconnect.testutil.FakeConnectivityObserver
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import org.junit.After
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.GraphicsMode
import java.io.File

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class AttachmentPhotoTransferTest {
    private val context = ApplicationProvider.getApplicationContext<Context>()
    private val cache = File(context.cacheDir, AttachmentRepository.CACHE_DIR)
    private val dispatcher = StandardTestDispatcher()
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private val api = FakeAttachmentApi()
    private val repo = AttachmentRepository(context, api, FakeSettingsRepository(), FakeConnectivityObserver(), repoScope)

    @After
    fun tearDown() {
        repoScope.cancel()
        cache.deleteRecursively()
    }

    @Test
    fun `preview only and an evicted full file do not publish or download`() = runTest(dispatcher) {
        val source = writePhoto("25.jpg", Color.BLUE)
        val photo = photo(25, source)
        writePhoto("25-preview.jpg", Color.RED)
        assertThat(repo.hasCachedFullPhoto(photo)).isTrue()
        assertThat(source.delete()).isTrue()

        assertThat(repo.hasCachedFullPhoto(photo)).isFalse()
        assertThat(repo.transferCachedPhoto(photo) { error("No local full photo") }).isFalse()

        // A partial cache file also cannot stand in for the immutable original.
        source.writeBytes(ByteArray(1))
        assertThat(repo.hasCachedFullPhoto(photo)).isFalse()
        assertThat(repo.transferCachedPhoto(photo) { error("Incomplete cache file") }).isFalse()
        assertThat(api.calls).isEmpty()
    }

    @Test
    fun `the copied URI owns a full transparent snapshot without a warm bitmap`() = runTest(dispatcher) {
        val source = writePhoto("25.jpg", Color.GREEN)
        val photo = photo(25, source)
        writePhoto("25-preview.jpg", Color.RED)
        assertThat(repo.cached(photo.id, preview = false)).isNull()

        var snapshot: File? = null
        assertThat(repo.transferCachedPhoto(photo) {
            snapshot = it
            true
        }).isTrue()
        val uri = FileProvider.getUriForFile(context, "${context.packageName}.fileprovider", checkNotNull(snapshot))
        assertThat(context.contentResolver.getType(uri)).isEqualTo("image/png")
        assertThat(source.delete()).isTrue()
        val copy = context.contentResolver.openInputStream(uri).use { BitmapFactory.decodeStream(it) }
        assertThat(copy.width).isEqualTo(80)
        assertThat(copy.height).isEqualTo(48)
        assertThat(copy.getPixel(0, 0)).isEqualTo(Color.TRANSPARENT)
        assertThat(copy.getPixel(20, 20)).isEqualTo(Color.GREEN)
        copy.recycle()
        assertThat(api.calls).isEmpty()
    }

    @Test
    fun `a locally saved full JPEG uses the selected id and bakes its EXIF orientation`() = runTest(dispatcher) {
        writePhoto("25.jpg", Color.RED)
        val source = File(cache, "files/26/synthetic.jpg")
        source.parentFile?.mkdirs()
        val jpeg = Bitmap.createBitmap(60, 40, Bitmap.Config.ARGB_8888).apply { eraseColor(Color.BLUE) }
        source.outputStream().use { jpeg.compress(Bitmap.CompressFormat.JPEG, 95, it) }
        jpeg.recycle()
        ExifInterface(source).apply {
            setAttribute(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_ROTATE_90.toString())
            saveAttributes()
        }
        val photo = photo(26, source).copy(name = "synthetic.jpg", mime = "image/jpeg")

        assertThat(repo.transferCachedPhoto(photo) { file ->
            val copy = BitmapFactory.decodeFile(file.path)
            assertThat(copy.width).isEqualTo(40)
            assertThat(copy.height).isEqualTo(60)
            assertThat(Color.blue(copy.getPixel(10, 10))).isGreaterThan(240)
            copy.recycle()
            true
        }).isTrue()
        assertThat(api.calls).isEmpty()
    }

    @Test
    fun `failed preparation or publication leaves the previous payload readable`() = runTest(dispatcher) {
        val source = writePhoto("25.jpg", Color.BLUE)
        var prior: File? = null
        assertThat(repo.transferCachedPhoto(photo(25, source)) {
            prior = it
            true
        }).isTrue()
        val previous = checkNotNull(prior)
        val bytes = previous.readBytes()

        val corrupt = File(cache, "26.jpg").apply { writeText("no usable pixels") }
        assertThat(repo.transferCachedPhoto(photo(26, corrupt)) { error("Cannot publish corrupt bytes") }).isFalse()
        assertThat(repo.transferCachedPhoto(photo(25, source)) { false }).isFalse()
        assertThat(previous.readBytes()).isEqualTo(bytes)
        assertThat(previous.parentFile?.listFiles()?.toList()).containsExactly(previous)
        assertThat(api.calls).isEmpty()
    }

    @Test
    fun `stickers video and animated image bytes are outside still-photo transfer`() = runTest(dispatcher) {
        val source = writePhoto("25.jpg", Color.BLUE)
        val photo = photo(25, source)
        assertThat(repo.hasCachedFullPhoto(photo.copy(kind = "video"))).isFalse()
        assertThat(repo.hasCachedFullPhoto(photo.copy(sticker = true))).isFalse()

        val frame = AnimatedFixtures.GifImage(0, 0, 2, 2, ByteArray(4))
        source.writeBytes(AnimatedFixtures.gif(2, 2, intArrayOf(0, Color.GREEN), listOf(frame, frame)))
        val animated = photo(25, source).copy(mime = "image/gif")
        assertThat(repo.transferCachedPhoto(animated) { error("Cannot publish an animation") }).isFalse()
        assertThat(api.calls).isEmpty()
    }

    private fun writePhoto(name: String, color: Int): File {
        val bitmap = Bitmap.createBitmap(80, 48, Bitmap.Config.ARGB_8888)
        bitmap.eraseColor(color)
        bitmap.setPixel(0, 0, Color.TRANSPARENT)
        cache.mkdirs()
        val file = File(cache, name)
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        bitmap.recycle()
        return file
    }

    private fun photo(id: Long, file: File) = AttachmentDto(
        id = id,
        kind = AttachmentDto.KIND_PHOTO,
        mime = "image/png",
        size = file.length(),
        hasPreview = true,
    )
}
