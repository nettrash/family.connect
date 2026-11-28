#if os(macOS)
import AppKit
import ImageIO
import SwiftUI
import UniformTypeIdentifiers

/// The viewer owns clipboard state separately from its normal media loading.
struct MacViewerCopyImage: ViewModifier {
    let attachment: AttachmentDTO

    @Environment(AttachmentStore.self) private var store
    @State private var copying = false
    @State private var failedAttachmentID: Int64?

    private var hasFullImage: Bool {
        // Watch the full file only. The viewer's render lookup can fetch on a miss.
        _ = store.version(id: attachment.id)
        return FileManager.default.fileExists(atPath: store.originalURL(id: attachment.id).path)
    }

    func body(content: Content) -> some View {
        if attachment.kind == AttachmentDTO.Kind.photo && !attachment.sticker {
            let available = hasFullImage
            content
                .toolbar {
                    ToolbarItem {
                        Button(action: copyImage) {
                            Label("Copy Image", systemImage: "document.on.document")
                        }
                        .help("Copy Image")
                        .disabled(!available || copying)
                    }
                }
                .safeAreaInset(edge: .bottom, spacing: 0) {
                    if !available || failedAttachmentID == attachment.id {
                        Text("Full image unavailable")
                            .font(.callout)
                            .foregroundStyle(.secondary)
                            .padding(8)
                            .frame(maxWidth: .infinity)
                            .background(.bar)
                    }
                }
                .background(MacViewerCopyShortcut(copyImage: copyImage))
        } else {
            content
        }
    }

    private func copyImage() {
        guard !copying else { return }
        let id = attachment.id
        copying = true
        Task {
            let copied = await MacAttachmentImageCopy.copy(id: id, store: store, to: .general)
            failedAttachmentID = copied ? nil : id
            copying = false
        }
    }
}

enum MacAttachmentImageCopy {
    static func copy(id: Int64, store: AttachmentStore, to pasteboard: NSPasteboard) async -> Bool {
        guard let bytes = await store.cachedBytes(id: id),
              let png = await encodeStillPNG(bytes)
        else { return false }

        // Give the pasteboard data now. There is no provider to call back into
        // the viewer when a different app asks for the image later.
        let item = NSPasteboardItem()
        guard item.setData(png, forType: .png) else { return false }
        pasteboard.clearContents()
        return pasteboard.writeObjects([item])
    }

    @concurrent
    private static func encodeStillPNG(_ bytes: Data) async -> Data? {
        guard let source = CGImageSourceCreateWithData(bytes as CFData, nil),
              CGImageSourceGetCount(source) == 1,
              let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = properties[kCGImagePropertyPixelWidth] as? Int,
              let height = properties[kCGImagePropertyPixelHeight] as? Int,
              let image = CGImageSourceCreateThumbnailAtIndex(source, 0, [
                kCGImageSourceCreateThumbnailFromImageAlways: true,
                kCGImageSourceCreateThumbnailWithTransform: true,
                kCGImageSourceThumbnailMaxPixelSize: max(width, height),
              ] as CFDictionary)
        else { return nil }

        let png = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(png, UTType.png.identifier as CFString, 1, nil)
        else { return nil }
        CGImageDestinationAddImage(destination, image, nil)
        return CGImageDestinationFinalize(destination) ? png as Data : nil
    }
}

/// A local monitor keeps Cmd+C in the key attachment window even when a
/// toolbar button or the scroll view has focus. Text responders keep Copy.
private struct MacViewerCopyShortcut: NSViewRepresentable {
    let copyImage: () -> Void

    func makeNSView(context: Context) -> Observer { Observer() }

    func updateNSView(_ view: Observer, context: Context) {
        view.copyImage = copyImage
    }

    static func dismantleNSView(_ view: Observer, coordinator: ()) {
        view.stopObserving()
    }

    final class Observer: NSView {
        var copyImage: () -> Void = {}
        private var monitor: Any?

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            stopObserving()
            guard window != nil else { return }
            monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
                guard let self, let window, window.isKeyWindow, event.window === window,
                      !(window.firstResponder is NSTextView),
                      event.modifierFlags.intersection([.shift, .control, .option, .command]) == .command,
                      event.charactersIgnoringModifiers?.lowercased() == "c"
                else { return event }
                if !event.isARepeat { copyImage() }
                return nil
            }
        }

        func stopObserving() {
            if let monitor { NSEvent.removeMonitor(monitor) }
            monitor = nil
        }
    }
}
#endif
