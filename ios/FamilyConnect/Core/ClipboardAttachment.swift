//
//  ClipboardAttachment.swift
//  FamilyConnect
//
//  What is on the clipboard, as something a message can carry.
//
//  Paste-to-attach is not a protocol feature: a pasted item becomes an
//  ordinary attachment upload followed by the existing claim-on-send
//  (docs/protocol.md, "Photos, videos and files"). Nothing new goes over
//  the wire. What this file owns is the two decisions the clipboard forces
//  that a picker never does:
//
//  1. WHICH of the several representations to take. A copied animation sits
//     on the clipboard as a GIF *and* as a flattened PNG or TIFF, and
//     taking the flat one is how an animation dies silently. The preference
//     list below is ordered so the richer representation wins.
//  2. WHAT TO CALL IT. A picked file has a name; a clipboard item usually
//     has none, and `kind=file` requires 1–255 characters. The item's own
//     suggested name is used when it has one, and otherwise a name is
//     synthesised from the type — never the `fc-upload-<UUID>.pdf` scratch
//     file the bytes were written into.
//
//  Preparation itself is NOT here. Everything goes through MediaPrep, so a
//  pasted photo is downscaled exactly like a picked one, a pasted clip gets
//  the same poster frame, and the single 100 MB ceiling is enforced in one
//  place.
//
//  Android counterpart: the composer's ClipData handling.
//

import Foundation
import UniformTypeIdentifiers

#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

enum ClipboardAttachment {

    enum Failure: Error {
        /// The clipboard holds nothing this app can attach — it is empty,
        /// or it holds only words, which belong in the composer.
        case nothingToPaste
    }

    // MARK: - Policy (no clipboard involved, so it can be tested)

    // `nonisolated` throughout this section, and not decoration: these
    // are pure rules over UTType with no pasteboard behind them, and the
    // main-actor default made `first(where: isAttachable)` a warning the
    // moment the function reference crossed into a nonisolated closure
    // parameter. The clipboard itself starts at "Reading the clipboard".

    /// Concrete types worth attaching, most-preferred first.
    ///
    /// GIF and WebP lead the image group deliberately: they are the two
    /// that animate, they are offered ALONGSIDE a flattened PNG or TIFF by
    /// almost everything that copies one, and `MediaPrep.sendsAsFile`
    /// then sends their original bytes untouched. Taking the PNG instead
    /// would produce a perfectly good still of a moving picture.
    nonisolated static let preferredTypes: [UTType] = [
        .gif, .webP, .heic, .heif, .png, .jpeg, .bmp, .tiff,
        .mpeg4Movie, .quickTimeMovie,
        .mpeg4Audio, .mp3, .wav, .aiff,
        .pdf,
    ]

    /// What the Mac's paste command listens for.
    ///
    /// Deliberately NOT `.item`: a list that matches everything would fire
    /// on a plain text paste too, and answering "there's nothing to paste"
    /// to somebody who pasted words would be worse than doing nothing. A
    /// copied FILE arrives as a URL rather than as bytes, so that one is
    /// added by hand.
    nonisolated static let pasteCommandTypes: [UTType] = preferredTypes + [.fileURL]

    /// Types that are words, links or nothing nameable — never attachments.
    private nonisolated static let textLike: [UTType] = [.text, .rtf, .html, .url]

    /// Whether a representation is worth turning into an attachment.
    nonisolated static func isAttachable(_ type: UTType) -> Bool {
        // Words and links belong in the message, not beside it — and a
        // FILE url is not bytes either; it has its own path through here.
        if textLike.contains(where: { type.conforms(to: $0) }) { return false }
        // Something no framework can name has no honest file name to give
        // a recipient, and `kind=file` insists on one.
        if type.isDynamic || type.preferredFilenameExtension == nil { return false }
        return type.conforms(to: .data)
    }

    /// The representation to take, out of everything the clipboard offers.
    nonisolated static func chosenType(from offered: [UTType]) -> UTType? {
        if let preferred = preferredTypes.first(where: { offered.contains($0) }) {
            return preferred
        }
        return offered.first(where: isAttachable)
    }

    /// Whether the platform's own paste gesture should ATTACH rather than
    /// type, given what the clipboard holds.
    ///
    /// The rule that keeps an ordinary text paste working is the second
    /// line: WORDS WIN. The exception above it is a copied FILE — every
    /// Finder copy puts the file's name on the clipboard as text too, so
    /// "there are words" cannot be the test there, and taking the name
    /// instead of the file would make the feature useless exactly where it
    /// is most wanted.
    nonisolated static func prefersAttachment(
        hasFileURL: Bool, hasAttachableData: Bool, hasText: Bool
    ) -> Bool {
        if hasFileURL { return true }
        if hasText { return false }
        return hasAttachableData
    }

    /// What to call something the clipboard handed over.
    ///
    /// The item's own suggested name wins — that is the name the person
    /// saw in the app they copied from — and only gains an extension if it
    /// arrived without one. With nothing suggested, the name is built from
    /// the type, because `kind=file` requires 1–255 characters and the
    /// scratch file's `fc-upload-<UUID>` name must never be what the
    /// recipient reads.
    nonisolated static func suggestedName(for type: UTType, suggested: String?) -> String {
        if let cleaned = MediaPrep.sanitizedName(suggested) {
            guard URL(fileURLWithPath: cleaned).pathExtension.isEmpty,
                  let ext = type.preferredFilenameExtension
            else { return cleaned }
            return MediaPrep.sanitizedName("\(cleaned).\(ext)") ?? cleaned
        }
        let base = type.conforms(to: .image)
            ? String(localized: "Pasted image")
            : String(localized: "Pasted file")
        guard let ext = type.preferredFilenameExtension else { return base }
        return "\(base).\(ext)"
    }

    // MARK: - The clipboard itself

    #if os(iOS)

    /// Whether ⌘V should attach rather than let the field paste words.
    ///
    /// Every property read here is one the system treats as harmless:
    /// `numberOfItems`, `hasStrings`, `hasImages` and
    /// `contains(pasteboardTypes:)` describe the clipboard without
    /// revealing it, so none of them shows the "pasted from" alert. Reading
    /// the ITEMS does, which is why that happens only in `prepare`, at the
    /// moment somebody actually asked for a paste.
    @MainActor
    static var offersAttachment: Bool {
        let pasteboard = UIPasteboard.general
        guard pasteboard.numberOfItems > 0 else { return false }
        return prefersAttachment(
            hasFileURL: pasteboard.contains(pasteboardTypes: [UTType.fileURL.identifier]),
            hasAttachableData: pasteboard.hasImages
                || pasteboard.contains(pasteboardTypes: preferredTypes.map(\.identifier)),
            hasText: pasteboard.hasStrings)
    }

    /// The clipboard's words, for the fallback described in the composer's
    /// ⌘V door. Nil when there are none.
    @MainActor
    static var pendingText: String? {
        guard UIPasteboard.general.hasStrings else { return nil }
        return UIPasteboard.general.string
    }

    @MainActor
    static func prepare(limit: Int) async throws -> MediaPrep.Prepared {
        guard let provider = UIPasteboard.general.itemProviders.first else {
            throw Failure.nothingToPaste
        }
        let offered = provider.registeredContentTypes
        let suggested = provider.suggestedName

        if let type = chosenType(from: offered) {
            let scratch = try await copyFileRepresentation(of: provider, type: type)
            do {
                let prepared = try await MediaPrep.prepare(
                    fileAt: scratch,
                    type: type,
                    name: suggestedName(for: type, suggested: suggested),
                    limit: limit)
                // prepareVideo hands back the source itself when it already
                // fits; only delete the scratch file when it made a new one.
                if prepared.fileURL != scratch {
                    try? FileManager.default.removeItem(at: scratch)
                }
                return prepared
            } catch {
                try? FileManager.default.removeItem(at: scratch)
                throw error
            }
        }

        // Some apps put nothing on the clipboard but a file URL. It is
        // security-scoped, which `MediaPrep.prepareFile` already knows how
        // to hold open while it copies.
        if offered.contains(.fileURL), let url = await fileURL(from: provider) {
            return try await MediaPrep.prepare(fileAt: url, limit: limit)
        }
        throw Failure.nothingToPaste
    }

    /// Copy the item's file representation somewhere we own.
    ///
    /// The URL the completion hands over is alive only until that closure
    /// returns, so the copy happens INSIDE it — the same reason
    /// `MediaPrep.prepareFile` copies what the document picker gives it.
    private static func copyFileRepresentation(
        of provider: NSItemProvider, type: UTType
    ) async throws -> URL {
        let ext = type.preferredFilenameExtension ?? "dat"
        return try await withCheckedThrowingContinuation { continuation in
            provider.loadFileRepresentation(forTypeIdentifier: type.identifier) { url, error in
                guard let url else {
                    continuation.resume(throwing: error ?? MediaPrep.PrepError.unreadable)
                    return
                }
                let destination = MediaPrep.temporaryURL(extension: ext)
                do {
                    try FileManager.default.copyItem(at: url, to: destination)
                    continuation.resume(returning: destination)
                } catch {
                    continuation.resume(throwing: MediaPrep.PrepError.unreadable)
                }
            }
        }
    }

    private static func fileURL(from provider: NSItemProvider) async -> URL? {
        await withCheckedContinuation { continuation in
            _ = provider.loadObject(ofClass: URL.self) { url, _ in
                continuation.resume(returning: url?.isFileURL == true ? url : nil)
            }
        }
    }

    #elseif os(macOS)

    // The Mac needs no `offersAttachment`: it never claims ⌘V. Its paste
    // command reaches this app only once the responder chain has got past
    // the composer's field editor, so an ordinary text paste has already
    // been taken by the field and cannot be intercepted here — see
    // MacConversationView's `.onPasteCommand`.

    @MainActor
    static func prepare(limit: Int) async throws -> MediaPrep.Prepared {
        let pasteboard = NSPasteboard.general
        // A copied file first: it has real bytes on disk and a real name,
        // which is strictly better than any representation of it.
        if let url = fileURL(on: pasteboard) {
            return try await MediaPrep.prepare(fileAt: url, limit: limit)
        }
        guard let type = dataType(on: pasteboard),
              let data = pasteboard.data(forType: NSPasteboard.PasteboardType(type.identifier)),
              !data.isEmpty
        else {
            throw Failure.nothingToPaste
        }
        return try await MediaPrep.prepare(
            data: data,
            type: type,
            name: suggestedName(for: type, suggested: nil),
            limit: limit)
    }

    private static func fileURL(on pasteboard: NSPasteboard) -> URL? {
        let options: [NSPasteboard.ReadingOptionKey: Any] = [.urlReadingFileURLsOnly: true]
        let urls = pasteboard.readObjects(forClasses: [NSURL.self], options: options) as? [URL]
        return urls?.first
    }

    private static func dataType(on pasteboard: NSPasteboard) -> UTType? {
        let offered = (pasteboard.types ?? []).compactMap { UTType($0.rawValue) }
        return chosenType(from: offered)
    }

    #endif
}
