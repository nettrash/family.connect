//
//  NowPlaying.swift
//  FamilyConnect
//
//  ONE THING PLAYS AT A TIME, and a recording pauses it (#79,
//  docs/audio-video-messages-2026-10-04.md, S1.7, S2.7, S5.3).
//
//  Before this, two voice notes could play at once (each AudioPlayerView
//  owned its own player and knew nothing of the others), and starting a
//  recording left whatever was playing running under the microphone. This
//  is the small owner the plan calls for: whoever starts playing CLAIMS it,
//  which pauses whoever had it; a recording starting pauses everything.
//
//  It starts small on purpose. Phase 1 needs the claim and the pause; the
//  rest of S5.3's list — pausing for a call, the background, another app's
//  sound and lost headphones, and keeping the screen awake while something
//  plays — arrives with Phase 2's round video, which is when bubbles get the
//  `.playback` session too.
//
//  `LocalVoicePlayer` is the one player that already needs all of it: a
//  voice note IN REVIEW, or one waiting in the "not sent" row, played from
//  the file on this device (S2.7). On iPhone and iPad it plays through
//  `.playback` — heard with the silent switch on, and never through the
//  earpiece a recording session leaves the route on — and gives the session
//  back with `.notifyOthersOnDeactivation` when it stops. A session a call
//  holds is never touched.
//

import AVFoundation
import Foundation
import Observation

@MainActor
@Observable
final class NowPlaying {

    /// The app's one owner.
    static let shared = NowPlaying()

    /// Who is playing now, if anybody.
    private(set) var current: UUID?

    @ObservationIgnored private var pauseCurrent: (() -> Void)?

    var isPlaying: Bool { current != nil }

    /// `id` starts playing: whoever was playing is paused first.
    func claim(_ id: UUID, pause: @escaping () -> Void) {
        let previous = pauseCurrent
        let previousID = current
        current = id
        pauseCurrent = pause
        if let previousID, previousID != id {
            previous?()
        }
    }

    /// `id` stopped by itself, or was paused by its own control. Somebody
    /// else's release changes nothing.
    func release(_ id: UUID) {
        guard current == id else { return }
        current = nil
        pauseCurrent = nil
    }

    /// A recording is starting, or VoiceOver's Magic Tap asked: whatever
    /// plays, pauses. True when something was playing.
    @discardableResult
    func pauseAll() -> Bool {
        guard current != nil else { return false }
        let pause = pauseCurrent
        current = nil
        pauseCurrent = nil
        pause?()
        return true
    }
}

/// How a local voice note takes and gives back the audio session. A seam
/// for the reason `AudioSessionControl` is one: what matters is a session
/// that was NOT touched, and only a test can count that.
struct PlaybackSessionControl {
    var begin: () -> Void
    var end: () -> Void

    static var system: PlaybackSessionControl {
        #if os(iOS)
        PlaybackSessionControl(
            begin: {
                // Never while a call holds the session (S5.3, Decision 23).
                guard !VoiceRecordingArbiter.shared.callIsActive() else { return }
                let session = AVAudioSession.sharedInstance()
                try? session.setCategory(.playback, mode: .default)
                try? session.setActive(true)
            },
            end: {
                guard !VoiceRecordingArbiter.shared.callIsActive(),
                      !VoiceRecordingArbiter.shared.isRecording
                else { return }
                try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
            })
        #else
        PlaybackSessionControl(begin: {}, end: {})
        #endif
    }
}

/// What plays a local file: `AVAudioPlayer` in the app, a fake in a test.
@MainActor
protocol LocalAudioEngine: AnyObject {
    var currentTime: TimeInterval { get set }
    var duration: TimeInterval { get }
    var isPlaying: Bool { get }
    @discardableResult func play() -> Bool
    func pause()
    func stop()
    /// Set by the player: the file played to its end.
    var onFinish: (() -> Void)? { get set }
}

/// A voice note on THIS device, playing: the review chip's and the not-sent
/// row's ▶ (S2.7, S2.8).
@MainActor
@Observable
final class LocalVoicePlayer {

    private(set) var isPlaying = false
    /// Seconds played, for "0:12 / 0:42".
    private(set) var elapsed: TimeInterval = 0
    /// The file it was last asked to play.
    private(set) var url: URL?

    @ObservationIgnored let id = UUID()
    @ObservationIgnored var owner: NowPlaying = .shared
    @ObservationIgnored var session: PlaybackSessionControl = .system
    @ObservationIgnored var makeEngine: (URL) throws -> any LocalAudioEngine = { try SystemLocalAudioEngine(url: $0) }
    @ObservationIgnored private var engine: (any LocalAudioEngine)?
    @ObservationIgnored private var ticker: Task<Void, Never>?

    init() {}

    /// Play `url` from where it was, or from the start once it has ended;
    /// pause when it is playing.
    func toggle(_ url: URL) {
        if isPlaying, self.url == url {
            pause()
        } else {
            play(url)
        }
    }

    func play(_ url: URL) {
        if self.url != url {
            stop()
            self.url = url
        }
        do {
            if engine == nil {
                let made = try makeEngine(url)
                made.onFinish = { [weak self] in self?.finished() }
                engine = made
            }
            guard let engine else { return }
            owner.claim(id) { [weak self] in self?.pause() }
            session.begin()
            if engine.duration > 0, engine.currentTime >= engine.duration - 0.05 {
                engine.currentTime = 0
            }
            guard engine.play() else {
                owner.release(id)
                session.end()
                return
            }
            isPlaying = true
            startTicking()
        } catch {
            owner.release(id)
            self.engine = nil
        }
    }

    func pause() {
        guard isPlaying else { return }
        engine?.pause()
        elapsed = engine?.currentTime ?? elapsed
        isPlaying = false
        stopTicking()
        owner.release(id)
        session.end()
    }

    /// Let the file go — the chip or row went away, or the file is about to.
    func stop() {
        let wasPlaying = isPlaying
        engine?.stop()
        engine = nil
        isPlaying = false
        elapsed = 0
        stopTicking()
        owner.release(id)
        if wasPlaying { session.end() }
    }

    /// One look at the engine — the ticker's job, callable from a test.
    func tick() {
        guard isPlaying, let engine else { return }
        elapsed = engine.currentTime
    }

    private func finished() {
        guard isPlaying else { return }
        isPlaying = false
        elapsed = 0
        engine?.currentTime = 0
        stopTicking()
        owner.release(id)
        session.end()
    }

    private func startTicking() {
        stopTicking()
        ticker = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(200))
                guard let self, self.isPlaying else { return }
                self.tick()
            }
        }
    }

    private func stopTicking() {
        ticker?.cancel()
        ticker = nil
    }
}

/// The real engine: `AVAudioPlayer` over the file.
@MainActor
final class SystemLocalAudioEngine: NSObject, LocalAudioEngine, AVAudioPlayerDelegate {
    private let player: AVAudioPlayer
    var onFinish: (() -> Void)?

    init(url: URL) throws {
        player = try AVAudioPlayer(contentsOf: url)
        super.init()
        player.delegate = self
        player.prepareToPlay()
    }

    var currentTime: TimeInterval {
        get { player.currentTime }
        set { player.currentTime = newValue }
    }

    var duration: TimeInterval { player.duration }
    var isPlaying: Bool { player.isPlaying }

    @discardableResult
    func play() -> Bool { player.play() }
    func pause() { player.pause() }
    func stop() { player.stop() }

    nonisolated func audioPlayerDidFinishPlaying(_ player: AVAudioPlayer, successfully flag: Bool) {
        Task { @MainActor [weak self] in self?.onFinish?() }
    }

    nonisolated func audioPlayerDecodeErrorDidOccur(_ player: AVAudioPlayer, error: (any Error)?) {
        Task { @MainActor [weak self] in self?.onFinish?() }
    }
}
