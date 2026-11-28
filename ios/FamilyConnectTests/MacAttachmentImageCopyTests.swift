#if os(macOS)
import AppKit
import ImageIO
import Testing
import UniformTypeIdentifiers
@testable import FamilyConnect

@MainActor
struct MacAttachmentImageCopyTests {
    @Test func copiesSelectedFullImageWithOrientationAndAlphaWithoutRequests() async throws {
        let harness = Harness()
        defer { harness.cleanUp() }

        let firstID: Int64 = 11
        let selectedID: Int64 = 12
        harness.store.seed(try image(width: 35, height: 23), id: firstID, preview: false)
        harness.store.seed(try image(width: 12, height: 7), id: selectedID, preview: true)
        harness.store.seed(try image(width: 1_601, height: 43, orientation: 6), id: selectedID, preview: false)
        harness.store.purgeMemory()

        #expect(await MacAttachmentImageCopy.copy(id: selectedID, store: harness.store, to: harness.pasteboard))

        let png = try #require(harness.pasteboard.data(forType: .png))
        let bitmap = try #require(NSBitmapImageRep(data: png))
        #expect(bitmap.pixelsWide == 43)
        #expect(bitmap.pixelsHigh == 1_601)
        #expect(bitmap.hasAlpha)
        let alpha = try [100, 1_500].map { y in
            try #require(bitmap.colorAt(x: 21, y: y)).alphaComponent
        }
        #expect(alpha.contains { $0 == 0 })
        #expect(alpha.contains { abs($0 - 0.5) < 0.02 })
        #expect(NSImage(pasteboard: harness.pasteboard) != nil)
        #expect(StubURLProtocol.requests(host: harness.host).isEmpty)
    }

    @Test func previewOnlyKeepsClipboardAndMakesNoRequests() async throws {
        let harness = Harness()
        defer { harness.cleanUp() }
        harness.store.seed(try image(width: 12, height: 7), id: 21, preview: true)

        await harness.expectUnavailable(id: 21)
    }

    @Test func evictedFullImageKeepsClipboardEvenWithHotDecodedImage() async throws {
        let harness = Harness()
        defer { harness.cleanUp() }
        harness.store.seed(try image(width: 35, height: 23), id: 31, preview: false)
        try FileManager.default.removeItem(at: harness.store.originalURL(id: 31))
        #expect(harness.store.image(id: 31, preview: false) != nil)

        await harness.expectUnavailable(id: 31)
    }

    @Test func unreadableFullImageKeepsClipboardAndMakesNoRequests() async throws {
        let harness = Harness()
        defer { harness.cleanUp() }
        try Data("invalid image bytes".utf8).write(to: harness.store.originalURL(id: 41))

        await harness.expectUnavailable(id: 41)
    }

    private func image(width: Int, height: Int, orientation: Int = 1) throws -> Data {
        let context = try #require(CGContext(
            data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.clear(CGRect(x: 0, y: 0, width: width, height: height))
        context.setFillColor(red: 1, green: 0, blue: 0, alpha: 0.5)
        context.fill(CGRect(x: 0, y: 0, width: width / 2, height: height))
        let bitmap = try #require(context.makeImage())
        let bytes = NSMutableData()
        let destination = try #require(CGImageDestinationCreateWithData(bytes, UTType.png.identifier as CFString, 1, nil))
        CGImageDestinationAddImage(destination, bitmap, [kCGImagePropertyOrientation: orientation] as CFDictionary)
        try #require(CGImageDestinationFinalize(destination))
        return bytes as Data
    }

    @MainActor
    private struct Harness {
        let host = "mac-image-copy-\(UUID().uuidString.lowercased()).test"
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let pasteboard = NSPasteboard(name: .init("FamilyConnect.MacAttachmentImageCopyTests.\(UUID())"))
        let store: AttachmentStore

        init() {
            StubURLProtocol.register(host: host) { _ in .empty(404) }
            let api = APIClient(serverURL: URL(string: "https://\(host)"), session: StubURLProtocol.makeSession())
            store = AttachmentStore(api: api, directory: directory)
        }

        func cleanUp() {
            StubURLProtocol.unregister(host: host)
            pasteboard.releaseGlobally()
            try? FileManager.default.removeItem(at: directory)
        }

        func expectUnavailable(id: Int64) async {
            pasteboard.clearContents()
            #expect(pasteboard.setString("previous clipboard contents", forType: .string))
            let changeCount = pasteboard.changeCount

            #expect(await !MacAttachmentImageCopy.copy(id: id, store: store, to: pasteboard))

            #expect(pasteboard.changeCount == changeCount)
            #expect(pasteboard.string(forType: .string) == "previous clipboard contents")
            #expect(pasteboard.data(forType: .png) == nil)
            #expect(StubURLProtocol.requests(host: host).isEmpty)
        }
    }
}
#endif
