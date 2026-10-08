//
//  VideoRecorderMachineTests.swift
//  FamilyConnectTests
//
//  The round-video recorder's rules (#79, Phase 3 —
//  docs/audio-video-messages-2026-10-04.md, S3.2–S3.6, S4, S6): every row of
//  S3.4's state table and S4's video columns, asked of the pure reducer with
//  no camera, no window and a clock that is just a number.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("Video recorder: the states, the guards and the interruptions")
struct VideoRecorderMachineTests {

    typealias M = VideoRecorderMachine

    // MARK: - Helpers

    /// Run events in order, collecting every effect.
    private func run(_ state: M.State, _ events: [M.Event]) -> (M.State, [M.Effect]) {
        var current = state
        var all: [M.Effect] = []
        for event in events {
            let (next, effects) = M.step(current, event)
            current = next
            all += effects
        }
        return (current, all)
    }

    /// PREVIEW with a frame delivered at 1 000.
    private func preview() -> M.State {
        let (state, _) = run(M.State(), [
            .opened(camera: .granted, microphone: .granted, atMS: 0),
            .firstFrame(atMS: 1_000),
        ])
        return state
    }

    /// RECORDING since 2 000.
    private func recording() -> M.State {
        run(preview(), [.record(atMS: 2_000)]).0
    }

    /// REVIEW of a clip `duration` long, stopped by Stop at 2 000 + duration.
    private func review(_ duration: UInt64 = 5_000) -> M.State {
        run(recording(), [
            .stop(atMS: 2_000 + duration),
            .finished(durationMS: duration, tooBig: false, atMS: 2_000 + duration + 50),
        ]).0
    }

    // MARK: - S3.2 Permission

    @Test("both granted: straight to PREVIEW, the camera on and the microphone not")
    func openedGranted() {
        let (state, effects) = M.step(M.State(), .opened(camera: .granted, microphone: .granted, atMS: 5))
        #expect(state.phase == .preview)
        #expect(effects == [.startCamera])
        #expect(!effects.contains(.startRecording))
    }

    @Test("a refusal read first opens straight to it, and raises no prompt for the other")
    func openedRefused() {
        let micOff = M.step(M.State(), .opened(camera: .notAsked, microphone: .denied, atMS: 0))
        #expect(micOff.0.phase == .refused(.microphone))
        #expect(micOff.1.isEmpty, "no camera prompt for somebody whose microphone is off")

        let cameraOff = M.step(M.State(), .opened(camera: .denied, microphone: .notAsked, atMS: 0))
        #expect(cameraOff.0.phase == .refused(.camera))
        #expect(cameraOff.1.isEmpty, "no microphone prompt either")

        // Both refused: the microphone first — the camera refusal would
        // offer a voice message that needs it too.
        let both = M.step(M.State(), .opened(camera: .denied, microphone: .denied, atMS: 0))
        #expect(both.0.phase == .refused(.microphone))
        #expect(!both.0.cameraOn)
    }

    @Test("the first time: the camera is asked, then the microphone, then PREVIEW")
    func askingInOrder() {
        var (state, effects) = M.step(M.State(), .opened(camera: .notAsked, microphone: .notAsked, atMS: 0))
        #expect(state.phase == .asking)
        #expect(effects == [.requestCamera])
        (state, effects) = M.step(state, .cameraAnswered(true, atMS: 10))
        #expect(state.phase == .asking)
        #expect(effects == [.requestMicrophone])
        (state, effects) = M.step(state, .microphoneAnswered(true, atMS: 20))
        #expect(state.phase == .preview)
        #expect(effects == [.startCamera])

        let onlyMic = M.step(M.State(), .opened(camera: .granted, microphone: .notAsked, atMS: 0))
        #expect(onlyMic.0.phase == .asking)
        #expect(onlyMic.1 == [.requestMicrophone])
    }

    @Test("an answer of no lands on that refusal, with the camera off")
    func answersOfNo() {
        let asking = M.step(M.State(), .opened(camera: .notAsked, microphone: .notAsked, atMS: 0)).0
        let camera = M.step(asking, .cameraAnswered(false, atMS: 1))
        #expect(camera.0.phase == .refused(.camera))
        #expect(!camera.1.contains(.startCamera))
        let mic = run(asking, [.cameraAnswered(true, atMS: 1), .microphoneAnswered(false, atMS: 2)])
        #expect(mic.0.phase == .refused(.microphone))
        #expect(!mic.1.contains(.startCamera))
    }

    // MARK: - PREVIEW → RECORDING

    @Test("Record is dimmed until the first frame, then opens the microphone and says so")
    func recordAfterFirstFrame() {
        let opened = M.step(M.State(), .opened(camera: .granted, microphone: .granted, atMS: 0)).0
        let early = M.step(opened, .record(atMS: 500))
        #expect(early.0.phase == .preview)
        #expect(early.1.isEmpty)
        #expect(!opened.canRecord)

        let ready = M.step(opened, .firstFrame(atMS: 700))
        #expect(ready.1 == [.announce(.cameraReady)])
        #expect(ready.0.canRecord)
        let again = M.step(ready.0, .firstFrame(atMS: 800))
        #expect(again.1.isEmpty, "Camera ready is said once")

        let (state, effects) = M.step(ready.0, .record(atMS: 1_000))
        #expect(state.phase == .recording(startMS: 1_000))
        #expect(effects == [.holdOrientation(true), .startRecording, .haptic, .announce(.recordingVideo), .taught])
    }

    @Test("a camera problem dims Record")
    func problemDimsRecord() {
        let busy = M.step(preview(), .camera(.inUse)).0
        #expect(!busy.canRecord)
        #expect(M.step(busy, .record(atMS: 5_000)).0.phase == .preview)
        let back = M.step(busy, .camera(nil)).0
        #expect(back.canRecord)
    }

    // MARK: - The 600 ms guard (S1.1)

    @Test("Record → Stop → Send each ignore activation for 600 ms")
    func guards() {
        let rec = recording()
        #expect(M.step(rec, .stop(atMS: 2_599)).0.phase == .recording(startMS: 2_000))
        let stopping = M.step(rec, .stop(atMS: 2_600))
        #expect(stopping.0.phase == .finishing(.stop))
        #expect(stopping.1 == [.stopRecording])

        let reviewing = M.step(
            M.step(recording(), .stop(atMS: 4_000)).0,
            .finished(durationMS: 2_000, tooBig: false, atMS: 4_100)).0
        #expect(M.step(reviewing, .send(atMS: 4_599)).1.isEmpty, "a double tap on Stop must not send")
        #expect(M.step(reviewing, .send(atMS: 4_600)).1.contains(.send(round: true)))
    }

    // MARK: - RECORDING → REVIEW, and too short

    @Test("a take of 1.0 s or more goes to REVIEW with the camera off")
    func stopToReview() {
        let (state, effects) = run(recording(), [
            .stop(atMS: 5_000), .finished(durationMS: 3_000, tooBig: false, atMS: 5_100),
        ])
        #expect(state.phase == .review(durationMS: 3_000))
        #expect(effects.contains(.stopCamera))
        #expect(effects.contains(.holdOrientation(false)))
        #expect(!state.cameraOn)
    }

    @Test("Stop under 1.0 s goes back to PREVIEW with \"That video was too short.\"")
    func tooShort() {
        let (state, effects) = run(recording(), [
            .stop(atMS: 2_700), .finished(durationMS: 700, tooBig: false, atMS: 2_750),
        ])
        #expect(state.phase == .preview)
        #expect(state.notice == .tooShort)
        #expect(effects.contains(.deleteClip))
        #expect(effects.contains(.announce(.tooShort)))
        #expect(!effects.contains(.stopCamera), "the camera stays on")
        #expect(M.Notice.tooShort.sentence == String(localized: "That video was too short."))
    }

    @Test("nothing readable: back to PREVIEW, saying the recording stopped unexpectedly")
    func unreadable() {
        let (state, _) = run(recording(), [.stop(atMS: 5_000), .finished(durationMS: nil, tooBig: false, atMS: 5_100)])
        #expect(state.phase == .preview)
        #expect(state.notice == .stoppedUnexpectedly)
    }

    // MARK: - The length limit

    @Test("at 50 s \"10 seconds left\" is said once; at 59.5 s the take stops into REVIEW")
    func lengthLimit() {
        let rec = recording()
        #expect(rec.capMS == 59_500)
        #expect(rec.warningMS == 50_000)
        let before = M.step(rec, .tick(atMS: 2_000 + 49_999))
        #expect(before.1.isEmpty)
        #expect(!before.0.inWarning(atMS: 2_000 + 49_999))
        let warned = M.step(before.0, .tick(atMS: 2_000 + 50_000))
        #expect(warned.1 == [.announce(.tenSecondsLeft)])
        #expect(warned.0.inWarning(atMS: 2_000 + 50_000))
        #expect(M.step(warned.0, .tick(atMS: 2_000 + 51_000)).1.isEmpty, "said once")

        let (state, effects) = run(warned.0, [
            .tick(atMS: 2_000 + 59_500),
            .finished(durationMS: 59_480, tooBig: false, atMS: 2_000 + 59_600),
        ])
        #expect(state.phase == .review(durationMS: 59_480))
        #expect(state.notice == .stoppedAtLimit)
        #expect(effects.contains(.stopRecording))
        #expect(effects.contains(.announce(.stoppedAtLimit)))
        #expect(!effects.contains(where: { if case .send = $0 { true } else { false } }),
                "a length limit never sends")
    }

    @Test("the writer reaching the limit first stops it the same way")
    func writerLimit() {
        let (state, effects) = run(recording(), [
            .limitReached, .finished(durationMS: 59_500, tooBig: false, atMS: 62_000),
        ])
        #expect(state.phase == .review(durationMS: 59_500))
        #expect(state.notice == .stoppedAtLimit)
        #expect(effects.first == .stopRecording)
    }

    @Test("a shorter server limit moves both numbers")
    func serverLimit() {
        let state = M.State(maxRoundVideoMS: 30_000)
        #expect(state.capMS == 29_500)
        #expect(state.warningMS == 20_000)
    }

    // MARK: - Delete, Retake, Keep (S3.4)

    @Test("Delete under 10 s in RECORDING goes at once; the camera stays on")
    func deleteShortTake() {
        let (state, effects) = M.step(recording(), .delete(atMS: 2_000 + 9_999))
        #expect(state.phase == .preview)
        #expect(effects == [.cancelRecording, .holdOrientation(false)])
    }

    @Test("Delete at 10 s or more stops first, then asks; Keep goes to REVIEW")
    func deleteLongTake() {
        let asked = run(recording(), [
            .delete(atMS: 12_000),
            .finished(durationMS: 10_000, tooBig: false, atMS: 12_100),
        ])
        #expect(asked.0.phase == .review(durationMS: 10_000))
        #expect(asked.0.question == .discardTake)
        #expect(asked.1.contains(.stopRecording))

        let kept = M.step(asked.0, .answer(delete: false, atMS: 13_000))
        #expect(kept.0.phase == .review(durationMS: 10_000))
        #expect(kept.0.question == nil)
        #expect(kept.1.isEmpty)

        let deleted = M.step(asked.0, .answer(delete: true, atMS: 13_000))
        #expect(deleted.0.phase == .preview)
        #expect(deleted.1.contains(.deleteClip))
        #expect(deleted.1.contains(.startCamera), "the camera comes back on")
    }

    @Test("REVIEW's Delete: at once under 10 s, asked from 10 s; Delete closes")
    func deleteInReview() {
        let short = M.step(review(9_999), .delete(atMS: 20_000))
        #expect(short.0.phase == .closed)
        #expect(short.1.contains(.deleteClip))
        #expect(short.1.last == .close)

        let long = M.step(review(10_000), .delete(atMS: 20_000))
        #expect(long.0.question == .delete)
        #expect(long.1.isEmpty)
        let closed = M.step(long.0, .answer(delete: true, atMS: 20_100))
        #expect(closed.0.phase == .closed)
        #expect(closed.1.contains(.deleteClip))
    }

    @Test("Retake: back to PREVIEW with the camera on — asked first from 10 s")
    func retake() {
        let short = M.step(review(4_000), .retake(atMS: 20_000))
        #expect(short.0.phase == .preview)
        #expect(short.1.contains(.deleteClip))
        #expect(short.1.contains(.startCamera))
        #expect(!short.0.canRecord, "Record waits for the new first frame")

        let long = M.step(review(12_000), .retake(atMS: 20_000))
        #expect(long.0.question == .retake)
        #expect(long.0.phase == .review(durationMS: 12_000))
        let again = M.step(long.0, .answer(delete: true, atMS: 20_500))
        #expect(again.0.phase == .preview)
        #expect(again.1.contains(.startCamera))
    }

    // MARK: - Send (S3.4, S3.6)

    @Test("Send hands the clip over as a video message, says so and closes")
    func send() {
        let (state, effects) = M.step(review(), .send(atMS: 30_000))
        #expect(state.phase == .closed)
        #expect(effects == [.stopPlayback, .send(round: true), .announce(.sent), .close])
    }

    @Test("over the byte ceiling it goes as a regular video, without the flag")
    func tooBig() {
        let state = run(recording(), [
            .stop(atMS: 8_000), .finished(durationMS: 6_000, tooBig: true, atMS: 8_100),
        ]).0
        #expect(state.tooBig)
        #expect(M.step(state, .send(atMS: 9_000)).1.contains(.send(round: false)))
    }

    @Test("nothing but Send ever sends")
    func onlySendSends() {
        // A seeded walk over every event, from every state it reaches.
        var rng = SystemRandomNumberGeneratorStandIn(seed: 79)
        for _ in 0..<300 {
            var state = M.State()
            var now: UInt64 = 0
            for _ in 0..<40 {
                now += UInt64(rng.next() % 4_000)
                let event = Self.events(now: now, pick: rng.next())
                let (next, effects) = M.step(state, event)
                let sent = effects.contains { if case .send = $0 { true } else { false } }
                if case .send = event {} else {
                    #expect(!sent, "\(event) sent from \(state.phase)")
                }
                state = next
            }
        }
    }

    private static func events(now: UInt64, pick: UInt64) -> M.Event {
        let all: [M.Event] = [
            .opened(camera: .granted, microphone: .granted, atMS: now),
            .firstFrame(atMS: now), .frame(dark: pick % 2 == 0, atMS: now), .camera(.inUse), .camera(nil),
            .record(atMS: now), .stop(atMS: now), .delete(atMS: now), .retake(atMS: now),
            .send(atMS: now), .close, .escape(atMS: now), .magicTap(atMS: now), .playPause,
            .answer(delete: pick % 3 == 0, atMS: now), .voiceInstead(notSent: false), .used(atMS: now),
            .tick(atMS: now), .limitReached,
            .finished(durationMS: pick % 5 == 0 ? nil : pick % 20_000, tooBig: false, atMS: now),
            .callStarted, .wentAway, .focusLost, .microphoneTaken, .recorderFailed,
            .closing(.window), .closing(.quit),
            .record(atMS: now, speakFirst: true), .magicTap(atMS: now, speakFirst: true), .spoken(atMS: now),
        ]
        return all[Int(pick % UInt64(all.count))]
    }

    // MARK: - Esc, Magic Tap, play/pause (S3.4, S6)

    @Test("Esc closes PREVIEW, stops RECORDING, and in REVIEW always asks")
    func escape() {
        let closed = M.step(preview(), .escape(atMS: 3_000))
        #expect(closed.0.phase == .closed)
        #expect(closed.1 == [.stopCamera, .close])

        let stopped = M.step(recording(), .escape(atMS: 2_100))
        #expect(stopped.0.phase == .finishing(.stop), "Esc is Stop, with no guard")

        let asked = M.step(review(3_000), .escape(atMS: 9_000))
        #expect(asked.0.question == .escape, "asked even under 10 s")
        #expect(asked.1.isEmpty)
        let kept = M.step(asked.0, .answer(delete: false, atMS: 9_100))
        #expect(kept.0.phase == .review(durationMS: 3_000))
    }

    @Test("Magic Tap: Record in PREVIEW, Stop in RECORDING, play/pause in REVIEW")
    func magicTap() {
        #expect(M.step(preview(), .magicTap(atMS: 3_000)).0.phase == .recording(startMS: 3_000))
        #expect(M.step(recording(), .magicTap(atMS: 9_000)).0.phase == .finishing(.stop))
        #expect(M.step(review(), .magicTap(atMS: 20_000)).1 == [.togglePlayback])
    }

    @Test("play/pause only plays a clip in REVIEW, and not while a question is up")
    func playPause() {
        #expect(M.step(preview(), .playPause).1.isEmpty)
        #expect(M.step(recording(), .playPause).1.isEmpty)
        #expect(M.step(review(), .playPause).1 == [.togglePlayback])
        let asking = M.step(review(12_000), .delete(atMS: 30_000)).0
        #expect(M.step(asking, .playPause).1.isEmpty)
    }

    // MARK: - The 60 s idle close and the near-black check

    @Test("PREVIEW closes after 60 s with no control used, and says the camera turned off")
    func idleClose() {
        let opened = preview()
        #expect(M.step(opened, .tick(atMS: 59_999)).0.phase == .preview)
        let (state, effects) = M.step(opened, .tick(atMS: 60_000))
        #expect(state.phase == .closed)
        #expect(effects == [.stopCamera, .announce(.cameraTurnedOff), .close])

        let used = M.step(opened, .used(atMS: 30_000)).0
        #expect(M.step(used, .tick(atMS: 60_000)).0.phase == .preview, "a control used restarts it")
        #expect(M.step(used, .tick(atMS: 90_000)).0.phase == .closed)
    }

    @Test("two seconds of near-black PREVIEW asks whether the camera is covered")
    func nearBlack() {
        var state = preview()
        state = M.step(state, .frame(dark: true, atMS: 3_000)).0
        #expect(!state.looksDark(atMS: 4_999))
        #expect(state.looksDark(atMS: 5_000))
        state = M.step(state, .frame(dark: true, atMS: 4_000)).0
        #expect(state.looksDark(atMS: 5_000), "the run started at the first dark frame")
        state = M.step(state, .frame(dark: false, atMS: 5_100)).0
        #expect(!state.looksDark(atMS: 9_000), "a bright frame ends it")
        #expect(state.canRecord, "a dark room is not an error")

        let rec = M.step(M.step(preview(), .frame(dark: true, atMS: 1_000)).0, .record(atMS: 2_000)).0
        #expect(!rec.looksDark(atMS: 9_000), "PREVIEW only")
    }

    @Test("a frame counts as near-black at or under 5 % luma, video and full range")
    func nearBlackLuma() {
        #expect(NearBlack.meanLuma(sum: 16 * 100, count: 100, videoRange: true) == 0)
        #expect(NearBlack.meanLuma(sum: 235 * 100, count: 100, videoRange: true) == 1)
        #expect(NearBlack.meanLuma(sum: 0, count: 100, videoRange: false) == 0)
        #expect(NearBlack.meanLuma(sum: 255 * 10, count: 10, videoRange: false) == 1)
        #expect(NearBlack.meanLuma(sum: 0, count: 0, videoRange: false) == 0)
        #expect(NearBlack.isDark(meanLuma: NearBlack.meanLuma(sum: 26 * 10, count: 10, videoRange: true)))
        #expect(!NearBlack.isDark(meanLuma: NearBlack.meanLuma(sum: 40 * 10, count: 10, videoRange: true)))
        #expect(NearBlack.isDark(meanLuma: 0.05))
        #expect(!NearBlack.isDark(meanLuma: 0.051))
    }

    // MARK: - S4: interruptions

    @Test("a call: PREVIEW closes, RECORDING stops into REVIEW, REVIEW stays")
    func call() {
        let closed = M.step(preview(), .callStarted)
        #expect(closed.0.phase == .closed)
        #expect(closed.1 == [.stopCamera, .close])

        let (reviewed, _) = run(recording(), [
            .callStarted, .finished(durationMS: 4_000, tooBig: false, atMS: 6_100),
        ])
        #expect(reviewed.phase == .review(durationMS: 4_000))

        let kept = M.step(review(), .callStarted)
        #expect(kept.0.phase == .review(durationMS: 5_000))
        #expect(kept.1 == [.stopPlayback])
    }

    @Test("under 1.0 s an interruption deletes the take instead, and closes")
    func shortInterruption() {
        let (state, effects) = run(recording(), [
            .wentAway, .finished(durationMS: 400, tooBig: false, atMS: 2_500),
        ])
        #expect(state.phase == .closed)
        #expect(effects.contains(.deleteClip))
        #expect(state.notice != .tooShort)
    }

    @Test("the background, a lock, sleep or a minimised window: the same as a call")
    func wentAway() {
        #expect(M.step(preview(), .wentAway).0.phase == .closed)
        #expect(M.step(recording(), .wentAway).0.phase == .finishing(.away))
        #expect(M.step(review(), .wentAway).0.phase == .review(durationMS: 5_000))
        let asking = M.step(M.State(), .opened(camera: .notAsked, microphone: .notAsked, atMS: 0)).0
        #expect(M.step(asking, .wentAway).0.phase == .closed)
    }

    @Test("a desktop window losing focus closes PREVIEW only; a take keeps recording")
    func focusLost() {
        #expect(M.step(preview(), .focusLost).0.phase == .closed)
        #expect(M.step(recording(), .focusLost).0.phase == .recording(startMS: 2_000))
        #expect(M.step(review(), .focusLost).0.phase == .review(durationMS: 5_000))
        let asking = M.step(M.State(), .opened(camera: .notAsked, microphone: .notAsked, atMS: 0)).0
        #expect(M.step(asking, .focusLost).0.phase == .asking, "never to a permission prompt")
    }

    @Test("the microphone taken, or the recorder failing: REVIEW with what was recorded")
    func microphoneTaken() {
        for event in [M.Event.microphoneTaken, .recorderFailed] {
            let (state, _) = run(recording(), [event, .finished(durationMS: 3_000, tooBig: false, atMS: 6_000)])
            #expect(state.phase == .review(durationMS: 3_000))
            #expect(state.notice == .stoppedUnexpectedly)
        }
        #expect(M.step(preview(), .microphoneTaken).1.isEmpty, "the microphone is not open in PREVIEW")
    }

    @Test("the camera taken mid-take stops it into REVIEW with the camera's sentence")
    func cameraTaken() {
        let (state, _) = run(recording(), [
            .camera(.inUse), .finished(durationMS: 3_000, tooBig: false, atMS: 6_000),
        ])
        #expect(state.phase == .review(durationMS: 3_000))
        #expect(state.notice == .camera(.inUse))
        #expect(M.CameraProblem.inUse.sentence == String(localized: "The camera is being used by another app."))
        #expect(M.CameraProblem.multitasking.sentence
                == String(localized: "The camera isn't available while other apps are on screen."))
    }

    @Test("closing the window: PREVIEW lets it go; a take stops, then asks; Keep cancels")
    func closingWindow() {
        let free = M.step(preview(), .closing(.window))
        #expect(free.0.phase == .closed)
        #expect(free.1 == [.stopCamera, .close, .proceed(.window)])
        #expect(preview().closesFreely)
        #expect(!recording().closesFreely)
        #expect(!review().closesFreely)

        let (asked, effects) = run(recording(), [
            .closing(.window), .finished(durationMS: 4_000, tooBig: false, atMS: 6_100),
        ])
        #expect(asked.phase == .review(durationMS: 4_000))
        #expect(asked.question == .closing(.window))
        #expect(!effects.contains(.proceed(.window)))

        let kept = M.step(asked, .answer(delete: false, atMS: 7_000))
        #expect(kept.1.isEmpty, "Keep cancels the close")
        #expect(kept.0.phase == .review(durationMS: 4_000))

        let deleted = M.step(asked, .answer(delete: true, atMS: 7_000))
        #expect(deleted.0.phase == .closed)
        #expect(deleted.1 == [.stopPlayback, .deleteClip, .close, .proceed(.window)])
    }

    @Test("quitting over REVIEW asks; Delete quits after all")
    func quitting() {
        let asked = M.step(review(), .closing(.quit))
        #expect(asked.0.question == .closing(.quit))
        #expect(M.step(asked.0, .answer(delete: true, atMS: 30_000)).1.contains(.proceed(.quit)))
    }

    @Test("a take too short to keep lets the close go on")
    func closingShortTake() {
        let (state, effects) = run(recording(), [
            .closing(.quit), .finished(durationMS: 300, tooBig: false, atMS: 2_400),
        ])
        #expect(state.phase == .closed)
        #expect(effects.contains(.proceed(.quit)))
    }

    @Test("sign-out deletes everything recorded and not sent")
    func signOut() {
        let taking = M.step(recording(), .signedOut)
        #expect(taking.0.phase == .closed)
        #expect(taking.1.contains(.cancelRecording))
        let reviewing = M.step(review(), .signedOut)
        #expect(reviewing.1.contains(.deleteClip))
        #expect(reviewing.0.phase == .closed)
    }

    // MARK: - Record a voice message instead

    @Test("\"Record a voice message instead\" closes the camera and starts voice — or explains")
    func voiceInstead() {
        let started = M.step(preview(), .voiceInstead(notSent: false))
        #expect(started.0.phase == .closed)
        #expect(started.1 == [.stopCamera, .close, .startVoice])

        let waiting = M.step(preview(), .voiceInstead(notSent: true))
        #expect(waiting.0.phase == .preview)
        #expect(waiting.0.notice == .notSent)
        #expect(waiting.1.isEmpty)
        #expect(M.Notice.notSent.sentence == ComposerSlot.Dimmed.notSent.notice)

        let refusedCamera = M.step(
            M.step(M.State(), .opened(camera: .denied, microphone: .granted, atMS: 0)).0,
            .voiceInstead(notSent: false))
        #expect(refusedCamera.1 == [.close, .startVoice])

        let refusedMic = M.step(
            M.step(M.State(), .opened(camera: .granted, microphone: .denied, atMS: 0)).0,
            .voiceInstead(notSent: false))
        #expect(refusedMic.1.isEmpty, "voice needs the microphone too")
    }

    @Test("every announcement is the catalogue's sentence")
    func words() {
        #expect(M.Announcement.cameraReady.text == String(localized: "Camera ready"))
        #expect(M.Announcement.recordingVideo.text == String(localized: "Recording video"))
        #expect(M.Announcement.tenSecondsLeft.text == String(localized: "10 seconds left"))
        #expect(M.Announcement.sent.text == String(localized: "Video message sent"))
        #expect(M.Announcement.cameraTurnedOff.text == String(localized: "Camera turned off"))
        #expect(M.Announcement.stoppedAtLimit.text == String(localized: "Recording stopped at one minute."))
    }

    // MARK: - VoiceOver: the app's own speech stays out of the clip (S6)

    @Test("with VoiceOver, Record says \"Recording video\" FIRST and opens the microphone only after")
    func speaksBeforeTheMicrophone() {
        let (state, effects) = M.step(preview(), .record(atMS: 2_000, speakFirst: true))
        #expect(state.phase == .recording(startMS: 2_000))
        #expect(state.speaking)
        #expect(!effects.contains(.startRecording), "the microphone opened while the app was still speaking")
        #expect(!effects.contains(.announce(.recordingVideo)), "said twice: once spoken, once announced")
        #expect(effects == [.holdOrientation(true), .haptic, .speakThenRecord, .taught])
        // The clock has not started: no length is counted while it speaks.
        #expect(state.recordedMS(atMS: 2_900) == 0)

        let (spoken, opened) = M.step(state, .spoken(atMS: 3_100))
        #expect(opened == [.startRecording])
        #expect(!spoken.speaking)
        #expect(spoken.phase == .recording(startMS: 3_100), "the clock starts with the microphone")
        #expect(spoken.recordedMS(atMS: 4_100) == 1_000)
        // A late or repeated "spoken" changes nothing.
        #expect(M.step(spoken, .spoken(atMS: 5_000)).1.isEmpty)
        #expect(M.step(preview(), .spoken(atMS: 5_000)).1.isEmpty)
    }

    @Test("without VoiceOver Record opens the microphone at once, as before")
    func noSpeechWithoutVoiceOver() {
        let effects = M.step(preview(), .record(atMS: 2_000, speakFirst: false)).1
        #expect(effects.contains(.startRecording))
        #expect(!effects.contains(.speakThenRecord))
    }

    @Test("Magic Tap under VoiceOver records the same way: words first")
    func magicTapSpeaksFirst() {
        let effects = M.step(preview(), .magicTap(atMS: 2_000, speakFirst: true)).1
        #expect(effects.contains(.speakThenRecord))
        #expect(!effects.contains(.startRecording))
    }

    @Test("the length limit is not spent on the words: no cap while speaking")
    func noCapWhileSpeaking() {
        let speaking = M.step(preview(), .record(atMS: 2_000, speakFirst: true)).0
        let (state, effects) = M.step(speaking, .tick(atMS: 2_000 + 70_000))
        #expect(state.phase == .recording(startMS: 2_000))
        #expect(effects.isEmpty)
    }

    @Test("Stop while it speaks: nothing was recorded, so PREVIEW says it was too short")
    func stopWhileSpeaking() {
        let speaking = M.step(preview(), .record(atMS: 2_000, speakFirst: true)).0
        let (state, effects) = M.step(speaking, .stop(atMS: 2_700))
        #expect(state.phase == .preview)
        #expect(!state.speaking)
        #expect(state.notice == .tooShort)
        #expect(effects.contains(.cancelRecording), "the words in flight are forgotten")
        #expect(!effects.contains(.stopRecording), "nothing was ever started to stop")
        #expect(effects.contains(.holdOrientation(false)))
        // And the "spoken" that comes after does not open the microphone.
        #expect(M.step(state, .spoken(atMS: 3_000)).1.isEmpty)
    }

    @Test("a call while it speaks closes the recorder; Delete goes back to PREVIEW")
    func interruptedWhileSpeaking() {
        let speaking = M.step(preview(), .record(atMS: 2_000, speakFirst: true)).0
        let call = M.step(speaking, .callStarted)
        #expect(call.0.phase == .closed)
        #expect(call.1.contains(.cancelRecording))
        #expect(!call.1.contains(.stopRecording))

        let deleted = M.step(speaking, .delete(atMS: 2_800))
        #expect(deleted.0.phase == .preview)
        #expect(!deleted.0.speaking)
        #expect(M.step(deleted.0, .spoken(atMS: 3_000)).1.isEmpty)
    }

    // MARK: - A Stop that lands after the limit (S1.7)

    @Test("the length limit turns Stop into Send: a tap meant for Stop just after does not send")
    func limitGuardsSend() {
        let rec = recording()
        let capped = run(rec, [
            .tick(atMS: 2_000 + rec.capMS),
            .finished(durationMS: rec.capMS, tooBig: false, atMS: 2_000 + rec.capMS + 80),
        ]).0
        #expect(capped.phase == .review(durationMS: rec.capMS))
        let early = M.step(capped, .send(atMS: 2_000 + rec.capMS + 300))
        #expect(!early.1.contains(.send(round: true)), "a length limit running out sent the clip")
        #expect(early.0.phase == .review(durationMS: rec.capMS))
        let later = M.step(capped, .send(atMS: 2_000 + rec.capMS + 80 + RecordRules.activationGuardMS))
        #expect(later.1.contains(.send(round: true)))
    }

    @Test("an interruption into REVIEW guards Send the same way")
    func interruptionGuardsSend() {
        let stopped = run(recording(), [
            .microphoneTaken,
            .finished(durationMS: 20_000, tooBig: false, atMS: 22_000),
        ]).0
        #expect(stopped.phase == .review(durationMS: 20_000))
        #expect(!M.step(stopped, .send(atMS: 22_200)).1.contains(.send(round: true)))
    }
}

/// A seeded xorshift, so the walk is the same every run.
struct SystemRandomNumberGeneratorStandIn {
    private var state: UInt64
    init(seed: UInt64) { state = seed &* 0x9E37_79B9_7F4A_7C15 | 1 }
    mutating func next() -> UInt64 {
        state ^= state << 13
        state ^= state >> 7
        state ^= state << 17
        return state
    }
}
