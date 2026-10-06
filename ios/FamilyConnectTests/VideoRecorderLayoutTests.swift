//
//  VideoRecorderLayoutTests.swift
//  FamilyConnectTests
//
//  #79, decision 41 (docs/audio-video-messages-2026-10-04.md, 2026-10-06): on
//  the owner's iPhone the composer row showed through the video recorder —
//  Close over the paperclip, Switch over ✨, the "Voice message" caption over
//  the field, the composer's Send over Record — and "Not recording" floated
//  over message text. Two halves, each measured on the real views:
//
//    - THE COMPOSER is neither drawn nor hittable while a recorder is open —
//      out of the hierarchy on iPhone and iPad (`ComposerUnlessRecording`),
//      keeping its height; hidden and untouchable on the Mac
//      (`hiddenWhileRecorderOpen`) — read from pixels, the view tree and the
//      window's own hit test. (The show-through itself does not reproduce in
//      a simulator snapshot of a NavigationStack inset under an overlay —
//      measured: the unhidden composer stayed covered there — so it is the
//      owner's device that confirms the fix; these pin what the fix does.)
//    - THE RECORDER's parts never overlap: the status capsule, the circle,
//      the control row, each caption and the solid bar, at 320, 375 and 430
//      points wide, on its side, on an iPad, on a Mac window and at large
//      text, in PREVIEW (with the first-time line and a reply), RECORDING and
//      REVIEW. Where each part landed is reported by the view itself
//      (`RecorderLayoutProbe`), so this checks the layout that is drawn, not
//      a copy of its arithmetic.
//

import AVFoundation
import Foundation
import SwiftUI
import Testing
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif
@testable import FamilyConnect

@MainActor
@Suite("The video recorder's layout: nothing overlaps, the composer is gone", .serialized)
struct VideoRecorderLayoutTests {

    // MARK: - A real window

    #if os(iOS)
    @MainActor
    final class Host {
        let window: UIWindow
        let controller: UIHostingController<AnyView>

        /// `edgeToEdge`: the view is laid out over the whole window, with no
        /// safe area — so a test's coordinates are the view's own.
        init(_ view: some View, size: CGSize, edgeToEdge: Bool = true) {
            let canvas = CGRect(origin: .zero, size: size)
            let sized = view.frame(width: size.width, height: size.height)
            controller = UIHostingController(
                rootView: edgeToEdge ? AnyView(sized.ignoresSafeArea()) : AnyView(sized))
            controller.view.backgroundColor = .clear
            let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
            window = scene.map { UIWindow(windowScene: $0) } ?? UIWindow(frame: canvas)
            window.frame = canvas
            window.backgroundColor = .black
            window.rootViewController = controller
            window.isHidden = false
            settle()
        }

        func settle() {
            for _ in 0..<4 {
                window.layoutIfNeeded()
                RunLoop.main.run(until: Date().addingTimeInterval(0.05))
            }
        }

        /// Pixels at scale 1, RGBA.
        func pixels() -> (data: [UInt8], width: Int, height: Int) {
            let format = UIGraphicsImageRendererFormat()
            format.scale = 1
            format.opaque = true
            let image = UIGraphicsImageRenderer(bounds: window.bounds, format: format).image { context in
                window.layer.render(in: context.cgContext)
            }
            guard let cgImage = image.cgImage else { return ([], 0, 0) }
            var data = [UInt8](repeating: 0, count: cgImage.width * cgImage.height * 4)
            let context = CGContext(
                data: &data, width: cgImage.width, height: cgImage.height, bitsPerComponent: 8,
                bytesPerRow: cgImage.width * 4, space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
            context?.draw(cgImage, in: CGRect(x: 0, y: 0, width: cgImage.width, height: cgImage.height))
            return (data, cgImage.width, cgImage.height)
        }

        func hit(_ point: CGPoint) -> UIView? { window.hitTest(point, with: nil) }

        func close() { window.isHidden = true }
    }
    #elseif os(macOS)
    @MainActor
    final class Host {
        let window: NSWindow
        let hosting: NSHostingView<AnyView>

        init(_ view: some View, size: CGSize) {
            hosting = NSHostingView(rootView: AnyView(view.frame(width: size.width, height: size.height)))
            window = NSWindow(
                contentRect: NSRect(origin: .zero, size: size),
                styleMask: [.borderless], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = hosting
            hosting.frame = NSRect(origin: .zero, size: size)
            settle()
        }

        func settle() {
            for _ in 0..<4 {
                hosting.layoutSubtreeIfNeeded()
                window.displayIfNeeded()
                RunLoop.main.run(until: Date().addingTimeInterval(0.03))
            }
        }

        func pixels() -> (data: [UInt8], width: Int, height: Int) {
            guard let rep = hosting.bitmapImageRepForCachingDisplay(in: hosting.bounds) else { return ([], 0, 0) }
            hosting.cacheDisplay(in: hosting.bounds, to: rep)
            // Sampled once per POINT, in sRGB, whatever the backing scale.
            let width = Int(hosting.bounds.width), height = Int(hosting.bounds.height)
            let scale = CGFloat(rep.pixelsWide) / hosting.bounds.width
            var data = [UInt8](repeating: 0, count: width * height * 4)
            for y in 0..<height {
                for x in 0..<width {
                    let color = rep.colorAt(
                        x: Int((CGFloat(x) + 0.5) * scale), y: Int((CGFloat(y) + 0.5) * scale)
                    )?.usingColorSpace(.sRGB)
                    let i = (y * width + x) * 4
                    data[i] = UInt8(max(0, min(255, (color?.redComponent ?? 0) * 255)))
                    data[i + 1] = UInt8(max(0, min(255, (color?.greenComponent ?? 0) * 255)))
                    data[i + 2] = UInt8(max(0, min(255, (color?.blueComponent ?? 0) * 255)))
                    data[i + 3] = UInt8(max(0, min(255, (color?.alphaComponent ?? 0) * 255)))
                }
            }
            return (data, width, height)
        }

        /// The deepest view under a point given in top-left coordinates.
        func hit(_ point: CGPoint) -> NSView? {
            let local = hosting.isFlipped ? point : NSPoint(x: point.x, y: hosting.bounds.height - point.y)
            return hosting.hitTest(hosting.convert(local, to: hosting.superview))
        }

        func close() { window.close() }
    }
    #endif

    /// Green — a colour nothing in the recorder draws. Loose enough for the
    /// Mac, whose snapshot comes back colour-matched (pure green reads about
    /// 130, 246, 93 in sRGB there).
    private static func isMarker(_ data: [UInt8], _ i: Int) -> Bool {
        data[i] < 170 && data[i + 1] > 190 && data[i + 2] < 150
    }

    private static func markerPixels(_ host: Host) -> Int {
        let (data, width, height) = host.pixels()
        var count = 0
        for i in stride(from: 0, to: width * height * 4, by: 4) where isMarker(data, i) { count += 1 }
        return count
    }

    // MARK: - A recorder to lay out

    private func session(
        firstTime: Bool = false, reply: Bool = false
    ) -> (VideoMessageSession, FakeCaptureEngine, (UInt64) -> Void) {
        let engine = FakeCaptureEngine()
        var request = VideoMessageSession.Request(chatID: 42)
        if reply {
            request.reply = ReplyToDTO(messageID: 41, senderID: 7, excerpt: "Dinner at 8?")
            request.replyTitle = "Replying to Anna"
            request.replyText = "Dinner at 8? Bring the long cable, the short one is in the car"
        }
        let session = VideoMessageSession(request: request, engine: engine, firstTime: firstTime)
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
                .appendingPathComponent("fc-test-layout-\(UUID().uuidString).mp4")
        }
        session.playbackSession = PlaybackSessionControl(begin: {}, end: {})
        let arbiter = VoiceRecordingArbiter()
        arbiter.keepAwake = { _ in }
        session.arbiter = arbiter
        session.nowPlaying = NowPlaying()
        session.markTaught = {}
        session.onClose = {}
        session.onProceed = { _ in }
        return (session, engine, { now = $0 })
    }

    enum Phase: String, CaseIterable, CustomTestStringConvertible {
        case preview, recording, review
        var testDescription: String { rawValue }
    }

    /// Bring a session to `phase`. REVIEW leaves a clip: the caller deletes it.
    private func reach(_ phase: Phase, _ session: VideoMessageSession, _ engine: FakeCaptureEngine,
                       _ setNow: (UInt64) -> Void) throws {
        session.start()
        engine.say(.firstFrame)
        guard phase != .preview else { return }
        setNow(1_000)
        session.record()
        guard phase != .recording else { return }
        setNow(6_000)
        session.stop()
        try engine.finish(ms: 5_000)
    }

    struct Window: CustomTestStringConvertible, Sendable {
        let width: CGFloat
        let height: CGFloat
        let large: Bool
        var testDescription: String { "\(Int(width))×\(Int(height))\(large ? ", large text" : "")" }
    }

    #if os(iOS)
    /// iPhone SE, iPhone, iPhone Pro Max — upright and on their side — and an
    /// iPad both ways; the smallest and the tallest also at large text.
    static let windows: [Window] = [
        Window(width: 320, height: 568, large: false), Window(width: 375, height: 667, large: false),
        Window(width: 430, height: 932, large: false), Window(width: 568, height: 320, large: false),
        Window(width: 667, height: 375, large: false), Window(width: 932, height: 430, large: false),
        Window(width: 1024, height: 1366, large: false), Window(width: 1366, height: 1024, large: false),
        Window(width: 320, height: 568, large: true), Window(width: 430, height: 932, large: true),
        Window(width: 932, height: 430, large: true),
        // What the app actually hands the recorder: the conversation pane, a
        // navigation bar and the home indicator shorter than the screen —
        // iPhone SE, iPhone and Pro Max, upright and on their side (less the
        // side insets), and a Slide Over column.
        Window(width: 375, height: 603, large: false), Window(width: 375, height: 603, large: true),
        Window(width: 667, height: 343, large: false), Window(width: 667, height: 343, large: true),
        Window(width: 750, height: 337, large: false), Window(width: 750, height: 337, large: true),
        Window(width: 838, height: 377, large: true), Window(width: 320, height: 504, large: false),
        Window(width: 320, height: 504, large: true),
    ]
    #else
    /// A Mac window at the conversation's minimum, a usual one, and a wide one.
    static let windows: [Window] = [
        Window(width: 420, height: 520, large: false), Window(width: 800, height: 700, large: false),
        Window(width: 1280, height: 800, large: false), Window(width: 420, height: 520, large: true),
    ]
    #endif

    private func layOut(
        _ phase: Phase, in window: Window, firstTime: Bool, reply: Bool
    ) throws -> (RecorderLayoutProbe, () -> Void) {
        let (session, engine, setNow) = session(firstTime: firstTime, reply: reply)
        try reach(phase, session, engine, setNow)
        let probe = RecorderLayoutProbe()
        let host = Host(
            VideoMessageRecorderView(session: session, paneFrame: nil)
                .environment(\.recorderLayoutProbe, probe)
                .environment(\.dynamicTypeSize, window.large ? .accessibility3 : .large),
            size: CGSize(width: window.width, height: window.height))
        return (probe, {
            host.close()
            if phase == .review { session.delete() }
        })
    }

    private static func describe(_ rect: CGRect) -> String {
        "(\(Int(rect.minX)),\(Int(rect.minY)) \(Int(rect.width))×\(Int(rect.height)))"
    }

    /// Two frames overlap by more than a hairline.
    private static func overlap(_ a: CGRect, _ b: CGRect) -> Bool {
        let shared = a.intersection(b)
        return !shared.isNull && shared.width > 0.5 && shared.height > 0.5
    }

    // MARK: - Nothing overlaps

    @Test("status, circle, controls, captions and the bar never overlap", arguments: windows, Phase.allCases)
    func nothingOverlaps(window: Window, phase: Phase) throws {
        // PREVIEW carries the most: the first-time line under the status and
        // a reply banner over the controls.
        let busy = phase == .preview
        let (probe, done) = try layOut(phase, in: window, firstTime: busy, reply: busy)
        defer { done() }
        let bounds = CGRect(x: 0, y: 0, width: window.width, height: window.height)

        let status = try #require(probe.rects(.status).first, "\(window): no status line")
        let circle = try #require(probe.rects(.circle).first, "\(window): no circle")
        let controls = try #require(probe.rects(.controls).first, "\(window): no control row")
        let bar = try #require(probe.rects(.bar).first, "\(window): no control bar")
        let captions = probe.rects(.caption)
        #expect(probe.rects(.status).count == 1 && probe.rects(.circle).count == 1)
        #expect(captions.count >= 2, "\(window) \(phase): \(captions.count) captions")

        let label = "\(window) \(phase)"
        #expect(!Self.overlap(circle, status),
                "\(label): the circle \(Self.describe(circle)) runs into the status \(Self.describe(status))")
        #expect(!Self.overlap(circle, controls),
                "\(label): the circle \(Self.describe(circle)) runs into the controls \(Self.describe(controls))")
        #expect(!Self.overlap(circle, bar),
                "\(label): the circle \(Self.describe(circle)) runs onto the bar \(Self.describe(bar))")
        #expect(!Self.overlap(status, bar) && !Self.overlap(status, controls),
                "\(label): the status \(Self.describe(status)) runs into the controls")
        // The controls sit ON the bar, inside the window.
        #expect(bar.insetBy(dx: -0.5, dy: -0.5).contains(controls),
                "\(label): the controls \(Self.describe(controls)) are off their bar \(Self.describe(bar))")
        #expect(bounds.insetBy(dx: -0.5, dy: -0.5).contains(controls),
                "\(label): the controls \(Self.describe(controls)) leave the window")
        #expect(bounds.insetBy(dx: -0.5, dy: -0.5).contains(status),
                "\(label): the status \(Self.describe(status)) leaves the window")
        for (i, caption) in captions.enumerated() {
            #expect(bounds.insetBy(dx: -0.5, dy: -0.5).contains(caption),
                    "\(label): a caption \(Self.describe(caption)) leaves the window")
            #expect(!Self.overlap(caption, circle) && !Self.overlap(caption, status),
                    "\(label): a caption \(Self.describe(caption)) runs into the circle or the status")
            for other in captions[(i + 1)...] {
                #expect(!Self.overlap(caption, other),
                        "\(label): captions \(Self.describe(caption)) and \(Self.describe(other)) collide")
            }
        }
        // And the circle is still a circle worth looking at.
        let floor: CGFloat = window.large ? 80 : 140
        #expect(circle.width >= floor, "\(label): the circle shrank to \(Int(circle.width))")
        #expect(abs(circle.width - circle.height) < 1)
    }

    @Test("the circle is as large as the room allows, never over 320, never larger than its room")
    func diameterFitsTheRoom() {
        let chrome = VideoMessageRecorderView.circleChrome
        #expect(VideoMessageRecorderView.diameter(fitting: CGSize(width: 1000, height: 1000)) == 320)
        #expect(VideoMessageRecorderView.diameter(fitting: CGSize(width: 300, height: 1000)) == 300 - chrome)
        #expect(VideoMessageRecorderView.diameter(fitting: CGSize(width: 1000, height: 200)) == 200 - chrome)
        #expect(VideoMessageRecorderView.diameter(fitting: CGSize(width: 10, height: 10)) == 0)
    }

    // MARK: - The composer is not there

    #if os(iOS)
    private final class Tappable: UIView {}
    private struct TappableView: UIViewRepresentable {
        func makeUIView(context: Context) -> Tappable { Tappable() }
        func updateUIView(_ view: Tappable, context: Context) {}
    }
    #else
    private final class Tappable: NSView {}
    private struct TappableView: NSViewRepresentable {
        func makeNSView(context: Context) -> Tappable { Tappable() }
        func updateNSView(_ view: Tappable, context: Context) {}
    }
    #endif

    /// A stand-in composer: a green bar with a control in it.
    private var standIn: some View {
        HStack(spacing: 0) {
            Color(red: 0, green: 1, blue: 0)
            TappableView().frame(width: 60, height: 44)
        }
        .frame(height: 44)
    }

    #if os(iOS)
    private func composer(open: Bool) -> some View {
        ComposerUnlessRecording(open: open, height: 44) { standIn }
    }
    #else
    private func composer(open: Bool) -> some View {
        standIn.hiddenWhileRecorderOpen(open)
    }
    #endif

    #if os(iOS)
    private func contains(_ view: UIView, _ type: Tappable.Type) -> Bool {
        view is Tappable || view.subviews.contains { contains($0, type) }
    }
    #endif

    private func isTappable(_ view: Any?) -> Bool {
        #if os(iOS)
        var current = view as? UIView
        while let v = current {
            if v is Tappable { return true }
            current = v.superview
        }
        return false
        #else
        var current = view as? NSView
        while let v = current {
            if v is Tappable { return true }
            current = v.superview
        }
        return false
        #endif
    }

    @Test("while a recorder is open the composer is neither drawn nor hittable, and keeps its height — and is all three once it closes")
    func composerHidden() throws {
        for open in [false, true] {
            let host = Host(
                VStack(spacing: 0) {
                    Color(red: 1, green: 0, blue: 0)
                    composer(open: open)
                },
                size: CGSize(width: 320, height: 200))
            defer { host.close() }
            let green = Self.markerPixels(host)
            let hit = host.hit(CGPoint(x: 290, y: 178))
            // The thread above ends where it always did: the composer's
            // height is kept, open or not.
            let (data, width, _) = host.pixels()
            let red = { (y: Int) -> Bool in
                let i = (y * width + 10) * 4
                return data[i] > 190 && data[i + 1] < 120 && data[i + 2] < 120
            }
            let probe = [10, 100, 150, 160, 190].map { y -> String in
                let i = (y * width + 10) * 4
                return "\(y):\(data[i]),\(data[i + 1]),\(data[i + 2])"
            }.joined(separator: " ")
            #expect(red(150) && !red(160), "the composer's height changed (open: \(open)); column at x=10: \(probe)")
            if open {
                #expect(green == 0, "the composer is drawn under an open recorder: \(green) of its pixels")
                #expect(!isTappable(hit), "the composer's control still takes touches under an open recorder")
                #if os(iOS)
                #expect(!contains(host.window, Tappable.self), "the composer is still in the hierarchy under an open recorder")
                #endif
            } else {
                #expect(green > 1_000, "the stand-in composer did not draw")
                #expect(isTappable(hit), "the stand-in composer's control took no touch: \(String(describing: hit))")
            }
        }
    }
}
