/*
 * BoardPictureTest.kt
 * Family Connect (Android)
 *
 * A PHOTO IS DRAWN WHOLE (docs/protocol.md, "Board"): fitted in both
 * dimensions, never cropped to fill its box. Issue #71 — the wall kept the
 * middle of every portrait photograph and threw the rest away.
 *
 * The same numbers the other clients pin: `fc_text::board::fitted_picture`
 * and `BoardPicture.fitted` on Apple.
 */

package me.nettrash.familyconnect.ui.board

import androidx.compose.ui.layout.ContentScale
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class BoardPictureTest {

    /**
     * The word #71 was: a picture that FILLS its box is a picture cropped
     * to the box's shape, and on a square sticker that is the middle of
     * every portrait photograph.
     */
    @Test
    fun aPinnedPictureIsFittedNeverCropped() {
        assertThat(BoardPicture.scale).isEqualTo(ContentScale.Fit)
    }

    @Test
    fun aPortraitIsFittedNarrowAndAWideOneShort() {
        // The 600x1200 photograph #71 was reported with, on the Mac's
        // medium card: narrow, and every pixel of its height.
        val (width, height) = BoardPicture.fitted(150f, 110f, 600, 1200)
        assertThat(width).isWithin(0.01f).of(55f)
        assertThat(height).isWithin(0.01f).of(110f)
        // Whole: the picture's own shape, not the card's.
        assertThat(width / height).isWithin(0.001f).of(0.5f)

        val (wide, short) = BoardPicture.fitted(150f, 110f, 1600, 900)
        assertThat(wide).isWithin(0.01f).of(150f)
        assertThat(short).isWithin(0.01f).of(84.375f)
    }

    @Test
    fun aPictureNeverGrowsPastItsSpace() {
        assertThat(BoardPicture.fitted(132f, 132f, 300, 300)).isEqualTo(132f to 132f)
        val (width, height) = BoardPicture.fitted(132f, 132f, 30, 20)
        assertThat(width).isAtMost(132f)
        assertThat(height).isAtMost(132f)
        // And a panorama still leaves something to tap.
        val (_, hairline) = BoardPicture.fitted(132f, 132f, 20_000, 10)
        assertThat(hairline).isAtLeast(1f)
    }

    @Test
    fun dimensionsTheServerNeverGaveTakeTheWholeSpace() {
        // A margin at worst, and never a crop: the picture is still fitted
        // inside the space it is handed.
        assertThat(BoardPicture.fitted(132f, 132f, null, null)).isEqualTo(132f to 132f)
        assertThat(BoardPicture.fitted(132f, 132f, 600, null)).isEqualTo(132f to 132f)
        assertThat(BoardPicture.fitted(132f, 132f, 0, 1200)).isEqualTo(132f to 132f)
        assertThat(BoardPicture.fitted(132f, 132f, -4, 8)).isEqualTo(132f to 132f)
    }
}
