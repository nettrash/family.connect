//
//  MacAttachmentViewer.swift
//  FamilyConnect
//
//  A photo or video at full size, in its own sheet.
//
//  The Mac's Save and Share are the platform's own, not the phone's: an
//  NSSavePanel puts the file wherever the person wants it (which is what
//  "save" means on a Mac — there is no single gallery), and
//  NSSharingServicePicker is the same menu every other Mac app shows.
//  That is why this is not the iOS viewer with conditionals.
//

#if os(macOS)

import AVKit
import AppKit
import SwiftUI

struct MacAttachmentViewer: View {
    let attachment: AttachmentDTO

    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(AttachmentStore.self) private var store

    /// 1 = the whole picture fits the window. Above that it is scrollable.
    @State private var zoom: CGFloat = 1
    @State private var busy = false

    var body: some View {
        content
            .frame(minWidth: 480, minHeight: 360)
            .navigationTitle(attachment.displayName)
            .toolbar {
                ToolbarItem {
                    if busy { ProgressView().controlSize(.small) }
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
                                .onChanged { zoom = min(max($0.magnification, 1), 6) })
                        .onTapGesture(count: 2) {
                            withAnimation(.easeOut(duration: 0.15)) {
                                zoom = zoom > 1 ? 1 : 2
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
    private func save() {
        Task {
            busy = true
            defer { busy = false }
            guard let source = await coordinator.localFileURL(for: attachment) else { return }
            let panel = NSSavePanel()
            panel.nameFieldStringValue = attachment.name
                ?? ChatSyncCoordinator.fallbackName(for: attachment)
            guard panel.runModal() == .OK, let destination = panel.url else { return }
            // Replace rather than fail: the panel already asked about
            // overwriting, and the person said yes.
            try? FileManager.default.removeItem(at: destination)
            try? FileManager.default.copyItem(at: source, to: destination)
        }
    }

    private func share() {
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

/// Streams with the session token attached, like the iOS player.
private struct MacVideoPlayer: View {
    let attachment: AttachmentDTO

    @Environment(ChatSyncCoordinator.self) private var coordinator
    @State private var player: AVPlayer?

    var body: some View {
        Group {
            if let player {
                VideoPlayer(player: player)
            } else {
                ProgressView()
            }
        }
        .onAppear {
            guard player == nil,
                  let stream = coordinator.api.attachmentStreamURL(id: attachment.id)
            else { return }
            // AVURLAsset again: AVPlayer(url:) sends no Authorization
            // header, and every byte-range request needs one.
            let asset = AVURLAsset(
                url: stream.url,
                options: ["AVURLAssetHTTPHeaderFieldsKey": stream.headers])
            let created = AVPlayer(playerItem: AVPlayerItem(asset: asset))
            created.play()
            player = created
        }
        .onDisappear {
            player?.pause()
            player = nil
        }
    }
}

#endif
