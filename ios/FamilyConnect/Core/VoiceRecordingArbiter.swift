//
//  VoiceRecordingArbiter.swift
//  FamilyConnect
//
//  ONE RECORDING AT A TIME IN THE WHOLE APP, and the app-wide things that
//  stop one (#79, docs/audio-video-messages-2026-10-04.md, S1.7, S4, S8.3).
//
//  Each composer owns its own `AudioRecorder`, and before this nothing knew
//  about more than one of them: two Mac windows could record at once, a
//  closed window or a sleeping laptop left a microphone open, and a call
//  placed from anywhere ran straight over a recording. This is the one
//  place that knows which composer holds the microphone, and how to make it
//  let go.
//
//  WHAT LETTING GO MEANS is the composer's business, not this file's: it
//  stops its recorder and keeps what was recorded in the "Voice message not
//  sent" row (ParkedRecordings), with the reply it was recorded under. This
//  only says WHEN — another composer starting one, the app going to the
//  background, the Mac locking, sleeping, hiding, minimising the window or
//  quitting — and, at sign-out, that what was recorded is to be thrown away
//  rather than kept.
//
//  The events a composer sees for itself stay with the composer, where the
//  plan puts them: leaving the chat (`onDisappear`), its scene going to the
//  background (`scenePhase`), a call turning live (`calls.isIdle`). The
//  app-level background notification here is a BACKSTOP for the second; both
//  paths end in the same idempotent park, so whichever runs first wins and
//  the other finds nothing left to stop.
//
//  It also keeps the screen awake while a recording runs (S1.7). Phase 0
//  stops a recording when the phone locks or the Mac's display sleeps, so
//  without this, auto-lock would turn every long story into a "not sent" row
//  halfway through — the very thing Decision 34 exists to prevent.
//

import Foundation
import Observation
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

@MainActor
@Observable
final class VoiceRecordingArbiter {

    /// The app's one arbiter. Watches the system from the moment it exists.
    static let shared = VoiceRecordingArbiter(watchesTheSystem: true)

    /// What the composer holding the microphone is asked to do with it.
    nonisolated enum LetGo: Equatable, Sendable {
        /// Stop and keep: the "not sent" row (S2.8).
        case park
        /// Stop and delete — sign-out, where "everything recorded and not
        /// sent is deleted" (S4).
        case discard
    }

    /// The composer recording right now, if any.
    private(set) var holder: UUID?

    /// Is any composer anywhere in the app recording? What the call buttons
    /// are disabled by (S1.7), in every window.
    var isRecording: Bool { holder != nil }

    /// Whether a call is in progress, in any phase but idle or ended. Wired
    /// once, at launch, to the app's CallManager.
    @ObservationIgnored var callIsActive: () -> Bool = { false }

    /// Keeps the screen awake, or lets it sleep again. A seam: the app gets
    /// the platform's own switch, a test counts.
    @ObservationIgnored var keepAwake: (Bool) -> Void = VoiceRecordingArbiter.systemKeepAwake

    /// On iOS the call screen owns the same idle-timer flag (CallView), so
    /// a recording that a call has just stopped must not switch it off
    /// under a video call. The Mac's display assertion is reference-counted
    /// and shared with nothing, so it is always given back.
    @ObservationIgnored var callSharesTheScreenFlag: Bool = {
        #if os(iOS)
        true
        #else
        false
        #endif
    }()

    @ObservationIgnored private var letGo: ((LetGo) -> Void)?
    /// The holder's window, compared by identity: minimising or closing THAT
    /// window stops the recording, another one's does not (S4).
    @ObservationIgnored private weak var holderWindow: AnyObject?
    /// Composers with something a quit must not lose — a recording, or a
    /// voice note waiting in review — and how each keeps it.
    @ObservationIgnored private var quitKeepers: [UUID: () -> Void] = [:]
    @ObservationIgnored private var observers: [(center: NotificationCenter, token: any NSObjectProtocol)] = []
    /// Whoever must also hear that the app or a window went away without
    /// holding the microphone — the video recorder, whose PREVIEW closes on
    /// the same events that stop a recording (#79, S4) — and the window it
    /// is drawn in.
    @ObservationIgnored private var awayWatchers: [UUID: (window: () -> AnyObject?, action: () -> Void)] = [:]

    init(watchesTheSystem: Bool = false) {
        if watchesTheSystem { watchTheSystem() }
    }

    // MARK: - Holding the microphone

    /// Take the microphone for `id` — parking whatever any OTHER composer was
    /// recording first (S1.7: "Starting one anywhere — another chat, another
    /// Mac window — stops any other first").
    func claim(_ id: UUID, window: AnyObject? = nil, letGo: @escaping (LetGo) -> Void) {
        let previous = holder
        let previousLetGo = self.letGo
        // The new holder is in place BEFORE the old one is asked to let go,
        // so the old composer's own release finds the microphone no longer
        // its own and changes nothing — the screen stays awake throughout.
        holder = id
        self.letGo = letGo
        holderWindow = window
        if let previous, previous != id {
            previousLetGo?(.park)
        }
        if previous == nil { keepAwake(true) }
    }

    /// Give the microphone back. A composer that does not hold it changes
    /// nothing — a late release must not end somebody else's recording.
    func release(_ id: UUID) {
        guard holder == id else { return }
        holder = nil
        letGo = nil
        holderWindow = nil
        if !(callSharesTheScreenFlag && callIsActive()) {
            keepAwake(false)
        }
    }

    /// Make the holder let go, however it was asked.
    func stopHolder(_ how: LetGo) {
        guard let holder else { return }
        letGo?(how)
        // In case the composer did not release it itself.
        release(holder)
    }

    // MARK: - Watching without holding

    /// `action` runs whenever the app goes away (the background, a lock,
    /// sleep, the screen saver, ⌘H) or `window` is minimised or closed —
    /// the events that stop a recording, heard by somebody that may not be
    /// recording at all (S4: the video recorder's PREVIEW closes on them).
    func watchAway(_ id: UUID, window: @escaping () -> AnyObject? = { nil }, _ action: @escaping () -> Void) {
        awayWatchers[id] = (window, action)
    }

    func unwatchAway(_ id: UUID) {
        awayWatchers[id] = nil
    }

    // MARK: - Quitting (the Mac)

    /// `keep` runs when the app quits: ⌘Q gives no `onDisappear`, so a
    /// composer that holds something not yet sent says how to keep it.
    func keepOnQuit(_ id: UUID, _ keep: @escaping () -> Void) {
        quitKeepers[id] = keep
    }

    func forgetOnQuit(_ id: UUID) {
        quitKeepers[id] = nil
    }

    // MARK: - The system

    /// The app stopped being in front of anybody: the background, a lock,
    /// sleep, the screen saver, another user's session, ⌘H. Stop and keep.
    func appWentAway() {
        stopHolder(.park)
        for watcher in awayWatchers.values { watcher.action() }
    }

    /// A window was minimised or closed. Only the holder's own window
    /// matters: a desktop window that merely loses focus keeps recording.
    func windowWentAway(_ window: AnyObject) {
        for watcher in awayWatchers.values where watcher.window() === window {
            watcher.action()
        }
        guard let holderWindow, holderWindow === window else { return }
        stopHolder(.park)
    }

    /// The app is quitting: the recording and every composer's voice note in
    /// review are kept — synchronously, because nothing runs after this.
    func appWillQuit() {
        stopHolder(.park)
        let keepers = quitKeepers
        quitKeepers = [:]
        for keep in keepers.values { keep() }
    }

    private func watchTheSystem() {
        #if os(iOS)
        observe(UIApplication.didEnterBackgroundNotification, on: .default) { $0.appWentAway() }
        #elseif os(macOS)
        let workspace = NSWorkspace.shared.notificationCenter
        // S8.3's three, plus the two S4 names that have notifications of
        // their own: the screen saver starting, and the session being
        // switched away from (another user at the login window).
        observe(NSWorkspace.willSleepNotification, on: workspace) { $0.appWentAway() }
        observe(NSWorkspace.screensDidSleepNotification, on: workspace) { $0.appWentAway() }
        observe(NSWorkspace.sessionDidResignActiveNotification, on: workspace) { $0.appWentAway() }
        let distributed = DistributedNotificationCenter.default()
        observe(Notification.Name("com.apple.screenIsLocked"), on: distributed) { $0.appWentAway() }
        observe(Notification.Name("com.apple.screensaver.didstart"), on: distributed) { $0.appWentAway() }
        // ⌘H hides every window; a hidden window records nobody (S4).
        observe(NSApplication.didHideNotification, on: .default) { $0.appWentAway() }
        observe(NSApplication.willTerminateNotification, on: .default) { $0.appWillQuit() }
        observeWindow(NSWindow.willMiniaturizeNotification)
        observeWindow(NSWindow.willCloseNotification)
        #endif
    }

    private func observe(
        _ name: Notification.Name,
        on center: NotificationCenter,
        _ action: @escaping @MainActor @Sendable (VoiceRecordingArbiter) -> Void
    ) {
        let token = center.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
            // Delivered on the main queue, so this IS the main actor — and
            // the quit has to be handled before the block returns, which a
            // hop through a Task would not be.
            MainActor.assumeIsolated {
                guard let self else { return }
                action(self)
            }
        }
        observers.append((center, token))
    }

    #if os(macOS)
    private func observeWindow(_ name: Notification.Name) {
        let token = NotificationCenter.default.addObserver(
            forName: name, object: nil, queue: .main
        ) { [weak self] note in
            guard let window = note.object as? NSWindow else { return }
            MainActor.assumeIsolated { self?.windowWentAway(window) }
        }
        observers.append((NotificationCenter.default, token))
    }
    #endif

    // MARK: - Keeping the screen awake

    #if os(macOS)
    private static var displayActivity: (any NSObjectProtocol)?
    #endif

    /// The platform's own switch (S1.7): the idle timer on iPhone and iPad,
    /// a display-sleep assertion on the Mac.
    static func systemKeepAwake(_ on: Bool) {
        #if os(iOS)
        UIApplication.shared.isIdleTimerDisabled = on
        #elseif os(macOS)
        if on {
            guard displayActivity == nil else { return }
            displayActivity = ProcessInfo.processInfo.beginActivity(
                options: [.idleDisplaySleepDisabled, .userInitiated],
                reason: "Recording a voice message")
        } else if let activity = displayActivity {
            ProcessInfo.processInfo.endActivity(activity)
            displayActivity = nil
        }
        #endif
    }
}
