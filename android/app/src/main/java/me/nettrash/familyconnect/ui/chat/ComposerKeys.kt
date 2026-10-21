/*
 * ComposerKeys.kt
 * Family Connect (Android)
 *
 * What Enter does in a composer's field (#106,
 * docs/composer-and-wide-layout-2026-10-20.md, section 2.3):
 *
 *  - from a HARDWARE keyboard, Enter sends and Shift+Enter breaks the line
 *    — as the Mac, the web and Windows already do, and as the iPad now
 *    does: a tablet with a keyboard used to meet two meanings of Return in
 *    one app. Ctrl+Enter and Cmd+Enter, the old way to send here, keep
 *    sending;
 *  - from the ON-SCREEN keyboard, Enter is a line break on phones and
 *    tablets alike, unchanged: nothing here touches what an IME does.
 *
 * Why "hardware" has to be asked of the event and not of the window: an
 * IME may hand its Return key to the field as a KEY EVENT (sendKeyEvent)
 * rather than as text, and such an event reaches onPreviewKeyEvent exactly
 * like a keyboard's. What tells them apart is the event itself — the IME's
 * carry FLAG_SOFT_KEYBOARD and come from the virtual keyboard device, a
 * keyboard's come from a device of their own. A window with a keyboard
 * plugged in still has an IME that may send one.
 *
 * Sending is the composer's Send, which never records: Enter in an empty
 * field sends nothing and starts nothing (S1.3 of
 * docs/audio-video-messages-2026-10-04.md — "Return/Enter in an empty field
 * never records").
 *
 * The decision is plain values, no Compose, so every combination is pinned
 * by a unit test; [composerAction] only reads a Compose key event into it.
 */

package me.nettrash.familyconnect.ui.chat

import android.view.KeyCharacterMap
import android.view.KeyEvent
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.isAltPressed
import androidx.compose.ui.input.key.isCtrlPressed
import androidx.compose.ui.input.key.isMetaPressed
import androidx.compose.ui.input.key.isShiftPressed
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.type

object ComposerKeys {

    /** What the field does with a key. */
    enum class Action {
        /** Send what is written (the composer's own Send, which never records). */
        SEND,

        /** Let the field break the line, as it would anyway. */
        NEWLINE,

        /** Not ours: the field (or the IME) does what it always did. */
        PASS,
    }

    /**
     * The decision for one key event.
     *
     * @param keyDown the press, not the release — a field acts on the press.
     * @param enter Enter or the keypad's Enter.
     * @param hardware from a physical keyboard ([isHardware]).
     */
    fun decide(
        keyDown: Boolean,
        enter: Boolean,
        hardware: Boolean,
        shift: Boolean,
        ctrl: Boolean,
        meta: Boolean,
        alt: Boolean,
    ): Action = when {
        !keyDown || !enter || !hardware -> Action.PASS
        // The old chord first: Ctrl/Cmd+Enter sent before #106, with or
        // without Shift, and a person's habit must keep working.
        ctrl || meta -> Action.SEND
        // Alt+Enter means something to nobody here; the field decides.
        alt -> Action.PASS
        shift -> Action.NEWLINE
        else -> Action.SEND
    }

    /**
     * Whether a key event came from a physical keyboard: not flagged as the
     * soft keyboard's, and not from the virtual keyboard device an IME's
     * (and adb's) events are attributed to.
     */
    fun isHardware(flags: Int, deviceId: Int): Boolean =
        (flags and KeyEvent.FLAG_SOFT_KEYBOARD) == 0 && deviceId != KeyCharacterMap.VIRTUAL_KEYBOARD
}

/** [ComposerKeys.decide], asked of a Compose key event — every composer's field calls this. */
internal fun androidx.compose.ui.input.key.KeyEvent.composerAction(): ComposerKeys.Action =
    ComposerKeys.decide(
        keyDown = type == KeyEventType.KeyDown,
        enter = key == Key.Enter || key == Key.NumPadEnter,
        hardware = ComposerKeys.isHardware(nativeKeyEvent.flags, nativeKeyEvent.deviceId),
        shift = isShiftPressed,
        ctrl = isCtrlPressed,
        meta = isMetaPressed,
        alt = isAltPressed,
    )
