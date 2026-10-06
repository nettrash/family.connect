//
//  VoiceVideoPolishTests.swift
//  FamilyConnectTests
//
//  The approved design for #79 ("Voice and Video Messages"), held where the
//  pixels can be read:
//
//    - the waveform bars: one bar per level that fits, each (2 + level) / 17
//      of the height, the played ones in the played colour;
//    - the voice bubble draws the SENDER's waveform, and a flat placeholder
//      without one;
//    - the live waveform scrolls in from the trailing edge;
//    - the speed cycle, remembered;
//    - a recording's menu: no Copy, Edit or Share; Show text, Playback speed
//      (voice), Save to Files — ordinary messages' menus unchanged;
//    - the held microphone grows, red, under the finger; a touch long press
//      on the microphone meets nothing but the hold;
//    - the video message's badge carries its unplayed dot INSIDE it, white,
//      and the circle has a soft shadow;
//    - the recorder's one big button: a red disc, a red disc with a white
//      square, a disc in the tint — 64 across.
//
//  ImageRenderer at scale 1, as RoundBubbleTests and VoiceComposerRowsTests.
//

import CoreGraphics
import Foundation
import SwiftData
import SwiftUI
import Testing
@testable import FamilyConnect
#if os(iOS)
import UIKit
#endif

@MainActor
@Suite("Voice and video: the approved design")
struct VoiceVideoPolishTests {

    // MARK: - Pixels

    struct Pixels {
        let data: [UInt8]
        let width: Int
        let height: Int

        func rgba(_ x: Int, _ y: Int) -> (r: UInt8, g: UInt8, b: UInt8, a: UInt8) {
            let offset = (y * width + x) * 4
            return (data[offset], data[offset + 1], data[offset + 2], data[offset + 3])
        }
    }

    static func render(_ view: some View, scale: CGFloat = 1) throws -> Pixels {
        let renderer = ImageRenderer(content: view)
        renderer.scale = scale
        let image = try #require(renderer.cgImage, "the view did not render")
        var data = [UInt8](repeating: 0, count: image.width * image.height * 4)
        let context = try #require(CGContext(
            data: &data, width: image.width, height: image.height,
            bitsPerComponent: 8, bytesPerRow: image.width * 4,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
        return Pixels(data: data, width: image.width, height: image.height)
    }

    static func isRed(_ p: (r: UInt8, g: UInt8, b: UInt8, a: UInt8)) -> Bool {
        p.a > 200 && p.r > 200 && p.g < 80 && p.b < 80
    }

    static func isBlue(_ p: (r: UInt8, g: UInt8, b: UInt8, a: UInt8)) -> Bool {
        p.a > 200 && p.b > 200 && p.r < 80 && p.g < 80
    }

    // MARK: - The bars

    @Test("one bar per level, (2 + level) / 17 of the height, the played half in the played colour")
    func barsAreTheLevels() throws {
        // 48 bars of 3 with gaps of 2 fill 238 exactly; 34 tall makes every
        // bar 2 × (2 + level) points.
        let levels = (0..<48).map { UInt8($0 % 16) }
        let pixels = try Self.render(
            VoiceWaveformBars(
                levels: levels,
                playedFraction: (positionMS: 2_100, durationMS: 4_200),
                played: Color(red: 1, green: 0, blue: 0),
                unplayed: Color(red: 0, green: 0, blue: 1))
                .frame(width: 238, height: 34))
        #expect(VoiceWaveformBars.barCount(width: 238) == 48)
        for index in 0..<48 {
            let x = index * 5 + 1
            var red = 0
            var blue = 0
            for y in 0..<pixels.height {
                let p = pixels.rgba(x, y)
                if Self.isRed(p) { red += 1 }
                if Self.isBlue(p) { blue += 1 }
            }
            let expected = 2 * (2 + Int(levels[index]))
            if index < 24 {
                #expect(blue == 0, "bar \(index) is played but drawn unplayed")
                #expect(abs(red - expected) <= 1, "bar \(index) is \(red) tall, not \(expected)")
            } else {
                #expect(red == 0, "bar \(index) is not played but drawn played")
                #expect(abs(blue - expected) <= 1, "bar \(index) is \(blue) tall, not \(expected)")
            }
        }
    }

    @Test("never more bars than the 48 levels, never fewer than one")
    func barCount() {
        #expect(VoiceWaveformBars.barCount(width: 0) == 1)
        #expect(VoiceWaveformBars.barCount(width: 3) == 1)
        #expect(VoiceWaveformBars.barCount(width: 100) == 20)
        #expect(VoiceWaveformBars.barCount(width: 2_000) == 48)
    }

    // MARK: - The live waveform

    @Test("the live waveform: the newest peaks at the trailing edge, silence before them")
    func liveWaveform() {
        let levels = VoiceLiveWaveform.visibleLevels(peaks: [-60, -30, 0], count: 5)
        #expect(levels == [0, 0, 0, 8, 15])
        let scrolled = VoiceLiveWaveform.visibleLevels(peaks: [0, 0, -60, -60, -30, -2], count: 3)
        #expect(scrolled == [0, 8, 15], "the oldest peaks did not scroll off the leading edge")
        #expect(VoiceLiveWaveform.visibleLevels(peaks: [], count: 0).isEmpty)
    }

    // MARK: - Speed

    @Test("the speed goes 1× → 1.5× → 2× → 1×, and is remembered")
    func speedCycle() {
        var stored = 0.0
        let speed = VoicePlaybackSpeed(read: { 1 }, write: { stored = $0 })
        #expect(speed.rate == 1)
        speed.cycle()
        #expect(speed.rate == 1.5 && stored == 1.5)
        speed.cycle()
        #expect(speed.rate == 2 && stored == 2)
        speed.cycle()
        #expect(speed.rate == 1 && stored == 1)
        speed.set(3)
        #expect(speed.rate == 1, "a speed off the cycle was taken")
        #expect(VoicePlaybackSpeed.next(after: 0.75) == 1)
        #expect(VoicePlaybackSpeed.label(1.5) == String(localized: "1.5×"))
    }

    // MARK: - The played stores

    @Test("a played voice note and a played circle are separate facts, wiped together")
    func playedStoresAreSeparate() throws {
        let suite = "voice-plays-\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let circles = RoundVideoPlays(defaults: defaults, account: { "s-u7" })
        let voices = RoundVideoPlays(defaults: defaults, account: { "s-u7" }, keyPrefix: "voiceNotePlays.v1.")
        voices.markPlayed(5)
        #expect(voices.isPlayed(5))
        #expect(!circles.isPlayed(5), "a played voice note took a circle's dot away")
        circles.markPlayed(6)
        voices.removeAll()
        #expect(!voices.isPlayed(5))
        #expect(RoundVideoPlays(defaults: defaults, account: { "s-u7" }).isPlayed(6),
                "wiping the voice notes wiped the circles")
    }

    /// CROSS-CLIENT PARITY: a circle's value is "Not played" while its dot
    /// shows and "Played" once this device has played it — as the voice
    /// bubble says it, and as Android, Windows and the web say it of a
    /// circle — and nothing on one's own.
    @Test("a circle says Not played, then Played; one's own says nothing")
    func circlePlayedValue() throws {
        let suite = "round-value-\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let plays = RoundVideoPlays(defaults: defaults, account: { "s-u7" })
        let theirs = RoundVideoTile(attachment: roundVideo(), plays: plays)
        #expect(theirs.playedValue == String(localized: "Not played"))
        plays.markPlayed(91)
        #expect(theirs.playedValue == String(localized: "Played"))
        #expect(RoundVideoTile(attachment: roundVideo(), isMine: true, plays: plays).playedValue.isEmpty)
        #expect(RoundVideoTile(attachment: roundVideo(), upload: .sending, plays: plays).playedValue.isEmpty)
    }

    // MARK: - The menu

    private func message(body: String, attachments: [AttachmentDTO], sender: Int64 = 9) -> MessageSnapshot {
        MessageSnapshot(
            localID: "m:1", serverID: 500, chatID: 42, senderID: sender, body: body,
            createdAt: Date(timeIntervalSince1970: 0), state: .sent,
            attachment: attachments.first, attachments: attachments)
    }

    private func audio(name: String? = nil) -> AttachmentDTO {
        AttachmentDTO(
            id: 77, kind: "audio", mime: "audio/mp4", size: 4096, width: nil, height: nil,
            durationMS: 4_200, hasPreview: false, name: name, latitude: nil, longitude: nil,
            accuracyM: nil, waveform: VoiceWaveformFlowTests.shape)
    }

    private func roundVideo() -> AttachmentDTO {
        AttachmentDTO(
            id: 91, kind: "video", mime: "video/mp4", size: 4096, width: 480, height: 480,
            durationMS: 23_400, hasPreview: true, name: nil, latitude: nil, longitude: nil,
            accuracyM: nil, isRound: true)
    }

    @Test("a voice message is a recording; words beside it make it an ordinary message again")
    func whatIsARecording() {
        #expect(MessagePresentation.isVoiceMessage(message(body: "", attachments: [audio()])))
        #expect(MessagePresentation.isRecordingMessage(message(body: "", attachments: [audio()])))
        #expect(MessagePresentation.isRecordingMessage(message(body: "", attachments: [roundVideo()])))
        #expect(!MessagePresentation.isVoiceMessage(message(body: "listen", attachments: [audio()])))
        #expect(!MessagePresentation.isVoiceMessage(message(body: "", attachments: [audio(), audio()])))
        #expect(!MessagePresentation.isRecordingMessage(message(body: "hi", attachments: [])))
        // Edit is off your own voice message — there is nothing to edit —
        // and stays on your own captioned one.
        #expect(!MessagePresentation.offersEdit(
            message(body: "", attachments: [audio()], sender: 7), currentUserID: 7))
        #expect(MessagePresentation.offersEdit(
            message(body: "listen", attachments: [audio()], sender: 7), currentUserID: 7))
    }

    #if os(iOS)
    private static let row: CGFloat = 44

    private func rows(_ size: CGSize) -> Int {
        Int(((size.height + 1) / (Self.row + 1)).rounded())
    }

    @Test("a voice message's menu: Reply, Show text, Playback speed, Save to Files, Safety")
    func voiceMenu() {
        let size = MessageContextMenu.size(
            canReply: true, canCopy: false, canReport: true, blockState: .notBlocked,
            isRecording: true, transcriptRow: .show, offersSpeed: true)
        #expect(rows(size) == 5)
        // Without the transcript door: four.
        #expect(rows(MessageContextMenu.size(
            canReply: true, canCopy: false, canReport: true, blockState: .notBlocked,
            isRecording: true, offersSpeed: true)) == 4)
    }

    @Test("a video message's: Reply, Show text, Save to Files, Open Full Screen, Safety — six at most")
    func videoMenu() {
        let size = MessageContextMenu.size(
            canReply: true, canViewThread: true, canOpenFullScreen: true, canCopy: false,
            canReport: true, blockState: .notBlocked,
            isRecording: true, transcriptRow: .hide)
        #expect(rows(size) == 6)
    }

    @Test("a recording never offers Copy, Edit or Share — even when asked to")
    func recordingDropsTextItems() {
        let recording = MessageContextMenu.size(
            canReply: true, canEdit: true, canCopy: true, isRecording: true)
        // Reply and Save to Files, nothing else.
        #expect(rows(recording) == 2)
    }

    @Test("an ordinary message's menu is unchanged: Reply, Edit, Copy, Share")
    func ordinaryMenuUnchanged() {
        let size = MessageContextMenu.size(canReply: true, canEdit: true, canCopy: true)
        #expect(rows(size) == 4)
        #expect(size == MessageContextMenu.size(
            canReply: true, canEdit: true, canCopy: true, page: .main,
            transcriptRow: .show, offersSpeed: true),
            "a message that is not a recording grew recording rows")
    }
    #endif

    // MARK: - Show text from the menu

    @Test("the menu's text row follows the text: Show while it can be asked or is folded, Hide while shown")
    func transcriptMenuRow() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("transcript-menu-\(UUID().uuidString)")
        let store = TranscriptStore(
            api: APIClient(serverURL: URL(string: "https://transcripts.invalid")), directory: directory)
        #expect(store.menuRow(for: 77, door: .absent) == nil)
        #expect(store.menuRow(for: 77, door: .open) == .show)
        #expect(store.menuRow(for: 77, door: .asksFirst) == .show)

        // Asked from the menu: the section is told to ask.
        store.performMenuRow(.show, for: 77)
        #expect(store.showRequests[77] == 1)

        store.keep(TranscriptDTO(text: "hello", language: "en"), source: .stored, for: 77)
        #expect(store.menuRow(for: 77, door: .absent) == .hide, "kept text could not be hidden")
        store.performMenuRow(.hide, for: 77)
        #expect(store.kept(77)?.hidden == true)
        #expect(store.menuRow(for: 77, door: .absent) == .show)
        store.performMenuRow(.show, for: 77)
        #expect(store.kept(77)?.hidden == false)
        #expect(store.showRequests[77] == 1, "unfolding kept text asked the server again")
    }

    // MARK: - The voice bubble draws the sender's waveform

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
            let api = APIClient(serverURL: URL(string: "https://polish.invalid"))
            coordinator = ChatSyncCoordinator(modelContainer: container, api: api)
            coordinator.currentUserIDOverride = 7
            session = AppSession(api: api, defaultServerURL: { nil })
            let directory = URL(fileURLWithPath: NSTemporaryDirectory())
                .appendingPathComponent("polish-\(UUID().uuidString)")
            store = AttachmentStore(api: api, directory: directory)
        }
    }

    /// The heights of the waveform's bars: an own bubble draws its unplayed
    /// bars in white at 42 %, which nothing else in the row is.
    private func barHeights(waveform: String?) throws -> [Int] {
        let world = try World()
        let attachment = AttachmentDTO(
            id: 77, kind: "audio", mime: "audio/mp4", size: 4096, width: nil, height: nil,
            durationMS: 42_000, hasPreview: false, name: nil, latitude: nil, longitude: nil,
            accuracyM: nil, waveform: waveform)
        let pixels = try Self.render(
            AudioPlayerView(attachment: attachment, isMine: true)
                .frame(width: 260)
                .environment(world.coordinator)
                .environment(\.colorScheme, .light))
        // The bars' band: right of the 44-point button, above the time line.
        var heights: [Int] = []
        for x in 60..<pixels.width {
            var count = 0
            for y in 0..<min(34, pixels.height) {
                let p = pixels.rgba(x, y)
                if p.a > 80, p.a < 135, p.r > 80, p.g > 80, p.b > 80 { count += 1 }
            }
            if count > 0 { heights.append(count) }
        }
        return heights
    }

    @Test("the voice bubble draws the sender's waveform — and a flat row without one")
    func bubbleDrawsTheWaveform() throws {
        let shaped = try barHeights(waveform: String(repeating: "0f", count: 24))
        let flat = try barHeights(waveform: nil)
        try #require(!shaped.isEmpty && !flat.isEmpty, "no bars were drawn")
        // 28 points of waveform: level 15 is all of it, level 0 2/17 of it,
        // and the placeholder's level 4 is 6/17 — about 10.
        #expect((shaped.max() ?? 0) >= 25, "the loud bars are \(shaped.max() ?? 0) tall")
        #expect((shaped.min() ?? 99) <= 5, "the quiet bars are \(shaped.min() ?? 99) tall")
        #expect((flat.max() ?? 99) <= 12, "the placeholder's bars reach \(flat.max() ?? 99)")
        #expect((flat.max() ?? 0) >= 8, "the placeholder's bars are \(flat.max() ?? 0) tall")
    }

    #if os(iOS)

    // MARK: - The held microphone

    /// A view in a real window, laid out and drawn from its layer tree —
    /// for what ImageRenderer cannot draw: the slot's UIKit control and the
    /// recorder's focusable button (BubbleLayoutTests' recipe).
    static func hosted(_ view: some View, size: CGSize) throws -> Pixels {
        let canvas = CGRect(origin: .zero, size: size)
        let host = UIHostingController(rootView: view.frame(width: size.width, height: size.height))
        host.view.backgroundColor = .clear
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        let window = scene.map { UIWindow(windowScene: $0) } ?? UIWindow(frame: canvas)
        window.frame = canvas
        window.backgroundColor = .clear
        window.rootViewController = host
        window.isHidden = false
        defer { window.isHidden = true }
        for _ in 0..<3 {
            window.layoutIfNeeded()
            RunLoop.main.run(until: Date().addingTimeInterval(0.05))
        }
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        format.opaque = false
        let image = UIGraphicsImageRenderer(bounds: canvas, format: format).image { context in
            window.layer.render(in: context.cgContext)
        }
        let cgImage = try #require(image.cgImage, "the hosted view did not render")
        var data = [UInt8](repeating: 0, count: cgImage.width * cgImage.height * 4)
        let context = try #require(CGContext(
            data: &data, width: cgImage.width, height: cgImage.height,
            bitsPerComponent: 8, bytesPerRow: cgImage.width * 4,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.draw(cgImage, in: CGRect(x: 0, y: 0, width: cgImage.width, height: cgImage.height))
        return Pixels(data: data, width: cgImage.width, height: cgImage.height)
    }

    private func slotExtent(_ slot: ComposerSlot, matching: ((r: UInt8, g: UInt8, b: UInt8, a: UInt8)) -> Bool) throws -> Int {
        let pixels = try Self.hosted(
            RecordSendSlot(
                slot: slot, isPressed: false, focusRequest: 0, side: 36, glyph: 30,
                events: RecordSendEvents())
                .tint(Color(red: 0, green: 0, blue: 1)),
            size: CGSize(width: 120, height: 120))
        var minX = Int.max, maxX = -1
        for y in 0..<pixels.height {
            for x in 0..<pixels.width where matching(pixels.rgba(x, y)) {
                minX = min(minX, x); maxX = max(maxX, x)
            }
        }
        return maxX < 0 ? 0 : maxX - minX + 1
    }

    @Test("held, the microphone grows red under the finger; at rest it is the tint")
    func heldMicrophoneGrows() throws {
        let resting = try slotExtent(.microphone, matching: Self.isBlue)
        let restingRed = try slotExtent(.microphone, matching: Self.isRed)
        let held = try slotExtent(.heldMicrophone, matching: Self.isRed)
        #expect(resting > 20, "the resting microphone was not drawn in the tint")
        #expect(restingRed == 0, "the resting microphone is red")
        #expect(Double(held) >= Double(resting) * 1.25,
                "the held microphone is \(held) across, the resting one \(resting)")
    }

    @Test("a finger's long press on the microphone meets only the hold — in a window, too")
    func longPressMeetsOnlyTheHold() throws {
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 200, height: 200))
        let control = RecordSendControl(frame: CGRect(x: 78, y: 78, width: 44, height: 44))
        control.update(slot: .microphone, offersVideo: true, rtl: false, events: RecordSendEvents())
        window.addSubview(control)
        window.makeKeyAndVisible()
        window.layoutIfNeeded()
        defer { window.isHidden = true }

        // Once in a window, the edit menu installs its private bridge: a
        // context-menu interaction driven by the SECONDARY CLICK, and two
        // relationship recognizers that only order others (measured on the
        // iOS 26 SDK). Every recognizer a FINGER can reach on the control —
        // whatever an interaction installed — is the hold or one of those
        // three; nothing a finger's press can drive to a menu.
        #expect(!control.isContextMenuInteractionEnabled)
        let finger = NSNumber(value: UITouch.TouchType.direct.rawValue)
        let reachable = (control.gestureRecognizers ?? []).filter {
            $0.isEnabled && $0.allowedTouchTypes.contains(finger)
        }
        let longPresses = reachable.filter { $0 is UILongPressGestureRecognizer }
        #expect(longPresses.count == 1 && longPresses.first === control.longPress,
                "a finger's long press can reach \(longPresses.map { type(of: $0) })")
        let harmless: Set<String> = [
            "_UISecondaryClickDriverGestureRecognizer", "_UIRelationshipGestureRecognizer",
        ]
        for recognizer in reachable where recognizer !== control.longPress {
            let name = String(describing: type(of: recognizer))
            #expect(harmless.contains(name), "a finger can reach \(name)")
        }
        // And the menu is only ever built for a pointer's secondary click.
        #expect(control.secondaryClick.allowedTouchTypes.map(\.intValue)
                == [UITouch.TouchType.indirectPointer.rawValue])
    }

    // MARK: - The video message's badge and shadow

    private func circle(isMine: Bool, sender: Int64) throws -> (Pixels, minX: Int, minY: Int, maxX: Int, maxY: Int) {
        let world = try World()
        world.store.seed(RoundBubbleTestsPoster.red(), id: 91, preview: true)
        let snapshot = MessageSnapshot(
            localID: "r:1", serverID: 1, chatID: 42, senderID: sender, body: "",
            createdAt: Date(timeIntervalSince1970: 0), state: .sent,
            attachment: roundVideo(), attachments: [roundVideo()])
        let pixels = try Self.render(
            MessageBubbleView(
                message: snapshot, isMine: isMine, showsSenderName: false, senderName: nil,
                isRead: false, memberNames: [7: "You", 9: "Ana"], currentUserID: 7)
                .frame(width: 360)
                .tint(Color(red: 0, green: 0, blue: 1))
                .environment(\.horizontalSizeClass, .compact)
                .environment(\.dynamicTypeSize, .large)
                .environment(LinkPreviewLoader())
                .environment(world.store)
                .environment(world.coordinator)
                .environment(world.session)
                .environment(AvatarStore(api: APIClient(serverURL: URL(string: "https://avatars.invalid")))))
        var minX = Int.max, maxX = -1, minY = Int.max, maxY = -1
        for y in 0..<pixels.height {
            for x in 0..<pixels.width where Self.isRed(pixels.rgba(x, y)) {
                minX = min(minX, x); maxX = max(maxX, x)
                minY = min(minY, y); maxY = max(maxY, y)
            }
        }
        try #require(maxX > minX, "the poster was not drawn")
        return (pixels, minX, minY, maxX, maxY)
    }

    @Test("the unplayed dot is white and INSIDE the dark length badge; the circle casts a soft shadow")
    func badgeCarriesTheDot() throws {
        let own = try circle(isMine: true, sender: 7)
        let others = try circle(isMine: false, sender: 9)
        let diameter = own.maxX - own.minX + 1
        #expect(abs((others.maxX - others.minX + 1) - diameter) <= 1)

        // The badge's band: the bottom quarter of the circle, its middle half.
        func badge(_ shot: (Pixels, minX: Int, minY: Int, maxX: Int, maxY: Int))
            -> (whites: [(Int, Int)], capsule: (minX: Int, maxX: Int, minY: Int, maxY: Int))
        {
            var whites: [(Int, Int)] = []
            var cMinX = Int.max, cMaxX = -1, cMinY = Int.max, cMaxY = -1
            for dy in (diameter * 3 / 4)..<diameter {
                for dx in (diameter / 4)..<(diameter * 3 / 4) {
                    let p = shot.0.rgba(shot.minX + dx, shot.minY + dy)
                    if p.r > 220, p.g > 220, p.b > 220 { whites.append((dx, dy)) }
                    // The poster's red under black at 55 %.
                    if p.a > 200, p.r < 150, p.r > 60, p.g < 30, p.b < 30 {
                        cMinX = min(cMinX, dx); cMaxX = max(cMaxX, dx)
                        cMinY = min(cMinY, dy); cMaxY = max(cMaxY, dy)
                    }
                }
            }
            return (whites, (cMinX, cMaxX, cMinY, cMaxY))
        }
        let mine = badge(own)
        let theirs = badge(others)
        // The same "0:23" in both: the extra white is the dot.
        #expect(theirs.whites.count - mine.whites.count >= 12,
                "no white dot: \(theirs.whites.count) white pixels against \(mine.whites.count)")
        // And every white pixel sits inside the dark capsule.
        let capsule = theirs.capsule
        let outside = theirs.whites.filter { x, y in
            x < capsule.minX - 1 || x > capsule.maxX + 1 || y < capsule.minY - 1 || y > capsule.maxY + 1
        }
        #expect(capsule.maxX > capsule.minX, "the badge has no dark capsule")
        #expect(outside.isEmpty, "\(outside.count) white pixels lie outside the badge's capsule")

        // The shadow: just under the circle, outside it, a little darkness.
        let below = own.0.rgba((own.minX + own.maxX) / 2, min(own.0.height - 1, own.maxY + 4))
        #expect(below.a > 5, "the circle casts no shadow")
    }

    // MARK: - The recorder's one big button

    private func recorderSession() -> (VideoMessageSession, FakeCaptureEngine, (UInt64) -> Void) {
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
        return (session, engine, { now = $0 })
    }

    /// The big button: the widest run of `matching` pixels in the control
    /// row, and the colour at its centre.
    private func bigButton(
        _ session: VideoMessageSession, matching: ((r: UInt8, g: UInt8, b: UInt8, a: UInt8)) -> Bool
    ) throws -> (width: Int, centre: (r: UInt8, g: UInt8, b: UInt8, a: UInt8)) {
        let pixels = try Self.hosted(
            VideoMessageRecorderView(session: session, paneFrame: nil)
                .tint(Color(red: 0, green: 0, blue: 1))
                .environment(\.horizontalSizeClass, .compact),
            size: CGSize(width: 390, height: 760))
        var minX = Int.max, maxX = -1, minY = Int.max, maxY = -1
        for y in (pixels.height - 150)..<pixels.height {
            for x in 0..<pixels.width where matching(pixels.rgba(x, y)) {
                minX = min(minX, x); maxX = max(maxX, x)
                minY = min(minY, y); maxY = max(maxY, y)
            }
        }
        guard maxX >= 0 else { return (0, (0, 0, 0, 0)) }
        return (maxX - minX + 1, pixels.rgba((minX + maxX) / 2, (minY + maxY) / 2))
    }

    @Test("Record is a red disc 64 across; Stop a red disc with a white square; Send a disc in the tint")
    func recorderBigButton() throws {
        let (session, engine, setNow) = recorderSession()
        session.start()
        engine.say(.firstFrame)
        let record = try bigButton(session, matching: Self.isRed)
        #expect(abs(record.width - 64) <= 2, "Record is \(record.width) across")
        #expect(Self.isRed(record.centre), "Record is not a solid red disc")

        setNow(1_000)
        session.record()
        let stop = try bigButton(session, matching: Self.isRed)
        #expect(abs(stop.width - 64) <= 2, "Stop is \(stop.width) across")
        #expect(stop.centre.r > 230 && stop.centre.g > 230 && stop.centre.b > 230,
                "Stop has no white square at its centre")

        setNow(6_000)
        session.stop()
        try engine.finish(ms: 5_000)
        defer { session.delete() }
        let send = try bigButton(session, matching: Self.isBlue)
        #expect(abs(send.width - 64) <= 2, "Send is \(send.width) across")
    }

    // MARK: - The Undo row under Reduce Motion

    @Test("the Undo window counts whole seconds down and drains its line")
    func undoCountdown() {
        var now: UInt64 = 10_000
        let row = VoiceUndoRow(
            recordedMS: 12_000, untilMS: 15_000, windowMS: 5_000, clock: { now }, onUndo: {})
        #expect(row.secondsLeft == 5)
        #expect(row.fractionLeft == 1)
        now = 12_500
        #expect(row.secondsLeft == 3)
        #expect(abs(row.fractionLeft - 0.5) < 0.001)
        now = 15_000
        #expect(row.secondsLeft == 0)
        #expect(row.fractionLeft == 0)
    }

    #endif
}

/// A solid red square JPEG — a circle's poster.
enum RoundBubbleTestsPoster {
    static func red() -> Data {
        let context = CGContext(
            data: nil, width: 64, height: 64, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
        context.setFillColor(red: 1, green: 0, blue: 0, alpha: 1)
        context.fill(CGRect(x: 0, y: 0, width: 64, height: 64))
        return PlatformImage.jpegData(from: context.makeImage()!, quality: 1) ?? Data()
    }
}
