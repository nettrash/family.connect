//
//  AudioRecorderFailureTests.swift
//  FamilyConnectTests
//
//  "Record Audio" used to do nothing at all when the microphone had been
//  denied: the recorder set its failure faithfully and BOTH composers
//  ignored it, so there was no bar, no message and no explanation on either
//  platform. Nothing here would have caught that, because nothing read the
//  flag — so what these pin is the half that can be pinned: the recorder
//  reports the right cause, and each cause gets its own sentence.
//

import Foundation
import Testing
@testable import FamilyConnect

@MainActor
struct AudioRecorderFailureTests {

    @Test("a denied microphone is reported as denied, not as a generic failure")
    func deniedIsDistinct() async {
        let recorder = AudioRecorder()
        recorder.permissionProvider = { false }

        await recorder.start()

        #expect(recorder.failure == .microphoneDenied)
        #expect(!recorder.isRecording)
    }

    /// The distinction is the point of the type. Sending somebody to
    /// Settings for a permission they already granted wastes their time, and
    /// saying "couldn't start" to somebody who denied the mic tells them
    /// nothing they can act on.
    @Test("the two causes do not share a sentence")
    func causesReadDifferently() {
        let denied = AudioRecorder.message(for: .microphoneDenied)
        let broken = AudioRecorder.message(for: .couldNotStart)

        #expect(denied != broken)
        #expect(!broken.lowercased().contains("permission"),
                "a recorder that failed to start told the reader to change a permission")
    }

    /// A Mac has no Settings app, and the microphone switch is three levels
    /// into System Settings — the same class of wrong advice as telling a
    /// Mac user to reinstall a sandboxed app.
    @Test("the denial names the right place for the platform")
    func deniedNamesThePlatformsSettings() {
        let denied = AudioRecorder.message(for: .microphoneDenied)
        #if os(macOS)
        #expect(denied.contains("System Settings"))
        #else
        #expect(denied.contains("Settings"))
        #expect(!denied.contains("System Settings"))
        #endif
    }

    /// `start()` clears the previous outcome, or a granted retry would still
    /// be showing the refusal from the attempt before it.
    @Test("a retry clears the previous failure")
    func retryClearsFailure() async {
        let recorder = AudioRecorder()
        recorder.permissionProvider = { false }
        await recorder.start()
        #expect(recorder.failure == .microphoneDenied)

        recorder.permissionProvider = { true }
        await recorder.start()

        #expect(recorder.failure != .microphoneDenied,
                "the refusal from the previous attempt survived a granted retry")
        recorder.cancel()
    }
}
