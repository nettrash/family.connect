//
//  MediaTypeHonestyTests.swift
//  FamilyConnectTests
//
//  WHAT THIS CLIENT IS WILLING TO CLAIM ABOUT BYTES IT HAS NOT LOOKED AT.
//
//  The server verifies that a declared type matches what the bytes ARE for
//  every photo, video and audio upload, and refuses the whole upload when it
//  does not (docs/protocol.md: "a type that contradicts the kind, or bytes
//  that do not match the type declared, is `invalid_attachment`"). So a
//  client that types a file by its EXTENSION can be wrong in a way the
//  sender cannot act on — the send simply fails, again, for that file.
//
//  Two real ones, both of them the Mac's file picker (the phones' pickers
//  hand over photos and videos the system produced):
//
//    * a `.aac` file is usually raw ADTS, not ISO base media, and the
//      extension table called it `audio/mp4`;
//    * a `.mkv` conforms to `public.movie`, so it went down the video path
//      and was uploaded UNCHANGED as `video/mp4` — Matroska bytes claiming
//      an MP4 container. AVFoundation cannot re-encode Matroska either.
//
//  The protocol's own answer is the file path, "where nothing is verified",
//  and that is what these pin.
//

import Foundation
import Testing
import UniformTypeIdentifiers
@testable import FamilyConnect

@Suite("Media type honesty")
struct MediaTypeHonestyTests {

    /// Bytes on disk, with the extension a picker would have handed over.
    private func file(_ bytes: [UInt8], extension ext: String) throws -> URL {
        let url = MediaPrep.temporaryURL(extension: ext)
        try Data(bytes).write(to: url, options: .atomic)
        return url
    }

    /// An ISO base media header: any brand will do, the check is `ftyp` at 4.
    private var isoBaseMedia: [UInt8] {
        [0x00, 0x00, 0x00, 0x18] + Array("ftypmp42".utf8) + [0x00, 0x00, 0x00, 0x00]
    }

    /// Matroska's own signature — an EBML header.
    private var matroska: [UInt8] {
        [0x1A, 0x45, 0xDF, 0xA3, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x23]
    }

    /// A raw AAC frame: ADTS sync, which is what most `.aac` files are.
    private var adts: [UInt8] {
        [0xFF, 0xF1, 0x50, 0x80, 0x00, 0x1F, 0xFC, 0x00, 0x00, 0x00, 0x00, 0x00]
    }

    @Test("the magic table answers what the server's answers")
    func magicTableMatchesTheServers() {
        #expect(MediaPrep.Magic.matches(mime: "video/mp4", head: Data(isoBaseMedia)))
        #expect(MediaPrep.Magic.matches(mime: "audio/mp4", head: Data(isoBaseMedia)))
        #expect(MediaPrep.Magic.matches(mime: "image/heic", head: Data(isoBaseMedia)))
        #expect(!MediaPrep.Magic.matches(mime: "video/mp4", head: Data(matroska)))
        #expect(!MediaPrep.Magic.matches(mime: "audio/mp4", head: Data(adts)))

        #expect(MediaPrep.Magic.matches(mime: "image/jpeg", head: Data([0xFF, 0xD8, 0xFF, 0xE0])))
        #expect(MediaPrep.Magic.matches(
            mime: "image/png", head: Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])))
        // An MP3 is a tag or a frame sync, and an MP3 frame sync is NOT an ADTS one to look at —
        // but it is to this check, which is why `.aac` must not be offered as `audio/mpeg`.
        #expect(MediaPrep.Magic.matches(mime: "audio/mpeg", head: Data(Array("ID3\u{03}".utf8))))
        #expect(MediaPrep.Magic.matches(mime: "audio/wav", head: Data(
            Array("RIFF".utf8) + [0, 0, 0, 0] + Array("WAVE".utf8))))
        #expect(MediaPrep.Magic.matches(mime: "audio/ogg", head: Data(Array("OggS".utf8))))
        // A type the server would not take at all is never honest to claim.
        #expect(!MediaPrep.Magic.matches(mime: "video/x-matroska", head: Data(matroska)))
        #expect(!MediaPrep.Magic.matches(mime: "audio/aac", head: Data(adts)))
        // And too few bytes to judge is not a pass.
        #expect(!MediaPrep.Magic.matches(mime: "video/mp4", head: Data([0x00, 0x00])))
    }

    @Test("a file nobody can read is not honest to claim anything about")
    func unreadableFileIsNotHonest() {
        let missing = MediaPrep.temporaryURL(extension: "mp4")

        #expect(!MediaPrep.Magic.honest(url: missing, mime: "video/mp4"))
    }

    /// THE REGRESSION, audio half: raw ADTS under a `.aac` name.
    @Test("a raw AAC file is not offered as audio the server will check")
    func rawAACIsNotClaimedAsMP4() throws {
        let url = try file(adts, extension: "aac")
        defer { try? FileManager.default.removeItem(at: url) }

        // The extension table still says what such a file WOULD be called…
        #expect(MediaPrep.audioMIME(for: url) == "audio/mp4")
        // …and the bytes say it must not be sent as that.
        #expect(!MediaPrep.isSupportedAudio(url))
    }

    @Test("an m4a that really is one is still sent as audio")
    func realM4AIsStillAudio() throws {
        let url = try file(isoBaseMedia, extension: "m4a")
        defer { try? FileManager.default.removeItem(at: url) }

        #expect(MediaPrep.audioMIME(for: url) == "audio/mp4")
        #expect(MediaPrep.isSupportedAudio(url))
    }

    @Test("an MP3, a WAV and an OGG are all still audio")
    func theOtherThreeContainersAreStillAudio() throws {
        let mp3 = try file(Array("ID3\u{03}\u{00}".utf8) + [0, 0, 0, 0, 0, 0, 0], extension: "mp3")
        let wav = try file(Array("RIFF".utf8) + [0, 0, 0, 0] + Array("WAVE".utf8), extension: "wav")
        let ogg = try file(Array("OggS".utf8) + [0, 0, 0, 0, 0, 0, 0, 0], extension: "ogg")
        defer {
            for url in [mp3, wav, ogg] { try? FileManager.default.removeItem(at: url) }
        }

        #expect(MediaPrep.isSupportedAudio(mp3))
        #expect(MediaPrep.isSupportedAudio(wav))
        #expect(MediaPrep.isSupportedAudio(ogg))
    }

    /// A renamed file is the same defect one extension over: the name says
    /// `.m4a`, the bytes are an MP3, and the server checks the bytes.
    @Test("a renamed audio file is sent as a file rather than refused")
    func renamedAudioIsNotClaimed() throws {
        let url = try file(Array("ID3\u{03}".utf8) + [0, 0, 0, 0, 0, 0, 0, 0], extension: "m4a")
        defer { try? FileManager.default.removeItem(at: url) }

        #expect(!MediaPrep.isSupportedAudio(url))
    }

    /// THE REGRESSION, video half. A `.mkv` under the size ceiling was
    /// uploaded UNCHANGED with a flat `video/mp4` — the one path that does
    /// not re-encode, and therefore the one that must check.
    @Test("a Matroska file under the ceiling is sent as a file, not as an MP4")
    func matroskaIsSentAsAFile() async throws {
        let url = try file(matroska, extension: "mkv")
        defer { try? FileManager.default.removeItem(at: url) }

        let prepared = try await MediaPrep.prepareVideo(from: url, limit: 10_000_000)
        defer { try? FileManager.default.removeItem(at: prepared.fileURL) }

        #expect(prepared.kind == "file")
        #expect(prepared.mime != "video/mp4")
        // A file carries its name, which is its whole identity.
        #expect(prepared.name?.hasSuffix(".mkv") == true)
    }

    @Test("a real MP4 under the ceiling is still sent as a video, unchanged")
    func realMP4IsStillAVideo() async throws {
        let url = try file(isoBaseMedia, extension: "mp4")
        defer { try? FileManager.default.removeItem(at: url) }

        let prepared = try await MediaPrep.prepareVideo(from: url, limit: 10_000_000)

        #expect(prepared.kind == "video")
        #expect(prepared.mime == "video/mp4")
        // The source itself: the one path that does not re-encode.
        #expect(prepared.fileURL == url)
    }
}
