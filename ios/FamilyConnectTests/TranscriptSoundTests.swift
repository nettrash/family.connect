//
//  TranscriptSoundTests.swift
//  FamilyConnectTests
//
//  The text of a video, an Ogg file or a recording over the ceiling: sound
//  this device takes out of the file and sends with the request
//  (docs/protocol.md, "Transcripts on request" — the multipart form).
//
//  What is decided (pass through / re-encode / cannot), what the request
//  looks like on the wire, the size bound, what the row is told, and REAL
//  extractions through AVFoundation from files written in the test: an MP4
//  and a MOV with an AAC track, a video with no sound, a WAV, and an Ogg
//  Opus voice note muxed here byte by byte (Firefox records exactly that).
//

import AVFoundation
import AudioToolbox
import Foundation
import Testing
@testable import FamilyConnect

// MARK: - Fixtures

enum SoundFixtures {
    static let ceiling: Int64 = 26_214_400

    static func attachment(
        id: Int64 = 51, kind: String = "video", mime: String = "video/mp4",
        size: Int64 = 40_000_000, durationMS: Int? = 60_000
    ) -> AttachmentDTO {
        AttachmentDTO(
            id: id, kind: kind, mime: mime, size: size, width: nil, height: nil,
            durationMS: durationMS, hasPreview: false, name: nil,
            latitude: nil, longitude: nil, accuracyM: nil)
    }

    static func aac(channels: Int? = 2, bitrate: Int? = 128_000) -> TranscriptSound.Track {
        TranscriptSound.Track(codec: "aac", channels: channels, bitrate: bitrate)
    }

    static func folder() throws -> URL {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("fc-sound-test-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    /// Bytes that START like an M4A (`ftyp` at 4), which is all the server
    /// checks of a supplied part, followed by some that are not text.
    static func fakeM4A(in folder: URL, count: Int = 3000) throws -> URL {
        var bytes: [UInt8] = [0, 0, 0, 0x18] + Array("ftypM4A ".utf8)
        bytes += (0..<count).map { UInt8(truncatingIfNeeded: $0 &* 31 &+ 7) }
        let url = folder.appendingPathComponent("sound.m4a")
        try Data(bytes).write(to: url)
        return url
    }

    /// The brand after `ftyp`, or nil when the file is not ISO base media.
    static func brand(of url: URL) throws -> String? {
        let head = [UInt8](try Data(contentsOf: url).prefix(12))
        guard head.count == 12, Array(head[4..<8]) == Array("ftyp".utf8) else { return nil }
        return String(decoding: head[8..<12], as: UTF8.self)
    }

    static func size(of url: URL) -> Int64 {
        Int64((try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0)
    }
}

/// An Ogg Opus file, as Firefox's recorder writes a voice note — muxed here
/// from Opus packets the system encoder makes, because no Apple API WRITES
/// Ogg (only reads it). RFC 7845's three parts: an `OpusHead` page, an
/// `OpusTags` page, then one page per packet with its granule position, and
/// RFC 3533's page CRC (polynomial 0x04C11DB7, not reflected).
enum OggOpusFixture {
    enum Failure: Error { case encode(String) }

    static func write(seconds: Double) throws -> URL {
        // Opus packets, in a CAF — the one container Apple writes them to.
        let caf = MediaFixtures.scratch("caf")
        defer { try? FileManager.default.removeItem(at: caf) }
        do {
            let file = try AVAudioFile(forWriting: caf, settings: [
                AVFormatIDKey: kAudioFormatOpus,
                AVSampleRateKey: 48_000,
                AVNumberOfChannelsKey: 1,
            ])
            let format = file.processingFormat
            let frames = AVAudioFrameCount(48_000 * seconds)
            guard let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: frames),
                  let samples = buffer.floatChannelData
            else { throw Failure.encode("buffer") }
            buffer.frameLength = frames
            for index in 0..<Int(frames) {
                samples[0][index] = Float(0.25 * sin(Double(index) * 2 * Double.pi * 330 / 48_000))
            }
            try file.write(from: buffer)
        }

        var fileID: AudioFileID?
        guard AudioFileOpenURL(caf as CFURL, .readPermission, 0, &fileID) == noErr, let fileID else {
            throw Failure.encode("open caf")
        }
        defer { AudioFileClose(fileID) }
        var format = AudioStreamBasicDescription()
        var size = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)
        AudioFileGetProperty(fileID, kAudioFilePropertyDataFormat, &size, &format)
        var packets: UInt64 = 0
        size = UInt32(MemoryLayout<UInt64>.size)
        AudioFileGetProperty(fileID, kAudioFilePropertyAudioDataPacketCount, &size, &packets)
        var largest: UInt32 = 0
        size = UInt32(MemoryLayout<UInt32>.size)
        AudioFileGetProperty(fileID, kAudioFilePropertyMaximumPacketSize, &size, &largest)
        guard packets > 0, largest > 0, format.mFramesPerPacket > 0 else {
            throw Failure.encode("no packets")
        }

        var ogg = Ogg()
        let head: [UInt8] = Array("OpusHead".utf8) + [1, 1] + le(UInt16(312), 2)
            + le(UInt32(48_000), 4) + le(UInt16(0), 2) + [0]
        ogg.page([head], granule: 0, flags: 0x02)
        let vendor = Array("family.connect tests".utf8)
        ogg.page(
            [Array("OpusTags".utf8) + le(UInt32(vendor.count), 4) + vendor + le(UInt32(0), 4)],
            granule: 0, flags: 0)
        var granule: UInt64 = 0
        for index in 0..<Int64(packets) {
            var bytes = largest
            var count: UInt32 = 1
            var buffer = [UInt8](repeating: 0, count: Int(largest))
            var description = AudioStreamPacketDescription()
            guard AudioFileReadPacketData(
                fileID, false, &bytes, &description, index, &count, &buffer) == noErr, count == 1
            else { throw Failure.encode("packet \(index)") }
            granule += UInt64(format.mFramesPerPacket)
            ogg.page(
                [Array(buffer[0..<Int(bytes)])], granule: granule,
                flags: index + 1 == Int64(packets) ? 0x04 : 0)
        }
        let url = MediaFixtures.scratch("ogg")
        try Data(ogg.bytes).write(to: url)
        return url
    }

    private static func le<T: FixedWidthInteger>(_ value: T, _ width: Int) -> [UInt8] {
        (0..<width).map { UInt8(truncatingIfNeeded: value >> (8 * $0)) }
    }

    private struct Ogg {
        var bytes: [UInt8] = []
        private var sequence: UInt32 = 0
        private static let table: [UInt32] = (0..<256).map { index in
            var r = UInt32(index) << 24
            for _ in 0..<8 { r = r & 0x8000_0000 != 0 ? (r << 1) ^ 0x04C1_1DB7 : r << 1 }
            return r
        }

        mutating func page(_ packets: [[UInt8]], granule: UInt64, flags: UInt8) {
            var lacing: [UInt8] = []
            for packet in packets {
                var left = packet.count
                while left >= 255 { lacing.append(255); left -= 255 }
                lacing.append(UInt8(left))
            }
            var page: [UInt8] = Array("OggS".utf8) + [0, flags] + le(granule, 8)
                + le(UInt32(0x4643_5354), 4) + le(sequence, 4) + le(UInt32(0), 4)
                + [UInt8(lacing.count)] + lacing
            page += packets.flatMap { $0 }
            let crc = page.reduce(UInt32(0)) { ($0 << 8) ^ Self.table[Int((($0 >> 24) ^ UInt32($1)) & 0xFF)] }
            page.replaceSubrange(22..<26, with: le(crc, 4))
            sequence += 1
            bytes += page
        }
    }
}

// MARK: - Deciding

@Suite("Transcript sound: the decision")
struct TranscriptSoundDecisionTests {
    typealias F = SoundFixtures
    typealias S = TranscriptSound

    @Test("an AAC track that fits is passed through untouched")
    func passThrough() {
        #expect(S.treatment(track: F.aac(), durationMS: 60_000, maxBytes: nil) == .success(.passThrough))
        #expect(S.treatment(track: F.aac(channels: 1, bitrate: 64_000), durationMS: 600_000, maxBytes: nil)
            == .success(.passThrough))
        // Its rate or length unknown: passed through, and measured after.
        #expect(S.treatment(track: F.aac(bitrate: nil), durationMS: 60_000, maxBytes: nil)
            == .success(.passThrough))
        #expect(S.treatment(track: F.aac(), durationMS: nil, maxBytes: nil) == .success(.passThrough))
        // A low-rate AAC track carries a recording a re-encode could not:
        // an hour at 32 kbit/s is 14.4 MB, an hour at 64 is 28.8.
        #expect(S.treatment(track: F.aac(channels: 1, bitrate: 32_000), durationMS: 3_600_000, maxBytes: nil)
            == .success(.passThrough))
    }

    @Test("anything else is re-encoded to 64 kbit/s mono")
    func reencode() {
        for codec in ["mp3", "pcm", "opus", "alac", "flac", "unknown"] {
            let track = S.Track(codec: codec, channels: 2, bitrate: 256_000)
            #expect(S.treatment(track: track, durationMS: 60_000, maxBytes: nil) == .success(.reencode),
                    "\(codec)")
        }
        // AAC, but not mono or stereo.
        #expect(S.treatment(track: F.aac(channels: 6), durationMS: 60_000, maxBytes: nil)
            == .success(.reencode))
        #expect(S.treatment(track: F.aac(channels: nil), durationMS: 60_000, maxBytes: nil)
            == .success(.reencode))
        // AAC too big to pass: 40 minutes at 128 kbit/s is 38 MB; at 64 mono, 19.
        #expect(S.treatment(track: F.aac(), durationMS: 2_400_000, maxBytes: nil) == .success(.reencode))
        // Length unknown: worth trying.
        #expect(S.treatment(track: S.Track(codec: "mp3", channels: 2, bitrate: nil),
                            durationMS: nil, maxBytes: nil) == .success(.reencode))
        #expect(S.bitrate == 64_000)
    }

    @Test("no sound track, or one this OS cannot open: unreadable")
    func unreadable() {
        #expect(S.treatment(track: nil, durationMS: 60_000, maxBytes: nil) == .failure(.unreadable))
        #expect(S.treatment(track: nil, durationMS: nil, maxBytes: nil) == .failure(.unreadable))
    }

    @Test("over the ceiling even at 64 kbit/s: too long")
    func tooLong() {
        // An hour of MP3: 28.8 MB at 64 kbit/s.
        let mp3 = S.Track(codec: "mp3", channels: 2, bitrate: 128_000)
        #expect(S.treatment(track: mp3, durationMS: 3_600_000, maxBytes: nil) == .failure(.tooLong))
        #expect(S.treatment(track: F.aac(), durationMS: 3_600_000, maxBytes: nil) == .failure(.tooLong))
        // A server that lowered its ceiling lowers this one.
        #expect(S.treatment(track: mp3, durationMS: 60_000, maxBytes: 100_000) == .failure(.tooLong))
        #expect(S.treatment(track: mp3, durationMS: 60_000, maxBytes: 1_000_000) == .success(.reencode))
        // Never above the protocol's, whatever a server claims.
        #expect(S.treatment(track: mp3, durationMS: 3_600_000, maxBytes: 90_000_000) == .failure(.tooLong))
    }

    /// Said before a byte is downloaded — and only when it must be true.
    @Test("a length that proves it too long is known without downloading")
    func knownTooLong() {
        #expect(!S.knownTooLong(F.attachment(durationMS: 50 * 60_000), maxBytes: nil))
        #expect(S.knownTooLong(F.attachment(durationMS: 55 * 60_000), maxBytes: nil))
        #expect(!S.knownTooLong(F.attachment(durationMS: nil), maxBytes: nil))
        #expect(!S.knownTooLong(F.attachment(durationMS: 0), maxBytes: nil))
        #expect(S.knownTooLong(F.attachment(durationMS: 60_000), maxBytes: 100_000))
        // The estimate is never under what a 64 kbit/s track really costs.
        for minutes in [1, 10, 30, 50] {
            let ms = minutes * 60_000
            #expect(S.reencodedBytes(durationMS: ms) > Int64(ms) * 8, "\(minutes) min")
        }
    }

    @Test("the size bound is the server's check of the part")
    func fits() {
        #expect(S.fits(bytes: F.ceiling, maxBytes: nil))
        #expect(!S.fits(bytes: F.ceiling + 1, maxBytes: nil))
        #expect(!S.fits(bytes: 0, maxBytes: nil))
        #expect(S.fits(bytes: 1000, maxBytes: 1000))
        #expect(!S.fits(bytes: 1001, maxBytes: 1000))
        #expect(!S.fits(bytes: F.ceiling + 1, maxBytes: 90_000_000))
    }

    @Test("a downloaded file is named so AVFoundation can open it")
    func extensions() {
        #expect(S.fileExtension(for: "audio/ogg") == "ogg")
        #expect(S.fileExtension(for: "audio/mp4") == "m4a")
        #expect(S.fileExtension(for: "audio/mpeg") == "mp3")
        #expect(S.fileExtension(for: "audio/wav") == "wav")
        #expect(S.fileExtension(for: "video/quicktime") == "mov")
        #expect(S.fileExtension(for: "video/mp4") == "mp4")
        #expect(S.fileExtension(for: "video/x-something") == "mp4")
        #expect(S.fileExtension(for: "audio/x-something") == "m4a")
    }
}

// MARK: - The request

@Suite("Transcript sound: the multipart request")
struct TranscriptSoundRequestTests {
    typealias F = SoundFixtures

    private func client(host: String, handler: @escaping StubURLProtocol.Handler) -> APIClient {
        StubURLProtocol.register(host: host, handler: handler)
        return APIClient(
            serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())
    }

    @Test("POST multipart/form-data, one part `audio`, audio/mp4, the bytes exactly, its own timeout")
    func shape() async throws {
        let host = "transcript-supplied-shape.test"
        defer { StubURLProtocol.unregister(host: host) }
        let api = client(host: host) { _ in
            .json(200, #"{"transcript": {"text": "Мы будем в шесть", "language": "ru"}}"#)
        }
        let folder = try F.folder()
        defer { try? FileManager.default.removeItem(at: folder) }
        let sound = try F.fakeM4A(in: folder)
        let soundBytes = try Data(contentsOf: sound)

        let answer = try await api.transcript(
            chatID: 42, messageID: 1338, attachmentID: 51, suppliedSound: sound)
        #expect(answer == TranscriptDTO(text: "Мы будем в шесть", language: "ru"))

        let call = try #require(StubURLProtocol.requests(host: host).first)
        #expect(call.method == "POST")
        #expect(call.url.path() == "/api/v1/chats/42/messages/1338/attachments/51/transcript")
        #expect(call.timeoutInterval >= 90)
        let contentType = try #require(call.headers["Content-Type"])
        #expect(contentType.hasPrefix("multipart/form-data; boundary="))
        let boundary = String(contentType.dropFirst("multipart/form-data; boundary=".count))
        #expect(!boundary.isEmpty)

        let expected = Data((
            "--\(boundary)\r\n"
            + "Content-Disposition: form-data; name=\"audio\"; filename=\"sound.m4a\"\r\n"
            + "Content-Type: audio/mp4\r\n\r\n").utf8)
            + soundBytes + Data("\r\n--\(boundary)--\r\n".utf8)
        #expect(call.body == expected)
        // The boundary is nowhere in the sound (it would end the part early).
        #expect(soundBytes.range(of: Data(boundary.utf8)) == nil)
        // The form file it was streamed from is gone; the sound is the
        // caller's and stays.
        let left = try FileManager.default.contentsOfDirectory(atPath: folder.path)
        #expect(left == ["sound.m4a"])
    }

    @Test("the form is written in chunks and holds a large part whole")
    func largeForm() throws {
        let folder = try F.folder()
        defer { try? FileManager.default.removeItem(at: folder) }
        let sound = try F.fakeM4A(in: folder, count: 3 << 20)
        let form = folder.appendingPathComponent("form.body")
        try APIClient.writeTranscriptForm(sound: sound, boundary: "b0undary", to: form)
        let body = try Data(contentsOf: form)
        let soundBytes = try Data(contentsOf: sound)
        let head = "--b0undary\r\nContent-Disposition: form-data; name=\"audio\"; filename=\"sound.m4a\"\r\nContent-Type: audio/mp4\r\n\r\n"
        #expect(body.count == head.utf8.count + soundBytes.count + "\r\n--b0undary--\r\n".utf8.count)
        #expect(body.prefix(head.utf8.count) == Data(head.utf8))
        #expect(body.dropFirst(head.utf8.count).prefix(soundBytes.count) == soundBytes)
    }

    @Test("refusals of the supplied form keep their codes and are never retried")
    func refusals() async throws {
        let cases: [(Int, String, TranscriptOutcome)] = [
            (400, "not_transcribable", .failed(.notAvailable)),
            (400, "validation", .failed(.retry)),
            (403, "assistant_consent_required", .consentRequired),
            (403, "transcript_not_allowed", .failed(.notAvailable)),
            (400, "transcript_refused", .failed(.refused)),
            (413, "too_large", .failed(.tooLong)),
            (500, "internal", .failed(.retry)),
        ]
        let folder = try F.folder()
        defer { try? FileManager.default.removeItem(at: folder) }
        let sound = try F.fakeM4A(in: folder)
        for (index, (status, code, expected)) in cases.enumerated() {
            let host = "transcript-supplied-refusal-\(index).test"
            defer { StubURLProtocol.unregister(host: host) }
            let api = client(host: host) { _ in .json(status, TranscriptFixtures.error(code)) }
            do {
                _ = try await api.transcript(
                    chatID: 1, messageID: 2, attachmentID: 3, suppliedSound: sound)
                Issue.record("\(code) did not throw")
            } catch {
                #expect(TranscriptOutcome(error: error) == expected, "\(code)")
            }
            #expect(StubURLProtocol.requests(host: host).count == 1, "\(code)")
        }
    }
}

// MARK: - The store's supplied route

@MainActor
@Suite("Transcript sound: asking", .serialized)
struct TranscriptSoundStoreTests {
    typealias F = SoundFixtures

    actor Seen {
        private(set) var folders: [URL] = []
        private(set) var calls = 0
        func add(_ folder: URL) { folders.append(folder); calls += 1 }
    }

    private func client(host: String, handler: @escaping StubURLProtocol.Handler) -> APIClient {
        StubURLProtocol.register(host: host, handler: handler)
        return APIClient(
            serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())
    }

    private func directory() -> URL {
        FileManager.default.temporaryDirectory
            .appendingPathComponent("transcripts-\(UUID().uuidString)", isDirectory: true)
    }

    @Test("a video: the device's sound goes as multipart, the answer is kept here as supplied")
    func suppliedKept() async throws {
        let host = "transcript-supplied-store.test"
        defer { StubURLProtocol.unregister(host: host) }
        let api = client(host: host) { _ in .json(200, #"{"transcript": {"text": "hello there"}}"#) }
        let seen = Seen()
        let store = TranscriptStore(api: api, directory: directory()) { attachment, maxBytes, folder in
            await seen.add(folder)
            #expect(attachment.id == 51)
            #expect(maxBytes == 1_000_000)
            return try SoundFixtures.fakeM4A(in: folder)
        }
        let outcome = await store.ask(
            chatID: 42, messageID: 1338, attachment: F.attachment(), maxBytes: 1_000_000)
        #expect(outcome == .text(TranscriptDTO(text: "hello there", language: nil)))
        #expect(store.kept(51)?.source == .supplied)
        #expect(store.kept(51)?.text == "hello there")
        #expect(store.activity(for: 51) == .idle)

        let call = try #require(StubURLProtocol.requests(host: host).first)
        #expect(call.headers["Content-Type"]?.hasPrefix("multipart/form-data") == true)
        // The scratch folder — download, sound and form — is gone.
        let folder = try #require(await seen.folders.first)
        #expect(!FileManager.default.fileExists(atPath: folder.path))
    }

    @Test("a voice note the server can send goes by the stored copy, with no sound made")
    func storedStaysStored() async throws {
        let host = "transcript-supplied-stored.test"
        defer { StubURLProtocol.unregister(host: host) }
        let api = client(host: host) { _ in .json(200, #"{"transcript": {"text": "stored"}}"#) }
        let seen = Seen()
        let store = TranscriptStore(api: api, directory: directory()) { _, _, folder in
            await seen.add(folder)
            return folder
        }
        let voice = F.attachment(id: 34, kind: "audio", mime: "audio/mp4", size: 120_000)
        let outcome = await store.ask(chatID: 1, messageID: 2, attachment: voice, maxBytes: nil)
        #expect(outcome == .text(TranscriptDTO(text: "stored", language: nil)))
        #expect(store.kept(34)?.source == .stored)
        #expect(await seen.calls == 0)
        let call = try #require(StubURLProtocol.requests(host: host).first)
        #expect(call.headers["Content-Type"] == nil)
    }

    /// The server's own reading of the stored copy can disagree with the
    /// attachment's metadata; its `not_transcribable` then sends the
    /// device's sound instead, as the protocol allows and Android and
    /// Windows do — under the one "Getting the text…", kept as supplied.
    @Test("a stored copy the server will not send goes on by the device's sound")
    func storedRefusedFallsBack() async throws {
        let host = "transcript-supplied-fallback.test"
        defer { StubURLProtocol.unregister(host: host) }
        let api = client(host: host) { request in
            let multipart = request.headers["Content-Type"]?
                .hasPrefix("multipart/form-data") == true
            return multipart
                ? .json(200, #"{"transcript": {"text": "from the device"}}"#)
                : .json(400, #"{"error": {"code": "not_transcribable", "message": "no"}}"#)
        }
        let seen = Seen()
        let store = TranscriptStore(api: api, directory: directory()) { _, _, folder in
            await seen.add(folder)
            return try SoundFixtures.fakeM4A(in: folder)
        }
        let voice = F.attachment(id: 34, kind: "audio", mime: "audio/mp4", size: 120_000)
        let outcome = await store.ask(chatID: 1, messageID: 2, attachment: voice, maxBytes: nil)
        #expect(outcome == .text(TranscriptDTO(text: "from the device", language: nil)))
        #expect(store.kept(34)?.source == .supplied)
        #expect(await seen.calls == 1)
        #expect(StubURLProtocol.requests(host: host).count == 2)

        // Any other refusal of the stored copy makes no sound.
        let refusing = client(host: host + ".2") { _ in
            .json(403, #"{"error": {"code": "transcript_not_allowed", "message": "no"}}"#)
        }
        defer { StubURLProtocol.unregister(host: host + ".2") }
        let other = TranscriptStore(api: refusing, directory: directory()) { _, _, folder in
            await seen.add(folder)
            return try SoundFixtures.fakeM4A(in: folder)
        }
        #expect(await other.ask(chatID: 1, messageID: 2, attachment: voice, maxBytes: nil)
            == .failed(.notAvailable))
        #expect(await seen.calls == 1)
    }

    @Test("what the device cannot do is said, terminally, and nothing is sent")
    func deviceFailures() async throws {
        let host = "transcript-supplied-fail.test"
        defer { StubURLProtocol.unregister(host: host) }
        let api = client(host: host) { _ in .json(200, #"{"transcript": {"text": "never"}}"#) }

        let tooLong = TranscriptStore(api: api, directory: directory()) { _, _, _ in
            throw TranscriptSound.Failure.tooLong
        }
        #expect(await tooLong.ask(chatID: 1, messageID: 2, attachment: F.attachment(), maxBytes: nil)
            == .failed(.tooLong))
        #expect(tooLong.activity(for: 51) == .failed(.tooLong))
        #expect(tooLong.kept(51) == nil)

        let unreadable = TranscriptStore(api: api, directory: directory()) { _, _, _ in
            throw TranscriptSound.Failure.unreadable
        }
        #expect(await unreadable.ask(chatID: 1, messageID: 2, attachment: F.attachment(), maxBytes: nil)
            == .failed(.unreadable))
        #expect(!TranscriptFailure.unreadable.offersRetry)

        // The download failed: worth asking again.
        let offline = TranscriptStore(api: api, directory: directory()) { _, _, _ in
            throw APIError.transport(URLError(.notConnectedToInternet))
        }
        #expect(await offline.ask(chatID: 1, messageID: 2, attachment: F.attachment(), maxBytes: nil)
            == .failed(.retry))

        #expect(StubURLProtocol.requests(host: host).isEmpty)
    }

    @Test("a photo has no sound to send, and nothing is asked")
    func photo() async throws {
        let host = "transcript-supplied-photo.test"
        defer { StubURLProtocol.unregister(host: host) }
        let api = client(host: host) { _ in .empty(500) }
        let store = TranscriptStore(api: api, directory: directory())
        let photo = F.attachment(kind: "photo", mime: "image/jpeg")
        #expect(await store.ask(chatID: 1, messageID: 2, attachment: photo, maxBytes: nil) == nil)
        #expect(StubURLProtocol.requests(host: host).isEmpty)
    }
}

// MARK: - Real extraction

/// AVFoundation doing it, on files written here. Serialized: each writes
/// and reads media, and the encoders are a shared resource.
@Suite("Transcript sound: extraction", .serialized)
struct TranscriptSoundExtractionTests {
    typealias F = SoundFixtures

    private static let clip = MediaFixtures.Clip(
        width: 320, height: 240, frameRate: 15, seconds: 1.5, bitrate: 400_000)

    @Test("a video's AAC track is passed through: stereo, its own rate, no picture", arguments: [
        AVFileType.mp4, AVFileType.mov,
    ])
    func videoPassThrough(fileType: AVFileType) async throws {
        var clip = Self.clip
        clip.fileType = fileType
        let source = try await MediaFixtures.write(clip)
        defer { try? FileManager.default.removeItem(at: source) }
        let folder = try F.folder()
        defer { try? FileManager.default.removeItem(at: folder) }

        let probed = await TranscriptSound.probe(source)
        #expect(probed.track?.codec == "aac")
        #expect(TranscriptSound.treatment(
            track: probed.track, durationMS: probed.durationMS, maxBytes: nil) == .success(.passThrough))

        let sound = try await TranscriptSound.extract(
            from: source, durationMS: nil, maxBytes: nil, into: folder)
        #expect(try F.brand(of: sound) == "M4A ")
        let asset = AVURLAsset(url: sound)
        #expect(try await asset.loadTracks(withMediaType: .video).isEmpty)
        let facts = try await MediaFixtures.audioFacts(of: sound)
        #expect(MediaProbe.audioCodec(facts.codec) == "aac")
        // The fixture's own track: stereo at 128 kbit/s. A re-encode would
        // have made it mono at 64.
        #expect(facts.channels == 2)
        #expect(facts.dataRate > 96_000)
        #expect(abs(facts.durationSeconds - clip.seconds) < 0.2)
        #expect(F.size(of: sound) < F.size(of: source))
    }

    @Test("a video with no sound cannot be read for any")
    func silentVideo() async throws {
        var clip = Self.clip
        clip.audioChannels = nil
        let source = try await MediaFixtures.write(clip)
        defer { try? FileManager.default.removeItem(at: source) }
        let folder = try F.folder()
        defer { try? FileManager.default.removeItem(at: folder) }
        await #expect(throws: TranscriptSound.Failure.unreadable) {
            _ = try await TranscriptSound.extract(
                from: source, durationMS: nil, maxBytes: nil, into: folder)
        }
    }

    @Test("a WAV is re-encoded to 64 kbit/s mono AAC")
    func wavReencoded() async throws {
        let source = try MediaFixtures.writeWAV(seconds: 2, sampleRate: 44_100, channels: 2)
        defer { try? FileManager.default.removeItem(at: source) }
        let folder = try F.folder()
        defer { try? FileManager.default.removeItem(at: folder) }

        let sound = try await TranscriptSound.extract(
            from: source, durationMS: nil, maxBytes: nil, into: folder)
        #expect(try F.brand(of: sound) == "M4A ")
        let facts = try await MediaFixtures.audioFacts(of: sound)
        #expect(MediaProbe.audioCodec(facts.codec) == "aac")
        #expect(facts.channels == 1)
        #expect(abs(Double(facts.dataRate) - 64_000) < 12_000, "\(facts.dataRate)")
        #expect(abs(facts.durationSeconds - 2) < 0.2)
    }

    @Test("an Ogg Opus voice note is read and re-encoded")
    func oggOpus() async throws {
        let source = try OggOpusFixture.write(seconds: 2)
        defer { try? FileManager.default.removeItem(at: source) }
        #expect(MediaPrep.Magic.honest(url: source, mime: "audio/ogg"))
        let folder = try F.folder()
        defer { try? FileManager.default.removeItem(at: folder) }

        let probed = await TranscriptSound.probe(source)
        #expect(probed.track?.codec == "opus")
        let sound = try await TranscriptSound.extract(
            from: source, durationMS: nil, maxBytes: nil, into: folder)
        #expect(try F.brand(of: sound) == "M4A ")
        let facts = try await MediaFixtures.audioFacts(of: sound)
        #expect(MediaProbe.audioCodec(facts.codec) == "aac")
        #expect(facts.channels == 1)
        #expect(abs(facts.durationSeconds - 2) < 0.3, "\(facts.durationSeconds)")
    }

    @Test("over a lowered ceiling the sound is too long, and nothing is left behind")
    func overCeiling() async throws {
        let source = try MediaFixtures.writeWAV(seconds: 2, sampleRate: 44_100, channels: 1)
        defer { try? FileManager.default.removeItem(at: source) }
        let folder = try F.folder()
        defer { try? FileManager.default.removeItem(at: folder) }
        await #expect(throws: TranscriptSound.Failure.tooLong) {
            _ = try await TranscriptSound.extract(
                from: source, durationMS: nil, maxBytes: 2_000, into: folder)
        }
    }

    /// The whole of it: downloaded through the attachment endpoint to a
    /// file named for its type, the sound taken out, the part returned.
    @Test("prepare: downloaded, extracted, the part returned")
    func prepare() async throws {
        let source = try await MediaFixtures.write(Self.clip)
        defer { try? FileManager.default.removeItem(at: source) }
        let bytes = try Data(contentsOf: source)
        let host = "transcript-sound-prepare.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { request in
            request.url.path() == "/api/v1/attachments/51"
                ? StubResponse(status: 200, headers: ["Content-Type": "video/mp4"], body: bytes)
                : .json(404, TranscriptFixtures.error("attachment_not_found"))
        }
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())
        let folder = try F.folder()
        defer { try? FileManager.default.removeItem(at: folder) }

        let video = F.attachment(size: Int64(bytes.count), durationMS: 1500)
        let sound = try await TranscriptSound.prepare(video, maxBytes: nil, api: api, into: folder)
        #expect(try F.brand(of: sound) == "M4A ")
        #expect(FileManager.default.fileExists(atPath: folder.appendingPathComponent("source.mp4").path))
        #expect(StubURLProtocol.requests(host: host).map(\.method) == ["GET"])

        // Gone from the server: a 404, which the row says as "not available".
        let missing = F.attachment(id: 52, durationMS: 1500)
        do {
            _ = try await TranscriptSound.prepare(missing, maxBytes: nil, api: api, into: folder)
            Issue.record("a 404 did not throw")
        } catch {
            #expect(TranscriptOutcome(error: error) == .failed(.notAvailable))
        }
    }

    @Test("prepare: a length that proves it too long downloads nothing")
    func prepareKnownTooLong() async throws {
        let host = "transcript-sound-too-long.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { _ in .empty(500) }
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())
        let folder = try F.folder()
        defer { try? FileManager.default.removeItem(at: folder) }
        await #expect(throws: TranscriptSound.Failure.tooLong) {
            _ = try await TranscriptSound.prepare(
                F.attachment(durationMS: 3 * 3_600_000), maxBytes: nil, api: api, into: folder)
        }
        #expect(StubURLProtocol.requests(host: host).isEmpty)
    }
}
