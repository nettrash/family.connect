//
//  MediaTranscoder.swift
//  FamilyConnect
//
//  Carrying out a transcode `MediaPlan` asked for: a video to the profile's
//  MP4 (H.264 High, AAC-LC, `moov` first, 8-bit SDR), a picked sound file to
//  an M4A of AAC-LC. docs/protocol.md, "Preparing media before upload".
//
//  WHY A READER AND A WRITER, NOT AN EXPORT SESSION. 1.1 used
//  `AVAssetExportPreset1920x1080`, and a preset is a bundle of decisions this
//  client does not get to see: its own bitrate, its own frame rate, its own
//  size. The protocol's numbers are exact — a size, a frame rate, a bitrate —
//  and `AVAssetReader` → `AVAssetWriter` is the one AVFoundation path that
//  takes all three explicitly (`AVVideoAverageBitRateKey`,
//  `AVVideoProfileLevelKey`, the render size and frame duration).
//
//  WHAT THE VIDEO COMPOSITION IS FOR. The frames are read through an
//  `AVMutableVideoComposition`, which does four jobs in one pass on the GPU:
//
//    * ROTATION, BAKED IN. A portrait phone clip is a landscape track with a
//      90° `preferredTransform`. The composition applies that turn to the
//      pixels, so the output is upright with an identity transform and its
//      stored size IS its displayed size — the TURNED size the upload
//      reports. Baking rather than carrying the matrix is deliberate: every
//      player honours a plain upright track, and a rotation matrix is one
//      more thing four platforms' players have to agree on.
//    * SCALING, to exactly the planner's even target size.
//    * SDR. Its colour properties are BT.709, and a composition asked for
//      BT.709 tone-maps HLG / Dolby Vision / PQ sources down to it, where
//      passing HDR through would show a recipient washed-out grey. The frames
//      come out 8-bit (`420v`), and the writer tags BT.709 on the track.
//    * THE FRAME-RATE CAP. Its `frameDuration` is the target's, so frames are
//      picked by presentation time on that grid — a 60 fps source gives every
//      other frame, and the output can never run faster than the target
//      (`frameDuration(for:)` rounds so that it cannot, even by a hair).
//
//  CANCELLATION IS HONOURED, and it is not a failure. A cancelled task stops
//  the reader, the writer is abandoned and this throws `CancellationError`;
//  the caller removes the half-written file. Rule C's fallback is for a
//  transcode that FAILED, not for a send somebody walked away from.
//
//  The reader and writer run on their own serial queues, never on the
//  caller's actor: `@concurrent` below says so explicitly, because this
//  target builds with NonisolatedNonsendingByDefault, where a plain
//  `nonisolated async` function would run on whoever called it — the main
//  actor, from a composer.
//

import AVFoundation
import AudioToolbox
import CoreMedia
import CoreVideo
import Foundation

nonisolated enum MediaTranscoder {

    nonisolated enum Failure: Error, Equatable {
        /// The source has no track of the kind that was to be transcoded.
        case noTrack
        /// AVFoundation will not take the settings this source needs — an
        /// encoder that refuses H.264, or an audio format no AAC-LC bitrate
        /// fits.
        case unsupportedSettings
        /// Reading the source stopped with an error.
        case readerFailed(String)
        /// Writing the result stopped with an error.
        case writerFailed(String)
    }

    // MARK: - Video

    /// Transcode the video at `source` to `target`, writing an MP4 to
    /// `output`. Throws on anything short of a complete file; the caller owns
    /// `output` either way and removes it on a throw.
    @concurrent
    static func transcodeVideo(
        from source: URL, to target: MediaPlan.VideoTarget, writingTo output: URL
    ) async throws {
        try Task.checkCancellation()
        let asset = AVURLAsset(url: source)
        guard let videoTrack = try await asset.loadTracks(withMediaType: .video).first else {
            throw Failure.noTrack
        }
        let duration = try await asset.load(.duration)
        let (natural, preferred) = try await videoTrack.load(.naturalSize, .preferredTransform)
        // The planner gives no audio target to a source with no audio track,
        // and a transcode does not invent silence.
        let audioTrack = target.audioBitrate == nil
            ? nil : try await asset.loadTracks(withMediaType: .audio).first

        try? FileManager.default.removeItem(at: output)
        let reader = try AVAssetReader(asset: asset)
        let writer = try AVAssetWriter(outputURL: output, fileType: .mp4)
        // The `moov` box before `mdat` — "faststart" — so a player can begin
        // on the first bytes of a Range read instead of fetching the tail.
        writer.shouldOptimizeForNetworkUse = true

        let renderSize = CGSize(width: target.width, height: target.height)
        let composition = AVMutableVideoComposition()
        composition.renderSize = renderSize
        composition.frameDuration = frameDuration(for: target.frameRate)
        composition.colorPrimaries = AVVideoColorPrimaries_ITU_R_709_2
        composition.colorTransferFunction = AVVideoTransferFunction_ITU_R_709_2
        composition.colorYCbCrMatrix = AVVideoYCbCrMatrix_ITU_R_709_2
        let layer = AVMutableVideoCompositionLayerInstruction(assetTrack: videoTrack)
        layer.setTransform(
            uprightTransform(natural: natural, preferred: preferred, renderSize: renderSize), at: .zero)
        let instruction = AVMutableVideoCompositionInstruction()
        instruction.timeRange = CMTimeRange(start: .zero, duration: duration)
        instruction.layerInstructions = [layer]
        composition.instructions = [instruction]

        let videoOutput = AVAssetReaderVideoCompositionOutput(
            videoTracks: [videoTrack],
            videoSettings: [
                // 8-bit 4:2:0 — what an H.264 High encoder takes, and the
                // half of "8-bit SDR" that is a pixel format.
                kCVPixelBufferPixelFormatTypeKey as String:
                    kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
            ])
        videoOutput.videoComposition = composition
        videoOutput.alwaysCopiesSampleData = false
        guard reader.canAdd(videoOutput) else { throw Failure.unsupportedSettings }
        reader.add(videoOutput)

        guard let videoSettings = h264Settings(for: target, writer: writer) else {
            throw Failure.unsupportedSettings
        }
        let videoInput = AVAssetWriterInput(mediaType: .video, outputSettings: videoSettings)
        videoInput.expectsMediaDataInRealTime = false
        guard writer.canAdd(videoInput) else { throw Failure.unsupportedSettings }
        writer.add(videoInput)

        var pumps = [Pump(output: videoOutput, input: videoInput, label: "video")]
        if let audioTrack, let bitrate = target.audioBitrate {
            pumps.append(try await audioPump(
                track: audioTrack, bitrate: bitrate, reader: reader, writer: writer))
        }
        try await Pump.run(pumps, reader: reader, writer: writer)
    }

    /// The layer transform that turns the stored frame upright and scales it
    /// onto the render size: the track's own rotation, moved back to the
    /// origin (a turn about the corner leaves the picture off-canvas), then
    /// stretched onto the target. The target is the turned size to the pixel
    /// after rounding and dropping to even, so the stretch differs between
    /// the two axes by less than one pixel's worth.
    static func uprightTransform(
        natural: CGSize, preferred: CGAffineTransform, renderSize: CGSize
    ) -> CGAffineTransform {
        let turned = CGRect(origin: .zero, size: natural).applying(preferred)
        let moved = preferred.concatenating(
            CGAffineTransform(translationX: -turned.minX, y: -turned.minY))
        guard turned.width > 0, turned.height > 0 else { return moved }
        return moved.concatenating(CGAffineTransform(
            scaleX: renderSize.width / turned.width, y: renderSize.height / turned.height))
    }

    /// One frame's duration at `frameRate`, never SHORTER than `1 ÷ frameRate`
    /// — a duration rounded down would make the output a hair faster than the
    /// cap (30.5 fps rounded to the grid is 30.5002). The timescale is fine
    /// enough to hold 30, 25, 24 and 30 000/1001 exactly.
    static func frameDuration(for frameRate: Double) -> CMTime {
        let timescale: CMTimeScale = 600_000
        let rate = frameRate.isFinite && frameRate > 0 ? frameRate : MediaPlan.maxFrameRate
        var ticks = CMTimeValue((Double(timescale) / rate).rounded())
        if Double(timescale) / Double(ticks) > rate * (1 + 1e-12) {
            ticks += 1
        }
        return CMTime(value: max(ticks, 1), timescale: timescale)
    }

    /// H.264 at the target's size and bitrate, High profile — Main only when
    /// the encoder will not take High, which is the protocol's own wording.
    private static func h264Settings(
        for target: MediaPlan.VideoTarget, writer: AVAssetWriter
    ) -> [String: Any]? {
        for profile in [AVVideoProfileLevelH264HighAutoLevel, AVVideoProfileLevelH264MainAutoLevel] {
            let settings: [String: Any] = [
                AVVideoCodecKey: AVVideoCodecType.h264,
                AVVideoWidthKey: target.width,
                AVVideoHeightKey: target.height,
                AVVideoCompressionPropertiesKey: [
                    AVVideoAverageBitRateKey: target.videoBitrate,
                    AVVideoProfileLevelKey: profile,
                    AVVideoExpectedSourceFrameRateKey: target.frameRate,
                ] as [String: Any],
                // SDR, said on the track as well as done to the pixels, so a
                // player never has to guess what the frames are.
                AVVideoColorPropertiesKey: [
                    AVVideoColorPrimariesKey: AVVideoColorPrimaries_ITU_R_709_2,
                    AVVideoTransferFunctionKey: AVVideoTransferFunction_ITU_R_709_2,
                    AVVideoYCbCrMatrixKey: AVVideoYCbCrMatrix_ITU_R_709_2,
                ],
            ]
            if writer.canApply(outputSettings: settings, forMediaType: .video) {
                return settings
            }
        }
        return nil
    }

    // MARK: - Audio alone

    /// Transcode the sound file at `source` to an M4A of AAC-LC at `bitrate`,
    /// writing it to `output`. Mono stays mono; anything else becomes stereo,
    /// which is what the planner's bitrate assumed.
    @concurrent
    static func transcodeAudio(from source: URL, bitrate: Int, writingTo output: URL) async throws {
        try Task.checkCancellation()
        let asset = AVURLAsset(url: source)
        guard let track = try await asset.loadTracks(withMediaType: .audio).first else {
            throw Failure.noTrack
        }
        try? FileManager.default.removeItem(at: output)
        let reader = try AVAssetReader(asset: asset)
        // `.m4a` writes the `M4A ` brand in an ISO base media `ftyp` — what the
        // server's magic-number check takes as `audio/mp4`.
        let writer = try AVAssetWriter(outputURL: output, fileType: .m4a)
        writer.shouldOptimizeForNetworkUse = true
        let pump = try await audioPump(track: track, bitrate: bitrate, reader: reader, writer: writer)
        try await Pump.run([pump], reader: reader, writer: writer)
    }

    // MARK: - AAC-LC

    /// The reader output and writer input for one audio track, decoded to
    /// float PCM and encoded as AAC-LC at `bitrate` — constant, so the rate
    /// the protocol names is the rate the file has. (Apple's default is
    /// variable, and on a simple signal a "128 000" request came out near
    /// 32 000: smaller, but not what four ports agreed to send.)
    private static func audioPump(
        track: AVAssetTrack, bitrate: Int, reader: AVAssetReader, writer: AVAssetWriter
    ) async throws -> Pump {
        let format = try await track.load(.formatDescriptions).first
            .flatMap { CMAudioFormatDescriptionGetStreamBasicDescription($0)?.pointee }
        let channels = format?.mChannelsPerFrame == 1 ? 1 : 2
        guard let aac = AACFormat.fitting(
            bitrate: bitrate, channels: channels, sourceSampleRate: format?.mSampleRate)
        else {
            throw Failure.unsupportedSettings
        }
        var layout = AudioChannelLayout()
        layout.mChannelLayoutTag = channels == 1
            ? kAudioChannelLayoutTag_Mono : kAudioChannelLayoutTag_Stereo
        let layoutData = withUnsafeBytes(of: &layout) { Data($0) }

        // Decoded to the rate and channel count the encoder will get, so any
        // resampling or down-mixing happens once, in the reader.
        let output = AVAssetReaderTrackOutput(track: track, outputSettings: [
            AVFormatIDKey: kAudioFormatLinearPCM,
            AVLinearPCMIsFloatKey: true,
            AVLinearPCMBitDepthKey: 32,
            AVLinearPCMIsBigEndianKey: false,
            AVLinearPCMIsNonInterleaved: false,
            AVSampleRateKey: aac.sampleRate,
            AVNumberOfChannelsKey: channels,
            AVChannelLayoutKey: layoutData,
        ])
        output.alwaysCopiesSampleData = false
        guard reader.canAdd(output) else { throw Failure.unsupportedSettings }
        reader.add(output)

        let settings: [String: Any] = [
            AVFormatIDKey: kAudioFormatMPEG4AAC,
            AVSampleRateKey: aac.sampleRate,
            AVNumberOfChannelsKey: channels,
            AVChannelLayoutKey: layoutData,
            AVEncoderBitRateKey: aac.bitrate,
            AVEncoderBitRateStrategyKey: AVAudioBitRateStrategy_Constant,
        ]
        guard writer.canApply(outputSettings: settings, forMediaType: .audio) else {
            throw Failure.unsupportedSettings
        }
        let input = AVAssetWriterInput(mediaType: .audio, outputSettings: settings)
        input.expectsMediaDataInRealTime = false
        guard writer.canAdd(input) else { throw Failure.unsupportedSettings }
        writer.add(input)
        return Pump(output: output, input: input, label: "audio")
    }

    /// A sample rate and bitrate Apple's AAC-LC encoder will actually take.
    ///
    /// It takes a RANGE of bitrates per sample rate and channel count, and
    /// refuses anything outside it outright ("The encoding parameters are not
    /// supported"): at 44.1 or 48 kHz mono runs 32 000–256 000, stereo
    /// 64 000–320 000, but a 16 kHz mono recording tops out at 48 000 — so the
    /// profile's 64 000 cannot be had at the rate the source was recorded at.
    /// Measured with `kAudioConverterApplicableEncodeBitRates`, which is also
    /// what this asks, rather than a table that a later OS could outdate.
    nonisolated struct AACFormat: Equatable {
        let sampleRate: Double
        let bitrate: Int

        /// The rates AAC-LC is defined at, highest first.
        static let sampleRates: [Double] = [
            48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000,
        ]

        /// The planner's `bitrate` exactly, at the source's own rate where
        /// the encoder takes it there — otherwise at the nearest rate that
        /// does, upward first (resampling up costs no quality; a lower
        /// bitrate would). Only when no rate takes it exactly is the bitrate
        /// lowered to the nearest the encoder accepts BELOW it: never above,
        /// which is rule B.
        static func fitting(bitrate: Int, channels: Int, sourceSampleRate: Double?) -> AACFormat? {
            let source = sourceSampleRate.flatMap { $0 > 0 ? $0 : nil } ?? 44_100
            let own = sampleRates.first { $0 <= source } ?? sampleRates[sampleRates.count - 1]
            let higher = sampleRates.filter { $0 > own }.reversed()
            let lower = sampleRates.filter { $0 < own }
            let order = [own] + higher + lower
            for rate in order {
                let accepted = applicableBitrates(sampleRate: rate, channels: channels)
                if let low = accepted.min(), let high = accepted.max(), (low...high).contains(bitrate) {
                    return AACFormat(sampleRate: rate, bitrate: bitrate)
                }
            }
            for rate in order {
                if let below = applicableBitrates(sampleRate: rate, channels: channels)
                    .filter({ $0 <= bitrate }).max() {
                    return AACFormat(sampleRate: rate, bitrate: below)
                }
            }
            return nil
        }

        /// What the system's AAC-LC encoder lists for this rate and channel
        /// count. Empty when it will not encode that combination at all.
        static func applicableBitrates(sampleRate: Double, channels: Int) -> [Int] {
            let count = UInt32(channels)
            var input = AudioStreamBasicDescription(
                mSampleRate: sampleRate, mFormatID: kAudioFormatLinearPCM,
                mFormatFlags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked,
                mBytesPerPacket: 4 * count, mFramesPerPacket: 1, mBytesPerFrame: 4 * count,
                mChannelsPerFrame: count, mBitsPerChannel: 32, mReserved: 0)
            var output = AudioStreamBasicDescription(
                mSampleRate: sampleRate, mFormatID: kAudioFormatMPEG4AAC, mFormatFlags: 0,
                mBytesPerPacket: 0, mFramesPerPacket: 1024, mBytesPerFrame: 0,
                mChannelsPerFrame: count, mBitsPerChannel: 0, mReserved: 0)
            var converter: AudioConverterRef?
            guard AudioConverterNew(&input, &output, &converter) == noErr, let converter else {
                return []
            }
            defer { AudioConverterDispose(converter) }
            var size: UInt32 = 0
            guard AudioConverterGetPropertyInfo(
                converter, kAudioConverterApplicableEncodeBitRates, &size, nil) == noErr, size > 0
            else {
                return []
            }
            var ranges = [AudioValueRange](
                repeating: AudioValueRange(), count: Int(size) / MemoryLayout<AudioValueRange>.size)
            guard AudioConverterGetProperty(
                converter, kAudioConverterApplicableEncodeBitRates, &size, &ranges) == noErr
            else {
                return []
            }
            // The list comes back padded with empty ranges.
            return ranges.filter { $0.mMaximum > 0 }.map { Int($0.mMinimum) }
        }
    }

    // MARK: - Moving samples

    /// One reader output feeding one writer input, on its own queue. A video
    /// and its audio each need one: the writer interleaves the two, and it
    /// stops asking for one until the other catches up, so they must be
    /// served independently or the pair can wait on each other for ever.
    nonisolated final class Pump: @unchecked Sendable {
        private let output: AVAssetReaderOutput
        private let input: AVAssetWriterInput
        private let queue: DispatchQueue
        /// Touched only on `queue`.
        private var finished = false

        init(output: AVAssetReaderOutput, input: AVAssetWriterInput, label: String) {
            self.output = output
            self.input = input
            self.queue = DispatchQueue(label: "me.nettrash.FamilyConnect.transcode.\(label)")
        }

        /// Move samples until the reader runs dry — finished, failed or
        /// cancelled, which `run` tells apart afterwards — or the writer
        /// refuses one.
        func start(whenDone done: @escaping @Sendable () -> Void) {
            input.requestMediaDataWhenReady(on: queue) { [self] in
                guard !finished else { return }
                while input.isReadyForMoreMediaData {
                    guard let sample = output.copyNextSampleBuffer(), input.append(sample) else {
                        finished = true
                        input.markAsFinished()
                        done()
                        return
                    }
                }
            }
        }

        /// Start the reader and writer, run every pump to the end, and finish
        /// the file — or throw, having abandoned it.
        static func run(_ pumps: [Pump], reader: AVAssetReader, writer: AVAssetWriter) async throws {
            let session = Session(reader: reader, writer: writer)
            try await withTaskCancellationHandler {
                try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
                    guard session.start() else {
                        continuation.resume(throwing: session.failure())
                        return
                    }
                    let group = DispatchGroup()
                    for pump in pumps {
                        group.enter()
                        pump.start { group.leave() }
                    }
                    group.notify(queue: .global(qos: .userInitiated)) {
                        guard session.reader.status == .completed, session.writer.status == .writing else {
                            session.abandon()
                            continuation.resume(throwing: session.failure())
                            return
                        }
                        session.writer.finishWriting {
                            if session.writer.status == .completed {
                                continuation.resume()
                            } else {
                                continuation.resume(throwing: session.failure())
                            }
                        }
                    }
                }
            } onCancel: {
                session.cancel()
            }
        }
    }

    /// The reader and writer of one transcode, and whether it was cancelled —
    /// behind a lock, because cancellation arrives on whatever thread
    /// cancelled the task, possibly before the reader has even started.
    private nonisolated final class Session: @unchecked Sendable {
        let reader: AVAssetReader
        let writer: AVAssetWriter
        private let lock = NSLock()
        private var cancelled = false
        private var started = false

        init(reader: AVAssetReader, writer: AVAssetWriter) {
            self.reader = reader
            self.writer = writer
        }

        /// Start both, unless the task was cancelled first. A reader that
        /// never starts cannot be stopped by `cancelReading`, so the check
        /// and the start happen under the same lock the cancel takes.
        func start() -> Bool {
            lock.lock()
            defer { lock.unlock() }
            guard !cancelled, reader.startReading() else { return false }
            guard writer.startWriting() else {
                reader.cancelReading()
                return false
            }
            writer.startSession(atSourceTime: .zero)
            started = true
            return true
        }

        func cancel() {
            lock.lock()
            defer { lock.unlock() }
            cancelled = true
            // Stopping the reader is enough: every pump then runs dry, and
            // `run` abandons the writer once they have.
            if started { reader.cancelReading() }
        }

        /// Drop a half-written file's writer. Only one still writing can be
        /// cancelled; a failed one has already stopped.
        func abandon() {
            if writer.status == .writing { writer.cancelWriting() }
        }

        /// Why it stopped. Cancellation is the TASK's, read from the flag and
        /// not from the reader: a writer that would not start cancels the
        /// reader too, and that is a failure, not somebody walking away.
        func failure() -> Error {
            lock.lock()
            let wasCancelled = cancelled
            lock.unlock()
            if wasCancelled {
                return CancellationError()
            }
            if reader.status == .failed {
                return Failure.readerFailed(reader.error.map { String(describing: $0) } ?? "unknown")
            }
            return Failure.writerFailed(writer.error.map { String(describing: $0) } ?? "unknown")
        }
    }
}
