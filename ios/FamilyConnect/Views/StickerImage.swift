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
//  ANIMATION is ImageIO's: `CGAnimateImageDataWithBlock` decodes an
//  animated WebP (or an animated PNG) frame by frame at the file's own
//  timing, on the main run loop, and hands each frame over. A still
//  sticker — one frame — never starts it and is drawn once from the
//  store's own decoded copy. Where ImageIO cannot animate the bytes the
//  call fails and the still frame simply stays, which is the protocol's
//  "frame zero where it cannot": the same sticker, holding still.
//
//  WHEN it moves (*amended 2026-10-19, #96*): everywhere a sticker is drawn
//  — the chat, the panel, the family's pack, the enlarged view — but only
//  while it is ON SCREEN (by geometry: the thread is a non-lazy window of
//  rows, so appearing says nothing about being seen), and never while the
//  system asks for less motion, when it is frame zero. `StickerMotion` is
//  that rule.
//

import ImageIO
import SwiftUI

/// Plays one animated sticker. One per view that is animating, torn down
/// with it.
@MainActor @Observable
final class StickerPlayer {
    /// The frame to draw, or nil while there is none — a still sticker, or
    /// an animated one that has not produced its first frame yet. The view
    /// falls back to the store's still either way, so nil is never a blank.
    private(set) var frame: CGImage?

    /// Which run of the animation is current. ImageIO offers no handle to
    /// cancel with — the block is told to stop from INSIDE, on its next
    /// frame — so a run that is no longer wanted is one whose number has
    /// been passed.
    @ObservationIgnored private var run = 0

    /// Start (or restart) from these bytes. Does nothing for a still.
    func play(_ data: Data) {
        run += 1
        let mine = run
        guard StickerPack.frameCount(of: data) > 1 else {
            frame = nil
            return
        }
        // The block runs on the main run loop, which is what makes writing
        // an observed property from it sound.
        let status = CGAnimateImageDataWithBlock(data as CFData, nil) { [weak self] _, image, stop in
            guard let self, self.run == mine else {
                stop.pointee = true
                return
            }
            self.frame = image
        }
        if status != noErr { frame = nil }
    }

    /// Stop after the frame in flight. Idempotent.
    func stop() {
        run += 1
    }
}

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
    @State private var player = StickerPlayer()
    /// On screen, by geometry. False until the first measurement, so a
    /// sticker starts still and moves the moment it is known to be seen.
    @State private var onScreen = false

    var body: some View {
        // Reading `generation` is what redraws this when the fetch lands —
        // the store's caches are ObservationIgnored (AttachmentView has the
        // whole reason).
        let _ = store.generation
        // The ORIGINAL, unconditionally (see the file header) — and decoded
        // off the main actor, which is what `stickerImage` is for: this
        // body runs once per cell of a panel of two hundred.
        let still = store.stickerImage(id: attachmentID)
        let moving = StickerMotion.animates(
            requested: animates, reduceMotion: reduceMotion, onScreen: onScreen)
        ZStack {
            if moving, let frame = player.frame {
                Image(decorative: frame, scale: 1)
                    .resizable()
                    .interpolation(.high)
                    .aspectRatio(contentMode: .fit)
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
        // Keyed on the bytes ARRIVING, so an animation starts the moment
        // there is something to animate and not only on appearance.
        //
        // The bytes are read — and asked whether they move at all — off the
        // main actor; only a sticker that does comes back, so a still one
        // costs this view nothing but its header.
        //
        // And on whether it should move at all: off screen or under Reduce
        // Motion the run is stopped and frame zero — the store's still — is
        // what is drawn.
        .task(id: [still != nil, moving]) {
            guard moving, still != nil else {
                player.stop()
                return
            }
            guard let data = await StickerPack.animatedBytes(
                      fileAt: store.originalURL(id: attachmentID)),
                  !Task.isCancelled
            else { return }
            player.play(data)
        }
        .onGeometryChange(for: Bool.self) { geometry in
            // Outside a scroll view (the enlarged view) it is on screen when
            // it is drawn at all.
            guard let viewport = geometry.bounds(of: .scrollView) else { return true }
            let frame = geometry.frame(in: .scrollView)
            return frame.maxY >= 0 && frame.minY <= viewport.height
                && frame.maxX >= 0 && frame.minX <= viewport.width
        } action: { visible in
            onScreen = visible
        }
        .onDisappear { player.stop() }
    }
}
