//
//  VideoMessageSession.swift
//  FamilyConnect
//
//  One opening of the round-video recorder, and the window's door to it
//  (#79, Phase 3 — docs/audio-video-messages-2026-10-04.md, S3, S4, S8.1,
//  S8.3).
//
//  `VideoRecorderMachine` decides; this object DOES what it decides: turns
//  the camera on and off (`VideoCaptureEngine`), asks the system for the
//  camera and the microphone, takes the microphone through the app's one
//  `VoiceRecordingArbiter` (one recording at a time, in every window), plays
//  the clip in REVIEW through `.playback`, speaks, holds the phone's
//  orientation, and finally hands the clip to the composer's send — the
//  Phase 2 path, `round: true` — with the poster `MediaPrep` makes from it.
//  Every system it touches is a seam, so the whole flow runs in a test with a
//  fake camera and a clock that moves only when told to.
//
//  `VideoMessagePresenter` is the window's: one per window root (iPhone and
//  iPad: the scene; the Mac: the main window and each conversation window),
//  so the recorder covers the window it was opened from and nothing else.
//

import AVFoundation
import Foundation
import Observation
import os
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

@MainActor
@Observable
final class VideoMessageSession {

    typealias Machine = VideoRecorderMachine

    /// What the composer that opened the recorder gives it.
    struct Request {
        var chatID: Int64
        /// The composer's primed reply — the video carries it (S1.5, S3.3).
        var reply: ReplyToDTO?
        /// "Replying to Anna", and the quote's words, as the composer draws
        /// its own banner.
        var replyTitle: String?
        var replyText: String?
        /// `max_round_video_ms` and `max_round_video_bytes` (On the wire).
        var maxRoundVideoMS: UInt64 = RecordRules.defaultMaxRoundVideoMS
        var maxRoundVideoBytes: Int?
        /// A not-sent voice message waits in this chat (row 9).
        var notSent: () -> Bool = { false }
        /// Hand the clip to the outbox: the reply, and whether it goes as a
        /// video message. False when it could not be queued.
        var send: (MediaPrep.Prepared, ReplyToDTO?, Bool) -> Bool = { _, _, _ in false }
        /// The reply left with the video, or its ✕ was used: the composer
        /// drops its own.
        var dropReply: () -> Void = {}
        /// "Record a voice message instead" (S3.4).
        var startVoice: () -> Void = {}
        /// The recorder closed: focus back to what opened it (S3.4).
        var closed: () -> Void = {}
    }

    // MARK: - What the view draws

    private(set) var state: Machine.State
    /// The recorder's clock as of the last tick.
    private(set) var nowMS: UInt64 = 0
    private(set) var reply: ReplyToDTO?
    let replyTitle: String?
    let replyText: String?
    /// "Only you can see this until you start recording." (S3.4)
    private(set) var showsFirstTimeLine: Bool
    /// A permission prompt the recorder raised is on screen.
    private(set) var askingPermission = false
    /// The clip in REVIEW, and its playback.
    private(set) var clipURL: URL?
    private(set) var player: AVPlayer?
    private(set) var isPlaying = false
    private(set) var playbackProgress: Double = 0
    /// Bumped when the slot should take focus again.
    private(set) var focusRequest = 0

    let request: Request
    let engine: any VideoCaptureEngine
    /// This recorder's name to the app's one-recording rule.
    let id = UUID()

    // MARK: - Seams

    @ObservationIgnored var clock: () -> UInt64 = VoiceComposer.uptimeMS
    @ObservationIgnored var cameraPermission: () -> Machine.Permission = VideoMessageSession.systemCameraPermission
    @ObservationIgnored var microphonePermission: () -> Machine.Permission = VideoMessageSession.systemMicrophonePermission
    @ObservationIgnored var askCamera: () async -> Bool = { await AVCaptureDevice.requestAccess(for: .video) }
    @ObservationIgnored var askMicrophone: () async -> Bool = VoiceComposer.askTheSystem
    @ObservationIgnored var announce: (String) -> Void = VoiceComposer.post
    /// VoiceOver is running: the microphone opens only once "Recording
    /// video" has been spoken (S6), as a voice note's does.
    @ObservationIgnored var voiceOverRunning: () -> Bool = VoiceComposer.voiceOverRuns
    /// Say this and return once it has been said (bounded).
    @ObservationIgnored var speak: (String) async -> Void = VoiceComposer.speakAndWait
    @ObservationIgnored var haptic: () -> Void = VideoMessageSession.mediumHaptic
    @ObservationIgnored var holdOrientation: (Bool) -> Void = VideoMessageSession.systemOrientationHold
    @ObservationIgnored var fileSize: (URL) -> Int = MediaPrep.fileSize(of:)
    @ObservationIgnored var prepare: (URL) async -> MediaPrep.Prepared = { await MediaPrep.preparedVideo(at: $0) }
    @ObservationIgnored var makeClipURL: () -> URL = { MediaPrep.temporaryURL(extension: "mp4") }
    @ObservationIgnored var playbackSession: PlaybackSessionControl = .system
    @ObservationIgnored var arbiter: VoiceRecordingArbiter = .shared
    @ObservationIgnored var nowPlaying: NowPlaying = .shared
    @ObservationIgnored var markTaught: () -> Void = { AppSettings.videoMessagePreviewTaught = true }
    /// The window it is drawn in (the Mac): minimising or closing THAT one
    /// stops it (S4).
    @ObservationIgnored var window: () -> AnyObject? = { nil }
    /// Takes the recorder off the screen.
    @ObservationIgnored var onClose: () -> Void = {}
    /// Lets the window close, or the app quit, after all.
    @ObservationIgnored var onProceed: (Machine.Closing) -> Void = { _ in }
    /// False in a test, which calls `tick()` itself.
    @ObservationIgnored var runsTicker = true
    /// The send's preparation, for a test to await.
    @ObservationIgnored private(set) var sendTask: Task<Void, Never>?

    @ObservationIgnored private var ticker: Task<Void, Never>?
    /// "Recording video" being spoken before the microphone opens; a newer
    /// Record overtakes older words. (Words overtaken by a Stop, a Delete or
    /// a close need nothing here: the reducer ignores a `.spoken` that comes
    /// when it is no longer speaking.)
    @ObservationIgnored private var speechAttempt = 0
    /// The speech in flight, for a test to await.
    @ObservationIgnored private(set) var speechTask: Task<Void, Never>?
    /// REVIEW's playback holds the `.playback` audio session — begun on
    /// play, given back on EVERY pause, end and stop (S5.3).
    @ObservationIgnored private var holdsPlaybackSession = false
    @ObservationIgnored private var playerEnd: (any NSObjectProtocol)?
    @ObservationIgnored private var started = false

    init(request: Request, engine: any VideoCaptureEngine, firstTime: Bool = !AppSettings.videoMessagePreviewTaught) {
        self.request = request
        self.engine = engine
        reply = request.reply
        replyTitle = request.replyTitle
        replyText = request.replyText
        showsFirstTimeLine = firstTime
        state = Machine.State(maxRoundVideoMS: request.maxRoundVideoMS)
    }

    // MARK: - Opening

    /// Read both statuses, then open onto the right state (S3.2).
    func start() {
        guard !started else { return }
        started = true
        engine.onEvent = { [weak self] event in self?.received(event) }
        arbiter.watchAway(id, window: { [weak self] in self?.window() }) { [weak self] in
            self?.handle(.wentAway)
        }
        nowMS = clock()
        handle(.opened(camera: cameraPermission(), microphone: microphonePermission(), atMS: nowMS))
        if runsTicker { startTicker() }
    }

    var isClosed: Bool { state.phase == .closed }

    // MARK: - Controls

    func record() { handle(.record(atMS: clock(), speakFirst: voiceOverRunning())) }
    func stop() { handle(.stop(atMS: clock())) }
    func delete() { handle(.delete(atMS: clock())) }
    func retake() { handle(.retake(atMS: clock())) }
    func send() { handle(.send(atMS: clock())) }
    func close() { handle(.close) }
    func escape() { handle(.escape(atMS: clock())) }
    func magicTap() { handle(.magicTap(atMS: clock(), speakFirst: voiceOverRunning())) }
    func playPause() { handle(.playPause) }
    func answer(delete: Bool) { handle(.answer(delete: delete, atMS: clock())) }
    func voiceInstead() { handle(.voiceInstead(notSent: request.notSent())) }

    /// The slot: Record, Stop or Send, by the phase (S3.4).
    func activateSlot() {
        switch state.phase {
        case .preview: record()
        case .recording: stop()
        case .review: send()
        default: break
        }
    }

    func switchCamera() {
        engine.switchCamera()
        handle(.used(atMS: clock()))
    }

    func chooseCamera(_ id: String) {
        engine.chooseCamera(id)
        handle(.used(atMS: clock()))
    }

    /// The reply banner's ✕: the video goes without it, and so does the
    /// composer (S3.3).
    func dropReply() {
        reply = nil
        request.dropReply()
        handle(.used(atMS: clock()))
    }

    func tick() {
        nowMS = clock()
        if let player, let item = player.currentItem {
            let duration = CMTimeGetSeconds(item.duration)
            let now = CMTimeGetSeconds(player.currentTime())
            playbackProgress = duration.isFinite && duration > 0 ? min(1, max(0, now / duration)) : 0
        }
        handle(.tick(atMS: nowMS))
    }

    // MARK: - The interruptions the window hears (S4)

    func callStarted() { handle(.callStarted) }
    func wentAway() { handle(.wentAway) }
    /// Never closes over a permission prompt the recorder raised (S3.4):
    /// that is `.asking`, which the rule leaves alone.
    func focusLost() { handle(.focusLost) }
    func signedOut() { handle(.signedOut) }

    /// The window is asked to close (⌘W, the close button): true lets it
    /// close now; false keeps it, and REVIEW asks (S4, S8.3).
    func windowShouldClose() -> Bool {
        if state.closesFreely {
            let proceed = onProceed
            onProceed = { _ in }
            handle(.closing(.window))
            onProceed = proceed
            return true
        }
        handle(.closing(.window))
        return false
    }

    /// The app is asked to quit (⌘Q): true lets it; false keeps it, and
    /// REVIEW asks — Delete quits after all (S4, S8.3).
    func appShouldQuit() -> Bool {
        if state.closesFreely {
            let proceed = onProceed
            onProceed = { _ in }
            handle(.closing(.quit))
            onProceed = proceed
            return true
        }
        handle(.closing(.quit))
        return false
    }

    // MARK: - The reducer, and what it says to do

    func handle(_ event: Machine.Event) {
        let (next, effects) = Machine.step(state, event)
        state = next
        for effect in effects { perform(effect) }
    }

    private func perform(_ effect: Machine.Effect) {
        switch effect {
        case .requestCamera:
            askingPermission = true
            Task { @MainActor [weak self] in
                guard let self else { return }
                let granted = await self.askCamera()
                self.askingPermission = false
                self.handle(.cameraAnswered(granted, atMS: self.clock()))
            }
        case .requestMicrophone:
            askingPermission = true
            Task { @MainActor [weak self] in
                guard let self else { return }
                let granted = await self.askMicrophone()
                self.askingPermission = false
                self.handle(.microphoneAnswered(granted, atMS: self.clock()))
            }
        case .startCamera:
            engine.startPreview()
        case .stopCamera:
            engine.stopPreview()
        case .startRecording:
            let url = makeClipURL()
            clipURL = url
            takeTheRecording()
            engine.startRecording(to: url, capMS: state.capMS)
        case .speakThenRecord:
            // Playback paused and the one recording taken AT Record, so
            // nothing plays over the words; the microphone waits for them.
            takeTheRecording()
            speechAttempt += 1
            let attempt = speechAttempt
            let sentence = Machine.Announcement.recordingVideo.text
            speechTask = Task { @MainActor [weak self] in
                guard let self else { return }
                await self.speak(sentence)
                guard attempt == self.speechAttempt else { return }
                self.handle(.spoken(atMS: self.clock()))
            }
        case .stopRecording:
            engine.stopRecording()
        case .cancelRecording:
            engine.cancelRecording()
            arbiter.release(id)
            removeClip()
        case .deleteClip:
            removeClip()
        case .send(let round):
            guard let url = clipURL else { return }
            // The outbox owns the file from here: it moves it somewhere the
            // system will not reclaim and keeps it until the ack.
            clipURL = nil
            let reply = self.reply
            let request = self.request
            let prepare = self.prepare
            request.dropReply()
            sendTask = Task { @MainActor in
                // The poster from the clip itself (MediaPrep), so the circle
                // draws at once from the local file (S5.6).
                let prepared = await prepare(url)
                if !request.send(prepared, reply, round) {
                    AppLog.sync.error("A video message could not be queued; its clip is kept in tmp")
                }
            }
        case .announce(let announcement):
            announce(announcement.text)
        case .haptic:
            haptic()
        case .togglePlayback:
            togglePlayback()
        case .stopPlayback:
            stopPlayback()
        case .holdOrientation(let on):
            holdOrientation(on)
        case .close:
            finishClosing()
        case .proceed(let closing):
            onProceed(closing)
        case .startVoice:
            request.startVoice()
        case .taught:
            if showsFirstTimeLine {
                showsFirstTimeLine = false
                markTaught()
            }
        }
    }

    /// Nothing of the app's plays while recording (S1.7), and any other
    /// recording anywhere stops first.
    private func takeTheRecording() {
        nowPlaying.pauseAll()
        arbiter.claim(id, window: window()) { [weak self] how in
            self?.handle(how == .discard ? .signedOut : .wentAway)
        }
    }

    private func received(_ event: VideoCaptureEvent) {
        let now = clock()
        switch event {
        case .firstFrame: handle(.firstFrame(atMS: now))
        case .frame(let dark): handle(.frame(dark: dark, atMS: now))
        case .problem(let problem): handle(.camera(problem))
        case .microphoneTaken: handle(.microphoneTaken)
        case .limitReached: handle(.limitReached)
        case .failed: handle(.recorderFailed)
        case .finished(let clip):
            arbiter.release(id)
            guard case .finishing = state.phase else {
                // Cancelled or closed meanwhile: nothing will review it.
                if let clip { try? FileManager.default.removeItem(at: clip.url) }
                return
            }
            if let clip {
                clipURL = clip.url
                let ceiling = request.maxRoundVideoBytes
                let tooBig = ceiling.map { fileSize(clip.url) > $0 } ?? false
                handle(.finished(durationMS: clip.durationMS, tooBig: tooBig, atMS: now))
                // REVIEW shows the clip's first frame at once, paused.
                if case .review = state.phase { makePlayer() }
            } else {
                clipURL = nil
                handle(.finished(durationMS: nil, tooBig: false, atMS: now))
            }
        }
    }

    private func finishClosing() {
        ticker?.cancel()
        ticker = nil
        stopPlayback()
        engine.stopPreview()
        arbiter.release(id)
        arbiter.unwatchAway(id)
        // Whatever was not sent is gone (a closed recorder is never parked,
        // S4) — `send` has already taken a clip that left.
        removeClip()
        onClose()
        request.closed()
    }

    private func removeClip() {
        stopPlayback()
        if let playerEnd { NotificationCenter.default.removeObserver(playerEnd) }
        playerEnd = nil
        player = nil
        playbackProgress = 0
        if let clipURL { try? FileManager.default.removeItem(at: clipURL) }
        clipURL = nil
    }

    private func startTicker() {
        ticker = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(250))
                guard let self, !Task.isCancelled else { return }
                self.tick()
            }
        }
    }

    // MARK: - REVIEW's playback (S3.4: the clip as it will be sent, with sound)

    private func togglePlayback() {
        guard clipURL != nil else { return }
        if isPlaying {
            player?.pause()
            isPlaying = false
            nowPlaying.release(id)
            giveThePlaybackSessionBack()
            return
        }
        if player == nil { makePlayer() }
        // Whoever played before is paused first, then the session is taken —
        // the bubble players' order, so the outgoing player's release of the
        // session cannot land after this one's begin.
        nowPlaying.claim(id, kind: .circle) { [weak self] in
            self?.player?.pause()
            self?.isPlaying = false
            self?.giveThePlaybackSessionBack()
        }
        // `.playback`, never the earpiece the recording left the route on
        // (S3.4, S5.3).
        if !holdsPlaybackSession {
            holdsPlaybackSession = true
            playbackSession.begin()
        }
        player?.play()
        isPlaying = true
    }

    private func makePlayer() {
        guard let url = clipURL, player == nil else { return }
        let made = AVPlayer(url: url)
        player = made
        if let playerEnd { NotificationCenter.default.removeObserver(playerEnd) }
        playerEnd = NotificationCenter.default.addObserver(
            forName: .AVPlayerItemDidPlayToEndTime, object: made.currentItem, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.playedToEnd() }
        }
    }

    private func playedToEnd() {
        player?.seek(to: .zero)
        isPlaying = false
        playbackProgress = 0
        nowPlaying.release(id)
        giveThePlaybackSessionBack()
    }

    /// Paused, ended or stopped: the session goes back, and other apps'
    /// sound is told it may resume (`.notifyOthersOnDeactivation`, S5.3).
    private func giveThePlaybackSessionBack() {
        guard holdsPlaybackSession else { return }
        holdsPlaybackSession = false
        playbackSession.end()
    }

    /// Back to the first frame, paused; the player stays for REVIEW.
    private func stopPlayback() {
        guard let player else { return }
        player.pause()
        player.seek(to: .zero)
        giveThePlaybackSessionBack()
        isPlaying = false
        playbackProgress = 0
        nowPlaying.release(id)
    }

    // MARK: - The system's defaults

    static func systemCameraPermission() -> Machine.Permission {
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized: .granted
        case .notDetermined: .notAsked
        default: .denied
        }
    }

    static func systemMicrophonePermission() -> Machine.Permission {
        switch VoiceComposer.systemPermission() {
        case .granted: .granted
        case .denied: .denied
        case .notAsked: .notAsked
        }
    }

    /// A medium haptic at Record — phones only (S3.4).
    static func mediumHaptic() {
        #if os(iOS)
        guard UIDevice.current.userInterfaceIdiom == .phone else { return }
        UIImpactFeedbackGenerator(style: .medium).impactOccurred()
        #endif
    }

    static func systemOrientationHold(_ on: Bool) {
        #if os(iOS)
        OrientationHold.set(on)
        #endif
    }
}

// MARK: - The window's door

@MainActor
@Observable
final class VideoMessagePresenter {

    /// The open recorder, if any.
    private(set) var session: VideoMessageSession?

    var isOpen: Bool { session != nil }

    /// The conversation pane the recorder lays out over, in the window's
    /// coordinates — reported by the composer that opened it (S3.3).
    var paneFrame: CGRect?

    /// The window this presenter covers (the Mac).
    @ObservationIgnored var window: () -> AnyObject? = { nil }
    /// Closes the window or quits after a REVIEW clip's Delete (S4).
    @ObservationIgnored var proceed: (VideoRecorderMachine.Closing) -> Void = { _ in }
    /// The camera to use; a test passes a fake.
    @ObservationIgnored var makeEngine: () -> any VideoCaptureEngine = { VideoMessageRecorder() }
    /// Last word on a session before it starts; a test swaps its seams.
    @ObservationIgnored var prepareSession: (VideoMessageSession) -> Void = { _ in }

    /// Every window's presenter, for ⌘Q to ask (the Mac).
    @ObservationIgnored private static var live: [WeakPresenter] = []

    private struct WeakPresenter {
        weak var presenter: VideoMessagePresenter?
    }

    init() {
        Self.live.removeAll { $0.presenter == nil }
        Self.live.append(WeakPresenter(presenter: self))
    }

    /// Open the recorder over this window. One at a time.
    func open(_ request: VideoMessageSession.Request) {
        guard session == nil else { return }
        let opened = VideoMessageSession(request: request, engine: makeEngine())
        opened.window = { [weak self] in self?.window() }
        opened.onClose = { [weak self, weak opened] in
            guard let self, self.session === opened else { return }
            self.session = nil
            self.paneFrame = nil
        }
        opened.onProceed = { [weak self] closing in self?.proceed(closing) }
        prepareSession(opened)
        session = opened
        opened.start()
    }

    /// Whether ⌘Q may go on: false when any window's recorder holds a take,
    /// which then asks (S4, S8.3).
    static func appShouldQuit() -> Bool {
        live.removeAll { $0.presenter == nil }
        var quits = true
        for entry in live {
            guard let session = entry.presenter?.session else { continue }
            if !session.appShouldQuit() { quits = false }
        }
        return quits
    }
}
