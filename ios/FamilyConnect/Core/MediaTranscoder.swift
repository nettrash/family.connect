//
//  MediaTranscoder.swift
//  FamilyConnect
//
//  Carrying out a transcode `MediaPlan` asked for: a video to the profile's
//  MP4 (H.264 High, AAC-LC, `moov` first, 8-bit SDR), a picked sound file to
//  an M4A of AAC-LC. docs/protocol.md, "Preparing media before upload".
//  And one job that is not a send: a file's SOUND TRACK, taken out as an
//  M4A for the text of a recording (`extractSound`, "Transcripts on
//  request") — the same reader, writer and pumps, with no picture read.
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
//  THE BITRATE IS ASKED FOR TWICE, because asking once is not enough.
//  `AVVideoAverageBitRateKey` is an average the encoder steers towards over
//  a window far longer than a chat clip: measured on an M4's hardware
//  encoder, two seconds asked for 2 000 000 bit/s came out at 2 540 000.
//  So the settings also carry a hard ceiling (`DataRateLimits`) of one and
//  a half times the target in any one second, which brought the same clip
//  to 2 170 000 — inside rule A's own tolerance of 1.25 ×, so that a clip
//  this client produced is one every client then leaves alone.
//
//  WHAT NO SETTING CAN DO is make an encoder hit a bitrate the picture
//  does not fit in. H.264's coarsest quantiser still spends about fifty
//  megabits a second on 720p of white noise, on the hardware encoder and
//  the software one alike, ceiling or no ceiling — which is how this
//  file's first tests, fed synthetic noise, measured 48 591 064 bit/s on a
//  CI runner for a 2 000 000 request. Nothing a camera records is like
//  that; and if something ever is, rule D (in `MediaPrep`) throws away a
//  result that came out bigger than its source.
//
//  A VIDEO'S AUDIO IS ENCODED — OR, RARELY, COPIED. The planner caps the
//  audio target at the source's own rate, and Apple's AAC encoder has a
//  floor the protocol does not: a silent track states less than it can be
//  asked for. Such a track, when it is already AAC, is passed through
//  rather than costing the clip its whole transcode (`audioTreatment`).
//
//  CANCELLATION IS HONOURED, and it is not a failure. A cancelled task stops
//  the pumps between two samples, the reader and writer are abandoned and
//  this throws `CancellationError`; the half-written file is removed. Rule
//  C's fallback is for a transcode that FAILED, not for a send somebody
//  walked away from.
//
//  IT ENDS EXACTLY ONCE, WHATEVER HAPPENS (`Session`). A transcode that
//  never returned would be the one thing rule C forbids — a send that used
//  to work and now sits at "Preparing…" for ever — so ending does not
//  depend on AVFoundation calling every pump back one last time: the first
//  refusal, or the cancel, settles it there and then.
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
import VideoToolbox

nonisolated enum MediaTranscoder {

    nonisolated enum Failure: Error, Equatable {
        /// The source has no track of the kind that was to be transcoded.
        case noTrack
        /// AVFoundation will not take the settings this source needs — an
        /// encoder that refuses H.264, or an audio track no AAC-LC bitrate
        /// fits and that cannot be passed through either (`audioTreatment`).
        case unsupportedSettings
        /// Reading the source stopped with an error.
        case readerFailed(String)
        /// Writing the result stopped with an error.
        case writerFailed(String)
    }

    /// Which H.264 encoder does the work.
    ///
    /// The app never chooses: the system picks the hardware encoder wherever
    /// there is one. `softwareOnly` exists for the tests, and for a reason
    /// that cost a red CI run to learn — a hosted runner is a virtual machine
    /// with NO hardware encoder, so the code it exercises is a path a
    /// developer's Mac never takes by itself. Asking for it by name lets the
    /// Mac take it too. macOS only: iOS has no way to ask, and ignores it.
    nonisolated enum Encoder: Sendable {
        case systemChoice
        case softwareOnly
    }

    // MARK: - Video

    /// Transcode the video at `source` to `target`, writing an MP4 to
    /// `output`. Throws on anything short of a complete file; the caller owns
    /// `output` either way and removes it on a throw.
    @concurrent
    static func transcodeVideo(
        from source: URL, to target: MediaPlan.VideoTarget, writingTo output: URL,
        encoder: Encoder = .systemChoice
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

        guard let videoSettings = h264Settings(for: target, writer: writer, encoder: encoder) else {
            throw Failure.unsupportedSettings
        }
        let videoInput = AVAssetWriterInput(mediaType: .video, outputSettings: videoSettings)
        videoInput.expectsMediaDataInRealTime = false
        guard writer.canAdd(videoInput) else { throw Failure.unsupportedSettings }
        writer.add(videoInput)

        var pumps = [Pump(output: videoOutput, input: videoInput, label: "video")]
        if let audioTrack, let bitrate = target.audioBitrate {
            pumps.append(try await audioPump(
                track: audioTrack, bitrate: bitrate, reader: reader, writer: writer,
                mayPassThrough: true))
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

    /// How far over the target any one second of video may run: the hard
    /// ceiling that goes with the average (see the top of this file).
    static let dataRateCeiling = (numerator: 3, denominator: 2)

    /// H.264 at the target's size and bitrate, High profile — Main only when
    /// the encoder will not take High, which is the protocol's own wording.
    ///
    /// A LADDER, best first, and the writer is asked about each rung before
    /// it is used: a compression property an encoder does not know is not
    /// ignored, it is an exception from `AVAssetWriterInput`'s initialiser.
    /// So the data-rate ceiling is dropped before the transcode is — a clip
    /// encoded to the average alone is still the profile, where no clip at
    /// all would be rule C and the original's megabytes.
    static func h264Settings(
        for target: MediaPlan.VideoTarget, writer: AVAssetWriter, encoder: Encoder = .systemChoice
    ) -> [String: Any]? {
        let profiles = [AVVideoProfileLevelH264HighAutoLevel, AVVideoProfileLevelH264MainAutoLevel]
        for capped in [true, false] {
            for profile in profiles {
                var compression: [String: Any] = [
                    AVVideoAverageBitRateKey: target.videoBitrate,
                    AVVideoProfileLevelKey: profile,
                    AVVideoExpectedSourceFrameRateKey: target.frameRate,
                ]
                if capped {
                    // Bytes, then the seconds they are counted over.
                    let bytes = target.videoBitrate * dataRateCeiling.numerator
                        / dataRateCeiling.denominator / 8
                    compression[kVTCompressionPropertyKey_DataRateLimits as String] = [bytes, 1]
                }
                var settings: [String: Any] = [
                    AVVideoCodecKey: AVVideoCodecType.h264,
                    AVVideoWidthKey: target.width,
                    AVVideoHeightKey: target.height,
                    AVVideoCompressionPropertiesKey: compression,
                    // SDR, said on the track as well as done to the pixels, so a
                    // player never has to guess what the frames are.
                    AVVideoColorPropertiesKey: [
                        AVVideoColorPrimariesKey: AVVideoColorPrimaries_ITU_R_709_2,
                        AVVideoTransferFunctionKey: AVVideoTransferFunction_ITU_R_709_2,
                        AVVideoYCbCrMatrixKey: AVVideoYCbCrMatrix_ITU_R_709_2,
                    ],
                ]
                #if os(macOS)
                if encoder == .softwareOnly {
                    settings[AVVideoEncoderSpecificationKey] = [
                        kVTVideoEncoderSpecification_EnableHardwareAcceleratedVideoEncoder as String: false,
                    ]
                }
                #endif
                if writer.canApply(outputSettings: settings, forMediaType: .video) {
                    return settings
                }
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

    // MARK: - A sound track, for a transcript

    /// Take the sound track out of the video or sound file at `source` and
    /// write it to `output` as an M4A — the multipart `audio` part of a
    /// transcript request (docs/protocol.md, "Transcripts on request").
    ///
    /// ONLY THE AUDIO TRACK IS READ. The reader is given one output, for
    /// the first audio track, so no picture is ever decoded — a 90 MB
    /// video costs the reading of its sound and nothing more.
    ///
    /// `.passThrough` copies an AAC track's own samples into the new
    /// container, untouched; `.reencode` decodes it and encodes 64 kbit/s
    /// MONO AAC-LC — mono because a speech model hears one voice as well
    /// from one channel, and at that rate about fifty minutes fit the
    /// protocol's 25 MiB. `TranscriptSound.treatment` decides which.
    @concurrent
    static func extractSound(
        from source: URL, treatment: TranscriptSound.Treatment, writingTo output: URL
    ) async throws {
        try Task.checkCancellation()
        let asset = AVURLAsset(url: source)
        guard let track = try await asset.loadTracks(withMediaType: .audio).first else {
            throw Failure.noTrack
        }
        try? FileManager.default.removeItem(at: output)
        let reader = try AVAssetReader(asset: asset)
        // `.m4a`: the `M4A ` brand in an ISO base media `ftyp`, which is what
        // the server checks the part for.
        let writer = try AVAssetWriter(outputURL: output, fileType: .m4a)
        writer.shouldOptimizeForNetworkUse = true
        let pump: Pump
        switch treatment {
        case .passThrough:
            let description = try await track.load(.formatDescriptions).first
            pump = try passThroughPump(
                track: track, description: description, reader: reader, writer: writer)
        case .reencode:
            let description = try await track.load(.formatDescriptions).first
            let sampleRate = description
                .flatMap { CMAudioFormatDescriptionGetStreamBasicDescription($0)?.pointee }?
                .mSampleRate
            guard let aac = AACFormat.fitting(
                bitrate: TranscriptSound.bitrate, channels: 1, sourceSampleRate: sampleRate)
            else {
                throw Failure.unsupportedSettings
            }
            pump = try encodingPump(
                track: track, aac: aac, channels: 1, reader: reader, writer: writer)
        }
        try await Pump.run([pump], reader: reader, writer: writer)
    }

    // MARK: - AAC-LC

    /// What becomes of one audio track.
    nonisolated enum AudioTreatment: Equatable {
        /// Decoded, and encoded again as AAC-LC in this format.
        case encode(AACFormat)
        /// The track's own samples, copied as they are.
        case passThrough
    }

    /// What the planner was told about a video's audio track — read the way
    /// `MediaProbe` read it, so the two cannot disagree about one file.
    nonisolated struct SourceAudio: Equatable {
        /// AAC in any profile, as rule A means it.
        var isAAC: Bool
        var channels: Int?
        /// The rate the container states, or nil when it states none.
        var bitrate: Int?
    }

    /// Encode a video's audio track, or hand it back as it was read.
    ///
    /// THE PLANNER'S NUMBER CAN BE ONE NO ENCODER TAKES. The audio target is
    /// capped at the source's own rate (rule B), and a source's rate has no
    /// floor: a screen recording with the microphone off carries a stereo
    /// AAC track that states about 2 000 bit/s, and Apple's AAC-LC encoder
    /// starts at 16 000 for stereo, and only at 8 kHz. Rule B forbids
    /// raising it to reach that floor, so there is nothing to ENCODE with —
    /// and giving up on the whole clip over it, which is what this did
    /// first, sent a 4K original because its soundtrack was too QUIET.
    ///
    /// So, in this order:
    ///
    ///   1. the planner's exact bitrate, at the source's sample rate or a
    ///      higher one — the protocol as written;
    ///   2. otherwise, when the track is ALREADY AAC, mono or stereo, and
    ///      its stated rate is the very number asked for — which it is
    ///      exactly when the cap was the source itself, at or under the
    ///      profile's row — the track is passed through. Nothing is raised
    ///      and nothing is re-encoded; the track was within the profile to
    ///      begin with, and a second generation at its own rate could only
    ///      cost it quality. The same answer Windows reached for the rates
    ///      Media Foundation's encoder refuses (`MediaEncoding.KeepsAudio`);
    ///   3. otherwise whatever the encoder does take — a lower sample rate,
    ///      then a lower bitrate, never a higher one (`AACFormat.fitting`);
    ///   4. and nil when even that is nothing: a track that is NOT AAC and
    ///      states less than the encoder's floor. That one is still a
    ///      failed transcode and rule C, because the protocol names no
    ///      floor and no other way out.
    ///
    /// `source` is nil for a sound file picked on its own: the audio rules
    /// only ever re-encode what is lossless or above 192 000, and neither
    /// is something to pass through.
    static func audioTreatment(
        bitrate: Int, channels: Int, sourceSampleRate: Double?, source: SourceAudio?
    ) -> AudioTreatment? {
        let fitted = AACFormat.fitting(
            bitrate: bitrate, channels: channels, sourceSampleRate: sourceSampleRate)
        if let fitted, fitted.bitrate == bitrate,
           fitted.sampleRate >= AACFormat.ownSampleRate(sourceSampleRate) {
            return .encode(fitted)
        }
        if let source, source.isAAC, source.channels == 1 || source.channels == 2,
           source.bitrate == bitrate {
            return .passThrough
        }
        return fitted.map(AudioTreatment.encode)
    }

    /// The reader output and writer input for one audio track: decoded to
    /// float PCM and encoded as AAC-LC at `bitrate` — constant, so the rate
    /// the protocol names is the rate the file has. (Apple's default is
    /// variable, and on a simple signal a "128 000" request came out near
    /// 32 000: smaller, but not what four ports agreed to send.) Or, for a
    /// video's track that no encode fits, copied (`audioTreatment`).
    static func audioPump(
        track: AVAssetTrack, bitrate: Int, reader: AVAssetReader, writer: AVAssetWriter,
        mayPassThrough: Bool = false
    ) async throws -> Pump {
        let description = try await track.load(.formatDescriptions).first
        let format = description
            .flatMap { CMAudioFormatDescriptionGetStreamBasicDescription($0)?.pointee }
        let channels = format?.mChannelsPerFrame == 1 ? 1 : 2
        var source: SourceAudio?
        if mayPassThrough, let description {
            source = SourceAudio(
                isAAC: MediaProbe.audioCodec(CMFormatDescriptionGetMediaSubType(description)) == "aac",
                channels: MediaProbe.channels(of: description),
                bitrate: MediaProbe.bitrate((try? await track.load(.estimatedDataRate)) ?? 0))
        }
        switch audioTreatment(
            bitrate: bitrate, channels: channels, sourceSampleRate: format?.mSampleRate, source: source) {
        case .encode(let aac):
            return try encodingPump(
                track: track, aac: aac, channels: channels, reader: reader, writer: writer)
        case .passThrough:
            return try passThroughPump(
                track: track, description: description, reader: reader, writer: writer)
        case nil:
            throw Failure.unsupportedSettings
        }
    }

    /// The track's own compressed samples, from the reader straight to the
    /// writer: no output settings on either end, and the source's format
    /// handed over as the hint an MP4 writer needs to write the track's
    /// description before it has seen a sample.
    static func passThroughPump(
        track: AVAssetTrack, description: CMFormatDescription?, reader: AVAssetReader,
        writer: AVAssetWriter
    ) throws -> Pump {
        let output = AVAssetReaderTrackOutput(track: track, outputSettings: nil)
        output.alwaysCopiesSampleData = false
        guard reader.canAdd(output) else { throw Failure.unsupportedSettings }
        reader.add(output)
        let input = AVAssetWriterInput(
            mediaType: .audio, outputSettings: nil, sourceFormatHint: description)
        input.expectsMediaDataInRealTime = false
        guard writer.canAdd(input) else { throw Failure.unsupportedSettings }
        writer.add(input)
        return Pump(output: output, input: input, label: "audio")
    }

    static func encodingPump(
        track: AVAssetTrack, aac: AACFormat, channels: Int, reader: AVAssetReader,
        writer: AVAssetWriter
    ) throws -> Pump {
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
            let own = ownSampleRate(sourceSampleRate)
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

        /// The AAC-LC rate a source at `sourceSampleRate` is encoded at when
        /// nothing forces another: its own, or the next one down for a rate
        /// AAC-LC is not defined at — 44.1 kHz when the source does not say.
        static func ownSampleRate(_ sourceSampleRate: Double?) -> Double {
            let source = sourceSampleRate.flatMap { $0 > 0 ? $0 : nil } ?? 44_100
            return sampleRates.first { $0 <= source } ?? sampleRates[sampleRates.count - 1]
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
    ///
    /// And each input is marked finished THE MOMENT its own samples run
    /// out, not when both have: a writer that has not been told one track
    /// is over keeps waiting for the samples its encoder is still holding,
    /// and will not take the other track past them. (The tests' own clip
    /// writer did it the other way, and hung.)
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

        /// Move samples until the reader runs dry, the writer refuses one,
        /// or the session has been settled by somebody else — and tell the
        /// session which.
        ///
        /// ASKED FOR ON THE PUMP'S OWN QUEUE, like `stop`, and that is a
        /// crash fixed rather than a preference. A cancel — or the other
        /// pump's first sample being refused — can land between the writer
        /// starting and this pump being started: the session is settled,
        /// `stop` marks this input finished, and asking a FINISHED input for
        /// its media data is not an error AVFoundation returns but an
        /// NSInternalInconsistencyException ("Cannot call method when status
        /// is 2") that no Swift `catch` sees and that takes the process
        /// with it. On one queue the two are in an order, and a pump that
        /// was stopped first is simply never started.
        fileprivate func start(in session: Session) {
            queue.async { [self] in
                guard !finished else { return }
                input.requestMediaDataWhenReady(on: queue) { [self] in
                    guard !finished else { return }
                    while input.isReadyForMoreMediaData {
                        // Cancelled, or the other track failed: there is no
                        // file to finish. The input is still marked finished —
                        // one left open and ready is called back in a loop.
                        if session.isSettled {
                            finished = true
                            input.markAsFinished()
                            return
                        }
                        guard let sample = output.copyNextSampleBuffer() else {
                            finished = true
                            input.markAsFinished()
                            session.pumpRanDry()
                            return
                        }
                        guard input.append(sample) else {
                            finished = true
                            input.markAsFinished()
                            session.fail()
                            return
                        }
                    }
                }
            }
        }

        /// Stop for good, from outside — on this pump's OWN queue, so that it
        /// lands between two appends and never beside one. `done` runs once
        /// this pump can no longer be inside the writer.
        fileprivate func stop(then done: @escaping @Sendable () -> Void) {
            queue.async { [self] in
                if !finished {
                    finished = true
                    input.markAsFinished()
                }
                done()
            }
        }

        /// Start the reader and writer, run every pump to the end, and finish
        /// the file — or throw, having abandoned it.
        ///
        /// `betweenStartAndPumps` is the tests' and nobody else's: it runs
        /// in the one window a cancel is hardest to aim at — the writer
        /// started, no pump yet — which a cancel from outside hit about
        /// never on a fast Mac and would hit on a slow runner (see `start`).
        static func run(
            _ pumps: [Pump], reader: AVAssetReader, writer: AVAssetWriter,
            betweenStartAndPumps: (() -> Void)? = nil
        ) async throws {
            let session = Session(reader: reader, writer: writer, pumps: pumps)
            try await withTaskCancellationHandler {
                try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
                    guard session.start(continuation) else {
                        continuation.resume(throwing: session.failure())
                        return
                    }
                    betweenStartAndPumps?()
                    for pump in pumps {
                        pump.start(in: session)
                    }
                }
            } onCancel: {
                session.cancel()
            }
        }
    }

    /// The reader and writer of one transcode, and the one continuation
    /// waiting on them — behind a lock, because its three endings arrive on
    /// three different threads: the last pump running dry (a pump's queue),
    /// a refused sample (the other pump's), and cancellation (whatever
    /// thread cancelled the task, possibly before the reader has started).
    ///
    /// SETTLED ONCE. Whoever takes the continuation out ends the transcode;
    /// everybody after finds it gone and does nothing. That is what makes a
    /// failure prompt: the first refusal throws there and then, rather than
    /// waiting for a second pump that a stopped writer may never call again.
    fileprivate nonisolated final class Session: @unchecked Sendable {
        private let reader: AVAssetReader
        private let writer: AVAssetWriter
        private let lock = NSLock()
        private var cancelled = false
        /// Every pump, until the transcode is over — kept to stop them.
        private var pumps: [Pump]
        /// Pumps still moving samples.
        private var running: Int
        /// Nil before the start and again once settled.
        private var continuation: CheckedContinuation<Void, Error>?
        private var settled = false

        init(reader: AVAssetReader, writer: AVAssetWriter, pumps: [Pump]) {
            self.reader = reader
            self.writer = writer
            self.pumps = pumps
            self.running = pumps.count
        }

        /// Start both, unless the task was cancelled first — the check and
        /// the start under the same lock the cancel takes, so a cancel lands
        /// either wholly before (nothing starts) or wholly after (there is
        /// a continuation to settle). On false the caller still holds the
        /// continuation, and resumes it.
        func start(_ continuation: CheckedContinuation<Void, Error>) -> Bool {
            lock.lock()
            defer { lock.unlock() }
            guard !cancelled, reader.startReading() else {
                settled = true
                return false
            }
            guard writer.startWriting() else {
                reader.cancelReading()
                settled = true
                return false
            }
            writer.startSession(atSourceTime: .zero)
            self.continuation = continuation
            return true
        }

        var isSettled: Bool {
            lock.lock()
            defer { lock.unlock() }
            return settled
        }

        /// One pump has moved its last sample. When it was the last pump,
        /// the file is finished — provided the reader really reached the
        /// end, rather than stopping on an error that also reads as "no
        /// more samples".
        func pumpRanDry() {
            lock.lock()
            guard !settled else {
                lock.unlock()
                return
            }
            running -= 1
            // A reader that FAILED hands every output "no more samples" too.
            // Said now, not once the other pump has noticed.
            if reader.status == .failed {
                lock.unlock()
                fail()
                return
            }
            guard running == 0 else {
                lock.unlock()
                return
            }
            guard reader.status == .completed, writer.status == .writing else {
                lock.unlock()
                fail()
                return
            }
            settled = true
            let continuation = self.continuation
            self.continuation = nil
            pumps = []
            lock.unlock()
            writer.finishWriting { [self] in
                if writer.status == .completed {
                    continuation?.resume()
                } else {
                    continuation?.resume(throwing: failure())
                }
            }
        }

        /// The writer refused a sample, or the reader stopped short.
        func fail() {
            settle()
        }

        /// The task was cancelled — before the start, during, or after the
        /// end, where there is nothing left to do.
        func cancel() {
            lock.lock()
            cancelled = true
            lock.unlock()
            settle()
        }

        /// Take the continuation, stop both ends and throw why.
        ///
        /// THE ORDER IS LOAD-BEARING, and it was learned from two crashes.
        /// Neither end may be stopped beside a pump that is inside it:
        /// `cancelWriting` beside an `append`, and `cancelReading` beside a
        /// `copyNextSampleBuffer`, are both segmentation faults deep in
        /// MediaToolbox. The test that cancels a transcode mid-way found
        /// the first two runs in three and the second once in a couple of
        /// dozen — and the second is what cancelling did from the day this
        /// file was written, on whatever thread the task was cancelled from.
        ///
        /// So nothing is stopped from here. Each pump is stopped on its own
        /// serial queue, which puts the stop between two samples rather
        /// than beside one; and only when every pump has answered — so none
        /// can be inside the reader or the writer — are the two cancelled
        /// and the caller told. That waits for at most the one sample each
        /// pump had in hand, and for nothing AVFoundation has to remember
        /// to do.
        private func settle() {
            lock.lock()
            guard !settled, let continuation else {
                lock.unlock()
                return
            }
            settled = true
            self.continuation = nil
            let pumps = self.pumps
            self.pumps = []
            lock.unlock()
            // Read before the two are stopped: stopping them overwrites the
            // very status that says what went wrong.
            let error = failure()
            let stopped = DispatchGroup()
            for pump in pumps {
                stopped.enter()
                pump.stop { stopped.leave() }
            }
            stopped.notify(queue: .global(qos: .userInitiated)) { [self] in
                if reader.status == .reading { reader.cancelReading() }
                // Only a writer still writing can be cancelled; a failed one
                // has already stopped. Cancelling removes the half-written file.
                if writer.status == .writing { writer.cancelWriting() }
                continuation.resume(throwing: error)
            }
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
