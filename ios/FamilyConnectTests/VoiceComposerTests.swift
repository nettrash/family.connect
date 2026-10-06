//
//  VoiceComposerTests.swift
//  FamilyConnectTests
//
//  #79, Phase 1: the voice half of the composer, driven end to end through
//  its seams (docs/audio-video-messages-2026-10-04.md, S2, S4, S6) — a fake
//  recorder engine, a clock that moves only when the test says, and spies
//  for everything only the composer can do.
//
//  The reducer's every step is pinned by RecordVectorTests against the
//  shared reference. What is pinned HERE is the part no vector can see: that
//  each effect does the thing it names — the microphone opens and closes,
//  the arbiter is claimed and given back, the Send arrow hands the note to
//  the outbox at once, an interruption keeps it as "not sent", and nothing
//  recorded is ever dropped. (The hold, its Undo window and the "sending"
//  entry it parked are gone since 2026-10-06; so are their tests.)
//

import Foundation
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Voice composer: the slot's recording and the interruptions", .serialized)
struct VoiceComposerTests {

    // MARK: - The harness

    /// A recorder engine that records nothing real; it writes a file big
    /// enough to pass the recorder's 1024-byte floor.
    @MainActor
    final class Engine: VoiceRecordingEngine {
        let url: URL
        var currentTime: TimeInterval = 0
        var isRecording = false
        var onFinish: ((Bool) -> Void)?
        var loud = false

        init(url: URL) { self.url = url }

        func record(forDuration duration: TimeInterval) -> Bool {
            try? Data(count: 8192).write(to: url)
            isRecording = true
            return true
        }

        func stop() { isRecording = false }

        func peakPower() -> Float { loud ? -20 : AudioRecorder.quietest }

        /// What `AVAudioRecorder` does at its duration (`true`) or on an
        /// encoder error (`false`).
        func stopsByItself(successfully: Bool) {
            isRecording = false
            onFinish?(successfully)
        }
    }

    @MainActor
    final class Harness {
        let root: URL
        let arbiter = VoiceRecordingArbiter()
        let nowPlaying = NowPlaying()
        var voice: VoiceComposer!

        var now: UInt64 = 10_000
        var engine: Engine?
        var callActive = false
        var permission: RecordGesture.Permission = .granted
        var answer = true
        var voiceOver = false

        var announced: [String] = []
        var spoken: [String] = []
        var reply: ReplyToDTO? = ReplyToDTO(messageID: 41, senderID: 7, excerpt: "Dinner at 8?")
        var sent: [(recording: AudioRecorder.Recording, reply: ReplyToDTO?)] = []
        var sendSucceeds = true
        var reviewed: [AudioRecorder.Recording] = []
        var parked: [AudioRecorder.Recording] = []
        var explained: [ComposerSlot.Dimmed] = []
        var denials = 0
        var hints: [RecordGesture.Hint] = []
        var startFailures: [AudioRecorder.Failure] = []
        var unexpected = 0
        var handsFreeStarts = 0
        var focusReturns = 0
        var blocked: ComposerSlot.Dimmed?

        init() throws {
            let root = URL(fileURLWithPath: NSTemporaryDirectory())
                .appendingPathComponent("voice-composer-\(UUID().uuidString)", isDirectory: true)
            try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
            self.root = root
            arbiter.keepAwake = { _ in }

            let recorder = AudioRecorder()
            recorder.directory = root
            recorder.permissionProvider = { true }
            recorder.callIsActive = { [unowned self] in self.callActive }
            recorder.audioSession = AudioSessionControl(activate: {}, deactivate: {})
            recorder.makeEngine = { [unowned self] url, _ in
                let engine = Engine(url: url)
                self.engine = engine
                return engine
            }

            let voice = VoiceComposer(recorder: recorder)
            voice.clock = { [unowned self] in self.now }
            voice.permission = { [unowned self] in self.permission }
            voice.requestPermission = { [unowned self] in self.answer }
            voice.voiceOverRunning = { [unowned self] in self.voiceOver }
            voice.speak = { [unowned self] sentence in self.spoken.append(sentence) }
            voice.announce = { [unowned self] sentence in self.announced.append(sentence) }
            voice.arbiter = arbiter
            voice.nowPlaying = nowPlaying

            var hooks = VoiceComposer.Hooks()
            hooks.blocked = { [unowned self] in self.blocked }
            hooks.takeReply = { [unowned self] in
                let reply = self.reply
                self.reply = nil
                return reply
            }
            hooks.send = { [unowned self] recording, reply in
                self.sent.append((recording, reply))
                return self.sendSucceeds
            }
            hooks.review = { [unowned self] in self.reviewed.append($0) }
            hooks.park = { [unowned self] in self.parked.append($0) }
            hooks.explain = { [unowned self] in self.explained.append($0) }
            hooks.denied = { [unowned self] in self.denials += 1 }
            hooks.hint = { [unowned self] in self.hints.append($0) }
            hooks.startFailed = { [unowned self] in self.startFailures.append($0) }
            hooks.stoppedUnexpectedly = { [unowned self] in self.unexpected += 1 }
            hooks.startedHandsFree = { [unowned self] in self.handsFreeStarts += 1 }
            hooks.returnFocus = { [unowned self] in self.focusReturns += 1 }
            voice.hooks = hooks
            self.voice = voice
        }

        var recorder: AudioRecorder { voice.recorder }

        /// A tap on the microphone — the control's completed tap, however
        /// long the finger stayed: one activation, at the lift.
        func tap(at time: UInt64) async {
            now = time
            voice.activate()
            await voice.permissionTask?.value
            await voice.startTask?.value
        }

        /// The recorder's clock says this many seconds, and it heard sound.
        func recorded(_ seconds: TimeInterval, loud: Bool = true) {
            engine?.currentTime = seconds
            engine?.loud = loud
            recorder.tick()
        }

        func fileExists(_ recording: AudioRecorder.Recording) -> Bool {
            FileManager.default.fileExists(atPath: recording.url.path)
        }
    }

    // MARK: - Tap to record, the same slot sends

    @Test("a tap records hands-free: the microphone opens, the arbiter is claimed, playback pauses, the field lets go")
    func tapStartsHandsFree() async throws {
        let h = try Harness()
        var paused = 0
        h.nowPlaying.claim(UUID()) { paused += 1 }

        await h.tap(at: 10_000)

        #expect(h.voice.isHandsFree)
        #expect(h.recorder.isRecording, "the microphone did not open")
        #expect(h.arbiter.holder == h.voice.id)
        #expect(paused == 1, "playback kept running under the microphone")
        #expect(h.handsFreeStarts == 1)
        #expect(h.voice.slotFocusRequest == 1, "VoiceOver's focus was not sent to the slot")
        #expect(h.announced == ["Recording"])
        #expect(h.voice.haptic?.haptic == .light)
    }

    @Test("the Send arrow sends what was recorded with the primed reply, and gives everything back")
    func sendArrowSends() async throws {
        let h = try Harness()
        await h.tap(at: 10_000)
        h.recorded(2.4)

        h.now = 12_500
        h.voice.activate()

        let sent = try #require(h.sent.first, "nothing was sent")
        #expect(h.sent.count == 1)
        #expect(sent.reply?.messageID == 41, "the primed reply did not go with the note")
        #expect(abs(sent.recording.duration - 2.4) < 0.01)
        #expect(!h.recorder.isRecording)
        #expect(h.arbiter.holder == nil, "the microphone was not given back")
        #expect(h.voice.state.phase == .idle)
        #expect(h.announced.last == "Voice message sent")
        #expect(h.voice.haptic?.haptic == .success)
        #expect(h.focusReturns == 1)
        #expect(h.parked.isEmpty && h.reviewed.isEmpty, "the Send arrow's note waited somewhere instead of going")
    }

    @Test("under a second the Send arrow discards: too short, never sent, the file gone")
    func tooShortIsDiscarded() async throws {
        let h = try Harness()
        await h.tap(at: 10_000)
        h.recorded(0.6)
        let file = try #require(h.engine?.url)

        h.now = 10_800
        h.voice.activate()

        #expect(h.sent.isEmpty)
        #expect(h.hints == [.tooShort])
        #expect(!FileManager.default.fileExists(atPath: file.path), "a discarded recording stayed on the disk")
        #expect(h.arbiter.holder == nil)
        #expect(h.voice.haptic?.haptic == .warning)
    }

    @Test("a double tap cannot send a blip: the second tap inside 600 ms is ignored")
    func doubleTapIsGuarded() async throws {
        let h = try Harness()
        await h.tap(at: 10_000)
        h.recorded(1.5)

        h.now = 10_400   // 300 ms after the tap that started it
        h.voice.activate()

        #expect(h.voice.isHandsFree, "the guard did not hold the second tap")
        #expect(h.sent.isEmpty)
        #expect(h.recorder.isRecording)
    }

    @Test("a send that emptied the composer guards the microphone it turned into")
    func emptiedGuards() async throws {
        let h = try Harness()
        h.now = 10_000
        h.voice.emptied()

        await h.tap(at: 10_300)

        #expect(h.voice.state.phase == .idle, "a double tap on Send started a recording")
        #expect(!h.recorder.isRecording)
    }

    @Test("typing after a send lifts the guard — the next tap records at once — but never the guard on a recording's Send")
    func typingLiftsTheGuard() async throws {
        let h = try Harness()
        h.now = 10_000
        h.voice.emptied()
        #expect(h.voice.sendIsGuarded)
        // "o", then deleted again: the person's own change.
        h.now = 10_100
        h.voice.otherAction()
        #expect(!h.voice.sendIsGuarded, "a character typed right after a send left the slot guarded")

        await h.tap(at: 10_200)
        #expect(h.voice.isHandsFree, "the tap after typing was swallowed as half of a double tap")

        // While it records, the field is behind the row: nothing typed can
        // lift the guard that keeps a double tap from sending.
        h.recorded(1.5)
        h.now = 10_350
        h.voice.otherAction()
        h.voice.activate()
        #expect(h.voice.isHandsFree, "the guard on the recording's Send was lifted")
        #expect(h.sent.isEmpty)
    }

    @Test("Stop keeps it for review, the shortcut too — a shortcut never sends")
    func stopAndShortcutReview() async throws {
        let h = try Harness()
        await h.tap(at: 10_000)
        h.recorded(3)
        h.now = 13_200
        h.voice.stop()
        #expect(h.reviewed.count == 1)
        #expect(h.announced.last == "Ready to review, 0:03")

        await h.tap(at: 20_000)
        h.recorded(2)
        h.now = 22_200
        h.voice.record(besideDraft: false)
        #expect(h.reviewed.count == 2, "the shortcut did not stop into review")
        #expect(h.sent.isEmpty, "a shortcut sent")
    }

    @Test("beside words the slot is Stop: it stages the note, and a double tap cannot then send it")
    func besideDraftStops() async throws {
        let h = try Harness()
        h.now = 10_000
        h.voice.record(besideDraft: true)
        await h.voice.startTask?.value
        #expect(h.voice.isBesideDraft)
        h.recorded(4)

        h.now = 14_000
        h.voice.activate()

        #expect(h.reviewed.count == 1)
        #expect(h.sent.isEmpty)
        #expect(h.voice.sendIsGuarded, "a row-5 Send right after the Stop square was not guarded")
        h.now = 14_700
        #expect(!h.voice.sendIsGuarded)
    }

    @Test("✕ on a staged item is the person's change: after a Stop beside words, taking the note off lifts the guard — Send sends at once, as on Android and the web")
    func takingAnItemOffLiftsTheGuard() async throws {
        let h = try Harness()
        h.now = 10_000
        h.voice.record(besideDraft: true)
        await h.voice.startTask?.value
        h.recorded(4)
        h.now = 14_000
        h.voice.activate()
        #expect(h.voice.sendIsGuarded)

        var removed = 0
        h.now = 14_100
        h.voice.tookOff(voiceNote: true) { removed += 1 }

        #expect(removed == 1)
        #expect(!h.voice.sendIsGuarded, "a Send right after ✕ on the note was still guarded")
        #expect(h.announced.last == "Recording deleted")

        // A picked file is not a recording: taken off without that word.
        let said = h.announced.count
        h.voice.tookOff(voiceNote: false) { removed += 1 }
        #expect(removed == 2)
        #expect(h.announced.count == said)
    }

    // MARK: - Delete

    @Test("Delete under ten seconds deletes at once; from ten it stops first and asks")
    func deleteAsksFromTen() async throws {
        let h = try Harness()
        await h.tap(at: 10_000)
        h.recorded(4)
        h.now = 14_200
        h.voice.delete()
        #expect(!h.recorder.isRecording)
        #expect(h.voice.state.phase == .idle)

        for keep in [true, false] {
            let asked = try Harness()
            await asked.tap(at: 10_000)
            asked.recorded(12)
            asked.now = 22_200
            asked.voice.delete()
            #expect(asked.voice.isAskingDelete)
            #expect(!asked.recorder.isRecording, "the recording was not stopped before the question")
            #expect(asked.arbiter.holder == nil)
            asked.now = 25_000
            asked.voice.answerDelete(!keep)
            #expect(asked.reviewed.count == (keep ? 1 : 0))
        }
    }

    @Test("an interruption while 'Delete this recording?' is asked keeps the recording")
    func interruptionWhileAsking() async throws {
        let h = try Harness()
        await h.tap(at: 10_000)
        h.recorded(12)
        h.now = 22_200
        h.voice.delete()

        h.voice.interrupt()

        #expect(h.parked.count == 1, "the unanswered question lost the recording")
        #expect(h.fileExists(try #require(h.parked.first)))
    }

    // MARK: - Interruptions (S4)

    @Test("leaving, the background or a call parks a recording of a second or more, deletes a shorter one")
    func interruptionParks() async throws {
        let h = try Harness()
        await h.tap(at: 10_000)
        h.recorded(3)
        h.now = 13_200
        h.voice.interrupt()
        #expect(h.parked.count == 1)
        #expect(h.sent.isEmpty && h.reviewed.isEmpty, "an interruption sent or reviewed")
        #expect(h.arbiter.holder == nil)

        let short = try Harness()
        await short.tap(at: 10_000)
        short.recorded(0.5)
        short.now = 10_700
        short.voice.interrupt()
        #expect(short.parked.isEmpty)
        #expect(!short.recorder.isRecording)
    }

    @Test("Siri or another app taking the microphone parks what was recorded")
    func systemInterruptionParks() async throws {
        let h = try Harness()
        await h.tap(at: 10_000)
        h.recorded(3)

        h.recorder.interrupted()

        #expect(h.parked.count == 1)
        #expect(h.voice.state.phase == .idle)
        #expect(h.arbiter.holder == nil)
    }

    @Test("a recorder that fails keeps what is readable and says so")
    func failureParksAndSays() async throws {
        let h = try Harness()
        await h.tap(at: 10_000)
        h.recorded(3)

        h.engine?.stopsByItself(successfully: false)

        #expect(h.parked.count == 1)
        #expect(h.unexpected == 1)
    }

    @Test("a recording the recorder hands back when no step can use it is kept as not sent, never lost")
    func leftoverIsParked() async throws {
        let h = try Harness()
        await h.tap(at: 10_000)
        h.recorded(3)
        h.now = 13_200
        h.voice.stop()
        #expect(h.reviewed.count == 1)
        #expect(h.voice.state.phase == .idle)

        // A late word from the recorder, after the reducer has nothing
        // running that its ending could belong to.
        let late = AudioRecorder.Recording(url: h.root.appendingPathComponent("late.m4a"), duration: 2)
        h.recorder.onEnded?(AudioRecorder.Ended(reason: .interrupted, recording: late))

        #expect(h.parked == [late], "a recording no step took was thrown away")
        #expect(h.reviewed.count == 1 && h.sent.isEmpty, "the leftover was reviewed or sent")
        #expect(h.voice.state.phase == .idle)
        #expect(h.arbiter.holder == nil)
    }

    @Test("five minutes stop into review with the sentence — never sent")
    func capReviews() async throws {
        let h = try Harness()
        h.recorder.cap = 2
        await h.tap(at: 10_000)
        h.recorded(2)

        h.engine?.stopsByItself(successfully: true)

        #expect(h.reviewed.count == 1)
        #expect(h.hints == [.stoppedAtFiveMinutes])
        #expect(h.announced.last == "Recording stopped at five minutes.")
        #expect(h.sent.isEmpty)
    }

    @Test("another composer starting a recording parks this one: one recording in the app")
    func oneRecordingInTheApp() async throws {
        let h = try Harness()
        await h.tap(at: 10_000)
        h.recorded(3)

        let other = try Harness()
        other.voice.arbiter = h.arbiter
        await other.tap(at: 20_000)

        #expect(h.parked.count == 1, "the first composer's recording was not parked")
        #expect(!h.recorder.isRecording)
        #expect(h.arbiter.holder == other.voice.id)
    }

    @MainActor
    final class PermissionGate {
        var continuation: CheckedContinuation<Bool, Never>?
        var calls = 0
    }

    @Test("a start that finishes after a newer one began never touches the newer recording")
    func staleStartLeavesTheNewOneAlone() async throws {
        let h = try Harness()
        let gate = PermissionGate()
        // The FIRST open waits on a prompt the test answers late; every
        // later one is answered at once.
        h.recorder.permissionProvider = {
            gate.calls += 1
            if gate.calls == 1 {
                return await withCheckedContinuation { gate.continuation = $0 }
            }
            return true
        }
        h.now = 10_100
        h.voice.activate()
        let stale = h.voice.startTask
        for _ in 0..<50 where gate.continuation == nil { await Task.yield() }
        try #require(gate.continuation != nil, "the first open never reached its prompt")

        // Given up on before it opened, then a new recording, which opens.
        h.now = 10_800
        h.voice.interrupt()
        await h.tap(at: 12_000)
        try #require(h.recorder.isRecording)
        let file = try #require(h.engine?.url)

        // The first open's answer finally arrives.
        gate.continuation?.resume(returning: true)
        await stale?.value

        #expect(h.recorder.isRecording, "a stale start threw the new recording away")
        #expect(h.voice.isHandsFree)
        #expect(FileManager.default.fileExists(atPath: file.path))
    }

    @Test("sign-out throws everything away")
    func discardDeletes() async throws {
        let h = try Harness()
        await h.tap(at: 10_000)
        h.recorded(3)
        let file = try #require(h.engine?.url)

        h.voice.discard()

        #expect(!FileManager.default.fileExists(atPath: file.path))
        #expect(h.voice.state == RecordGesture.HoldState())
        #expect(h.arbiter.holder == nil)
        #expect(h.parked.isEmpty && h.reviewed.isEmpty && h.sent.isEmpty)
    }

    // MARK: - Refusals and permission

    @Test("a dimmed microphone says why, at a tap and from the menu, and never records")
    func blockedExplains() async throws {
        let h = try Harness()
        h.blocked = .call
        await h.tap(at: 10_000)
        h.now = 20_000
        h.voice.record(besideDraft: false)

        #expect(h.explained == [.call, .call])
        #expect(!h.recorder.isRecording)
        #expect(h.arbiter.holder == nil)
    }

    @Test("not yet asked: a tap or the menu prompts and records on Allow")
    func permissionFlows() async throws {
        let h = try Harness()
        h.permission = .notAsked
        await h.tap(at: 10_000)
        #expect(h.recorder.isRecording, "Allow after a tap did not record")

        let menu = try Harness()
        menu.permission = .notAsked
        menu.now = 10_000
        menu.voice.record(besideDraft: true)
        await menu.voice.permissionTask?.value
        await menu.voice.startTask?.value
        #expect(menu.recorder.isRecording, "Allow after the menu did not record")
        #expect(menu.voice.isBesideDraft, "the menu's recording forgot the draft beside it")

        let refused = try Harness()
        refused.permission = .notAsked
        refused.answer = false
        await refused.tap(at: 10_000)
        #expect(refused.denials == 1)
        #expect(!refused.recorder.isRecording)
    }

    @Test("a denied microphone gets the denial notice")
    func deniedSays() async throws {
        let h = try Harness()
        h.permission = .denied
        await h.tap(at: 10_000)
        #expect(h.denials == 1)
        #expect(!h.recorder.isRecording)
    }

    @Test("a recorder that will not open resets the slot and says why")
    func startFailureResets() async throws {
        let h = try Harness()
        h.callActive = true
        await h.tap(at: 10_000)

        #expect(h.startFailures == [.callInProgress])
        #expect(h.voice.state.phase == .idle, "the slot still thinks it records")
        #expect(h.arbiter.holder == nil)
    }

    // MARK: - VoiceOver, Magic Tap, the escape gesture

    /// Holds VoiceOver "speaking" until the test lets it finish.
    @MainActor
    final class SpeechGate {
        var continuation: CheckedContinuation<Void, Never>?
    }

    @Test("with VoiceOver the microphone opens only once 'Recording' has been spoken, and it is said once")
    func voiceOverSpeaksFirst() async throws {
        let h = try Harness()
        h.voiceOver = true
        let gate = SpeechGate()
        h.voice.speak = { sentence in
            h.spoken.append(sentence)
            await withCheckedContinuation { gate.continuation = $0 }
        }

        h.now = 10_000
        h.voice.activate()
        let start = h.voice.startTask
        for _ in 0..<50 where gate.continuation == nil { await Task.yield() }
        #expect(gate.continuation != nil, "nothing was spoken before the microphone")
        #expect(!h.recorder.isRecording, "the microphone opened while VoiceOver was still speaking")

        gate.continuation?.resume()
        await start?.value

        #expect(h.recorder.isRecording)
        #expect(h.spoken == ["Recording"])
        #expect(!h.announced.contains("Recording"), "'Recording' was said twice")
    }

    @Test("Magic Tap stops a recording, pauses the app's playback, and otherwise lets the system have it")
    func magicTapOrder() async throws {
        let h = try Harness()
        #expect(h.voice.magicTap() == false, "Magic Tap took the gesture with nothing to do")
        #expect(h.voice.state.phase == .idle, "Magic Tap started a recording")

        var paused = 0
        h.nowPlaying.claim(UUID()) { paused += 1 }
        #expect(h.voice.magicTap() == true)
        #expect(paused == 1)

        await h.tap(at: 10_000)
        h.recorded(2)
        h.now = 12_500
        #expect(h.voice.magicTap() == true)
        #expect(h.reviewed.count == 1)
    }

    @Test("the escape gesture stops a recording into review, and declines otherwise")
    func escapeStops() async throws {
        let h = try Harness()
        #expect(h.voice.escape() == false)
        await h.tap(at: 10_000)
        h.recorded(2)
        h.now = 12_500
        #expect(h.voice.escape() == true)
        #expect(h.reviewed.count == 1)
    }

    // MARK: - What the row says (S2.5, S2.9)

    @Test("'30 seconds left' and the silence warning are shown and said once each")
    func warningsOnce() async throws {
        let h = try Harness()
        await h.tap(at: 10_000)

        h.recorded(3.2, loud: false)
        #expect(h.voice.showsSilenceWarning)
        h.recorded(3.6, loud: false)
        #expect(h.announced.filter { $0 == "We can't hear anything. Is the microphone muted?" }.count == 1)
        h.recorded(4, loud: true)
        #expect(!h.voice.showsSilenceWarning, "the warning stayed after sound arrived")

        #expect(!h.voice.showsThirtySecondsLeft)
        h.recorded(270)
        h.recorded(271)
        #expect(h.voice.showsThirtySecondsLeft)
        #expect(h.announced.filter { $0 == "30 seconds left" }.count == 1)
    }

    // MARK: - The Mac: a click, and the window it is in (S8.3)

    @Test("a click is an activation: it records hands-free, the same click sends, a double click cannot")
    func clickRecordsAndSends() async throws {
        let h = try Harness()
        h.now = 10_000
        h.voice.activate()
        await h.voice.startTask?.value
        #expect(h.voice.isHandsFree, "a click did not record hands-free")

        h.recorded(1.5)
        h.now = 10_400
        h.voice.activate()
        #expect(h.voice.isHandsFree, "a double click sent a blip")

        h.now = 11_800
        h.voice.activate()
        #expect(h.sent.count == 1)
        #expect(h.voice.state.phase == .idle)
    }

    @Test("the microphone is claimed with the composer's window: minimising THAT window parks it, another's does not")
    func windowGoesWithTheClaim() async throws {
        let h = try Harness()
        let mine = NSObject()
        let another = NSObject()
        h.voice.window = { mine }
        h.now = 10_000
        h.voice.activate()
        await h.voice.startTask?.value
        h.recorded(2.5)

        h.arbiter.windowWentAway(another)
        #expect(h.recorder.isRecording, "another window going away stopped this recording")
        #expect(h.parked.isEmpty)

        h.arbiter.windowWentAway(mine)
        #expect(!h.recorder.isRecording, "its own window going away left the microphone open")
        #expect(h.parked.count == 1, "the recording was not kept as not sent")
        #expect(h.sent.isEmpty, "an interruption sent")
        #expect(h.arbiter.holder == nil)
    }

    // MARK: - What is said is what happened (S2.5, S6)

    /// The reducer says "Voice message sent" after `.send`,
    /// and "Ready to review" after `.review`, believing the step worked. The
    /// composer says them only when it did: a screen reader must never hear
    /// "sent" about a note still on the device (WCAG 4.1.3).

    /// The recorder's file shrinks under its 1024-byte floor, so its own
    /// stop hands back nothing — what a recording that never got audio is.
    private static func underTheFloor(_ h: Harness) throws {
        let url = try #require(h.engine?.url)
        try Data(count: 10).write(to: url)
    }

    @Test("a Send the outbox would not queue is not announced as sent: the failure is said and felt instead")
    func failedSendIsNotAnnouncedAsSent() async throws {
        let h = try Harness()
        h.sendSucceeds = false
        await h.tap(at: 10_000)
        h.recorded(2.4)

        h.now = 12_500
        h.voice.activate()

        #expect(h.sent.count == 1, "the send was not tried")
        #expect(!h.announced.contains("Voice message sent"), "VoiceOver heard \"sent\" about a note still on the device")
        #expect(h.announced.last == "Couldn't send that — try again.")
        #expect(h.voice.haptic?.haptic == .warning, "a failed send played the success haptic")
        #expect(h.focusReturns == 1)
    }

    @Test("a Send whose recording the recorder could not keep says too short — never sent")
    func sendWithNothingKeptSaysTooShort() async throws {
        let h = try Harness()
        await h.tap(at: 10_000)
        h.recorded(2.4)
        try Self.underTheFloor(h)

        h.now = 12_500
        h.voice.activate()

        #expect(h.sent.isEmpty)
        #expect(h.hints == [.tooShort])
        #expect(h.announced.last == "That recording was too short.")
        #expect(!h.announced.contains("Voice message sent"))
        #expect(h.voice.haptic?.haptic == .warning)
        #expect(h.arbiter.holder == nil)
        #expect(h.focusReturns == 1, "too short did not give keyboard focus back to the field")
    }

    @Test("a Stop whose recording the recorder could not keep says too short, not 'Ready to review'")
    func stopWithNothingKeptSaysTooShort() async throws {
        let h = try Harness()
        await h.tap(at: 10_000)
        h.recorded(3)
        try Self.underTheFloor(h)

        h.now = 13_200
        h.voice.stop()

        #expect(h.reviewed.isEmpty)
        #expect(h.hints == [.tooShort])
        #expect(h.announced.last == "That recording was too short.")
        #expect(!h.announced.contains { $0.hasPrefix("Ready to review") }, "\"Ready to review\" with nothing staged")
        #expect(h.voice.haptic?.haptic == .warning)
        #expect(h.focusReturns == 1, "too short did not give keyboard focus back to the field")
    }
}
