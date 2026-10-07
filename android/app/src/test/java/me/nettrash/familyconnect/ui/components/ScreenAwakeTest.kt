/*
 * ScreenAwakeTest.kt
 * Family Connect (Android)
 *
 * FLAG_KEEP_SCREEN_ON, counted per window (#79). A voice recording holds it
 * now as well as a video call, and the call screen comes up while the
 * chat's recording is still being put away: whichever lets go first must
 * not switch the other's hold off.
 */

package me.nettrash.familyconnect.ui.components

import android.app.Activity
import android.view.WindowManager
import com.google.common.truth.Truth.assertThat
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class ScreenAwakeTest {

    private val window = Robolectric.buildActivity(Activity::class.java).setup().get().window

    private val keptOn: Boolean
        get() = window.attributes.flags and WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON != 0

    @Test
    fun `the screen stays on until the last holder lets go`() {
        assertThat(keptOn).isFalse()

        ScreenAwake.acquire(window) // a recording
        ScreenAwake.acquire(window) // a video call coming up over it
        assertThat(keptOn).isTrue()

        ScreenAwake.release(window) // the recording put away
        assertThat(keptOn).isTrue()

        ScreenAwake.release(window) // the call over
        assertThat(keptOn).isFalse()
    }

    @Test
    fun `a release with nothing held leaves the screen to the system`() {
        ScreenAwake.release(window)

        assertThat(keptOn).isFalse()
        ScreenAwake.acquire(window)
        assertThat(keptOn).isTrue()
        ScreenAwake.release(window)
        assertThat(keptOn).isFalse()
    }
}
