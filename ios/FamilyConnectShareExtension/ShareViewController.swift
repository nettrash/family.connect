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
//  The hand-off contract — scheme, host, App Group, inbox folder, the
//  ten-item cap, the URL, the staging path and the filename cleaning —
//  is NOT redeclared here. It lives in ShareHandoff.swift
//  (ios/FamilyConnectShared), compiled into this appex and into the app,
//  because a constant that disagrees across the process boundary makes a
//  share vanish without either side reporting a thing (issue #34). The
//  file is dependency-free, so nothing of the app's world comes with it.
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
        // No container means the App Group entitlement is missing: there
        // is nowhere the app could read from, so stage nothing at all
        // rather than copying files into a folder only this appex sees.
        guard let container = ShareHandoff.containerURL() else { return }
        try? FileManager.default.createDirectory(
            at: ShareHandoff.inboxURL(container: container), withIntermediateDirectories: true)

        // Take out whatever an earlier hand-off abandoned, BEFORE staging
        // this one (issue #35). Two reasons for this appex rather than
        // only the app, and for here rather than after the hand-off:
        //
        // This process is the only one guaranteed to run when an orphan
        // is CREATED. The app may never be opened between two shares —
        // share, change your mind, share again next week — and every one
        // of those leaves a staging directory nothing else will ever look
        // at. Whereas after `openHostApp` is the moment the system has
        // what it needs to tear this extension down, so a sweep there is
        // a sweep that may not run. Before staging is also the ordering
        // that cannot go wrong: the sweep can only see directories from
        // earlier hand-offs, over and above the day-long floor that
        // already spares anything this run is about to write.
        //
        // Affordable under the jetsam limit this class is written around:
        // one directory listing plus a stat per entry — of a folder that
        // is empty whenever the last share landed — and not one byte of
        // any file is read.
        _ = ShareHandoff.sweepOrphanedStaging(container: container)

        var ids: [String] = []
        for provider in attachmentProviders().prefix(ShareHandoff.maxAttachmentsPerMessage) {
            let id = UUID().uuidString
            // Through the shared helper, never by hand: this is the path
            // the app will rebuild from the id in the URL, and the two
            // spellings have to be one spelling.
            guard let directory = ShareHandoff.stagingDirectory(
                container: container, id: id) else { continue }
            if await stage(provider, into: directory) {
                ids.append(id)
            } else {
                try? FileManager.default.removeItem(at: directory)
            }
        }
        guard let url = ShareHandoff.handoffURL(ids: ids) else { return }
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
                let name = ShareHandoff.safeFileName(
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

    // MARK: - Hand-off

    /// Open the app on the hand-off URL ShareHandoff just built.
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
