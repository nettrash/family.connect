//
//  AnimationEncoder.swift
//  FamilyConnect
//
//  An animated GIF, WebP or PNG made into the protocol's silent looping MP4
//  (issue #96, docs/protocol.md "Animations" › "The file").
//
//  ImageIO decodes: `CGImageSourceCreateImageAtIndex` hands back each frame
//  COMPOSITED — the canvas as a viewer would show it at that moment, the
//  frame's disposal and blending already applied (measured: a GIF's second
//  frame covering a corner comes back as the whole canvas with the first
//  frame under it). Each one is then drawn onto WHITE, scaled to the plan's
//  size, and `AVAssetWriter` encodes it: H.264 through the transcoder's own
//  settings ladder (High, else Main; 8-bit SDR BT.709), no audio track, a
//  timescale of 600 — a multiple of 30, so every frame lasts its ticks
//  exactly — and `moov` before `mdat`.
//
//  WHICH frames, and for how long, is `AnimationTiming.plan` alone — the
//  part four encoders have to agree on. The poster is the first frame on
//  white, as a JPEG, through the same preview size every video's has.
//
//  Anything that goes wrong throws; `MediaPrep.prepareAnimation` turns that
//  into rule C — the picture goes exactly as it went before, as a file.
//

import AVFoundation
import CoreGraphics
import CoreVideo
import Foundation
import ImageIO

nonisolated enum AnimationEncoder {

    enum Failure: Error, Equatable {
        /// Not an animation ImageIO can read, or a frame it could not decode.
        case unreadable
        /// No plan: no size, no frame, or a size that rounds to nothing.
        case nothingToPlan
        /// The writer refused the settings, an input or a frame.
        case writer(String)
    }

    /// What was written.
    struct Encoded: Equatable, Sendable {
        let width: Int
        let height: Int
        let durationMS: Int
        /// How many video frames the file holds — the plan's samples.
        let frameCount: Int
        /// The first frame on white, as a JPEG, at most `MediaPrep.previewEdge`.
        let posterJPEG: Data?
    }

    /// The track and movie timescale: a multiple of 30, so a frame of `n`
    /// ticks is exactly `n × 20` units long.
    static let timescale: CMTimeScale = 600

    /// Encode the animation in `data` into an MP4 at `output`. Throws on
    /// anything short of a complete file; the caller owns `output` either
    /// way and removes it on a throw.
    @concurrent
    static func encode(_ data: Data, writingTo output: URL) async throws -> Encoded {
        try Task.checkCancellation()
        guard AnimatedPicture.isAnimation(data),
              let source = CGImageSourceCreateWithData(data as CFData, [
                  kCGImageSourceShouldCache: false,
              ] as CFDictionary),
              let delays = AnimatedPicture.statedDelaysMS(data, source: source),
              let first = CGImageSourceCreateImageAtIndex(source, 0, nil)
        else { throw Failure.unreadable }
        // The canvas: ImageIO's composited frames are the whole canvas.
        guard let plan = AnimationTiming.plan(
            width: first.width, height: first.height, statedMS: delays)
        else { throw Failure.nothingToPlan }

        try? FileManager.default.removeItem(at: output)
        let writer = try AVAssetWriter(outputURL: output, fileType: .mp4)
        writer.shouldOptimizeForNetworkUse = true
        writer.movieTimeScale = timescale
        let target = MediaPlan.VideoTarget(
            width: plan.width, height: plan.height, frameRate: MediaPlan.maxFrameRate,
            videoBitrate: plan.videoBitrate, audioBitrate: nil)
        guard let settings = MediaTranscoder.h264Settings(for: target, writer: writer) else {
            throw Failure.writer("no H.264 settings this encoder takes")
        }
        let input = AVAssetWriterInput(mediaType: .video, outputSettings: settings)
        input.expectsMediaDataInRealTime = false
        input.mediaTimeScale = timescale
        guard writer.canAdd(input) else { throw Failure.writer("cannot add the video input") }
        writer.add(input)
        let adaptor = AVAssetWriterInputPixelBufferAdaptor(
            assetWriterInput: input,
            sourcePixelBufferAttributes: [
                kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
                kCVPixelBufferWidthKey as String: plan.width,
                kCVPixelBufferHeightKey as String: plan.height,
                kCVPixelBufferCGImageCompatibilityKey as String: true,
                kCVPixelBufferCGBitmapContextCompatibilityKey as String: true,
            ])

        guard writer.startWriting() else {
            throw Failure.writer(String(describing: writer.error))
        }
        writer.startSession(atSourceTime: .zero)

        var poster: Data?
        var tick = 0
        do {
            for (index, sample) in plan.samples.enumerated() {
                try Task.checkCancellation()
                let image: CGImage?
                if sample.frame == 0 {
                    image = first
                } else {
                    image = CGImageSourceCreateImageAtIndex(source, sample.frame, nil)
                }
                guard let image else { throw Failure.unreadable }
                while !input.isReadyForMoreMediaData {
                    guard writer.status == .writing else {
                        throw Failure.writer(String(describing: writer.error))
                    }
                    try await Task.sleep(for: .milliseconds(5))
                }
                guard let buffer = pixelBuffer(
                    drawing: image, width: plan.width, height: plan.height,
                    pool: adaptor.pixelBufferPool)
                else { throw Failure.writer("no pixel buffer") }
                if index == 0 { poster = posterJPEG(from: buffer) }
                let time = CMTime(value: CMTimeValue(tick) * ticksToUnits, timescale: timescale)
                guard adaptor.append(buffer, withPresentationTime: time) else {
                    throw Failure.writer(String(describing: writer.error))
                }
                tick += sample.ticks
            }
        } catch {
            writer.cancelWriting()
            throw error
        }
        input.markAsFinished()
        // The last frame lasts its ticks too: the session ends where the
        // animation does.
        writer.endSession(atSourceTime: CMTime(
            value: CMTimeValue(plan.totalTicks) * ticksToUnits, timescale: timescale))
        await writer.finishWriting()
        guard writer.status == .completed else {
            throw Failure.writer(String(describing: writer.error))
        }
        // A file this device cannot play back is not one to send: measured
        // on the iOS Simulator, an 8 × 8 sticker-sized animation encodes and
        // then will not decode. Rule C is the honest answer to that.
        guard try await firstFrameDecodes(output) else {
            throw Failure.writer("the written clip does not decode")
        }
        return Encoded(
            width: plan.width,
            height: plan.height,
            durationMS: plan.durationMS,
            frameCount: plan.samples.count,
            posterJPEG: poster)
    }

    /// Whether the clip's first frame decodes into pixels here.
    private static func firstFrameDecodes(_ url: URL) async throws -> Bool {
        let asset = AVURLAsset(url: url)
        guard let track = try await asset.loadTracks(withMediaType: .video).first else {
            return false
        }
        let reader = try AVAssetReader(asset: asset)
        let output = AVAssetReaderTrackOutput(track: track, outputSettings: [
            kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
        ])
        guard reader.canAdd(output) else { return false }
        reader.add(output)
        guard reader.startReading() else { return false }
        defer { reader.cancelReading() }
        while let sample = output.copyNextSampleBuffer() {
            if CMSampleBufferGetImageBuffer(sample) != nil { return reader.status != .failed }
        }
        return false
    }

    /// Timescale units per tick.
    private static var ticksToUnits: CMTimeValue {
        CMTimeValue(timescale) / CMTimeValue(AnimationTiming.ticksPerSecond)
    }

    /// One composited frame, drawn onto white at the plan's size.
    private static func pixelBuffer(
        drawing image: CGImage, width: Int, height: Int, pool: CVPixelBufferPool?
    ) -> CVPixelBuffer? {
        var created: CVPixelBuffer?
        if let pool {
            CVPixelBufferPoolCreatePixelBuffer(kCFAllocatorDefault, pool, &created)
        }
        if created == nil {
            CVPixelBufferCreate(
                kCFAllocatorDefault, width, height, kCVPixelFormatType_32BGRA,
                [kCVPixelBufferCGBitmapContextCompatibilityKey: true] as CFDictionary, &created)
        }
        guard let buffer = created else { return nil }
        CVPixelBufferLockBaseAddress(buffer, [])
        defer { CVPixelBufferUnlockBaseAddress(buffer, []) }
        guard let context = CGContext(
            data: CVPixelBufferGetBaseAddress(buffer),
            width: width,
            height: height,
            bitsPerComponent: 8,
            bytesPerRow: CVPixelBufferGetBytesPerRow(buffer),
            space: CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue
                | CGBitmapInfo.byteOrder32Little.rawValue)
        else { return nil }
        let rect = CGRect(x: 0, y: 0, width: width, height: height)
        let white = AnimationTiming.backgroundRGB.map { CGFloat($0) / 255 }
        context.setFillColor(red: white[0], green: white[1], blue: white[2], alpha: 1)
        context.fill(rect)
        context.interpolationQuality = .high
        context.draw(image, in: rect)
        return buffer
    }

    /// The first frame as the poster: the drawn buffer (already on white),
    /// no larger than a video poster ever is.
    private static func posterJPEG(from buffer: CVPixelBuffer) -> Data? {
        CVPixelBufferLockBaseAddress(buffer, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(buffer, .readOnly) }
        guard let context = CGContext(
            data: CVPixelBufferGetBaseAddress(buffer),
            width: CVPixelBufferGetWidth(buffer),
            height: CVPixelBufferGetHeight(buffer),
            bitsPerComponent: 8,
            bytesPerRow: CVPixelBufferGetBytesPerRow(buffer),
            space: CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue
                | CGBitmapInfo.byteOrder32Little.rawValue),
            let frame = context.makeImage()
        else { return nil }
        let fitted = PlatformImage.scaled(frame, maxEdge: MediaPrep.previewEdge) ?? frame
        return PlatformImage.jpegData(from: fitted, quality: MediaPrep.previewQuality)
    }
}
