//
//  AttachmentViewer.swift
//  FamilyConnect
//
//  A message's photos and videos, full screen, one page each.
//
//  The two kinds of page work differently on purpose. A photo is FETCHED —
//  the whole JPEG lands in AttachmentStore's disk cache and is then
//  zoomable offline, which is what you want for something you will open
//  again. A video is STREAMED: AVPlayer pulls ranges from the server as it
//  plays, so a 90 MB clip starts in a second and never occupies 90 MB
//  anywhere. That is also why the store has no video path (AttachmentStore's
//  header).
//
//  Paging is a horizontal ScrollView with .paging, NOT a TabView(.page):
//  a zoomed photo has to be pannable, and a pan is a horizontal drag — the
//  same drag that turns a page. The ScrollView can be told to stand down
//  while the photo is zoomed (`scrollDisabled`); a page-style TabView
//  cannot, and turned the page on every pan.
//
//  Android counterpart: ui/chat/AttachmentViewer.kt
//

// iOS only — the Mac has its own views (MacViews/).
#if os(iOS)

import AVKit
import SwiftUI

struct AttachmentViewer: View {
    /// What to page through, and where to start. A lone photo is an album
    /// of one: no label, nowhere to page.
    let album: AttachmentAlbum
    /// Fetches the bytes and opens the share sheet for the page that is
    /// up. Owned by the thread, which already has the coordinator and the
    /// sheet state.
    var onShare: (AttachmentDTO) -> Void = { _ in }

    @Environment(\.dismiss) private var dismiss

    /// The page the scroll view has settled on, by attachment id — what
    /// the label, Share and the video players read.
    @State private var position: Int64?
    /// True while the page that is up is zoomed in. The pager stands down
    /// so a pan pans; the moment the photo is back at 1x it pages again.
    @State private var isZoomed = false

    init(album: AttachmentAlbum, onShare: @escaping (AttachmentDTO) -> Void = { _ in }) {
        self.album = album
        self.onShare = onShare
        _position = State(initialValue: album.current.id)
    }

    private var current: AttachmentDTO {
        album.items.first { $0.id == position } ?? album.current
    }

    private var pageNumber: Int {
        (album.items.firstIndex { $0.id == position } ?? album.index) + 1
    }

    var body: some View {
        ZStack {
            // Black, in both appearances: a photo full-screen wants no
            // surrounding colour competing with it.
            Color.black.ignoresSafeArea()

            ScrollView(.horizontal) {
                LazyHStack(spacing: 0) {
                    ForEach(album.items) { item in
                        page(item)
                            .containerRelativeFrame(.horizontal)
                            .id(item.id)
                    }
                }
                .scrollTargetLayout()
            }
            .scrollTargetBehavior(.paging)
            .scrollPosition(id: $position)
            .scrollIndicators(.hidden)
            .scrollDisabled(isZoomed)
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
                    // Drawn size unchanged; only the tappable circle grows
                    // to ~44pt so a near-miss closes instead of panning.
                    .contentShape(Circle().inset(by: -6))
            }
            .padding(16)
            .accessibilityLabel("Close")
        }
        .overlay(alignment: .top) {
            // Only when there is somewhere to page: a lone photo needs no
            // "1 of 1" over it.
            if album.count > 1 {
                Text(
                    "\(pageNumber) of \(album.count)",
                    comment: "Which photo of an album is being looked at: the first number is its position, the second the album's size.")
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(.white)
                    .monospacedDigit()
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    .background(.black.opacity(0.45), in: Capsule())
                    .padding(16)
            }
        }
        .overlay(alignment: .topTrailing) {
            // Share is also how this gets saved: the sheet's own "Save
            // Image" / "Save Video" put it in the library.
            Button {
                let item = current
                dismiss()
                onShare(item)
            } label: {
                Image(systemName: "square.and.arrow.up")
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundStyle(.white)
                    .padding(10)
                    .background(.black.opacity(0.45), in: Circle())
                    .contentShape(Circle().inset(by: -6))
            }
            .padding(16)
            .accessibilityLabel("Share")
        }
        .statusBarHidden()
    }

    @ViewBuilder
    private func page(_ item: AttachmentDTO) -> some View {
        if item.isVideo {
            // Plays only while it is the page that is up: a lazy stack
            // materialises the neighbours early, and a clip that started
            // playing off-screen would be heard before it was seen.
            VideoAttachmentPlayer(attachment: item, isCurrent: position == item.id)
        } else {
            ZoomablePhoto(attachment: item, isZoomed: $isZoomed)
        }
    }
}

// MARK: - Photo

/// Pinch and double-tap to zoom, drag to pan while zoomed.
private struct ZoomablePhoto: View {
    let attachment: AttachmentDTO
    /// Reported up so the pager can stand down while this is zoomed.
    /// Written only on this photo's own transitions, so several pages
    /// sharing the binding never fight over it: a page cannot leave while
    /// it is zoomed, and a page that is not zoomed writes nothing.
    @Binding var isZoomed: Bool

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
        .onChange(of: zoom > 1) { _, zoomed in isZoomed = zoomed }
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
    /// Whether this is the page that is up. The stream starts when it
    /// becomes so and is released when it stops being so — or when the
    /// page leaves the hierarchy, whichever comes first.
    let isCurrent: Bool

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
        .onAppear { if isCurrent { start() } }
        .onChange(of: isCurrent) { _, current in
            if current { start() } else { stop() }
        }
        .onDisappear { stop() }
    }

    private func start() {
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

    private func stop() {
        player?.pause()
        player = nil
    }
}

#endif
