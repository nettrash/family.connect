/*
 * CaptureGate.kt
 * Family Connect (Android)
 *
 * Whether the chat's take-photo / record-video hand-off may launch.
 *
 * The manifest now DECLARES android.permission.CAMERA (video calls own
 * the camera), and MediaStore's capture intents are documented to throw
 * a SecurityException for an app that DECLARES the permission without
 * holding the runtime grant — the exact inverse of the old rule, where
 * NOT declaring it was what kept the hand-off permission-free. So the
 * capture is gated on the grant, asked at the moment of use exactly like
 * the microphone beside it, and photo/video capture in chat prompts for
 * the camera on first use.
 *
 * A one-line rule, extracted so it is pinned on the JVM rather than
 * living only inside a Compose lambda.
 */

package me.nettrash.familyconnect.ui.chat

object CaptureGate {
    fun mayLaunchCapture(cameraGranted: Boolean): Boolean = cameraGranted
}
