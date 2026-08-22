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
        if attachment.isFile {
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
                .foregroundStyle(.tint)
                .frame(width: 34)
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
        .padding(.vertical, 6)
        .padding(.horizontal, 8)
        .frame(maxWidth: Self.maxWidth)
        .background(.black.opacity(0.06), in: RoundedRectangle(cornerRadius: 10))
        .contentShape(Rectangle())
        .onTapGesture { onOpen() }
        .simultaneousGesture(TapGesture(count: 2).onEnded { onDoubleTap() })
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
            RoundedRectangle(cornerRadius: 10)
                .fill(.black.opacity(0.08))
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
        .clipShape(RoundedRectangle(cornerRadius: 10))
        .contentShape(Rectangle())
        .onTapGesture { onOpen() }
        .simultaneousGesture(TapGesture(count: 2).onEnded { onDoubleTap() })
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
