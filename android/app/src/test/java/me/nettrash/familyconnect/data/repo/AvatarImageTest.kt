/*
 * AvatarImageTest.kt
 * Family Connect (Android)
 *
 * sampleSize is the load-bearing part: it decides how much of a
 * multi-megapixel photo ever reaches the heap. Getting it one step too
 * aggressive means uploading a blurry avatar, one step too timid means
 * decoding a 48 MB bitmap on a phone.
 *
 * Robolectric only because BitmapFactory is an Android class — the
 * assertions here are arithmetic, not pixels.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class AvatarImageTest {

    @Test
    fun `sample size never lands below the target`() {
        // 3000 (the short edge, the one the crop keeps) down to a 1024
        // target: /2 = 1500, /4 = 750 < 1024, so 2 is as far as
        // subsampling may go and the scale step does the rest.
        assertThat(AvatarImage.sampleSize(4000, 3000, 1024)).isEqualTo(2)
        assertThat(AvatarImage.sampleSize(3000, 4000, 1024)).isEqualTo(2)
    }

    @Test
    fun `an image already at or under the target is not subsampled`() {
        assertThat(AvatarImage.sampleSize(1024, 768, 1024)).isEqualTo(1)
        assertThat(AvatarImage.sampleSize(200, 200, 1024)).isEqualTo(1)
    }

    /**
     * The regression that matters: measuring the LONGEST edge would
     * subsample a panorama to 1500x250 and upload a 250 px avatar, because
     * the centre crop then throws away everything but those 250 rows.
     */
    @Test
    fun `a panorama keeps the resolution the crop will actually use`() {
        assertThat(AvatarImage.sampleSize(6000, 1000, 1024)).isEqualTo(1)
        assertThat(AvatarImage.sampleSize(1000, 6000, 1024)).isEqualTo(1)
        assertThat(AvatarImage.sampleSize(8000, 100, 1024)).isEqualTo(1)
    }

    @Test
    fun `a very long image still gets subsampled for memory`() {
        // Short edge alone would stop at 2 (20000x2000 = 40M pixels,
        // 160 MB decoded); the area ceiling takes it one step further.
        assertThat(AvatarImage.sampleSize(40_000, 4000, 1024)).isEqualTo(4)
    }

    @Test
    fun `sample size is a power of two and tracks the short edge`() {
        // 16384 → /2 8192 → /4 4096 → /8 2048 → /16 1024; an exact
        // power-of-two multiple lands exactly ON the target, which still
        // satisfies "never below".
        assertThat(AvatarImage.sampleSize(16_384, 16_384, 1024)).isEqualTo(16)
    }

    @Test
    fun `a nonsense target does not divide by zero`() {
        assertThat(AvatarImage.sampleSize(4000, 3000, 0)).isEqualTo(1)
        assertThat(AvatarImage.sampleSize(4000, 3000, -1)).isEqualTo(1)
    }

    /**
     * Mirrored constants: the two platforms must produce interchangeable
     * uploads, so a change here is a change on iOS too. The budget exists
     * because a proxy in front of a self-hosted server (this project's own
     * nginx config: 64k globally) rejects an oversize body with a bare 413
     * that carries none of the protocol's explanation — and because the
     * two JPEG encoders disagree by tens of kilobytes at the same nominal
     * quality, which had the same photo passing on one platform and
     * failing on the other.
     *
     * iOS: AvatarImage.swift — edge, qualitySteps, maxBytes.
     */
    @Test
    fun `the ladder and budget match the iOS constants`() {
        assertThat(AvatarImage.EDGE).isEqualTo(512)
        assertThat(AvatarImage.MAX_BYTES).isEqualTo(56 * 1024)
        assertThat(AvatarImage.QUALITY_STEPS.toList())
            .containsExactly(80, 65, 50, 40).inOrder()
        // Monotonically decreasing, or the ladder would step the wrong way.
        assertThat(AvatarImage.QUALITY_STEPS.toList())
            .isInStrictOrder(compareByDescending<Int> { it })
    }

    @Test
    fun `empty bytes decode to null rather than throwing`() {
        // Only the empty case is asserted: Robolectric's BitmapFactory
        // shadow fabricates a bitmap for any non-empty input, so
        // "undecodable bytes" cannot be exercised off-device.
        assertThat(AvatarImage.decode(ByteArray(0), maxPixels = 512)).isNull()
        assertThat(AvatarImage.squareJpeg(ByteArray(0))).isNull()
    }
}
