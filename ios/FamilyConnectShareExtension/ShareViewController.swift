//
//  ShareViewController.swift
//  FamilyConnectShareExtension
//
//  Entry point for the Share Extension, on BOTH platforms: one source
//  file, a UIViewController on iOS and an NSViewController on macOS,
//  selected below. The system instantiates it when the user picks
//  "Family" from the share sheet (after the activation rule in Info.plist
//  matches: files, images and movies, up to ten of them).
//
//  This extension is a thin stager, the exchange-ios recipe: it copies
//  each shared item into the shared App Group container
//  (ShareInbox/<uuid>/<name>) and opens the main app via the
//  `familyconnect://share?ids=…` URL scheme. The app then asks WHICH CHAT
//  and stages the files in that chat's composer — nothing is sent until
//  the person presses Send there. Keeping the upload in the app means one
//  pipeline, one progress strip, and a much smaller extension.
//
//  Memory: a Share Extension runs under a tight jetsam limit (~tens of
//  MB), so the shared file is NEVER read into memory here — it is copied
//  on disk with FileManager (an APFS clone where possible). A ~300 MB
//  video would OOM-kill the appex under `Data(contentsOf:)`.
//
//  Constants are duplicated from the app on purpose (ShareImport.swift is
//  the app's copy): sharing a source file with an appex means membership
//  exceptions in the synchronized group, which is the lesson exchange-ios
//  already paid for.
//

import UniformTypeIdentifiers

#if os(iOS)
import UIKit
typealias SharePlatformViewController = UIViewController
#else
import AppKit
typealias SharePlatformViewController = NSViewController
#endif

final class ShareViewController: SharePlatformViewController {

    // Must match ShareImport in the main app.
    private let appGroupIdentifier = "group.me.nettrash.FamilyConnect"
    private let handoffScheme = "familyconnect"
    private let handoffHost = "share"
    private let inboxFolder = "ShareInbox"
    /// The protocol's cap on one message's attachments — also the
    /// activation rule's MaxCount, so more than ten never even offers us.
    private let maxItems = 10

    #if os(macOS)
    // No nib: the view is made by hand on macOS, where loadView would
    // otherwise look for one and throw.
    override func loadView() {
        view = NSView()
    }
    #endif

    override func viewDidLoad() {
        super.viewDidLoad()
        installProgressUI()
        Task { await handleShare() }
    }

    // MARK: - UI

    /// A spinner and one line — this extension is on screen for well under
    /// a second, staging files on disk and handing off.
    private func installProgressUI() {
        #if os(iOS)
        view.backgroundColor = .systemBackground
        let spinner = UIActivityIndicatorView(style: .large)
        spinner.translatesAutoresizingMaskIntoConstraints = false
        spinner.startAnimating()
        view.addSubview(spinner)
        NSLayoutConstraint.activate([
            spinner.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            spinner.centerYAnchor.constraint(equalTo: view.centerYAnchor),
        ])
        #else
        let spinner = NSProgressIndicator()
        spinner.style = .spinning
        spinner.startAnimation(nil)
        spinner.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(spinner)
        NSLayoutConstraint.activate([
            view.widthAnchor.constraint(greaterThanOrEqualToConstant: 220),
            view.heightAnchor.constraint(greaterThanOrEqualToConstant: 120),
            spinner.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            spinner.centerYAnchor.constraint(equalTo: view.centerYAnchor),
        ])
        #endif
    }

    // MARK: - Flow

    private func handleShare() async {
        defer { finish() }
        guard let inbox = inboxURL() else { return }
        try? FileManager.default.createDirectory(at: inbox, withIntermediateDirectories: true)

        var ids: [String] = []
        for provider in attachmentProviders().prefix(maxItems) {
            let id = UUID().uuidString
            let directory = inbox.appendingPathComponent(id, isDirectory: true)
            if await stage(provider, into: directory) {
                ids.append(id)
            } else {
                try? FileManager.default.removeItem(at: directory)
            }
        }
        guard !ids.isEmpty, let url = handoffURL(ids: ids) else { return }
        openHostApp(url)
    }

    /// Every attachment of every extension item, in the order shared —
    /// which is the order the message will carry them.
    private func attachmentProviders() -> [NSItemProvider] {
        guard let items = extensionContext?.inputItems as? [NSExtensionItem] else { return [] }
        return items.flatMap { $0.attachments ?? [] }
    }

    /// Copy one shared item into `directory` WITHOUT reading it into
    /// memory, under its best-effort original name (the app reads the
    /// name back off the file — the UUID directory is what travels in
    /// the URL). Returns false when the item offers no file at all.
    private func stage(_ provider: NSItemProvider, into directory: URL) async -> Bool {
        guard provider.hasItemConformingToTypeIdentifier(UTType.data.identifier) else {
            return false
        }
        // Read the suggested name HERE: the completion closure is
        // @Sendable and must not capture the non-Sendable provider — the
        // Swift 6 gotcha exchange-ios documents.
        let suggestedName = provider.suggestedName
        return await withCheckedContinuation { (continuation: CheckedContinuation<Bool, Never>) in
            // loadFileRepresentation hands over a temp URL valid only
            // inside this closure; copyItem is a filesystem-level copy —
            // no bytes flow through this process's heap.
            provider.loadFileRepresentation(forTypeIdentifier: UTType.data.identifier) { url, _ in
                guard let url else {
                    continuation.resume(returning: false)
                    return
                }
                let name = Self.safeFileName(
                    Self.bestFileName(suggested: suggestedName, url: url))
                do {
                    try FileManager.default.createDirectory(
                        at: directory, withIntermediateDirectories: true)
                    try FileManager.default.copyItem(
                        at: url, to: directory.appendingPathComponent(name))
                    continuation.resume(returning: true)
                } catch {
                    continuation.resume(returning: false)
                }
            }
        }
    }

    /// Prefer the provider's suggested name (the original filename); fall
    /// back to the temp file's. Graft on the temp file's extension when
    /// the suggested name lacks one — the extension is how the app
    /// decides photo/video/file.
    private static func bestFileName(suggested: String?, url: URL) -> String {
        let fallback = url.lastPathComponent
        guard let suggested, !suggested.isEmpty else { return fallback }
        if (suggested as NSString).pathExtension.isEmpty, !url.pathExtension.isEmpty {
            return suggested + "." + url.pathExtension
        }
        return suggested
    }

    /// A name safe to create INSIDE the staging directory: no path
    /// separators, nothing hidden, never empty — the same cleaning the
    /// app applies to names off the wire.
    private static func safeFileName(_ name: String) -> String {
        let cleaned = name
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: ":", with: "_")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let visible = String(cleaned.drop(while: { $0 == "." }))
        return visible.isEmpty ? "file" : String(visible.prefix(255))
    }

    // MARK: - Hand-off

    private func inboxURL() -> URL? {
        FileManager.default
            .containerURL(forSecurityApplicationGroupIdentifier: appGroupIdentifier)?
            .appendingPathComponent(inboxFolder, isDirectory: true)
    }

    /// `familyconnect://share?ids=<uuid>,<uuid>` — ids only. A filename
    /// does not survive a query string with its commas intact, so names
    /// stay on the staged files, and UUIDs are what the app will accept
    /// (anything else would become a path over there).
    private func handoffURL(ids: [String]) -> URL? {
        var components = URLComponents()
        components.scheme = handoffScheme
        components.host = handoffHost
        components.queryItems = [URLQueryItem(name: "ids", value: ids.joined(separator: ","))]
        return components.url
    }

    private func openHostApp(_ url: URL) {
        #if os(iOS)
        // An extension cannot call UIApplication.shared.open — walk the
        // responder chain to the UIApplication and open through it.
        var responder: UIResponder? = self
        while let current = responder {
            if let app = current as? UIApplication {
                app.open(url, options: [:], completionHandler: nil)
                return
            }
            responder = current.next
        }
        #else
        NSWorkspace.shared.open(url)
        #endif
    }

    private func finish() {
        extensionContext?.completeRequest(returningItems: [], completionHandler: nil)
    }
}
