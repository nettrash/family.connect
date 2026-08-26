//
//  ClipboardPasteTests.swift
//  FamilyConnectTests
//
//  Paste-to-attach. There is no protocol change here, so what these check
//  is the two decisions the clipboard forces that a picker never does —
//  WHICH representation to take, and WHAT TO CALL IT — plus the rule that
//  keeps an ordinary text paste landing in the composer.
//
//  Nothing touches the real pasteboard: reading it would clobber whatever
//  the person running the suite had copied, and the platform halves of
//  ClipboardAttachment are a thin shell over these decisions. The shell
//  itself needs a device (see the report).
//

import Foundation
import SwiftUI
import Testing
import UniformTypeIdentifiers
@testable import FamilyConnect

@MainActor
@Suite("Paste to attach")
struct ClipboardPasteTests {

    // MARK: - Which representation

    /// The one that matters: a copied animation is on the clipboard as a
    /// GIF *and* as a flattened PNG or TIFF, and taking the flat one posts
    /// a perfectly good still of a moving picture.
    @Test("An animation wins over the flattened copy of itself")
    func animationWinsOverStill() {
        #expect(ClipboardAttachment.chosenType(from: [.tiff, .png, .gif]) == .gif)
        #expect(ClipboardAttachment.chosenType(from: [.png, .webP]) == .webP)
        // Order on the clipboard must not decide it.
        #expect(ClipboardAttachment.chosenType(from: [.gif, .png]) == .gif)
    }

    @Test("A type nobody listed is still attachable when it can be named")
    func unlistedTypeFallsThrough() {
        #expect(ClipboardAttachment.chosenType(from: [.zip]) == .zip)
        #expect(ClipboardAttachment.isAttachable(.zip))
    }

    @Test("Words and links are never attachments")
    func wordsAreNotAttachments() {
        #expect(ClipboardAttachment.chosenType(from: [.utf8PlainText, .plainText]) == nil)
        #expect(ClipboardAttachment.chosenType(from: [.rtf, .html]) == nil)
        #expect(ClipboardAttachment.chosenType(from: [.url]) == nil)
        #expect(!ClipboardAttachment.isAttachable(.text))
        #expect(!ClipboardAttachment.isAttachable(.html))
    }

    /// A file URL is bytes somewhere else, not bytes here: it has its own
    /// path through `prepare`, where the security scope is held open.
    @Test("A file URL is not taken as data")
    func fileURLIsNotData() {
        #expect(ClipboardAttachment.chosenType(from: [.fileURL]) == nil)
        #expect(!ClipboardAttachment.isAttachable(.fileURL))
    }

    /// An extension nothing on the system has ever heard of resolves to a
    /// DYNAMIC type — a placeholder identifier invented on the spot. It
    /// tells a recipient's device nothing, so it is not worth a message.
    @Test("A type nothing can name is refused")
    func dynamicTypeIsRefused() throws {
        let invented = try #require(UTType(filenameExtension: "nettrashthing"))
        #expect(invented.isDynamic)
        #expect(!ClipboardAttachment.isAttachable(invented))
        #expect(ClipboardAttachment.chosenType(from: [invented]) == nil)
    }

    // MARK: - Whether ⌘V attaches at all

    /// The rule that keeps the ordinary paste working.
    @Test("Words win, except against a copied file")
    func pasteGesturePrefersWords() {
        // Text on the clipboard: the composer's field takes it, always.
        #expect(!ClipboardAttachment.prefersAttachment(
            hasFileURL: false, hasAttachableData: true, hasText: true))
        // ...but every Finder copy puts the file's NAME on the clipboard as
        // text too, so words cannot be the test there.
        #expect(ClipboardAttachment.prefersAttachment(
            hasFileURL: true, hasAttachableData: false, hasText: true))
        // An image and nothing else.
        #expect(ClipboardAttachment.prefersAttachment(
            hasFileURL: false, hasAttachableData: true, hasText: false))
        // Nothing at all.
        #expect(!ClipboardAttachment.prefersAttachment(
            hasFileURL: false, hasAttachableData: false, hasText: false))
    }

    // MARK: - What to call it

    @Test("The item's own name is what the recipient sees")
    func suggestedNameWins() {
        #expect(ClipboardAttachment.suggestedName(
            for: .pdf, suggested: "Rechnung März.pdf") == "Rechnung März.pdf")
    }

    @Test("A suggested name with no extension gains the right one")
    func suggestedNameGainsExtension() {
        #expect(ClipboardAttachment.suggestedName(for: .pdf, suggested: "Invoice") == "Invoice.pdf")
        #expect(ClipboardAttachment.suggestedName(for: .png, suggested: "Shot") == "Shot.png")
    }

    /// `kind=file` requires 1–255 characters and a clipboard item usually
    /// has none. What it must NEVER be is the scratch file's name.
    @Test("A nameless item is named from its type, never from the temp file")
    func synthesisedName() {
        #expect(ClipboardAttachment.suggestedName(for: .gif, suggested: nil) == "Pasted image.gif")
        #expect(ClipboardAttachment.suggestedName(for: .png, suggested: "   ") == "Pasted image.png")
        #expect(ClipboardAttachment.suggestedName(for: .pdf, suggested: nil) == "Pasted file.pdf")
        #expect(ClipboardAttachment.suggestedName(for: .mpeg4Movie, suggested: nil)
            == "Pasted file.mp4")
    }

    @Test("A name is cleaned, and capped at what the protocol allows")
    func namesAreCleaned() throws {
        #expect(MediaPrep.sanitizedName(nil) == nil)
        #expect(MediaPrep.sanitizedName("   ") == nil)
        #expect(MediaPrep.sanitizedName("  taxes.zip  ") == "taxes.zip")
        // A name lands on somebody else's disk, so it cannot carry a path.
        #expect(MediaPrep.sanitizedName("../../etc/passwd") == ".._.._etc_passwd")

        let long = String(repeating: "a", count: 400) + ".pdf"
        let capped = try #require(MediaPrep.sanitizedName(long))
        #expect(capped.count == MediaPrep.maxNameLength)
        // Truncated at the STEM, so the recipient's system still knows what
        // it is holding.
        #expect(capped.hasSuffix(".pdf"))
    }

    // MARK: - Which kind it is sent as

    @Test("An animated format is a file, not a photo")
    func animatedFormatsAreFiles() {
        #expect(MediaPrep.sendsAsFile(imageType: .gif))
        #expect(MediaPrep.sendsAsFile(imageType: .webP))
        #expect(MediaPrep.sendsAsFile(imageType: .bmp))
        #expect(!MediaPrep.sendsAsFile(imageType: .jpeg))
        #expect(!MediaPrep.sendsAsFile(imageType: .png))
        #expect(!MediaPrep.sendsAsFile(imageType: .heic))
        #expect(!MediaPrep.sendsAsFile(imageType: .heif))
        #expect(!MediaPrep.sendsAsFile(imageType: nil))
    }

    /// The defect this feature exposed: the Mac's own picker sent a GIF
    /// down the photo path, where `preparePhoto` re-encodes frame zero to
    /// JPEG and the animation is gone without a word.
    @Test("A GIF file keeps its bytes and goes as a file")
    func gifFileIsNotReEncoded() async throws {
        let bytes = gifBytes()
        let source = MediaPrep.temporaryURL(extension: "gif")
        try bytes.write(to: source)
        defer { try? FileManager.default.removeItem(at: source) }

        let prepared = try await MediaPrep.prepare(fileAt: source, limit: MediaPrep.sizeLimit)
        defer { try? FileManager.default.removeItem(at: prepared.fileURL) }

        #expect(prepared.kind == AttachmentDTO.Kind.file)
        #expect(prepared.mime == "image/gif")
        #expect(try Data(contentsOf: prepared.fileURL) == bytes)
        #expect(prepared.previewJPEG == nil)
    }

    @Test("Pasted GIF bytes travel whole, under a name a person can read")
    func pastedGIFIsAFile() async throws {
        let bytes = gifBytes()
        let prepared = try await MediaPrep.prepare(
            data: bytes,
            type: .gif,
            name: ClipboardAttachment.suggestedName(for: .gif, suggested: nil),
            limit: MediaPrep.sizeLimit)
        defer { try? FileManager.default.removeItem(at: prepared.fileURL) }

        #expect(prepared.kind == AttachmentDTO.Kind.file)
        #expect(prepared.name == "Pasted image.gif")
        #expect(try Data(contentsOf: prepared.fileURL) == bytes)
        // The file on disk IS a scratch file — which is exactly why the
        // name may not come from it.
        #expect(prepared.fileURL.lastPathComponent.hasPrefix("fc-upload-"))
        #expect(prepared.name?.contains("fc-upload") == false)
    }

    /// One preparation path, not two: a pasted photo is downscaled and
    /// given a preview exactly as a picked one is.
    @Test("A pasted photo goes through the ordinary photo preparation")
    func pastedPhotoIsPrepared() async throws {
        let prepared = try await MediaPrep.prepare(
            data: TestImages.solid(width: 4032, height: 3024),
            type: .jpeg,
            limit: MediaPrep.sizeLimit)
        defer { try? FileManager.default.removeItem(at: prepared.fileURL) }

        #expect(prepared.kind == AttachmentDTO.Kind.photo)
        #expect(prepared.mime == "image/jpeg")
        #expect(prepared.width == MediaPrep.photoEdge)
        #expect(prepared.previewJPEG != nil)
        // A photo carries no name on the wire; the bubble draws the picture.
        #expect(prepared.name == nil)
    }

    @Test("A pasted document over the ceiling is refused before any upload")
    func oversizedPasteIsRefused() async throws {
        await #expect(throws: MediaPrep.PrepError.self) {
            _ = try await MediaPrep.prepare(
                data: Data(repeating: 0x41, count: 4096),
                type: .pdf,
                name: "Big.pdf",
                limit: 1024)
        }
    }

    @Test("Bytes nothing can name still get a file that opens somewhere")
    func unnamedTypeStillPrepares() async throws {
        let prepared = try await MediaPrep.prepare(
            data: Data("hello".utf8),
            type: .zip,
            name: ClipboardAttachment.suggestedName(for: .zip, suggested: "backup"),
            limit: MediaPrep.sizeLimit)
        defer { try? FileManager.default.removeItem(at: prepared.fileURL) }
        #expect(prepared.kind == AttachmentDTO.Kind.file)
        #expect(prepared.name == "backup.zip")
        #expect(prepared.mime == "application/zip")
    }

    // MARK: - Names that were never meant to be seen

    /// A voice note is recorded into `fc-voice-<UUID>.m4a`, and reading the
    /// file name here put that on the wire and in the composer's chip.
    @Test("A recording has no name; a picked track keeps its own")
    func audioNameComesFromTheCaller() async throws {
        let source = MediaPrep.temporaryURL(extension: "m4a")
        try Data(repeating: 0x00, count: 64).write(to: source)
        defer { try? FileManager.default.removeItem(at: source) }

        let recorded = try await MediaPrep.prepareAudio(from: source, limit: MediaPrep.sizeLimit)
        defer { try? FileManager.default.removeItem(at: recorded.fileURL) }
        #expect(recorded.name == nil)
        #expect(recorded.kind == AttachmentDTO.Kind.audio)

        let picked = try await MediaPrep.prepareAudio(
            from: source, name: "Für Elise.m4a", limit: MediaPrep.sizeLimit)
        defer { try? FileManager.default.removeItem(at: picked.fileURL) }
        #expect(picked.name == "Für Elise.m4a")
    }

    @Test("A file name override beats the scratch file it was written into")
    func fileNameOverride() async throws {
        let source = MediaPrep.temporaryURL(extension: "pdf")
        try Data("%PDF-1.7".utf8).write(to: source)
        defer { try? FileManager.default.removeItem(at: source) }

        let prepared = try await MediaPrep.prepareFile(
            from: source, name: "Menu.pdf", limit: MediaPrep.sizeLimit)
        defer { try? FileManager.default.removeItem(at: prepared.fileURL) }
        #expect(prepared.name == "Menu.pdf")

        // With nothing to override it, the file's own name is still used —
        // which is right for a picked document and wrong for a scratch one,
        // so every clipboard door passes a name.
        let plain = try await MediaPrep.prepareFile(from: source, limit: MediaPrep.sizeLimit)
        defer { try? FileManager.default.removeItem(at: plain.fileURL) }
        #expect(plain.name == source.lastPathComponent)
    }

    // MARK: - The chip

    @Test("The chip calls a nameless attachment by its kind")
    func chipLabels() {
        #expect(staged(kind: AttachmentDTO.Kind.audio, name: nil).label == "Audio")
        #expect(staged(kind: AttachmentDTO.Kind.video, name: nil).label == "Video")
        #expect(staged(kind: AttachmentDTO.Kind.file, name: nil).label == "File")
        #expect(staged(kind: AttachmentDTO.Kind.photo, name: nil).label == "Photo")
        #expect(staged(kind: AttachmentDTO.Kind.file, name: "taxes.zip").label == "taxes.zip")
    }

    @Test("A sound gets a waveform, not a document icon")
    func chipPlaceholder() {
        #expect(staged(kind: AttachmentDTO.Kind.audio, name: nil).placeholderSymbol == "waveform")
        #expect(staged(kind: AttachmentDTO.Kind.file, name: nil).placeholderSymbol == "doc")
    }

    // MARK: - Helpers

    private func staged(kind: String, name: String?) -> StagedAttachment {
        StagedAttachment(prepared: MediaPrep.Prepared(
            fileURL: URL(fileURLWithPath: "/tmp/none"),
            mime: "application/octet-stream",
            kind: kind,
            name: name))
    }

    /// A real GIF header with a byte of payload — enough that anything
    /// which DID decode it would produce different bytes on the way out.
    private func gifBytes() -> Data {
        var bytes = Data("GIF89a".utf8)
        bytes.append(contentsOf: [0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00])
        bytes.append(contentsOf: [0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x21, 0xF9])
        bytes.append(contentsOf: [0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2C])
        bytes.append(contentsOf: [0x3B])
        return bytes
    }
}
