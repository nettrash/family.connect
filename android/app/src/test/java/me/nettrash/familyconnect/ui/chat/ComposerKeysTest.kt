/*
 * ComposerKeysTest.kt
 * Family Connect (Android) — tests
 *
 * Enter in a composer's field (#106, docs/composer-and-wide-layout-2026-10-20.md,
 * 2.3): a hardware keyboard sends with Enter and breaks the line with
 * Shift+Enter, Ctrl/Cmd+Enter keeps sending, and nothing an IME sends is
 * touched.
 */

package me.nettrash.familyconnect.ui.chat

import android.view.KeyCharacterMap
import android.view.KeyEvent
import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.ui.chat.ComposerKeys.Action
import org.junit.Test

class ComposerKeysTest {

    private fun decide(
        keyDown: Boolean = true,
        enter: Boolean = true,
        hardware: Boolean = true,
        shift: Boolean = false,
        ctrl: Boolean = false,
        meta: Boolean = false,
        alt: Boolean = false,
    ) = ComposerKeys.decide(keyDown, enter, hardware, shift, ctrl, meta, alt)

    @Test
    fun aHardwareEnterSends() {
        assertThat(decide()).isEqualTo(Action.SEND)
    }

    @Test
    fun shiftEnterBreaksTheLine() {
        assertThat(decide(shift = true)).isEqualTo(Action.NEWLINE)
    }

    /** The chord that sent before #106 keeps sending, Shift or not. */
    @Test
    fun ctrlOrCmdEnterStillSends() {
        assertThat(decide(ctrl = true)).isEqualTo(Action.SEND)
        assertThat(decide(meta = true)).isEqualTo(Action.SEND)
        assertThat(decide(ctrl = true, shift = true)).isEqualTo(Action.SEND)
    }

    /** The on-screen keyboard's Enter is a line break, as always — never a send. */
    @Test
    fun anImesEnterIsNeverOurs() {
        assertThat(decide(hardware = false)).isEqualTo(Action.PASS)
        assertThat(decide(hardware = false, ctrl = true)).isEqualTo(Action.PASS)
        assertThat(decide(hardware = false, shift = true)).isEqualTo(Action.PASS)
    }

    @Test
    fun otherKeysReleasesAndAltAreNotOurs() {
        assertThat(decide(enter = false)).isEqualTo(Action.PASS)
        assertThat(decide(keyDown = false)).isEqualTo(Action.PASS)
        assertThat(decide(alt = true)).isEqualTo(Action.PASS)
    }

    /**
     * What "hardware" means: an IME's key events are flagged as the soft
     * keyboard's or come from the virtual keyboard device (adb's too).
     */
    @Test
    fun hardwareIsAPhysicalDeviceWithoutTheSoftKeyboardFlag() {
        assertThat(ComposerKeys.isHardware(flags = 0, deviceId = 3)).isTrue()
        assertThat(ComposerKeys.isHardware(flags = KeyEvent.FLAG_FROM_SYSTEM, deviceId = 3)).isTrue()
        assertThat(ComposerKeys.isHardware(flags = KeyEvent.FLAG_SOFT_KEYBOARD, deviceId = 3)).isFalse()
        assertThat(
            ComposerKeys.isHardware(
                flags = KeyEvent.FLAG_SOFT_KEYBOARD or KeyEvent.FLAG_KEEP_TOUCH_MODE,
                deviceId = KeyCharacterMap.VIRTUAL_KEYBOARD,
            ),
        ).isFalse()
        assertThat(ComposerKeys.isHardware(flags = 0, deviceId = KeyCharacterMap.VIRTUAL_KEYBOARD)).isFalse()
    }
}
