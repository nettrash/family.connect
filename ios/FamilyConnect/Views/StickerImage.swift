//
//  StickerImage.swift
//  FamilyConnect
//
//  A sticker's picture: the ORIGINAL bytes, fitted whole, transparent where
//  the sticker is, and moving where the sticker moves (docs/protocol.md,
//  "How it is drawn").
//
//  Shared by both platforms and by every place a sticker is drawn — a chat
//  row, the panel, the Family screen's pack, the enlarged view — so the one
//  rule with no second chance lives in one file: a sticker is drawn from
//  `GET /attachments/{id}` and NEVER from the preview. A preview is a JPEG,
//  which has no transparency and one frame, and `has_preview` can be true
//  on a sticker by inheritance from a photograph with the same bytes. So
//  nothing here reads the flag at all.
//
//  ANIMATION is Core Animation's (*rewritten for speed*): the frames are
//  decoded ONCE, off the main actor, at the size they are drawn, and kept
//  in the store's byte-bounded cache (`StickerFrames`,
//  `AttachmentStore.stickerFrames`); a layer then plays them with a
//  discrete `CAKeyframeAnimation` on its `contents`, so the render server
//  does every tick and neither the main thread nor SwiftUI runs per frame.
//  It used to be `CGAnimateImageDataWithBlock`, one per view: a full-size
//  decode of every frame on the main run loop and a SwiftUI invalidation
//  per tick per sticker, which is what made the whole app slower whenever
//  one moved. A still sticker — one frame — never asks for frames and is
//  drawn once from the store's own decoded copy. Where ImageIO cannot read
//  the frames the decode answers nil and the still simply stays, which is
//  the protocol's "frame zero where it cannot": the same sticker, holding
//  still.
//
//  WHEN it moves (*amended 2026-10-19, #96*): everywhere a sticker is drawn
//  — the chat, the panel, the family's pack, the enlarged view — but only
//  while it is ON SCREEN (by geometry: the thread is a non-lazy window of
//  rows, so appearing says nothing about being seen), and never while the
//  system asks for less motion, when it is frame zero. `StickerMotion` is
//  that rule. Going off screen and coming back no longer re-reads the file
//  or decodes anything: the frames are still in the cache.
//

import QuartzCore
import SwiftUI
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

/// Whether a sticker moves right now (docs/protocol.md, "Sticker pack" ›
/// "How it is drawn", amended for #96). Pure, so it is tested without a view.
nonisolated enum StickerMotion {
    static func animates(requested: Bool, reduceMotion: Bool, onScreen: Bool) -> Bool {
        requested && !reduceMotion && onScreen
    }
}

struct StickerImage: View {
    /// The attachment whose bytes are the sticker — a message's own copy,
    /// a pack item's picture, or a pending send's provisional id.
    let attachmentID: Int64
    /// False only where a caller wants frame zero for its own reasons.
    /// Every place a sticker is drawn animates it since #96 — the panel's
    /// grid and the Family screen's pack included, which used to hold
    /// still; what keeps two hundred of them from all running is that only
    /// the ones on screen do.
    var animates = true

    @Environment(AttachmentStore.self) private var store
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.displayScale) private var displayScale
    /// On screen, by geometry, and how big the box is in POINTS — the one
    /// measurement feeds both, so a moving sticker costs one geometry hook.
    /// Not visible and zero-sized until the first measurement, so a sticker
    /// starts still and moves the moment it is known to be seen.
    @State private var placement = Placement()

    /// What the geometry hook reports.
    nonisolated struct Placement: Equatable, Sendable {
        var onScreen = false
        /// The longer side of the box, in points.
        var edge: CGFloat = 0
    }

    var body: some View {
        // The ORIGINAL, unconditionally (see the file header) — and decoded
        // off the main actor, which is what `stickerImage` is for: this
        // body runs once per cell of a panel of two hundred. The lookup
        // registers this view on its own key, so it redraws when its bytes
        // land and for nothing else.
        let still = store.stickerImage(id: attachmentID)
        let moving = still != nil && StickerMotion.animates(
            requested: animates, reduceMotion: reduceMotion, onScreen: placement.onScreen)
        // Asked for only while it should move: asking is what spends the
        // decode, and an off-screen sticker must not. Cached afterwards, so
        // scrolling it away and back costs a lookup.
        let frames = moving
            ? store.stickerFrames(
                id: attachmentID,
                edge: StickerFrames.requestedEdge(points: placement.edge, scale: displayScale))
            : nil
        ZStack {
            // NEVER both. A sticker is transparent, so a still under the
            // moving layer would show through every frame's transparent
            // pixels — frame zero's arms behind frame six's.
            if let frames {
                // Sized like the still it replaces: a platform view takes
                // the whole box it is offered, and the enlarged view lays
                // the footer out under whatever the picture claims.
                StickerAnimationSurface(frames: frames)
                    .aspectRatio(frames.aspectRatio, contentMode: .fit)
            } else if let still {
                still
                    .resizable()
                    .interpolation(.high)
                    .aspectRatio(contentMode: .fit)
            } else {
                // No wash and no frame behind it: a sticker has no bubble,
                // and a grey rectangle where a transparent picture is about
                // to be would be the bubble coming back for a second.
                ProgressView()
            }
        }
        .onGeometryChange(for: Placement.self) { geometry in
            let edge = max(geometry.size.width, geometry.size.height)
            // Outside a scroll view (the enlarged view) it is on screen when
            // it is drawn at all.
            guard let viewport = geometry.bounds(of: .scrollView) else {
                return Placement(onScreen: true, edge: edge)
            }
            let frame = geometry.frame(in: .scrollView)
            let visible = frame.maxY >= 0 && frame.minY <= viewport.height
                && frame.maxX >= 0 && frame.minX <= viewport.width
            return Placement(onScreen: visible, edge: edge)
        } action: { placement in
            self.placement = placement
        }
    }
}

// MARK: - The layer that plays

/// Hands a sticker's frames to Core Animation: a layer whose `contents`
/// steps through them on a discrete keyframe animation, forever. Once it is
/// added, every tick happens in the render server — this code runs again
/// only when the frames themselves change.
enum StickerLayerAnimation {
    /// The animation's key on the layer.
    static let key = "fc.sticker.frames"

    /// Put `frames` on `layer` and start them, replacing whatever played.
    /// Frame zero is also set as the layer's plain `contents`, so between
    /// the animation being removed (a window change) and re-added there is
    /// still a picture rather than a hole.
    static func play(_ frames: StickerFrames, on layer: CALayer) {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        layer.removeAnimation(forKey: key)
        layer.contents = frames.frames.first
        if let animation = animation(for: frames) {
            layer.add(animation, forKey: key)
        }
        CATransaction.commit()
    }

    /// The keyframe animation for these frames, or nil for fewer than two.
    static func animation(for frames: StickerFrames) -> CAKeyframeAnimation? {
        guard frames.frames.count > 1,
              let keyTimes = StickerFrames.keyTimes(durationsMS: frames.durationsMS)
        else { return nil }
        let animation = CAKeyframeAnimation(keyPath: "contents")
        animation.values = frames.frames
        animation.keyTimes = keyTimes.map { NSNumber(value: $0) }
        animation.calculationMode = .discrete
        animation.duration = Double(frames.totalMS) / 1000
        animation.repeatCount = .infinity
        // Kept across the app going to the background and coming back;
        // a removed-on-completion animation is dropped there, and an
        // infinite one never "completes" to put it back.
        animation.isRemovedOnCompletion = false
        animation.fillMode = .forwards
        return animation
    }
}

#if os(iOS)
/// The view that owns the playing layer. Re-adds the animation when it
/// comes back into a window, which is where UIKit drops a layer's
/// animations (a navigation push and pop, a sheet over it).
final class StickerLayerView: UIView {
    var frames: StickerFrames? {
        didSet { restart() }
    }

    override init(frame: CGRect) {
        super.init(frame: frame)
        isUserInteractionEnabled = false
        isAccessibilityElement = false
        backgroundColor = .clear
        isOpaque = false
        layer.contentsGravity = .resizeAspect
        layer.minificationFilter = .trilinear
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        if window != nil, layer.animation(forKey: StickerLayerAnimation.key) == nil {
            restart()
        }
    }

    private func restart() {
        guard let frames else {
            layer.removeAnimation(forKey: StickerLayerAnimation.key)
            layer.contents = nil
            return
        }
        StickerLayerAnimation.play(frames, on: layer)
    }
}

struct StickerAnimationSurface: UIViewRepresentable {
    let frames: StickerFrames

    func makeUIView(context: Context) -> StickerLayerView {
        let view = StickerLayerView()
        view.frames = frames
        return view
    }

    func updateUIView(_ view: StickerLayerView, context: Context) {
        // Restarting is the expensive half — the frames go to the render
        // server again — so only when they are different frames: a new
        // decode at another size, never an unrelated redraw of the cell.
        if view.frames?.frames.first !== frames.frames.first {
            view.frames = frames
        }
    }
}
#elseif os(macOS)
/// The Mac's playing view: layer-hosting, so the layer is ours to animate,
/// and transparent to clicks, which belong to the tile's gestures.
final class StickerLayerView: NSView {
    private let player = CALayer()
    var frames: StickerFrames? {
        didSet { restart() }
    }

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        player.contentsGravity = .resizeAspect
        player.minificationFilter = .trilinear
        layer = player
        wantsLayer = true
        setAccessibilityElement(false)
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil, player.animation(forKey: StickerLayerAnimation.key) == nil {
            restart()
        }
    }

    private func restart() {
        guard let frames else {
            player.removeAnimation(forKey: StickerLayerAnimation.key)
            player.contents = nil
            return
        }
        StickerLayerAnimation.play(frames, on: player)
    }
}

struct StickerAnimationSurface: NSViewRepresentable {
    let frames: StickerFrames

    func makeNSView(context: Context) -> StickerLayerView {
        let view = StickerLayerView()
        view.frames = frames
        return view
    }

    func updateNSView(_ view: StickerLayerView, context: Context) {
        if view.frames?.frames.first !== frames.frames.first {
            view.frames = frames
        }
    }
}
#endif
