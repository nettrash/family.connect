//
//  AttachmentView.swift
//  FamilyConnect
//
//  A photo or video inside a bubble: the preview, at the attachment's own
//  aspect ratio, with a play badge and duration on a video.
//
//  The bubble reserves the right SHAPE before any bytes arrive — the
//  attachment carries its dimensions, so the placeholder is exactly the
//  size the image will be. Without that, every photo that finishes loading
//  would resize its bubble and shove the whole thread.
//
//  Several photos in one message are not several of these: they are one
//  AlbumStackView, which borrows the placeholder and the video badge
//  from here so a pile's top card looks exactly like a lone photo.
//
//  Android counterpart: the attachment block in ui/chat/ChatScreen.kt
//

// iOS only — the Mac has its own views (MacViews/).
#if os(iOS)

import SwiftUI

struct AttachmentView: View {
    let attachment: AttachmentDTO
    let onOpen: () -> Void
    /// The bubble's own gestures, forwarded.
    ///
    /// A caption-less photo IS the balloon — its own tap handler used to
    /// consume the press, so long-pressing to react or double-tapping to
    /// heart worked only on the few points of padding around the edge.
    /// Same defect the reply quote had, and the same fix.
    var onLongPress: () -> Void = {}
    var onDoubleTap: () -> Void = {}
    /// Which balloon this is sitting in.
    ///
    /// Needed for CONTRAST, not decoration: an own balloon is filled with
    /// the tint, and a file row that painted its icon `.tint` there was
    /// drawing tint on tint — invisible. Everything inside a balloon has to
    /// take its colour from whatever is directly behind it.
    var isMine: Bool = false
    /// False when this tile IS the message — a media-only bubble, drawn
    /// with no balloon behind it. Only the hairline reads it
    /// (MessagePresentation.drawsHairline): with no balloon there is
    /// nothing for a pale photo to melt into, and the stroke would be a
    /// frame around a picture.
    var onBalloon: Bool = true

    @Environment(AttachmentStore.self) private var store

    /// Widest a bubble's media gets. Beyond this a photo stops being a
    /// message and starts being a wall.
    static let maxWidth: CGFloat = 240
    private static let maxHeight: CGFloat = 320

    private var size: CGSize {
        let ratio = attachment.aspectRatio
        var width = Self.maxWidth
        var height = width / ratio
        if height > Self.maxHeight {
            height = Self.maxHeight
            width = height * ratio
        }
        return CGSize(width: width, height: height)
    }

    /// The preview when there is one; the full photo otherwise, so a
    /// message sent before its preview landed still shows something.
    ///
    /// READING `store.generation` HERE IS LOAD-BEARING. The store's caches
    /// are @ObservationIgnored (a dictionary that changes on every scroll
    /// frame would invalidate every reader), so `generation` is the only
    /// observable thing it has — and without touching it this view
    /// registers no dependency at all and never redraws when the fetch
    /// lands. The bubble then sits on its spinner until something else
    /// happens to rebuild it, which is why leaving the chat and coming
    /// back "fixed" it. Same reason AttachmentViewer reads it.
    private var image: Image? {
        _ = store.generation
        return Self.image(for: attachment, in: store)
    }

    /// The lookup behind `image`, for any view that draws a bubble-sized
    /// preview of an attachment. The caller reads `store.generation` — the
    /// paragraph above is about the caller, not this function.
    ///
    /// `hasPreview` IS A HINT, NOT A FACT, and the difference is the whole
    /// reason the two kinds are treated differently here.
    ///
    /// The flag is a snapshot the server took when this device happened to
    /// read the message, and it is the one attachment field that changes
    /// afterwards (`AttachmentDTO.withPreviewFlag`). Nothing can correct a
    /// stale copy: history sync is `after_id` only, so it cannot see a
    /// mutation of an older row — the constraint that forced separate seqs
    /// for reactions, edits and board notes — and a message already held is
    /// not rewritten when it is re-delivered. A `false` stored on a video
    /// was therefore permanent, and so was the grey tile it produced,
    /// because a video has no second source of pixels: leaving the chat,
    /// relaunching, nothing brought the poster back.
    ///
    /// So a PHOTO may still skip the request on the flag — being wrong
    /// there costs nothing, since the full bytes are the fallback — while a
    /// VIDEO always asks. The server is the only thing that actually knows.
    /// A 404 settles the key after a bounded re-check and the tile keeps its
    /// play badge; see `AttachmentStore.image(id:preview:mayArriveLate:)`.
    static func image(for attachment: AttachmentDTO, in store: AttachmentStore) -> Image? {
        if attachment.hasPreview || attachment.isVideo {
            return store.image(
                id: attachment.id, preview: true, mayArriveLate: attachment.isVideo)
        }
        return store.image(id: attachment.id, preview: false)
    }

    /// What a media tile shows until its bytes arrive. A soft vertical
    /// ramp rather than flat grey: while the bytes are still coming this
    /// rectangle is all there is to look at, and a dead slab reads as a
    /// broken image.
    static func placeholder(isMine: Bool) -> LinearGradient {
        LinearGradient(
            colors: isMine
                ? [.white.opacity(0.22), .white.opacity(0.10)]
                : [.primary.opacity(0.10), .primary.opacity(0.04)],
            startPoint: .top,
            endPoint: .bottom)
    }

    /// True while bytes we expect are still on their way.
    ///
    /// A video whose flag says there is no poster is asked about anyway
    /// (see `image(for:in:)`), but it does not get a SPINNER for it: the
    /// answer may well be that there is no poster and never will be, and a
    /// spinner promises an arrival. It keeps the play badge over a plain
    /// placeholder, and if the poster does turn up it simply appears.
    private var isAwaitingBytes: Bool {
        image == nil && (attachment.hasPreview || !attachment.isVideo)
    }

    var body: some View {
        if attachment.isLocation {
            // Its own view: a location has no bytes, so none of the
            // download/preview machinery below applies to it at all.
            LocationAttachmentView(
                attachment: attachment,
                onLongPress: onLongPress,
                onDoubleTap: onDoubleTap,
                isMine: isMine)
        } else if attachment.isAudio {
            // Audio has nothing to look at, so it gets a player rather than
            // a tile or a document row (docs/protocol.md, "Audio").
            AudioPlayerView(attachment: attachment, isMine: isMine)
                .onTapGesture(count: 2) { onDoubleTap() }
                .simultaneousGesture(LongPressGesture().onEnded { _ in onLongPress() })
        } else if attachment.isFile {
            fileRow
        } else {
            mediaThumbnail
        }
    }

    /// A file has nothing to show but what it is: an icon, its name, and
    /// its size. Deliberately a ROW, not a square — a document is a line
    /// item in a conversation, and a 240pt tile of grey would be a lie
    /// about how much there is to look at.
    @ViewBuilder
    private var fileRow: some View {
        HStack(spacing: 10) {
            Image(systemName: Self.icon(for: attachment.mime))
                .font(.system(size: 26))
                .foregroundStyle(isMine ? AnyShapeStyle(.white) : AnyShapeStyle(.tint))
                .frame(width: 36, height: 36)
                .background(
                    (isMine ? Color.white.opacity(0.18) : Color.accentColor.opacity(0.12)),
                    in: RoundedRectangle(cornerRadius: 9, style: .continuous))
            VStack(alignment: .leading, spacing: 1) {
                Text(attachment.displayName)
                    .font(.subheadline.weight(.medium))
                    .lineLimit(2)
                    .truncationMode(.middle)
                Text(attachment.displaySize)
                    .font(.caption)
                    .opacity(0.7)
            }
            Spacer(minLength: 0)
        }
        .padding(.vertical, 8)
        .padding(.horizontal, 10)
        .frame(maxWidth: Self.maxWidth)
        // A wash that works on BOTH grounds: lightening over the tinted own
        // balloon, darkening over the neutral one. A single black wash went
        // muddy over the accent colour. Theirs-side uses .primary, not
        // .black, so the wash flips with the appearance instead of
        // vanishing on a dark balloon (identical in light mode).
        .background(
            (isMine ? Color.white.opacity(0.14) : Color.primary.opacity(0.05)),
            in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .strokeBorder(isMine ? Color.white.opacity(0.16) : Color.primary.opacity(0.06)))
        .contentShape(Rectangle())
        // Count 2 BEFORE count 1, and both as onTapGesture: that is what
        // makes them exclusive. The double tap used to be a
        // `.simultaneousGesture`, which means "recognise alongside" — so a
        // double tap fired the heart AND opened the attachment, because the
        // first tap-up had already run onOpen.
        .onTapGesture(count: 2) { onDoubleTap() }
        .onTapGesture(count: 1) { onOpen() }
        .simultaneousGesture(LongPressGesture().onEnded { _ in onLongPress() })
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(attachment.displayName), \(attachment.displaySize)")
        .accessibilityAddTraits(.isButton)
        .accessibilityAction { onOpen() }
    }

    /// SF Symbols only — a document icon set would be a pile of assets for
    /// something the system already draws.
    static func icon(for mime: String) -> String {
        switch mime {
        case "application/pdf": "doc.richtext"
        case let m where m.hasPrefix("audio/"): "waveform"
        case let m where m.hasPrefix("video/"): "film"
        case let m where m.hasPrefix("image/"): "photo"
        case let m where m.hasPrefix("text/"): "doc.plaintext"
        case "application/zip", "application/x-tar", "application/gzip",
             "application/x-7z-compressed", "application/vnd.rar":
            "doc.zipper"
        default: "doc"
        }
    }

    @ViewBuilder
    private var mediaThumbnail: some View {
        ZStack {
            Self.placeholder(isMine: isMine)
            if let image {
                image
                    .resizable()
                    .aspectRatio(contentMode: .fill)
            } else if isAwaitingBytes {
                ProgressView()
            }
            if attachment.isVideo {
                VideoBadgeOverlay(attachment: attachment)
            }
        }
        .frame(width: size.width, height: size.height)
        .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
        // A hairline only where the tile's own pixels are not already the
        // edge — over a balloon, or before the picture lands
        // (MessagePresentation.drawsHairline has the rule and the reason).
        .overlay {
            if MessagePresentation.drawsHairline(onBalloon: onBalloon, hasImage: image != nil) {
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .strokeBorder(isMine ? Color.white.opacity(0.18) : Color.primary.opacity(0.08))
            }
        }
        .contentShape(Rectangle())
        // Count 2 BEFORE count 1, and both as onTapGesture: that is what
        // makes them exclusive. The double tap used to be a
        // `.simultaneousGesture`, which means "recognise alongside" — so a
        // double tap fired the heart AND opened the attachment, because the
        // first tap-up had already run onOpen.
        .onTapGesture(count: 2) { onDoubleTap() }
        .onTapGesture(count: 1) { onOpen() }
        .simultaneousGesture(LongPressGesture().onEnded { _ in onLongPress() })
        .accessibilityElement(children: .combine)
        .accessibilityLabel(attachment.isVideo ? "Video" : "Photo")
        .accessibilityAddTraits(.isButton)
        // A bare gesture publishes no accessibility action — measured, see
        // ZZAXProbeTests.
        .accessibilityAction { onOpen() }
    }

}

/// The play badge and, when known, the duration in the bottom-trailing
/// corner — over a lone video and over an album whose top card is one.
struct VideoBadgeOverlay: View {
    let attachment: AttachmentDTO

    var body: some View {
        ZStack {
            Image(systemName: "play.circle.fill")
                .font(.system(size: 42))
                .foregroundStyle(.white.opacity(0.9))
                .shadow(radius: 4)
            if let durationMS = attachment.durationMS, durationMS > 0 {
                VStack {
                    Spacer()
                    HStack {
                        Spacer()
                        Text(Self.duration(durationMS))
                            .font(.caption2.weight(.medium))
                            .foregroundStyle(.white)
                            .padding(.horizontal, 6)
                            .padding(.vertical, 2)
                            .background(.black.opacity(0.55), in: Capsule())
                            .padding(6)
                    }
                }
            }
        }
    }

    private static func duration(_ milliseconds: Int) -> String {
        let total = milliseconds / 1000
        return String(format: "%d:%02d", total / 60, total % 60)
    }
}

#endif
