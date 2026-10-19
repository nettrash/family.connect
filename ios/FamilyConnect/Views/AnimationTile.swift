//
//  AnimationTile.swift
//  FamilyConnect
//
//  An ANIMATION, drawn (issue #96, docs/protocol.md "Animations" › "How it
//  is drawn"): a picture that moves, not a video. Shared by the phone's
//  AttachmentView and the Mac's MacAttachmentBlock, which keep their own
//  tile — its box, its poster, its gestures — and lay these over it:
//
//    - `AnimationSurface`, the video itself, muted and looping, over the
//      tile's poster — only while the tile is ON SCREEN, never when the
//      system asks for less motion, and never above 8 MiB (then the poster
//      stays and a tap plays it in the viewer). Below that the tile
//      DOWNLOADS the video to draw itself (`AttachmentStore.animationURL`),
//      the protocol's one exception to "a tile never downloads a video".
//    - `AnimationBadge`, the small "GIF" in a corner — no play button, no
//      duration, no sound control.
//    - `AnimationViewerPlayer`, the viewer's: larger, playing, muted,
//      looping, without player controls, whatever the size or the motion
//      setting — a tap asked for it.
//
//  THE AUDIO SESSION IS NEVER TOUCHED. The video has no sound track and the
//  player is muted; nothing here takes `.playback`, joins NowPlaying's
//  one-thing-plays rule or activates the session, so a GIF scrolling past
//  never interrupts somebody's music or a voice note that is playing.
//

import AVFoundation
import SwiftUI
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

// MARK: - The rules

/// When an animation plays by itself. Pure, so it is tested without a view.
nonisolated enum AnimationPlayback {

    /// Above this, a tile shows its poster and plays only when tapped: below
    /// it, the tile downloads the video to draw itself (docs/protocol.md,
    /// "How it is drawn").
    static let tileCeilingBytes: Int64 = 8 * 1024 * 1024

    /// Does a tile play by itself? Not when the system asks for less motion,
    /// and not above the ceiling. A size of 0 is a pending send's own
    /// provisional row, whose video is already on this device.
    static func playsInTile(size: Int64, reduceMotion: Bool) -> Bool {
        !reduceMotion && size <= tileCeilingBytes
    }
}

// MARK: - The label

/// The small "GIF" in a tile's corner — the word every messenger uses, the
/// same in every language.
struct AnimationBadge: View {
    var body: some View {
        Text("GIF")
            .font(.caption2.weight(.bold))
            .foregroundStyle(.white)
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background(.black.opacity(0.55), in: Capsule())
            .padding(6)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomLeading)
            .allowsHitTesting(false)
            .accessibilityHidden(true)
    }
}

// MARK: - One looping, muted player

/// An `AVQueuePlayer` looping one item through `AVPlayerLooper`, muted.
/// One per surface, torn down with it.
@MainActor
@Observable
final class LoopingVideo {
    /// The player a surface shows, or nil while there is none.
    private(set) var player: AVQueuePlayer?
    @ObservationIgnored private var looper: AVPlayerLooper?
    /// What is playing, so asking again for the same thing is a no-op.
    @ObservationIgnored private var source: URL?

    /// Play `url` (a local file, or a stream with `headers`) on a loop —
    /// or, for nil, stop.
    func show(_ url: URL?, headers: [String: String] = [:]) {
        guard url != source else { return }
        stop()
        guard let url else { return }
        source = url
        let options: [String: Any] = headers.isEmpty
            ? [:] : ["AVURLAssetHTTPHeaderFieldsKey": headers]
        let item = AVPlayerItem(asset: AVURLAsset(url: url, options: options))
        let player = AVQueuePlayer()
        // Silent by construction, and said twice: the file has no sound
        // track, and the player is muted anyway.
        player.isMuted = true
        // A GIF on screen is not a reason to keep the screen awake, and it
        // is nothing to show on a television.
        player.preventsDisplaySleepDuringVideoPlayback = false
        #if os(iOS)
        player.allowsExternalPlayback = false
        #endif
        looper = AVPlayerLooper(player: player, templateItem: item)
        self.player = player
        player.play()
    }

    /// Stop and release. Idempotent.
    func stop() {
        player?.pause()
        looper?.disableLooping()
        looper = nil
        player = nil
        source = nil
    }
}

// MARK: - The surface

#if os(iOS)
final class AnimationPlayerLayerView: UIView {
    override class var layerClass: AnyClass { AVPlayerLayer.self }
    var playerLayer: AVPlayerLayer { layer as! AVPlayerLayer }

    override init(frame: CGRect) {
        super.init(frame: frame)
        clipsToBounds = true
        isUserInteractionEnabled = false
        isAccessibilityElement = false
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
}

/// An `AVPlayerLayer`, which draws nothing until it has a frame — so the
/// poster under it shows until the video moves.
struct AnimationPlayerSurface: UIViewRepresentable {
    let player: AVPlayer
    var gravity: AVLayerVideoGravity = .resizeAspectFill
    var cornerRadius: CGFloat = 0

    func makeUIView(context: Context) -> AnimationPlayerLayerView {
        let view = AnimationPlayerLayerView()
        view.playerLayer.player = player
        view.playerLayer.videoGravity = gravity
        view.layer.cornerRadius = cornerRadius
        return view
    }

    func updateUIView(_ view: AnimationPlayerLayerView, context: Context) {
        if view.playerLayer.player !== player { view.playerLayer.player = player }
        view.playerLayer.videoGravity = gravity
        view.layer.cornerRadius = cornerRadius
    }
}
#elseif os(macOS)
/// The Mac's surface: a layer-hosting view whose layer IS the player layer,
/// clipped by that layer itself — the reason is RoundPlayerLayerView's.
final class AnimationPlayerLayerView: NSView {
    let playerLayer = AVPlayerLayer()

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        layer = playerLayer
        wantsLayer = true
        clipsToBounds = true
        setAccessibilityElement(false)
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func round(_ radius: CGFloat) {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        playerLayer.cornerRadius = radius
        playerLayer.masksToBounds = true
        CATransaction.commit()
    }

    /// Clicks belong to the tile's gestures, not to this view.
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

struct AnimationPlayerSurface: NSViewRepresentable {
    let player: AVPlayer
    var gravity: AVLayerVideoGravity = .resizeAspectFill
    var cornerRadius: CGFloat = 0

    func makeNSView(context: Context) -> AnimationPlayerLayerView {
        let view = AnimationPlayerLayerView()
        view.playerLayer.player = player
        view.playerLayer.videoGravity = gravity
        view.round(cornerRadius)
        return view
    }

    func updateNSView(_ view: AnimationPlayerLayerView, context: Context) {
        if view.playerLayer.player !== player { view.playerLayer.player = player }
        view.playerLayer.videoGravity = gravity
        view.round(cornerRadius)
    }
}
#endif

// MARK: - In a tile

/// The moving half of an animation's tile, laid over its poster.
struct AnimationSurface: View {
    let attachment: AttachmentDTO
    /// The tile's corner, so the video is cut where the poster is.
    var cornerRadius: CGFloat = 0

    @Environment(AttachmentStore.self) private var store
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    /// On screen, by geometry: the thread is a non-lazy window of rows, so
    /// `onAppear` says nothing about whether the reader can see this one.
    @State private var isVisible = false
    @State private var video = LoopingVideo()

    var body: some View {
        // `generation` is what redraws this when the download lands — the
        // store's caches are ObservationIgnored (AttachmentView explains).
        let _ = store.generation
        let plays = isVisible
            && AnimationPlayback.playsInTile(size: attachment.size, reduceMotion: reduceMotion)
        // Asked only when it would play: that is what starts the download.
        let url = plays ? store.animationURL(id: attachment.id) : nil
        ZStack {
            if let player = video.player {
                AnimationPlayerSurface(player: player, cornerRadius: cornerRadius)
            }
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
        .onGeometryChange(for: Bool.self) { geometry in
            // Outside a scroll view (a preview, a test host) it is on screen
            // when it is drawn at all.
            guard let viewport = geometry.bounds(of: .scrollView) else { return true }
            let frame = geometry.frame(in: .scrollView)
            return frame.maxY >= 0 && frame.minY <= viewport.height
                && frame.maxX >= 0 && frame.minX <= viewport.width
        } action: { visible in
            isVisible = visible
        }
        .onChange(of: url, initial: true) { _, url in
            video.show(url)
        }
        .onDisappear { video.stop() }
    }
}

// MARK: - In the viewer

/// The viewer's animation: larger, playing, muted, looping, no controls.
/// Plays whatever the size or the motion setting — the reader tapped it.
/// From the downloaded copy when there is one (or can be, at most 8 MiB),
/// otherwise streamed with the session's Authorization header.
struct AnimationViewerPlayer: View {
    let attachment: AttachmentDTO
    /// Whether this is the page that is up; a neighbour materialised early
    /// by a lazy pager plays nothing.
    var isCurrent: Bool = true

    @Environment(AttachmentStore.self) private var store
    @Environment(ChatSyncCoordinator.self) private var coordinator
    @State private var video = LoopingVideo()

    var body: some View {
        let _ = store.generation
        let local = isCurrent && attachment.size <= AnimationPlayback.tileCeilingBytes
            ? store.animationURL(id: attachment.id) : nil
        ZStack {
            // The poster until the video moves.
            if let poster = store.image(id: attachment.id, preview: true, mayArriveLate: true) {
                poster
                    .resizable()
                    .aspectRatio(contentMode: .fit)
            }
            if let player = video.player {
                AnimationPlayerSurface(player: player, gravity: .resizeAspect)
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text("GIF"))
        .task(id: [isCurrent, local != nil]) {
            guard isCurrent else {
                video.stop()
                return
            }
            if let local {
                video.show(local)
            } else if attachment.size > AnimationPlayback.tileCeilingBytes,
                      let stream = await coordinator.api.attachmentStreamURL(id: attachment.id),
                      !Task.isCancelled {
                video.show(stream.url, headers: stream.headers)
            }
        }
        .onDisappear { video.stop() }
    }
}
