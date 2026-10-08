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
    func deniedIsDistinct() async throws {
        let recorder = try Self.recorderThatTouchesNothingReal()
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
    ///
    /// The granted retry starts a FAKE engine. It used to start the real
    /// `AVAudioRecorder` — a unit test opening the microphone of whatever
    /// machine ran it, which on a Mac hung the run — and it could only say
    /// "not denied", since a real microphone may still fail to start. With
    /// every seam answered nothing real is touched, and the retry has to
    /// come back clean: recording, no failure at all.
    @Test("a retry clears the previous failure")
    func retryClearsFailure() async throws {
        let recorder = try Self.recorderThatTouchesNothingReal()
        var engines: [StandInEngine] = []
        recorder.makeEngine = { url, _ in
            let engine = StandInEngine(url: url)
            engines.append(engine)
            return engine
        }
        recorder.permissionProvider = { false }
        await recorder.start()
        #expect(recorder.failure == .microphoneDenied)
        #expect(engines.isEmpty, "a denied start built an engine")

        recorder.permissionProvider = { true }
        await recorder.start()

        #expect(recorder.failure == nil,
                "the refusal from the previous attempt survived a granted retry")
        #expect(recorder.isRecording && engines.count == 1 && engines[0].started)
        recorder.cancel()
        #expect(engines.first?.isRecording == false)
    }

    // MARK: - Nothing real

    /// Records nothing: writes the bytes a real recorder would have, so the
    /// recorder's own bookkeeping runs as it does in the app.
    final class StandInEngine: VoiceRecordingEngine {
        let url: URL
        var currentTime: TimeInterval = 0
        var isRecording = false
        var started = false
        var onFinish: ((Bool) -> Void)?

        init(url: URL) { self.url = url }

        func record(forDuration duration: TimeInterval) -> Bool {
            try? Data(count: 4096).write(to: url)
            started = true
            isRecording = true
            return true
        }

        func stop() { isRecording = false }

        func peakPower() -> Float { AudioRecorder.quietest }
    }

    /// A recorder whose every way out to the hardware is answered by the
    /// test: no audio session is taken, no call is asked about, and an
    /// engine nobody replaced fails the test instead of opening the real
    /// microphone. Permission is still the caller's to answer.
    static func recorderThatTouchesNothingReal() throws -> AudioRecorder {
        let recorder = AudioRecorder()
        let directory = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("recorder-failure-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        recorder.directory = directory
        recorder.callIsActive = { false }
        recorder.audioSession = AudioSessionControl(activate: {}, deactivate: {})
        recorder.makeEngine = { _, _ in
            Issue.record("a unit test reached for the real microphone")
            throw CocoaError(.featureUnsupported)
        }
        return recorder
    }
}
