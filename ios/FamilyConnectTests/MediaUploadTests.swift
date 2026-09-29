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

    /// A landscape 1080p60 MP4 — sendable as it is, and never within the
    /// profile: the short side is 1080 and it runs at 60.
    private func landscape1080p60(seconds: Double = 1) async throws -> URL {
        try await MediaFixtures.write(.init(
            width: 1920, height: 1080, frameRate: 60, seconds: seconds, bitrate: 8_000_000,
            fileType: .mp4, noise: true))
    }

    // MARK: - Rule 5: a clip outside the profile is brought to it

    /// The phone's own case, and every property the profile table names.
    @Test("a portrait 1080p60 HDR-tagged camera clip becomes 720×1280 at 30 fps, H.264 High, SDR, moov first",
          .timeLimit(.minutes(2)))
    func portraitCameraClipBecomesTheProfile() async throws {
        let source = try await MediaFixtures.write(.init(
            width: 1920, height: 1080, frameRate: 60, seconds: 2, bitrate: 12_000_000,
            fileType: .mov,
            transform: MediaFixtures.portrait(storedWidth: 1920, storedHeight: 1080),
            audioChannels: 2, hdrTagged: true, noise: true))
        defer { remove(source) }
        let sourceBytes = MediaPrep.fileSize(of: source)

        // The source is what the test says it is: HDR-tagged, stored on its side.
        let sourceFacts = try await MediaFixtures.facts(of: source)
        #expect(sourceFacts.transferFunction == kCMFormatDescriptionTransferFunction_ITU_R_2100_HLG as String)
        #expect(sourceFacts.naturalSize == CGSize(width: 1920, height: 1080))

        // What the planner is handed — the precondition for everything below.
        let container = try #require(MediaProbe.videoContainer(of: source))
        let probed = await MediaProbe.video(at: source, container: container, sizeBytes: sourceBytes)
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

        let prepared = try await MediaPrep.prepareVideo(from: source, limit: MediaPrep.sizeLimit)
        defer { remove(prepared.fileURL) }

        #expect(prepared.fileURL != source)
        #expect(prepared.kind == AttachmentDTO.Kind.video)
        #expect(prepared.mime == "video/mp4")
        #expect(prepared.width == 720 && prepared.height == 1280, "the TURNED size is what the upload reports")
        #expect(abs((prepared.durationMS ?? 0) - 2000) <= 100)
        #expect(prepared.previewJPEG != nil, "a transcoded clip still gets its poster")
        #expect(MediaPrep.Magic.honest(url: prepared.fileURL, mime: "video/mp4"))
        #expect(MediaPrep.fileSize(of: prepared.fileURL) < sourceBytes)

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
        #expect(abs(out.frameCount - 60) <= 2, "\(out.frameCount) frames in two seconds")
        #expect(abs(out.durationSeconds - 2) < 0.1)
        // The bitrate an encoder hits is an average, and on two seconds of
        // noise it runs a little either side of the request.
        #expect(Double(out.videoDataRate) > 2_000_000 * 0.6, "\(out.videoDataRate)")
        #expect(Double(out.videoDataRate) < 2_000_000 * 1.25, "\(out.videoDataRate)")
        #expect(out.audioCodec == kAudioFormatMPEG4AAC)
        #expect(out.audioChannels == 2, "stereo stays stereo")
        let audioRate = Double(try #require(out.audioDataRate))
        #expect(abs(audioRate - Double(audioTarget)) < Double(audioTarget) * 0.05, "\(audioRate)")
    }

    // MARK: - Rule A: within the profile, left alone

    @Test("a 720p30 H.264 MP4 at about 1.5 Mbit/s is sent byte for byte as it is (rule A)",
          .timeLimit(.minutes(1)))
    func withinProfileIsUntouched() async throws {
        let source = try await MediaFixtures.write(.init(
            width: 1280, height: 720, frameRate: 30, seconds: 2, bitrate: 1_500_000,
            fileType: .mp4, audioChannels: 2, noise: true))
        defer { remove(source) }
        let before = try Data(contentsOf: source)

        let probed = await MediaProbe.video(
            at: source, container: try #require(MediaProbe.videoContainer(of: source)),
            sizeBytes: before.count)
        #expect(probed.container == "video/mp4")
        let rate = try #require(probed.videoBitrate)
        #expect(rate > 1_000_000 && rate <= 2_500_000, "the fixture is the ~1.5 Mbit/s clip it claims: \(rate)")
        #expect(MediaPlan.planVideo(probed) == .keep)

        let prepared = try await MediaPrep.prepareVideo(from: source, limit: MediaPrep.sizeLimit)

        #expect(prepared.fileURL == source, "the ORIGINAL goes, not a copy and not a re-encode")
        #expect(try Data(contentsOf: prepared.fileURL) == before)
        #expect(prepared.kind == AttachmentDTO.Kind.video)
        #expect(prepared.mime == "video/mp4")
        #expect(prepared.width == 1280 && prepared.height == 720)
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

    @Test("a transcoded video bigger than its sendable source is thrown away (rule D)", .timeLimit(.minutes(1)))
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

    @Test("a transcoded sound file bigger than its source is thrown away (rule D)", .timeLimit(.minutes(1)))
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

    // MARK: - Cancellation is not a failure

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
