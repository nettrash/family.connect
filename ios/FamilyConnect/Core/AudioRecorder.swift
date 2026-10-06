//
//  AudioRecorder.swift
//  FamilyConnect
//
//  Recording a voice note, on both Apple platforms.
//
//  Records straight into AAC-in-MP4 (`.m4a`), which is what the server's
//  magic-number check recognises as `audio/mp4` — so nothing is re-encoded
//  on the way out and a recording is uploadable the moment it stops.
//
//  Tap to start, tap to stop, everywhere: hold-to-talk is a phone gesture
//  that has no sensible desktop equivalent, and the composer is shared with a
//  Mac where the mouse button would have to stay down for the length of the
//  message. Since #79's Phase 1 a phone or tablet ALSO lets a finger hold the
//  microphone to talk — a shortcut, never the only way, decided by the shared
//  rules (RecordGesture) and driven by the composer (VoiceComposer); this
//  recorder neither knows nor cares which of the two started it.
//
//  Permission is asked by the system on first record. iOS carries
//  NSMicrophoneUsageDescription already (video capture records sound); the
//  Mac additionally needs the `device.audio-input` sandbox entitlement, or
//  the recorder is handed a silent input with no error.
//
//  #79, Phase 0 — the recorder made safe (docs/audio-video-messages-2026-10-04.md,
//  S2.5, S2.8, S4). Four things it used to get wrong:
//
//  - It deactivated the shared audio session on every stop and cancel, and
//    deactivating a session while audio runs "stops the objects" in it — a
//    call's audio included. It no longer touches a session a call holds.
//  - At the five-minute cap the hardware stopped and nothing said so: the
//    counter froze and nothing was staged. The cap is now REPORTED
//    (`onEnded`, reason `.cap`) and the composer puts the note into review.
//  - Siri, an alarm or another app taking the microphone stopped it
//    silently. An interruption now stops it and hands what was recorded
//    back to be kept (`.interrupted`) — never sent, never thrown away.
//  - Nothing measured the input. It now meters the PEAK level, the measure
//    the level meter and the silence check share on every client (S2.9).
//

import AVFoundation
import Observation

/// What records the sound: an `AVAudioRecorder` in the app, a fake in a test.
///
/// A seam for the same reason `permissionProvider` is one — the interesting
/// endings (the cap, an encoder failure, the hardware stopping by itself)
/// cannot be produced on demand from a real microphone, and a test that
/// waited five minutes for the cap would never be run.
protocol VoiceRecordingEngine: AnyObject {
    /// Seconds recorded so far.
    var currentTime: TimeInterval { get }
    var isRecording: Bool { get }
    /// Begin, stopping by itself after `duration`. False when it could not
    /// begin at all.
    func record(forDuration duration: TimeInterval) -> Bool
    func stop()
    /// The loudest instant since the previous call, in dBFS.
    func peakPower() -> Float
    /// Set by the recorder. Called when the engine stops BY ITSELF: `true`
    /// at the end of the duration it was given, `false` on an error.
    var onFinish: ((Bool) -> Void)? { get set }
}

/// How the shared audio session is taken and given back.
///
/// Only iOS has one; on the Mac both closures do nothing. A seam so a test
/// can count deactivations — the bug this exists to prevent is a
/// deactivation that should not have happened.
struct AudioSessionControl {
    var activate: () throws -> Void
    var deactivate: () -> Void

    static var system: AudioSessionControl {
        #if os(iOS)
        AudioSessionControl(
            activate: {
                // Without this the recorder is silent when anything else has
                // the session, and playback afterwards routes to the earpiece.
                let session = AVAudioSession.sharedInstance()
                try session.setCategory(.playAndRecord, mode: .default, options: [.defaultToSpeaker])
                try session.setActive(true)
            },
            deactivate: {
                try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
            })
        #else
        AudioSessionControl(activate: {}, deactivate: {})
        #endif
    }
}

@MainActor
@Observable
final class AudioRecorder {

    /// Something recorded, and how long it is — by the recorder's own
    /// clock, the one the person watched count up.
    nonisolated struct Recording: Equatable, Sendable {
        let url: URL
        let duration: TimeInterval
        /// Its shape, 48 lowercase hex digits from every peak the meter read
        /// (`Waveform.fromPeaks`) — sent with the upload so every reader
        /// draws it before downloading a byte (#79, docs/protocol.md, "A
        /// voice note's waveform"). nil only where nothing metered it.
        var waveform: String? = nil
    }

    /// A recording that stopped without the person asking it to.
    nonisolated struct Ended: Equatable, Sendable {
        nonisolated enum Reason: Equatable, Sendable {
            /// The five-minute cap. Into review — never sent by a limit
            /// running out (S1.7, S2.5).
            case cap
            /// Siri, an alarm, another app or the system took the
            /// microphone. Kept as "not sent" (S2.8, S4).
            case interrupted
            /// The recorder itself failed. What is readable is kept, and the
            /// composer says "The recording stopped unexpectedly." (S4).
            case failed
        }

        let reason: Reason
        /// What was recorded; nil when nothing worth keeping was — under a
        /// second, or unreadable — in which case the file is already gone.
        let recording: Recording?
    }

    /// Where a finished recording landed. Nil until one has been made.
    private(set) var recordedURL: URL?
    private(set) var isRecording = false
    /// Seconds elapsed, for the composer's counter.
    private(set) var elapsed: TimeInterval = 0
    /// The loudest instant of the last tick, in dBFS of the PEAK level — the
    /// measure the level meter lights from (S2.9).
    private(set) var peakLevel: Float = AudioRecorder.quietest
    /// Whether any peak since the recording started rose above the silence
    /// line. A recording that never did is digital silence: a muted
    /// microphone, not a quiet room (S1.1, "Silence").
    private(set) var heardSound = false
    /// Every peak the ticker read since the recording started, in dBFS and
    /// in time order: the input of the waveform the note is sent with, and
    /// what the recording row's live waveform scrolls (#79).
    private(set) var peaks: [Float] = []

    /// Why a recording did not start.
    ///
    /// Two causes, and they need DIFFERENT sentences: telling somebody to
    /// grant a permission they have already granted sends them into Settings
    /// to look at a switch that is already on. The same distinction
    /// `LocationProvider.Failure` draws, for the same reason.
    nonisolated enum Failure: Equatable, Sendable {
        /// The person said no to the microphone, now or at some earlier
        /// point. Only this one is worth pointing at Settings for.
        case microphoneDenied
        /// The session or the recorder itself refused — another app holding
        /// the audio session, a device with no input, a disk that will not
        /// take the file.
        case couldNotStart
        /// A call is in progress, in any phase: no recording during a call
        /// (S1.7). The composers refuse before they get here; this is the
        /// backstop, and it is checked again after the permission prompt,
        /// which a call can ring through.
        case callInProgress
    }

    /// How permission is asked. The app passes nothing and gets the real
    /// system prompt; a test supplies an answer, which is the only way to
    /// exercise the denied path — `AVAudioApplication` and `AVCaptureDevice`
    /// cannot be told to refuse. Same seam idea as `APIClient`'s injected
    /// `URLSession`.
    var permissionProvider: (() async -> Bool)?

    /// Whether a call holds the audio session right now. Wired once for the
    /// whole app (`VoiceRecordingArbiter.callIsActive`); a test answers it.
    var callIsActive: () -> Bool = { VoiceRecordingArbiter.shared.callIsActive() }

    /// How the shared session is taken and given back. See the type.
    var audioSession: AudioSessionControl = .system

    /// How the engine is made. See `VoiceRecordingEngine`.
    var makeEngine: (URL, [String: any Sendable]) throws -> any VoiceRecordingEngine = { url, settings in
        try SystemVoiceRecordingEngine(url: url, settings: settings)
    }

    /// Where recordings are written while they run.
    var directory: URL = FileManager.default.temporaryDirectory

    /// The longest single note — `maxDuration`, a seam only so a test can
    /// reach it in a second rather than in five minutes.
    var cap: TimeInterval = AudioRecorder.maxDuration

    /// Told when the recording stopped WITHOUT the person asking: the cap,
    /// an interruption, a failure. Called synchronously, so whoever owns the
    /// composer can keep what was recorded before anything else happens —
    /// not on the next redraw, which an app on its way to the background
    /// may never get.
    var onEnded: ((Ended) -> Void)?

    /// Told after every tick that found the recording still running — what
    /// the composer says "30 seconds left" and "We can't hear anything" by
    /// (#79, Phase 1, S2.5, S2.9). Display only: nothing here decides.
    var onTick: (() -> Void)?

    /// Seconds recorded RIGHT NOW, read from the engine rather than from the
    /// last 200 ms tick: a release or a Send decides on the length (S1.1's
    /// one-second floor), and a tick can be most of a second behind it.
    var recordedNow: TimeInterval {
        guard isRecording, let engine else { return elapsed }
        return max(engine.currentTime, elapsed)
    }

    /// Set when `start()` gives up, cleared when it is called again.
    ///
    /// NOTHING READ THIS UNTIL NOW, which was the bug: the flag was set
    /// faithfully and both composers ignored it, so denying the microphone
    /// made "Record Audio" do nothing at all — no bar, no alert, no
    /// explanation, on either platform.
    private(set) var failure: Failure?

    @ObservationIgnored private var engine: (any VoiceRecordingEngine)?
    @ObservationIgnored private var fileURL: URL?
    @ObservationIgnored private var ticker: Task<Void, Never>?
    @ObservationIgnored private var observers: [any NSObjectProtocol] = []
    /// Bumped by every start AND every stop, cancel or park. A start that is
    /// still waiting on the permission prompt compares it when the answer
    /// comes back: a composer that went away meanwhile, or a recording that
    /// was stopped meanwhile, must not have a microphone opened behind it.
    @ObservationIgnored private var attempt = 0
    /// Ticks in a row that found the engine stopped with nobody having said
    /// why — see `tick()`.
    @ObservationIgnored private var silentStops = 0

    /// Longest single note. A voice message is not a podcast, and the
    /// 100 MB ceiling is nowhere near reachable in AAC — this is about the
    /// listener, not the disk.
    static let maxDuration: TimeInterval = 5 * 60

    /// Under this, a recording something else stopped is deleted rather
    /// than kept: there is nothing worth keeping (S4, `SHORTEST_RECORDING_MS`).
    nonisolated static let shortestKept = TimeInterval(RecordRules.shortestRecordingMS) / 1000

    /// A peak at or below this is digital silence (S1.1, `SILENCE_PEAK_DBFS`).
    nonisolated static let silenceLine = Float(RecordRules.silencePeakDBFS)

    /// The level meter's five bars, lit at these PEAK levels (S2.9).
    nonisolated static let meterSteps: [Float] = [-50, -40, -30, -20, -10]

    /// What `peakPower` reports with no input at all.
    nonisolated static let quietest: Float = -160

    /// The prefix every recording file carries — what the launch sweep
    /// recognises a dead process's recording by (`MediaOutbox`).
    nonisolated static let filePrefix = "fc-voice-"

    /// The protocol's voice-note row (docs/protocol.md, "Preparing media
    /// before upload"): M4A, AAC-LC, mono, 44.1 kHz, 64 000 bit/s.
    ///
    /// AAC in an MP4 container: `ftyp` at offset 4, which is exactly what
    /// the server checks for `audio/mp4`. Mono — a voice note gains nothing
    /// from stereo and doubles for free.
    ///
    /// The bitrate is SAID, not implied. It used to be
    /// `AVEncoderAudioQuality.medium`, which leaves the rate to the encoder,
    /// and a number four recorders are meant to agree on cannot be left to
    /// each platform's idea of "medium". Constant, so the recording is the
    /// rate the protocol names; and a voice note is the one audio the upload
    /// path never re-encodes, so this is the only place it is decided.
    nonisolated static let settings: [String: any Sendable] = [
        AVFormatIDKey: Int(kAudioFormatMPEG4AAC),
        AVSampleRateKey: 44_100.0,
        AVNumberOfChannelsKey: 1,
        AVEncoderBitRateKey: MediaPlan.voiceNoteBitrate,
        AVEncoderBitRateStrategyKey: AVAudioBitRateStrategy_Constant,
    ]

    /// What to tell the composer when a recording did not start.
    ///
    /// Lives here rather than in each view so the two cannot drift, and so
    /// the rule that matters — a denial and a failure say DIFFERENT things —
    /// can be tested. Sending somebody to Settings for a permission they
    /// already granted is the mistake this is shaped to prevent.
    static func message(for failure: Failure) -> String {
        switch failure {
        case .microphoneDenied:
            #if os(macOS)
            // A Mac has no Settings app, and the switch is three levels deep.
            return String(localized: "Family needs permission to use your microphone. Turn it on in System Settings › Privacy & Security › Microphone.")
            #else
            return String(localized: "Family needs permission to use your microphone. Turn it on in Settings.")
            #endif
        case .couldNotStart:
            return String(localized: "Couldn't start recording.")
        case .callInProgress:
            return String(localized: "You can record a message after the call.")
        }
    }

    /// How many of the level meter's bars a peak lights (S2.9).
    nonisolated static func litBars(peak: Float) -> Int {
        meterSteps.filter { peak >= $0 }.count
    }

    /// Whether a peak is sound rather than digital silence.
    nonisolated static func isAudible(peak: Float) -> Bool {
        peak > silenceLine
    }

    func start() async {
        guard !isRecording else { return }
        attempt += 1
        let thisAttempt = attempt
        failure = nil
        recordedURL = nil
        elapsed = 0
        peakLevel = Self.quietest
        heardSound = false
        peaks = []

        guard !callIsActive() else {
            failure = .callInProgress
            return
        }
        let granted = await (permissionProvider ?? requestPermission)()
        // Abandoned while the prompt was up — the composer went away, or
        // whatever it was asking for was stopped: nothing may open the
        // microphone now, and nothing is left to report to.
        guard thisAttempt == attempt, !isRecording else { return }
        guard granted else {
            failure = .microphoneDenied
            return
        }
        // A call can ring while a permission prompt is up.
        guard !callIsActive() else {
            failure = .callInProgress
            return
        }

        let url = directory
            .appendingPathComponent("\(Self.filePrefix)\(UUID().uuidString)")
            .appendingPathExtension("m4a")

        do {
            try audioSession.activate()
            let engine = try makeEngine(url, Self.settings)
            engine.onFinish = { [weak self] successfully in
                self?.engineStopped(successfully: successfully)
            }
            guard engine.record(forDuration: cap) else {
                throw CocoaError(.fileWriteUnknown)
            }
            self.engine = engine
            self.fileURL = url
            silentStops = 0
            isRecording = true
            startTicking()
            observeTheSystem()
        } catch {
            failure = .couldNotStart
            releaseSession()
            try? FileManager.default.removeItem(at: url)
        }
    }

    /// Stop and hand back the file, or nil if nothing usable was captured.
    @discardableResult
    func stop() -> URL? {
        stopRecording()?.url
    }

    /// Stop and hand back what was recorded, with its length — nil if
    /// nothing usable was captured, and the file is gone then.
    ///
    /// The person's own Stop: today's floor, which is "a file with sound in
    /// it" (more than 1024 bytes — a recording that never got any audio is a
    /// file of a few bytes, and sending it would put an unplayable bubble in
    /// the thread).
    func stopRecording() -> Recording? {
        finish(keepingFrom: 0)
    }

    /// Stop a recording the person did NOT stop — they left the chat, the
    /// app went to the background, a call rang — and hand it back to be
    /// kept. Under a second there is nothing worth keeping: nil, and the
    /// file is gone (S4).
    func stopKeeping() -> Recording? {
        finish(keepingFrom: Self.shortestKept)
    }

    /// Abandon it and delete the file — the way out of a recording you did
    /// not mean to start.
    func cancel() {
        attempt += 1
        let url = fileURL
        let wasRunning = engine != nil
        if let engine {
            engine.onFinish = nil
            engine.stop()
        }
        tearDown()
        if let url { try? FileManager.default.removeItem(at: url) }
        elapsed = 0
        recordedURL = nil
        if wasRunning { releaseSession() }
    }

    /// The system took the microphone: Siri, an alarm, a call, another app.
    /// Stop and keep (S4) — never sent, and under a second, deleted.
    func interrupted() {
        guard isRecording else { return }
        let recording = finish(keepingFrom: Self.shortestKept)
        onEnded?(Ended(reason: .interrupted, recording: recording))
    }

    /// One look at the engine — what the ticker does every 200 ms, callable
    /// on its own so a test does not have to wait on a clock.
    func tick() {
        guard isRecording, let engine else { return }
        guard engine.isRecording else {
            // Stopped, and nobody said why. At the cap that is the cap —
            // `record(forDuration:)` stops the hardware and the delegate may
            // not have spoken yet. Anywhere else it is the input going away
            // with no notification (a microphone unplugged, an interruption
            // that was never posted): given a tick for the delegate to speak
            // first, it is kept as an interruption, which ends the same way
            // the explicit one does.
            if elapsed >= cap - 1 {
                engineStopped(successfully: true)
            } else {
                silentStops += 1
                if silentStops >= 2 { interrupted() }
            }
            return
        }
        silentStops = 0
        elapsed = engine.currentTime
        let peak = engine.peakPower()
        peakLevel = peak
        peaks.append(peak)
        if Self.isAudible(peak: peak) { heardSound = true }
        onTick?()
    }

    // MARK: - Ending

    /// Stop the engine and decide what was recorded.
    private func finish(keepingFrom floor: TimeInterval) -> Recording? {
        attempt += 1
        guard let engine, let url = fileURL else { return nil }
        let duration = max(engine.currentTime, elapsed)
        engine.onFinish = nil
        engine.stop()
        tearDown()
        releaseSession()
        return keep(url, duration: duration, floor: floor)
    }

    /// The engine stopped by itself: the cap (`successfully`), or a failure.
    private func engineStopped(successfully: Bool) {
        guard isRecording, let engine, let url = fileURL else { return }
        let duration = successfully ? max(cap, elapsed) : max(engine.currentTime, elapsed)
        engine.onFinish = nil
        if engine.isRecording { engine.stop() }
        attempt += 1
        tearDown()
        releaseSession()
        let recording = keep(url, duration: duration, floor: Self.shortestKept)
        onEnded?(Ended(reason: successfully ? .cap : .failed, recording: recording))
    }

    /// The file if it is worth keeping; otherwise it is deleted.
    private func keep(_ url: URL, duration: TimeInterval, floor: TimeInterval) -> Recording? {
        guard MediaPrep.fileSize(of: url) > 1024, duration >= floor else {
            try? FileManager.default.removeItem(at: url)
            return nil
        }
        recordedURL = url
        return Recording(
            url: url, duration: duration,
            waveform: Waveform.fromPeaks(peaks.map(Double.init)))
    }

    private func tearDown() {
        ticker?.cancel()
        ticker = nil
        for observer in observers { NotificationCenter.default.removeObserver(observer) }
        observers = []
        engine = nil
        fileURL = nil
        isRecording = false
    }

    /// Give the session back — NEVER while a call holds it. Deactivating a
    /// session while audio runs in it "stops the objects", and a stop or a
    /// cancel used to do exactly that to a call's audio, unconditionally.
    private func releaseSession() {
        guard !callIsActive() else { return }
        audioSession.deactivate()
    }

    private func startTicking() {
        ticker = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(200))
                guard let self, self.isRecording else { return }
                self.tick()
            }
        }
    }

    /// iOS only: the session's own interruptions, and the media server
    /// resetting under it (which invalidates the recorder outright).
    private func observeTheSystem() {
        #if os(iOS)
        let center = NotificationCenter.default
        observers.append(center.addObserver(
            forName: AVAudioSession.interruptionNotification, object: nil, queue: .main
        ) { [weak self] note in
            // `.began` only. An interruption that ENDS is not a reason to
            // start recording again: nobody asked to.
            guard let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
                  AVAudioSession.InterruptionType(rawValue: raw) == .began
            else { return }
            MainActor.assumeIsolated { self?.interrupted() }
        })
        observers.append(center.addObserver(
            forName: AVAudioSession.mediaServicesWereResetNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.engineStopped(successfully: false) }
        })
        #endif
    }

    private func requestPermission() async -> Bool {
        #if os(iOS)
        await withCheckedContinuation { continuation in
            AVAudioApplication.requestRecordPermission { granted in
                continuation.resume(returning: granted)
            }
        }
        #else
        // macOS asks through AVCaptureDevice, and a sandboxed app also
        // needs com.apple.security.device.audio-input — without it this
        // returns true and records silence.
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized:
            return true
        case .notDetermined:
            return await AVCaptureDevice.requestAccess(for: .audio)
        default:
            return false
        }
        #endif
    }

    /// `0:07` — what the composer counts up in, and what a bubble shows.
    /// Nonisolated: pure, and the shared record rules (RecordGesture) say
    /// "Ready to review, 0:42" from outside the main actor.
    nonisolated static func timeLabel(_ seconds: TimeInterval) -> String {
        let whole = max(0, Int(seconds.rounded()))
        return String(format: "%d:%02d", whole / 60, whole % 60)
    }
}

/// The real engine: `AVAudioRecorder`, metering on.
final class SystemVoiceRecordingEngine: NSObject, VoiceRecordingEngine, AVAudioRecorderDelegate {
    private let recorder: AVAudioRecorder
    var onFinish: ((Bool) -> Void)?

    init(url: URL, settings: [String: any Sendable]) throws {
        recorder = try AVAudioRecorder(url: url, settings: settings)
        super.init()
        recorder.delegate = self
        recorder.isMeteringEnabled = true
    }

    var currentTime: TimeInterval { recorder.currentTime }
    var isRecording: Bool { recorder.isRecording }

    func record(forDuration duration: TimeInterval) -> Bool {
        recorder.record(forDuration: duration)
    }

    func stop() {
        recorder.stop()
    }

    func peakPower() -> Float {
        recorder.updateMeters()
        return recorder.peakPower(forChannel: 0)
    }

    // The delegate's thread is not promised, so both hop to the main actor.
    // `stop()` clears `onFinish` before it stops the recorder, which is what
    // keeps the person's own Stop from arriving here as an ending.

    nonisolated func audioRecorderDidFinishRecording(_ recorder: AVAudioRecorder, successfully flag: Bool) {
        Task { @MainActor [weak self] in self?.onFinish?(flag) }
    }

    nonisolated func audioRecorderEncodeErrorDidOccur(_ recorder: AVAudioRecorder, error: (any Error)?) {
        Task { @MainActor [weak self] in self?.onFinish?(false) }
    }
}
