/*
 * RoundVideoPrepTest.kt
 * Family Connect (Android)
 *
 * MediaPrep.prepareRoundVideo's contract for what CameraX left behind (#79,
 * S8.4, S4's "the recorder fails" row): nothing readable is null — the
 * recorder then says "The recording stopped unexpectedly." — and leaves no
 * file behind. What it does with a real clip (the probe, the Media3 pass,
 * `moov` first) needs a real encoder: the plan's device trial (Blocked 4).
 */

package me.nettrash.familyconnect.data.repo

import androidx.test.core.app.ApplicationProvider
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.test.runTest
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import java.io.File

@RunWith(RobolectricTestRunner::class)
class RoundVideoPrepTest {

    private val context = ApplicationProvider.getApplicationContext<android.content.Context>()
    private val mediaPrep = MediaPrep(context, context.contentResolver)

    private fun clip(bytes: ByteArray): File =
        File(context.cacheDir, "round-test-${bytes.size}.mp4").apply { writeBytes(bytes) }

    @Test
    fun `an empty file is nothing readable, and goes`() = runTest {
        val file = clip(ByteArray(0))
        assertThat(mediaPrep.prepareRoundVideo(file)).isNull()
        assertThat(file.exists()).isFalse()
    }

    @Test
    fun `a file with no video track is nothing readable, and goes`() = runTest {
        val file = clip(ByteArray(2048) { 7 })
        assertThat(mediaPrep.prepareRoundVideo(file)).isNull()
        assertThat(file.exists()).isFalse()
    }

    @Test
    fun `a file that is not there is nothing readable`() = runTest {
        assertThat(mediaPrep.prepareRoundVideo(File(context.cacheDir, "never-written.mp4"))).isNull()
    }
}
