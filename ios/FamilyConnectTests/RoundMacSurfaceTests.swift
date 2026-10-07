//
//  RoundMacSurfaceTests.swift
//  FamilyConnectTests
//
//  The Mac's video-message circle (#79, docs/audio-video-messages-2026-10-04.md,
//  S5.2, S5.3, S8.3): a circle the Mac recorded did not look like a circle on
//  the Mac, while the same message was round on the web and on Android.
//
//  Two halves, measured separately:
//
//    - The PICTURE at rest — the poster in a circle, in the Mac's own row —
//      was always round (the row tests below pass on the old code too). The
//      flag survives the send, the ack and a page (RoundCacheRepairTests).
//    - The VIDEO — the clip in REVIEW, and a circle playing in place — goes
//      through `RoundPlayerLayerView`, whose own clip did not survive AppKit:
//      `masksToBounds` set before the layer was handed over was reset, so
//      the player layer had a corner radius and no clip, which is a SQUARE
//      picture with SwiftUI's clip as its only hope. The camera preview,
//      layer-backed, kept its clip; the iPhone's surface clips itself. The
//      surface tests here fail on the old code and pass on the fix.
//
//  A pixel test where pixels can be had: a red layer stands in for the
//  video's frames (an AVPlayerLayer with no item draws nothing), and the
//  corners of the circle's square must not be red.
//

#if os(macOS)

import AppKit
import AVFoundation
import Observation
import SwiftData
import SwiftUI
import Testing
@testable import FamilyConnect

@MainActor
@Observable
final class RoundSurfaceSize {
    var edge: CGFloat = 240
}

@MainActor
@Suite("Video message on the Mac")
struct RoundMacSurfaceTests {

    // MARK: - Harness

    /// A borderless window that is never ordered in, holding `root`.
    @MainActor
    final class Hosted<Root: View> {
        let window: NSWindow
        let hosting: NSHostingView<Root>

        init(_ root: Root, size: NSSize) {
            hosting = NSHostingView(rootView: root)
            window = NSWindow(
                contentRect: NSRect(origin: .zero, size: size),
                styleMask: [.borderless], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = hosting
            hosting.frame = NSRect(origin: .zero, size: size)
            settle()
        }

        func settle() {
            for _ in 0..<3 {
                hosting.layoutSubtreeIfNeeded()
                window.displayIfNeeded()
                RunLoop.main.run(until: Date().addingTimeInterval(0.02))
            }
        }

        func surfaces() -> [RoundPlayerLayerView] { Self.surfaces(in: hosting) }

        static func surfaces(in view: NSView) -> [RoundPlayerLayerView] {
            var found: [RoundPlayerLayerView] = []
            if let surface = view as? RoundPlayerLayerView { found.append(surface) }
            for sub in view.subviews { found += surfaces(in: sub) }
            return found
        }

        /// The window's picture, as AppKit composes it.
        func snapshot() throws -> NSBitmapImageRep {
            let rep = try #require(hosting.bitmapImageRepForCachingDisplay(in: hosting.bounds))
            hosting.cacheDisplay(in: hosting.bounds, to: rep)
            return rep
        }

        func close() { window.close() }
    }

    /// The video's frames, standing in: red, filling the player layer.
    private func paintFrames(_ surface: RoundPlayerLayerView) {
        let frames = CALayer()
        frames.backgroundColor = NSColor.red.cgColor
        frames.frame = surface.playerLayer.bounds
        frames.autoresizingMask = [.layerWidthSizable, .layerHeightSizable]
        surface.playerLayer.addSublayer(frames)
    }

    /// Red, at `alpha` or more — a pending row is drawn at 0.55.
    private func isRed(_ color: NSColor?, alpha: CGFloat = 0.8) -> Bool {
        guard let color = color?.usingColorSpace(.deviceRGB) else { return false }
        return color.alphaComponent > alpha && color.redComponent > 0.75
            && color.greenComponent < 0.35 && color.blueComponent < 0.35
    }

    /// `rect` (points, top-left origin) is a circle of red: red at its centre
    /// and across its middle, and NOT red just inside the corners of its
    /// square — where a square picture would be.
    private func expectRoundRed(
        _ rep: NSBitmapImageRep, in rect: CGRect, hostWidth: CGFloat,
        sourceLocation: SourceLocation = #_sourceLocation
    ) {
        let scale = CGFloat(rep.pixelsWide) / hostWidth
        func at(_ x: CGFloat, _ y: CGFloat) -> NSColor? {
            rep.colorAt(x: Int((x * scale).rounded()), y: Int((y * scale).rounded()))
        }
        // Above the centre, where the play disc is not.
        #expect(isRed(at(rect.midX, rect.minY + rect.height * 0.3)), "the frames were not drawn at all",
                sourceLocation: sourceLocation)
        #expect(isRed(at(rect.minX + rect.width * 0.08, rect.midY)), "the circle is not full width",
                sourceLocation: sourceLocation)
        let inset = rect.width * 0.06
        for (x, y) in [
            (rect.minX + inset, rect.minY + inset), (rect.maxX - inset, rect.minY + inset),
            (rect.minX + inset, rect.maxY - inset), (rect.maxX - inset, rect.maxY - inset),
        ] {
            #expect(!isRed(at(x, y)),
                    "a corner of the circle's square shows the video (at \(Int(x)),\(Int(y))) — it is a square",
                    sourceLocation: sourceLocation)
        }
    }

    // MARK: - The surface (fails on the old code)

    @Test("the playing surface is a circle by itself, with no SwiftUI clip around it")
    func theSurfaceCutsItself() throws {
        let hosted = Hosted(
            RoundVideoSurface(player: AVPlayer())
                .frame(width: 240, height: 240)
                .frame(width: 300, height: 300)
                .background(Color.white),
            size: NSSize(width: 300, height: 300))
        defer { hosted.close() }
        let surface = try #require(hosted.surfaces().first)
        #expect(surface.bounds.size == CGSize(width: 240, height: 240))
        #expect(surface.layer === surface.playerLayer, "the player layer IS the view's layer")
        #expect(surface.playerLayer.masksToBounds,
                "the player layer has a corner radius and no clip: its video is a square")
        #expect(abs(surface.playerLayer.cornerRadius - 120) < 0.5)

        paintFrames(surface)
        hosted.settle()
        expectRoundRed(try hosted.snapshot(), in: CGRect(x: 30, y: 30, width: 240, height: 240), hostWidth: 300)
    }

    @Test("it stays a circle when its size changes — the recorder's circle follows the window")
    func itFollowsItsSize() throws {
        let size = RoundSurfaceSize()
        let player = AVPlayer()
        let hosted = Hosted(
            SizedSurface(size: size, player: player)
                .frame(width: 400, height: 400)
                .background(Color.white),
            size: NSSize(width: 400, height: 400))
        defer { hosted.close() }
        size.edge = 320
        hosted.settle()
        let surface = try #require(hosted.surfaces().first)
        #expect(surface.bounds.size == CGSize(width: 320, height: 320))
        #expect(surface.playerLayer.masksToBounds)
        #expect(abs(surface.playerLayer.cornerRadius - 160) < 0.5,
                "the radius stayed at the first size's: \(surface.playerLayer.cornerRadius)")
        paintFrames(surface)
        hosted.settle()
        expectRoundRed(try hosted.snapshot(), in: CGRect(x: 40, y: 40, width: 320, height: 320), hostWidth: 400)
    }

    @Test("REVIEW: the clip as it will be sent plays in a circle that cuts itself")
    func reviewIsRound() throws {
        let engine = FakeCaptureEngine()
        let session = VideoMessageSession(
            request: VideoMessageSession.Request(chatID: 42), engine: engine, firstTime: false)
        var now: UInt64 = 0
        session.runsTicker = false
        session.clock = { now }
        session.cameraPermission = { .granted }
        session.microphonePermission = { .granted }
        session.announce = { _ in }
        session.voiceOverRunning = { false }
        session.speak = { _ in }
        session.haptic = {}
        session.holdOrientation = { _ in }
        session.makeClipURL = {
            FileManager.default.temporaryDirectory
                .appendingPathComponent("fc-test-round-\(UUID().uuidString).mp4")
        }
        session.playbackSession = PlaybackSessionControl(begin: {}, end: {})
        let arbiter = VoiceRecordingArbiter()
        arbiter.keepAwake = { _ in }
        session.arbiter = arbiter
        session.nowPlaying = NowPlaying()
        session.markTaught = {}
        session.onClose = {}
        session.onProceed = { _ in }
        session.start()
        engine.say(.firstFrame)
        now = 1_000
        session.record()
        now = 6_000
        session.stop()
        try engine.finish(ms: 5_000)
        #expect(session.state.phase == .review(durationMS: 5_000))
        defer { session.delete() }

        let size = NSSize(width: 800, height: 700)
        let hosted = Hosted(
            VideoMessageRecorderView(session: session, paneFrame: nil)
                .frame(width: size.width, height: size.height),
            size: size)
        defer { hosted.close() }
        let surface = try #require(hosted.surfaces().first, "REVIEW draws no clip")
        let edge = surface.bounds.width
        #expect(edge >= 160 && surface.bounds.height == edge)
        #expect(surface.playerLayer.masksToBounds,
                "REVIEW's clip has a corner radius and no clip: the recorded circle plays square")
        #expect(abs(surface.playerLayer.cornerRadius - edge / 2) < 0.5)
    }

    // MARK: - The picture at rest (round before and after)

    @MainActor
    final class World {
        let container: ModelContainer
        let coordinator: ChatSyncCoordinator
        let session: AppSession
        let store: AttachmentStore

        init() throws {
            container = try ModelContainer(
                for: ChatEntity.self, MessageEntity.self, MemberEntity.self, NoteEntity.self,
                PendingMediaItemEntity.self,
                configurations: ModelConfiguration(isStoredInMemoryOnly: true))
            let api = APIClient(serverURL: URL(string: "https://circles.invalid"))
            coordinator = ChatSyncCoordinator(modelContainer: container, api: api)
            coordinator.currentUserIDOverride = 7
            session = AppSession(api: api, defaultServerURL: { nil })
            let directory = URL(fileURLWithPath: NSTemporaryDirectory())
                .appendingPathComponent("round-mac-row-\(UUID().uuidString)")
            store = AttachmentStore(api: api, directory: directory)
            let context = try #require(CGContext(
                data: nil, width: 64, height: 64, bitsPerComponent: 8, bytesPerRow: 0,
                space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue))
            context.setFillColor(red: 1, green: 0, blue: 0, alpha: 1)
            context.fill(CGRect(x: 0, y: 0, width: 64, height: 64))
            let poster = PlatformImage.jpegData(from: try #require(context.makeImage()), quality: 1) ?? Data()
            store.seed(poster, id: 91, preview: true)
            store.seed(poster, id: -91, preview: true)
        }
    }

    private func video(id: Int64 = 91, round: Bool) -> AttachmentDTO {
        AttachmentDTO(
            id: id, kind: "video", mime: "video/mp4", size: 1_649_700, width: 480, height: 480,
            durationMS: 23_400, hasPreview: true, name: nil, latitude: nil, longitude: nil,
            accuracyM: nil, isRound: round)
    }

    /// The Mac's own row, drawn by SwiftUI alone (the poster is a picture).
    private func renderRow(
        _ world: World, attachment: AttachmentDTO, mine: Bool,
        serverID: Int64? = 1, state: MessageStatus = .sent
    ) throws -> NSBitmapImageRep {
        let message = MessageSnapshot(
            localID: "r:1", serverID: serverID, chatID: 42, senderID: mine ? 7 : 9, body: "",
            createdAt: Date(timeIntervalSince1970: 0), state: state,
            attachment: attachment, attachments: [attachment])
        let row = MacMessageRow(message: message, senderName: mine ? nil : "Ana", isMine: mine)
            .frame(width: 600)
            .tint(.blue)
            .environment(LinkPreviewLoader())
            .environment(world.store)
            .environment(world.coordinator)
            .environment(world.session)
        let renderer = ImageRenderer(content: row)
        renderer.scale = 1
        return NSBitmapImageRep(cgImage: try #require(renderer.cgImage, "the row did not render"))
    }

    /// The red's bounding box — the poster's extent.
    private func redExtent(_ rep: NSBitmapImageRep, alpha: CGFloat = 0.8) -> CGRect? {
        var minX = Int.max, minY = Int.max, maxX = -1, maxY = -1
        for y in 0..<rep.pixelsHigh {
            for x in 0..<rep.pixelsWide where isRed(rep.colorAt(x: x, y: y), alpha: alpha) {
                minX = min(minX, x); maxX = max(maxX, x)
                minY = min(minY, y); maxY = max(maxY, y)
            }
        }
        guard maxX >= 0 else { return nil }
        return CGRect(x: minX, y: minY, width: maxX - minX + 1, height: maxY - minY + 1)
    }

    @Test("a video message in the Mac's row is a circle 240 across — the sender's own, and another's")
    func theRowIsACircle() throws {
        let world = try World()
        for mine in [true, false] {
            let rep = try renderRow(world, attachment: video(round: true), mine: mine)
            let red = try #require(redExtent(rep), "the poster was not drawn")
            #expect(abs(red.width - 240) <= 3 && abs(red.height - 240) <= 3, "\(red)")
            expectRoundRed(rep, in: red, hostWidth: CGFloat(rep.pixelsWide))
        }
    }

    @Test("and still a circle while it is going up, from its own poster (S5.6)")
    func theSendingRowIsACircle() throws {
        let world = try World()
        let rep = try renderRow(
            world, attachment: video(id: -91, round: true), mine: true, serverID: nil, state: .pending)
        // Dimmed until the server has it.
        let red = try #require(redExtent(rep, alpha: 0.3), "the pending poster was not drawn")
        #expect(abs(red.width - 240) <= 3 && abs(red.height - 240) <= 3, "\(red)")
        let inset = Int(red.width * 0.06)
        #expect(!isRed(rep.colorAt(x: Int(red.minX) + inset, y: Int(red.minY) + inset), alpha: 0.3),
                "the sender's own circle is drawn square while it goes up")
    }

    @Test("the control: the same video without the flag is the square tile")
    func theControlIsSquare() throws {
        let world = try World()
        let rep = try renderRow(world, attachment: video(round: false), mine: false)
        let red = try #require(redExtent(rep), "the poster was not drawn")
        // Where a circle's square is empty (expectRoundRed's inset), a plain
        // video's tile is still picture.
        let inset = Int(red.width * 0.06)
        let corner = rep.colorAt(x: Int(red.minX) + inset, y: Int(red.minY) + inset)
        #expect(isRed(corner), "a plain video's tile has corners: \(red)")
    }
}

/// A surface whose size a test changes after it is first drawn.
private struct SizedSurface: View {
    let size: RoundSurfaceSize
    let player: AVPlayer

    var body: some View {
        RoundVideoSurface(player: player)
            .frame(width: size.edge, height: size.edge)
    }
}

#endif
