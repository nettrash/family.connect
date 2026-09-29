//
//  MediaFixtures.swift
//  FamilyConnectTests
//
//  Real media, made in the test, for the "Preparing media before upload"
//  tests: clips written with AVAssetWriter, WAV and AIFF files, an AAC
//  track — and the reading-back that checks what a transcode produced.
//
//  SYNTHESISED, NOT COMMITTED. A 1080p60 clip checked into the repo would
//  be megabytes of binary nobody can review, pinned to whatever encoder
//  made it. Written here, every property the tests turn on is a parameter
//  in plain sight: the stored size, the rotation, the frame rate, the
//  container, the bitrate, the colour tags. The content is noise where the
//  source has to be BIG (noise is what an encoder cannot shrink, so a
//  12 Mbit/s request produces a 12 Mbit/s file) and a sine where it has to
//  be steady.
//
//  Both platforms: nothing here touches UIKit or AppKit.
//

import AVFoundation
import CoreMedia
import CoreVideo
import Foundation
@testable import FamilyConnect

enum MediaFixtures {

    enum FixtureError: Error {
        case writerFailed(String)
        case unsupported(String)
    }

    /// A clip to write. `width × height` is the STORED size; `transform` is
    /// the rotation a player applies, as a phone records portrait.
    struct Clip {
        var width: Int
        var height: Int
        var frameRate: Int
        var seconds: Double
        var bitrate: Int
        var fileType: AVFileType = .mp4
        var transform: CGAffineTransform = .identity
        /// Nil for a clip with no audio track at all.
        var audioChannels: Int? = 2
        /// Tag the video BT.2020 / HLG, as a recent iPhone records HDR.
        var hdrTagged = false
        /// Random pixels, so the encoder has to spend the bitrate asked of
        /// it; otherwise a gently moving gradient.
        var noise = false
    }

    /// The 90° turn a phone writes for a portrait clip stored landscape.
    static func portrait(storedWidth: Int, storedHeight: Int) -> CGAffineTransform {
        CGAffineTransform(a: 0, b: 1, c: -1, d: 0, tx: CGFloat(storedHeight), ty: 0)
    }

    static func scratch(_ ext: String) -> URL {
        FileManager.default.temporaryDirectory
            .appendingPathComponent("fc-fixture-\(UUID().uuidString)")
            .appendingPathExtension(ext)
    }

    // MARK: - Video

    /// Write `clip` to a fresh file and return where. Throws
    /// `FixtureError.unsupported` when this platform's encoder will not take
    /// what was asked (HDR tags on H.264, say), so a test can say which.
    static func write(_ clip: Clip) async throws -> URL {
        let ext = clip.fileType == .mov ? "mov" : "mp4"
        let url = scratch(ext)
        let writer = try AVAssetWriter(outputURL: url, fileType: clip.fileType)

        var videoSettings: [String: Any] = [
            AVVideoCodecKey: AVVideoCodecType.h264,
            AVVideoWidthKey: clip.width,
            AVVideoHeightKey: clip.height,
            AVVideoCompressionPropertiesKey: [
                AVVideoAverageBitRateKey: clip.bitrate,
                AVVideoExpectedSourceFrameRateKey: clip.frameRate,
                AVVideoMaxKeyFrameIntervalKey: clip.frameRate,
            ] as [String: Any],
        ]
        if clip.hdrTagged {
            videoSettings[AVVideoColorPropertiesKey] = [
                AVVideoColorPrimariesKey: AVVideoColorPrimaries_ITU_R_2020,
                AVVideoTransferFunctionKey: AVVideoTransferFunction_ITU_R_2100_HLG,
                AVVideoYCbCrMatrixKey: AVVideoYCbCrMatrix_ITU_R_2020,
            ]
        }
        guard writer.canApply(outputSettings: videoSettings, forMediaType: .video) else {
            throw FixtureError.unsupported("video settings \(clip)")
        }
        let videoInput = AVAssetWriterInput(mediaType: .video, outputSettings: videoSettings)
        videoInput.expectsMediaDataInRealTime = false
        videoInput.transform = clip.transform
        let adaptor = AVAssetWriterInputPixelBufferAdaptor(
            assetWriterInput: videoInput,
            sourcePixelBufferAttributes: [
                kCVPixelBufferPixelFormatTypeKey as String:
                    kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
                kCVPixelBufferWidthKey as String: clip.width,
                kCVPixelBufferHeightKey as String: clip.height,
            ])
        writer.add(videoInput)

        var audioInput: AVAssetWriterInput?
        if let channels = clip.audioChannels {
            let input = AVAssetWriterInput(mediaType: .audio, outputSettings: [
                AVFormatIDKey: kAudioFormatMPEG4AAC,
                AVSampleRateKey: 44_100,
                AVNumberOfChannelsKey: channels,
                AVEncoderBitRateKey: channels == 1 ? 64_000 : 128_000,
                AVEncoderBitRateStrategyKey: AVAudioBitRateStrategy_Constant,
            ])
            input.expectsMediaDataInRealTime = false
            writer.add(input)
            audioInput = input
        }

        guard writer.startWriting() else {
            throw FixtureError.writerFailed(String(describing: writer.error))
        }
        writer.startSession(atSourceTime: .zero)

        let totalFrames = Int(Double(clip.frameRate) * clip.seconds)
        let totalAudioFrames = Int(44_100 * clip.seconds)
        var frame = 0
        var audioFrame = 0
        var noise = Noise(seed: 0x5EED)
        // Interleaved by hand: the writer stops taking one track until the
        // other has caught up, so feeding them one after the other stalls.
        while frame < totalFrames || (audioInput != nil && audioFrame < totalAudioFrames) {
            var progressed = false
            if frame < totalFrames, videoInput.isReadyForMoreMediaData {
                guard let pool = adaptor.pixelBufferPool else {
                    throw FixtureError.writerFailed("no pixel buffer pool: \(String(describing: writer.error))")
                }
                let buffer = try pixelBuffer(
                    from: pool, frame: frame, noise: &noise, useNoise: clip.noise, hdr: clip.hdrTagged)
                let time = CMTime(value: CMTimeValue(frame), timescale: CMTimeScale(clip.frameRate))
                guard adaptor.append(buffer, withPresentationTime: time) else {
                    throw FixtureError.writerFailed(String(describing: writer.error))
                }
                frame += 1
                progressed = true
            }
            if let audioInput, audioFrame < totalAudioFrames, audioInput.isReadyForMoreMediaData {
                let count = min(1024, totalAudioFrames - audioFrame)
                let sample = try sineSampleBuffer(
                    frames: count, startFrame: audioFrame, sampleRate: 44_100,
                    channels: clip.audioChannels ?? 2)
                guard audioInput.append(sample) else {
                    throw FixtureError.writerFailed(String(describing: writer.error))
                }
                audioFrame += count
                progressed = true
            }
            if !progressed {
                try await Task.sleep(for: .milliseconds(1))
            }
        }
        videoInput.markAsFinished()
        audioInput?.markAsFinished()
        await writer.finishWriting()
        guard writer.status == .completed else {
            throw FixtureError.writerFailed(String(describing: writer.error))
        }
        return url
    }

    /// A tiny, fast PRNG — noise, not cryptography.
    struct Noise {
        var state: UInt64
        init(seed: UInt64) { state = seed }
        mutating func next() -> UInt64 {
            state ^= state << 13
            state ^= state >> 7
            state ^= state << 17
            return state
        }
    }

    private static func pixelBuffer(
        from pool: CVPixelBufferPool, frame: Int, noise: inout Noise, useNoise: Bool, hdr: Bool
    ) throws -> CVPixelBuffer {
        var made: CVPixelBuffer?
        guard CVPixelBufferPoolCreatePixelBuffer(nil, pool, &made) == kCVReturnSuccess,
              let buffer = made
        else {
            throw FixtureError.writerFailed("no pixel buffer")
        }
        CVPixelBufferLockBaseAddress(buffer, [])
        defer { CVPixelBufferUnlockBaseAddress(buffer, []) }
        for plane in 0..<CVPixelBufferGetPlaneCount(buffer) {
            guard let base = CVPixelBufferGetBaseAddressOfPlane(buffer, plane) else { continue }
            let rowBytes = CVPixelBufferGetBytesPerRowOfPlane(buffer, plane)
            let rows = CVPixelBufferGetHeightOfPlane(buffer, plane)
            let bytes = base.assumingMemoryBound(to: UInt8.self)
            if useNoise {
                // Eight bytes of noise at a time; the row padding gets some
                // too, which nothing reads.
                let words = (rowBytes * rows) / 8
                let wide = base.assumingMemoryBound(to: UInt64.self)
                for index in 0..<words {
                    wide[index] = noise.next()
                }
            } else {
                for row in 0..<rows {
                    let value = UInt8(truncatingIfNeeded: (row + frame * 3) % 200 + 16)
                    memset(bytes + row * rowBytes, Int32(plane == 0 ? value : 128), rowBytes)
                }
            }
        }
        if hdr {
            CVBufferSetAttachment(
                buffer, kCVImageBufferColorPrimariesKey, kCVImageBufferColorPrimaries_ITU_R_2020,
                .shouldPropagate)
            CVBufferSetAttachment(
                buffer, kCVImageBufferTransferFunctionKey,
                kCVImageBufferTransferFunction_ITU_R_2100_HLG, .shouldPropagate)
            CVBufferSetAttachment(
                buffer, kCVImageBufferYCbCrMatrixKey, kCVImageBufferYCbCrMatrix_ITU_R_2020,
                .shouldPropagate)
        }
        return buffer
    }

    // MARK: - Audio

    /// 16-bit PCM of a 440 Hz sine — the LPCM an audio writer input takes.
    private static func sineSampleBuffer(
        frames: Int, startFrame: Int, sampleRate: Int, channels: Int
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
        let bytes = sine(frames: frames, startFrame: startFrame, sampleRate: sampleRate, channels: channels)
        var block: CMBlockBuffer?
        CMBlockBufferCreateWithMemoryBlock(
            allocator: nil, memoryBlock: nil, blockLength: bytes.count, blockAllocator: nil,
            customBlockSource: nil, offsetToData: 0, dataLength: bytes.count,
            flags: kCMBlockBufferAssureMemoryNowFlag, blockBufferOut: &block)
        guard let block, let format else { throw FixtureError.writerFailed("audio buffer") }
        bytes.withUnsafeBytes { raw in
            _ = CMBlockBufferReplaceDataBytes(
                with: raw.baseAddress!, blockBuffer: block, offsetIntoDestination: 0,
                dataLength: bytes.count)
        }
        var sample: CMSampleBuffer?
        CMAudioSampleBufferCreateReadyWithPacketDescriptions(
            allocator: nil, dataBuffer: block, formatDescription: format, sampleCount: frames,
            presentationTimeStamp: CMTime(value: CMTimeValue(startFrame), timescale: CMTimeScale(sampleRate)),
            packetDescriptions: nil, sampleBufferOut: &sample)
        guard let sample else { throw FixtureError.writerFailed("audio sample") }
        return sample
    }

    /// Little-endian 16-bit interleaved samples of a 440 Hz sine.
    private static func sine(frames: Int, startFrame: Int, sampleRate: Int, channels: Int) -> Data {
        var samples = [Int16](repeating: 0, count: frames * channels)
        for index in 0..<frames {
            let phase = Double(startFrame + index) * 2 * Double.pi * 440 / Double(sampleRate)
            let value = Int16(8_000 * sin(phase))
            for channel in 0..<channels {
                samples[index * channels + channel] = value.littleEndian
            }
        }
        return samples.withUnsafeBytes { Data($0) }
    }

    /// A canonical 16-bit PCM WAV, written byte by byte — the uncompressed
    /// file the audio rules exist for.
    static func writeWAV(seconds: Double, sampleRate: Int, channels: Int) throws -> URL {
        let frames = Int(Double(sampleRate) * seconds)
        let pcm = sine(frames: frames, startFrame: 0, sampleRate: sampleRate, channels: channels)
        var data = Data()
        func append32(_ value: UInt32) { withUnsafeBytes(of: value.littleEndian) { data.append(contentsOf: $0) } }
        func append16(_ value: UInt16) { withUnsafeBytes(of: value.littleEndian) { data.append(contentsOf: $0) } }
        data.append(contentsOf: Array("RIFF".utf8))
        append32(UInt32(36 + pcm.count))
        data.append(contentsOf: Array("WAVE".utf8))
        data.append(contentsOf: Array("fmt ".utf8))
        append32(16)
        append16(1)
        append16(UInt16(channels))
        append32(UInt32(sampleRate))
        append32(UInt32(sampleRate * channels * 2))
        append16(UInt16(channels * 2))
        append16(16)
        data.append(contentsOf: Array("data".utf8))
        append32(UInt32(pcm.count))
        data.append(pcm)
        let url = scratch("wav")
        try data.write(to: url)
        return url
    }

    /// An AIFF — PCM the server does not accept as audio at all.
    static func writeAIFF(seconds: Double, sampleRate: Int, channels: Int) throws -> URL {
        let url = scratch("aiff")
        let file = try AVAudioFile(forWriting: url, settings: [
            AVFormatIDKey: kAudioFormatLinearPCM,
            AVSampleRateKey: sampleRate,
            AVNumberOfChannelsKey: channels,
            AVLinearPCMBitDepthKey: 16,
            AVLinearPCMIsBigEndianKey: true,
            AVLinearPCMIsFloatKey: false,
        ])
        try writeSine(into: file, seconds: seconds)
        return url
    }

    /// An M4A of AAC-LC at `bitrate`, constant — a track the rules keep.
    static func writeAAC(seconds: Double, bitrate: Int, channels: Int, settings: [String: Any]? = nil) throws -> URL {
        let url = scratch("m4a")
        let file = try AVAudioFile(forWriting: url, settings: settings ?? [
            AVFormatIDKey: kAudioFormatMPEG4AAC,
            AVSampleRateKey: 44_100,
            AVNumberOfChannelsKey: channels,
            AVEncoderBitRateKey: bitrate,
            AVEncoderBitRateStrategyKey: AVAudioBitRateStrategy_Constant,
        ])
        try writeSine(into: file, seconds: seconds)
        return url
    }

    private static func writeSine(into file: AVAudioFile, seconds: Double) throws {
        let format = file.processingFormat
        let frames = AVAudioFrameCount(format.sampleRate * seconds)
        guard let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: frames),
              let channels = buffer.floatChannelData
        else {
            throw FixtureError.writerFailed("pcm buffer")
        }
        buffer.frameLength = frames
        for index in 0..<Int(frames) {
            let value = Float(0.25 * sin(Double(index) * 2 * Double.pi * 440 / format.sampleRate))
            for channel in 0..<Int(format.channelCount) {
                channels[channel][index] = value
            }
        }
        try file.write(from: buffer)
    }

    // MARK: - Reading back

    /// The top-level ISO base media boxes of a file, in order — where `moov`
    /// sits relative to `mdat` is the "faststart" property.
    static func topLevelBoxes(of url: URL) throws -> [String] {
        let bytes = [UInt8](try Data(contentsOf: url))
        var names: [String] = []
        var offset = 0
        while offset + 8 <= bytes.count {
            let size32 = bytes[offset..<offset + 4].reduce(0) { ($0 << 8) | Int($1) }
            let name = String(decoding: bytes[offset + 4..<offset + 8], as: UTF8.self)
            names.append(name)
            let size: Int
            if size32 == 1, offset + 16 <= bytes.count {
                size = bytes[offset + 8..<offset + 16].reduce(0) { ($0 << 8) | Int($1) }
            } else if size32 == 0 {
                size = bytes.count - offset
            } else {
                size = size32
            }
            guard size >= 8 else { break }
            offset += size
        }
        return names
    }

    /// What a written video actually is.
    struct VideoFacts {
        var codec: FourCharCode
        /// H.264 `profile_idc` from the `avcC`: 100 High, 77 Main.
        var profile: UInt8?
        var naturalSize: CGSize
        var transform: CGAffineTransform
        var nominalFrameRate: Float
        var frameCount: Int
        /// The shortest gap between two frames' presentation times.
        var shortestFrameGap: Double
        var durationSeconds: Double
        var videoDataRate: Float
        var colorPrimaries: String?
        var transferFunction: String?
        var audioCodec: FourCharCode?
        var audioChannels: Int?
        var audioDataRate: Float?
    }

    static func facts(of url: URL) async throws -> VideoFacts {
        let asset = AVURLAsset(url: url)
        guard let track = try await asset.loadTracks(withMediaType: .video).first else {
            throw FixtureError.unsupported("no video track in \(url.lastPathComponent)")
        }
        let (natural, transform, rate, dataRate, formats) = try await track.load(
            .naturalSize, .preferredTransform, .nominalFrameRate, .estimatedDataRate,
            .formatDescriptions)
        guard let format = formats.first else { throw FixtureError.unsupported("no video format") }
        var profile: UInt8?
        if let atoms = CMFormatDescriptionGetExtension(
            format, extensionKey: kCMFormatDescriptionExtension_SampleDescriptionExtensionAtoms)
            as? [String: Any],
           let avcC = atoms["avcC"] as? Data, avcC.count > 1 {
            profile = [UInt8](avcC)[1]
        }
        let primaries = CMFormatDescriptionGetExtension(
            format, extensionKey: kCMFormatDescriptionExtension_ColorPrimaries) as? String
        let transfer = CMFormatDescriptionGetExtension(
            format, extensionKey: kCMFormatDescriptionExtension_TransferFunction) as? String

        // Presentation times, sorted: frames are stored in DECODE order,
        // which B-frames make differ from the order they are shown in.
        let reader = try AVAssetReader(asset: asset)
        let output = AVAssetReaderTrackOutput(track: track, outputSettings: nil)
        reader.add(output)
        reader.startReading()
        var times: [Double] = []
        while let sample = output.copyNextSampleBuffer() {
            if CMSampleBufferGetNumSamples(sample) > 0 {
                times.append(CMSampleBufferGetPresentationTimeStamp(sample).seconds)
            }
        }
        times.sort()
        let gaps = zip(times.dropFirst(), times).map { $0 - $1 }

        var facts = VideoFacts(
            codec: CMFormatDescriptionGetMediaSubType(format), profile: profile,
            naturalSize: natural, transform: transform, nominalFrameRate: rate,
            frameCount: times.count, shortestFrameGap: gaps.min() ?? 0,
            durationSeconds: try await asset.load(.duration).seconds, videoDataRate: dataRate,
            colorPrimaries: primaries, transferFunction: transfer,
            audioCodec: nil, audioChannels: nil, audioDataRate: nil)
        if let audio = try await asset.loadTracks(withMediaType: .audio).first {
            let (audioRate, audioFormats) = try await audio.load(.estimatedDataRate, .formatDescriptions)
            if let audioFormat = audioFormats.first {
                facts.audioCodec = CMFormatDescriptionGetMediaSubType(audioFormat)
                facts.audioChannels = MediaProbe.channels(of: audioFormat)
            }
            facts.audioDataRate = audioRate
        }
        return facts
    }

    /// What a written sound file actually is.
    struct AudioFacts {
        var codec: FourCharCode
        var channels: Int?
        var sampleRate: Double
        var dataRate: Float
        var durationSeconds: Double
    }

    static func audioFacts(of url: URL) async throws -> AudioFacts {
        let asset = AVURLAsset(url: url)
        guard let track = try await asset.loadTracks(withMediaType: .audio).first else {
            throw FixtureError.unsupported("no audio track in \(url.lastPathComponent)")
        }
        let (rate, formats) = try await track.load(.estimatedDataRate, .formatDescriptions)
        guard let format = formats.first else { throw FixtureError.unsupported("no audio format") }
        let description = CMAudioFormatDescriptionGetStreamBasicDescription(format)?.pointee
        return AudioFacts(
            codec: CMFormatDescriptionGetMediaSubType(format),
            channels: MediaProbe.channels(of: format),
            sampleRate: description?.mSampleRate ?? 0,
            dataRate: rate,
            durationSeconds: try await asset.load(.duration).seconds)
    }
}
