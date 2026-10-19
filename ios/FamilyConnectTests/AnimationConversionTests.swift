//
//  AnimationConversionTests.swift
//  FamilyConnectTests
//
//  An animated picture made into an ANIMATION (issue #96, docs/protocol.md
//  "Animations" › "What becomes one" and "The file"): which pictures are
//  one, the delays read off their bytes, and the MP4 itself — every frame
//  the plan says, for exactly its ticks, on white where the picture is
//  transparent, at the profile's size, with no sound, `moov` first. And
//  rule C: a picture that cannot be made into one goes exactly as it went
//  before.
//
//  The GIF and the APNG are drawn here with CGImageDestination (a platform
//  that can write both); the WebP is StickerFixtures' — an Apple device
//  decodes WebP and cannot write it.
//

import AVFoundation
import CoreGraphics
import Foundation
import ImageIO
import Testing
import UniformTypeIdentifiers
@testable import FamilyConnect

@Suite("Animations: what becomes one, and the file it becomes")
struct AnimationConversionTests {

    // MARK: - Pictures

    /// One frame of a synthesized animation: a colour over the whole canvas
    /// or, with `leftHalfOnly`, over its left half with the right half
    /// fully transparent.
    struct Frame {
        var red: CGFloat
        var green: CGFloat
        var blue: CGFloat
        var leftHalfOnly = false
        var delaySeconds: Double
    }

    static func image(_ frame: Frame, width: Int, height: Int) -> CGImage? {
        guard let context = CGContext(
            data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { return nil }
        context.clear(CGRect(x: 0, y: 0, width: width, height: height))
        context.setFillColor(red: frame.red, green: frame.green, blue: frame.blue, alpha: 1)
        context.fill(CGRect(
            x: 0, y: 0, width: frame.leftHalfOnly ? width / 2 : width, height: height))
        return context.makeImage()
    }

    /// An animated GIF or PNG of these frames, looping for ever.
    static func animation(
        _ type: UTType, width: Int, height: Int, frames: [Frame]
    ) throws -> Data {
        let output = NSMutableData()
        let destination = try #require(CGImageDestinationCreateWithData(
            output, type.identifier as CFString, frames.count, nil))
        let (dictionary, loop, delay): (CFString, CFString, CFString) = type == .gif
            ? (kCGImagePropertyGIFDictionary, kCGImagePropertyGIFLoopCount, kCGImagePropertyGIFDelayTime)
            : (kCGImagePropertyPNGDictionary, kCGImagePropertyAPNGLoopCount, kCGImagePropertyAPNGDelayTime)
        CGImageDestinationSetProperties(destination, [dictionary: [loop: 0]] as CFDictionary)
        for frame in frames {
            let image = try #require(Self.image(frame, width: width, height: height))
            CGImageDestinationAddImage(
                destination, image, [dictionary: [delay: frame.delaySeconds]] as CFDictionary)
        }
        #expect(CGImageDestinationFinalize(destination))
        return output as Data
    }

    /// Red with a transparent right half, a 20 ms blue flash, a "0" green
    /// (shown for 100 ms), and 70 ms of red.
    static let gifFrames = [
        Frame(red: 1, green: 0, blue: 0, leftHalfOnly: true, delaySeconds: 0.1),
        Frame(red: 0, green: 0, blue: 1, delaySeconds: 0.02),
        Frame(red: 0, green: 1, blue: 0, delaySeconds: 0),
        Frame(red: 1, green: 0, blue: 0, delaySeconds: 0.07),
    ]

    static let pngFrames = [
        Frame(red: 1, green: 0, blue: 0, leftHalfOnly: true, delaySeconds: 0.1),
        Frame(red: 0, green: 0, blue: 1, delaySeconds: 0.05),
        Frame(red: 0, green: 1, blue: 0, delaySeconds: 0.2),
    ]

    static func scratch(_ ext: String) -> URL {
        FileManager.default.temporaryDirectory
            .appendingPathComponent("animation-test-\(UUID().uuidString)")
            .appendingPathExtension(ext)
    }

    // MARK: - What becomes one

    @Test("an animated GIF, WebP or PNG is an animation; a still one and a TIFF are not")
    func detection() throws {
        #expect(AnimatedPicture.isAnimation(StickerFixtures.animatedGIF))
        #expect(AnimatedPicture.isAnimation(StickerFixtures.animatedPNG))
        #expect(AnimatedPicture.isAnimation(StickerFixtures.animatedWebP))
        #expect(!AnimatedPicture.isAnimation(StickerFixtures.stillWebP))
        #expect(!AnimatedPicture.isAnimation(StickerFixtures.png(width: 8, height: 8)))
        #expect(!AnimatedPicture.isAnimation(StickerFixtures.twoPageTIFF))
        let stillGIF = try Self.animation(
            .gif, width: 8, height: 8, frames: [Frame(red: 1, green: 0, blue: 0, delaySeconds: 0.1)])
        #expect(!AnimatedPicture.isAnimation(stillGIF))
        #expect(!AnimatedPicture.isAnimation(TestImages.photograph(width: 16, height: 16)))
        #expect(!AnimatedPicture.isAnimation(Data("GIF89a not really".utf8)))
        #expect(!AnimatedPicture.isAnimation(Data()))
    }

    @Test("the format is read from the magic numbers")
    func format() {
        #expect(AnimatedPicture.format(of: StickerFixtures.animatedGIF) == .gif)
        #expect(AnimatedPicture.format(of: StickerFixtures.animatedPNG) == .png)
        #expect(AnimatedPicture.format(of: StickerFixtures.animatedWebP) == .webp)
        #expect(AnimatedPicture.format(of: TestImages.photograph(width: 8, height: 8)) == nil)
    }

    // MARK: - Delays, as the bytes state them

    @Test("a GIF's delays are its Graphic Control Extensions', in hundredths")
    func gifDelays() throws {
        let gif = try Self.animation(.gif, width: 16, height: 12, frames: Self.gifFrames)
        #expect(AnimatedPicture.gifDelaysMS(gif) == [100, 20, 0, 70])
        let source = try #require(CGImageSourceCreateWithData(gif as CFData, nil))
        #expect(AnimatedPicture.statedDelaysMS(gif, source: source) == [100, 20, 0, 70])
    }

    @Test("an APNG's delays are its fcTL chunks', num over den")
    func apngDelays() throws {
        let png = try Self.animation(.png, width: 16, height: 12, frames: Self.pngFrames)
        #expect(AnimatedPicture.apngDelaysMS(png) == [100, 50, 200])
        let source = try #require(CGImageSourceCreateWithData(png as CFData, nil))
        #expect(AnimatedPicture.statedDelaysMS(png, source: source) == [100, 50, 200])
    }

    @Test("a WebP's delays are its ANMF chunks', in milliseconds")
    func webpDelays() throws {
        #expect(AnimatedPicture.webpDelaysMS(StickerFixtures.animatedWebP) == [40, 40])
        let source = try #require(CGImageSourceCreateWithData(StickerFixtures.animatedWebP as CFData, nil))
        #expect(AnimatedPicture.statedDelaysMS(StickerFixtures.animatedWebP, source: source) == [40, 40])
    }

    @Test("a GIF delay of zero and an image with no extension at all both read as 0")
    func gifWithoutControlExtension() {
        // GIF89a, 1×1, a two-colour global table, then two images: the first
        // with no Graphic Control Extension, the second after one saying 7.
        var bytes: [UInt8] = Array("GIF89a".utf8) + [1, 0, 1, 0, 0x80, 0, 0, 0, 0, 0, 255, 255, 255]
        let image: [UInt8] = [0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0, 2, 2, 0x44, 0x01, 0]
        bytes += image
        bytes += [0x21, 0xF9, 4, 0, 7, 0, 0, 0]
        bytes += image
        bytes += [0x3B]
        #expect(AnimatedPicture.gifDelaysMS(Data(bytes)) == [0, 70])
    }

    // MARK: - The file

    /// What the written MP4 is, frame by frame.
    struct Decoded {
        var presentationTicks: [Int]
        var durationTicks: Int
        /// (r, g, b) at the left and right quarter of each frame's middle row.
        var left: [[Int]]
        var right: [[Int]]
        var width: Int
        var height: Int
        var hasAudio: Bool
        var trackTimescale: CMTimeScale
    }

    static func decode(_ url: URL) async throws -> Decoded {
        let asset = AVURLAsset(url: url)
        let track = try #require(try await asset.loadTracks(withMediaType: .video).first)
        let size = try await track.load(.naturalSize)
        let timescale = try await track.load(.naturalTimeScale)
        let duration = try await asset.load(.duration)
        let reader = try AVAssetReader(asset: asset)
        let output = AVAssetReaderTrackOutput(track: track, outputSettings: [
            kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
        ])
        reader.add(output)
        #expect(reader.startReading())
        var ticks: [Int] = []
        var left: [[Int]] = []
        var right: [[Int]] = []
        while let sample = output.copyNextSampleBuffer() {
            guard CMSampleBufferGetNumSamples(sample) > 0,
                  let buffer = CMSampleBufferGetImageBuffer(sample) else { continue }
            let time = CMSampleBufferGetPresentationTimeStamp(sample)
            // Ticks of 1/30 s, exactly — or not a whole number of them.
            let exact = time.convertScale(30, method: .default)
            #expect(CMTimeCompare(exact, time) == 0, "a frame off the 30-a-second grid: \(time)")
            ticks.append(Int(exact.value))
            CVPixelBufferLockBaseAddress(buffer, .readOnly)
            let base = try #require(CVPixelBufferGetBaseAddress(buffer)).assumingMemoryBound(to: UInt8.self)
            let row = CVPixelBufferGetBytesPerRow(buffer)
            let width = CVPixelBufferGetWidth(buffer)
            let height = CVPixelBufferGetHeight(buffer)
            func rgb(_ x: Int, _ y: Int) -> [Int] {
                let at = y * row + x * 4
                return [Int(base[at + 2]), Int(base[at + 1]), Int(base[at])]
            }
            left.append(rgb(width / 4, height / 2))
            right.append(rgb(width * 3 / 4, height / 2))
            CVPixelBufferUnlockBaseAddress(buffer, .readOnly)
        }
        #expect(reader.status == .completed, "\(String(describing: reader.error)) after \(ticks.count) frames")
        let total = duration.convertScale(30, method: .default)
        #expect(CMTimeCompare(total, duration) == 0, "the clip does not end on a tick: \(duration)")
        return Decoded(
            presentationTicks: ticks, durationTicks: Int(total.value),
            left: left, right: right,
            width: Int(size.width), height: Int(size.height),
            hasAudio: !(try await asset.loadTracks(withMediaType: .audio)).isEmpty,
            trackTimescale: timescale)
    }

    /// Within what H.264's 4:2:0 and a round trip through YUV leave of a
    /// flat colour.
    static func near(_ rgb: [Int], _ expected: [Int], tolerance: Int = 24) -> Bool {
        zip(rgb, expected).allSatisfy { abs($0 - $1) <= tolerance }
    }

    /// The plan's frames as presentation ticks.
    static func startTicks(of plan: AnimationTiming.Plan) -> [Int] {
        var starts: [Int] = []
        var tick = 0
        for sample in plan.samples {
            starts.append(tick)
            tick += sample.ticks
        }
        return starts
    }

    @Test("an animated GIF becomes the plan's silent MP4, on white, moov first")
    func gifBecomesTheClip() async throws {
        let gif = try Self.animation(.gif, width: 64, height: 48, frames: Self.gifFrames)
        let plan = try #require(AnimationTiming.plan(width: 64, height: 48, statedMS: [100, 20, 0, 70]))
        // The reference's arithmetic, written out: 0–100 red, the flash at
        // 100 owns tick 3, green to 220, red to 290 — nine ticks.
        #expect(plan.samples == [
            .init(frame: 0, ticks: 3), .init(frame: 1, ticks: 1),
            .init(frame: 2, ticks: 3), .init(frame: 3, ticks: 2),
        ])

        let output = Self.scratch("mp4")
        defer { try? FileManager.default.removeItem(at: output) }
        let encoded = try await AnimationEncoder.encode(gif, writingTo: output)
        #expect(encoded.width == 64 && encoded.height == 48)
        #expect(encoded.durationMS == 300)
        #expect(encoded.frameCount == 4)
        #expect(encoded.posterJPEG.map { MediaPrep.Magic.matches(mime: "image/jpeg", head: $0) } == true)

        let topLevel = try MediaFixtures.topLevelBoxes(of: output)
        let moov = try #require(topLevel.firstIndex(of: "moov"))
        let mdat = try #require(topLevel.firstIndex(of: "mdat"))
        #expect(moov < mdat, "moov must come first: \(topLevel)")

        let decoded = try await Self.decode(output)
        #expect(!decoded.hasAudio)
        #expect(decoded.trackTimescale % 30 == 0)
        #expect(decoded.width == 64 && decoded.height == 48)
        #expect(decoded.presentationTicks == Self.startTicks(of: plan))
        #expect(decoded.durationTicks == plan.totalTicks)
        // Frame 0: red on the left, WHITE where the GIF is transparent.
        #expect(Self.near(decoded.left[0], [255, 0, 0]), "\(decoded.left[0])")
        #expect(Self.near(decoded.right[0], [255, 255, 255]), "\(decoded.right[0])")
        // The flash, then green, then red — each the source frame the plan names.
        #expect(Self.near(decoded.left[1], [0, 0, 255]), "\(decoded.left[1])")
        #expect(Self.near(decoded.right[2], [0, 255, 0]), "\(decoded.right[2])")
        #expect(Self.near(decoded.right[3], [255, 0, 0]), "\(decoded.right[3])")

        let facts = try await MediaFixtures.facts(of: output)
        #expect(facts.codec == kCMVideoCodecType_H264)
        #expect(facts.profile == 100 || facts.profile == 77, "H.264 High, or Main")
    }

    @Test("an animated PNG becomes the plan's silent MP4 too — no longer flattened")
    func apngBecomesTheClip() async throws {
        let png = try Self.animation(.png, width: 40, height: 30, frames: Self.pngFrames)
        let plan = try #require(AnimationTiming.plan(width: 40, height: 30, statedMS: [100, 50, 200]))
        #expect(plan.samples == [
            .init(frame: 0, ticks: 3), .init(frame: 1, ticks: 2), .init(frame: 2, ticks: 6),
        ])

        let prepared = try await MediaPrep.prepare(
            data: png, type: .png, limit: MediaPrep.sizeLimit, animates: true)
        defer { MediaPrep.discard(prepared) }
        #expect(prepared.animation)
        #expect(prepared.kind == "video")
        #expect(prepared.mime == "video/mp4")
        #expect(prepared.width == 40 && prepared.height == 30)
        #expect(prepared.durationMS == plan.durationMS)
        #expect(prepared.durationMS == 367)
        #expect(prepared.previewJPEG != nil, "the poster is the first frame")

        let decoded = try await Self.decode(prepared.fileURL)
        #expect(!decoded.hasAudio)
        #expect(decoded.presentationTicks == Self.startTicks(of: plan))
        #expect(decoded.durationTicks == 11)
        #expect(Self.near(decoded.right[0], [255, 255, 255]), "\(decoded.right[0])")
        #expect(Self.near(decoded.left[1], [0, 0, 255]), "\(decoded.left[1])")
        #expect(Self.near(decoded.left[2], [0, 255, 0]), "\(decoded.left[2])")
    }

    @Test("a picture larger than the profile is scaled to it, both sides even")
    func theSizeFollowsTheProfile() async throws {
        let gif = try Self.animation(.gif, width: 1001, height: 1500, frames: [
            Frame(red: 1, green: 0, blue: 0, delaySeconds: 0.1),
            Frame(red: 0, green: 0, blue: 1, delaySeconds: 0.1),
        ])
        let output = Self.scratch("mp4")
        defer { try? FileManager.default.removeItem(at: output) }
        let encoded = try await AnimationEncoder.encode(gif, writingTo: output)
        let expected = MediaPlan.targetSize(width: 1001, height: 1500)
        #expect(encoded.width == expected.width && encoded.height == expected.height)
        #expect(encoded.width == 720)
        let decoded = try await Self.decode(output)
        #expect(decoded.width == expected.width && decoded.height == expected.height)
    }

    @Test("the animated WebP fixture becomes a clip, or goes as before")
    func webpBecomesTheClipOrAFile() async throws {
        // Eight pixels a side is below what some H.264 encoders take; either
        // outcome is the protocol's — an animation, or rule C's file.
        let prepared = try await MediaPrep.prepare(
            data: StickerFixtures.animatedWebP, type: .webP, limit: MediaPrep.sizeLimit,
            animates: true)
        defer { MediaPrep.discard(prepared) }
        if prepared.animation {
            #expect(prepared.kind == "video")
            #expect(prepared.durationMS == 100, "two 40 ms frames: 80 ms, three ticks")
            let decoded = try await Self.decode(prepared.fileURL)
            #expect(decoded.durationTicks == 3)
            #expect(!decoded.hasAudio)
        } else {
            #expect(prepared.kind == "file")
            #expect(try Data(contentsOf: prepared.fileURL) == StickerFixtures.animatedWebP)
        }
    }

    // MARK: - Through MediaPrep

    @Test("a GIF dropped or picked from Files becomes an animation in a family chat")
    func aGIFFileBecomesAnAnimation() async throws {
        let url = Self.scratch("gif")
        try Self.animation(.gif, width: 32, height: 24, frames: Self.gifFrames).write(to: url)
        defer { try? FileManager.default.removeItem(at: url) }

        let dropped = try await MediaPrep.prepare(fileAt: url, limit: MediaPrep.sizeLimit, animates: true)
        defer { MediaPrep.discard(dropped) }
        #expect(dropped.animation && dropped.kind == "video")

        let picked = try await MediaPrep.preparePickedFile(
            from: url, limit: MediaPrep.sizeLimit, animates: true)
        defer { MediaPrep.discard(picked) }
        #expect(picked.animation && picked.kind == "video")
    }

    @Test("where nothing animates — the assistant's chat, the board — a GIF goes as it always went")
    func withoutAnimatesNothingChanges() async throws {
        let gif = try Self.animation(.gif, width: 32, height: 24, frames: Self.gifFrames)
        let asFile = try await MediaPrep.prepare(data: gif, type: .gif, limit: MediaPrep.sizeLimit)
        defer { MediaPrep.discard(asFile) }
        #expect(!asFile.animation)
        #expect(asFile.kind == "file")
        #expect(asFile.mime == "image/gif")
        #expect(try Data(contentsOf: asFile.fileURL) == gif)

        let png = try Self.animation(.png, width: 32, height: 24, frames: Self.pngFrames)
        let asPhoto = try await MediaPrep.prepare(data: png, type: .png, limit: MediaPrep.sizeLimit)
        defer { MediaPrep.discard(asPhoto) }
        #expect(!asPhoto.animation)
        #expect(asPhoto.kind == "photo")

        let url = Self.scratch("gif")
        try gif.write(to: url)
        defer { try? FileManager.default.removeItem(at: url) }
        let picked = try await MediaPrep.preparePickedFile(
            from: url, limit: MediaPrep.sizeLimit, animates: false)
        defer { MediaPrep.discard(picked) }
        #expect(picked.kind == "file" && !picked.animation)
    }

    @Test("a still GIF, a still WebP and a BMP go exactly as before")
    func stillsAreUnchanged() async throws {
        let stillGIF = try Self.animation(
            .gif, width: 8, height: 8, frames: [Frame(red: 1, green: 0, blue: 0, delaySeconds: 0.1)])
        let gif = try await MediaPrep.prepare(
            data: stillGIF, type: .gif, limit: MediaPrep.sizeLimit, animates: true)
        defer { MediaPrep.discard(gif) }
        #expect(gif.kind == "file" && !gif.animation)

        let webp = try await MediaPrep.prepare(
            data: StickerFixtures.stillWebP, type: .webP, limit: MediaPrep.sizeLimit, animates: true)
        defer { MediaPrep.discard(webp) }
        #expect(webp.kind == "file" && !webp.animation)

        let png = try await MediaPrep.prepare(
            data: StickerFixtures.png(width: 16, height: 16), type: .png,
            limit: MediaPrep.sizeLimit, animates: true)
        defer { MediaPrep.discard(png) }
        #expect(png.kind == "photo" && !png.animation)
    }

    // MARK: - Rule C

    /// A seam whose animation encoder always fails — the only way to reach
    /// rule C on demand for a picture that IS an animation.
    static let failingEncoder = MediaPrep.Transcoder(
        video: { _, _, _ in throw MediaTranscoder.Failure.unsupportedSettings },
        audio: { _, _, _ in throw MediaTranscoder.Failure.unsupportedSettings },
        animation: { _, _ in throw AnimationEncoder.Failure.writer("refused on purpose") })

    @Test("rule C: an animation that cannot be encoded goes as before — a GIF as its file")
    func ruleCForAGIF() async throws {
        let gif = try Self.animation(.gif, width: 32, height: 24, frames: Self.gifFrames)
        let prepared = try await MediaPrep.prepare(
            data: gif, type: .gif, limit: MediaPrep.sizeLimit,
            transcoder: Self.failingEncoder, animates: true)
        defer { MediaPrep.discard(prepared) }
        #expect(!prepared.animation)
        #expect(prepared.kind == "file")
        #expect(try Data(contentsOf: prepared.fileURL) == gif, "the original bytes, untouched")
        #expect(try await MediaPrep.prepareAnimation(
            from: gif, limit: MediaPrep.sizeLimit, transcoder: Self.failingEncoder) == nil)
    }

    @Test("rule C: an animated PNG that cannot be encoded takes the photo path, as before")
    func ruleCForAnAPNG() async throws {
        let png = try Self.animation(.png, width: 32, height: 24, frames: Self.pngFrames)
        let prepared = try await MediaPrep.prepare(
            data: png, type: .png, limit: MediaPrep.sizeLimit,
            transcoder: Self.failingEncoder, animates: true)
        defer { MediaPrep.discard(prepared) }
        #expect(!prepared.animation)
        #expect(prepared.kind == "photo")
    }

    @Test("rule C: a broken GIF is no animation and goes as the file it is")
    func aBrokenGIF() async throws {
        // A GIF header and a truncated screen descriptor: nothing decodes.
        let broken = Data("GIF89a\u{2}\u{0}".utf8) + Data(repeating: 0xEE, count: 40)
        #expect(!AnimatedPicture.isAnimation(broken))
        let output = Self.scratch("mp4")
        await #expect(throws: AnimationEncoder.Failure.unreadable) {
            _ = try await AnimationEncoder.encode(broken, writingTo: output)
        }
        let prepared = try await MediaPrep.prepare(
            data: broken, type: .gif, limit: MediaPrep.sizeLimit, animates: true)
        defer { MediaPrep.discard(prepared) }
        #expect(prepared.kind == "file")
        #expect(try Data(contentsOf: prepared.fileURL) == broken)

        // An animated GIF cut off inside its second image: ImageIO may still
        // count both frames, so whichever way it goes, nothing is lost —
        // a clip, or the very bytes that were given.
        let whole = try Self.animation(.gif, width: 32, height: 24, frames: Self.gifFrames)
        let cut = whole.prefix(whole.count * 2 / 3)
        let truncated = try await MediaPrep.prepare(
            data: Data(cut), type: .gif, limit: MediaPrep.sizeLimit, animates: true)
        defer { MediaPrep.discard(truncated) }
        if truncated.animation {
            #expect(truncated.kind == "video")
        } else {
            #expect(truncated.kind == "file")
            #expect(try Data(contentsOf: truncated.fileURL) == Data(cut))
        }
    }

    @Test("a cancellation is not rule C: nothing is sent at all")
    func cancellationIsNotRuleC() async throws {
        let gif = try Self.animation(.gif, width: 32, height: 24, frames: Self.gifFrames)
        let cancelling = MediaPrep.Transcoder(
            video: { _, _, _ in }, audio: { _, _, _ in },
            animation: { _, _ in throw CancellationError() })
        await #expect(throws: CancellationError.self) {
            _ = try await MediaPrep.prepareAnimation(
                from: gif, limit: MediaPrep.sizeLimit, transcoder: cancelling)
        }
    }
}
