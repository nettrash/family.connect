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
    /// Non-nil when this attachment is one CELL of a multi-attachment
    /// grid: a square of exactly this side, filled and clipped, instead
    /// of the free-standing thumbnail sized by the attachment's own
    /// aspect ratio. Media only — files and audio stack as rows.
    var cellSide: CGFloat? = nil

    @Environment(AttachmentStore.self) private var store

    /// Widest a bubble's media gets. Beyond this a photo stops being a
    /// message and starts being a wall.
    static let maxWidth: CGFloat = 240
    private static let maxHeight: CGFloat = 320

    private var size: CGSize {
        // A grid cell's shape is the grid's, not the attachment's.
        if let cellSide { return CGSize(width: cellSide, height: cellSide) }
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
        if attachment.hasPreview {
            return store.image(id: attachment.id, preview: true)
        }
        return attachment.isVideo ? nil : store.image(id: attachment.id, preview: false)
    }

    /// True while bytes we expect are still on their way. A video with no
    /// preview is NOT waiting for anything — nobody uploaded a poster and
    /// nobody will — so a spinner there promises an arrival that never
    /// comes. It gets the play badge over a plain placeholder instead.
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
            // A soft vertical ramp rather than flat grey: while the bytes
            // are still coming this rectangle is all there is to look at,
            // and a dead slab reads as a broken image.
            LinearGradient(
                colors: isMine
                    ? [.white.opacity(0.22), .white.opacity(0.10)]
                    : [.primary.opacity(0.10), .primary.opacity(0.04)],
                startPoint: .top,
                endPoint: .bottom)
            if let image {
                image
                    .resizable()
                    .aspectRatio(contentMode: .fill)
            } else if isAwaitingBytes {
                ProgressView()
            }
            if attachment.isVideo {
                videoOverlay
            }
        }
        .frame(width: size.width, height: size.height)
        .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
        // A hairline so a pale photo does not melt into a pale balloon.
        .overlay(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .strokeBorder(isMine ? Color.white.opacity(0.18) : Color.primary.opacity(0.08)))
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

    @ViewBuilder
    private var videoOverlay: some View {
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
