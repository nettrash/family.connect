//
//  ComposerSlotTests.swift
//  FamilyConnectTests
//
//  #79, Phase 1: what RecordVectorTests cannot see about the shared rules'
//  Swift port (docs/audio-video-messages-2026-10-04.md, S1.3, S6, S10).
//
//  The vectors compare the ENGLISH keys — `labelKey`, `noticeKey`, `key` —
//  because that is what the reference prints. What a person reads is the
//  localized twin beside each key, written as a literal so the catalogue
//  check can see it. Nothing ties the two together but this suite: every
//  twin must resolve to exactly what its key resolves to, in whatever
//  language the test host runs, or a sentence on screen is not the sentence
//  the rule decided.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("Composer slot: the localized twins, the blocked row, the keyboard rows")
struct ComposerSlotTests {

    /// What the catalogue says for `key`, looked up by its English source —
    /// the same lookup `String(localized:)` makes for a literal.
    private func catalogue(_ key: String) -> String {
        String(localized: String.LocalizationValue(key))
    }

    private static let slots: [ComposerSlot] = [
        .recorder, .heldMicrophone, .sendVoice, .stopRecording, .save(enabled: true), .save(enabled: false),
        .send, .sendDisabled, .dimmed(.call), .dimmed(.busy), .dimmed(.notSent), .microphone,
    ]

    @Test("every slot's label on screen is its key's", arguments: slots)
    func slotLabels(slot: ComposerSlot) {
        guard let key = slot.labelKey else {
            #expect(slot.label == nil)
            return
        }
        #expect(slot.label == catalogue(key), "\(slot): '\(slot.label ?? "nil")' is not '\(key)'")
    }

    @Test("every dimmed row's sentence on screen is its key's", arguments: [
        ComposerSlot.Dimmed.call, .busy, .notSent,
    ])
    func dimmedNotices(reason: ComposerSlot.Dimmed) {
        #expect(reason.notice == catalogue(reason.noticeKey))
        #expect(ComposerSlot.dimmed(reason).notice == reason.notice)
        #expect(ComposerSlot.dimmed(reason).noticeKey == reason.noticeKey)
    }

    @Test("every hint on screen is its key's", arguments: [
        RecordGesture.Hint.stillRecording, .nextTimeSends, .nothingHeard, .stoppedAtFiveMinutes, .tooShort,
        .canRecordNow,
    ])
    func hints(hint: RecordGesture.Hint) {
        #expect(hint.text == catalogue(hint.key))
    }

    @Test("every announcement said is its key's", arguments: [
        RecordGesture.Announcement.recording, .recordingLocked, .recordingDeleted, .voiceMessageSent, .tooShort,
        .stoppedAtFiveMinutes,
    ])
    func announcements(announcement: RecordGesture.Announcement) {
        #expect(announcement.text == catalogue(announcement.key))
    }

    @Test("'Ready to review' carries the length as m:ss, through its key")
    func readyToReview() {
        let announcement = RecordGesture.Announcement.readyToReview(recordedMS: 42_400)
        #expect(announcement.key == "Ready to review, %@")
        #expect(announcement.text == String(format: catalogue(announcement.key), "0:42"))
    }

    // MARK: - The rules beside the vectors

    @Test("a recording meets the dimmed rows in the slot's order: call, then busy, then not sent")
    func blockedPrecedence() {
        #expect(ComposerSlot.Inputs().blocked == nil)
        #expect(ComposerSlot.Inputs(call: true, busy: true, notSent: true).blocked == .call)
        #expect(ComposerSlot.Inputs(busy: true, notSent: true).blocked == .busy)
        #expect(ComposerSlot.Inputs(notSent: true).blocked == .notSent)
        // Asked whether or not the slot is a microphone: beside words the
        // paperclip's Record Voice Message still meets a call.
        let typing = ComposerSlot.Inputs(draftBlank: false, call: true)
        #expect(ComposerSlot.of(typing) == .send)
        #expect(typing.blocked == .call)
    }

    @Test("⌘↩ and the Mac's Return take rows 2 to 5 only — never a microphone")
    func returnRows() {
        for slot in Self.slots {
            #expect(slot.takesReturn == (2...5).contains(slot.row), "\(slot)")
            if slot.isMicrophone { #expect(!slot.takesReturn, "Return reached a microphone: \(slot)") }
        }
    }

    @Test("iOS holds at 500 ms, the floor")
    func iosHoldThreshold() {
        #expect(VoiceComposer.systemLongPressMS == 500)
        #expect(RecordGesture.HoldConstants.forSystem(longPressMS: VoiceComposer.systemLongPressMS) == .standard)
    }

    @Test("u64 arithmetic saturates as the reference's does")
    func saturating() {
        #expect(RecordGesture.saturatingAdd(UInt64.max - 1, 600) == UInt64.max)
        #expect(RecordGesture.saturatingAdd(10, 600) == 610)
        #expect(RecordGesture.saturatingSub(400, 500) == 0)
        #expect(RecordGesture.saturatingSub(500, 500) == 0)
        #expect(RecordGesture.saturatingSub(501, 500) == 1)
    }

    @Test("Phase 3: this Apple build records, so the door opens where all else allows")
    func videoDoorOpensInPhaseThree() {
        #expect(VideoDoor.thisBuildRecords == true)
        let everythingElse = VideoDoor.Inputs(
            slot: ComposerSlot.Inputs(), familyOrDirectChat: true, undoWindow: false,
            serverOffersRound: true, hasCamera: true, encoderProbePasses: true)
        #expect(VideoDoor.of(everythingElse) == .shown)
        // Decision 40 still holds through the input: a build that only
        // receives shows nothing.
        var receivesOnly = everythingElse
        receivesOnly.recordsRoundVideo = false
        #expect(VideoDoor.of(receivesOnly) == .hidden)
    }
}

/// The recorder's two Phase 1 seams: the length read live for a decision,
/// and the tick the composer's warnings are said on.
@MainActor
@Suite("Voice recorder: the live length and the tick")
struct AudioRecorderPhaseOneTests {

    @MainActor
    final class Engine: VoiceRecordingEngine {
        let url: URL
        var currentTime: TimeInterval = 0
        var isRecording = false
        var onFinish: ((Bool) -> Void)?
        init(url: URL) { self.url = url }
        func record(forDuration duration: TimeInterval) -> Bool {
            try? Data(count: 4096).write(to: url)
            isRecording = true
            return true
        }
        func stop() { isRecording = false }
        func peakPower() -> Float { -30 }
    }

    @MainActor
    final class Made {
        var engine: Engine?
    }

    private func recording() async throws -> (AudioRecorder, Engine) {
        let directory = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("recorder-p1-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let recorder = AudioRecorder()
        recorder.directory = directory
        recorder.permissionProvider = { true }
        recorder.callIsActive = { false }
        recorder.audioSession = AudioSessionControl(activate: {}, deactivate: {})
        let made = Made()
        recorder.makeEngine = { url, _ in
            let engine = Engine(url: url)
            made.engine = engine
            return engine
        }
        await recorder.start()
        let engine = try #require(made.engine)
        try #require(recorder.isRecording)
        return (recorder, engine)
    }

    @Test("the length a decision reads is the engine's now, not the last tick's")
    func recordedNowIsLive() async throws {
        let (recorder, engine) = try await recording()
        engine.currentTime = 0.4
        recorder.tick()
        engine.currentTime = 1.1

        #expect(recorder.elapsed == 0.4)
        #expect(recorder.recordedNow == 1.1, "a release would be judged on a stale length")
        _ = recorder.stopRecording()
        #expect(recorder.recordedNow == recorder.elapsed, "a stopped recorder still read the engine")
    }

    @Test("every tick of a running recording is reported, and none after it stops")
    func tickReports() async throws {
        let (recorder, engine) = try await recording()
        var ticks = 0
        recorder.onTick = { ticks += 1 }

        engine.currentTime = 1
        recorder.tick()
        recorder.tick()
        #expect(ticks == 2)

        _ = recorder.stopRecording()
        recorder.tick()
        #expect(ticks == 2, "a stopped recorder still ticked")
    }
}
