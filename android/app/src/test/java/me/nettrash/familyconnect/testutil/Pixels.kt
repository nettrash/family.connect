/*
 * Pixels.kt
 * Family Connect (Android) — tests
 *
 * What a composable actually DRAWS, on the JVM: the activity's window drawn
 * into a software bitmap under Robolectric's native graphics, and the pixels
 * of one tagged node read out of it. `captureToImage` waits for a frame
 * Robolectric's paused looper never schedules; drawing the window ourselves
 * is the same pixels without the wait — the iOS ImageRenderer trick's twin.
 *
 * Use with `createAndroidComposeRule<ComponentActivity>()` and
 * `@GraphicsMode(GraphicsMode.Mode.NATIVE)`.
 */

package me.nettrash.familyconnect.testutil

import android.graphics.Bitmap
import android.graphics.Canvas
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.junit4.AndroidComposeTestRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.ext.junit.rules.ActivityScenarioRule

/** The pixels of one node, as ARGB ints, row by row. */
class NodePixels(val width: Int, val height: Int, private val argb: IntArray) {
    operator fun get(x: Int, y: Int): Int = argb[y * width + x]
}

/** Draw the window and cut out the node tagged [tag] (unmerged tree, so a drawn-only node is found). */
fun AndroidComposeTestRule<ActivityScenarioRule<ComponentActivity>, ComponentActivity>.pixelsOf(tag: String): NodePixels {
    waitForIdle()
    val bounds = onNodeWithTag(tag, useUnmergedTree = true).fetchSemanticsNode().boundsInWindow
    var result: NodePixels? = null
    runOnIdle {
        val root = activity.window.decorView
        val bitmap = Bitmap.createBitmap(root.width.coerceAtLeast(1), root.height.coerceAtLeast(1), Bitmap.Config.ARGB_8888)
        root.draw(Canvas(bitmap))
        val left = bounds.left.toInt().coerceIn(0, bitmap.width - 1)
        val top = bounds.top.toInt().coerceIn(0, bitmap.height - 1)
        val width = (bounds.right.toInt() - left).coerceIn(1, bitmap.width - left)
        val height = (bounds.bottom.toInt() - top).coerceIn(1, bitmap.height - top)
        val argb = IntArray(width * height)
        bitmap.getPixels(argb, 0, width, left, top, width, height)
        result = NodePixels(width, height, argb)
    }
    return result!!
}
