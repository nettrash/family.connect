//
//  AttachmentViewer.swift
//  FamilyConnect
//
//  A photo or video, full screen.
//
//  The two halves work differently on purpose. A photo is FETCHED — the
//  whole JPEG lands in AttachmentStore's disk cache and is then zoomable
//  offline, which is what you want for something you will open again.
//  A video is STREAMED: AVPlayer pulls ranges from the server as it plays,
//  so a 90 MB clip starts in a second and never occupies 90 MB anywhere.
//  That is also why the store has no video path (AttachmentStore's header).
//
//  Android counterpart: ui/chat/AttachmentViewerScreen.kt
//

// iOS only — the Mac has its own views (MacViews/).
#if os(iOS)

import AVKit
import SwiftUI

struct AttachmentViewer: View {
    let attachment: AttachmentDTO
    /// Fetches the bytes and opens the share sheet. Owned by the thread,
    /// which already has the coordinator and the sheet state.
    var onShare: () -> Void = {}

    @Environment(\.dismiss) private var dismiss

    var body: some View {
        ZStack {
            // Black, in both appearances: a photo full-screen wants no
            // surrounding colour competing with it.
            Color.black.ignoresSafeArea()

            if attachment.isVideo {
                VideoAttachmentPlayer(attachment: attachment)
            } else {
                ZoomablePhoto(attachment: attachment)
            }
        }
        .overlay(alignment: .topLeading) {
            Button {
                dismiss()
            } label: {
                Image(systemName: "xmark")
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(.white)
                    .padding(10)
                    .background(.black.opacity(0.45), in: Circle())
            }
            .padding(16)
            .accessibilityLabel("Close")
        }
        .overlay(alignment: .topTrailing) {
            // Share is also how this gets saved: the sheet's own "Save
            // Image" / "Save Video" put it in the library.
            Button {
                dismiss()
                onShare()
            } label: {
                Image(systemName: "square.and.arrow.up")
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundStyle(.white)
                    .padding(10)
                    .background(.black.opacity(0.45), in: Circle())
            }
            .padding(16)
            .accessibilityLabel("Share")
        }
        .statusBarHidden()
    }
}

// MARK: - Photo

/// Pinch and double-tap to zoom, drag to pan while zoomed.
private struct ZoomablePhoto: View {
    let attachment: AttachmentDTO

    @Environment(AttachmentStore.self) private var store

    @State private var zoom: CGFloat = 1
    /// Zoom at the moment the current pinch began; the gesture reports a
    /// factor relative to its own start, not to 1.
    @State private var pinchBase: CGFloat = 1
    @State private var offset: CGSize = .zero
    @State private var dragBase: CGSize = .zero

    private static let maxZoom: CGFloat = 4

    var body: some View {
        GeometryReader { geometry in
            // Reading `store.generation` inside the body is what makes the
            // view re-render when the fetch lands (the store is the only
            // thing that knows), the same trick AttachmentView uses.
            let _ = store.generation
            if let image = store.image(id: attachment.id, preview: false) {
                image
                    .resizable()
                    .aspectRatio(contentMode: .fit)
                    .frame(width: geometry.size.width, height: geometry.size.height)
                    .scaleEffect(zoom)
                    .offset(offset)
                    .gesture(magnification)
                    .simultaneousGesture(pan)
                    .onTapGesture(count: 2) { toggleZoom() }
                    .accessibilityLabel("Photo")
            } else {
                // While the full photo is on its way, show the preview
                // rather than a spinner over black: the bubble already had
                // it, so the transition is continuous.
                ZStack {
                    if attachment.hasPreview,
                       let preview = store.image(id: attachment.id, preview: true) {
                        preview
                            .resizable()
                            .aspectRatio(contentMode: .fit)
                            .blur(radius: 8)
                    }
                    ProgressView()
                        .tint(.white)
                }
                .frame(width: geometry.size.width, height: geometry.size.height)
            }
        }
    }

    private var magnification: some Gesture {
        MagnifyGesture()
            .onChanged { value in
                zoom = min(max(pinchBase * value.magnification, 1), Self.maxZoom)
            }
            .onEnded { _ in
                pinchBase = zoom
                if zoom <= 1 { resetPan() }
            }
    }

    private var pan: some Gesture {
        DragGesture()
            .onChanged { value in
                // Panning an unzoomed photo would just slide it off the
                // screen with nothing to reveal.
                guard zoom > 1 else { return }
                offset = CGSize(
                    width: dragBase.width + value.translation.width,
                    height: dragBase.height + value.translation.height)
            }
            .onEnded { _ in dragBase = offset }
    }

    private func toggleZoom() {
        withAnimation(.snappy(duration: 0.2)) {
            if zoom > 1 {
                zoom = 1
                resetPan()
            } else {
                zoom = 2.5
            }
            pinchBase = zoom
        }
    }

    private func resetPan() {
        offset = .zero
        dragBase = .zero
    }
}

// MARK: - Video

/// Streams from the server, with the session token on every range request.
private struct VideoAttachmentPlayer: View {
    let attachment: AttachmentDTO

    @Environment(ChatSyncCoordinator.self) private var coordinator

    @State private var player: AVPlayer?

    var body: some View {
        Group {
            if let player {
                VideoPlayer(player: player)
                    .ignoresSafeArea()
            } else {
                ProgressView().tint(.white)
            }
        }
        .onAppear {
            guard player == nil,
                  let stream = coordinator.api.attachmentStreamURL(id: attachment.id)
            else { return }
            // AVURLAsset is the only way to attach an Authorization header:
            // AVPlayer(url:) sends none, and the attachment endpoint needs
            // one on every byte-range request it makes.
            let asset = AVURLAsset(
                url: stream.url,
                options: ["AVURLAssetHTTPHeaderFieldsKey": stream.headers])
            let item = AVPlayerItem(asset: asset)
            let player = AVPlayer(playerItem: item)
            player.play()
            self.player = player
        }
        .onDisappear {
            player?.pause()
            player = nil
        }
    }
}

#endif
