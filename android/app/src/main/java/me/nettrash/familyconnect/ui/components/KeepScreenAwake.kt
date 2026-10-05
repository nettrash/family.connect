/*
 * KeepScreenAwake.kt
 * Family Connect (Android)
 *
 * FLAG_KEEP_SCREEN_ON, held while something says so and counted per window.
 *
 * Two things hold it now: a VIDEO call (CallScreen) and a voice recording
 * (#79, S1.7 — auto-lock would otherwise turn a long story into a parked
 * draft halfway through). They should never overlap, since nothing records
 * during a call, but the call screen comes up while the chat's recording is
 * still being put away, and a plain add/clear pair would let whichever
 * finished second switch the other's flag off. So each holder takes a count,
 * and the flag goes when the last one lets go.
 */

package me.nettrash.familyconnect.ui.components

import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.view.Window
import android.view.WindowManager
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.ui.platform.LocalContext
import java.util.WeakHashMap

/** Keep this window's screen on for as long as [active] is true and this is composed. */
@Composable
fun KeepScreenAwake(active: Boolean) {
    val context = LocalContext.current
    DisposableEffect(active, context) {
        val window = if (active) context.findActivity()?.window else null
        window?.let(ScreenAwake::acquire)
        onDispose { window?.let(ScreenAwake::release) }
    }
}

/** The count behind [KeepScreenAwake]: main thread only, as composition is. */
internal object ScreenAwake {

    /** Weak, so a window an activity took with it is not held here. */
    private val holders = WeakHashMap<Window, Int>()

    fun acquire(window: Window) {
        val count = (holders[window] ?: 0) + 1
        holders[window] = count
        if (count == 1) window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
    }

    fun release(window: Window) {
        val count = (holders[window] ?: 0) - 1
        if (count > 0) {
            holders[window] = count
        } else {
            holders.remove(window)
            window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        }
    }
}

/**
 * The Activity behind a Compose context. LocalContext is seldom the
 * Activity itself (a theme or configuration wrapper, usually), so this
 * walks the ContextWrapper chain until it finds one — or gives up, in
 * which case there is no window to keep awake and nothing to clean up.
 */
internal fun Context.findActivity(): Activity? {
    var current: Context? = this
    while (current is ContextWrapper) {
        if (current is Activity) return current
        current = current.baseContext
    }
    return null
}
