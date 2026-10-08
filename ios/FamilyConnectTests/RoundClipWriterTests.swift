//
//  RoundClipWriterTests.swift
//  FamilyConnectTests
//
//  The round video's file (#79, Phase 3 — docs/audio-video-messages-2026-10-04.md,
//  "The recording profile for a round video"): synthetic camera sample
//  buffers — 640 × 480 at 60 fps, the shape a front camera hands over, and
//  48 kHz stereo sound — written by the recorder's own `RoundClipWriter`, then
//  read back. What comes out must be the profile exactly: 480 × 480 H.264
//  High, AAC mono, `moov` before `mdat`, upright with an identity transform,
//  30 fps at most — and `MediaPlan.planVideo` must keep it as it is, because
//  a video message is recorded to the profile and never planned again.
//

import AVFoundation
import CoreMedia
import Foundation
import Testing
@testable import FamilyConnect

@Suite("Video message: the recorder's file")
struct RoundClipWriterTests {

    /// Where camera clocks start: nowhere near zero.
    private static let base = CMTime(seconds: 1_000, preferredTimescale: 600)

    // MARK: - Making camera-like samples

    private static func pixelBuffer(
        width: Int, height: Int, frame: Int, scene: inout MediaFixtures.Scene, white: Bool = false
    ) throws -> CVPixelBuffer {
        var made: CVPixelBuffer?
        let attributes: [String: Any] = [kCVPixelBufferIOSurfacePropertiesKey as String: [String: Any]()]
        CVPixelBufferCreate(
            nil, width, height, kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
            attributes as CFDictionary, &made)
        let buffer = try #require(made)
        CVPixelBufferLockBaseAddress(buffer, [])
        defer { CVPixelBufferUnlockBaseAddress(buffer, []) }
        if white {
            let luma = CVPixelBufferGetBaseAddressOfPlane(buffer, 0)!
            memset(luma, 235, CVPixelBufferGetBytesPerRowOfPlane(buffer, 0) * CVPixelBufferGetHeightOfPlane(buffer, 0))
            let chroma = CVPixelBufferGetBaseAddressOfPlane(buffer, 1)!
            memset(chroma, 128, CVPixelBufferGetBytesPerRowOfPlane(buffer, 1) * CVPixelBufferGetHeightOfPlane(buffer, 1))
        } else {
            scene.draw(into: buffer, frame: frame, frameRate: 60)
        }
        return buffer
    }

    private static func videoSample(_ buffer: CVPixelBuffer, at time: CMTime) throws -> CMSampleBuffer {
        var format: CMVideoFormatDescription?
        CMVideoFormatDescriptionCreateForImageBuffer(allocator: nil, imageBuffer: buffer, formatDescriptionOut: &format)
        var timing = CMSampleTimingInfo(
            duration: CMTime(value: 1, timescale: 60), presentationTimeStamp: time, decodeTimeStamp: .invalid)
        var sample: CMSampleBuffer?
        CMSampleBufferCreateReadyWithImageBuffer(
            allocator: nil, imageBuffer: buffer, formatDescription: try #require(format),
            sampleTiming: &timing, sampleBufferOut: &sample)
        return try #require(sample)
    }

    /// 16-bit interleaved PCM of a 440 Hz sine — what a microphone's data
    /// output hands over, here in stereo so the mono mix-down is seen.
    private static func audioSample(
        frames: Int, startFrame: Int, sampleRate: Int = 48_000, channels: Int = 2, at time: CMTime
    ) throws -> CMSampleBuffer {
        var description = AudioStreamBasicDescription(
            mSampleRate: Double(sampleRate), mFormatID: kAudioFormatLinearPCM,
            mFormatFlags: kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked,
            mBytesPerPacket: UInt32(2 * channels), mFramesPerPacket: 1,
            mBytesPerFrame: UInt32(2 * channels), mChannelsPerFrame: UInt32(channels),
            mBitsPerChannel: 16, mReserved: 0)
        var format: CMAudioFormatDescription?
        CMAudioFormatDescriptionCreate(
            allocator: nil, asbd: &description, layoutSize: 0, layout: nil,
            magicCookieSize: 0, magicCookie: nil, extensions: nil, formatDescriptionOut: &format)
        var samples = [Int16](repeating: 0, count: frames * channels)
        for index in 0..<frames {
            let phase = Double(startFrame + index) * 2 * Double.pi * 440 / Double(sampleRate)
            let value = Int16(8_000 * sin(phase))
            for channel in 0..<channels { samples[index * channels + channel] = value }
        }
        let bytes = samples.withUnsafeBytes { Data($0) }
        var block: CMBlockBuffer?
        CMBlockBufferCreateWithMemoryBlock(
            allocator: nil, memoryBlock: nil, blockLength: bytes.count, blockAllocator: nil,
            customBlockSource: nil, offsetToData: 0, dataLength: bytes.count,
            flags: kCMBlockBufferAssureMemoryNowFlag, blockBufferOut: &block)
        let made = try #require(block)
        bytes.withUnsafeBytes { raw in
            _ = CMBlockBufferReplaceDataBytes(
                with: raw.baseAddress!, blockBuffer: made, offsetIntoDestination: 0, dataLength: bytes.count)
        }
        var sample: CMSampleBuffer?
        CMAudioSampleBufferCreateReadyWithPacketDescriptions(
            allocator: nil, dataBuffer: made, formatDescription: try #require(format), sampleCount: frames,
            presentationTimeStamp: time, packetDescriptions: nil, sampleBufferOut: &sample)
        return try #require(sample)
    }

    /// Feed `seconds` of 60 fps video and 48 kHz sound, interleaved by time
    /// as a capture session delivers them, waiting for the writer as a test
    /// may (a camera's writer drops instead). Returns what each video frame
    /// came to.
    @discardableResult
    private static func feed(
        _ writer: RoundClipWriter, seconds: Double, width: Int = 640, height: Int = 480,
        white: Bool = false, audioLead: Double = 0
    ) async throws -> [RoundClipWriter.Outcome] {
        var scene = MediaFixtures.Scene(width: width, height: height, seconds: seconds)
        let totalFrames = Int(seconds * 60)
        let chunk = 1_024
        // Sound that starts BEFORE the first frame (`audioLead`) is not part
        // of the clip.
        let audioStart = CMTimeSubtract(base, CMTime(seconds: audioLead, preferredTimescale: 48_000))
        let totalAudio = Int((seconds + audioLead) * 48_000)
        var frame = 0
        var audioFrame = 0
        var outcomes: [RoundClipWriter.Outcome] = []
        let deadline = Date().addingTimeInterval(120)
        while frame < totalFrames || audioFrame < totalAudio {
            #expect(Date() < deadline, "the writer stalled")
            if Date() >= deadline { break }
            let videoTime = CMTimeAdd(base, CMTime(value: CMTimeValue(frame), timescale: 60))
            let audioTime = CMTimeAdd(audioStart, CMTime(value: CMTimeValue(audioFrame), timescale: 48_000))
            let videoNext = frame < totalFrames
                && (audioFrame >= totalAudio || CMTimeCompare(videoTime, audioTime) <= 0)
            if videoNext {
                // Only frames the writer would take need it to be ready.
                if writer.startTime != nil, !writer.isReadyForVideo {
                    try await Task.sleep(for: .milliseconds(1))
                    continue
                }
                let buffer = try pixelBuffer(width: width, height: height, frame: frame, scene: &scene, white: white)
                outcomes.append(writer.appendVideo(try videoSample(buffer, at: videoTime)))
                frame += 1
            } else {
                if writer.startTime != nil, CMTimeCompare(audioTime, base) >= 0, !writer.isReadyForAudio {
                    try await Task.sleep(for: .milliseconds(1))
                    continue
                }
                let count = min(chunk, totalAudio - audioFrame)
                _ = writer.appendAudio(try audioSample(frames: count, startFrame: audioFrame, at: audioTime))
                audioFrame += count
            }
        }
        return outcomes
    }

    private static func finish(_ writer: RoundClipWriter) async -> RoundClipWriter.Clip? {
        await withCheckedContinuation { continuation in
            writer.finish { continuation.resume(returning: $0) }
        }
    }

    // MARK: - The profile

    @Test("480 × 480 H.264 High, AAC mono, moov first, upright, at most 30 fps — and kept as it is")
    func writesTheProfile() async throws {
        let url = MediaFixtures.scratch("mp4")
        defer { try? FileManager.default.removeItem(at: url) }
        let writer = try RoundClipWriter(url: url, capMS: 59_500, realTime: false)
        let outcomes = try await Self.feed(writer, seconds: 6, audioLead: 0.3)
        // 60 fps in, 30 fps kept: every other frame is too soon.
        #expect(outcomes.filter { $0 == .appended }.count == 180, "\(outcomes.filter { $0 == .appended }.count)")
        let clip = try #require(await Self.finish(writer))
        #expect(clip.url == url)
        #expect(clip.durationMS >= 5_990 && clip.durationMS <= 6_010, "\(clip.durationMS)")

        let facts = try await MediaFixtures.facts(of: url)
        #expect(facts.naturalSize == CGSize(width: 480, height: 480))
        #expect(facts.codec == kCMVideoCodecType_H264)
        #expect(facts.profile == 100, "H.264 High")
        #expect(facts.transform == .identity, "upright pixels, never a rotation matrix")
        #expect(facts.nominalFrameRate <= 30.5, "\(facts.nominalFrameRate)")
        #expect(facts.shortestFrameGap >= 1.0 / 30 - 0.005, "\(facts.shortestFrameGap)")
        #expect(facts.audioCodec == kAudioFormatMPEG4AAC)
        #expect(facts.audioChannels == 1, "mono")
        #expect(abs(facts.durationSeconds - 6) < 0.1, "\(facts.durationSeconds)")
        // The sound before the first frame is not in the clip.
        let audio = try await MediaFixtures.audioFacts(of: url)
        #expect(audio.durationSeconds < 6.1, "\(audio.durationSeconds)")
        #expect(audio.sampleRate == 48_000 || audio.sampleRate == 44_100)

        let boxes = try MediaFixtures.topLevelBoxes(of: url)
        let moov = try #require(boxes.firstIndex(of: "moov"))
        let mdat = try #require(boxes.firstIndex(of: "mdat"))
        #expect(moov < mdat, "moov before mdat: \(boxes)")

        // Within the profile in everything a client decides, so no client
        // would touch it: the plan for it is to keep it.
        let source = await MediaProbe.video(
            at: url, container: try #require(MediaProbe.videoContainer(of: url)),
            sizeBytes: MediaPrep.fileSize(of: url))
        #expect(MediaPlan.planVideo(source) == .keep, "\(source)")
        // ≈ 70 KB a second (the profile's size row), with room for an encoder.
        #expect(Double(MediaPrep.fileSize(of: url)) / 6 < 110_000, "\(MediaPrep.fileSize(of: url))")
    }

    @Test("the square is the CENTRE of the camera's picture, filled, never letterboxed")
    func fillsTheSquare() async throws {
        let url = MediaFixtures.scratch("mp4")
        defer { try? FileManager.default.removeItem(at: url) }
        let writer = try RoundClipWriter(url: url, capMS: 59_500, realTime: false)
        try await Self.feed(writer, seconds: 1.5, white: true)
        _ = try #require(await Self.finish(writer))
        let generator = AVAssetImageGenerator(asset: AVURLAsset(url: url))
        let image = try await generator.image(at: CMTime(seconds: 0.5, preferredTimescale: 600)).image
        #expect(image.width == 480 && image.height == 480)
        // A letterboxed 640 × 480 would leave 60-pixel black bars top and
        // bottom; filled, the corners are as white as the middle.
        for (x, y) in [(4, 4), (475, 4), (4, 475), (475, 475), (240, 240)] {
            #expect(try Self.brightness(of: image, x: x, y: y) > 0.8, "(\(x), \(y))")
        }
    }

    /// One pixel's brightness, 0…1.
    private static func brightness(of image: CGImage, x: Int, y: Int) throws -> Double {
        var pixel = [UInt8](repeating: 0, count: 4)
        let context = try #require(CGContext(
            data: &pixel, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.draw(image, in: CGRect(x: -x, y: -(image.height - 1 - y), width: image.width, height: image.height))
        return (Double(pixel[0]) + Double(pixel[1]) + Double(pixel[2])) / (3 * 255)
    }

    // MARK: - The edges

    @Test("the length limit: nothing past it is taken, and the writer says so")
    func limit() async throws {
        let url = MediaFixtures.scratch("mp4")
        defer { try? FileManager.default.removeItem(at: url) }
        let writer = try RoundClipWriter(url: url, capMS: 1_000, realTime: false)
        let outcomes = try await Self.feed(writer, seconds: 2)
        #expect(writer.reachedLimit)
        #expect(outcomes.contains(.limit))
        #expect(!outcomes.drop(while: { $0 != .limit }).contains(.appended), "nothing after the limit")
        let clip = try #require(await Self.finish(writer))
        #expect(clip.durationMS <= 1_034, "\(clip.durationMS)")
    }

    @Test("sound before the first frame is dropped, and nothing written is nothing kept")
    func nothingWritten() async throws {
        let url = MediaFixtures.scratch("mp4")
        let writer = try RoundClipWriter(url: url, capMS: 59_500, realTime: false)
        let early = try Self.audioSample(frames: 1_024, startFrame: 0, at: Self.base)
        #expect(writer.appendAudio(early) == .dropped)
        #expect(await Self.finish(writer) == nil)
        #expect(!FileManager.default.fileExists(atPath: url.path))
    }

    @Test("sound stamped before the first frame is dropped even once the clip has begun")
    func soundBeforeTheClip() async throws {
        let url = MediaFixtures.scratch("mp4")
        defer { try? FileManager.default.removeItem(at: url) }
        let writer = try RoundClipWriter(url: url, capMS: 59_500, realTime: false)
        var scene = MediaFixtures.Scene(width: 640, height: 480, seconds: 1)
        let first = try Self.pixelBuffer(width: 640, height: 480, frame: 0, scene: &scene)
        #expect(writer.appendVideo(try Self.videoSample(first, at: Self.base)) == .appended)
        let before = CMTimeSubtract(Self.base, CMTime(value: 2_048, timescale: 48_000))
        #expect(writer.appendAudio(try Self.audioSample(frames: 1_024, startFrame: 0, at: before)) == .dropped)
        #expect(writer.appendAudio(try Self.audioSample(frames: 1_024, startFrame: 0, at: Self.base)) == .appended)
        writer.cancel()
    }

    // MARK: - Upright from the first frame (S3.4, S3.5)

    @Test("frames turned at the preview's angle are not written: the clip begins at the first upright frame")
    func sidewaysFramesAreNotTaken() async throws {
        let url = MediaFixtures.scratch("mp4")
        defer { try? FileManager.default.removeItem(at: url) }
        // Held portrait at Record: the angle turns the camera's 640 × 480
        // into 480 × 640 — but frames already on their way were turned by
        // the preview's angle, still landscape.
        let writer = try RoundClipWriter(url: url, capMS: 59_500, shape: .portrait, realTime: false)
        var wide = MediaFixtures.Scene(width: 640, height: 480, seconds: 2)
        var tall = MediaFixtures.Scene(width: 480, height: 640, seconds: 2)
        func time(_ frame: Int) -> CMTime { CMTimeAdd(Self.base, CMTime(value: CMTimeValue(frame), timescale: 30)) }
        for frame in 0..<10 {
            let sideways = try Self.pixelBuffer(width: 640, height: 480, frame: frame, scene: &wide)
            #expect(writer.appendVideo(try Self.videoSample(sideways, at: time(frame))) == .dropped,
                    "a sideways frame was written")
        }
        #expect(writer.startTime == nil, "the clip began at a sideways frame")
        var frame = 10
        let deadline = Date().addingTimeInterval(60)
        while frame < 40, Date() < deadline {
            if writer.startTime != nil, !writer.isReadyForVideo {
                try await Task.sleep(for: .milliseconds(1))
                continue
            }
            let upright = try Self.pixelBuffer(width: 480, height: 640, frame: frame, scene: &tall, white: true)
            #expect(writer.appendVideo(try Self.videoSample(upright, at: time(frame))) == .appended)
            if frame == 20 {
                // One more still in flight after the start: not written either.
                let late = try Self.pixelBuffer(width: 640, height: 480, frame: frame, scene: &wide)
                let lateTime = CMTimeAdd(time(frame), CMTime(value: 1, timescale: 60))
                #expect(writer.appendVideo(try Self.videoSample(late, at: lateTime)) == .dropped)
            }
            frame += 1
        }
        #expect(writer.startTime == time(10))
        let clip = try #require(await Self.finish(writer))
        #expect(clip.durationMS >= 990 && clip.durationMS <= 1_010, "\(clip.durationMS)")
        // Its very first frame is the upright, white one.
        let generator = AVAssetImageGenerator(asset: AVURLAsset(url: url))
        generator.requestedTimeToleranceBefore = .zero
        generator.requestedTimeToleranceAfter = .zero
        let first = try await generator.image(at: .zero).image
        for (x, y) in [(4, 4), (475, 4), (4, 475), (475, 475), (240, 240)] {
            #expect(try Self.brightness(of: first, x: x, y: y) > 0.8, "(\(x), \(y))")
        }
    }

    @Test("the shape a capture angle delivers: a quarter turn swaps it, a half turn does not")
    func frameShapes() {
        #expect(FrameShape.delivered(native: .landscape, rotationAngle: 0) == .landscape)
        #expect(FrameShape.delivered(native: .landscape, rotationAngle: 90) == .portrait)
        #expect(FrameShape.delivered(native: .landscape, rotationAngle: 180) == .landscape)
        #expect(FrameShape.delivered(native: .landscape, rotationAngle: 270) == .portrait)
        #expect(FrameShape.delivered(native: .portrait, rotationAngle: 90) == .landscape)
        #expect(FrameShape.delivered(native: .portrait, rotationAngle: 360) == .portrait)
        #expect(FrameShape.of(width: 640, height: 480) == .landscape)
        #expect(FrameShape.of(width: 480, height: 640) == .portrait)
        #expect(FrameShape.of(width: 480, height: 480) == nil)
        #expect(FrameShape.portrait.matches(width: 480, height: 640))
        #expect(!FrameShape.portrait.matches(width: 640, height: 480))
        #expect(FrameShape.landscape.matches(width: 480, height: 480), "a square is either")
    }

    @Test("with no shape fixed (the Mac), every frame is taken")
    func noShapeTakesEverything() throws {
        let url = MediaFixtures.scratch("mp4")
        let writer = try RoundClipWriter(url: url, capMS: 59_500, realTime: false)
        var wide = MediaFixtures.Scene(width: 640, height: 480, seconds: 1)
        let buffer = try Self.pixelBuffer(width: 640, height: 480, frame: 0, scene: &wide)
        #expect(writer.appendVideo(try Self.videoSample(buffer, at: Self.base)) == .appended)
        writer.cancel()
    }

    @Test("a take thrown away leaves no file")
    func cancel() async throws {
        let url = MediaFixtures.scratch("mp4")
        let writer = try RoundClipWriter(url: url, capMS: 59_500, realTime: false)
        try await Self.feed(writer, seconds: 0.5)
        writer.cancel()
        #expect(!FileManager.default.fileExists(atPath: url.path))
        var scene = MediaFixtures.Scene(width: 64, height: 64, seconds: 1)
        let late = try Self.pixelBuffer(width: 64, height: 64, frame: 0, scene: &scene)
        #expect(writer.appendVideo(try Self.videoSample(late, at: Self.base)) == .dropped)
    }

    @Test("the writer's settings are the profile's numbers")
    func settings() throws {
        let video = RoundClipProfile.videoSettings
        #expect(video[AVVideoWidthKey] as? Int == 480)
        #expect(video[AVVideoHeightKey] as? Int == 480)
        #expect(video[AVVideoScalingModeKey] as? String == AVVideoScalingModeResizeAspectFill)
        let compression = try #require(video[AVVideoCompressionPropertiesKey] as? [String: Any])
        #expect(compression[AVVideoAverageBitRateKey] as? Int == 500_000)
        #expect(compression[AVVideoMaxKeyFrameIntervalDurationKey] as? Int == 2)
        let audio = RoundClipProfile.audioSettings
        #expect(audio[AVNumberOfChannelsKey] as? Int == 1)
        #expect(audio[AVEncoderBitRateKey] as? Int == 64_000)
        #expect(audio[AVFormatIDKey] as? AudioFormatID == kAudioFormatMPEG4AAC)
    }
}
