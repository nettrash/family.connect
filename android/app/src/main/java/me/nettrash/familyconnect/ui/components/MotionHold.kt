/*
 * MotionHold.kt
 * Family Connect (Android)
 *
 * Whether the moving pictures in a thread — animated stickers and GIF tiles —
 * should hold still for the moment.
 *
 * They hold while the thread is being SCROLLED and while a full-screen viewer
 * is over it. Each one that plays costs every frame it is on screen: an
 * animated sticker keeps the render thread drawing the whole window, and a
 * GIF tile is a video decoder feeding a texture. During a fling those frames
 * are the very ones the scroll needs, and nobody can watch a GIF go past at
 * that speed anyway; under the viewer they are not seen at all. When the
 * scroll comes to rest, or the viewer closes, they play again.
 *
 * A [State] rather than a Boolean in the local, on purpose: the list starts
 * and stops scrolling all the time, and a Boolean local would recompose every
 * sticker and tile on screen at each edge. A State is read where the picture
 * acts on it — inside an effect, through snapshotFlow — so a scroll edge
 * reaches the players and recomposes nothing.
 *
 * Provided around the chat's LIST only, never around the viewer: the viewer
 * plays an animation itself, and must not hold still because it is open.
 * Anywhere it is not provided (the sticker panel, the viewer, the thread
 * screen) nothing ever holds.
 */

package me.nettrash.familyconnect.ui.components

import androidx.compose.runtime.State
import androidx.compose.runtime.staticCompositionLocalOf

/** Never held: the default, for every surface that does not provide one. */
private object NeverHeld : State<Boolean> {
    override val value: Boolean get() = false
}

/**
 * True while moving pictures under it should hold still. Static because the
 * State object it carries is created once per screen and never replaced; the
 * VALUE changes, and that is read in effects, not in composition.
 */
val LocalMotionHold = staticCompositionLocalOf<State<Boolean>> { NeverHeld }
