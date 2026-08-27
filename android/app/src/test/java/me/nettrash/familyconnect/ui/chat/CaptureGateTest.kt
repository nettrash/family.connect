/*
 * CaptureGateTest.kt
 * Family Connect (Android)
 *
 * The capture-gate rule: since the manifest declares CAMERA (video
 * calls), MediaStore's capture intents throw for this app unless the
 * runtime grant is held — so the chat's take-photo / record-video
 * hand-off may only launch once it is. The behavioral consequence:
 * photo/video capture in chat now prompts for the camera on first use.
 */

package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class CaptureGateTest {

    @Test
    fun captureLaunchesOnlyWithTheCameraGrant() {
        assertThat(CaptureGate.mayLaunchCapture(cameraGranted = true)).isTrue()
        assertThat(CaptureGate.mayLaunchCapture(cameraGranted = false)).isFalse()
    }
}
