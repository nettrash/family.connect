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
//  Tap to start, tap to stop, rather than press-and-hold: hold-to-talk is a
//  phone gesture that has no sensible desktop equivalent, and the composer
//  is shared with a Mac where the mouse button would have to stay down for
//  the length of the message.
//
//  Permission is asked by the system on first record. iOS carries
//  NSMicrophoneUsageDescription already (video capture records sound); the
//  Mac additionally needs the `device.audio-input` sandbox entitlement, or
//  the recorder is handed a silent input with no error.
//

import AVFoundation
import Observation

@MainActor
@Observable
final class AudioRecorder {

    /// Where a finished recording landed. Nil until one has been made.
    private(set) var recordedURL: URL?
    private(set) var isRecording = false
    /// Seconds elapsed, for the composer's counter.
    private(set) var elapsed: TimeInterval = 0
    /// Set when recording could not start — almost always a refused
    /// microphone permission.
    private(set) var failed = false

    @ObservationIgnored private var recorder: AVAudioRecorder?
    @ObservationIgnored private var ticker: Task<Void, Never>?

    /// Longest single note. A voice message is not a podcast, and the
    /// 100 MB ceiling is nowhere near reachable in AAC — this is about the
    /// listener, not the disk.
    static let maxDuration: TimeInterval = 5 * 60

    func start() async {
        guard !isRecording else { return }
        failed = false
        recordedURL = nil
        elapsed = 0

        guard await requestPermission() else {
            failed = true
            return
        }

        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("fc-voice-\(UUID().uuidString)")
            .appendingPathExtension("m4a")

        // AAC in an MP4 container: `ftyp` at offset 4, which is exactly what
        // the server checks for `audio/mp4`. Mono at 44.1k — a voice note
        // gains nothing from stereo and doubles for free.
        let settings: [String: Any] = [
            AVFormatIDKey: Int(kAudioFormatMPEG4AAC),
            AVSampleRateKey: 44_100.0,
            AVNumberOfChannelsKey: 1,
            AVEncoderAudioQualityKey: AVAudioQuality.medium.rawValue,
        ]

        do {
            #if os(iOS)
            // Without this the recorder is silent when anything else has
            // the session, and playback afterwards routes to the earpiece.
            let session = AVAudioSession.sharedInstance()
            try session.setCategory(.playAndRecord, mode: .default, options: [.defaultToSpeaker])
            try session.setActive(true)
            #endif
            let recorder = try AVAudioRecorder(url: url, settings: settings)
            recorder.record(forDuration: Self.maxDuration)
            self.recorder = recorder
            isRecording = true
            startTicking()
        } catch {
            failed = true
        }
    }

    /// Stop and hand back the file, or nil if nothing usable was captured.
    @discardableResult
    func stop() -> URL? {
        ticker?.cancel()
        ticker = nil
        guard let recorder else { return nil }
        let url = recorder.url
        recorder.stop()
        self.recorder = nil
        isRecording = false
        #if os(iOS)
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        #endif

        // A recording that never got any audio is a file of a few bytes;
        // sending it would put an unplayable bubble in the thread.
        guard MediaPrep.fileSize(of: url) > 1024 else {
            try? FileManager.default.removeItem(at: url)
            return nil
        }
        recordedURL = url
        return url
    }

    /// Abandon it and delete the file — the way out of a recording you did
    /// not mean to start.
    func cancel() {
        ticker?.cancel()
        ticker = nil
        if let recorder {
            let url = recorder.url
            recorder.stop()
            try? FileManager.default.removeItem(at: url)
        }
        recorder = nil
        isRecording = false
        elapsed = 0
        recordedURL = nil
        #if os(iOS)
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        #endif
    }

    private func startTicking() {
        ticker = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(200))
                guard let self, self.isRecording else { return }
                self.elapsed = self.recorder?.currentTime ?? self.elapsed
                // `record(forDuration:)` stops the hardware at the cap, but
                // nothing tells the UI — so the counter would sit there
                // climbing past a recording that had already ended.
                if self.elapsed >= Self.maxDuration { return }
            }
        }
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
    static func timeLabel(_ seconds: TimeInterval) -> String {
        let whole = max(0, Int(seconds.rounded()))
        return String(format: "%d:%02d", whole / 60, whole % 60)
    }
}
