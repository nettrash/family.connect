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
//  The other half of that bargain is the photo's own gestures: a child's
//  gesture wins over an ancestor's, so a DragGesture that is merely
//  attached to the photo eats the swipe the pager needs — and it used to
//  be attached at 1x too, where it had nothing to pan. The result was a
//  page that turned only from the letterbox around the picture. The pan
//  is now attached ONLY while zoomed (a GestureMask), so at 1x the
//  ScrollView is the only thing that wants a one-finger drag — see
//  ZoomablePhoto. The pinch stays an ordinary (exclusive) gesture: two
//  fingers never competed with the swipe, and keeping it exclusive is
//  what stops the pager from turning a page under a pinch.
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
            // Safety net: should a page ever leave while zoomed (it writes
            // `isZoomed` only on its own transitions, and a page that is
            // gone writes nothing), the pager must not stay disabled
            // forever — the page that arrives is at 1x.
            .onChange(of: position) { _, _ in isZoomed = false }
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
            // Esc on a hardware keyboard, the way every other cover closes.
            .keyboardShortcut(.cancelAction)
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
                    .accessibilityIdentifier("attachment-viewer-page")
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
                    // The whole page, letterbox included, pinches and
                    // double-taps: a plain frame is not hit-testable
                    // outside its content.
                    .contentShape(Rectangle())
                    // The pinch is exclusive on purpose (see the header).
                    // The pan exists only while zoomed — `.subviews` masks
                    // it out entirely at 1x, so the ScrollView alone owns
                    // a one-finger drag then; zoomed, scrollDisabled has
                    // the ScrollView standing down and the pan takes over.
                    // The trade-off: a drag that BEGAN as the pinch (fingers
                    // never lifted) is not picked up by the pan — a fresh
                    // touch is; the alternative was the pan eating every
                    // swipe at 1x, which is the bug this replaced.
                    .gesture(magnification(in: geometry.size))
                    .simultaneousGesture(pan(in: geometry.size), including: zoom > 1 ? .all : .subviews)
                    // The tap's own point, so a double-tap zooms INTO the
                    // face that was tapped rather than into the centre of
                    // the frame.
                    .onTapGesture(count: 2) { point in toggleZoom(at: point, in: geometry.size) }
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
        // A page that leaves comes back at 1x, like a fresh one — and
        // writing zoom here releases `isZoomed` through the onChange above.
        .onDisappear {
            zoom = 1
            pinchBase = 1
            resetPan()
        }
    }

    private func magnification(in container: CGSize) -> some Gesture {
        MagnifyGesture()
            .onChanged { value in
                zoom = min(max(pinchBase * value.magnification, 1), Self.maxZoom)
                // Zooming OUT shrinks the room to pan; a pan that was legal
                // at 4x would leave the picture off-centre at 2x.
                offset = clamped(offset, zoom: zoom, in: container)
                dragBase = offset
            }
            .onEnded { _ in
                pinchBase = zoom
                if zoom <= 1 { resetPan() }
            }
    }

    private func pan(in container: CGSize) -> some Gesture {
        DragGesture()
            .onChanged { value in
                // Masked out at 1x (see the body), so this only ever runs
                // zoomed; the guard is belt and braces for a drag that
                // began zoomed and outlived a double-tap back to 1x.
                guard zoom > 1 else { return }
                offset = clamped(
                    CGSize(
                        width: dragBase.width + value.translation.width,
                        height: dragBase.height + value.translation.height),
                    zoom: zoom, in: container)
            }
            .onEnded { _ in dragBase = offset }
    }

    /// The pan held to the picture: a zoomed photo can be dragged until
    /// its edge meets the frame's, and no further — it used to be
    /// draggable clean off a 13-inch screen, leaving black and no way
    /// back but a double-tap. The picture's fitted size comes from the
    /// attachment's own aspect ratio (metadata), which is what the
    /// `.fit` frame above draws it at.
    private func clamped(_ proposed: CGSize, zoom: CGFloat, in container: CGSize) -> CGSize {
        let ratio = attachment.aspectRatio
        let fitted = container.width / max(container.height, 1) > ratio
            ? CGSize(width: container.height * ratio, height: container.height)
            : CGSize(width: container.width, height: container.width / ratio)
        let maxX = max(0, (fitted.width * zoom - container.width) / 2)
        let maxY = max(0, (fitted.height * zoom - container.height) / 2)
        return CGSize(
            width: min(max(proposed.width, -maxX), maxX),
            height: min(max(proposed.height, -maxY), maxY))
    }

    private func toggleZoom(at point: CGPoint, in container: CGSize) {
        withAnimation(.snappy(duration: 0.2)) {
            if zoom > 1 {
                zoom = 1
                resetPan()
            } else {
                zoom = 2.5
                // Keep what was under the finger under the finger: the
                // scale is about the frame's centre, so the tapped point
                // moves by (point − centre) × (zoom − 1) and the offset
                // walks it back — then clamped, so a tap near an edge
                // lands on the edge rather than past it.
                let centre = CGPoint(x: container.width / 2, y: container.height / 2)
                offset = clamped(
                    CGSize(
                        width: (point.x - centre.x) * (1 - zoom),
                        height: (point.y - centre.y) * (1 - zoom)),
                    zoom: zoom, in: container)
                dragBase = offset
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

    /// The player AND the fetch of the URL it is built from, in one
    /// object. It cannot be a bare `@State var player` any more: the
    /// stream URL lives behind the API actor, so getting it suspends, and
    /// this view asks for a start from TWO places — `onAppear` and the
    /// `isCurrent` change. Two starts across one suspension used to mean
    /// two AVPlayers, the first of them leaked and audible. The loader
    /// holds the slot across the await and cancels it on the way out; see
    /// Core/AttachmentStreamPlayer.swift for why the guard is a task
    /// handle rather than a flag.
    @State private var stream = AttachmentStreamPlayer()

    var body: some View {
        Group {
            // Nil for one runloop turn longer than before — the actor hop
            // that fetches the URL. Nothing downstream needed `player`
            // synchronously: this branch was already the state the page
            // rendered in before `onAppear` had run at all, so the
            // spinner it shows is the one that was always there.
            if let player = stream.player {
                VideoPlayer(player: player)
                    .ignoresSafeArea()
            } else {
                ProgressView().tint(.white)
            }
        }
        .onAppear { if isCurrent { start() } }
        .onChange(of: isCurrent) { _, current in
            if current { start() } else { stream.stop() }
        }
        // Paging away or dismissing the viewer mid-load cancels it, so a
        // player is never handed to a page that has gone.
        .onDisappear { stream.stop() }
    }

    private func start() {
        stream.start(attachment: attachment.id, from: coordinator.api)
    }
}

#endif
