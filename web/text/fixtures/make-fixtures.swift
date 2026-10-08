// The media fixtures the web client's MP4 reader and transcoder are tested
// against (issue #74; docs/media-upload-2026-09-28.md, "Tests").
//
// They are made by AVFoundation on purpose: a reader tested only against its
// own writer's files proves the two agree with each other, not that either
// reads what a phone hands over. These are Apple's own muxer's output — a
// QuickTime movie with a rotation matrix and HDR colour, an MP4 with its index
// at the END, sound descriptions in QuickTime's layout — and small, because
// the content is flat colour: what is tested is the container, not the
// picture.
//
// Regenerate (macOS, from this directory):
//
//     DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer xcrun swift make-fixtures.swift
//
// Every file is rewritten; the tests read them with include_bytes!.

import AVFoundation
import CoreVideo
import Foundation
import VideoToolbox

let here = URL(fileURLWithPath: FileManager.default.currentDirectoryPath)

struct Clip {
    let name: String
    let fileType: AVFileType
    let codec: AVVideoCodecType
    let width: Int
    let height: Int
    let fps: Int
    let frames: Int
    let quarterTurns: Int
    let hdr: Bool
    let videoBitrate: Int
    let faststart: Bool
    let audioChannels: Int
    let audioRate: Double
    let audioBitrate: Int
}

/// Flat halves: the stored picture's TOP half red, its BOTTOM half blue, so a
/// test can tell which way a rotation was drawn.
func fill(_ buffer: CVPixelBuffer, hdr: Bool) {
    CVPixelBufferLockBaseAddress(buffer, [])
    defer { CVPixelBufferUnlockBaseAddress(buffer, []) }
    let height = CVPixelBufferGetHeight(buffer)
    // Y, Cb, Cr for red and blue at 75 %: BT.709 video range in 8 bits, or
    // BT.2020 video range in 10 bits (HLG), stored in the high bits of 16.
    let red: (Int, Int, Int) = hdr ? (237, 418, 848) : (51, 109, 212)
    let blue: (Int, Int, Int) = hdr ? (103, 848, 485) : (28, 212, 120)
    for plane in 0..<2 {
        let base = CVPixelBufferGetBaseAddressOfPlane(buffer, plane)!
        let stride = CVPixelBufferGetBytesPerRowOfPlane(buffer, plane)
        let rows = CVPixelBufferGetHeightOfPlane(buffer, plane)
        let columns = CVPixelBufferGetWidthOfPlane(buffer, plane)
        for row in 0..<rows {
            let top = row * height / rows < height / 2
            let colour = top ? red : blue
            let line = base.advanced(by: row * stride)
            for column in 0..<columns {
                if hdr {
                    let words = line.assumingMemoryBound(to: UInt16.self)
                    if plane == 0 {
                        words[column] = UInt16(colour.0 << 6)
                    } else {
                        words[2 * column] = UInt16(colour.1 << 6)
                        words[2 * column + 1] = UInt16(colour.2 << 6)
                    }
                } else {
                    let bytes = line.assumingMemoryBound(to: UInt8.self)
                    if plane == 0 {
                        bytes[column] = UInt8(colour.0)
                    } else {
                        bytes[2 * column] = UInt8(colour.1)
                        bytes[2 * column + 1] = UInt8(colour.2)
                    }
                }
            }
        }
    }
}

/// A 440 Hz tone, as interleaved 16-bit PCM sample buffers of 1024 frames.
func tone(rate: Double, channels: Int, frames: Int, start: Int) -> CMSampleBuffer {
    var description = AudioStreamBasicDescription(
        mSampleRate: rate, mFormatID: kAudioFormatLinearPCM,
        mFormatFlags: kLinearPCMFormatFlagIsSignedInteger | kLinearPCMFormatFlagIsPacked,
        mBytesPerPacket: UInt32(2 * channels), mFramesPerPacket: 1,
        mBytesPerFrame: UInt32(2 * channels), mChannelsPerFrame: UInt32(channels),
        mBitsPerChannel: 16, mReserved: 0)
    var format: CMAudioFormatDescription?
    CMAudioFormatDescriptionCreate(
        allocator: nil, asbd: &description, layoutSize: 0, layout: nil,
        magicCookieSize: 0, magicCookie: nil, extensions: nil, formatDescriptionOut: &format)
    var samples = [Int16](repeating: 0, count: frames * channels)
    for frame in 0..<frames {
        let value = Int16(8000 * sin(2 * Double.pi * 440 * Double(start + frame) / rate))
        for channel in 0..<channels { samples[frame * channels + channel] = value }
    }
    let length = samples.count * 2
    var block: CMBlockBuffer?
    CMBlockBufferCreateWithMemoryBlock(
        allocator: nil, memoryBlock: nil, blockLength: length, blockAllocator: nil,
        customBlockSource: nil, offsetToData: 0, dataLength: length, flags: 0,
        blockBufferOut: &block)
    samples.withUnsafeBytes { raw in
        _ = CMBlockBufferReplaceDataBytes(
            with: raw.baseAddress!, blockBuffer: block!, offsetIntoDestination: 0,
            dataLength: length)
    }
    var buffer: CMSampleBuffer?
    CMAudioSampleBufferCreateReadyWithPacketDescriptions(
        allocator: nil, dataBuffer: block!, formatDescription: format!,
        sampleCount: frames, presentationTimeStamp: CMTime(value: CMTimeValue(start), timescale: CMTimeScale(rate)),
        packetDescriptions: nil, sampleBufferOut: &buffer)
    return buffer!
}

func write(_ clip: Clip) throws {
    let url = here.appendingPathComponent(clip.name)
    try? FileManager.default.removeItem(at: url)
    let writer = try AVAssetWriter(outputURL: url, fileType: clip.fileType)
    writer.shouldOptimizeForNetworkUse = clip.faststart
    var compression: [String: Any] = [
        AVVideoAverageBitRateKey: clip.videoBitrate,
        AVVideoExpectedSourceFrameRateKey: clip.fps,
    ]
    var settings: [String: Any] = [
        AVVideoCodecKey: clip.codec,
        AVVideoWidthKey: clip.width,
        AVVideoHeightKey: clip.height,
    ]
    if clip.hdr {
        compression[AVVideoProfileLevelKey] = kVTProfileLevel_HEVC_Main10_AutoLevel as String
        settings[AVVideoColorPropertiesKey] = [
            AVVideoColorPrimariesKey: AVVideoColorPrimaries_ITU_R_2020,
            AVVideoTransferFunctionKey: AVVideoTransferFunction_ITU_R_2100_HLG,
            AVVideoYCbCrMatrixKey: AVVideoYCbCrMatrix_ITU_R_2020,
        ]
    } else if clip.codec == .h264 {
        compression[AVVideoProfileLevelKey] = AVVideoProfileLevelH264HighAutoLevel
    }
    settings[AVVideoCompressionPropertiesKey] = compression
    let video = AVAssetWriterInput(mediaType: .video, outputSettings: settings)
    video.expectsMediaDataInRealTime = false
    video.transform = CGAffineTransform(rotationAngle: CGFloat(clip.quarterTurns) * .pi / 2)
    let pixelFormat = clip.hdr
        ? kCVPixelFormatType_420YpCbCr10BiPlanarVideoRange
        : kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
    let adaptor = AVAssetWriterInputPixelBufferAdaptor(
        assetWriterInput: video,
        sourcePixelBufferAttributes: [
            kCVPixelBufferPixelFormatTypeKey as String: pixelFormat,
            kCVPixelBufferWidthKey as String: clip.width,
            kCVPixelBufferHeightKey as String: clip.height,
        ])
    writer.add(video)
    let audio = AVAssetWriterInput(
        mediaType: .audio,
        outputSettings: [
            AVFormatIDKey: kAudioFormatMPEG4AAC,
            AVSampleRateKey: clip.audioRate,
            AVNumberOfChannelsKey: clip.audioChannels,
            AVEncoderBitRateKey: clip.audioBitrate,
        ])
    audio.expectsMediaDataInRealTime = false
    writer.add(audio)
    guard writer.startWriting() else { throw writer.error! }
    writer.startSession(atSourceTime: .zero)

    let seconds = Double(clip.frames) / Double(clip.fps)
    let audioFrames = Int(seconds * clip.audioRate)
    var frame = 0
    var sample = 0
    while frame < clip.frames || sample < audioFrames {
        let videoTime = Double(frame) / Double(clip.fps)
        let audioTime = Double(sample) / clip.audioRate
        if frame < clip.frames && (sample >= audioFrames || videoTime <= audioTime) {
            while !video.isReadyForMoreMediaData { usleep(1000) }
            var buffer: CVPixelBuffer?
            CVPixelBufferPoolCreatePixelBuffer(nil, adaptor.pixelBufferPool!, &buffer)
            fill(buffer!, hdr: clip.hdr)
            // 600 ticks a second, QuickTime's own clock: 10 a frame at 60 fps.
            let time = CMTime(value: CMTimeValue(frame * 600 / clip.fps), timescale: 600)
            adaptor.append(buffer!, withPresentationTime: time)
            frame += 1
        } else {
            while !audio.isReadyForMoreMediaData { usleep(1000) }
            let count = min(1024, audioFrames - sample)
            audio.append(tone(rate: clip.audioRate, channels: clip.audioChannels, frames: count, start: sample))
            sample += count
        }
    }
    video.markAsFinished()
    audio.markAsFinished()
    let done = DispatchSemaphore(value: 0)
    writer.finishWriting { done.signal() }
    done.wait()
    if writer.status != .completed { throw writer.error! }
    print(clip.name, (try FileManager.default.attributesOfItem(atPath: url.path)[.size] as! Int), "bytes")
}

/// Half a second of sound as a plain WAV in the scratch directory — a 440 Hz
/// tone, or white noise, which an AAC encoder cannot make cheaper than the
/// rate it is asked for (a pure tone comes out at a fraction of it, and would
/// not be the "above 192 000" case it is named for).
func wav(_ name: String, rate: Double = 44_100, channels: Int, noise: Bool) throws -> URL {
    let url = FileManager.default.temporaryDirectory.appendingPathComponent(name)
    try? FileManager.default.removeItem(at: url)
    let format = AVAudioFormat(standardFormatWithSampleRate: rate, channels: AVAudioChannelCount(channels))!
    let frames = AVAudioFrameCount(rate / 2)
    let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: frames)!
    buffer.frameLength = frames
    var seed: UInt32 = 74
    for channel in 0..<channels {
        for frame in 0..<Int(frames) {
            let value: Double
            if noise {
                seed = seed &* 1_664_525 &+ 1_013_904_223
                value = 0.25 * (Double(seed >> 8) / Double(1 << 24) * 2 - 1)
            } else {
                value = 0.25 * sin(2 * Double.pi * 440 * Double(frame) / rate)
            }
            buffer.floatChannelData![channel][frame] = Float(value)
        }
    }
    do {
        let file = try AVAudioFile(
            forWriting: url,
            settings: [
                AVFormatIDKey: kAudioFormatLinearPCM, AVSampleRateKey: rate,
                AVNumberOfChannelsKey: channels, AVLinearPCMBitDepthKey: 16,
                AVLinearPCMIsFloatKey: false, AVLinearPCMIsBigEndianKey: false,
            ],
            commonFormat: .pcmFormatFloat32, interleaved: false)
        try file.write(from: buffer)
    }
    return url
}

/// `source` through Apple's own `afconvert`, as `name` here.
func convert(_ source: URL, _ name: String, _ arguments: [String]) throws {
    let url = here.appendingPathComponent(name)
    try? FileManager.default.removeItem(at: url)
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/bin/afconvert")
    process.arguments = arguments + [source.path, url.path]
    try process.run()
    process.waitUntilExit()
    guard process.terminationStatus == 0 else { fatalError("afconvert failed for \(name)") }
    print(name, (try FileManager.default.attributesOfItem(atPath: url.path)[.size] as! Int), "bytes")
}

// A phone held upright, shooting 4K at 60 in HLG: HEVC Main 10, a landscape
// track turned a quarter clockwise, stereo AAC — everything the profile has
// to undo. Half a second.
try write(Clip(
    name: "portrait-hevc-hlg-4k60.mov", fileType: .mov, codec: .hevc,
    width: 3840, height: 2160, fps: 60, frames: 30, quarterTurns: 1, hdr: true,
    videoBitrate: 20_000_000, faststart: true,
    audioChannels: 2, audioRate: 48_000, audioBitrate: 128_000))

// Already within the profile — 720p30 H.264 High, AAC — with its index at the
// END of the file, as a camera leaves it when nothing asks for faststart.
try write(Clip(
    name: "within-h264-720p30.mp4", fileType: .mp4, codec: .h264,
    width: 1280, height: 720, fps: 30, frames: 15, quarterTurns: 0, hdr: false,
    videoBitrate: 1_500_000, faststart: false,
    audioChannels: 2, audioRate: 44_100, audioBitrate: 128_000))

// The same codecs in a QuickTime movie, which is never within the profile:
// Firefox will not play the container. Mono sound, QuickTime's sound layout.
try write(Clip(
    name: "quicktime-h264-360p.mov", fileType: .mov, codec: .h264,
    width: 640, height: 360, fps: 30, frames: 15, quarterTurns: 0, hdr: false,
    videoBitrate: 500_000, faststart: true,
    audioChannels: 1, audioRate: 44_100, audioBitrate: 64_000))

// Picked sound files, each a case of the audio rules.
let tone = try wav("tone.wav", channels: 1, noise: false)
let noise = try wav("noise.wav", channels: 2, noise: true)
try convert(tone, "lossless-alac.m4a", ["-f", "m4af", "-d", "alac"])
try convert(tone, "lossless.flac", ["-f", "flac", "-d", "flac"])
try convert(tone, "uncompressed.aiff", ["-f", "AIFF", "-d", "BEI16"])
try convert(noise, "aac-256k.m4a", ["-f", "m4af", "-d", "aac", "-b", "256000"])
try convert(noise, "aac-128k.m4a", ["-f", "m4af", "-d", "aac", "-b", "128000"])
// No Ogg: afconvert reads Ogg but will not write it on this macOS, so the
// Ogg rule is tested against a header built in the tests instead.
