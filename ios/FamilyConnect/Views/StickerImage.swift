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

struct StickerImage: View {
    /// The attachment whose bytes are the sticker — a message's own copy,
    /// a pack item's picture, or a pending send's provisional id.
    let attachmentID: Int64
    /// False where a moving picture would be noise rather than the point:
    /// the panel's grid and the pack on the Family screen show two hundred
    /// of these at once. Frame zero is a correct drawing of any sticker.
    var animates = true

    @Environment(AttachmentStore.self) private var store
    @State private var player = StickerPlayer()

    var body: some View {
        // Reading `generation` is what redraws this when the fetch lands —
        // the store's caches are ObservationIgnored (AttachmentView has the
        // whole reason).
        let _ = store.generation
        // The ORIGINAL, unconditionally (see the file header) — and decoded
        // off the main actor, which is what `stickerImage` is for: this
        // body runs once per cell of a panel of two hundred.
        let still = store.stickerImage(id: attachmentID)
        ZStack {
            if let frame = player.frame {
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
        .task(id: still != nil) {
            guard animates, still != nil,
                  let data = await StickerPack.animatedBytes(
                      fileAt: store.originalURL(id: attachmentID)),
                  !Task.isCancelled
            else { return }
            player.play(data)
        }
        .onDisappear { player.stop() }
    }
}
