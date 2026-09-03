/*
 * PlayListingTest.kt
 * Family Connect (Android)
 *
 * Pins the Play Console's hard upload rules on the listing assets that
 * live in this repo (android/fastlane/metadata/…). They are rules the
 * Console enforces at upload, hours after the work is done and usually
 * on somebody else's evening — and this repo has already been bitten by
 * two of them: the first Android screenshots were 1344x2992 (a 2.226:1
 * ratio, past Play's 2:1 ceiling) and RGBA where the feature graphic
 * spec asks for no alpha. Both faults were invisible in the files
 * themselves and only named at upload.
 *
 * Deliberately NOT a test of the WORDS. Copy is nettrash's, and a test
 * that pinned sentences would be rewritten every time the listing was.
 * What is pinned is only what Play will refuse:
 *
 *   title              <= 30 characters
 *   short description  <= 80 characters
 *   full description   <= 4000 characters
 *   feature graphic    exactly 1024x500, no alpha channel
 *   phone screenshots  2..8 of them, longer side <= 2 x shorter side
 *
 * The PNG header is read by hand rather than through BitmapFactory:
 * Robolectric's decoder happily reports a colour type this test exists
 * to catch (see the Kotlin coroutines/Robolectric notes elsewhere in
 * this suite), and IHDR is thirteen well-specified bytes.
 */

package me.nettrash.familyconnect.store

import com.google.common.truth.Truth.assertThat
import com.google.common.truth.Truth.assertWithMessage
import java.io.File
import org.junit.Test

class PlayListingTest {

    /**
     * The unit-test working directory is the MODULE directory
     * (`android/app`), so the metadata tree is one level up. Resolved
     * once and asserted, because a wrong root would make every
     * assertion below vacuously pass on a file that isn't there.
     */
    private val metadata: File =
        File("../fastlane/metadata/android/en-US").canonicalFile

    private fun text(name: String): String {
        val f = File(metadata, name)
        assertWithMessage("listing file %s", f.path).that(f.isFile).isTrue()
        // Play counts characters, and a trailing newline is not one of
        // them — supply strips it, so the test must too or a listing at
        // exactly the limit would fail here and pass at upload.
        return f.readText().trimEnd('\n')
    }

    @Test
    fun titleFitsPlaysThirtyCharacterLimit() {
        assertThat(text("title.txt").length).isAtMost(30)
    }

    @Test
    fun shortDescriptionFitsPlaysEightyCharacterLimit() {
        assertThat(text("short_description.txt").length).isAtMost(80)
    }

    @Test
    fun fullDescriptionFitsPlaysFourThousandCharacterLimit() {
        val full = text("full_description.txt")
        assertThat(full.length).isAtMost(4000)
        // A blank listing uploads cleanly and reads as an abandoned app,
        // which no character limit catches.
        assertThat(full.length).isAtLeast(200)
    }

    @Test
    fun featureGraphicIsExactlyTenTwentyFourByFiveHundredAndOpaque() {
        val png = PngHeader.read(File(metadata, "images/featureGraphic.png"))
        assertThat(png.width).isEqualTo(1024)
        assertThat(png.height).isEqualTo(500)
        assertThat(png.hasAlpha).isFalse()
    }

    @Test
    fun phoneScreenshotsAreWithinPlaysCountAndAspectRules() {
        val shots = File(metadata, "images/phoneScreenshots")
            .listFiles { f -> f.isFile && f.name.endsWith(".png") }
            ?.sortedBy { it.name }
            .orEmpty()
        assertThat(shots.size).isAtLeast(2)
        assertThat(shots.size).isAtMost(8)
        for (shot in shots) {
            val png = PngHeader.read(shot)
            val long = maxOf(png.width, png.height)
            val short = minOf(png.width, png.height)
            // "The longer side may be at most twice the shorter one."
            assertWithMessage("%s is %sx%s", shot.name, png.width, png.height)
                .that(long).isAtMost(2 * short)
            // RGBA is legal for a screenshot (only the feature graphic
            // forbids alpha), so nothing is asserted about colour type
            // here — but a zero dimension means the header was misread.
            assertWithMessage("%s width", shot.name).that(short).isGreaterThan(0)
        }
    }
}

/** The thirteen bytes of a PNG IHDR chunk that this test cares about. */
private data class PngHeader(val width: Int, val height: Int, val colourType: Int) {

    /** Colour types 4 (grey+alpha) and 6 (RGBA) carry an alpha channel. */
    val hasAlpha: Boolean get() = colourType == 4 || colourType == 6

    companion object {
        private val SIGNATURE = byteArrayOf(
            0x89.toByte(), 'P'.code.toByte(), 'N'.code.toByte(), 'G'.code.toByte(),
            0x0D, 0x0A, 0x1A, 0x0A,
        )

        fun read(file: File): PngHeader {
            require(file.isFile) { "${file.path} is missing" }
            val bytes = file.readBytes()
            require(bytes.size >= 33) { "${file.path} is too short to be a PNG" }
            require(bytes.copyOfRange(0, 8).contentEquals(SIGNATURE)) {
                "${file.path} is not a PNG"
            }
            // Bytes 8..15 are the IHDR length + type; the header itself
            // starts at 16: width, height, bit depth, colour type.
            fun int(at: Int) = ((bytes[at].toInt() and 0xFF) shl 24) or
                ((bytes[at + 1].toInt() and 0xFF) shl 16) or
                ((bytes[at + 2].toInt() and 0xFF) shl 8) or
                (bytes[at + 3].toInt() and 0xFF)
            return PngHeader(
                width = int(16),
                height = int(20),
                colourType = bytes[25].toInt() and 0xFF,
            )
        }
    }
}
