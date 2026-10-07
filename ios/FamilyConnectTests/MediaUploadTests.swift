//
//  MediaUploadTests.swift
//  FamilyConnectTests
//
//  Issue #74 end to end, on real media: docs/protocol.md's "Preparing media
//  before upload", as this client carries it out.
//
//  MediaPlanVectorTests proves the DECISION matches every other port. These
//  prove the part no vector can reach: that AVFoundation reads a file the
//  way the planner needs it read (the displayed size of a rotated clip, the
//  QuickTime container of a camera clip), and that what comes out of the
//  transcoder really IS the profile — H.264 High, 8-bit SDR, the turned
//  size, no more than 30 frames a second, the bitrate, `moov` before `mdat`,
//  and smaller than what went in. Every clip is synthesised in the test
//  (MediaFixtures), so each property the rules turn on is visible here.
//
//  Rules C and D cannot be reached on cue with a working encoder, so those
//  tests hand `MediaPrep` a transcoder that fails, or one whose result is
//  bigger than its source — the `Transcoder` seam, like `APIClient`'s
//  injected `URLSession`.
//
//  Serialized: every test here encodes video, and a dozen encoders at once
//  measure the machine rather than the code.
//
//  BOTH ENCODERS, on the Mac. A hosted CI runner is a virtual machine with
//  no hardware encoder, so there the whole suite runs on the software one —
//  a path a developer's Mac never takes unless it is asked to. The tests
//  that turn on what the encoder DOES (the bitrate it hits, the file it
//  writes) therefore run once with the system's choice and once with the
//  software encoder named, so that what CI will see is seen here first.
//  The time limits are a backstop against a hang, not a measure of speed:
//  the slowest of these takes a few seconds on a loaded machine.
//

import AVFoundation
import CoreMedia
import Foundation
import Testing
@testable import FamilyConnect

@Suite("Preparing media before upload", .serialized)
struct MediaUploadTests {

    // MARK: - Seams and helpers

    /// The output URLs a stand-in transcoder was handed, so a test can check
    /// that nothing it wrote was left behind in tmp.
    actor Outputs {
        private(set) var urls: [URL] = []
        func add(_ url: URL) { urls.append(url) }
    }

    struct TranscodeFailed: Error {}

    /// What a transcoder under test threw, so a test can say it was the
    /// real thing failing and not the test's own stand-in.
    actor Failures {
        private(set) var errors: [any Error] = []
        func add(_ error: any Error) { errors.append(error) }
    }

    /// Writes a few bytes where the result would go — a half-written file —
    /// and then fails, as an encoder that runs out of disk or meets a codec
    /// it cannot read would.
    private func failing(_ outputs: Outputs) -> MediaPrep.Transcoder {
        MediaPrep.Transcoder(
            video: { _, _, output in
                try Data([0, 0, 0, 0]).write(to: output)
                await outputs.add(output)
                throw TranscodeFailed()
            },
            audio: { _, _, output in
                try Data([0, 0, 0, 0]).write(to: output)
                await outputs.add(output)
                throw TranscodeFailed()
            })
    }

    /// "Succeeds" with a result 4 KB bigger than its source — what an encoder
    /// does to a clip that was already smaller than the profile's bitrate.
    private func growing(_ outputs: Outputs) -> MediaPrep.Transcoder {
        MediaPrep.Transcoder(
            video: { source, _, output in
                try (Data(contentsOf: source) + Data(count: 4096)).write(to: output)
                await outputs.add(output)
            },
            audio: { source, _, output in
                try (Data(contentsOf: source) + Data(count: 4096)).write(to: output)
                await outputs.add(output)
            })
    }

    private func remove(_ urls: URL...) {
        for url in urls { try? FileManager.default.removeItem(at: url) }
    }

    private func exists(_ url: URL) -> Bool {
        FileManager.default.fileExists(atPath: url.path)
    }

    /// The encoders a test that depends on one runs against: the system's
    /// choice everywhere, and on the Mac the software encoder by name too.
    static let encoders: [MediaTranscoder.Encoder] = {
        #if os(macOS)
        [.systemChoice, .softwareOnly]
        #else
        [.systemChoice]
        #endif
    }()

    /// AVFoundation's transcoder, on the encoder named.
    private func transcoder(on encoder: MediaTranscoder.Encoder) -> MediaPrep.Transcoder {
        MediaPrep.Transcoder(
            video: { try await MediaTranscoder.transcodeVideo(from: $0, to: $1, writingTo: $2, encoder: encoder) },
            audio: MediaPrep.Transcoder.avFoundation.audio)
    }

    /// A landscape 1080p60 MP4 — sendable as it is, and never within the
    /// profile: the short side is 1080 and it runs at 60.
    private func landscape1080p60(seconds: Double = 1) async throws -> URL {
        try await MediaFixtures.write(.init(
            width: 1920, height: 1080, frameRate: 60, seconds: seconds, bitrate: 8_000_000,
            fileType: .mp4))
    }

    /// What the planner makes of a file on disk.
    private func probe(_ url: URL) async throws -> MediaPlan.VideoSource {
        await MediaProbe.video(
            at: url, container: try #require(MediaProbe.videoContainer(of: url)),
            sizeBytes: MediaPrep.fileSize(of: url))
    }

    // MARK: - Rule 5: a clip outside the profile is brought to it

    /// The phone's own case, and every property the profile table names.
    ///
    /// EIGHT SECONDS, not two. A rate controller needs time to act, and a
    /// clip too short for it measures the encoder's opening guess instead
    /// of the bitrate it was asked for (the numbers are at the assertion).
    @Test("a portrait 1080p60 HDR-tagged camera clip becomes 720×1280 at 30 fps, H.264 High, SDR, moov first",
          .timeLimit(.minutes(2)), arguments: MediaUploadTests.encoders)
    func portraitCameraClipBecomesTheProfile(encoder: MediaTranscoder.Encoder) async throws {
        let source = try await MediaFixtures.write(.init(
            width: 1920, height: 1080, frameRate: 60, seconds: 8, bitrate: 12_000_000,
            fileType: .mov,
            transform: MediaFixtures.portrait(storedWidth: 1920, storedHeight: 1080),
            audioChannels: 2, hdrTagged: true, softwareEncoder: encoder == .softwareOnly))
        defer { remove(source) }
        let sourceBytes = MediaPrep.fileSize(of: source)

        // The source is what the test says it is: HDR-tagged, stored on its
        // side, and at a camera's bitrate — not several times it, which is
        // what a picture no encoder can compress would be.
        let sourceFacts = try await MediaFixtures.facts(of: source)
        #expect(sourceFacts.transferFunction == kCMFormatDescriptionTransferFunction_ITU_R_2100_HLG as String)
        #expect(sourceFacts.naturalSize == CGSize(width: 1920, height: 1080))
        #expect(sourceFacts.videoDataRate > 6_000_000 && sourceFacts.videoDataRate < 18_000_000,
                "the fixture is the 12 Mbit/s clip it claims: \(sourceFacts.videoDataRate)")

        // What the planner is handed — the precondition for everything below.
        let probed = try await probe(source)
        #expect(probed.container == "video/quicktime", "a camera clip is QuickTime, never within the profile")
        #expect(probed.width == 1080 && probed.height == 1920, "the DISPLAYED size, after the turn")
        #expect(probed.videoCodec == "h264")
        #expect(probed.audioCodec == "aac")
        #expect(probed.audioChannels == 2)
        #expect(abs((probed.frameRate ?? 0) - 60) < 0.5)
        guard case .transcode(let target) = MediaPlan.planVideo(probed) else {
            Issue.record("a 1080p60 QuickTime clip was not planned for a transcode")
            return
        }
        #expect(target.width == 720 && target.height == 1280)
        #expect(target.frameRate == 30)
        #expect(target.videoBitrate == 2_000_000)
        let audioTarget = try #require(target.audioBitrate)
        #expect(audioTarget <= 128_000, "never above the profile's, nor the source's")

        let prepared = try await MediaPrep.prepareVideo(
            from: source, limit: MediaPrep.sizeLimit, transcoder: transcoder(on: encoder))
        defer { remove(prepared.fileURL) }

        #expect(prepared.fileURL != source)
        #expect(prepared.kind == AttachmentDTO.Kind.video)
        #expect(prepared.mime == "video/mp4")
        #expect(prepared.width == 720 && prepared.height == 1280, "the TURNED size is what the upload reports")
        #expect(abs((prepared.durationMS ?? 0) - 8000) <= 100)
        #expect(prepared.previewJPEG != nil, "a transcoded clip still gets its poster")
        #expect(MediaPrep.Magic.honest(url: prepared.fileURL, mime: "video/mp4"))
        #expect(MediaPrep.fileSize(of: prepared.fileURL) < sourceBytes / 3,
                "12 Mbit/s to 2: \(MediaPrep.fileSize(of: prepared.fileURL)) of \(sourceBytes) bytes")

        // moov before mdat: a Range reader can start on the first bytes.
        let boxes = try MediaFixtures.topLevelBoxes(of: prepared.fileURL)
        let moov = try #require(boxes.firstIndex(of: "moov"), "\(boxes)")
        let mdat = try #require(boxes.firstIndex(of: "mdat"), "\(boxes)")
        #expect(moov < mdat, "\(boxes)")

        let out = try await MediaFixtures.facts(of: prepared.fileURL)
        #expect(out.codec == kCMVideoCodecType_H264)
        #expect(out.profile == 100, "H.264 High (profile_idc 100), 8-bit by definition")
        #expect(out.naturalSize == CGSize(width: 720, height: 1280))
        #expect(out.transform == .identity, "the turn is baked into the pixels")
        // SDR, said on the track: an HLG source tone-mapped, not passed through.
        #expect(out.colorPrimaries == kCMFormatDescriptionColorPrimaries_ITU_R_709_2 as String)
        #expect(out.transferFunction == kCMFormatDescriptionTransferFunction_ITU_R_709_2 as String)
        // Never faster than 30: every other frame of the 60, by presentation time.
        #expect(out.nominalFrameRate > 29 && out.nominalFrameRate <= 30.5, "\(out.nominalFrameRate)")
        #expect(out.shortestFrameGap >= 1.0 / 30 - 0.001, "\(out.shortestFrameGap)")
        #expect(abs(out.frameCount - 240) <= 2, "\(out.frameCount) frames in eight seconds")
        #expect(abs(out.durationSeconds - 8) < 0.1)
        // The bitrate is the one that was asked for: no higher than rule A's
        // own tolerance over it, and not so far under that the clip was
        // starved. An encoder steers towards an average rather than landing
        // on it, and takes a few seconds to get there — which is why the
        // clip is eight seconds long. Measured for 2 000 000 on an M4, over
        // three, six and ten seconds: the hardware encoder 2 141 000,
        // 2 056 000 and 2 018 000; the software one, which has no data-rate
        // ceiling to obey, 2 479 000, 2 240 000 and 2 154 000.
        //
        // THE UPPER BOUND IS THE CLIENT'S OWN CEILING, 1.5 ×, and not rule
        // A's 1.25 ×. It was 1.25 ×, which is a bound on an ENCODER this
        // test does not own: the software encoder was measured at + 24 %
        // over three seconds and + 12 % over six, the protocol nowhere
        // requires a client's output to be within rule A, and a hosted
        // runner's software encoder has never been measured at all — so a
        // red run there would have been this line and not the product.
        // What stays asserted is that the request was steered to (neither
        // starved nor several times over, which is what noise gave), and
        // that no more than the source's megabytes go up, above.
        #expect(Double(out.videoDataRate) > 2_000_000 * 0.6, "\(out.videoDataRate)")
        #expect(Double(out.videoDataRate) <= 2_000_000 * 1.5, "\(out.videoDataRate)")
        #expect(out.audioCodec == kAudioFormatMPEG4AAC)
        #expect(out.audioChannels == 2, "stereo stays stereo")
        let audioRate = Double(try #require(out.audioDataRate))
        #expect(abs(audioRate - Double(audioTarget)) < Double(audioTarget) * 0.05, "\(audioRate)")

        // And the whole of it at once, in the protocol's own terms: what
        // came out is WITHIN the profile in everything a client decides —
        // the container, both codecs, the size, the frame rate — so this
        // client, or any other forwarding it, leaves it alone. The rate the
        // encoder landed on is the one thing judged separately (above):
        // here it is set to what was asked, so that this line cannot fail
        // on an encoder's overshoot.
        var again = try await probe(prepared.fileURL)
        #expect(again.videoBitrate != nil, "the result states its own rate: \(again)")
        again.videoBitrate = target.videoBitrate
        #expect(MediaPlan.planVideo(again) == .keep, "\(again)")
    }

    /// What a recent iPhone actually records, which the clip above is not:
    /// that one is 8-bit H.264 merely TAGGED HLG. This one is HEVC Main 10
    /// with ten-bit samples, so the reader really has a 10-bit HLG picture
    /// to bring down to the 8-bit BT.709 the profile asks for.
    ///
    /// What is asserted is that it is READ and comes out as the profile —
    /// the codec, the profile_idc, the tags, every frame. That the tone
    /// curve itself is right (HLG's greys moved, not just relabelled) was
    /// measured by hand against a pass-through and is not something a
    /// test can pin without decoding pixels on every platform's decoder.
    ///
    /// Skipped, not passed, where the machine cannot write such a clip: a
    /// runner with no 10-bit HEVC encoder has nothing to feed it.
    @Test("a 10-bit HEVC HLG clip is read and comes out as 8-bit H.264 High, BT.709",
          .enabled("needs a 10-bit HEVC encoder to write its source") { await MediaUploadTests.canWriteHEVC10Bit },
          .timeLimit(.minutes(2)), arguments: MediaUploadTests.encoders)
    func tenBitHEVCBecomesTheProfile(encoder: MediaTranscoder.Encoder) async throws {
        let source = try await MediaFixtures.write(.init(
            width: 1920, height: 1080, frameRate: 30, seconds: 2, bitrate: 8_000_000,
            fileType: .mov, audioChannels: 2, hdrTagged: true, hevc10Bit: true))
        defer { remove(source) }

        let sourceFacts = try await MediaFixtures.facts(of: source)
        #expect(sourceFacts.codec == kCMVideoCodecType_HEVC)
        #expect(sourceFacts.bitsPerComponent == 10, "the fixture is the 10-bit clip it claims")
        #expect(sourceFacts.transferFunction == kCMFormatDescriptionTransferFunction_ITU_R_2100_HLG as String)

        let probed = try await probe(source)
        #expect(probed.videoCodec == "hevc")
        guard case .transcode(let target) = MediaPlan.planVideo(probed) else {
            Issue.record("an HEVC clip was not planned for a transcode: \(probed)")
            return
        }
        #expect(target.width == 1280 && target.height == 720)

        // Straight through the transcoder, so that a failure is a failure
        // here and not rule C quietly sending the HEVC original.
        let output = MediaFixtures.scratch("mp4")
        defer { remove(output) }
        try await MediaTranscoder.transcodeVideo(from: source, to: target, writingTo: output, encoder: encoder)

        let out = try await MediaFixtures.facts(of: output)
        #expect(out.codec == kCMVideoCodecType_H264)
        #expect(out.profile == 100, "H.264 High (profile_idc 100), which has no 10-bit form")
        #expect(out.naturalSize == CGSize(width: 1280, height: 720))
        #expect(out.colorPrimaries == kCMFormatDescriptionColorPrimaries_ITU_R_709_2 as String)
        #expect(out.transferFunction == kCMFormatDescriptionTransferFunction_ITU_R_709_2 as String)
        #expect(abs(out.frameCount - 60) <= 2, "\(out.frameCount) frames in two seconds")
        #expect(out.audioCodec == kAudioFormatMPEG4AAC)
        let boxes = try MediaFixtures.topLevelBoxes(of: output)
        #expect((boxes.firstIndex(of: "moov") ?? .max) < (boxes.firstIndex(of: "mdat") ?? .min), "\(boxes)")
    }

    /// Whether this machine can write the 10-bit HEVC clip at all — found by
    /// writing two frames of one, because an encoder that is listed is not
    /// always an encoder that works on a virtual machine.
    static var canWriteHEVC10Bit: Bool {
        get async {
            guard let url = try? await MediaFixtures.write(.init(
                width: 320, height: 240, frameRate: 30, seconds: 0.1, bitrate: 500_000,
                fileType: .mov, audioChannels: nil, hdrTagged: true, hevc10Bit: true))
            else {
                return false
            }
            try? FileManager.default.removeItem(at: url)
            return true
        }
    }

    /// Every other clip here has a soundtrack, so every other transcode runs
    /// two pumps. A screen recording, a time-lapse and a clip somebody
    /// muted have none — one pump, a writer with one input, and a planner
    /// that gives no audio target. A transcode does not invent silence.
    @Test("a clip with no audio track is transcoded to a video with none",
          .timeLimit(.minutes(3)), arguments: MediaUploadTests.encoders)
    func clipWithNoAudioTrack(encoder: MediaTranscoder.Encoder) async throws {
        let source = try await MediaFixtures.write(.init(
            width: 1920, height: 1080, frameRate: 60, seconds: 2, bitrate: 8_000_000,
            fileType: .mp4, audioChannels: nil, softwareEncoder: encoder == .softwareOnly))
        defer { remove(source) }
        let probed = try await probe(source)
        #expect(probed.audioCodec == nil, "no track is absent, not unknown")
        guard case .transcode(let target) = MediaPlan.planVideo(probed) else {
            Issue.record("a 1080p60 clip was not planned for a transcode: \(probed)")
            return
        }
        #expect(target.audioBitrate == nil)

        let output = MediaFixtures.scratch("mp4")
        defer { remove(output) }
        try await MediaTranscoder.transcodeVideo(from: source, to: target, writingTo: output, encoder: encoder)

        let out = try await MediaFixtures.facts(of: output)
        #expect(out.naturalSize == CGSize(width: 1280, height: 720))
        #expect(abs(out.frameCount - 60) <= 2, "\(out.frameCount) frames in two seconds")
        #expect(out.audioCodec == nil, "a transcode invented an audio track")
        #expect(try MediaFixtures.topLevelBoxes(of: output).contains("moov"))
    }

    // MARK: - An audio track too quiet to encode

    /// The clip that showed it: a screen recording with the microphone off.
    /// Its AAC track is silence at a variable rate and states a couple of
    /// thousand bits a second; the planner caps the audio target there
    /// (rule B), and Apple's AAC encoder cannot be asked for less than
    /// 16 000. The transcoder used to throw over that — and rule C then
    /// sent the 1080p original, the whole point of issue #74 lost to a
    /// soundtrack with nothing in it. The track is passed through instead
    /// (`MediaTranscoder.audioTreatment`).
    @Test("a video whose audio track states less than the AAC encoder's floor is still transcoded",
          .timeLimit(.minutes(3)), arguments: MediaUploadTests.encoders)
    func quietAudioTrackDoesNotCostTheTranscode(encoder: MediaTranscoder.Encoder) async throws {
        let source = try await MediaFixtures.write(.init(
            width: 1920, height: 1080, frameRate: 30, seconds: 4, bitrate: 8_000_000,
            fileType: .mov, audioChannels: 2, silentAudio: true,
            softwareEncoder: encoder == .softwareOnly))
        defer { remove(source) }
        let sourceBytes = MediaPrep.fileSize(of: source)

        let probed = try await probe(source)
        let stated = try #require(probed.audioBitrate)
        // The precondition: below anything the encoder lists for stereo at
        // any sample rate, so there is no bitrate to encode it at.
        let floor = MediaTranscoder.AACFormat.sampleRates
            .flatMap { MediaTranscoder.AACFormat.applicableBitrates(sampleRate: $0, channels: 2) }.min()
        #expect(stated < (try #require(floor)), "the fixture's silence states \(stated) bit/s")
        #expect(MediaTranscoder.AACFormat.fitting(bitrate: stated, channels: 2, sourceSampleRate: 44_100) == nil)
        guard case .transcode(let target) = MediaPlan.planVideo(probed) else {
            Issue.record("a 1080p QuickTime clip was not planned for a transcode: \(probed)")
            return
        }
        #expect(target.audioBitrate == stated, "capped at the source's own rate")

        let failures = Failures()
        let real = transcoder(on: encoder)
        let prepared = try await MediaPrep.prepareVideo(
            from: source, limit: MediaPrep.sizeLimit,
            transcoder: MediaPrep.Transcoder(
                video: { source, target, output in
                    do {
                        try await real.video(source, target, output)
                    } catch {
                        await failures.add(error)
                        throw error
                    }
                },
                audio: real.audio))
        defer { if prepared.fileURL != source { remove(prepared.fileURL) } }

        let thrown = await failures.errors
        #expect(thrown.isEmpty, "the transcode threw: \(thrown)")
        #expect(prepared.fileURL != source, "rule C sent the original over its soundtrack")
        #expect(prepared.width == 1280 && prepared.height == 720)
        #expect(MediaPrep.fileSize(of: prepared.fileURL) < sourceBytes)

        let out = try await MediaFixtures.facts(of: prepared.fileURL)
        #expect(out.codec == kCMVideoCodecType_H264)
        #expect(out.naturalSize == CGSize(width: 1280, height: 720))
        // The soundtrack is still there, still AAC, still stereo, for the
        // whole clip — and not a bit a second more than it was (rule B).
        #expect(out.audioCodec == kAudioFormatMPEG4AAC)
        #expect(out.audioChannels == 2)
        let audioRate = try #require(out.audioDataRate)
        #expect(Double(audioRate) <= Double(stated) * 1.05, "\(audioRate) from \(stated)")
        #expect(abs(out.durationSeconds - 4) < 0.1)
        let boxes = try MediaFixtures.topLevelBoxes(of: prepared.fileURL)
        #expect((boxes.firstIndex(of: "moov") ?? .max) < (boxes.firstIndex(of: "mdat") ?? .min), "\(boxes)")
    }

    /// The decision on its own, where every branch can be reached without
    /// finding a file that takes it.
    @Test("a video's audio is encoded at the planner's rate, passed through when no encode fits, and refused only when neither can be done")
    func audioTreatmentOrder() {
        typealias Source = MediaTranscoder.SourceAudio
        func treatment(_ bitrate: Int, channels: Int = 2, rate: Double = 44_100, source: Source?)
            -> MediaTranscoder.AudioTreatment? {
            MediaTranscoder.audioTreatment(
                bitrate: bitrate, channels: channels, sourceSampleRate: rate, source: source)
        }
        // The protocol as written: the planner's number, at the source's rate.
        #expect(treatment(128_000, source: Source(isAAC: true, channels: 2, bitrate: 128_000))
                == .encode(.init(sampleRate: 44_100, bitrate: 128_000)),
                "an encode that fits is never skipped, even for a track that could be passed through")
        #expect(treatment(128_000, source: Source(isAAC: false, channels: 2, bitrate: 1_411_200))
                == .encode(.init(sampleRate: 44_100, bitrate: 128_000)))
        // Below the encoder's floor: passed through if it is AAC …
        #expect(treatment(2_067, source: Source(isAAC: true, channels: 2, bitrate: 2_067)) == .passThrough)
        #expect(treatment(1_378, channels: 1, source: Source(isAAC: true, channels: 1, bitrate: 1_378))
                == .passThrough)
        // … and rule C if it is not: nothing is raised to reach the floor.
        #expect(treatment(2_067, source: Source(isAAC: false, channels: 2, bitrate: 2_067)) == nil)
        #expect(treatment(2_067, source: nil) == nil, "a picked sound file is never passed through")
        // Above the floor but only at a lower sample rate — a telephone
        // band from a 44.1 kHz track — the AAC track is kept as it is.
        #expect(treatment(20_000, source: Source(isAAC: true, channels: 2, bitrate: 20_000)) == .passThrough)
        if case .encode(let thin) = treatment(20_000, source: Source(isAAC: false, channels: 2, bitrate: 20_000)) {
            #expect(thin.bitrate <= 20_000 && thin.sampleRate < 44_100, "\(thin)")
        } else {
            Issue.record("a 20 000 bit/s track that is not AAC was not encoded at a lower sample rate")
        }
        // Only the source's OWN rate is passed through: a track the row
        // brought down has to come down.
        #expect(treatment(128_000, source: Source(isAAC: true, channels: 2, bitrate: 256_000))
                == .encode(.init(sampleRate: 44_100, bitrate: 128_000)))
        #expect(treatment(2_067, source: Source(isAAC: true, channels: 2, bitrate: nil)) == nil)
        // And only mono or stereo: 5.1 is folded down, which is an encode.
        #expect(treatment(2_067, source: Source(isAAC: true, channels: 6, bitrate: 2_067)) == nil)
    }

    /// Rule B, on a clip already smaller and slower than the profile: only
    /// its container is wrong, and nothing about it may grow on the way to
    /// the right one.
    ///
    /// The BITRATE half is asserted as what the client guarantees, which is
    /// not what an encoder does. At a few hundred kilobits the software
    /// encoder overshoots by as much as the picture makes it (540 000 for a
    /// 306 000 request, measured on a slightly different scene), and no
    /// setting stops it. What the client promises is that it ASKS for no
    /// more than the source's own rate, and that a result which came out
    /// bigger anyway is thrown away — so the upload never grows.
    @Test("a 640×360 clip at 24 fps keeps its size and its frame rate, and the upload is no bigger (rule B)",
          .timeLimit(.minutes(3)), arguments: MediaUploadTests.encoders)
    func smallSlowClipIsNotRaised(encoder: MediaTranscoder.Encoder) async throws {
        let source = try await MediaFixtures.write(.init(
            width: 640, height: 360, frameRate: 24, seconds: 4, bitrate: 300_000,
            fileType: .mov, audioChannels: 1, softwareEncoder: encoder == .softwareOnly))
        defer { remove(source) }
        let sourceBytes = MediaPrep.fileSize(of: source)
        let probed = try await probe(source)
        let sourceRate = try #require(probed.videoBitrate)
        guard case .transcode(let target) = MediaPlan.planVideo(probed) else {
            Issue.record("a QuickTime clip was not planned for a transcode: \(probed)")
            return
        }
        #expect(target.width == 640 && target.height == 360, "never upscaled")
        #expect(target.frameRate == 24, "24 is kept as it is")
        #expect(target.videoBitrate == min(sourceRate, 400_000), "the profile's 400 000 for this size, or the source's if lower")
        #expect(target.audioBitrate.map { $0 <= 64_000 } == true, "mono, and never above the source's")

        // What the transcoder makes of that target.
        let output = MediaFixtures.scratch("mp4")
        defer { remove(output) }
        try await MediaTranscoder.transcodeVideo(from: source, to: target, writingTo: output, encoder: encoder)
        let out = try await MediaFixtures.facts(of: output)
        #expect(out.naturalSize == CGSize(width: 640, height: 360))
        #expect(abs(out.nominalFrameRate - 24) < 0.5, "\(out.nominalFrameRate)")
        #expect(out.shortestFrameGap >= 1.0 / 24 - 0.001, "\(out.shortestFrameGap)")
        #expect(abs(out.frameCount - 96) <= 2, "\(out.frameCount) frames in four seconds")
        #expect(out.audioChannels == 1, "mono stays mono")

        // And what goes up: the result, or — if the encoder overshot — the
        // source. Never more bytes than the sender picked.
        let prepared = try await MediaPrep.prepareVideo(
            from: source, limit: MediaPrep.sizeLimit, transcoder: transcoder(on: encoder))
        defer { if prepared.fileURL != source { remove(prepared.fileURL) } }
        #expect(MediaPrep.fileSize(of: prepared.fileURL) <= sourceBytes,
                "\(MediaPrep.fileSize(of: prepared.fileURL)) bytes from \(sourceBytes)")
        #expect(prepared.width == 640 && prepared.height == 360)
    }

    // MARK: - Rule A: within the profile, left alone

    @Test("a 720p30 H.264 MP4 at about 1.5 Mbit/s is sent byte for byte as it is (rule A)",
          .timeLimit(.minutes(3)), arguments: MediaUploadTests.encoders)
    func withinProfileIsUntouched(encoder: MediaTranscoder.Encoder) async throws {
        let source = try await MediaFixtures.write(.init(
            width: 1280, height: 720, frameRate: 30, seconds: 4, bitrate: 1_500_000,
            fileType: .mp4, audioChannels: 2, softwareEncoder: encoder == .softwareOnly))
        defer { remove(source) }
        let before = try Data(contentsOf: source)

        let probed = try await probe(source)
        #expect(probed.container == "video/mp4")
        // Rule A's bound for 720p30 is 1.25 × 2 000 000. The clip has to sit
        // under it for this test to be about rule A at all.
        let rate = try #require(probed.videoBitrate)
        #expect(rate > 750_000 && rate <= 2_500_000, "the fixture is the ~1.5 Mbit/s clip it claims: \(rate)")
        #expect(MediaPlan.planVideo(probed) == .keep)

        // A transcoder that would be caught if it were reached.
        let outputs = Outputs()
        let prepared = try await MediaPrep.prepareVideo(
            from: source, limit: MediaPrep.sizeLimit, transcoder: growing(outputs))

        #expect(await outputs.urls.isEmpty, "a clip within the profile reached the transcoder")
        #expect(prepared.fileURL == source, "the ORIGINAL goes, not a copy and not a re-encode")
        #expect(try Data(contentsOf: prepared.fileURL) == before)
        #expect(prepared.kind == AttachmentDTO.Kind.video)
        #expect(prepared.mime == "video/mp4")
        #expect(prepared.width == 1280 && prepared.height == 720)
        #expect(abs((prepared.durationMS ?? 0) - 4000) <= 100)
        #expect(prepared.previewJPEG != nil, "a clip sent as it is still gets its poster")
    }

    // MARK: - Audio alone

    @Test("a stereo WAV becomes an M4A of AAC-LC at 128 000 bit/s", .timeLimit(.minutes(1)))
    func stereoWAVBecomesAAC() async throws {
        let wav = try MediaFixtures.writeWAV(seconds: 3, sampleRate: 44_100, channels: 2)
        defer { remove(wav) }

        let prepared = try await MediaPrep.prepare(fileAt: wav, name: "Tone.wav", limit: MediaPrep.sizeLimit)
        defer { remove(prepared.fileURL) }

        #expect(prepared.kind == AttachmentDTO.Kind.audio)
        #expect(prepared.mime == "audio/mp4")
        #expect(prepared.fileURL.pathExtension == "m4a")
        #expect(prepared.name == "Tone.m4a", "the name follows the bytes")
        #expect(abs((prepared.durationMS ?? 0) - 3000) <= 100)
        #expect(MediaPrep.Magic.honest(url: prepared.fileURL, mime: "audio/mp4"))
        let boxes = try MediaFixtures.topLevelBoxes(of: prepared.fileURL)
        #expect((boxes.firstIndex(of: "moov") ?? .max) < (boxes.firstIndex(of: "mdat") ?? .min), "\(boxes)")
        // About a tenth: 1 411 200 bit/s of PCM against 128 000 of AAC.
        #expect(MediaPrep.fileSize(of: prepared.fileURL) < MediaPrep.fileSize(of: wav) / 5)

        let out = try await MediaFixtures.audioFacts(of: prepared.fileURL)
        #expect(out.codec == kAudioFormatMPEG4AAC)
        #expect(out.channels == 2)
        #expect(abs(Double(out.dataRate) - 128_000) < 128_000 * 0.05, "\(out.dataRate)")
    }

    /// 64 000 bit/s is more than Apple's AAC-LC encoder will spend on one
    /// channel at 16 kHz, where a voice recorder's WAV often is — so the
    /// rate is found by moving the sample rate, not by giving up on the
    /// protocol's number.
    @Test("a 16 kHz mono WAV becomes mono AAC-LC at 64 000 bit/s", .timeLimit(.minutes(1)))
    func monoLowRateWAVBecomesAAC() async throws {
        let wav = try MediaFixtures.writeWAV(seconds: 3, sampleRate: 16_000, channels: 1)
        defer { remove(wav) }

        let prepared = try await MediaPrep.prepare(fileAt: wav, limit: MediaPrep.sizeLimit)
        defer { remove(prepared.fileURL) }

        #expect(prepared.mime == "audio/mp4")
        let out = try await MediaFixtures.audioFacts(of: prepared.fileURL)
        #expect(out.codec == kAudioFormatMPEG4AAC)
        #expect(out.channels == 1, "mono stays mono")
        #expect(abs(Double(out.dataRate) - 64_000) < 64_000 * 0.05, "\(out.dataRate)")
    }

    @Test("a 128 000 bit/s AAC file is sent untouched", .timeLimit(.minutes(1)))
    func compressedAudioIsUntouched() async throws {
        let m4a = try MediaFixtures.writeAAC(seconds: 3, bitrate: 128_000, channels: 2)
        defer { remove(m4a) }
        let before = try Data(contentsOf: m4a)

        let probed = await MediaProbe.audio(at: m4a, container: "audio/mp4", sizeBytes: before.count)
        #expect(probed.codec == "aac")
        #expect(MediaPlan.planAudio(probed) == .keep, "\(probed)")

        let prepared = try await MediaPrep.prepare(fileAt: m4a, name: "Song.m4a", limit: MediaPrep.sizeLimit)
        defer { remove(prepared.fileURL) }

        #expect(prepared.kind == AttachmentDTO.Kind.audio)
        #expect(prepared.mime == "audio/mp4")
        #expect(prepared.name == "Song.m4a")
        #expect(try Data(contentsOf: prepared.fileURL) == before, "not a second lossy generation")
    }

    /// Lossless inside an ACCEPTED container: the file would go up as it is,
    /// and the rules re-encode it by its codec, not its name.
    @Test("an Apple Lossless M4A becomes AAC-LC at 128 000 bit/s", .timeLimit(.minutes(1)))
    func appleLosslessBecomesAAC() async throws {
        let alac = try MediaFixtures.writeALAC(seconds: 3, sampleRate: 44_100, channels: 2)
        defer { remove(alac) }
        #expect(MediaPrep.isSupportedAudio(alac), "an M4A is an accepted type whatever it holds")
        let probed = await MediaProbe.audio(
            at: alac, container: "audio/mp4", sizeBytes: MediaPrep.fileSize(of: alac))
        #expect(probed.codec == "alac")
        #expect(MediaPlan.planAudio(probed) == .transcode(bitrate: 128_000), "\(probed)")

        // Through the transcoder itself first: `prepare` would turn a
        // failure into the original going up, and the test into a pass.
        let output = MediaFixtures.scratch("m4a")
        defer { remove(output) }
        try await MediaTranscoder.transcodeAudio(from: alac, bitrate: 128_000, writingTo: output)
        let out = try await MediaFixtures.audioFacts(of: output)
        #expect(out.codec == kAudioFormatMPEG4AAC)
        #expect(out.channels == 2)
        #expect(abs(Double(out.dataRate) - 128_000) < 128_000 * 0.05, "\(out.dataRate)")
        #expect(abs(out.durationSeconds - 3) < 0.1)

        let prepared = try await MediaPrep.prepare(fileAt: alac, name: "Take 3.m4a", limit: MediaPrep.sizeLimit)
        defer { remove(prepared.fileURL) }
        #expect(prepared.kind == AttachmentDTO.Kind.audio)
        #expect(prepared.mime == "audio/mp4")
        #expect(prepared.name == "Take 3.m4a")
        // Whichever is smaller goes (rule D): a pure tone is the one signal
        // a lossless coder can beat AAC on, so this asserts only that what
        // goes is no bigger than what was picked.
        #expect(MediaPrep.fileSize(of: prepared.fileURL) <= MediaPrep.fileSize(of: alac))
    }

    /// 1.1 sent a FLAC as a FILE, as it did an AIFF. It is the largest
    /// lossless music there is, and AVFoundation reads it.
    @Test("a FLAC becomes playable AAC-LC audio", .timeLimit(.minutes(1)))
    func flacBecomesAudio() async throws {
        let flac = try MediaFixtures.writeFLAC(seconds: 3, sampleRate: 44_100, channels: 2)
        defer { remove(flac) }
        #expect(!MediaPrep.isSupportedAudio(flac))
        let probed = await MediaProbe.audio(
            at: flac, container: MediaPrep.audioMIME(for: flac), sizeBytes: MediaPrep.fileSize(of: flac))
        #expect(probed.codec == "flac")
        #expect(MediaPlan.planAudio(probed) == .transcode(bitrate: 128_000), "\(probed)")

        let output = MediaFixtures.scratch("m4a")
        defer { remove(output) }
        try await MediaTranscoder.transcodeAudio(from: flac, bitrate: 128_000, writingTo: output)
        let out = try await MediaFixtures.audioFacts(of: output)
        #expect(out.codec == kAudioFormatMPEG4AAC)
        #expect(out.channels == 2)
        #expect(abs(Double(out.dataRate) - 128_000) < 128_000 * 0.05, "\(out.dataRate)")

        // And through the door a picked file comes in by: audio, not the
        // document row 1.1 gave it, whichever of the two is smaller.
        let prepared = try await MediaPrep.prepare(fileAt: flac, name: "Song.flac", limit: MediaPrep.sizeLimit)
        defer { remove(prepared.fileURL) }
        #expect(prepared.kind == AttachmentDTO.Kind.audio, "a FLAC the server will not take went up as it was")
        #expect(prepared.mime == "audio/mp4")
        #expect(prepared.name == "Song.m4a", "the name follows the bytes")
    }

    /// 1.1 sent an AIFF as a FILE — the server does not take it as audio —
    /// so a family got a document row for a song. Re-encoded, it plays; and
    /// when it cannot be, it is still the file 1.1 sent.
    @Test("an AIFF becomes playable audio, and a failed transcode leaves it the file 1.1 sent",
          .timeLimit(.minutes(1)))
    func aiffBecomesAudioOrStaysAFile() async throws {
        let aiff = try MediaFixtures.writeAIFF(seconds: 2, sampleRate: 44_100, channels: 2)
        defer { remove(aiff) }
        #expect(!MediaPrep.isSupportedAudio(aiff))

        let prepared = try await MediaPrep.prepare(fileAt: aiff, limit: MediaPrep.sizeLimit)
        defer { remove(prepared.fileURL) }
        #expect(prepared.kind == AttachmentDTO.Kind.audio)
        #expect(prepared.mime == "audio/mp4")
        #expect(prepared.name?.hasSuffix(".m4a") == true)
        #expect(try await MediaFixtures.audioFacts(of: prepared.fileURL).codec == kAudioFormatMPEG4AAC)

        let outputs = Outputs()
        let fallback = try await MediaPrep.prepare(
            fileAt: aiff, limit: MediaPrep.sizeLimit, transcoder: failing(outputs))
        defer { remove(fallback.fileURL) }
        #expect(fallback.kind == AttachmentDTO.Kind.file)
        #expect(fallback.mime == MediaPrep.mimeType(for: aiff))
        #expect(try Data(contentsOf: fallback.fileURL) == Data(contentsOf: aiff))
        for output in await outputs.urls {
            #expect(!exists(output), "a failed transcode's file was left in tmp")
        }
    }

    // MARK: - Rule C: a failure sends what 1.1 would have sent

    @Test("a video transcode that fails sends the original untouched (rule C)", .timeLimit(.minutes(1)))
    func failedVideoTranscodeSendsTheOriginal() async throws {
        let source = try await landscape1080p60()
        defer { remove(source) }
        let before = try Data(contentsOf: source)
        let outputs = Outputs()

        let prepared = try await MediaPrep.prepareVideo(
            from: source, limit: MediaPrep.sizeLimit, transcoder: failing(outputs))

        #expect(await outputs.urls.count == 1, "the plan was a transcode, and it was tried")
        #expect(prepared.fileURL == source)
        #expect(try Data(contentsOf: prepared.fileURL) == before)
        #expect(prepared.kind == AttachmentDTO.Kind.video)
        #expect(prepared.mime == "video/mp4")
        #expect(prepared.width == 1920 && prepared.height == 1080)
        for output in await outputs.urls {
            #expect(!exists(output), "a failed transcode's file was left in tmp")
        }
    }

    /// The other half of rule C: "a platform that cannot transcode this
    /// source at all". AVFoundation reads nothing out of this file, so
    /// there is no size to transcode to — and 1.1 sent it anyway.
    @Test("an MP4 that AVFoundation cannot read is sent exactly as 1.1 sent it (rule C)", .timeLimit(.minutes(1)))
    func unreadableVideoIsSentAsBefore() async throws {
        let source = MediaFixtures.scratch("mp4")
        try Data([0, 0, 0, 0x18] + Array("ftypisom".utf8) + [UInt8](repeating: 0, count: 12)).write(to: source)
        defer { remove(source) }
        let outputs = Outputs()

        let probed = try await probe(source)
        #expect(probed.width == 0 && probed.height == 0)
        #expect(MediaPlan.planVideo(probed) == .fallback)

        let prepared = try await MediaPrep.prepareVideo(
            from: source, limit: MediaPrep.sizeLimit, transcoder: failing(outputs))

        #expect(await outputs.urls.isEmpty, "there was nothing to hand a transcoder")
        #expect(prepared.fileURL == source)
        #expect(prepared.kind == AttachmentDTO.Kind.video)
        #expect(prepared.mime == "video/mp4")
    }

    /// The real transcoder, really failing: it is told to write into a
    /// directory that is not there, so the writer will not start. It has to
    /// THROW — promptly — because rule C can only send the original if the
    /// transcode comes back.
    @Test("the AVFoundation transcoder throws when its writer cannot start, and rule C sends the original",
          .timeLimit(.minutes(1)))
    func realTranscoderFailureIsRuleC() async throws {
        let source = try await landscape1080p60()
        defer { remove(source) }
        let before = try Data(contentsOf: source)
        let nowhere = FileManager.default.temporaryDirectory
            .appendingPathComponent("fc-no-such-directory-\(UUID().uuidString)")
            .appendingPathComponent("out.mp4")
        let failures = Failures()
        let transcoder = MediaPrep.Transcoder(
            video: { source, target, _ in
                do {
                    try await MediaTranscoder.transcodeVideo(from: source, to: target, writingTo: nowhere)
                } catch {
                    await failures.add(error)
                    throw error
                }
            },
            audio: MediaPrep.Transcoder.avFoundation.audio)

        let prepared = try await MediaPrep.prepareVideo(
            from: source, limit: MediaPrep.sizeLimit, transcoder: transcoder)

        let thrown = await failures.errors
        #expect(thrown.count == 1, "the plan was a transcode, and it was tried")
        #expect(thrown.first is MediaTranscoder.Failure, "\(thrown)")
        #expect(prepared.fileURL == source)
        #expect(try Data(contentsOf: prepared.fileURL) == before)
        #expect(prepared.kind == AttachmentDTO.Kind.video)
    }

    /// Over the ceiling, "as before" is 1.1's own path — the 1080p export
    /// preset, refused only when even that does not fit. The refusal names
    /// the EXPORT's size, which is how this knows the export really ran.
    @Test("over the ceiling, a failed transcode falls back to 1.1's export and refusal (rule C)",
          .timeLimit(.minutes(2)))
    func failedVideoTranscodeOverTheCeilingTakesTodaysPath() async throws {
        let source = try await landscape1080p60()
        defer { remove(source) }
        let sourceBytes = MediaPrep.fileSize(of: source)

        do {
            let prepared = try await MediaPrep.prepareVideo(
                from: source, limit: 10_000, transcoder: failing(Outputs()))
            remove(prepared.fileURL)
            Issue.record("a clip over a 10 KB ceiling was not refused")
        } catch MediaPrep.PrepError.tooLargeAfterCompression(let bytes) {
            #expect(bytes != sourceBytes, "refused on the source's size: the 1.1 export never ran")
            #expect(bytes > 10_000)
        }
    }

    @Test("an audio transcode that fails sends the original WAV untouched (rule C)", .timeLimit(.minutes(1)))
    func failedAudioTranscodeSendsTheOriginal() async throws {
        let wav = try MediaFixtures.writeWAV(seconds: 1, sampleRate: 44_100, channels: 2)
        defer { remove(wav) }
        let outputs = Outputs()

        let prepared = try await MediaPrep.prepare(
            fileAt: wav, limit: MediaPrep.sizeLimit, transcoder: failing(outputs))
        defer { remove(prepared.fileURL) }

        #expect(await outputs.urls.count == 1)
        #expect(prepared.kind == AttachmentDTO.Kind.audio)
        #expect(prepared.mime == "audio/wav")
        #expect(try Data(contentsOf: prepared.fileURL) == Data(contentsOf: wav))
        for output in await outputs.urls {
            #expect(!exists(output))
        }
    }

    // MARK: - Rule D: a result bigger than its source is thrown away

    @Test("a transcoded video bigger than its sendable source is thrown away (rule D)", .timeLimit(.minutes(3)))
    func biggerVideoResultIsThrownAway() async throws {
        let source = try await landscape1080p60()
        defer { remove(source) }
        let outputs = Outputs()

        let prepared = try await MediaPrep.prepareVideo(
            from: source, limit: MediaPrep.sizeLimit, transcoder: growing(outputs))

        #expect(await outputs.urls.count == 1)
        #expect(prepared.fileURL == source, "the source goes")
        for output in await outputs.urls {
            #expect(!exists(output), "the bigger result was left in tmp")
        }
    }

    @Test("a transcoded sound file bigger than its source is thrown away (rule D)", .timeLimit(.minutes(3)))
    func biggerAudioResultIsThrownAway() async throws {
        let wav = try MediaFixtures.writeWAV(seconds: 1, sampleRate: 44_100, channels: 2)
        defer { remove(wav) }
        let outputs = Outputs()

        let prepared = try await MediaPrep.prepare(
            fileAt: wav, limit: MediaPrep.sizeLimit, transcoder: growing(outputs))
        defer { remove(prepared.fileURL) }

        #expect(prepared.mime == "audio/wav")
        #expect(try Data(contentsOf: prepared.fileURL) == Data(contentsOf: wav))
        for output in await outputs.urls {
            #expect(!exists(output))
        }
    }

    /// The second half of rule D: "otherwise the result is used, since it
    /// is the only thing that can be sent". The source here is a movie
    /// AVFoundation reads and the server would refuse — a QuickTime file
    /// from before `ftyp`, which 1.1 sent as a FILE — so a result that came
    /// out bigger is still the only VIDEO there is.
    @Test("a bigger result is still used when the source could not have gone as a video (rule D)",
          .timeLimit(.minutes(1)))
    func biggerResultIsUsedWhenTheSourceIsNotSendable() async throws {
        let source = try await oldQuickTimeMovie()
        defer { remove(source) }
        let sourceBytes = MediaPrep.fileSize(of: source)
        #expect(MediaProbe.videoContainer(of: source) == nil, "no ftyp: not a type the server takes as video")
        let probed = await MediaProbe.video(
            at: source, container: MediaPrep.mimeType(for: source), sizeBytes: sourceBytes)
        #expect(probed.width == 1920 && probed.height == 1080, "AVFoundation still reads it: \(probed)")

        // A real MP4, padded past the source's size: bigger, and sendable.
        let sendable = try await landscape1080p60()
        defer { remove(sendable) }
        let outputs = Outputs()
        let prepared = try await MediaPrep.prepareVideo(
            from: source, limit: MediaPrep.sizeLimit,
            transcoder: MediaPrep.Transcoder(
                video: { _, _, output in
                    try (Data(contentsOf: sendable) + Data(count: sourceBytes)).write(to: output)
                    await outputs.add(output)
                },
                audio: { _, _, _ in }))
        defer { if prepared.fileURL != source { remove(prepared.fileURL) } }

        let output = try #require(await outputs.urls.first)
        #expect(MediaPrep.fileSize(of: output) > sourceBytes, "the stand-in's result is the bigger one")
        #expect(prepared.fileURL == output, "the result goes: the source never could")
        #expect(prepared.kind == AttachmentDTO.Kind.video)
        #expect(prepared.mime == "video/mp4")
    }

    /// A result the server would still refuse is not a result. The source
    /// is over the ceiling and the transcode brought it down, but not far
    /// enough — so it is rule C, and over the ceiling that is 1.1's export,
    /// whose own size is the one the refusal names.
    @Test("a result still over the ceiling is thrown away and 1.1's path is taken (rule C)",
          .timeLimit(.minutes(2)))
    func resultOverTheCeilingIsNotSent() async throws {
        let source = try await landscape1080p60()
        defer { remove(source) }
        let outputs = Outputs()
        // The first 20 KB of the source: smaller than it, an honest `ftyp`
        // at the front, and twice the ceiling.
        let shrinking = MediaPrep.Transcoder(
            video: { source, _, output in
                try Data(contentsOf: source).prefix(20_000).write(to: output)
                await outputs.add(output)
            },
            audio: { _, _, _ in })

        do {
            let prepared = try await MediaPrep.prepareVideo(from: source, limit: 10_000, transcoder: shrinking)
            remove(prepared.fileURL)
            Issue.record("a result of 20 KB went up under a 10 KB ceiling")
        } catch MediaPrep.PrepError.tooLargeAfterCompression(let bytes) {
            #expect(bytes != 20_000, "refused on the transcode's size: the 1.1 export never ran")
            #expect(bytes > 10_000)
        }
        #expect(await outputs.urls.count == 1, "the plan was a transcode, and it was tried")
        for output in await outputs.urls {
            #expect(!exists(output), "the over-ceiling result was left in tmp")
        }
    }

    /// A QuickTime movie as they were written before `ftyp` existed: the
    /// same file with that box renamed `free`, which moves no byte and so
    /// leaves every offset in it true. AVFoundation reads it; the server's
    /// magic-number check does not know it.
    private func oldQuickTimeMovie() async throws -> URL {
        let modern = try await MediaFixtures.write(.init(
            width: 1920, height: 1080, frameRate: 30, seconds: 1, bitrate: 4_000_000, fileType: .mov))
        defer { remove(modern) }
        var bytes = try Data(contentsOf: modern)
        try #require(bytes.count > 8 && bytes[4..<8] == Data("ftyp".utf8))
        bytes.replaceSubrange(4..<8, with: Data("free".utf8))
        let url = MediaFixtures.scratch("mov")
        try bytes.write(to: url)
        return url
    }

    // MARK: - Cancellation is not a failure

    /// Rule A and rule C both end in "the original goes" without a single
    /// throwing call on the way, so a preparation cancelled while the file
    /// was being read used to come back as something to STAGE. Now that a
    /// composer can cancel one, that would be a clip reappearing after its
    /// Cancel was pressed.
    @Test("a cancelled preparation throws even when the plan was to send the original", .timeLimit(.minutes(1)))
    func cancelledPreparationOfAKeptClipSendsNothing() async throws {
        let source = try await MediaFixtures.write(.init(
            width: 1280, height: 720, frameRate: 30, seconds: 2, bitrate: 1_500_000, fileType: .mp4))
        defer { remove(source) }
        #expect(MediaPlan.planVideo(try await probe(source)) == .keep)
        let outputs = Outputs()
        let (gate, open) = AsyncStream<Void>.makeStream()

        let task = Task {
            for await _ in gate { break }
            return try await MediaPrep.prepareVideo(
                from: source, limit: MediaPrep.sizeLimit, transcoder: growing(outputs))
        }
        task.cancel()
        open.yield()

        await #expect(throws: CancellationError.self) { try await task.value }
        #expect(await outputs.urls.isEmpty)
        #expect(exists(source), "cancelling never deletes what was picked")
    }

    /// THE WINDOW A CANCEL CRASHED IN. The writer has started and no pump
    /// has: the cancel settles the session, which marks every input
    /// finished on its pump's queue — and the pump was then STARTED, which
    /// asks a finished input for its media data. AVFoundation answers that
    /// with an NSInternalInconsistencyException, so the failure was not a
    /// red test but a dead test host. A cancel from outside lands there
    /// about never on a fast Mac (3 300 tries found none) and would on a
    /// slow runner, so this puts one there on purpose, through the seam
    /// `Pump.run` has for it, and waits for the stop to have happened.
    @Test("a cancel between the writer starting and the pumps starting is a CancellationError, not an exception",
          .timeLimit(.minutes(1)))
    func cancelBeforeThePumpsStart() async throws {
        let wav = try MediaFixtures.writeWAV(seconds: 2, sampleRate: 44_100, channels: 2)
        defer { remove(wav) }
        let output = MediaFixtures.scratch("m4a")
        defer { remove(output) }

        let task = Task {
            let asset = AVURLAsset(url: wav)
            let track = try #require(try await asset.loadTracks(withMediaType: .audio).first)
            let reader = try AVAssetReader(asset: asset)
            let writer = try AVAssetWriter(outputURL: output, fileType: .m4a)
            let pump = try await MediaTranscoder.audioPump(
                track: track, bitrate: 128_000, reader: reader, writer: writer)
            try await MediaTranscoder.Pump.run([pump], reader: reader, writer: writer) {
                // Cancelling runs the task's cancellation handler here and
                // now; what that asks of the pump's queue takes a moment.
                withUnsafeCurrentTask { $0?.cancel() }
                Thread.sleep(forTimeInterval: 0.2)
            }
        }

        await #expect(throws: CancellationError.self) { try await task.value }
        #expect(!exists(output), "a cancelled transcode left its half-written file")
    }

    /// Somebody takes a clip off the strip mid-transcode. Rule C is for a
    /// transcode that FAILED; a cancelled one sends nothing and leaves
    /// nothing behind.
    @Test("a cancelled transcode throws CancellationError and leaves nothing in tmp", .timeLimit(.minutes(1)))
    func cancelledTranscodeSendsNothing() async throws {
        let source = try await landscape1080p60()
        defer { remove(source) }
        let (started, entered) = AsyncStream<URL>.makeStream()
        let transcoder = MediaPrep.Transcoder(
            video: { _, _, output in
                try Data([0]).write(to: output)
                entered.yield(output)
                try await Task.sleep(for: .seconds(60))
            },
            audio: { _, _, _ in })

        let task = Task {
            try await MediaPrep.prepareVideo(from: source, limit: MediaPrep.sizeLimit, transcoder: transcoder)
        }
        var iterator = started.makeAsyncIterator()
        let output = try #require(await iterator.next())
        task.cancel()

        await #expect(throws: CancellationError.self) { try await task.value }
        #expect(!exists(output))
    }

    @Test("the AVFoundation transcoder does not start for a task already cancelled", .timeLimit(.minutes(1)))
    func transcoderHonoursCancellation() async throws {
        let source = try await landscape1080p60()
        defer { remove(source) }
        let output = MediaFixtures.scratch("mp4")
        let (gate, open) = AsyncStream<Void>.makeStream()
        let target = MediaPlan.VideoTarget(
            width: 1280, height: 720, frameRate: 30, videoBitrate: 2_000_000, audioBitrate: nil)

        let task = Task {
            for await _ in gate { break }
            try await MediaTranscoder.transcodeVideo(from: source, to: target, writingTo: output)
        }
        task.cancel()
        open.yield()

        await #expect(throws: CancellationError.self) { try await task.value }
        #expect(!exists(output))
    }

    /// The other end of it: a transcode already under way. The output file
    /// appearing is the writer having started, which is when this cancels.
    @Test("the AVFoundation transcoder stops mid-way when its task is cancelled, and removes its file",
          .timeLimit(.minutes(3)), arguments: MediaUploadTests.encoders)
    func transcoderStopsMidWay(encoder: MediaTranscoder.Encoder) async throws {
        let source = try await landscape1080p60(seconds: 6)
        defer { remove(source) }
        let output = MediaFixtures.scratch("mp4")
        defer { remove(output) }
        let target = MediaPlan.VideoTarget(
            width: 1280, height: 720, frameRate: 30, videoBitrate: 2_000_000, audioBitrate: 128_000)

        let task = Task {
            try await MediaTranscoder.transcodeVideo(from: source, to: target, writingTo: output, encoder: encoder)
        }
        while !exists(output) {
            try await Task.sleep(for: .milliseconds(1))
        }
        task.cancel()

        await #expect(throws: CancellationError.self) { try await task.value }
        #expect(!exists(output), "a cancelled transcode left its half-written file")
    }

    // MARK: - Voice notes

    @Test("a voice note is never put through the audio rules")
    func voiceNoteIsNeverReEncoded() async throws {
        // A WAV on purpose: as a picked file it would be re-encoded.
        let wav = try MediaFixtures.writeWAV(seconds: 1, sampleRate: 44_100, channels: 1)
        defer { remove(wav) }
        let outputs = Outputs()

        let prepared = try await MediaPrep.prepareAudio(
            from: wav, limit: MediaPrep.sizeLimit, isVoiceNote: true, transcoder: failing(outputs))
        defer { remove(prepared.fileURL) }

        #expect(await outputs.urls.isEmpty, "a voice note reached the transcoder")
        #expect(prepared.name == nil)
        #expect(try Data(contentsOf: prepared.fileURL) == Data(contentsOf: wav))
    }

    /// The protocol's voice-note row, and proof the settings mean what they
    /// say: the same dictionary, handed to Core Audio's encoder, writes AAC
    /// mono at 64 000 bit/s. (The recorder itself needs a microphone.)
    @Test("a voice note is recorded as AAC-LC, mono, 64 000 bit/s")
    func voiceNoteSettingsAreTheProtocols() async throws {
        let settings = AudioRecorder.settings
        #expect(settings[AVFormatIDKey] as? Int == Int(kAudioFormatMPEG4AAC))
        #expect(settings[AVNumberOfChannelsKey] as? Int == 1)
        #expect(settings[AVSampleRateKey] as? Double == 44_100)
        #expect(settings[AVEncoderBitRateKey] as? Int == MediaPlan.voiceNoteBitrate)
        #expect(MediaPlan.voiceNoteBitrate == 64_000)
        #expect(settings[AVEncoderAudioQualityKey] == nil, "a quality level leaves the rate to the encoder")

        let recorded = try MediaFixtures.writeAAC(
            seconds: 3, bitrate: 0, channels: 1, settings: settings)
        defer { remove(recorded) }
        let out = try await MediaFixtures.audioFacts(of: recorded)
        #expect(out.codec == kAudioFormatMPEG4AAC)
        #expect(out.channels == 1)
        #expect(out.sampleRate == 44_100)
        #expect(abs(Double(out.dataRate) - 64_000) < 64_000 * 0.05, "\(out.dataRate)")
    }

    // MARK: - The pieces

    @Test("the container comes from the ftyp brand, not the extension")
    func containerIsReadOffTheBytes() throws {
        func file(brand: String, ext: String) throws -> URL {
            let url = MediaFixtures.scratch(ext)
            try Data([0, 0, 0, 0x18] + Array("ftyp".utf8) + Array(brand.utf8) + [0, 0, 0, 0]).write(to: url)
            return url
        }
        let mov = try file(brand: "qt  ", ext: "mp4")
        let mp4 = try file(brand: "isom", ext: "mov")
        let neither = MediaFixtures.scratch("mov")
        try Data([0x1A, 0x45, 0xDF, 0xA3, 0, 0, 0, 0, 0, 0, 0, 0]).write(to: neither)
        defer { remove(mov, mp4, neither) }

        #expect(MediaProbe.videoContainer(of: mov) == "video/quicktime")
        #expect(MediaProbe.videoContainer(of: mp4) == "video/mp4")
        #expect(MediaProbe.videoContainer(of: neither) == nil)
    }

    @Test("codecs are named as the reference names them")
    func codecNames() {
        #expect(MediaProbe.videoCodec(kCMVideoCodecType_H264) == "h264")
        #expect(MediaProbe.videoCodec(MediaProbe.fourCC("avc3")) == "h264")
        #expect(MediaProbe.videoCodec(kCMVideoCodecType_HEVC) == "hevc")
        #expect(MediaProbe.videoCodec(MediaProbe.fourCC("hev1")) == "hevc")
        #expect(MediaProbe.videoCodec(MediaProbe.fourCC("av01")) == "av1")
        #expect(MediaProbe.videoCodec(MediaProbe.fourCC("vp09")) == "vp9")
        #expect(MediaProbe.videoCodec(kCMVideoCodecType_AppleProRes422) == "unknown")
        #expect(MediaProbe.audioCodec(kAudioFormatMPEG4AAC) == "aac")
        #expect(MediaProbe.audioCodec(kAudioFormatMPEG4AAC_HE) == "aac", "any AAC profile")
        #expect(MediaProbe.audioCodec(kAudioFormatMPEG4AAC_HE_V2) == "aac")
        #expect(MediaProbe.audioCodec(kAudioFormatMPEGLayer3) == "mp3")
        #expect(MediaProbe.audioCodec(kAudioFormatLinearPCM) == "pcm")
        #expect(MediaProbe.audioCodec(kAudioFormatAppleLossless) == "alac")
        #expect(MediaProbe.audioCodec(kAudioFormatFLAC) == "flac")
        #expect(MediaProbe.audioCodec(kAudioFormatOpus) == "opus")
        #expect(MediaProbe.audioCodec(kAudioFormatAC3) == "unknown")
    }

    /// The settings ladder's first rung is the one in use — on every
    /// platform this runs on. A rung the writer refuses is skipped silently
    /// by design, so without this a platform could lose the data-rate
    /// ceiling, or drop to Main, and nothing would say.
    @Test("the writer takes H.264 High with the data-rate ceiling", arguments: MediaUploadTests.encoders)
    func h264SettingsAreTheFirstRung(encoder: MediaTranscoder.Encoder) throws {
        let writer = try AVAssetWriter(outputURL: MediaFixtures.scratch("mp4"), fileType: .mp4)
        let target = MediaPlan.VideoTarget(
            width: 720, height: 1280, frameRate: 30, videoBitrate: 2_000_000, audioBitrate: nil)
        let settings = try #require(MediaTranscoder.h264Settings(for: target, writer: writer, encoder: encoder))
        let compression = try #require(settings[AVVideoCompressionPropertiesKey] as? [String: Any])

        #expect(settings[AVVideoCodecKey] as? AVVideoCodecType == .h264)
        #expect(settings[AVVideoWidthKey] as? Int == 720)
        #expect(settings[AVVideoHeightKey] as? Int == 1280)
        #expect(compression[AVVideoProfileLevelKey] as? String == AVVideoProfileLevelH264HighAutoLevel)
        #expect(compression[AVVideoAverageBitRateKey] as? Int == 2_000_000)
        // 1.5 × the target, in bytes, in any one second.
        #expect(compression["DataRateLimits"] as? [Int] == [375_000, 1])
    }

    /// Never a hair faster than the target: a frame duration rounded DOWN
    /// to the grid would make 30.5 fps 30.5002.
    @Test("a frame duration never runs faster than its rate",
          arguments: [30.0, 29.97002997002997, 25.0, 24.0, 23.976023976023978, 30.5, 15.0,
                      Double(Float(29.97)), 12.5])
    func frameDurationNeverRunsFast(rate: Double) {
        let duration = MediaTranscoder.frameDuration(for: rate)
        let actual = Double(duration.timescale) / Double(duration.value)
        #expect(actual <= rate * (1 + 1e-12), "\(rate) → \(actual)")
        #expect(actual > rate * 0.9999, "\(rate) → \(actual)")
    }

    @Test("the common rates are held exactly")
    func frameDurationIsExactForCommonRates() {
        #expect(MediaTranscoder.frameDuration(for: 30) == CMTime(value: 1, timescale: 30))
        #expect(MediaTranscoder.frameDuration(for: 25) == CMTime(value: 1, timescale: 25))
        #expect(MediaTranscoder.frameDuration(for: 30_000.0 / 1001) == CMTime(value: 1001, timescale: 30_000))
    }

    @Test("a portrait turn lands the picture on the canvas, upright and scaled")
    func uprightTransformFillsTheCanvas() {
        let natural = CGSize(width: 1920, height: 1080)
        let render = CGSize(width: 720, height: 1280)
        let phone = MediaFixtures.portrait(storedWidth: 1920, storedHeight: 1080)
        // The same turn without its translation — a writer that stored only
        // the rotation — must land in the same place.
        let bare = CGAffineTransform(rotationAngle: .pi / 2)
        for preferred in [phone, bare] {
            let placed = CGRect(origin: .zero, size: natural)
                .applying(MediaTranscoder.uprightTransform(natural: natural, preferred: preferred, renderSize: render))
            #expect(abs(placed.minX) < 0.001 && abs(placed.minY) < 0.001, "\(placed)")
            #expect(abs(placed.width - 720) < 0.001 && abs(placed.height - 1280) < 0.001, "\(placed)")
        }
    }

    /// Apple's AAC-LC encoder refuses a bitrate outside its range for the
    /// rate and channel count; the fitting keeps the protocol's number when
    /// any sample rate allows it and never goes above it when none does.
    @Test("an AAC format is found for the planner's bitrate, never above it")
    func aacFormatFitting() throws {
        let stereo = try #require(MediaTranscoder.AACFormat.fitting(
            bitrate: 128_000, channels: 2, sourceSampleRate: 44_100))
        #expect(stereo == .init(sampleRate: 44_100, bitrate: 128_000), "the source's own rate when it can")

        let voice = try #require(MediaTranscoder.AACFormat.fitting(
            bitrate: 64_000, channels: 1, sourceSampleRate: 16_000))
        #expect(voice.bitrate == 64_000, "the protocol's number, by moving the sample rate")
        #expect(voice.sampleRate > 16_000)

        let thin = try #require(MediaTranscoder.AACFormat.fitting(
            bitrate: 20_000, channels: 1, sourceSampleRate: 48_000))
        #expect(thin.bitrate == 20_000)
        #expect(thin.sampleRate < 44_100, "a low rate needs a lower sample rate")

        let hires = try #require(MediaTranscoder.AACFormat.fitting(
            bitrate: 128_000, channels: 2, sourceSampleRate: 96_000))
        #expect(hires.sampleRate == 48_000, "AAC-LC tops out at 48 kHz")

        #expect(MediaTranscoder.AACFormat.fitting(bitrate: 1_000, channels: 1, sourceSampleRate: 8_000) == nil,
                "nothing is raised to reach the encoder's floor")
    }
}
