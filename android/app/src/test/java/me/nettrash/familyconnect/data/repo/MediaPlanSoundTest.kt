/*
 * MediaPlanSoundTest.kt
 * Family Connect (Android)
 *
 * Rule E — a transcode keeps the sound (docs/protocol.md, "Preparing media
 * before upload", issue #98) — as MediaPrep's picked-video path asks it:
 * what a finished transcode is worth once its sound tracks are counted. A
 * source with several is mixed (SoundMixTest), so a result of a source with
 * sound has exactly one. Not part of the reference's vectors.
 *
 * Plain JUnit: MediaPlan touches nothing from android.*.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.repo.MediaPlan.OnFailure
import me.nettrash.familyconnect.data.repo.MediaPlan.Upload
import org.junit.Test

class MediaPlanSoundTest {

    /** Two voices mixed into one track is the profile; two tracks out, or none, is not. */
    @Test
    fun `a source with sound must come out with exactly one sound track`() {
        assertThat(MediaPlan.keptSound(2, 1)).isTrue()
        assertThat(MediaPlan.keptSound(1, 1)).isTrue()
        assertThat(MediaPlan.keptSound(2, 0)).isFalse()
        assertThat(MediaPlan.keptSound(1, 2)).isFalse()
        assertThat(MediaPlan.judgeTranscode(80_000_000, true, 8_000_000, 2, 2)).isNull()
        assertThat(MediaPlan.judgeTranscode(80_000_000, true, 8_000_000, 3, 1)).isEqualTo(Upload.RESULT)
    }

    /**
     * The bug: a QuickTime PCM track Media3 cannot read, a result a tenth the
     * size, and rule D keeping it. Now it is rule C's — the source, since it
     * could go as it is.
     */
    @Test
    fun `a silent result of a source with sound is a failed transcode, however small`() {
        val verdict = MediaPlan.judgeTranscode(
            sourceBytes = 80_000_000,
            sourceSendable = true,
            resultBytes = 8_000_000,
            sourceSoundTracks = 1,
            resultSoundTracks = 0,
        )
        assertThat(verdict).isNull()
        assertThat(MediaPlan.onFailure(sourceSendable = true)).isEqualTo(OnFailure.ORIGINAL)

        // A source that cannot go as it is: still a failure, and 1.1's path.
        assertThat(
            MediaPlan.judgeTranscode(
                sourceBytes = 300_000_000,
                sourceSendable = false,
                resultBytes = 40_000_000,
                sourceSoundTracks = 2,
                resultSoundTracks = 0,
            ),
        ).isNull()
        assertThat(MediaPlan.onFailure(sourceSendable = false)).isEqualTo(OnFailure.TODAYS_PATH)
    }

    @Test
    fun `a result that kept its sound is rule D's as before`() {
        assertThat(MediaPlan.judgeTranscode(80_000_000, true, 8_000_000, 1, 1)).isEqualTo(Upload.RESULT)
        assertThat(MediaPlan.judgeTranscode(8_000_000, true, 9_000_000, 1, 1)).isEqualTo(Upload.SOURCE)
        assertThat(MediaPlan.judgeTranscode(8_000_000, false, 9_000_000, 1, 1)).isEqualTo(Upload.RESULT)
    }

    @Test
    fun `a source with no sound may come out with none`() {
        assertThat(MediaPlan.judgeTranscode(80_000_000, true, 8_000_000, 0, 0)).isEqualTo(Upload.RESULT)
    }

    /** A parse failure never blocks a send: unknown on either side is how things were before rule E. */
    @Test
    fun `an uncounted source or result changes nothing`() {
        assertThat(MediaPlan.keptSound(null, 0)).isTrue()
        assertThat(MediaPlan.keptSound(1, null)).isTrue()
        assertThat(MediaPlan.judgeTranscode(80_000_000, true, 8_000_000, null, 0)).isEqualTo(Upload.RESULT)
        assertThat(MediaPlan.judgeTranscode(80_000_000, true, 8_000_000, 1, null)).isEqualTo(Upload.RESULT)
    }
}
