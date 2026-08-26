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
//  And the THIRD decision, which is the one every door gets wrong on its
//  own: whether this paste is an attachment at all. `decide` answers that
//  from free probes, `action` turns the answer into the one thing a door
//  must do, and `door` is the single call every paste door makes — the
//  attach menu's Paste item, the hardware ⌘V, the Mac's paste command.
//  `prepare` is NOT that decision and must never be used as one: it takes
//  the best representation it can find, which on a clipboard holding both
//  words and a picture is the picture — the opposite of what the rule says.
//  It runs only after `decide` has said `.attachment`.
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

    /// What the clipboard is, as far as a composer is concerned.
    ///
    /// Three answers rather than two, because "there is nothing here" and
    /// "there are words here" want different things from a door: one has to
    /// say so, the other has to type.
    enum Decision: Equatable {
        /// Bytes worth staging as an attachment.
        case attachment
        /// Words, which belong in the text field.
        case text
        /// Nothing this app can do anything with.
        case nothing
    }

    /// THE rule, and the only one: what a paste means, given what the
    /// clipboard holds.
    ///
    /// The line that keeps an ordinary text paste working is the second:
    /// WORDS WIN. The exception above it is a copied FILE — every Finder
    /// copy puts the file's name on the clipboard as text too, so "there
    /// are words" cannot be the test there, and taking the name instead of
    /// the file would make the feature useless exactly where it is most
    /// wanted.
    nonisolated static func decide(
        hasFileURL: Bool, hasAttachableData: Bool, hasText: Bool
    ) -> Decision {
        if hasFileURL { return .attachment }
        if hasText { return .text }
        return hasAttachableData ? .attachment : .nothing
    }

    /// The one thing a paste door does.
    enum Action: Equatable {
        /// Stage what the clipboard holds, so a caption can be added and an
        /// accidental paste discarded.
        case attach
        /// Put the clipboard's words in the draft.
        case type
        /// The composer cannot take an attachment right now — and has to
        /// SAY so. The iPad's ⌘V used to be `.disabled` for this, so ⌘V of
        /// a picture during an edit did nothing and explained nothing.
        case busy
        /// There is nothing on the clipboard worth a message.
        case nothing
    }

    /// Turn the rule's answer into what the door must do.
    ///
    /// `composerIsBusy` gates the ATTACHMENT branch ONLY. Words are never
    /// busy: a draft being edited is still a draft, and a text paste into
    /// one is exactly what was meant. Gating words here is how the phone's
    /// ⌘V would start swallowing ordinary text pastes the moment an upload
    /// was running.
    nonisolated static func action(for decision: Decision, composerIsBusy: Bool) -> Action {
        switch decision {
        case .attachment: return composerIsBusy ? .busy : .attach
        case .text: return .type
        case .nothing: return .nothing
        }
    }

    /// The single call every paste door makes.
    ///
    /// Reads nothing but the free probes, so no door shows iOS's "Allow
    /// Paste?" alert merely by asking what it is looking at. The payload is
    /// read exactly once afterwards, by whichever of `prepare` or
    /// `pendingText` the answer names.
    @MainActor
    static func door(composerIsBusy: Bool) -> Action {
        action(for: decision, composerIsBusy: composerIsBusy)
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

    /// What the clipboard holds, decided WITHOUT reading it.
    ///
    /// Every property read here is one the system treats as harmless:
    /// `numberOfItems`, `hasStrings`, `hasImages` and
    /// `contains(pasteboardTypes:)` describe the clipboard without
    /// revealing it, so none of them shows the "pasted from" alert. Reading
    /// the ITEMS does — and so does reading the string — which is why both
    /// happen only in `prepare` and `pendingText`, once, after this has
    /// said which of the two the paste is.
    @MainActor
    static var decision: Decision {
        let pasteboard = UIPasteboard.general
        guard pasteboard.numberOfItems > 0 else { return .nothing }
        return decide(
            hasFileURL: pasteboard.contains(pasteboardTypes: [UTType.fileURL.identifier]),
            hasAttachableData: pasteboard.hasImages
                || pasteboard.contains(pasteboardTypes: preferredTypes.map(\.identifier)),
            hasText: pasteboard.hasStrings)
    }

    /// The clipboard's words. Nil when there are none.
    ///
    /// The one read of the payload on the text branch, and the one moment
    /// the "Allow Paste?" alert is worth showing: somebody has just asked
    /// for a paste and the rule has already said this one is words.
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

    // The Mac never claims ⌘V with a keyboard shortcut of its own: a
    // shortcut outranks the field editor and steals every ordinary text
    // paste. Its paste command reaches this app only once the responder
    // chain has got past the composer's field editor, so a text paste into
    // a focused field has already been taken by the field — see
    // MacConversationView's `.onPasteCommand`. Everything that DOES reach
    // this app still goes through `decision`, because the attach menu's
    // Paste item is a door like any other and the clipboard it reads may
    // very well be words.

    /// What the clipboard holds, decided without consuming it.
    ///
    /// `types` and the two `canReadObject` probes describe the pasteboard
    /// rather than take from it, which is the same discipline the phone
    /// needs for a different reason — there is no "Allow Paste?" alert
    /// here, but there is still exactly one right answer and reading the
    /// bytes is not how it is found.
    @MainActor
    static var decision: Decision {
        let pasteboard = NSPasteboard.general
        let offered = (pasteboard.types ?? []).compactMap { UTType($0.rawValue) }
        return decide(
            hasFileURL: pasteboard.canReadObject(
                forClasses: [NSURL.self],
                options: [.urlReadingFileURLsOnly: true]),
            hasAttachableData: chosenType(from: offered) != nil,
            hasText: pasteboard.canReadObject(forClasses: [NSString.self]))
    }

    /// The clipboard's words. Nil when there are none.
    @MainActor
    static var pendingText: String? {
        NSPasteboard.general.string(forType: .string)
    }

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
