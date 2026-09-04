//
//  MacAttachmentViewer.swift
//  FamilyConnect
//
//  A message's photos and videos at full size, in their own window, one
//  at a time with arrows between them.
//
//  The Mac's Save and Share are the platform's own, not the phone's: an
//  NSSavePanel puts the file wherever the person wants it (which is what
//  "save" means on a Mac — there is no single gallery), and
//  NSSharingServicePicker is the same menu every other Mac app shows.
//  That is why this is not the iOS viewer with conditionals. Paging is
//  the Mac's idiom too: toolbar arrows and the arrow keys, not a swipe —
//  a Mac window has no pages to drag between.
//

#if os(macOS)

import AVKit
import AppKit
import SwiftUI

struct MacAttachmentViewer: View {
    /// Where the window opened: the message's media and the clicked one.
    /// Copied into state so the arrows move the view, not the window's
    /// key (FamilyConnectApp explains why the key holds the index).
    @State private var album: AttachmentAlbum

    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(AttachmentStore.self) private var store

    /// 1 = the whole picture fits the window. Above that it is scrollable.
    @State private var zoom: CGFloat = 1
    /// Where the last pinch ended: a MagnifyGesture's magnification is
    /// relative to the pinch's start, so used as the zoom itself every
    /// pinch snapped the picture back to 1x before growing again.
    @State private var pinchBase: CGFloat = 1
    @State private var busy = false
    /// Set when a save fails, so the refusal is visible rather than silent.
    @State private var saveFailure: String?

    init(album: AttachmentAlbum) {
        _album = State(initialValue: album)
    }

    private var attachment: AttachmentDTO { album.current }

    /// "2 of 5" while there is somewhere to page; the item's own name when
    /// there is not, as the window has always been titled.
    private var title: String {
        guard album.count > 1 else { return attachment.displayName }
        return String(
            localized: "\(album.index + 1) of \(album.count)",
            comment: "Which photo of an album is being looked at: the first number is its position, the second the album's size.")
    }

    var body: some View {
        content
            // Keyed by the item, so a page turn starts the picture's scroll
            // view fresh instead of inheriting the last one's offset.
            .id(attachment.id)
            .frame(minWidth: 480, minHeight: 360)
            .navigationTitle(title)
            .alert("Couldn't save that file",
                   isPresented: Binding(get: { saveFailure != nil },
                                        set: { if !$0 { saveFailure = nil } })) {
                Button("OK", role: .cancel) { saveFailure = nil }
            } message: {
                if let saveFailure { Text(saveFailure) }
            }
            .toolbar {
                ToolbarItem {
                    if busy { ProgressView().controlSize(.small) }
                }
                if album.count > 1 {
                    ToolbarItem {
                        Button {
                            turn(to: album.previous())
                        } label: {
                            Label("Previous", systemImage: "chevron.left")
                        }
                        .keyboardShortcut(.leftArrow, modifiers: [])
                        .disabled(!album.hasPrevious)
                        .help("Previous")
                    }
                    ToolbarItem {
                        Button {
                            turn(to: album.next())
                        } label: {
                            Label("Next", systemImage: "chevron.right")
                        }
                        .keyboardShortcut(.rightArrow, modifiers: [])
                        .disabled(!album.hasNext)
                        .help("Next")
                    }
                }
                ToolbarItem {
                    Button {
                        save()
                    } label: {
                        Label("Save…", systemImage: "square.and.arrow.down")
                    }
                    .help("Save a copy")
                }
                ToolbarItem {
                    Button {
                        share()
                    } label: {
                        Label("Share…", systemImage: "square.and.arrow.up")
                    }
                    .help("Share")
                }
            }
    }

    /// A page turn is also a zoom reset: the next picture is a different
    /// shape, and a 3x window into it would show a corner of nothing.
    private func turn(to next: AttachmentAlbum) {
        guard next != album else { return }
        zoom = 1
        album = next
    }

    @ViewBuilder
    private var content: some View {
        let _ = store.generation
        if attachment.isVideo {
            MacVideoPlayer(attachment: attachment)
        } else if let image = store.image(id: attachment.id, preview: false)
            ?? store.image(id: attachment.id, preview: true) {
            // The picture is sized to the WINDOW, times the zoom — which is
            // what makes 1x show all of it. Left to its own ideal size
            // inside a ScrollView (which is what `.aspectRatio(.fit)` plus
            // an infinite frame amounts to), a 4000px photo lays itself out
            // at 4000px and the window shows a corner of it.
            GeometryReader { geometry in
                ScrollView([.horizontal, .vertical]) {
                    image
                        .resizable()
                        .aspectRatio(contentMode: .fit)
                        .frame(
                            width: geometry.size.width * zoom,
                            height: geometry.size.height * zoom)
                        .gesture(
                            MagnifyGesture()
                                .onChanged { zoom = min(max(pinchBase * $0.magnification, 1), 6) }
                                .onEnded { _ in pinchBase = zoom })
                        .onTapGesture(count: 2) {
                            withAnimation(.easeOut(duration: 0.15)) {
                                zoom = zoom > 1 ? 1 : 2
                                pinchBase = zoom
                            }
                        }
                }
                // Scrollable only when there is something to scroll to.
                .scrollDisabled(zoom <= 1)
            }
            .background(Color.black.opacity(0.85))
        } else {
            ProgressView()
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    /// NSSavePanel, not a fixed folder: on a Mac "save" means the person
    /// picks where, and the sandbox grants access to exactly that place.
    /// Acts on the page that is up when the button is clicked, even if the
    /// person pages on while the bytes are fetched.
    private func save() {
        let attachment = attachment
        Task {
            busy = true
            defer { busy = false }
            guard let source = await coordinator.localFileURL(for: attachment) else {
                // The other way this button goes dead: no bytes to copy,
                // because the fetch failed or the server has nothing. Say
                // so rather than returning into silence.
                saveFailure = String(localized: "The file could not be downloaded.")
                return
            }
            let panel = NSSavePanel()
            panel.nameFieldStringValue = attachment.name
                ?? ChatSyncCoordinator.fallbackName(for: attachment)
            guard panel.runModal() == .OK, let destination = panel.url else { return }
            do {
                // Replace rather than fail: the panel already asked about
                // overwriting, and the person said yes. A missing file is
                // the normal case, so only a real removal failure counts.
                if FileManager.default.fileExists(atPath: destination.path) {
                    try FileManager.default.removeItem(at: destination)
                }
                try FileManager.default.copyItem(at: source, to: destination)
            } catch {
                // Never swallow this. Both calls used to be `try?`, so when
                // the sandbox refused the write the button did nothing and
                // said nothing — the failure looked exactly like a click
                // that had not registered.
                saveFailure = error.localizedDescription
            }
        }
    }

    private func share() {
        let attachment = attachment
        Task {
            busy = true
            defer { busy = false }
            guard let url = await coordinator.localFileURL(for: attachment),
                  let view = NSApp.keyWindow?.contentView
            else { return }
            NSSharingServicePicker(items: [url])
                .show(relativeTo: .zero, of: view, preferredEdge: .minY)
        }
    }
}

/// AVKit's own player view, wrapped for SwiftUI.
///
/// NOT SwiftUI's `VideoPlayer`, and that is a crash fix rather than a
/// preference: on macOS `VideoPlayer` is itself an `NSViewRepresentable`
/// living in `_AVKit_SwiftUI`, and building it aborted the app in the Swift
/// runtime — `getSuperclassMetadata` fatal-errored while instantiating its
/// generic metadata, before a single frame was decoded. The crash report
/// is unambiguous: `NSViewRepresentable._makeView` →
/// `PlatformViewRepresentableFeature.modifyViewOutputs` →
/// `swift_getAssociatedTypeWitness` → `_AVKit_SwiftUI` → `abort()`.
///
/// `AVPlayerView` is what a Mac video player is anyway: real transport
/// controls, Picture in Picture and full-screen, none of which SwiftUI's
/// wrapper exposes on this platform.
private struct MacPlayerSurface: NSViewRepresentable {
    let player: AVPlayer

    func makeNSView(context: Context) -> AVPlayerView {
        let view = AVPlayerView()
        view.player = player
        view.controlsStyle = .inline
        view.showsFullScreenToggleButton = true
        view.videoGravity = .resizeAspect
        return view
    }

    func updateNSView(_ view: AVPlayerView, context: Context) {
        if view.player !== player { view.player = player }
    }

    /// Drop the player when the view goes, or the stream keeps running
    /// behind a closed window.
    static func dismantleNSView(_ view: AVPlayerView, coordinator: ()) {
        view.player?.pause()
        view.player = nil
    }
}

/// Streams with the session token attached, like the iOS player. Keyed by
/// its attachment in the viewer, so paging away from a clip tears this
/// down (`onDisappear`) and the stream stops with the page.
private struct MacVideoPlayer: View {
    let attachment: AttachmentDTO

    @Environment(ChatSyncCoordinator.self) private var coordinator
    /// Same loader as the iOS page, for the same reason: the stream URL
    /// comes from the API actor, so building the player suspends. This
    /// view has only ONE reason to start (it appears), so it cannot
    /// double-start the way the iOS pager can — but the other half of the
    /// problem is entirely real here. The viewer is keyed by attachment,
    /// so a fast arrow-key page turn or closing the window tears this down
    /// while the load is suspended, and the load must be abandoned rather
    /// than hand a playing player to a window that has gone. Keeping the
    /// two platforms on one loader is also how this stays fixed: only the
    /// macOS build compiles this file.
    @State private var stream = AttachmentStreamPlayer()

    var body: some View {
        Group {
            if let player = stream.player {
                MacPlayerSurface(player: player)
            } else {
                ProgressView()
            }
        }
        .onAppear {
            stream.start(attachment: attachment.id, from: coordinator.api)
        }
        .onDisappear {
            stream.stop()
        }
    }
}

#endif
