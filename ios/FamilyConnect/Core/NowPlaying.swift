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
//  Phase 2 (round video) adds the rest of S5.3 and S4's last column, here
//  so that every player gets them at once:
//
//    - WHAT PAUSES IT. A call (the conversations already call `pauseAll`
//      when `calls.isIdle` turns), the app going to the background, another
//      app's sound (an audio-session interruption), and headphones going
//      (`routeChangeNotification` with `.oldDeviceUnavailable` — coming
//      plays on). On the Mac: the session locking, the screen saver, sleep,
//      the display sleeping, another user's session, and a change of the
//      default output device; ⌘H or a minimised window pauses a CIRCLE and
//      leaves a voice note playing (S4).
//    - KEEPING THE SCREEN AWAKE while something plays, on iPhone and iPad
//      only (S1.7, Decision 34) — through the same idle-timer flag a
//      recording and the call screen use, so it is never switched off under
//      either of them.
//
//  Each bubble player still takes `.playback` itself (PlaybackSessionControl)
//  after its claim and gives it back when it stops, as `LocalVoicePlayer`
//  does: the owner is a referee, not a player.
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
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
import CoreAudio
#endif

@MainActor
@Observable
final class NowPlaying {

    /// The app's one owner, watching the system from the moment it exists.
    static let shared = NowPlaying(
        watchesTheSystem: true,
        keepAwake: NowPlaying.systemKeepAwake,
        screenHeldElsewhere: {
            VoiceRecordingArbiter.shared.isRecording
                || VoiceRecordingArbiter.shared.callIsActive()
        })

    /// What is playing — a desktop's minimised window or ⌘H pauses a circle
    /// and lets a voice note play on (S4).
    nonisolated enum Kind: Equatable, Sendable {
        case voice
        case circle
    }

    /// Who is playing now, if anybody.
    private(set) var current: UUID?
    /// What it is.
    private(set) var currentKind: Kind?

    @ObservationIgnored private var pauseCurrent: (() -> Void)?
    /// Keeps the screen awake, or lets it sleep again (S1.7). A seam: the
    /// app's owner gets the platform's switch, a test counts.
    @ObservationIgnored var keepAwake: (Bool) -> Void
    /// Whether a recording or a call holds the same switch — then letting it
    /// go is theirs, not playback's.
    @ObservationIgnored var screenHeldElsewhere: () -> Bool
    @ObservationIgnored private var observers: [(center: NotificationCenter, token: any NSObjectProtocol)] = []
    #if os(macOS)
    @ObservationIgnored private var outputListener: AudioObjectPropertyListenerBlock?
    #endif

    init(
        watchesTheSystem: Bool = false,
        keepAwake: @escaping (Bool) -> Void = { _ in },
        screenHeldElsewhere: @escaping () -> Bool = { false }
    ) {
        self.keepAwake = keepAwake
        self.screenHeldElsewhere = screenHeldElsewhere
        if watchesTheSystem { watchTheSystem() }
    }

    var isPlaying: Bool { current != nil }

    /// `id` starts playing: whoever was playing is paused first.
    func claim(_ id: UUID, kind: Kind = .voice, pause: @escaping () -> Void) {
        let previous = pauseCurrent
        let previousID = current
        current = id
        currentKind = kind
        pauseCurrent = pause
        if let previousID, previousID != id {
            previous?()
        }
        if previousID == nil { keepAwake(true) }
    }

    /// `id` stopped by itself, or was paused by its own control. Somebody
    /// else's release changes nothing.
    func release(_ id: UUID) {
        guard current == id else { return }
        current = nil
        currentKind = nil
        pauseCurrent = nil
        letTheScreenSleep()
    }

    /// A recording is starting, or VoiceOver's Magic Tap asked: whatever
    /// plays, pauses. True when something was playing.
    @discardableResult
    func pauseAll() -> Bool {
        guard current != nil else { return false }
        let pause = pauseCurrent
        current = nil
        currentKind = nil
        pauseCurrent = nil
        pause?()
        letTheScreenSleep()
        return true
    }

    /// A desktop window was minimised or the app hidden: a circle pauses, a
    /// voice note plays on (S4). True when something was paused.
    @discardableResult
    func pauseCircles() -> Bool {
        guard currentKind == .circle else { return false }
        return pauseAll()
    }

    /// An audio-session interruption BEGAN — another app's sound, Siri, an
    /// alarm (S4, "Another app starts playing").
    func interruptionBegan() {
        pauseAll()
    }

    #if os(iOS)
    /// A route change: only headphones or a speaker GOING pauses; one
    /// coming plays on (S4).
    func routeChanged(_ reason: AVAudioSession.RouteChangeReason) {
        if reason == .oldDeviceUnavailable { pauseAll() }
    }
    #endif

    private func letTheScreenSleep() {
        guard !screenHeldElsewhere() else { return }
        keepAwake(false)
    }

    // MARK: - The system

    private func watchTheSystem() {
        #if os(iOS)
        observe(UIApplication.didEnterBackgroundNotification, on: .default) { owner, _ in
            owner.pauseAll()
        }
        observe(AVAudioSession.interruptionNotification, on: .default) { owner, note in
            let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt
            if raw.flatMap(AVAudioSession.InterruptionType.init(rawValue:)) == .began {
                owner.interruptionBegan()
            }
        }
        observe(AVAudioSession.routeChangeNotification, on: .default) { owner, note in
            let raw = note.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt
            if let reason = raw.flatMap(AVAudioSession.RouteChangeReason.init(rawValue:)) {
                owner.routeChanged(reason)
            }
        }
        #elseif os(macOS)
        let workspace = NSWorkspace.shared.notificationCenter
        observe(NSWorkspace.willSleepNotification, on: workspace) { owner, _ in owner.pauseAll() }
        observe(NSWorkspace.screensDidSleepNotification, on: workspace) { owner, _ in owner.pauseAll() }
        observe(NSWorkspace.sessionDidResignActiveNotification, on: workspace) { owner, _ in owner.pauseAll() }
        let distributed = DistributedNotificationCenter.default()
        observe(Notification.Name("com.apple.screenIsLocked"), on: distributed) { owner, _ in owner.pauseAll() }
        observe(Notification.Name("com.apple.screensaver.didstart"), on: distributed) { owner, _ in owner.pauseAll() }
        observe(NSApplication.didHideNotification, on: .default) { owner, _ in owner.pauseCircles() }
        observe(NSWindow.willMiniaturizeNotification, on: .default) { owner, _ in owner.pauseCircles() }
        watchTheDefaultOutput()
        #endif
    }

    private func observe(
        _ name: Notification.Name,
        on center: NotificationCenter,
        _ action: @escaping @MainActor (NowPlaying, Notification) -> Void
    ) {
        let token = center.addObserver(forName: name, object: nil, queue: .main) { [weak self] note in
            // Delivered on the main queue, so this IS the main actor.
            MainActor.assumeIsolated {
                guard let self else { return }
                action(self, note)
            }
        }
        observers.append((center, token))
    }

    #if os(macOS)
    /// The Mac's "headphones going": the default output device changing
    /// (S4, "a change of the default output device on desktops").
    private func watchTheDefaultOutput() {
        var address = AudioObjectPropertyAddress(
            mSelector: kAudioHardwarePropertyDefaultOutputDevice,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain)
        let listener: AudioObjectPropertyListenerBlock = { [weak self] _, _ in
            Task { @MainActor in self?.pauseAll() }
        }
        let status = AudioObjectAddPropertyListenerBlock(
            AudioObjectID(kAudioObjectSystemObject), &address, DispatchQueue.main, listener)
        if status == noErr { outputListener = listener }
    }
    #endif

    // MARK: - Keeping the screen awake

    /// iPhone and iPad only: "on phones and tablets the screen stays awake
    /// while it plays" (Decision 23, S1.7). The Mac's display sleeps on its
    /// own schedule while something plays, as it does for any other app's.
    static func systemKeepAwake(_ on: Bool) {
        #if os(iOS)
        UIApplication.shared.isIdleTimerDisabled = on
        #endif
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
