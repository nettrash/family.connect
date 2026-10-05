//
//  RoundVideoPlayer.swift
//  FamilyConnect
//
//  A received VIDEO MESSAGE, drawn and played as a circle (#79,
//  docs/audio-video-messages-2026-10-04.md, S5.2–S5.6, S6; docs/protocol.md,
//  "Video messages" › "How it is drawn").
//
//  Three pieces, all platform-free but the surface:
//
//    - `RoundVideoPlayback`, the state of one circle's player — idle,
//      loading, playing, paused, failed — and every rule about moving
//      between them: a tap plays, a tap pauses, a second tap while it loads
//      gives up, a failure waits for a tap to try again, the end returns to
//      the poster and drops the unplayed dot, one thing plays at a time
//      (NowPlaying), nothing plays under a recording, `.playback` is taken
//      after the claim and given back when it stops. Its engine is a seam,
//      so all of that is tested without a network or a decoder.
//    - `SystemRoundVideoEngine`, the real engine: an `AVPlayer` from
//      `AttachmentStreamPlayer` (the session's Authorization header on every
//      byte-range request, the double-start guard), watched for "playing",
//      "waiting", progress, the end and failure.
//    - `RoundVideoTile`, the circle itself: no balloon, 200 in a compact
//      width and 240 otherwise (`RoundVideo.diameter`), the square POSTER
//      filling it over a neutral disc of the final size — so the row never
//      changes height — a duration capsule, a 44-unit play disc, an 8-unit
//      unplayed dot, and while it plays an `AVPlayerLayer`
//      (`.resizeAspectFill`) clipped to the same circle with a 3-unit accent
//      ring running round the edge.
//
//  NEVER AUTOPLAY, muted or otherwise (S5.3, Decision 21): drawing a circle
//  fetches its poster and nothing else — "a tile never downloads a VIDEO to
//  draw itself" — and the video is asked for only on a tap.
//
//  Shared by the phone's MessageBubbleView and the Mac's MacMessageRow; the
//  Mac's surface is an `AVPlayerLayer` in an `NSViewRepresentable`, never
//  SwiftUI's `VideoPlayer` (S8.3).
//

import AVFoundation
import Combine
import Observation
import SwiftUI
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

// MARK: - The engine seam

/// What happened to a circle's video, as its engine reports it.
nonisolated enum RoundVideoEvent: Equatable, Sendable {
    /// Frames are moving.
    case playing
    /// Waiting for bytes — before the first frame or in a stall.
    case waiting
    /// How far through, 0…1.
    case progress(Double)
    /// It played to its end.
    case finished
    /// It cannot be played: the stream could not be built, the item failed,
    /// or it stopped short of its end.
    case failed
}

/// What plays one circle's video: an `AVPlayer` in the app, a fake in a test.
@MainActor
protocol RoundVideoEngine: AnyObject {
    /// The player a surface shows, once there is one. Always nil in a test.
    var player: AVPlayer? { get }
    /// Fetch the video and start playing it, reporting through `events`.
    func load(events: @escaping @MainActor (RoundVideoEvent) -> Void)
    func play()
    func pause()
    /// Back to the first frame, paused.
    func rewind()
    /// Let it all go: the player, its observers, a load in flight.
    func stop()
}

// MARK: - One circle's player

@MainActor
@Observable
final class RoundVideoPlayback {

    nonisolated enum Phase: Equatable, Sendable {
        /// The poster, with the play disc. Before the first tap, after the
        /// end, and after giving up a load.
        case idle
        /// Tapped, and waiting for frames — the loading ring.
        case loading
        case playing
        case paused
        /// "Couldn't load the video. Tap to try again."
        case failed
    }

    /// What a tap did — the view announces the one that needs words.
    nonisolated enum TapResult: Equatable, Sendable {
        case started
        case paused
        case gaveUp
        /// A voice recording runs somewhere in the app: nothing of the app's
        /// plays under it (S1.7).
        case refusedWhileRecording
    }

    private(set) var phase: Phase = .idle
    /// How far through, 0…1 — the accent ring.
    private(set) var progress: Double = 0
    /// True once frames have moved for this load, so the surface is worth
    /// drawing over the poster.
    private(set) var hasFrames = false

    /// This circle's name to the app's one-thing-plays rule.
    @ObservationIgnored let id = UUID()
    @ObservationIgnored var owner: NowPlaying = .shared
    @ObservationIgnored var session: PlaybackSessionControl = .system
    @ObservationIgnored var isRecording: () -> Bool = { VoiceRecordingArbiter.shared.isRecording }
    /// Builds the engine at the first tap — the view sets it, because the
    /// API it streams from lives in the environment.
    @ObservationIgnored var makeEngine: (() -> (any RoundVideoEngine)?)?
    /// It played to its end: the dot goes (S5.3).
    @ObservationIgnored var onFinished: () -> Void = {}

    @ObservationIgnored private var engine: (any RoundVideoEngine)?
    @ObservationIgnored private var holdsSession = false

    init() {}

    /// The player the surface shows.
    var player: AVPlayer? { engine?.player }

    var isActive: Bool { phase == .loading || phase == .playing || phase == .paused }

    /// A tap on the circle (the single tap, after the double-tap window).
    @discardableResult
    func tap() -> TapResult {
        switch phase {
        case .playing:
            pause()
            return .paused
        case .loading:
            // "A second tap while it loads gives up" (S5.3).
            stop()
            return .gaveUp
        case .idle, .paused, .failed:
            guard !isRecording() else { return .refusedWhileRecording }
            start()
            return .started
        }
    }

    /// Pause by the circle's own control.
    func pause() {
        guard phase == .playing || phase == .loading else { return }
        if phase == .loading, !hasFrames {
            stop()
            return
        }
        engine?.pause()
        phase = .paused
        owner.release(id)
        giveSession()
    }

    /// Let everything go — the row scrolled out of view, the chat closed,
    /// the circle gave up a load (S5.3).
    func stop() {
        tearDownEngine()
        phase = .idle
        progress = 0
        owner.release(id)
        giveSession()
    }

    /// One report from the engine.
    func handle(_ event: RoundVideoEvent) {
        switch event {
        case .playing:
            hasFrames = true
            if phase == .loading { phase = .playing }
        case .waiting:
            if phase == .playing { phase = .loading }
        case .progress(let fraction):
            guard isActive else { return }
            progress = min(1, max(0, fraction))
        case .finished:
            guard isActive else { return }
            // Back to the poster, with the video kept for a replay.
            engine?.rewind()
            hasFrames = false
            phase = .idle
            progress = 0
            owner.release(id)
            giveSession()
            onFinished()
        case .failed:
            guard isActive else { return }
            tearDownEngine()
            phase = .failed
            progress = 0
            owner.release(id)
            giveSession()
        }
    }

    private func start() {
        // Whoever played before is paused first, then the session is taken —
        // the order LocalVoicePlayer keeps, so the outgoing player's release
        // of the session cannot land after this one's claim.
        owner.claim(id, kind: .circle) { [weak self] in self?.pausedByOwner() }
        takeSession()
        if phase == .failed { tearDownEngine() }
        if let engine {
            // Paused, or ended and rewound: the bytes are here.
            engine.play()
            phase = hasFrames ? .playing : .loading
            return
        }
        guard let made = makeEngine?() else {
            phase = .failed
            owner.release(id)
            giveSession()
            return
        }
        engine = made
        hasFrames = false
        progress = 0
        phase = .loading
        made.load { [weak self] event in self?.handle(event) }
    }

    /// Something else started playing, a recording started, a call came,
    /// the app went away (NowPlaying). The owner has already let go of us.
    private func pausedByOwner() {
        switch phase {
        case .loading where !hasFrames:
            tearDownEngine()
            phase = .idle
            progress = 0
        case .playing, .loading:
            engine?.pause()
            phase = .paused
        case .idle, .paused, .failed:
            break
        }
        giveSession()
    }

    private func tearDownEngine() {
        engine?.stop()
        engine = nil
        hasFrames = false
    }

    private func takeSession() {
        guard !holdsSession else { return }
        holdsSession = true
        session.begin()
    }

    private func giveSession() {
        guard holdsSession else { return }
        holdsSession = false
        session.end()
    }
}

// MARK: - The real engine

/// An `AVPlayer` streamed from the attachment endpoint, watched.
@MainActor
final class SystemRoundVideoEngine: RoundVideoEngine {
    private let attachmentID: Int64
    private let durationMS: Int?
    private let api: APIClient
    private let stream = AttachmentStreamPlayer()
    private var events: (@MainActor (RoundVideoEvent) -> Void)?
    private var timeObserver: Any?
    private var watchers: Set<AnyCancellable> = []
    private var tokens: [any NSObjectProtocol] = []
    private var watchdog: Task<Void, Never>?

    init(attachment: AttachmentDTO, api: APIClient) {
        attachmentID = attachment.id
        durationMS = attachment.durationMS
        self.api = api
    }

    var player: AVPlayer? { stream.player }

    func load(events: @escaping @MainActor (RoundVideoEvent) -> Void) {
        self.events = events
        stream.start(attachment: attachmentID, from: api) { [weak self] created in
            self?.watch(created)
        }
        // A stream that could not be built (no server configured) never
        // calls back at all — say so, rather than spin for ever.
        let loading = stream.loadTask
        watchdog = Task { [weak self] in
            await loading?.value
            guard let self, !Task.isCancelled else { return }
            if self.stream.player == nil { self.events?(.failed) }
        }
    }

    func play() { stream.player?.play() }
    func pause() { stream.player?.pause() }

    func rewind() {
        stream.player?.pause()
        stream.player?.seek(to: .zero, toleranceBefore: .zero, toleranceAfter: .zero)
    }

    func stop() {
        watchdog?.cancel()
        watchdog = nil
        watchers = []
        tokens.forEach { NotificationCenter.default.removeObserver($0) }
        tokens = []
        let observer = timeObserver
        timeObserver = nil
        stream.stop { player in
            if let observer { player.removeTimeObserver(observer) }
        }
        events = nil
    }

    private func report(_ event: RoundVideoEvent) {
        events?(event)
    }

    private func watch(_ player: AVPlayer) {
        // Key-value changes arrive on whatever thread AVFoundation is on;
        // received on the main queue, which is this type's actor.
        player.publisher(for: \.timeControlStatus)
            .receive(on: DispatchQueue.main)
            .sink { [weak self] status in
                switch status {
                case .playing: self?.report(.playing)
                case .waitingToPlayAtSpecifiedRate: self?.report(.waiting)
                case .paused: break
                @unknown default: break
                }
            }
            .store(in: &watchers)
        if let item = player.currentItem {
            item.publisher(for: \.status)
                .receive(on: DispatchQueue.main)
                .sink { [weak self] status in
                    if status == .failed { self?.report(.failed) }
                }
                .store(in: &watchers)
            tokens.append(NotificationCenter.default.addObserver(
                forName: .AVPlayerItemDidPlayToEndTime, object: item, queue: .main
            ) { [weak self] _ in
                MainActor.assumeIsolated { self?.report(.finished) }
            })
            tokens.append(NotificationCenter.default.addObserver(
                forName: .AVPlayerItemFailedToPlayToEndTime, object: item, queue: .main
            ) { [weak self] _ in
                MainActor.assumeIsolated { self?.report(.failed) }
            })
        }
        let known = Double(durationMS ?? 0) / 1000
        timeObserver = player.addPeriodicTimeObserver(
            forInterval: CMTime(seconds: 0.1, preferredTimescale: 600), queue: .main
        ) { [weak self, weak player] time in
            let itemSeconds = player?.currentItem.map { CMTimeGetSeconds($0.duration) } ?? .nan
            let total = known > 0 ? known : (itemSeconds.isFinite ? itemSeconds : 0)
            guard total > 0 else { return }
            let fraction = CMTimeGetSeconds(time) / total
            MainActor.assumeIsolated { self?.report(.progress(fraction)) }
        }
    }
}

// MARK: - The surface

#if os(iOS)
/// An `AVPlayerLayer` view, its corners cut to a circle at every size.
final class RoundPlayerLayerView: UIView {
    override class var layerClass: AnyClass { AVPlayerLayer.self }
    var playerLayer: AVPlayerLayer { layer as! AVPlayerLayer }

    override init(frame: CGRect) {
        super.init(frame: frame)
        playerLayer.videoGravity = .resizeAspectFill
        clipsToBounds = true
        isUserInteractionEnabled = false
        isAccessibilityElement = false
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func layoutSubviews() {
        super.layoutSubviews()
        layer.cornerRadius = min(bounds.width, bounds.height) / 2
    }
}

struct RoundVideoSurface: UIViewRepresentable {
    let player: AVPlayer

    func makeUIView(context: Context) -> RoundPlayerLayerView {
        let view = RoundPlayerLayerView()
        view.playerLayer.player = player
        return view
    }

    func updateUIView(_ view: RoundPlayerLayerView, context: Context) {
        if view.playerLayer.player !== player { view.playerLayer.player = player }
    }
}
#elseif os(macOS)
/// The Mac's surface: a layer-hosting view whose layer IS the player layer
/// (S8.3 — never SwiftUI's `VideoPlayer`), cut to a circle.
final class RoundPlayerLayerView: NSView {
    let playerLayer = AVPlayerLayer()

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        playerLayer.videoGravity = .resizeAspectFill
        playerLayer.masksToBounds = true
        layer = playerLayer
        wantsLayer = true
        setAccessibilityElement(false)
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func layout() {
        super.layout()
        playerLayer.cornerRadius = min(bounds.width, bounds.height) / 2
    }

    /// Clicks belong to the circle's gestures, not to this view.
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

struct RoundVideoSurface: NSViewRepresentable {
    let player: AVPlayer

    func makeNSView(context: Context) -> RoundPlayerLayerView {
        let view = RoundPlayerLayerView()
        view.playerLayer.player = player
        return view
    }

    func updateNSView(_ view: RoundPlayerLayerView, context: Context) {
        if view.playerLayer.player !== player { view.playerLayer.player = player }
    }
}
#endif

// MARK: - The circle

/// Where a circle is on its way to the server (S5.6). Only a SENT one can
/// play: until the server has the message it has no attachment to stream.
nonisolated enum RoundUpload: Equatable, Sendable {
    case sent
    /// The sender's own, still going up: "Sending…" and a thin neutral ring.
    case sending
    /// The sender's own, refused for good: the poster alone — no play disc,
    /// no ring — under today's failed bubble with Try Again and Delete.
    case failed

    static func of(isMine: Bool, serverID: Int64?, state: MessageStatus) -> RoundUpload {
        guard isMine, serverID == nil else { return .sent }
        switch state {
        case .pending: return .sending
        case .failed: return .failed
        case .sent: return .sent
        }
    }

    static func of(_ message: MessageSnapshot, isMine: Bool) -> RoundUpload {
        of(isMine: isMine, serverID: message.serverID, state: message.state)
    }

    /// A tap, Play and the play disc are offered.
    var playable: Bool { self == .sent }
}

struct RoundVideoTile: View {
    let attachment: AttachmentDTO
    /// The reader's own message: no unplayed dot — they recorded it.
    var isMine: Bool = false
    /// Sent, still going up, or failed (S5.6): nothing plays until it is
    /// sent — the server has no id for it.
    var upload: RoundUpload = .sent
    private var isSending: Bool { upload == .sending }
    var onDoubleTap: () -> Void = {}
    /// The message menu. The Mac's row has its own context menu, and passes
    /// nil.
    var onLongPress: (() -> Void)? = nil
    /// "Open Full Screen" — the existing viewer, with scrubbing (S5.4).
    var onOpenFullScreen: () -> Void = {}

    @Environment(AttachmentStore.self) private var store
    /// Optional, unlike the Mac row's own: a layout test that draws a circle
    /// should not have to assemble a coordinator. Without one there is
    /// nothing to stream from, and a tap says it could not load.
    @Environment(ChatSyncCoordinator.self) private var coordinator: ChatSyncCoordinator?
    #if os(iOS)
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    #endif
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.colorSchemeContrast) private var contrast

    @State private var playback = RoundVideoPlayback()
    /// "You can play this after recording." — a tap that came while a
    /// recording runs (S1.7).
    @State private var saysAfterRecording = false
    @State private var sendingSpin = false

    /// 200 in a compact width, 240 otherwise (S5.2). The Mac is never
    /// compact.
    var diameter: CGFloat {
        #if os(iOS)
        CGFloat(RoundVideo.diameter(horizontalSizeClass == .compact ? .compact : .regular))
        #else
        CGFloat(RoundVideo.diameter(.regular))
        #endif
    }

    private var plays: RoundVideoPlays { .shared }

    /// The dot: somebody else's circle this device has not played (S5.2).
    private var showsUnplayedDot: Bool {
        !isMine && upload.playable && attachment.id > 0 && !plays.isPlayed(attachment.id)
    }

    /// The poster, and only the poster: the protocol's "a tile never
    /// downloads a VIDEO to draw itself". A video poster may arrive late,
    /// so the store re-asks a bounded number of times.
    private var poster: Image? {
        _ = store.generation
        return store.image(id: attachment.id, preview: true, mayArriveLate: true)
    }

    private var durationSeconds: TimeInterval { Double(attachment.durationMS ?? 0) / 1000 }

    private var durationLabel: String { AudioRecorder.timeLabel(durationSeconds) }

    /// Under Increase Contrast the ring and the dot take the system's
    /// strongest ink (S6, "Low vision").
    private var accent: Color { contrast == .increased ? .primary : .accentColor }

    /// The ring's progress — stepped once a second under Reduce Motion (S6).
    private var ringProgress: Double {
        guard reduceMotion, durationSeconds > 0 else { return playback.progress }
        return (playback.progress * durationSeconds).rounded(.down) / durationSeconds
    }

    var body: some View {
        VStack(spacing: 4) {
            circle
            if isSending {
                Text("Sending…")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            } else if saysAfterRecording {
                Text("You can play this after recording.")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: diameter)
            }
        }
        .onDisappear { playback.stop() }
        .onChange(of: VoiceRecordingArbiter.shared.isRecording) { _, now in
            if !now { saysAfterRecording = false }
        }
    }

    private var circle: some View {
        ZStack {
            // A neutral disc of the final size until the poster lands, so
            // the row never changes height.
            Circle()
                .fill(Color.primary.opacity(0.10))
            if let poster {
                poster
                    .resizable()
                    .aspectRatio(contentMode: .fill)
            }
            if playback.hasFrames, playback.isActive, let player = playback.player {
                RoundVideoSurface(player: player)
                    .accessibilityHidden(true)
            }
            if playback.phase == .failed {
                Circle().fill(.black.opacity(0.45))
                VStack(spacing: 6) {
                    Image(systemName: "arrow.clockwise")
                        .font(.system(size: 22, weight: .semibold))
                    Text("Couldn't load the video. Tap to try again.")
                        .font(.caption)
                        .multilineTextAlignment(.center)
                }
                .foregroundStyle(.white)
                .padding(diameter * 0.18)
            }
        }
        .frame(width: diameter, height: diameter)
        .clipShape(Circle())
        .overlay { rings }
        .overlay { centreGlyph }
        .overlay(alignment: .bottom) { footer }
        .overlay(alignment: .topTrailing) { expandControl }
        .contentShape(Circle())
        // Count 2 BEFORE count 1, and both as onTapGesture: exclusive, so
        // the single tap waits out the double-tap window and the double tap
        // stays the heart (the sticker's precedent).
        .onTapGesture(count: 2) { onDoubleTap() }
        .onTapGesture(count: 1) { tap() }
        .modifier(LongPressIfAny(action: onLongPress))
        #if os(macOS)
        .hoverCursor(.pointingHand)
        #endif
        // A circle stops when its row scrolls out of view (S5.3). Geometry,
        // not onDisappear: the thread is a non-lazy window, and every row in
        // it "appears" at creation.
        .onGeometryChange(for: Bool.self) { geometry in
            guard let viewport = geometry.bounds(of: .scrollView) else { return true }
            let frame = geometry.frame(in: .scrollView)
            return frame.maxY >= 0 && frame.minY <= viewport.height
        } action: { visible in
            if !visible, playback.phase != .idle { playback.stop() }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(String(localized: "Video message, \(durationLabel)")))
        .accessibilityValue(showsUnplayedDot ? Text("Not played") : Text(verbatim: ""))
        .accessibilityAddTraits(.isButton)
        .accessibilityAddTraits(.startsMediaSession)
        .accessibilityHint(!upload.playable ? Text(verbatim: "") : playback.phase == .playing ? Text("Pause") : Text("Play"))
        // The default action is Play/Pause (S6).
        .accessibilityAction { tap() }
        .accessibilityAction(named: Text("Open Full Screen")) { openFullScreen() }
    }

    @ViewBuilder
    private var rings: some View {
        if isSending {
            // The upload has no byte count to show here, so the thin neutral
            // ring says only that it is going (S5.6).
            Circle()
                .trim(from: 0, to: reduceMotion ? 1 : 0.25)
                .stroke(Color.secondary.opacity(0.7), style: StrokeStyle(lineWidth: 2, lineCap: .round))
                .rotationEffect(.degrees(sendingSpin ? 360 : 0))
                .padding(1)
                .onAppear {
                    guard !reduceMotion else { return }
                    withAnimation(.linear(duration: 1.2).repeatForever(autoreverses: false)) {
                        sendingSpin = true
                    }
                }
        } else if playback.phase == .playing || playback.phase == .paused
                    || (playback.phase == .loading && playback.hasFrames) {
            Circle()
                .trim(from: 0, to: ringProgress)
                .stroke(accent, style: StrokeStyle(lineWidth: 3, lineCap: .round))
                .rotationEffect(.degrees(-90))
                .padding(1.5)
        }
    }

    @ViewBuilder
    private var centreGlyph: some View {
        switch playback.phase {
        case .loading:
            ZStack {
                Circle().fill(.black.opacity(0.35)).frame(width: 44, height: 44)
                ProgressView()
                    .tint(.white)
            }
            .accessibilityHidden(true)
        case .idle, .paused:
            if upload.playable {
                ZStack {
                    Circle().fill(.black.opacity(0.45))
                    Image(systemName: "play.fill")
                        .font(.system(size: 18, weight: .semibold))
                        .foregroundStyle(.white)
                        .offset(x: 2)
                }
                .frame(width: 44, height: 44)
                .accessibilityHidden(true)
            }
        case .playing, .failed:
            EmptyView()
        }
    }

    /// The duration capsule at the bottom centre inside the circle, and the
    /// unplayed dot beside it (S5.2).
    private var footer: some View {
        HStack(spacing: 4) {
            Text(verbatim: durationLabel)
                .font(.caption2.weight(.medium).monospacedDigit())
                .foregroundStyle(.white)
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(.black.opacity(0.55), in: Capsule())
            if showsUnplayedDot {
                Circle()
                    .fill(accent)
                    .frame(width: 8, height: 8)
            }
        }
        .padding(.bottom, diameter * 0.07)
        .accessibilityHidden(true)
    }

    /// While it plays: a 28-unit glyph in a 44-unit target at the top
    /// trailing edge, doing what "Open Full Screen" does (S5.4).
    @ViewBuilder
    private var expandControl: some View {
        if playback.phase == .playing || playback.phase == .paused {
            Button(action: openFullScreen) {
                Image(systemName: "arrow.up.left.and.arrow.down.right.circle.fill")
                    .font(.system(size: 28))
                    .foregroundStyle(.white, .black.opacity(0.45))
                    .frame(width: 44, height: 44)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(Text("Open Full Screen"))
            .help(Text("Open Full Screen"))
            .offset(x: -diameter * 0.06, y: diameter * 0.06)
        }
    }

    private func tap() {
        guard upload.playable else { return }
        let attachment = attachment
        let api = coordinator?.api
        playback.makeEngine = {
            guard let api else { return nil }
            return SystemRoundVideoEngine(attachment: attachment, api: api)
        }
        playback.onFinished = { RoundVideoPlays.shared.markPlayed(attachment.id) }
        switch playback.tap() {
        case .refusedWhileRecording:
            saysAfterRecording = true
            AccessibilityNotification.Announcement(
                String(localized: "You can play this after recording.")
            ).post()
        case .started, .paused, .gaveUp:
            saysAfterRecording = false
        }
    }

    private func openFullScreen() {
        // The viewer plays it from the start with its own controls; the
        // circle lets go first, so two copies never play at once.
        playback.stop()
        onOpenFullScreen()
    }
}

/// A long press, when the surface has a touch menu to open — the phone's
/// bubble. The Mac row passes nil and keeps its context menu.
private struct LongPressIfAny: ViewModifier {
    let action: (() -> Void)?

    func body(content: Content) -> some View {
        if let action {
            content.simultaneousGesture(LongPressGesture().onEnded { _ in action() })
        } else {
            content
        }
    }
}
