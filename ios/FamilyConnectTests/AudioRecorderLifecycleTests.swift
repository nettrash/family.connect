//
//  AudioRecorderLifecycleTests.swift
//  FamilyConnectTests
//
//  #79, Phase 0: the recorder made safe (docs/audio-video-messages-2026-10-04.md,
//  S2.5, S2.8, S4). Every ending a real microphone cannot be asked to
//  produce on demand — the five-minute cap, an encoder failure, the hardware
//  stopping by itself, an interruption — is driven through the recorder's
//  seams: a fake engine, a counting audio session, an answered call flag.
//
//  The bug at the top of the list is the one that touched somebody else's
//  audio: every stop and cancel deactivated the shared session
//  unconditionally, and deactivating a session while audio runs in it stops
//  that audio — a call's included.
//

import Foundation
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Voice recorder: endings, the call's session, metering", .serialized)
struct AudioRecorderLifecycleTests {

    /// A recorder that records nothing real: the file it would be writing is
    /// written by the fake, so the "something was captured" test is real.
    final class FakeEngine: VoiceRecordingEngine {
        let url: URL
        var currentTime: TimeInterval = 0
        var isRecording = false
        var onFinish: ((Bool) -> Void)?
        var peaks: [Float] = []
        var stops = 0
        /// Bytes "captured" — over the recorder's 1024-byte floor.
        var bytes = 4096

        init(url: URL) { self.url = url }

        func record(forDuration duration: TimeInterval) -> Bool {
            try? Data(count: bytes).write(to: url)
            isRecording = true
            return true
        }

        func stop() {
            stops += 1
            isRecording = false
        }

        func peakPower() -> Float {
            peaks.isEmpty ? AudioRecorder.quietest : peaks.removeFirst()
        }

        /// What `AVAudioRecorder` does at the end of `record(forDuration:)`
        /// (`true`), or on an encoder error (`false`).
        func stopsByItself(successfully: Bool) {
            isRecording = false
            onFinish?(successfully)
        }
    }

    final class World {
        var callActive = false
        var activations = 0
        var deactivations = 0
        var engine: FakeEngine?
        var ended: [AudioRecorder.Ended] = []
    }

    private func scratch() throws -> URL {
        let url = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("recorder-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    private func makeRecorder(_ world: World) throws -> AudioRecorder {
        let recorder = AudioRecorder()
        recorder.directory = try scratch()
        recorder.permissionProvider = { true }
        recorder.callIsActive = { world.callActive }
        recorder.audioSession = AudioSessionControl(
            activate: { world.activations += 1 },
            deactivate: { world.deactivations += 1 })
        recorder.makeEngine = { url, _ in
            let engine = FakeEngine(url: url)
            world.engine = engine
            return engine
        }
        recorder.onEnded = { world.ended.append($0) }
        return recorder
    }

    private func recording(_ world: World) async throws -> AudioRecorder {
        let recorder = try makeRecorder(world)
        await recorder.start()
        try #require(recorder.isRecording, "the fake engine did not start")
        return recorder
    }

    // MARK: - A call's session is never deactivated

    @Test("stop never deactivates a session a call holds")
    func stopLeavesTheCallsSession() async throws {
        let world = World()
        let recorder = try await recording(world)
        world.callActive = true

        _ = recorder.stop()

        #expect(world.deactivations == 0, "a stop switched off the audio a call was using")
    }

    @Test("with no call, stop gives the session back")
    func stopGivesTheSessionBack() async throws {
        let world = World()
        let recorder = try await recording(world)

        _ = recorder.stop()

        #expect(world.activations == 1)
        #expect(world.deactivations == 1)
    }

    @Test("cancel never deactivates a session a call holds, and gives it back otherwise")
    func cancelRespectsTheCall() async throws {
        let duringCall = World()
        let first = try await recording(duringCall)
        duringCall.callActive = true
        first.cancel()
        #expect(duringCall.deactivations == 0)

        let quiet = World()
        let second = try await recording(quiet)
        second.cancel()
        #expect(quiet.deactivations == 1)
    }

    /// A call is the commonest interruption there is: CallKit takes the
    /// session, iOS interrupts ours, and the recorder must stop WITHOUT
    /// handing back a session that is now the call's.
    @Test("an interruption by a call stops and keeps, and leaves the call's session alone")
    func interruptionByACall() async throws {
        let world = World()
        let recorder = try await recording(world)
        world.engine?.currentTime = 4
        world.callActive = true

        recorder.interrupted()

        #expect(!recorder.isRecording)
        #expect(world.deactivations == 0)
        #expect(world.ended.map(\.reason) == [.interrupted])
        let kept = try #require(world.ended.first?.recording)
        #expect(FileManager.default.fileExists(atPath: kept.url.path), "the interruption threw the recording away")
        #expect(kept.duration == 4)
    }

    @Test("a recording is refused outright during a call, before anything is touched")
    func refusedDuringACall() async throws {
        let world = World()
        world.callActive = true
        let recorder = try makeRecorder(world)
        var prompts = 0
        recorder.permissionProvider = {
            prompts += 1
            return true
        }

        await recorder.start()

        #expect(recorder.failure == .callInProgress)
        #expect(!recorder.isRecording)
        #expect(prompts == 0, "a permission prompt was raised over a call")
        #expect(world.engine == nil, "a recorder was made during a call")
        #expect(world.activations == 0, "the session was taken from a call")
    }

    /// A permission prompt can stay up for as long as somebody likes, and a
    /// call can ring through it.
    @Test("a call that rings while the permission prompt is up still refuses")
    func callDuringThePrompt() async throws {
        let world = World()
        let recorder = try makeRecorder(world)
        recorder.permissionProvider = {
            world.callActive = true
            return true
        }

        await recorder.start()

        #expect(recorder.failure == .callInProgress)
        #expect(world.engine == nil)
    }

    /// The composer went away — or stopped what it was asking for — while
    /// the prompt was up. Opening the microphone after that is exactly the
    /// "microphone left on with nobody looking" this phase exists to end.
    @Test("a start abandoned while the prompt is up never opens the microphone")
    func abandonedDuringThePrompt() async throws {
        let world = World()
        let recorder = try makeRecorder(world)
        recorder.permissionProvider = {
            recorder.cancel()
            return true
        }

        await recorder.start()

        #expect(!recorder.isRecording)
        #expect(world.engine == nil, "the microphone opened behind a composer that had let go")
        #expect(recorder.failure == nil)
    }

    @Test("the refusal during a call has its own sentence, which sends nobody to Settings")
    func callRefusalSentence() {
        let call = AudioRecorder.message(for: .callInProgress)
        #expect(call == String(localized: "You can record a message after the call."))
        #expect(call != AudioRecorder.message(for: .couldNotStart))
        #expect(!call.contains("Settings"))
    }

    // MARK: - The cap is reported

    @Test("the five-minute cap is reported, with the whole recording kept")
    func capReported() async throws {
        let world = World()
        let recorder = try await recording(world)
        let engine = try #require(world.engine)
        engine.currentTime = AudioRecorder.maxDuration

        engine.stopsByItself(successfully: true)

        #expect(!recorder.isRecording, "the counter would sit there climbing past a recording that had ended")
        #expect(world.ended.map(\.reason) == [.cap])
        let kept = try #require(world.ended.first?.recording)
        #expect(kept.duration == AudioRecorder.maxDuration)
        #expect(FileManager.default.fileExists(atPath: kept.url.path))
    }

    /// The delegate is not the only witness: the ticker notices a recorder
    /// that stopped at the cap even if the delegate never speaks.
    @Test("the ticker reports the cap when the recorder stops there unannounced")
    func tickerReportsTheCap() async throws {
        let world = World()
        let recorder = try makeRecorder(world)
        recorder.cap = 3
        await recorder.start()
        let engine = try #require(world.engine)
        engine.currentTime = 2.9
        recorder.tick()

        engine.isRecording = false
        recorder.tick()

        #expect(world.ended.map(\.reason) == [.cap])
        #expect(world.ended.first?.recording?.duration == 3)
    }

    @Test("the person's own stop is never reported as an ending")
    func ownStopIsNotAnEnding() async throws {
        let world = World()
        let recorder = try await recording(world)
        let engine = try #require(world.engine)
        engine.currentTime = 5

        let url = recorder.stop()

        #expect(url != nil)
        #expect(engine.onFinish == nil, "a late delegate call could still report the stop as an ending")
        #expect(world.ended.isEmpty)
    }

    // MARK: - Failures and silent stops

    @Test("a recorder that fails keeps what is readable and says so")
    func failureKeepsWhatIsReadable() async throws {
        let world = World()
        let recorder = try await recording(world)
        let engine = try #require(world.engine)
        engine.currentTime = 7

        engine.stopsByItself(successfully: false)

        #expect(!recorder.isRecording)
        #expect(world.ended.map(\.reason) == [.failed])
        #expect(world.ended.first?.recording != nil)
    }

    @Test("an input that vanishes with no word is kept as an interruption after a grace tick")
    func silentStopIsAnInterruption() async throws {
        let world = World()
        let recorder = try await recording(world)
        let engine = try #require(world.engine)
        engine.currentTime = 12
        recorder.tick()
        engine.isRecording = false

        recorder.tick()
        #expect(world.ended.isEmpty, "the delegate gets one tick to say why first")
        recorder.tick()

        #expect(world.ended.map(\.reason) == [.interrupted])
        #expect(world.ended.first?.recording?.duration == 12)
    }

    // MARK: - Under a second there is nothing worth keeping

    @Test("an interruption under a second deletes the recording")
    func shortInterruptionDeletes() async throws {
        let world = World()
        let recorder = try await recording(world)
        let engine = try #require(world.engine)
        engine.currentTime = 0.6

        recorder.interrupted()

        #expect(world.ended.map(\.reason) == [.interrupted])
        #expect(world.ended.first?.recording == nil)
        #expect(!FileManager.default.fileExists(atPath: engine.url.path), "a blip was kept as a not-sent message")
    }

    @Test("stopKeeping keeps a second or more, and deletes anything shorter")
    func stopKeepingFloor() async throws {
        let long = World()
        let first = try await recording(long)
        long.engine?.currentTime = 1.0
        let kept = first.stopKeeping()
        #expect(kept?.duration == 1.0)

        let short = World()
        let second = try await recording(short)
        let engine = try #require(short.engine)
        engine.currentTime = 0.99
        #expect(second.stopKeeping() == nil)
        #expect(!FileManager.default.fileExists(atPath: engine.url.path))
    }

    @Test("stopKeeping with nothing recording stops nothing and keeps nothing")
    func stopKeepingIdle() throws {
        let recorder = try makeRecorder(World())
        #expect(recorder.stopKeeping() == nil)
    }

    // MARK: - Metering

    @Test("every tick reads the peak level, and sound is heard only above −60 dBFS")
    func meteringAndSilence() async throws {
        let world = World()
        let recorder = try await recording(world)
        let engine = try #require(world.engine)
        engine.peaks = [-75, -60, -42]

        recorder.tick()
        #expect(recorder.peakLevel == -75)
        #expect(!recorder.heardSound)
        recorder.tick()
        #expect(!recorder.heardSound, "−60 dBFS is still digital silence")
        recorder.tick()
        #expect(recorder.peakLevel == -42)
        #expect(recorder.heardSound)
    }

    @Test("the level meter's five bars light at −50, −40, −30, −20 and −10 dBFS")
    func levelBars() {
        #expect(AudioRecorder.litBars(peak: AudioRecorder.quietest) == 0)
        #expect(AudioRecorder.litBars(peak: -50.1) == 0)
        #expect(AudioRecorder.litBars(peak: -50) == 1)
        #expect(AudioRecorder.litBars(peak: -35) == 2)
        #expect(AudioRecorder.litBars(peak: -20) == 4)
        #expect(AudioRecorder.litBars(peak: 0) == 5)
        #expect(!AudioRecorder.isAudible(peak: -60))
        #expect(AudioRecorder.isAudible(peak: -59.9))
    }

    @Test("a new recording starts unheard and at the quietest level")
    func meteringResets() async throws {
        let world = World()
        let recorder = try await recording(world)
        world.engine?.peaks = [-20]
        recorder.tick()
        #expect(recorder.heardSound)
        _ = recorder.stop()

        await recorder.start()

        #expect(!recorder.heardSound)
        #expect(recorder.peakLevel == AudioRecorder.quietest)
        recorder.cancel()
    }

    /// The launch sweep recognises a dead run's recording by its name.
    @Test("recordings are written under the prefix the launch sweep takes")
    func filePrefix() async throws {
        let world = World()
        let recorder = try await recording(world)
        let engine = try #require(world.engine)
        #expect(engine.url.lastPathComponent.hasPrefix(AudioRecorder.filePrefix))
        #expect(MediaOutbox.sweptPrefixes.contains(AudioRecorder.filePrefix))
        recorder.cancel()
    }
}
