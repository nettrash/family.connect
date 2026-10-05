//
//  VoiceRecordingArbiterTests.swift
//  FamilyConnectTests
//
//  One recording at a time in the whole app, and the app-wide events that
//  stop one (#79, docs/audio-video-messages-2026-10-04.md, S1.7, S4, S8.3).
//  Each test builds its own arbiter, which watches nothing — the system's
//  notifications reach the app's one, and these drive the same entry points
//  those notifications call.
//

import Foundation
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Voice recording arbiter: one microphone, and what makes it let go")
struct VoiceRecordingArbiterTests {

    final class Log {
        var letGo: [String] = []
        var awake: [Bool] = []
    }

    private func makeArbiter(_ log: Log, callSharesTheScreenFlag: Bool = false) -> VoiceRecordingArbiter {
        let arbiter = VoiceRecordingArbiter()
        arbiter.keepAwake = { log.awake.append($0) }
        arbiter.callSharesTheScreenFlag = callSharesTheScreenFlag
        return arbiter
    }

    /// A composer: its letting go is logged, and — as the real ones do — it
    /// gives the microphone back when it has parked.
    private func claim(
        _ name: String, _ id: UUID, on arbiter: VoiceRecordingArbiter, log: Log, window: AnyObject? = nil
    ) {
        arbiter.claim(id, window: window) { how in
            log.letGo.append("\(name):\(how == .park ? "park" : "discard")")
            arbiter.release(id)
        }
    }

    @Test("starting a second recording parks the first, in another chat or another window")
    func secondClaimParksTheFirst() {
        let log = Log()
        let arbiter = makeArbiter(log)
        let first = UUID(), second = UUID()
        claim("first", first, on: arbiter, log: log)

        claim("second", second, on: arbiter, log: log)

        #expect(log.letGo == ["first:park"])
        #expect(arbiter.holder == second, "the second recording lost the microphone to the first's release")
        #expect(arbiter.isRecording)
    }

    @Test("the same composer starting again parks nothing")
    func reclaimParksNothing() {
        let log = Log()
        let arbiter = makeArbiter(log)
        let id = UUID()
        claim("one", id, on: arbiter, log: log)
        claim("one", id, on: arbiter, log: log)

        #expect(log.letGo.isEmpty)
        #expect(arbiter.holder == id)
    }

    @Test("a release by a composer that does not hold the microphone changes nothing")
    func strayReleaseIsIgnored() {
        let log = Log()
        let arbiter = makeArbiter(log)
        let holder = UUID()
        claim("holder", holder, on: arbiter, log: log)

        arbiter.release(UUID())

        #expect(arbiter.holder == holder)
        #expect(log.awake == [true])
    }

    @Test("sign-out throws the recording away instead of keeping it")
    func signOutDiscards() {
        let log = Log()
        let arbiter = makeArbiter(log)
        claim("one", UUID(), on: arbiter, log: log)

        arbiter.stopHolder(.discard)

        #expect(log.letGo == ["one:discard"])
        #expect(!arbiter.isRecording)
    }

    @Test("a holder that does not release itself is released anyway")
    func stopHolderReleases() {
        let log = Log()
        let arbiter = makeArbiter(log)
        arbiter.claim(UUID()) { _ in log.letGo.append("silent") }

        arbiter.stopHolder(.park)

        #expect(log.letGo == ["silent"])
        #expect(!arbiter.isRecording)
    }

    // MARK: - Keeping the screen awake

    @Test("the screen stays awake for as long as any recording runs — once, across a hand-over")
    func keepsTheScreenAwake() {
        let log = Log()
        let arbiter = makeArbiter(log)
        let first = UUID(), second = UUID()
        claim("first", first, on: arbiter, log: log)
        claim("second", second, on: arbiter, log: log)
        #expect(log.awake == [true])

        arbiter.release(second)

        #expect(log.awake == [true, false])
    }

    /// iOS: the call screen owns the same idle-timer flag. A recording that
    /// a call has just stopped must not switch it off under a video call.
    @Test("where a call shares the screen's flag, a release during a call leaves it to the call")
    func callKeepsTheScreen() {
        let log = Log()
        let arbiter = makeArbiter(log, callSharesTheScreenFlag: true)
        var call = false
        arbiter.callIsActive = { call }
        let id = UUID()
        claim("one", id, on: arbiter, log: log)
        call = true

        arbiter.release(id)

        #expect(log.awake == [true])
    }

    @Test("where nothing shares it, the screen is let go even during a call")
    func macAlwaysLetsGo() {
        let log = Log()
        let arbiter = makeArbiter(log, callSharesTheScreenFlag: false)
        arbiter.callIsActive = { true }
        let id = UUID()
        claim("one", id, on: arbiter, log: log)

        arbiter.release(id)

        #expect(log.awake == [true, false])
    }

    // MARK: - The system

    @Test("the app going away — background, lock, sleep, screen saver, hide — parks the recording")
    func appWentAwayParks() {
        let log = Log()
        let arbiter = makeArbiter(log)
        claim("one", UUID(), on: arbiter, log: log)

        arbiter.appWentAway()

        #expect(log.letGo == ["one:park"])
        #expect(!arbiter.isRecording)
    }

    @Test("with nothing recording, the app going away does nothing")
    func appWentAwayIdle() {
        let log = Log()
        let arbiter = makeArbiter(log)
        arbiter.appWentAway()
        #expect(log.letGo.isEmpty)
        #expect(log.awake.isEmpty)
    }

    /// S4: "A desktop window only loses focus, still visible — keeps
    /// recording"; minimised or closed, it stops.
    @Test("minimising or closing the recording's own window parks it; another window's does not")
    func onlyItsOwnWindow() {
        let log = Log()
        let arbiter = makeArbiter(log)
        let own = NSObject(), other = NSObject()
        claim("one", UUID(), on: arbiter, log: log, window: own)

        arbiter.windowWentAway(other)
        #expect(log.letGo.isEmpty)
        #expect(arbiter.isRecording)

        arbiter.windowWentAway(own)
        #expect(log.letGo == ["one:park"])
    }

    @Test("quitting keeps every composer's unsent voice, the recording first")
    func quitKeepsEverything() {
        let log = Log()
        let arbiter = makeArbiter(log)
        let recordingOne = UUID(), reviewing = UUID(), gone = UUID()
        claim("recording", recordingOne, on: arbiter, log: log)
        arbiter.keepOnQuit(recordingOne) { log.letGo.append("recording:kept") }
        arbiter.keepOnQuit(reviewing) { log.letGo.append("reviewing:kept") }
        arbiter.keepOnQuit(gone) { log.letGo.append("gone:kept") }
        arbiter.forgetOnQuit(gone)

        arbiter.appWillQuit()

        #expect(log.letGo.first == "recording:park")
        #expect(Set(log.letGo.dropFirst()) == ["recording:kept", "reviewing:kept"])
        #expect(!arbiter.isRecording)
    }
}
