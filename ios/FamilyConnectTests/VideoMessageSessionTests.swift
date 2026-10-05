//
//  VideoMessageSessionTests.swift
//  FamilyConnectTests
//
//  One opening of the round-video recorder, end to end with a fake camera
//  (#79, Phase 3 — docs/audio-video-messages-2026-10-04.md, S3, S4, S8.3):
//  that what the reducer decides is what is DONE — the camera off in REVIEW,
//  the microphone taken through the app's one arbiter and given back, the
//  clip handed to the send as a video message with its reply, deleted when
//  it is not wanted, a window close and a quit asked about — and the window's
//  door, the video button's rule and the File menu's.
//

import AVFoundation
import Foundation
import SwiftData
import Testing
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif
@testable import FamilyConnect

/// A camera that does what it is told and says what the test says.
@MainActor
final class FakeCaptureEngine: VideoCaptureEngine {
    var onEvent: ((VideoCaptureEvent) -> Void)?
    var previewLayer: AVCaptureVideoPreviewLayer? { nil }
    var cameras: [VideoCameraChoice] = []
    var currentCameraID: String?
    var canSwitchCamera = true

    private(set) var calls: [String] = []
    private(set) var recordingURL: URL?
    private(set) var recordingCap: UInt64?
    var isOn = false

    func startPreview() { calls.append("startPreview"); isOn = true }
    func stopPreview() { calls.append("stopPreview"); isOn = false }
    func startRecording(to url: URL, capMS: UInt64) {
        calls.append("startRecording")
        recordingURL = url
        recordingCap = capMS
    }
    func stopRecording() { calls.append("stopRecording") }
    func cancelRecording() { calls.append("cancelRecording") }
    func switchCamera() { calls.append("switchCamera") }
    func chooseCamera(_ id: String) { calls.append("chooseCamera:\(id)") }

    func say(_ event: VideoCaptureEvent) { onEvent?(event) }

    /// The writer closing the file: `ms` long, written where it was told.
    func finish(ms: UInt64, bytes: Int = 4_096) throws {
        let url = try #require(recordingURL)
        try Data(repeating: 1, count: bytes).write(to: url)
        say(.finished(RoundClipWriter.Clip(url: url, durationMS: ms)))
    }
}

@MainActor
@Suite("Video recorder: one opening, with a fake camera", .serialized)
struct VideoMessageSessionTests {

    /// Everything the session reached outside itself.
    @MainActor
    final class Spy {
        var now: UInt64 = 0
        var sent: [(prepared: MediaPrep.Prepared, reply: ReplyToDTO?, round: Bool)] = []
        var said: [String] = []
        var droppedReply = 0
        var closed = 0
        var dismissed = 0
        var startedVoice = 0
        var proceeded: [VideoRecorderMachine.Closing] = []
        var held: [Bool] = []
        var haptics = 0
        var taught = 0
        var askedCamera = 0
        var askedMicrophone = 0
        var sendSucceeds = true
        var spoken: [String] = []
        /// What the camera had been told when the words were said.
        var cameraCallsWhenSpoken: [String] = []
        var began = 0
        var ended = 0
    }

    private let reply = ReplyToDTO(messageID: 41, senderID: 9, excerpt: "Hi")

    private func make(
        camera: VideoRecorderMachine.Permission = .granted,
        microphone: VideoRecorderMachine.Permission = .granted,
        maxBytes: Int? = nil,
        notSent: Bool = false,
        firstTime: Bool = true
    ) -> (VideoMessageSession, FakeCaptureEngine, Spy, VoiceRecordingArbiter) {
        let spy = Spy()
        let engine = FakeCaptureEngine()
        let arbiter = VoiceRecordingArbiter()
        arbiter.keepAwake = { _ in }
        var request = VideoMessageSession.Request(chatID: 42, reply: reply)
        request.replyTitle = "Replying to Anna"
        request.replyText = "Hi"
        request.maxRoundVideoBytes = maxBytes
        request.notSent = { notSent }
        request.send = { prepared, reply, round in
            spy.sent.append((prepared, reply, round))
            return spy.sendSucceeds
        }
        request.dropReply = { spy.droppedReply += 1 }
        request.startVoice = { spy.startedVoice += 1 }
        request.closed = { spy.closed += 1 }
        let session = VideoMessageSession(request: request, engine: engine, firstTime: firstTime)
        session.runsTicker = false
        session.clock = { spy.now }
        session.cameraPermission = { camera }
        session.microphonePermission = { microphone }
        session.askCamera = { spy.askedCamera += 1; return true }
        session.askMicrophone = { spy.askedMicrophone += 1; return true }
        session.announce = { spy.said.append($0) }
        session.voiceOverRunning = { false }
        session.speak = { [weak engine] sentence in
            spy.spoken.append(sentence)
            spy.cameraCallsWhenSpoken = engine?.calls ?? []
        }
        session.haptic = { spy.haptics += 1 }
        session.holdOrientation = { spy.held.append($0) }
        session.prepare = { url in
            MediaPrep.Prepared(
                fileURL: url, mime: "video/mp4", kind: "video", width: 480, height: 480,
                durationMS: 5_000, previewJPEG: Data([0xFF, 0xD8]))
        }
        session.makeClipURL = {
            FileManager.default.temporaryDirectory.appendingPathComponent("fc-test-round-\(UUID().uuidString).mp4")
        }
        session.playbackSession = PlaybackSessionControl(begin: {}, end: {})
        session.arbiter = arbiter
        session.nowPlaying = NowPlaying()
        session.markTaught = { spy.taught += 1 }
        session.onClose = { spy.dismissed += 1 }
        session.onProceed = { spy.proceeded.append($0) }
        return (session, engine, spy, arbiter)
    }

    /// Opened, a frame in, recording from 1 000.
    private func recording(
        maxBytes: Int? = nil
    ) -> (VideoMessageSession, FakeCaptureEngine, Spy, VoiceRecordingArbiter) {
        let made = make(maxBytes: maxBytes)
        made.0.start()
        made.1.say(.firstFrame)
        made.2.now = 1_000
        made.0.record()
        return made
    }

    /// In REVIEW with a clip of `ms`.
    private func reviewing(
        _ ms: UInt64 = 5_000, maxBytes: Int? = nil, bytes: Int = 4_096
    ) throws -> (VideoMessageSession, FakeCaptureEngine, Spy, VoiceRecordingArbiter) {
        let made = recording(maxBytes: maxBytes)
        made.2.now = 1_000 + ms
        made.0.stop()
        try made.1.finish(ms: ms, bytes: bytes)
        return made
    }

    // MARK: - Opening

    @Test("opened with both granted: the camera on, the microphone not, nothing asked")
    func opens() {
        let (session, engine, spy, _) = make()
        session.start()
        #expect(engine.calls == ["startPreview"])
        #expect(session.state.phase == .preview)
        #expect(spy.askedCamera == 0 && spy.askedMicrophone == 0)
        engine.say(.firstFrame)
        #expect(spy.said == [String(localized: "Camera ready")])
        #expect(session.state.canRecord)
    }

    @Test("the first time it asks the camera, then the microphone, then turns the camera on")
    func asksFirst() async {
        let (session, engine, spy, _) = make(camera: .notAsked, microphone: .notAsked)
        session.start()
        #expect(session.askingPermission)
        #expect(session.state.phase == .asking)
        // A desktop window losing focus to the prompt does not close it.
        session.focusLost()
        #expect(session.state.phase == .asking)
        for _ in 0..<50 where session.state.phase != .preview { await Task.yield() }
        #expect(spy.askedCamera == 1)
        #expect(spy.askedMicrophone == 1)
        #expect(session.state.phase == .preview)
        #expect(engine.calls == ["startPreview"])
        #expect(!session.askingPermission)
    }

    @Test("a refused camera never turns it on")
    func refused() {
        let (session, engine, _, _) = make(camera: .denied)
        session.start()
        #expect(session.state.phase == .refused(.camera))
        #expect(engine.calls.isEmpty)
    }

    // MARK: - Recording

    @Test("Record takes the microphone through the arbiter, pauses playback, writes to the cap")
    func record() {
        let (session, engine, spy, arbiter) = recording()
        #expect(engine.calls == ["startPreview", "startRecording"])
        #expect(engine.recordingCap == 59_500)
        #expect(engine.recordingURL != nil)
        #expect(arbiter.holder == session.id)
        #expect(spy.held == [true])
        #expect(spy.haptics == 1)
        #expect(spy.said.last == String(localized: "Recording video"))
        #expect(spy.taught == 1)
        #expect(!session.showsFirstTimeLine)
    }

    @Test("with VoiceOver the microphone opens only once \"Recording video\" has been spoken (S6)")
    func voiceOverSpeaksFirst() async {
        let (session, engine, spy, arbiter) = make()
        session.voiceOverRunning = { true }
        let playing = NowPlaying()
        session.nowPlaying = playing
        var paused = 0
        playing.claim(UUID(), kind: .voice) { paused += 1 }
        session.start()
        engine.say(.firstFrame)
        spy.now = 1_000
        session.record()
        // At Record: playback paused and the one recording taken — but no
        // microphone yet.
        #expect(paused == 1)
        #expect(arbiter.holder == session.id)
        #expect(!engine.calls.contains("startRecording"), "the microphone opened before the words were said")
        #expect(!spy.said.contains(String(localized: "Recording video")), "announced, not waited for")
        spy.now = 1_900
        await session.speechTask?.value
        #expect(spy.spoken == [String(localized: "Recording video")])
        #expect(!spy.cameraCallsWhenSpoken.contains("startRecording"))
        #expect(engine.calls.last == "startRecording")
        #expect(session.state.phase == .recording(startMS: 1_900), "the clock starts with the microphone")
    }

    @Test("with VoiceOver, a Stop before the words end never opens the microphone")
    func voiceOverStopBeforeSpoken() async {
        let (session, engine, spy, arbiter) = make()
        session.voiceOverRunning = { true }
        session.start()
        engine.say(.firstFrame)
        spy.now = 1_000
        session.record()
        spy.now = 1_700
        session.stop()
        await session.speechTask?.value
        #expect(!engine.calls.contains("startRecording"))
        #expect(session.state.phase == .preview)
        #expect(session.state.notice == .tooShort)
        #expect(arbiter.holder == nil, "the recording was given back")
    }

    @Test("with VoiceOver, words overtaken by a Stop do not open the microphone for the NEXT Record")
    func voiceOverOvertakenWords() async {
        let (session, engine, spy, _) = make()
        session.voiceOverRunning = { true }
        var waiting: [CheckedContinuation<Void, Never>] = []
        session.speak = { _ in await withCheckedContinuation { waiting.append($0) } }
        session.start()
        engine.say(.firstFrame)
        spy.now = 1_000
        session.record()
        let first = session.speechTask
        spy.now = 1_700
        session.stop()
        spy.now = 2_400
        session.record()
        let second = session.speechTask
        for _ in 0..<100 where waiting.count < 2 { await Task.yield() }
        #expect(waiting.count == 2)
        // The FIRST words end while the second are still being said.
        waiting[0].resume()
        await first?.value
        #expect(!engine.calls.contains("startRecording"), "the old words opened the microphone over the new ones")
        waiting[1].resume()
        await second?.value
        #expect(engine.calls.last == "startRecording")
    }

    @Test("whether there is a camera is asked once — not on every composer redraw — and again after a change")
    func hasCameraIsKept() {
        let real = VideoMessageRecorder.lookForCamera
        defer {
            VideoMessageRecorder.lookForCamera = real
            VideoMessageRecorder.camerasChanged()
        }
        var asked = 0
        VideoMessageRecorder.lookForCamera = { asked += 1; return true }
        VideoMessageRecorder.camerasChanged()
        for _ in 0..<5 { #expect(VideoMessageRecorder.hasCamera) }
        #expect(asked == 1)
        VideoMessageRecorder.lookForCamera = { asked += 1; return false }
        VideoMessageRecorder.camerasChanged()
        #expect(!VideoMessageRecorder.hasCamera)
        #expect(!VideoMessageRecorder.hasCamera)
        #expect(asked == 2)
    }

    @Test("REVIEW gives the playback session back on pause, not only at the end (S5.3)")
    func reviewPauseGivesTheSessionBack() throws {
        let (session, _, spy, _) = try reviewing()
        session.playbackSession = PlaybackSessionControl(begin: { spy.began += 1 }, end: { spy.ended += 1 })
        session.playPause()
        #expect(session.isPlaying)
        #expect(spy.began == 1)
        session.playPause()
        #expect(!session.isPlaying)
        #expect(spy.ended == 1, "a paused clip kept the audio session")
        // Played again and paused by something else starting.
        session.playPause()
        #expect(spy.began == 2)
        session.nowPlaying.claim(UUID(), kind: .voice) {}
        #expect(!session.isPlaying)
        #expect(spy.ended == 2, "paused by another player, it kept the session")
        // Sent while paused: nothing more to give back, and never twice.
        spy.now = 30_000
        session.send()
        #expect(spy.ended == spy.began)
    }

    @Test("Record pauses whatever of the app's was playing (S1.7)")
    func recordPausesPlayback() {
        let (session, engine, spy, _) = make()
        let playing = NowPlaying()
        session.nowPlaying = playing
        var paused = 0
        playing.claim(UUID(), kind: .voice) { paused += 1 }
        session.start()
        engine.say(.firstFrame)
        spy.now = 1_000
        session.record()
        #expect(paused == 1)
        #expect(!playing.isPlaying)
    }

    @Test("another recording starting anywhere stops this one into REVIEW")
    func anotherRecordingStops() throws {
        let (session, engine, _, arbiter) = recording()
        arbiter.claim(UUID()) { _ in }
        #expect(engine.calls.last == "stopRecording")
        #expect(session.state.phase == .finishing(.away))
        try engine.finish(ms: 3_000)
        #expect(session.state.phase == .review(durationMS: 3_000))
    }

    @Test("Stop: the file is whole, the camera goes off, the microphone goes back")
    func stopToReview() throws {
        let (session, engine, spy, arbiter) = try reviewing(5_000)
        #expect(session.state.phase == .review(durationMS: 5_000))
        #expect(engine.calls.suffix(2) == ["stopRecording", "stopPreview"])
        #expect(!engine.isOn, "the light goes out in REVIEW")
        #expect(arbiter.holder == nil)
        #expect(spy.held == [true, false])
        let url = try #require(session.clipURL)
        #expect(FileManager.default.fileExists(atPath: url.path))
        #expect(session.player != nil, "REVIEW shows the clip's first frame")
    }

    @Test("too short: the file is deleted and PREVIEW says so")
    func tooShort() throws {
        let (session, engine, spy, _) = recording()
        spy.now = 1_700
        session.stop()
        let url = try #require(engine.recordingURL)
        try engine.finish(ms: 700)
        #expect(session.state.phase == .preview)
        #expect(!FileManager.default.fileExists(atPath: url.path))
        #expect(spy.said.last == String(localized: "That video was too short."))
        #expect(engine.isOn, "the camera stays on")
    }

    // MARK: - Send

    @Test("Send: the clip goes as a video message with its reply, and the recorder closes")
    func send() async throws {
        let (session, _, spy, _) = try reviewing()
        let url = try #require(session.clipURL)
        spy.now = 20_000
        session.send()
        await session.sendTask?.value
        #expect(spy.sent.count == 1)
        #expect(spy.sent.first?.round == true)
        #expect(spy.sent.first?.reply == reply)
        #expect(spy.sent.first?.prepared.fileURL == url)
        #expect(FileManager.default.fileExists(atPath: url.path), "the outbox owns the file now")
        #expect(spy.said.last == String(localized: "Video message sent"))
        #expect(spy.droppedReply == 1, "the reply left with the video")
        #expect(spy.dismissed == 1 && spy.closed == 1)
        #expect(session.isClosed)
        try? FileManager.default.removeItem(at: url)
    }

    @Test("over max_round_video_bytes it goes as a regular video, and REVIEW says so first")
    func tooBig() async throws {
        let (session, _, spy, _) = try reviewing(maxBytes: 1_000, bytes: 1_001)
        #expect(session.state.tooBig)
        spy.now = 20_000
        session.send()
        await session.sendTask?.value
        #expect(spy.sent.first?.round == false)
        if let url = spy.sent.first?.prepared.fileURL { try? FileManager.default.removeItem(at: url) }
    }

    @Test("at the ceiling exactly it is still a video message")
    func atTheCeiling() async throws {
        let (session, _, spy, _) = try reviewing(maxBytes: 1_000, bytes: 1_000)
        #expect(!session.state.tooBig)
        spy.now = 20_000
        session.send()
        await session.sendTask?.value
        #expect(spy.sent.first?.round == true)
        if let url = spy.sent.first?.prepared.fileURL { try? FileManager.default.removeItem(at: url) }
    }

    @Test("the reply's ✕ drops it from the video and from the composer")
    func dropReply() async throws {
        let (session, _, spy, _) = try reviewing()
        session.dropReply()
        #expect(session.reply == nil)
        #expect(spy.droppedReply == 1)
        spy.now = 20_000
        session.send()
        await session.sendTask?.value
        #expect(spy.sent.first?.reply == nil)
        if let url = spy.sent.first?.prepared.fileURL { try? FileManager.default.removeItem(at: url) }
    }

    // MARK: - Delete and close

    @Test("Delete in REVIEW removes the file and closes")
    func deleteInReview() throws {
        let (session, _, spy, _) = try reviewing(4_000)
        let url = try #require(session.clipURL)
        session.delete()
        #expect(!FileManager.default.fileExists(atPath: url.path))
        #expect(session.isClosed)
        #expect(spy.dismissed == 1)
        #expect(spy.sent.isEmpty)
    }

    @Test("Delete mid-take under 10 s throws the take away and keeps the camera")
    func deleteMidTake() {
        let (session, engine, _, arbiter) = recording()
        session.delete()
        #expect(engine.calls.last == "cancelRecording")
        #expect(arbiter.holder == nil)
        #expect(session.state.phase == .preview)
    }

    @Test("closing turns the camera off and gives everything back")
    func close() {
        let (session, engine, spy, arbiter) = make()
        session.start()
        session.close()
        #expect(!engine.isOn)
        #expect(spy.dismissed == 1 && spy.closed == 1)
        // No longer watching: going away again changes nothing.
        arbiter.appWentAway()
        #expect(spy.dismissed == 1)
    }

    @Test("the app going away closes PREVIEW through the arbiter, and stops a take")
    func wentAway() throws {
        let (session, _, spy, arbiter) = make()
        session.start()
        arbiter.appWentAway()
        #expect(session.isClosed)
        #expect(spy.dismissed == 1)

        let (taking, engine, _, takingArbiter) = recording()
        takingArbiter.appWentAway()
        #expect(taking.state.phase == .finishing(.away))
        try engine.finish(ms: 4_000)
        #expect(taking.state.phase == .review(durationMS: 4_000), "kept while the app runs")
    }

    @Test("a minimised window: only the recorder in THAT window stops")
    func windowWentAway() {
        let (session, _, _, arbiter) = make()
        let mine = NSObject()
        session.window = { mine }
        session.start()
        arbiter.windowWentAway(NSObject())
        #expect(session.state.phase == .preview)
        arbiter.windowWentAway(mine)
        #expect(session.isClosed)
    }

    @Test("⌘W: PREVIEW closes at once; over a clip it asks, Delete closes the window")
    func windowClose() throws {
        let (preview, _, previewSpy, _) = make()
        preview.start()
        #expect(preview.windowShouldClose())
        #expect(preview.isClosed)
        #expect(previewSpy.proceeded.isEmpty, "the window is closing already")

        let (session, _, spy, _) = try reviewing()
        let url = try #require(session.clipURL)
        #expect(!session.windowShouldClose())
        #expect(session.state.question == .closing(.window))
        session.answer(delete: false)
        #expect(spy.proceeded.isEmpty)
        #expect(!session.windowShouldClose())
        session.answer(delete: true)
        #expect(spy.proceeded == [.window])
        #expect(!FileManager.default.fileExists(atPath: url.path))
    }

    @Test("⌘Q over a take: it stops into REVIEW first, then asks")
    func quit() throws {
        let (session, engine, spy, _) = recording()
        spy.now = 9_000
        #expect(!session.appShouldQuit())
        #expect(engine.calls.last == "stopRecording")
        try engine.finish(ms: 8_000)
        #expect(session.state.question == .closing(.quit))
        session.answer(delete: true)
        #expect(spy.proceeded == [.quit])
    }

    @Test("sign-out mid-take throws the take away")
    func signOut() {
        let (session, engine, _, _) = recording()
        session.signedOut()
        #expect(engine.calls.contains("cancelRecording"))
        #expect(session.isClosed)
    }

    @Test("the camera taken mid-take: REVIEW with what was recorded, and the sentence")
    func cameraTaken() throws {
        let (session, engine, _, _) = recording()
        engine.say(.problem(.inUse))
        #expect(engine.calls.last == "stopRecording")
        try engine.finish(ms: 2_500)
        #expect(session.state.phase == .review(durationMS: 2_500))
        #expect(session.state.notice == .camera(.inUse))
    }

    @Test("\"Record a voice message instead\" closes the camera, then starts voice")
    func voiceInstead() {
        let (session, engine, spy, _) = make()
        session.start()
        session.voiceInstead()
        #expect(!engine.isOn)
        #expect(spy.startedVoice == 1)
        #expect(spy.dismissed == 1)
    }

    @Test("…dimmed while a not-sent voice message waits: it says why and stays")
    func voiceInsteadDimmed() {
        let (session, _, spy, _) = make(notSent: true)
        session.start()
        session.voiceInstead()
        #expect(spy.startedVoice == 0)
        #expect(session.state.notice == .notSent)
    }

    @Test("Switch camera and the reply's ✕ count as use: the 60 s close restarts")
    func controlsCountAsUse() {
        let (session, engine, spy, _) = make()
        session.start()
        spy.now = 50_000
        session.switchCamera()
        #expect(engine.calls.last == "switchCamera")
        spy.now = 100_000
        session.tick()
        #expect(session.state.phase == .preview)
        spy.now = 110_000
        session.tick()
        #expect(session.isClosed)
        #expect(spy.said.last == String(localized: "Camera turned off"))
    }

    @Test("the slot is Record, Stop or Send by the phase")
    func slot() throws {
        let (session, engine, spy, _) = make()
        session.start()
        engine.say(.firstFrame)
        spy.now = 1_000
        session.activateSlot()
        #expect(engine.calls.last == "startRecording")
        spy.now = 3_000
        session.activateSlot()
        #expect(engine.calls.last == "stopRecording")
    }

    // MARK: - The window's door

    @Test("the presenter opens one recorder at a time and forgets it when it closes")
    func presenter() {
        let presenter = VideoMessagePresenter()
        let engine = FakeCaptureEngine()
        presenter.makeEngine = { engine }
        // Never the system's permission prompt in a test.
        presenter.prepareSession = { session in
            session.runsTicker = false
            session.cameraPermission = { .granted }
            session.microphonePermission = { .granted }
            session.arbiter = VoiceRecordingArbiter()
            session.nowPlaying = NowPlaying()
            session.announce = { _ in }
        }
        var request = VideoMessageSession.Request(chatID: 42)
        var closed = 0
        request.closed = { closed += 1 }
        presenter.open(request)
        let first = presenter.session
        #expect(presenter.isOpen)
        presenter.open(VideoMessageSession.Request(chatID: 43))
        #expect(presenter.session === first, "one at a time")
        first?.close()
        #expect(!presenter.isOpen)
        #expect(closed == 1)
    }
}

// MARK: - The doors to it

@Suite("Video recorder: the doors")
struct VideoDoorTests {

    @Test("this build records: the button shows where the server and a camera allow")
    func thisBuildRecords() {
        #expect(VideoDoor.thisBuildRecords)
        let open = VideoDoor.Inputs(
            slot: ComposerSlot.Inputs(), familyOrDirectChat: true, undoWindow: false,
            serverOffersRound: true, hasCamera: true, encoderProbePasses: true)
        #expect(VideoDoor.of(open) == .shown)
        var oldServer = open
        oldServer.serverOffersRound = false
        #expect(VideoDoor.of(oldServer) == .hidden)
        var noCamera = open
        noCamera.hasCamera = false
        #expect(VideoDoor.of(noCamera) == .hidden)
        #expect(VideoDoor.label == String(localized: "Record video message"))
        #expect(VideoDoor.tooltip == String(localized: "Record a video message"))
    }

    #if os(macOS)
    @Test("File ▸ Record Video Message: where round video is, never over the recorder")
    func macMenu() {
        let idle = ComposerSlot.Inputs()
        #expect(MacVoiceMenu.videoIsEnabled(idle, roundAvailable: true))
        #expect(!MacVoiceMenu.videoIsEnabled(idle, roundAvailable: false))
        #expect(!MacVoiceMenu.videoIsEnabled(ComposerSlot.Inputs(recorderOpen: true), roundAvailable: true))
        #expect(!MacVoiceMenu.videoIsEnabled(ComposerSlot.Inputs(recording: .handsFree), roundAvailable: true))
        #expect(!MacVoiceMenu.videoIsEnabled(ComposerSlot.Inputs(editing: true), roundAvailable: true))
        #expect(!MacVoiceMenu.videoIsEnabled(ComposerSlot.Inputs(assistantChat: true), roundAvailable: true))
        #expect(!MacVoiceMenu.videoIsEnabled(ComposerSlot.Inputs(call: true), roundAvailable: true))
        #expect(!MacVoiceMenu.videoIsEnabled(ComposerSlot.Inputs(busy: true), roundAvailable: true))
        // A waiting voice message is voice's rule; words typed do not matter.
        #expect(MacVoiceMenu.videoIsEnabled(ComposerSlot.Inputs(notSent: true), roundAvailable: true))
        #expect(MacVoiceMenu.videoIsEnabled(ComposerSlot.Inputs(draftBlank: false), roundAvailable: true))
        // And Record Voice Message is off while the recorder is open.
        #expect(!MacVoiceMenu.isEnabled(ComposerSlot.Inputs(recorderOpen: true)))
    }
    #endif

    @Test("GET /families/mine: the two keys, and their absence is the answer")
    func discovery() throws {
        let family = """
            "family": {"id": 1, "name": "The Smiths", "join_policy": "open"},
            "members": [], "blocked_user_ids": []
            """
        let decoder = APICoding.decoder()
        let mine = try decoder.decode(FamilyMineResponse.self, from: Data("""
            {\(family), "max_round_video_ms": 60000, "max_round_video_bytes": 12582912}
            """.utf8))
        #expect(mine.maxRoundVideoMS == 60_000)
        #expect(mine.maxRoundVideoBytes == 12_582_912)
        let old = try decoder.decode(FamilyMineResponse.self, from: Data("{\(family)}".utf8))
        #expect(old.maxRoundVideoMS == nil)
        #expect(old.maxRoundVideoBytes == nil)
        let odd = try decoder.decode(FamilyMineResponse.self, from: Data("""
            {\(family), "max_round_video_ms": "soon"}
            """.utf8))
        #expect(odd.maxRoundVideoMS == nil, "a malformed key is absent, not a failed roster")
    }

    #if os(iOS)
    @Test("a phone is held in the orientation it was in at Record")
    @MainActor
    func orientationMask() {
        #expect(OrientationHold.mask(for: .portrait) == .portrait)
        #expect(OrientationHold.mask(for: .landscapeLeft) == .landscapeLeft)
        #expect(OrientationHold.mask(for: .landscapeRight) == .landscapeRight)
        #expect(OrientationHold.mask(for: .portraitUpsideDown) == .portraitUpsideDown)
        #expect(OrientationHold.mask == nil, "nothing is held until Record")
    }
    #endif

    #if os(macOS)
    @Test("a desktop keyboard: Return is the slot, Esc escapes, Space plays only in REVIEW")
    @MainActor
    func keys() throws {
        let engine = FakeCaptureEngine()
        let session = VideoMessageSession(request: VideoMessageSession.Request(chatID: 42), engine: engine)
        session.runsTicker = false
        var now: UInt64 = 0
        session.clock = { now }
        session.cameraPermission = { .granted }
        session.microphonePermission = { .granted }
        session.arbiter = VoiceRecordingArbiter()
        session.nowPlaying = NowPlaying()
        session.markTaught = {}
        session.announce = { _ in }
        session.start()
        engine.say(.firstFrame)

        #expect(!VideoRecorderKeys.handle(keyCode: VideoRecorderKeys.spaceKey, modifiers: [], session: session),
                "Space in PREVIEW is the focused control's")
        #expect(!VideoRecorderKeys.handle(keyCode: VideoRecorderKeys.returnKey, modifiers: [.command], session: session))
        #expect(VideoRecorderKeys.handle(keyCode: VideoRecorderKeys.returnKey, modifiers: [], session: session))
        #expect(engine.calls.last == "startRecording")
        now = 2_000
        #expect(VideoRecorderKeys.handle(keyCode: VideoRecorderKeys.enterKey, modifiers: [], session: session))
        #expect(engine.calls.last == "stopRecording")
        let url = try #require(engine.recordingURL)
        try Data([1]).write(to: url)
        engine.say(.finished(RoundClipWriter.Clip(url: url, durationMS: 2_000)))
        #expect(VideoRecorderKeys.handle(keyCode: VideoRecorderKeys.escapeKey, modifiers: [], session: session))
        #expect(session.state.question == .escape)
        session.answer(delete: false)
        session.playbackSession = PlaybackSessionControl(begin: {}, end: {})
        #expect(VideoRecorderKeys.handle(keyCode: VideoRecorderKeys.spaceKey, modifiers: [], session: session))
        #expect(session.isPlaying, "Space plays in REVIEW — it never sends")
        #expect(!session.isClosed)
        session.delete()
    }
    #endif
}

// MARK: - Discovery through a resync

@MainActor
@Suite("Video recorder: what the server offers", .serialized)
struct VideoDiscoveryTests {

    @Test("a resync stores the two keys, and a server without them takes the door away")
    func resyncStoresTheKeys() async throws {
        let savedMS = AppSettings.roundVideoMaxMS
        let savedBytes = AppSettings.roundVideoMaxBytes
        defer {
            AppSettings.roundVideoMaxMS = savedMS
            AppSettings.roundVideoMaxBytes = savedBytes
        }
        for (host, keys) in [
            ("round-keys.test", #", "max_round_video_ms": 60000, "max_round_video_bytes": 1000000"#),
            ("round-nokeys.test", ""),
        ] {
            StubURLProtocol.register(host: host) { request in
                switch request.url.path() {
                case "/api/v1/me":
                    return .json(200, """
                    {"user": {"id": 7, "username": "anna", "display_name": "Anna",
                              "created_at": "2026-08-19T17:00:00Z"},
                     "family": {"id": 3, "name": "The Smiths", "join_policy": "open",
                                "created_at": "2026-08-19T17:00:00Z"},
                     "role": "member"}
                    """)
                case "/api/v1/families/mine":
                    return .json(200, """
                    {"family": {"id": 3, "name": "The Smiths", "join_policy": "open",
                                "created_at": "2026-08-19T17:00:00Z"},
                     "members": [{"id": 7, "username": "anna", "display_name": "Anna", "role": "member"}]\(keys)}
                    """)
                case "/api/v1/chats":
                    return .json(200, #"{"chats": []}"#)
                default:
                    return .json(404, #"{"error": {"code": "not_found", "message": "?"}}"#)
                }
            }
            defer { StubURLProtocol.unregister(host: host) }
            let container = try ModelContainer(
                for: ChatEntity.self, MessageEntity.self, MemberEntity.self, NoteEntity.self,
                PendingMediaItemEntity.self, BlockEntity.self, GoneNoteEntity.self,
                PackItemEntity.self, GonePackItemEntity.self,
                configurations: ModelConfiguration(isStoredInMemoryOnly: true))
            let api = APIClient(serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())
            let coordinator = ChatSyncCoordinator(modelContainer: container, api: api)
            coordinator.currentUserIDOverride = 7
            await coordinator.resync()
            await coordinator.pendingDelivery?.value
            if keys.isEmpty {
                #expect(AppSettings.roundVideoMaxMS == nil)
                #expect(AppSettings.roundVideoMaxBytes == nil)
                #expect(!AppSettings.offersRoundVideo)
            } else {
                #expect(AppSettings.roundVideoMaxMS == 60_000)
                #expect(AppSettings.roundVideoMaxBytes == 1_000_000)
                #expect(AppSettings.offersRoundVideo)
            }
            _ = container
        }
    }

    @Test("sign-out forgets them: a different server must not inherit the door")
    func wipeForgets() {
        let savedMS = AppSettings.roundVideoMaxMS
        let savedBytes = AppSettings.roundVideoMaxBytes
        defer {
            AppSettings.roundVideoMaxMS = savedMS
            AppSettings.roundVideoMaxBytes = savedBytes
        }
        AppSettings.roundVideoMaxMS = 60_000
        AppSettings.roundVideoMaxBytes = 5
        AppSettings.wipe(keepServerURL: true)
        #expect(AppSettings.roundVideoMaxMS == nil)
        #expect(AppSettings.roundVideoMaxBytes == nil)
    }
}
